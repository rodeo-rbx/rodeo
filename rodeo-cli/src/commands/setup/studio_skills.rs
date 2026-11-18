//! The Studio skills in a project's skill folders: Roblox's Studio Assistant
//! skills as this repository keeps them for agents outside Studio
//! (studio-skills/curated, each a folder named `rbx-<name>`). `setup` compares
//! a project's copies with the latest on GitHub and, when told to, installs
//! the ones it lacks and replaces the ones that differ.
//!
//! The skills carry no version: a copy is the latest when its content is.
//! GitHub lists each file with its git blob id, a hash of the content, and the
//! same hash of the project's file says whether the two are one. So one
//! listing says what differs, and only those files are downloaded.

use std::collections::BTreeMap;
use std::io::{IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{bail, Context, Result};
use futures::StreamExt;
use sha1::{Digest, Sha1};

use crate::cli::StudioSkills;

const REPOSITORY: &str = "rodeo-rbx/rodeo";
const BRANCH: &str = "main";
/// The skills' folder in the repository.
const FOLDER: &str = "studio-skills/curated";
const DOWNLOADS_AT_ONCE: usize = 8;

/// One file of the latest skills: `<skill>/<its path in the skill>`, and the
/// git blob id of its content.
#[derive(Clone, Debug, PartialEq)]
struct Listed {
    path: String,
    id: String,
}

/// Where the latest skills come from: GitHub, or a test's own files.
trait Source {
    fn files(&self) -> &[Listed];
    /// The content of a listed file.
    async fn file(&self, path: &str) -> Result<Vec<u8>>;
}

/// The skills as they are at one commit of the repository's branch, so the
/// listing and every download are of the same moment.
struct GitHub {
    client: reqwest::Client,
    commit: String,
    files: Vec<Listed>,
}

impl GitHub {
    /// Two requests to GitHub's API, which allows 60 an hour without a login.
    async fn latest() -> Result<GitHub> {
        let client = reqwest::Client::builder()
            .user_agent(concat!("rodeo/", env!("CARGO_PKG_VERSION")))
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(60))
            .build()
            .context("failed to start an HTTP client")?;
        let api = format!("https://api.github.com/repos/{REPOSITORY}");
        let commit = answer(client.get(format!("{api}/commits/{BRANCH}")).header("Accept", "application/vnd.github.sha")).await?;
        let commit = commit.text().await.context("GitHub's answer stopped short")?.trim().to_string();
        if commit.len() != 40 || !commit.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            bail!("GitHub named no commit for {REPOSITORY}'s {BRANCH}");
        }
        let listing = answer(client.get(format!("{api}/git/trees/{commit}:{FOLDER}?recursive=1")).header("Accept", "application/vnd.github+json")).await?;
        let listing: serde_json::Value = listing.json().await.context("GitHub's listing of the skills is not JSON")?;
        Ok(GitHub { client, commit, files: listed(&listing)? })
    }
}

impl Source for GitHub {
    fn files(&self) -> &[Listed] {
        &self.files
    }

    async fn file(&self, path: &str) -> Result<Vec<u8>> {
        let mut url = reqwest::Url::parse("https://raw.githubusercontent.com").expect("a URL");
        url.path_segments_mut()
            .expect("a URL with a host")
            .extend(REPOSITORY.split('/'))
            .push(&self.commit)
            .extend(FOLDER.split('/'))
            .extend(path.split('/'));
        let content = answer(self.client.get(url)).await?.bytes().await.context("the download stopped short")?;
        Ok(content.to_vec())
    }
}

/// The response to a request, or why there is none to use.
async fn answer(request: reqwest::RequestBuilder) -> Result<reqwest::Response> {
    let response = request.send().await.context("no answer from GitHub")?;
    let status = response.status();
    if status.is_success() {
        return Ok(response);
    }
    let spent = response.headers().get("x-ratelimit-remaining").is_some_and(|left| left == "0");
    if spent && matches!(status.as_u16(), 403 | 429) {
        bail!("GitHub's hourly limit on requests without a login is used up here; try again later");
    }
    bail!("GitHub answered {status} for {}", response.url());
}

/// The files of GitHub's listing of the skills' folder (a recursive git tree).
fn listed(listing: &serde_json::Value) -> Result<Vec<Listed>> {
    if listing["truncated"].as_bool() == Some(true) {
        bail!("GitHub cut its listing of the skills short");
    }
    let entries = listing["tree"].as_array().context("GitHub's listing of the skills has no entries")?;
    let mut files = Vec::new();
    for entry in entries {
        // Folders are listed too, and a link or a submodule is no file to write.
        let file = entry["type"] == "blob" && entry["mode"].as_str().is_some_and(|mode| mode.starts_with("100"));
        if !file {
            continue;
        }
        let (Some(path), Some(id)) = (entry["path"].as_str(), entry["sha"].as_str()) else {
            bail!("GitHub's listing of the skills has a file with no path or no id");
        };
        if !in_a_skill(path) {
            bail!("GitHub's listing of the skills has {path:?}, which is not a file in a skill's folder");
        }
        files.push(Listed { path: path.to_string(), id: id.to_string() });
    }
    if files.is_empty() {
        bail!("GitHub's listing of the skills is empty");
    }
    Ok(files)
}

/// Whether a listed path names a file inside a skill's folder and nothing
/// else: it comes from the network and is written below the project's folder.
fn in_a_skill(path: &str) -> bool {
    let parts: Vec<&str> = path.split('/').collect();
    parts.len() >= 2 && parts.iter().all(|part| !part.is_empty() && *part != "." && *part != ".." && !part.contains(['\\', ':']))
}

/// The id git gives a file with this content (the SHA-1 of a header and it).
fn blob_id(content: &[u8]) -> String {
    let mut hash = Sha1::new();
    hash.update(format!("blob {}\0", content.len()));
    hash.update(content);
    hash.finalize().iter().map(|byte| format!("{byte:02x}")).collect()
}

/// Whether a project's file holds what the listed one does. A checkout that
/// turned its line endings into Windows' still holds the same file.
fn same(content: &[u8], id: &str) -> bool {
    if blob_id(content) == id {
        return true;
    }
    if !content.windows(2).any(|pair| pair == b"\r\n") {
        return false;
    }
    let mut unix = Vec::with_capacity(content.len());
    for (index, byte) in content.iter().enumerate() {
        if !(*byte == b'\r' && content.get(index + 1) == Some(&b'\n')) {
            unix.push(*byte);
        }
    }
    blob_id(&unix) == id
}

#[derive(Debug, PartialEq)]
enum State {
    /// The project has no folder for the skill.
    Missing,
    /// It has one, and a file of the skill is absent from it or differs.
    Differs,
    Latest,
}

/// A skill as one of the project's folders has it.
struct Skill<'a> {
    name: &'a str,
    state: State,
    /// The files that are absent or differ: all there is to write.
    write: Vec<&'a Listed>,
}

fn local(folder: &Path, path: &str) -> PathBuf {
    path.split('/').fold(folder.to_path_buf(), |at, part| at.join(part))
}

/// Each listed skill as `folder` has it, by name. Only listed files count:
/// what else a skill's folder holds, and skills that aren't listed, are the
/// project's own.
fn compare<'a>(folder: &Path, files: &'a [Listed]) -> Vec<Skill<'a>> {
    let mut skills: BTreeMap<&str, Vec<&Listed>> = BTreeMap::new();
    for file in files {
        let name = file.path.split('/').next().unwrap_or_default();
        skills.entry(name).or_default().push(file);
    }
    skills
        .into_iter()
        .map(|(name, files)| {
            if !folder.join(name).is_dir() {
                return Skill { name, state: State::Missing, write: files };
            }
            let write: Vec<&Listed> = files
                .into_iter()
                .filter(|file| !std::fs::read(local(folder, &file.path)).is_ok_and(|content| same(&content, &file.id)))
                .collect();
            let state = if write.is_empty() { State::Latest } else { State::Differs };
            Skill { name, state, write }
        })
        .collect()
}

/// The folders among `candidates` that exist, each once: a project's
/// `.claude/skills` and `.agents/skills` are often links to one folder.
fn existing(candidates: &[&str]) -> Vec<(String, PathBuf)> {
    let mut seen = Vec::new();
    let mut folders = Vec::new();
    for name in candidates {
        let path = PathBuf::from(name);
        if !path.is_dir() {
            continue;
        }
        let real = std::fs::canonicalize(&path).unwrap_or_else(|_| path.clone());
        if !seen.contains(&real) {
            seen.push(real);
            folders.push((name.replace('\\', "/"), path));
        }
    }
    folders
}

/// "3 not installed (rbx-a, rbx-b, rbx-c), 1 differs from the latest (rbx-d)"
fn described(skills: &[Skill]) -> String {
    let names = |state: State| skills.iter().filter(|skill| skill.state == state).map(|skill| skill.name).collect::<Vec<_>>();
    let (missing, differing) = (names(State::Missing), names(State::Differs));
    let mut parts = Vec::new();
    if !missing.is_empty() {
        // (a project that has none of them: the count says it)
        let named = if missing.len() > 6 { String::new() } else { format!(" ({})", missing.join(", ")) };
        parts.push(format!("{} not installed{named}", missing.len()));
    }
    if !differing.is_empty() {
        let verb = if differing.len() == 1 { "differs" } else { "differ" };
        parts.push(format!("{} {verb} from the latest ({})", differing.len(), differing.join(", ")));
    }
    parts.join(", ")
}

/// What was decided about the skills that are missing or differ.
#[derive(Debug, PartialEq)]
enum Answer {
    Update,
    Leave,
    /// Nobody is there to ask.
    CannotAsk,
}

#[derive(Debug, PartialEq)]
enum Outcome {
    /// Every folder already had the latest of every skill.
    Latest,
    Left,
    /// Skills installed and replaced, counted once for each folder.
    Wrote { installed: usize, replaced: usize },
}

/// Says how each folder's skills stand against the source's, and when some are
/// missing or differ and `answer` (given the question) says to update, writes
/// the files that are absent or differ. Every download is checked against the
/// listing before anything is written.
async fn sync(folders: &[(String, PathBuf)], source: &impl Source, answer: impl FnOnce(&str) -> Answer) -> Result<Outcome> {
    let found: Vec<(&str, &Path, Vec<Skill>)> = folders.iter().map(|(shown, path)| (shown.as_str(), path.as_path(), compare(path, source.files()))).collect();
    let (mut missing, mut differing) = (0, 0);
    for (shown, _, skills) in &found {
        let count = |state: State| skills.iter().filter(|skill| skill.state == state).count();
        missing += count(State::Missing);
        differing += count(State::Differs);
        if skills.iter().all(|skill| skill.state == State::Latest) {
            tracing::info!("The {} Studio skills in {shown} are the latest", skills.len());
        } else {
            tracing::info!("Studio skills in {shown}: {}", described(skills));
        }
    }
    let them = if missing + differing == 1 { "it" } else { "them" };
    let question = match (missing > 0, differing > 0) {
        (false, false) => return Ok(Outcome::Latest),
        (true, false) => format!("Install {them}?"),
        (false, true) => format!("Replace {them} with the latest?"),
        (true, true) => "Install and replace them with the latest?".to_string(),
    };
    match answer(&question) {
        Answer::Update => {}
        Answer::Leave => {
            tracing::info!("Left the Studio skills as they are");
            return Ok(Outcome::Left);
        }
        Answer::CannotAsk => {
            tracing::info!("Nothing was changed: `rodeo setup --studio-skills update` installs the latest");
            return Ok(Outcome::Left);
        }
    }

    // Each file once, whichever folders want it.
    let wanted: BTreeMap<&str, &Listed> = found.iter().flat_map(|(_, _, skills)| skills).flat_map(|skill| &skill.write).map(|file| (file.path.as_str(), *file)).collect();
    let downloads = futures::stream::iter(wanted.into_values())
        .map(|file| async move {
            let content = source.file(&file.path).await.with_context(|| format!("failed to download {}", file.path))?;
            if blob_id(&content) != file.id {
                bail!("the download of {} is not the file GitHub listed", file.path);
            }
            Ok((file.path.as_str(), content))
        })
        .buffer_unordered(DOWNLOADS_AT_ONCE)
        .collect::<Vec<Result<(&str, Vec<u8>)>>>()
        .await;
    let contents: BTreeMap<&str, Vec<u8>> = downloads.into_iter().collect::<Result<_>>()?;

    let (mut installed, mut replaced) = (0, 0);
    for (shown, folder, skills) in &found {
        let (mut here_installed, mut here_replaced) = (0, 0);
        for skill in skills.iter().filter(|skill| skill.state != State::Latest) {
            for file in &skill.write {
                let path = local(folder, &file.path);
                std::fs::create_dir_all(path.parent().expect("has a parent")).with_context(|| format!("failed to create the folder for {shown}/{}", file.path))?;
                std::fs::write(&path, &contents[file.path.as_str()]).with_context(|| format!("failed to write {shown}/{}", file.path))?;
            }
            if skill.state == State::Missing {
                here_installed += 1;
            } else {
                here_replaced += 1;
            }
        }
        let said = match (here_installed, here_replaced) {
            (0, 0) => continue,
            (count, 0) => format!("Installed {count}"),
            (0, count) => format!("Replaced {count}"),
            (new, old) => format!("Installed {new} and replaced {old}"),
        };
        let skills = if here_installed + here_replaced == 1 { "skill" } else { "skills" };
        tracing::info!("{said} Studio {skills} in {shown}");
        installed += here_installed;
        replaced += here_replaced;
    }
    Ok(Outcome::Wrote { installed, replaced })
}

/// Asks on the terminal; anything but a yes is a no.
fn ask(question: &str) -> Answer {
    if !(std::io::stdin().is_terminal() && std::io::stderr().is_terminal()) {
        return Answer::CannotAsk;
    }
    eprint!("{question} [y/N] ");
    let _ = std::io::stderr().flush();
    let mut line = String::new();
    let _ = std::io::stdin().read_line(&mut line);
    if matches!(line.trim().to_ascii_lowercase().as_str(), "y" | "yes") {
        Answer::Update
    } else {
        Answer::Leave
    }
}

/// `setup`'s part for the Studio skills, in the skill folders (`candidates`)
/// the project has. Asked only to check, a GitHub that can't be reached is a
/// warning, since the rest of `setup` is done; told to update, it is the
/// command's failure.
pub(super) async fn run(mode: StudioSkills, candidates: &[&str]) -> Result<()> {
    if mode == StudioSkills::Skip {
        return Ok(());
    }
    let folders = existing(candidates);
    if folders.is_empty() {
        return Ok(());
    }
    let source = match GitHub::latest().await {
        Ok(source) => source,
        Err(error) if mode == StudioSkills::Update => return Err(error.context("failed to get the latest Studio skills")),
        Err(error) => {
            tracing::warn!("Couldn't check the Studio skills: {error:#}");
            return Ok(());
        }
    };
    let answer = |question: &str| if mode == StudioSkills::Update { Answer::Update } else { ask(question) };
    sync(&folders, &source, answer).await.context("failed to update the Studio skills")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A source of files a test names, with what it serves for each.
    struct Fake {
        files: Vec<Listed>,
        served: BTreeMap<String, Vec<u8>>,
    }

    impl Fake {
        fn of(files: &[(&str, &str)]) -> Fake {
            Fake {
                files: files.iter().map(|(path, content)| Listed { path: path.to_string(), id: blob_id(content.as_bytes()) }).collect(),
                served: files.iter().map(|(path, content)| (path.to_string(), content.as_bytes().to_vec())).collect(),
            }
        }
    }

    impl Source for Fake {
        fn files(&self) -> &[Listed] {
            &self.files
        }

        async fn file(&self, path: &str) -> Result<Vec<u8>> {
            self.served.get(path).cloned().context("not served")
        }
    }

    fn scratch(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("rodeo-setup-{name}-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    fn write(path: PathBuf, content: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, content).unwrap();
    }

    fn read(path: PathBuf) -> Option<String> {
        std::fs::read_to_string(path).ok()
    }

    const LATEST: [(&str, &str); 4] = [
        ("rbx-debug/SKILL.md", "# debug\n"),
        ("rbx-debug/commands/step.luau", "return 1\n"),
        ("rbx-docs/SKILL.md", "# docs\n"),
        ("rbx-input/SKILL.md", "# input\n"),
    ];

    fn states(folder: &Path, source: &Fake) -> Vec<(String, State, usize)> {
        compare(folder, source.files()).into_iter().map(|skill| (skill.name.to_string(), skill.state, skill.write.len())).collect()
    }

    #[test]
    fn blob_ids_are_gits() {
        // `git hash-object` of an empty file, and of one holding "hello\n"
        assert_eq!(blob_id(b""), "e69de29bb2d1d6434b8b29ae775ad8c2e48c5391");
        assert_eq!(blob_id(b"hello\n"), "ce013625030ba8dba906f756967f9e9ca394464a");
    }

    #[test]
    fn a_copy_with_windows_line_endings_is_the_same_file() {
        let id = blob_id(b"one\ntwo\n");
        assert!(same(b"one\ntwo\n", &id));
        assert!(same(b"one\r\ntwo\r\n", &id));
        assert!(!same(b"one\ntwo\nthree\n", &id));
        assert!(!same(b"one\rtwo\n", &id), "a lone carriage return is content");
    }

    #[test]
    fn reads_the_files_of_githubs_listing() {
        let listing = serde_json::json!({ "truncated": false, "tree": [
            { "path": "rbx-debug", "mode": "040000", "type": "tree", "sha": "aa" },
            { "path": "rbx-debug/SKILL.md", "mode": "100644", "type": "blob", "sha": "bb", "size": 3 },
            { "path": "rbx-debug/commands/step.luau", "mode": "100755", "type": "blob", "sha": "cc", "size": 3 },
            { "path": "rbx-debug/link", "mode": "120000", "type": "blob", "sha": "dd", "size": 3 },
        ]});
        let files = listed(&listing).unwrap();
        let listed_as: Vec<(&str, &str)> = files.iter().map(|file| (file.path.as_str(), file.id.as_str())).collect();
        assert_eq!(listed_as, [("rbx-debug/SKILL.md", "bb"), ("rbx-debug/commands/step.luau", "cc")]);

        let refused = |listing: serde_json::Value| format!("{:#}", listed(&listing).unwrap_err());
        assert!(refused(serde_json::json!({ "truncated": true, "tree": [] })).contains("short"));
        assert!(refused(serde_json::json!({ "tree": [] })).contains("empty"));
        assert!(refused(serde_json::json!({ "message": "Not Found" })).contains("no entries"));
        for path in ["README.md", "../outside/SKILL.md", "rbx-debug/../../outside", "/rbx-debug/SKILL.md", "rbx-debug//SKILL.md", "rbx-debug/C:evil", "rbx-debug\\SKILL.md"] {
            let listing = serde_json::json!({ "tree": [{ "path": path, "mode": "100644", "type": "blob", "sha": "bb" }] });
            assert!(refused(listing).contains("not a file in a skill's folder"), "{path}");
        }
    }

    #[test]
    fn a_skill_is_missing_differs_or_is_the_latest() {
        let folder = scratch("compare");
        let source = Fake::of(&LATEST);
        // rbx-debug lacks a file, rbx-docs was edited, rbx-input isn't there
        write(folder.join("rbx-debug/SKILL.md"), "# debug\n");
        write(folder.join("rbx-docs/SKILL.md"), "# docs, edited\n");
        assert_eq!(states(&folder, &source), [("rbx-debug".to_string(), State::Differs, 1), ("rbx-docs".to_string(), State::Differs, 1), ("rbx-input".to_string(), State::Missing, 1)]);
        assert_eq!(described(&compare(&folder, source.files())), "1 not installed (rbx-input), 2 differ from the latest (rbx-debug, rbx-docs)");

        // What the listing doesn't name is the project's own
        write(folder.join("rbx-debug/commands/step.luau"), "return 1\n");
        write(folder.join("rbx-debug/notes.md"), "mine\n");
        write(folder.join("my-skill/SKILL.md"), "# mine\n");
        assert_eq!(states(&folder, &source)[0], ("rbx-debug".to_string(), State::Latest, 0));
        let _ = std::fs::remove_dir_all(&folder);
    }

    #[tokio::test]
    async fn an_update_writes_what_is_absent_or_differs_and_nothing_else() {
        let folder = scratch("update");
        let source = Fake::of(&LATEST);
        write(folder.join("rbx-debug/SKILL.md"), "# debug\n");
        write(folder.join("rbx-debug/notes.md"), "mine\n");
        write(folder.join("rbx-docs/SKILL.md"), "# docs, edited\n");
        write(folder.join("my-skill/SKILL.md"), "# mine\n");
        let folders = [("skills".to_string(), folder.clone())];

        let mut asked = String::new();
        let left = sync(&folders, &source, |question| {
            asked = question.to_string();
            Answer::Leave
        })
        .await
        .unwrap();
        assert_eq!((left, asked.as_str()), (Outcome::Left, "Install and replace them with the latest?"));
        assert_eq!(read(folder.join("rbx-docs/SKILL.md")).as_deref(), Some("# docs, edited\n"), "a no leaves everything");
        assert!(!folder.join("rbx-input").exists());
        assert_eq!(sync(&folders, &source, |_| Answer::CannotAsk).await.unwrap(), Outcome::Left);
        assert!(!folder.join("rbx-input").exists(), "with nobody to ask, nothing is written");

        assert_eq!(sync(&folders, &source, |_| Answer::Update).await.unwrap(), Outcome::Wrote { installed: 1, replaced: 2 });
        for (path, content) in LATEST {
            assert_eq!(read(folder.join(path)).as_deref(), Some(content), "{path}");
        }
        assert_eq!(read(folder.join("rbx-debug/notes.md")).as_deref(), Some("mine\n"));
        assert_eq!(read(folder.join("my-skill/SKILL.md")).as_deref(), Some("# mine\n"));

        let again = sync(&folders, &source, |_| panic!("asked with nothing to do")).await.unwrap();
        assert_eq!(again, Outcome::Latest);
        let _ = std::fs::remove_dir_all(&folder);
    }

    #[tokio::test]
    async fn a_download_that_is_not_the_listed_file_writes_nothing() {
        let folder = scratch("mismatch");
        let mut source = Fake::of(&LATEST);
        source.served.insert("rbx-input/SKILL.md".to_string(), b"something else\n".to_vec());
        let folders = [("skills".to_string(), folder.clone())];

        let error = sync(&folders, &source, |_| Answer::Update).await.unwrap_err();
        assert!(format!("{error:#}").contains("rbx-input/SKILL.md is not the file GitHub listed"), "{error:#}");
        assert_eq!(std::fs::read_dir(&folder).unwrap().count(), 0, "the files that did download are not written either");

        source.served.remove("rbx-docs/SKILL.md");
        let error = sync(&folders, &source, |_| Answer::Update).await.unwrap_err();
        assert!(format!("{error:#}").contains("failed to download rbx-docs/SKILL.md"), "{error:#}");
        assert_eq!(std::fs::read_dir(&folder).unwrap().count(), 0);
        let _ = std::fs::remove_dir_all(&folder);
    }

    #[tokio::test]
    async fn each_folder_gets_what_it_lacks() {
        let root = scratch("folders");
        let source = Fake::of(&LATEST);
        let (claude, agents) = (root.join("claude"), root.join("agents"));
        for (path, content) in LATEST {
            write(claude.join(path), content);
        }
        std::fs::create_dir_all(&agents).unwrap();
        let folders = [("claude".to_string(), claude.clone()), ("agents".to_string(), agents.clone())];

        let mut asked = String::new();
        let wrote = sync(&folders, &source, |question| {
            asked = question.to_string();
            Answer::Update
        })
        .await
        .unwrap();
        assert_eq!((wrote, asked.as_str()), (Outcome::Wrote { installed: 3, replaced: 0 }, "Install them?"));

        // One skill to replace in one folder is "it"
        write(claude.join("rbx-docs/SKILL.md"), "# docs, edited\n");
        let left = sync(&folders, &source, |question| {
            asked = question.to_string();
            Answer::Leave
        })
        .await
        .unwrap();
        assert_eq!((left, asked.as_str()), (Outcome::Left, "Replace it with the latest?"));
        assert_eq!(read(agents.join("rbx-debug/commands/step.luau")).as_deref(), Some("return 1\n"));
        let _ = std::fs::remove_dir_all(&root);
    }

    /// `.claude/skills` and `.agents/skills` linked to one folder are one
    /// folder to check, and a name with no folder is none.
    #[cfg(unix)]
    #[test]
    fn folders_that_are_one_are_checked_once() {
        let root = scratch("links");
        std::fs::create_dir_all(root.join("skills")).unwrap();
        std::fs::create_dir_all(root.join("other")).unwrap();
        std::os::unix::fs::symlink(root.join("skills"), root.join("claude")).unwrap();
        std::os::unix::fs::symlink(root.join("skills"), root.join("agents")).unwrap();
        let name = |folder: &str| root.join(folder).to_string_lossy().into_owned();
        let names = [name("claude"), name("agents"), name("absent"), name("other")];
        let found = existing(&names.iter().map(String::as_str).collect::<Vec<_>>());
        assert_eq!(found.iter().map(|(shown, _)| shown.clone()).collect::<Vec<_>>(), [name("claude"), name("other")]);
        let _ = std::fs::remove_dir_all(&root);
    }
}
