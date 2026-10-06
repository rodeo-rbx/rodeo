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
use std::time::{Duration, SystemTime};

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
    /// The last sign-in failed (e.g. the saved sign-in expired), or Studio had
    /// no saved sign-in. Studio is showing its sign-in prompt and loads no
    /// plugins until someone signs in.
    Failed,
}

/// Sign-in state from Studio's `[FLog::StudioKeyEvents] login` lines and its
/// sign-in prompt (`[FLog::QuickSignInUtil] Starting awaitQuickSignIn`, logged
/// only when the prompt shows); the last one decides. After a failure, a later
/// sign-in logs only its `[end]` line.
pub fn sign_in_state(log: &str) -> SignIn {
    let mut state = SignIn::NotStarted;
    for line in log.lines() {
        if line.contains("[FLog::QuickSignInUtil] Starting awaitQuickSignIn") {
            state = SignIn::Failed;
            continue;
        }
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

/// Whether killing Studio now could lose its sign-in. Signing in can make
/// Roblox replace the stored token, revoking the old one; a Studio killed
/// before the replacement reaches disk leaves only the revoked token, and the
/// next Studio comes up signed out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Credential {
    /// No sign-in logged yet; one may be about to start.
    NotSignedIn,
    /// A sign-in is under way.
    SigningIn,
    /// Studio stored a renewed token in its cookie store at `at`, as the
    /// record `name`. Studio writes the store to disk lazily, so the token is
    /// safe only once [`super::cookies`] shows that record on disk.
    Renewed { name: String, at: SystemTime },
    /// Signed in (or failed to) without renewing the token.
    Settled,
}

pub fn credential_state(log: &str) -> Credential {
    match sign_in_state(log) {
        SignIn::InProgress => return Credential::SigningIn,
        SignIn::NotStarted => return Credential::NotSignedIn,
        SignIn::Succeeded | SignIn::Failed => {}
    }
    log.lines()
        .filter_map(|line| {
            // `... CookieKeyValueStorage: Saving 857 bytes to key https://www.roblox.com/RobloxStudioAuth/oauth2RefreshToken<user id>.`
            let (_, key) = line.split_once("CookieKeyValueStorage: Saving ")?.1.split_once(" to key ")?;
            let (_, host_and_path) = key.trim_end().trim_end_matches('.').split_once("://")?;
            let name = &host_and_path[host_and_path.find('/')?..];
            if !name.contains("/oauth2RefreshToken") {
                return None;
            }
            Some(Credential::Renewed { name: name.to_string(), at: line_time(line)? })
        })
        .last()
        .unwrap_or(Credential::Settled)
}

/// A log line's leading UTC timestamp (`2026-10-05T20:44:12.784Z,...`).
fn line_time(line: &str) -> Option<SystemTime> {
    let stamp = line.get(..24)?;
    let num = |range: std::ops::Range<usize>| stamp.get(range)?.parse::<i64>().ok();
    let (y, mo, d) = (num(0..4)?, num(5..7)?, num(8..10)?);
    let (h, mi, s, ms) = (num(11..13)?, num(14..16)?, num(17..19)?, num(20..23)?);
    // Days since 1970-01-01 for a proleptic Gregorian date (Howard Hinnant's days_from_civil).
    let (y, mo) = if mo <= 2 { (y - 1, mo + 9) } else { (y, mo - 3) };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * mo + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    let millis = ((days * 24 + h) * 60 + mi) * 60_000 + s * 1000 + ms;
    Some(SystemTime::UNIX_EPOCH + Duration::from_millis(u64::try_from(millis).ok()?))
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
    fn the_sign_in_prompt_counts_as_failed_until_someone_signs_in() {
        // With no saved sign-in, Studio shows the prompt without logging a failure.
        const PROMPT: &str = "2026-10-06T05:21:12.003Z,1.003510,7013b000,6,Info [FLog::QuickSignInUtil] Starting awaitQuickSignIn via polling: code=MHJRYJ";
        assert_eq!(sign_in_state(&[START, PROMPT].join("\n")), SignIn::Failed);
        assert_eq!(sign_in_state(&[START, PROMPT, SUCCESS].join("\n")), SignIn::Succeeded);
    }

    #[test]
    fn credential_tracks_sign_in_and_token_renewal() {
        const SAVE: &str = "2026-10-05T20:44:12.784Z,0.731000,6d7eb000,6,Debug [FLog::KeyValueStorage] CookieKeyValueStorage: Saving 857 bytes to key https://www.roblox.com/RobloxStudioAuth/oauth2RefreshToken902015375.";
        const READ: &str = "2026-10-05T20:08:42.615Z,0.615344,f5163f80,6,Debug [FLog::KeyValueStorage] CookieKeyValueStorage: Reading from key https://www.roblox.com/RobloxStudioAuth/oauth2RefreshToken902015375.";
        assert_eq!(credential_state(OTHER), Credential::NotSignedIn);
        assert_eq!(credential_state(&[START, READ].join("\n")), Credential::SigningIn);
        assert_eq!(credential_state(&[START, SAVE].join("\n")), Credential::SigningIn);
        assert_eq!(credential_state(&[START, READ, SUCCESS].join("\n")), Credential::Settled);
        assert_eq!(
            credential_state(&[START, SAVE, SUCCESS].join("\n")),
            Credential::Renewed {
                name: "/RobloxStudioAuth/oauth2RefreshToken902015375".to_string(),
                at: SystemTime::UNIX_EPOCH + Duration::from_millis(1_791_233_052_784),
            },
        );
    }

    #[test]
    fn parses_log_timestamps_as_utc() {
        assert_eq!(line_time("1970-01-01T00:00:00.000Z,0.0"), Some(SystemTime::UNIX_EPOCH));
        // 2026-10-05T20:44:12.784Z = 1791233052.784 (date -u -j -f %Y-%m-%dT%H:%M:%S 2026-10-05T20:44:12 +%s)
        assert_eq!(
            line_time("2026-10-05T20:44:12.784Z,0.731000,6d7eb000"),
            Some(SystemTime::UNIX_EPOCH + Duration::from_millis(1_791_233_052_784)),
        );
        assert_eq!(line_time("not a timestamp"), None);
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
