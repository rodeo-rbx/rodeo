//! Windows: an app launched with its output captured inherits the capture
//! pipes and no other handle of the launching process. PowerShell stands in
//! for Roblox Studio, which rodeo launches with its output captured.
#![cfg(windows)]

use std::io::Read;
use std::os::windows::io::FromRawHandle;
use std::process::Stdio;
use std::sync::mpsc;
use std::time::Duration;

use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
use windows_sys::Win32::Security::SECURITY_ATTRIBUTES;
use windows_sys::Win32::System::Pipes::CreatePipe;

const POWERSHELL: &str = r"C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe";

/// A pipe whose ends are both inheritable, as the stdout rodeo's studio
/// backend inherited from whoever ran it is.
fn inheritable_pipe() -> (HANDLE, HANDLE) {
    let sa = SECURITY_ATTRIBUTES {
        nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: std::ptr::null_mut(),
        bInheritHandle: 1,
    };
    let (mut read, mut write): (HANDLE, HANDLE) = (std::ptr::null_mut(), std::ptr::null_mut());
    assert!(unsafe { CreatePipe(&mut read, &mut write, &sa, 0) } != 0, "CreatePipe failed");
    (read, write)
}

#[test]
fn an_app_with_captured_output_keeps_no_other_pipe_open() {
    let (read, write) = inheritable_pipe();
    let mut app = launch_control::Command::new(POWERSHELL)
        .args(["-NoProfile", "-Command", "Start-Sleep 30"])
        .background(true)
        .stdout(Stdio::piped())
        .spawn()
        .expect("launch powershell");
    unsafe { CloseHandle(write) };

    // The write end's only copy is closed, so the read end is at EOF now —
    // unless the app inherited a copy, which it would hold until it exits.
    let mut pipe = unsafe { std::fs::File::from_raw_handle(read as _) };
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(pipe.read(&mut [0u8; 16]).ok());
    });
    let eof = rx.recv_timeout(Duration::from_secs(10));
    let _ = app.kill();
    assert_eq!(eof, Ok(Some(0)), "the app holds a copy of the launching process's pipe");
}
