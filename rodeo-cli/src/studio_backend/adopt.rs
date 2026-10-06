//! Taking over a `--detach` Studio whose launching serve has exited (#35).
//!
//! `rodeo run --place --detach` with no serve on its port starts a serve of
//! its own, which exits with the run and leaves the Studio up. When that
//! Studio's plugin reconnects to a later serve on the port, it reports the
//! launch session from its RunScript bootstrap — one this serve never
//! launched, so `rodeo kill` refused it and `rodeo state` had no paths for
//! it. The Studio's command line names that bootstrap, which is how the
//! serve finds the process to take over.

use std::path::{Path, PathBuf};
use std::time::Duration;

use rodeo_proto as proto;

use super::plugin_sweep::{bootstrap_paths, port_from_bootstrap};
use crate::master::SharedBackendState;

/// How often an adopted Studio's pid is checked: there is no process handle
/// to wait on.
const EXIT_POLL: Duration = Duration::from_secs(1);

/// A running Studio launched for a session by a backend on this port.
#[derive(Debug, PartialEq)]
pub struct Detached {
    pub pid: u32,
    /// Its RunScript bootstrap, `<cwd>/.rodeo/.temp/rodeo-bootstrap-<session>.luau`.
    pub bootstrap: PathBuf,
    /// The place file it has open (`-localPlaceFile`); `None` for a place-id
    /// launch.
    pub working_path: Option<String>,
}

/// The running Studio launched for `session_guid` by a backend on `port` —
/// pass this backend's own — if exactly one is.
pub fn find(session_guid: &str, port: u16) -> Option<Detached> {
    let app = rbx_control::studio::launch::studio_application_path().ok()?;
    let instances = launch_control::running_instances(Path::new(&app));
    find_in(&instances, session_guid, port, |path| std::fs::read_to_string(path).ok())
}

/// [`find`] over `(pid, command line)` pairs, reading bootstraps with `read`.
fn find_in(
    instances: &[(u32, String)],
    session_guid: &str,
    port: u16,
    read: impl Fn(&Path) -> Option<String>,
) -> Option<Detached> {
    // Sessions are master-minted UUIDs; anything else can't name a bootstrap.
    if session_guid.is_empty() || !session_guid.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
        return None;
    }
    let bootstrap_name = format!("rodeo-bootstrap-{session_guid}.luau");
    let mut launches = instances.iter().filter_map(|(pid, cmdline)| {
        // The edit Studio rodeo launched, not a play-test process it spawned.
        if !cmdline.contains("-task RunScript") {
            return None;
        }
        let bootstrap = bootstrap_paths(std::slice::from_ref(cmdline)).pop()?;
        // Split by hand: Path only knows this platform's separators.
        let file_name = bootstrap.to_str()?.rsplit(['/', '\\']).next()?;
        (file_name == bootstrap_name).then_some((*pid, cmdline, bootstrap))
    });
    let (pid, cmdline, bootstrap) = launches.next()?;
    // Two processes claiming one launch can't be told apart: take neither.
    if launches.next().is_some() {
        return None;
    }
    // The session is only what the connecting plugin says it is. What makes
    // the Studio ours is that its bootstrap stamps this backend's port: a
    // Studio launched for another backend stays that backend's.
    if port_from_bootstrap(&read(&bootstrap)?)? != port {
        return None;
    }
    Some(Detached { pid, bootstrap, working_path: local_place_file(cmdline) })
}

/// The place file on a launch command line: `-localPlaceFile <path>`, which
/// rodeo passes right before `-runScriptFile`. Paths may contain spaces, and
/// Windows command lines quote them.
fn local_place_file(cmdline: &str) -> Option<String> {
    let re = regex::Regex::new(r#"-localPlaceFile\s+"?(.+?)"?\s+-runScriptFile\s"#).expect("static regex");
    Some(re.captures(cmdline)?[1].to_string())
}

/// Drop an adopted Studio's row once its process is gone, as the exit
/// handler of a launched one would. `rodeo kill` removes the row itself;
/// this covers a Studio closed by hand.
pub fn watch_exit(state: SharedBackendState, session_guid: String, pid: u32) {
    tokio::spawn(async move {
        while rbx_control::pid_alive(pid) {
            tokio::time::sleep(EXIT_POLL).await;
        }
        let mut guard = state.lock().await;
        if guard.studio_instances.remove(&session_guid).is_none() {
            return;
        }
        tracing::info!(session_guid = session_guid.as_str(), pid, "adopted Studio exited");
        if let Some(ref relay_tx) = guard.relay_tx {
            let _ = relay_tx.send(proto::BackendMessage {
                msg: Some(proto::backend_message::Msg::SessionExited(Box::new(proto::SessionExited {
                    session_guid,
                    reason: "exited".to_string(),
                    ..Default::default()
                }))),
                ..Default::default()
            });
        }
        if let Some(ref notify) = guard.snapshot_trigger {
            notify.notify_one();
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    const SESSION: &str = "2b1c4e5f-0a1b-4c2d-8e3f-123456789abc";
    const PORT: u16 = 47711;

    fn launch(session: &str, place: &str) -> String {
        format!(
            "/Applications/RobloxStudio.app/Contents/MacOS/RobloxStudio -task RunScript -localPlaceFile {place} \
             -runScriptFile /Users/me/My Game/.rodeo/.temp/rodeo-bootstrap-{session}.luau -parentPid 42 \
             -ApplePersistenceIgnoreState YES"
        )
    }

    fn bootstrap_for(port: u16) -> impl Fn(&Path) -> Option<String> {
        move |_| Some(format!("local ws = game:GetService(\"Workspace\")\nws:SetAttribute(\"rodeoPort\", {port})\n"))
    }

    #[test]
    fn finds_the_launch_by_its_bootstrap() {
        let instances = vec![
            (100, "/Applications/RobloxStudio.app/Contents/MacOS/RobloxStudio".to_string()),
            (200, launch("0000aaaa-other", "/tmp/other.rbxl")),
            (300, launch(SESSION, "/Users/me/My Game/.rodeo/.temp/rodeo-9f.rbxl")),
        ];
        let found = find_in(&instances, SESSION, PORT, bootstrap_for(PORT)).expect("adopted");
        assert_eq!(found.pid, 300);
        assert_eq!(found.working_path.as_deref(), Some("/Users/me/My Game/.rodeo/.temp/rodeo-9f.rbxl"));
        assert!(found.bootstrap.ends_with(format!("rodeo-bootstrap-{SESSION}.luau")));
    }

    #[test]
    fn a_studio_launched_for_another_port_is_not_ours() {
        let instances = vec![(300, launch(SESSION, "/tmp/x.rbxl"))];
        assert_eq!(find_in(&instances, SESSION, PORT, bootstrap_for(PORT + 2)), None);
    }

    #[test]
    fn a_missing_bootstrap_adopts_nothing() {
        let instances = vec![(300, launch(SESSION, "/tmp/x.rbxl"))];
        assert_eq!(find_in(&instances, SESSION, PORT, |_| None), None);
    }

    #[test]
    fn two_processes_with_one_bootstrap_adopt_neither() {
        let instances = vec![(300, launch(SESSION, "/tmp/x.rbxl")), (301, launch(SESSION, "/tmp/x.rbxl"))];
        assert_eq!(find_in(&instances, SESSION, PORT, bootstrap_for(PORT)), None);
    }

    #[test]
    fn play_test_processes_are_not_the_launch() {
        let child = format!(
            "/Applications/RobloxStudio.app/Contents/MacOS/RobloxStudio -task StartServer editpid 300 \
             -runScriptFile /tmp/.rodeo/.temp/rodeo-bootstrap-{SESSION}.luau"
        );
        let instances = vec![(300, launch(SESSION, "/tmp/x.rbxl")), (400, child)];
        assert_eq!(find_in(&instances, SESSION, PORT, bootstrap_for(PORT)).map(|d| d.pid), Some(300));
    }

    #[test]
    fn sessions_that_cannot_name_a_bootstrap_are_refused() {
        let instances = vec![(300, launch(SESSION, "/tmp/x.rbxl"))];
        for session in ["", "../../etc/passwd", "a b", "x.luau"] {
            assert_eq!(find_in(&instances, session, PORT, bootstrap_for(PORT)), None, "{session:?}");
        }
    }

    #[test]
    fn place_id_launches_have_no_working_path() {
        let cmdline = format!(
            "RobloxStudio -task RunScript -placeId 1 -universeId 2 -runScriptFile /tmp/.rodeo/.temp/rodeo-bootstrap-{SESSION}.luau"
        );
        let found = find_in(&[(300, cmdline)], SESSION, PORT, bootstrap_for(PORT)).expect("adopted");
        assert_eq!(found.working_path, None);
    }

    #[test]
    fn windows_command_lines_unquote_the_place() {
        let cmdline = format!(
            r#""C:\Program Files\Roblox\RobloxStudioBeta.exe" -task RunScript -localPlaceFile "C:\Users\me\my game\.rodeo\.temp\rodeo-1.rbxl" -runScriptFile "C:\Users\me\my game\.rodeo\.temp\rodeo-bootstrap-{SESSION}.luau""#
        );
        let found = find_in(&[(300, cmdline)], SESSION, PORT, bootstrap_for(PORT)).expect("adopted");
        assert_eq!(found.working_path.as_deref(), Some(r"C:\Users\me\my game\.rodeo\.temp\rodeo-1.rbxl"));
    }
}
