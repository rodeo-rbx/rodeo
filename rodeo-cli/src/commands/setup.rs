use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

static TYPEDEF_INIT: &str = include_str!("../../../rodeo-pkg/rodeo/src/init.luau");
static TYPEDEF_FS: &str = include_str!("../../../rodeo-pkg/rodeo/src/fs.luau");
static TYPEDEF_IO: &str = include_str!("../../../rodeo-pkg/rodeo/src/io.luau");
static TYPEDEF_PROCESS: &str = include_str!("../../../rodeo-pkg/rodeo/src/process.luau");
static TYPEDEF_STREAM: &str = include_str!("../../../rodeo-pkg/rodeo/src/stream.luau");
static TYPEDEF_ROBLOX: &str = include_str!("../../../rodeo-pkg/rodeo/src/roblox.luau");
static TYPEDEF_INTERNAL_SINGLETON: &str = include_str!("../../../rodeo-pkg/rodeo/src/_internal/singleton.luau");

static TYPEDEFS: [(&str, &str); 7] = [
    ("init.luau", TYPEDEF_INIT),
    ("fs.luau", TYPEDEF_FS),
    ("io.luau", TYPEDEF_IO),
    ("process.luau", TYPEDEF_PROCESS),
    ("stream.luau", TYPEDEF_STREAM),
    ("roblox.luau", TYPEDEF_ROBLOX),
    ("_internal/singleton.luau", TYPEDEF_INTERNAL_SINGLETON),
];

/// The rodeo skill, as of this build (the build checks its version stamps
/// match the crate's).
static SKILL: &str = include_str!("../../../.claude/skills/rodeo/SKILL.md");

/// Where agents look for a project's skills. `setup` installs the rodeo skill
/// into each of these the project already has, and creates none.
const SKILL_DIRS: [&str; 2] = [".claude/skills", ".agents/skills"];

pub fn main() -> Result<()> {
    let version = env!("CARGO_PKG_VERSION");
    // dirs::home_dir() resolves %USERPROFILE% on Windows and $HOME on Unix —
    // `rodeo setup` was previously Unix-only because it read HOME directly.
    let home = dirs::home_dir().context("could not determine home directory")?;

    // Write typedefs to ~/.rodeo/typedefs/{version}/
    let typedefs_dir = home.join(".rodeo").join("typedefs").join(version);
    if typedefs_dir.exists() {
        std::fs::remove_dir_all(&typedefs_dir)
            .context("failed to remove existing typedefs directory")?;
    }
    std::fs::create_dir_all(&typedefs_dir)
        .context("failed to create typedefs directory")?;

    for (filename, content) in &TYPEDEFS {
        let path = typedefs_dir.join(filename);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .context(format!("failed to create parent dir for {filename}"))?;
        }
        std::fs::write(&path, content)
            .context(format!("failed to write {filename}"))?;
    }

    let pretty_path = format!("~/.rodeo/typedefs/{version}");
    tracing::info!("Wrote type definitions to {pretty_path}");

    // Update .rodeo/.luaurc in the current project
    let luaurc_path = PathBuf::from(".rodeo/.luaurc");
    let alias_value = format!("{pretty_path}");

    let mut rc: serde_json::Value = if luaurc_path.exists() {
        let content = std::fs::read_to_string(&luaurc_path)
            .context("failed to read .rodeo/.luaurc")?;
        serde_json::from_str(&content)
            .context("failed to parse .rodeo/.luaurc")?
    } else {
        std::fs::create_dir_all(".rodeo")
            .context("failed to create .rodeo directory")?;
        serde_json::json!({})
    };

    let aliases = rc
        .as_object_mut()
        .context(".luaurc is not a JSON object")?
        .entry("aliases")
        .or_insert_with(|| serde_json::json!({}));

    aliases
        .as_object_mut()
        .context("aliases is not a JSON object")?
        .insert("rodeo".to_string(), serde_json::Value::String(alias_value));

    let formatted = serde_json::to_string_pretty(&rc)
        .context("failed to serialize .luaurc")?;
    std::fs::write(&luaurc_path, formatted.as_bytes())
        .context("failed to write .rodeo/.luaurc")?;

    tracing::info!("Updated {}", luaurc_path.display());

    let version = semver::Version::parse(version).context("the crate version is not semver")?;
    for dir in SKILL_DIRS {
        install_skill(Path::new(dir), &version)?;
    }

    Ok(())
}

/// What `setup` does with a project's copy of the skill.
#[derive(Debug, PartialEq)]
enum SkillAction {
    /// There's no copy yet.
    Install,
    /// The copy is for an older rodeo (or says for none: it predates the
    /// stamp), so this build's replaces it.
    Update { from: Option<String> },
    /// The copy is for this rodeo or a newer one, so it stays.
    Keep { newer: Option<String> },
}

fn skill_action(existing: Option<&str>, version: &semver::Version) -> SkillAction {
    let Some(existing) = existing else {
        return SkillAction::Install;
    };
    let Some(stamp) = skill_version(existing) else {
        return SkillAction::Update { from: None };
    };
    match semver::Version::parse(&stamp) {
        Ok(theirs) if theirs > *version => SkillAction::Keep { newer: Some(stamp) },
        Ok(theirs) if theirs == *version => SkillAction::Keep { newer: None },
        _ => SkillAction::Update { from: Some(stamp) },
    }
}

/// The `metadata.version` a skill file's frontmatter carries.
fn skill_version(skill: &str) -> Option<String> {
    let frontmatter = skill.strip_prefix("---")?;
    let end = frontmatter.find("\n---")?;
    frontmatter[..end]
        .lines()
        .find_map(|line| line.trim().strip_prefix("version:").map(|v| v.trim().to_string()))
}

/// Installs the skill as `<dir>/rodeo/SKILL.md` if `dir` exists.
fn install_skill(dir: &Path, version: &semver::Version) -> Result<()> {
    if !dir.is_dir() {
        return Ok(());
    }
    let path = dir.join("rodeo").join("SKILL.md");
    // Forward slashes on every platform, as the folder was named.
    let shown = format!("{}/rodeo/SKILL.md", dir.to_string_lossy().replace('\\', "/"));
    let existing = std::fs::read_to_string(&path).ok();
    let action = skill_action(existing.as_deref(), version);
    match &action {
        SkillAction::Keep { newer: None } => {
            tracing::info!("The rodeo skill in {shown} is up to date");
            return Ok(());
        }
        SkillAction::Keep { newer: Some(theirs) } => {
            tracing::info!("Left the rodeo skill in {shown} as it is: it's for rodeo {theirs}, newer than this one ({version})");
            return Ok(());
        }
        _ => {}
    }
    std::fs::create_dir_all(path.parent().expect("has a parent"))
        .with_context(|| format!("failed to create the folder for {shown}"))?;
    std::fs::write(&path, SKILL).with_context(|| format!("failed to write {shown}"))?;
    match action {
        SkillAction::Update { from: Some(old) } => tracing::info!("Updated the rodeo skill in {shown} ({old} → {version})"),
        SkillAction::Update { from: None } => tracing::info!("Updated the rodeo skill in {shown} (an unversioned copy → {version})"),
        _ => tracing::info!("Installed the rodeo skill to {shown}"),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(s: &str) -> semver::Version {
        semver::Version::parse(s).unwrap()
    }

    fn skill_for(version: &str) -> String {
        format!("---\nname: rodeo\nmetadata:\n  version: {version}\n---\n\n# rodeo\n")
    }

    #[test]
    fn the_embedded_skill_is_for_this_build() {
        assert_eq!(skill_version(SKILL).as_deref(), Some(env!("CARGO_PKG_VERSION")));
    }

    #[test]
    fn reads_the_frontmatter_version() {
        assert_eq!(skill_version(&skill_for("1.5.0-rc.14")).as_deref(), Some("1.5.0-rc.14"));
        assert_eq!(skill_version("---\r\nmetadata:\r\n  version: 1.3.0\r\n---\r\n").as_deref(), Some("1.3.0"));
        assert_eq!(skill_version("---\nname: rodeo\n---\nversion: 2.0.0\n"), None);
        assert_eq!(skill_version("# rodeo\n"), None);
    }

    #[test]
    fn replaces_only_an_older_or_unversioned_copy() {
        let this = v("1.5.0-rc.15");
        assert_eq!(skill_action(None, &this), SkillAction::Install);
        assert_eq!(skill_action(Some("# rodeo\n"), &this), SkillAction::Update { from: None });
        assert_eq!(
            skill_action(Some(&skill_for("1.3.0")), &this),
            SkillAction::Update { from: Some("1.3.0".into()) }
        );
        assert_eq!(
            skill_action(Some(&skill_for("1.5.0-rc.14")), &this),
            SkillAction::Update { from: Some("1.5.0-rc.14".into()) }
        );
        assert_eq!(skill_action(Some(&skill_for("1.5.0-rc.15")), &this), SkillAction::Keep { newer: None });
        assert_eq!(
            skill_action(Some(&skill_for("1.5.0")), &this),
            SkillAction::Keep { newer: Some("1.5.0".into()) }
        );
    }

    #[test]
    fn installs_only_into_existing_skill_folders() {
        let root = std::env::temp_dir().join(format!("rodeo-setup-skill-{}", uuid::Uuid::new_v4()));
        let claude = root.join(".claude").join("skills");
        let agents = root.join(".agents").join("skills");
        std::fs::create_dir_all(&claude).unwrap();
        let this = v(env!("CARGO_PKG_VERSION"));

        install_skill(&claude, &this).unwrap();
        install_skill(&agents, &this).unwrap();

        assert_eq!(std::fs::read_to_string(claude.join("rodeo").join("SKILL.md")).unwrap(), SKILL);
        assert!(!root.join(".agents").exists());
        let _ = std::fs::remove_dir_all(&root);
    }
}
