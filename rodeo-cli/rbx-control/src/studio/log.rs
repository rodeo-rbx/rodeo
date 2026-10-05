//! Studio's own log files (see [`crate::paths::roblox_logs_dir`]): find the
//! log a given Studio process writes, and read its sign-in state.
//!
//! Studio names each log after its start time (`<version>_<UTC time>_Studio_<id>_last.log`)
//! and writes its full command line within the first few lines, so a launched
//! process's log is the one whose head contains something unique from its
//! argv. Verified on Studio 0.741 (macOS).

use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// Bytes of each log read when matching its command line (it is line 6).
const HEAD_BYTES: u64 = 8 * 1024;

/// The log of the Studio process whose command line contains `marker` (e.g.
/// its `-runScriptFile` path), looking only at logs in `dir` modified since
/// `since`. `None` if no log matches.
pub fn find_log(dir: &Path, marker: &str, since: SystemTime) -> Option<PathBuf> {
    std::fs::read_dir(dir).ok()?.flatten().find_map(|entry| {
        let path = entry.path();
        let name = path.file_name()?.to_str()?;
        if !name.contains("_Studio_") || !name.ends_with(".log") {
            return None;
        }
        if entry.metadata().ok()?.modified().ok()? < since {
            return None;
        }
        let mut head = String::new();
        File::open(&path).ok()?.take(HEAD_BYTES).read_to_string(&mut head).ok()?;
        head.contains(marker).then_some(path)
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignIn {
    /// No sign-in logged yet.
    NotStarted,
    /// A sign-in started and hasn't ended.
    InProgress,
    Succeeded,
    /// The last sign-in failed (e.g. the saved sign-in expired). Studio is
    /// showing its sign-in prompt and loads no plugins until someone signs in.
    Failed,
}

/// Sign-in state from Studio's `[FLog::StudioKeyEvents] login` lines; the last
/// one decides. After a failure, a later sign-in logs only its `[end]` line.
pub fn sign_in_state(log: &str) -> SignIn {
    let mut state = SignIn::NotStarted;
    for line in log.lines() {
        let Some((_, event)) = line.split_once("[FLog::StudioKeyEvents] login") else { continue };
        state = if event.contains("[end][success]") {
            SignIn::Succeeded
        } else if event.contains("[end]") {
            SignIn::Failed
        } else if event.contains("[start]") {
            SignIn::InProgress
        } else {
            state
        };
    }
    state
}

#[cfg(test)]
mod tests {
    use super::*;

    // Lines from real Studio 0.741 logs (2026-10), trimmed.
    const START: &str = "2026-10-02T22:57:22.572Z,0.572325,f5163f80,6,Info [FLog::StudioKeyEvents] login (automatic) [start]";
    const FAILURE: &str = "2026-10-02T22:57:22.650Z,0.650781,f5163f80,6,Error [FLog::StudioKeyEvents] login [end][failure]";
    const SUCCESS: &str = "2026-10-02T22:58:00.029Z,38.029663,f5163f80,6,Info [FLog::StudioKeyEvents] login [end][success]";
    const OTHER: &str = "2026-10-02T22:57:22.572Z,0.572360,f5163f80,6,Info [FLog::LoginController] LoginController::login with category 'Local'";

    #[test]
    fn sign_in_follows_the_last_login_event() {
        assert_eq!(sign_in_state(OTHER), SignIn::NotStarted);
        assert_eq!(sign_in_state(&[START, OTHER].join("\n")), SignIn::InProgress);
        assert_eq!(sign_in_state(&[START, SUCCESS].join("\n")), SignIn::Succeeded);
        assert_eq!(sign_in_state(&[START, FAILURE, OTHER].join("\n")), SignIn::Failed);
        // Signing in at the prompt after an expired sign-in.
        assert_eq!(sign_in_state(&[START, FAILURE, SUCCESS].join("\n")), SignIn::Succeeded);
    }

    #[test]
    fn finds_the_log_by_command_line() {
        let dir = std::env::temp_dir().join(format!("rbx-control-log-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let since = SystemTime::now() - std::time::Duration::from_secs(5);
        let mine = dir.join("0.741.19.7411056_20261005T194135Z_Studio_8b98a_last.log");
        let other = dir.join("0.741.19.7411056_20261005T194128Z_Studio_8242a_last.log");
        std::fs::write(&mine, "header\n/Applications/RobloxStudio.app/Contents/MacOS/RobloxStudio -task RunScript -runScriptFile /x/rodeo-bootstrap-aaaa.luau\n").unwrap();
        std::fs::write(&other, "header\n/Applications/RobloxStudio.app/Contents/MacOS/RobloxStudio -task RunScript -runScriptFile /x/rodeo-bootstrap-bbbb.luau\n").unwrap();
        std::fs::write(dir.join("not-a-studio.log"), "rodeo-bootstrap-aaaa").unwrap();

        assert_eq!(find_log(&dir, "rodeo-bootstrap-aaaa", since), Some(mine));
        assert_eq!(find_log(&dir, "rodeo-bootstrap-cccc", since), None);
        // Logs older than the launch are skipped.
        let later = SystemTime::now() + std::time::Duration::from_secs(60);
        assert_eq!(find_log(&dir, "rodeo-bootstrap-aaaa", later), None);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
