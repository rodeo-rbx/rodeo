//! Keeping Studio's sign-in intact while rodeo starts and kills Studios.
//!
//! Studio signs in at startup with the refresh token in its cookie store
//! ([`super::cookies`]). When that token is old enough, Studio trades it for a
//! new one and Roblox revokes the old one. Verified on Studio 0.741 (macOS):
//! renewals land 14–20 minutes apart, and the new token reaches disk about
//! 2.3 s after Studio logs saving it. Two ways to lose the sign-in follow:
//!
//! - Killing a Studio before its new token reaches disk leaves only the
//!   revoked token ([`wait_until_settled`] guards the kill).
//! - A second Studio that starts before the first's new token reaches disk
//!   presents the old token, gets "Token has been revoked", and signs the
//!   account out of every Studio ([`LaunchTurn`] makes launches take turns).

use std::fs::File;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use super::{cookies, log};

/// Cookie records holding Studio's refresh tokens, one per account.
const REFRESH_TOKEN_PREFIX: &str = "/RobloxStudioAuth/oauth2RefreshToken";

/// A refresh token younger than this is not renewed at startup (the shortest
/// interval observed between renewals is 14 minutes).
const FRESH_FOR: Duration = Duration::from_secs(12 * 60);

/// Where Studio's cookie store can't be read (Windows), how long after a token
/// renewal to assume Studio wrote it to disk. Measured at 2.4 s on macOS.
const UNVERIFIED_SAVE_WAIT: Duration = Duration::from_secs(5);

/// Longest a launch holds its turn waiting for its Studio's sign-in.
const MAX_TURN: Duration = Duration::from_secs(20);

/// Longest a launch waits for its turn before launching anyway. A turn ends
/// within [`MAX_TURN`] unless the process holding it is stuck.
const MAX_TURN_WAIT: Duration = Duration::from_secs(60);

/// How waiting for a Studio's sign-in to settle ended.
#[derive(Debug)]
pub(crate) enum Settle {
    /// Its sign-in ended and any renewed token is on disk (or this platform
    /// has no Studio logs to check).
    Settled,
    Exited,
    /// Still unsettled after the maximum wait, in this state.
    GaveUp(Option<log::Credential>),
}

/// Wait until the Studio whose command line contains `marker` can't lose its
/// sign-in, polling its log and the cookie store: its sign-in has ended, and
/// any token it renewed is in the store on disk. `on_wait` runs once if it
/// has to wait.
pub(crate) fn wait_until_settled(
    marker: &str,
    spawned_at: SystemTime,
    max_wait: Duration,
    alive: impl Fn() -> bool,
    mut on_wait: impl FnMut(&Option<log::Credential>),
) -> Settle {
    let Some(dir) = crate::paths::roblox_logs_dir() else {
        return Settle::Settled;
    };
    let give_up = Instant::now() + max_wait;
    let mut waiting = false;
    loop {
        if !alive() {
            return Settle::Exited;
        }
        let state = log::find_log(&dir, marker, spawned_at)
            .and_then(|path| std::fs::read_to_string(path).ok())
            .map(|text| log::credential_state(&text));
        let settled = match &state {
            // No log yet, or no finished sign-in: Studio may be renewing the token.
            None | Some(log::Credential::NotSignedIn | log::Credential::SigningIn) => false,
            Some(log::Credential::Renewed { name, at }) => renewed_token_on_disk(name, *at),
            Some(log::Credential::Settled) => true,
        };
        if settled {
            return Settle::Settled;
        }
        if Instant::now() >= give_up {
            return Settle::GaveUp(state);
        }
        if !waiting {
            on_wait(&state);
            waiting = true;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
}

/// Whether the token Studio renewed at `at`, stored as cookie record `name`,
/// is in its cookie store on disk.
fn renewed_token_on_disk(name: &str, at: SystemTime) -> bool {
    match cookies::store_path() {
        // The store keeps whole seconds, so this save's record is created no
        // earlier than `at` rounded down; the previous one is minutes older.
        Some(path) => std::fs::read(path)
            .ok()
            .and_then(|file| cookies::created_at(&file, name))
            .is_some_and(|created| created + Duration::from_secs(1) > at),
        None => SystemTime::now() >= at + UNVERIFIED_SAVE_WAIT,
    }
}

/// Whether a Studio starting now might renew the refresh token: the newest
/// one in the cookie store is at least [`FRESH_FOR`] old. Also true when the
/// store can't be read (Windows) or holds no token, since then it can't be
/// ruled out.
fn renewal_may_be_due() -> bool {
    let newest = cookies::store_path()
        .and_then(|path| std::fs::read(path).ok())
        .and_then(|file| cookies::newest_created_with_prefix(&file, REFRESH_TOKEN_PREFIX));
    newest.is_none_or(|created| created.elapsed().unwrap_or_default() >= FRESH_FOR)
}

/// A launch's turn to sign in: a lock on a file shared by every process of
/// this user, held from spawning a Studio until its sign-in settles. Launches
/// take turns only while a renewal may be due, so one Studio's renewal reaches
/// disk before the next Studio reads the token. While the token is fresh,
/// launches don't wait.
pub struct LaunchTurn {
    /// Locked; closing it ends the turn.
    _lock: File,
}

impl LaunchTurn {
    /// Wait for this launch's turn, if launches must take turns now. `None`
    /// when they needn't: the token is fresh, or another Studio renewed it
    /// while this one waited. Also `None` after [`MAX_TURN_WAIT`], or if the
    /// lock file can't be used; the launch goes ahead.
    pub fn take() -> Option<Self> {
        take_at(&lock_path(), renewal_may_be_due, MAX_TURN_WAIT)
    }

    /// End the turn once the Studio launched in it has settled its sign-in
    /// (see [`wait_until_settled`]), exited, or had [`MAX_TURN`]. Returns at
    /// once; the turn ends on a background thread. Without a log `marker`
    /// the sign-in can't be checked, and the turn ends now.
    pub fn end_after_sign_in(
        self,
        marker: Option<String>,
        spawned_at: SystemTime,
        alive: impl Fn() -> bool + Send + 'static,
    ) {
        let Some(marker) = marker else { return };
        let spawned = std::thread::Builder::new().name("studio-sign-in-turn".into()).spawn(move || {
            let settle = wait_until_settled(&marker, spawned_at, MAX_TURN, alive, |_| {});
            if let Settle::GaveUp(state) = settle {
                tracing::warn!(?state, "Studio's sign-in did not settle; letting the next launch go ahead");
            }
            drop(self);
        });
        if let Err(e) = spawned {
            tracing::warn!(error = %e, "could not wait for Studio's sign-in; ending its launch turn now");
        }
    }
}

fn lock_path() -> PathBuf {
    dirs::cache_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("rbx-control")
        .join("studio-sign-in.lock")
}

fn take_at(path: &Path, due: impl Fn() -> bool, max_wait: Duration) -> Option<LaunchTurn> {
    if !due() {
        return None;
    }
    let file = path
        .parent()
        .map_or(Ok(()), std::fs::create_dir_all)
        .and_then(|()| File::options().create(true).truncate(false).write(true).open(path));
    let file = match file {
        Ok(file) => file,
        Err(e) => {
            tracing::warn!(path = %path.display(), error = %e, "can't open the Studio sign-in lock; launching without it");
            return None;
        }
    };
    let give_up = Instant::now() + max_wait;
    let mut waiting = false;
    loop {
        match file.try_lock() {
            Ok(()) => return Some(LaunchTurn { _lock: file }),
            Err(std::fs::TryLockError::WouldBlock) => {}
            Err(std::fs::TryLockError::Error(e)) => {
                tracing::warn!(path = %path.display(), error = %e, "can't lock the Studio sign-in lock; launching without it");
                return None;
            }
        }
        if Instant::now() >= give_up {
            tracing::warn!("another Studio is still signing in; launching anyway");
            return None;
        }
        if !waiting {
            tracing::info!("waiting for another Studio to finish signing in before launching");
            waiting = true;
        }
        std::thread::sleep(Duration::from_millis(100));
        if !due() {
            tracing::info!("another Studio renewed the sign-in; launching");
            return None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    /// A lock file in a directory of its own, so tests running in parallel
    /// don't remove each other's.
    fn lock_file(test: &str) -> PathBuf {
        std::env::temp_dir()
            .join(format!("rbx-control-sign-in-{}-{test}", std::process::id()))
            .join("studio-sign-in.lock")
    }

    #[test]
    fn launches_take_turns_only_while_a_renewal_is_due() {
        let path = lock_file("turns");
        assert!(take_at(&path, || false, Duration::ZERO).is_none(), "fresh token: no turn");

        let first = take_at(&path, || true, Duration::ZERO).expect("first launch gets the turn");
        assert!(take_at(&path, || true, Duration::ZERO).is_none(), "second launch can't take a held turn");

        // The second launch waits until the first turn ends.
        let started = Instant::now();
        let ender = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(300));
            drop(first);
        });
        let second = take_at(&path, || true, Duration::from_secs(5));
        assert!(second.is_some());
        assert!(started.elapsed() >= Duration::from_millis(300));
        ender.join().unwrap();
        drop(second);
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn a_waiting_launch_goes_ahead_once_the_token_is_renewed() {
        let path = lock_file("renewed");
        let _first = take_at(&path, || true, Duration::ZERO).unwrap();
        let due = Arc::new(AtomicBool::new(true));
        let renewer = {
            let due = due.clone();
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(300));
                due.store(false, Ordering::Relaxed);
            })
        };
        let started = Instant::now();
        assert!(take_at(&path, || due.load(Ordering::Relaxed), Duration::from_secs(5)).is_none());
        assert!(started.elapsed() < Duration::from_secs(5), "went ahead without waiting out the turn");
        renewer.join().unwrap();
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }
}
