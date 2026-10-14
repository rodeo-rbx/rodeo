//! Windows: what a non-detached app's job takes down with it, and what it
//! lets go. PowerShell stands in for Roblox Studio: what it starts with
//! `Start-Process` is either a different program (like Studio's installer,
//! which must survive Studio exiting to update) or another PowerShell (an
//! instance of the app itself, like Studio's play server process).
#![cfg(windows)]

use std::path::Path;
use std::sync::{mpsc, Mutex};
use std::time::{Duration, Instant};

const POWERSHELL: &str = r"C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe";
const PING: &str = r"C:\Windows\System32\PING.EXE";

/// The tests launch PowerShell, and adoption treats a new PowerShell from a
/// concurrent test as a possible bootstrapper handoff; run them one at a time.
static SERIAL: Mutex<()> = Mutex::new(());

/// PIDs of running `exe` processes whose command line contains `marker`.
fn pids(exe: &str, marker: &str) -> Vec<u32> {
    launch_control::running_instances(Path::new(exe))
        .into_iter()
        .filter(|(_, cmdline)| cmdline.contains(marker))
        .map(|(pid, _)| pid)
        .collect()
}

fn wait_until(what: &str, mut pred: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(60);
    while !pred() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// Kills whatever a test leaves running, pass or fail.
struct Reap(&'static str, &'static str);

impl Drop for Reap {
    fn drop(&mut self) {
        for pid in pids(self.0, self.1) {
            let _ = std::process::Command::new("taskkill").args(["/F", "/PID", &pid.to_string()]).output();
        }
    }
}

/// Launch PowerShell running `script`: non-detached, in the background.
fn launch(script: &str) -> launch_control::Child {
    launch_control::Command::new(POWERSHELL)
        .args(["-NoProfile", "-Command", script])
        .background(true)
        .spawn()
        .expect("launch powershell")
}

/// Block until `app` exits and every exit handler has run.
fn wait_exited(app: &launch_control::Child) {
    let (tx, rx) = mpsc::channel();
    app.on_exit(move |_| {
        let _ = tx.send(());
    });
    rx.recv_timeout(Duration::from_secs(60)).expect("app to exit");
}

#[test]
fn killing_an_app_takes_what_it_started_with_it() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    const MARKER: &str = "-n 41 -w 1001";
    let _reap = Reap(PING, MARKER);
    let mut app = launch(&format!(
        "Start-Process '{PING}' -ArgumentList '{MARKER} 127.0.0.1' -WindowStyle Hidden; Start-Sleep 60"
    ));
    wait_until("ping to start", || !pids(PING, MARKER).is_empty());
    app.kill().expect("kill");
    wait_until("ping to die with the app", || pids(PING, MARKER).is_empty());
}

#[test]
fn an_app_that_exits_on_its_own_leaves_what_it_started_running() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    const MARKER: &str = "-n 42 -w 1001";
    let _reap = Reap(PING, MARKER);
    let app = launch(&format!(
        "Start-Process '{PING}' -ArgumentList '{MARKER} 127.0.0.1' -WindowStyle Hidden"
    ));
    wait_exited(&app);
    wait_until("ping to start", || !pids(PING, MARKER).is_empty());
    // Our last handle to the job: closing it kills whatever the job still binds.
    drop(app);
    std::thread::sleep(Duration::from_secs(1));
    assert!(!pids(PING, MARKER).is_empty(), "ping died with the app's job");
}

#[test]
fn instances_of_the_app_go_with_it_when_it_exits_on_its_own() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    const MARKER: &str = "launch-control-test-instance";
    let _reap = Reap(POWERSHELL, MARKER);
    // The second instance records that it started: `spawn` can return after
    // the app has exited (adoption waits out a same-exe newcomer that never
    // shows a window), by which time the instance is already gone.
    let started = std::env::temp_dir().join(format!("{MARKER}-{}", std::process::id()));
    let _ = std::fs::remove_file(&started);
    let app = launch(&format!(
        "Start-Process '{POWERSHELL}' -ArgumentList '-NoProfile','-Command','Set-Content ''{}'' up; Start-Sleep 60 # {MARKER}' -WindowStyle Hidden; Start-Sleep 5",
        started.display()
    ));
    wait_exited(&app);
    let instance_started = started.exists();
    let _ = std::fs::remove_file(&started);
    assert!(instance_started, "the second instance never started");
    wait_until("the second instance to die with the app", || pids(POWERSHELL, MARKER).is_empty());
}
