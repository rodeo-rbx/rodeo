//! The Lune and Lute builds the runtime bridge runs (`--adapter`).
//!
//! Each is pinned to the version rodeo's adapters were checked against: the
//! lute adapter's `@std`/`@batteries` and both conformance suites are copied
//! from these releases. `RODEO_LUNE` / `RODEO_LUTE` point at an executable to
//! use instead; otherwise the release zip is downloaded once into
//! `~/.rodeo/runtimes/<runtime>/<version>/`.

use std::io::Read;
use std::path::{Path, PathBuf};

pub struct Runtime {
    pub name: &'static str,
    pub version: &'static str,
    env_override: &'static str,
}

pub const LUNE: Runtime = Runtime { name: "lune", version: "0.10.5", env_override: "RODEO_LUNE" };
pub const LUTE: Runtime = Runtime {
    name: "lute",
    version: "1.0.1-nightly.20260730",
    env_override: "RODEO_LUTE",
};

pub fn by_name(name: &str) -> Option<&'static Runtime> {
    [&LUNE, &LUTE].into_iter().find(|r| r.name == name)
}

impl Runtime {
    fn exe_name(&self) -> String {
        if cfg!(windows) {
            format!("{}.exe", self.name)
        } else {
            self.name.to_string()
        }
    }

    /// The release asset for this platform, if the runtime publishes one.
    fn asset(&self) -> Option<String> {
        let os = match std::env::consts::OS {
            "macos" => "macos",
            "linux" => "linux",
            "windows" => "windows",
            _ => return None,
        };
        let arch = match std::env::consts::ARCH {
            "aarch64" => "aarch64",
            "x86_64" => "x86_64",
            _ => return None,
        };
        match self.name {
            "lune" => Some(format!("lune-{}-{os}-{arch}.zip", self.version)),
            // Lute publishes no Intel macOS or Windows ARM builds.
            _ if (os, arch) == ("macos", "x86_64") || (os, arch) == ("windows", "aarch64") => None,
            _ => Some(format!("lute-{os}-{arch}.zip")),
        }
    }

    fn url(&self, asset: &str) -> String {
        let repo = if self.name == "lune" { "lune-org/lune" } else { "luau-lang/lute" };
        format!("https://github.com/{repo}/releases/download/v{}/{asset}", self.version)
    }

    /// The executable to run: the override, else the cached download
    /// (downloading it first).
    pub async fn executable(&self) -> Result<PathBuf, String> {
        if let Some(path) = std::env::var_os(self.env_override) {
            return Ok(PathBuf::from(path));
        }
        let home = dirs::home_dir().ok_or("could not determine the home directory")?;
        let exe = home
            .join(".rodeo")
            .join("runtimes")
            .join(self.name)
            .join(self.version)
            .join(self.exe_name());
        if exe.is_file() {
            return Ok(exe);
        }
        let asset = self.asset().ok_or_else(|| {
            format!(
                "{} {} has no build for {}-{}; set {} to a {} executable",
                self.name,
                self.version,
                std::env::consts::OS,
                std::env::consts::ARCH,
                self.env_override,
                self.name,
            )
        })?;
        let url = self.url(&asset);
        tracing::info!(runtime = self.name, version = self.version, %url, "downloading the runtime for --adapter");
        let zip = download(&url).await?;
        let exe_name = self.exe_name();
        let target = exe.clone();
        tokio::task::spawn_blocking(move || extract(&zip, &exe_name, &target))
            .await
            .map_err(|e| format!("extracting {asset}: {e}"))??;
        Ok(exe)
    }
}

async fn download(url: &str) -> Result<Vec<u8>, String> {
    let response = reqwest::get(url)
        .await
        .and_then(|r| r.error_for_status())
        .map_err(|e| format!("downloading {url}: {e}"))?;
    let bytes = response.bytes().await.map_err(|e| format!("downloading {url}: {e}"))?;
    Ok(bytes.to_vec())
}

/// Writes the zip's `exe_name` entry to `target`, atomically: a concurrent
/// run extracting the same build renames over it with identical bytes.
fn extract(zip: &[u8], exe_name: &str, target: &Path) -> Result<(), String> {
    let mut archive =
        zip::ZipArchive::new(std::io::Cursor::new(zip)).map_err(|e| format!("reading the zip: {e}"))?;
    let index = (0..archive.len())
        .find(|&i| {
            archive
                .by_index(i)
                .ok()
                .and_then(|f| f.enclosed_name().and_then(|p| p.file_name().map(|n| n == exe_name)))
                .unwrap_or(false)
        })
        .ok_or_else(|| format!("the zip has no {exe_name}"))?;
    let mut contents = Vec::new();
    archive
        .by_index(index)
        .and_then(|mut f| f.read_to_end(&mut contents).map_err(Into::into))
        .map_err(|e| format!("reading {exe_name} from the zip: {e}"))?;

    let dir = target.parent().ok_or("runtime path has no parent")?;
    std::fs::create_dir_all(dir).map_err(|e| format!("creating {}: {e}", dir.display()))?;
    let partial = dir.join(format!(".{exe_name}.{}.partial", uuid::Uuid::new_v4().simple()));
    std::fs::write(&partial, &contents).map_err(|e| format!("writing {}: {e}", partial.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&partial, std::fs::Permissions::from_mode(0o755))
            .map_err(|e| format!("marking {} executable: {e}", partial.display()))?;
    }
    std::fs::rename(&partial, target).map_err(|e| format!("moving the runtime into place: {e}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn assets_and_urls_follow_each_release() {
        assert_eq!(
            LUNE.url("lune-0.10.5-macos-aarch64.zip"),
            "https://github.com/lune-org/lune/releases/download/v0.10.5/lune-0.10.5-macos-aarch64.zip"
        );
        assert_eq!(
            LUTE.url("lute-linux-x86_64.zip"),
            "https://github.com/luau-lang/lute/releases/download/v1.0.1-nightly.20260730/lute-linux-x86_64.zip"
        );
        assert!(by_name("lune").is_some() && by_name("lute").is_some() && by_name("nope").is_none());
    }

    #[test]
    fn extract_writes_the_named_executable() {
        let mut buf = Vec::new();
        {
            let mut zip = zip::ZipWriter::new(std::io::Cursor::new(&mut buf));
            let options: zip::write::SimpleFileOptions = Default::default();
            zip.start_file("README.md", options).unwrap();
            std::io::Write::write_all(&mut zip, b"readme").unwrap();
            zip.start_file("lute", options).unwrap();
            std::io::Write::write_all(&mut zip, b"binary").unwrap();
            zip.finish().unwrap();
        }
        let dir = std::env::temp_dir().join(format!("rodeo-runtimes-test-{}", uuid::Uuid::new_v4()));
        let target = dir.join("lute");
        extract(&buf, "lute", &target).unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"binary");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
