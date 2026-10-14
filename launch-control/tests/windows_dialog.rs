//! Windows: a keystroke for an app whose window a modal dialog blocks reports
//! the dialog instead of typing into it. PowerShell stands in for Roblox
//! Studio, whose "Lighting Technology Migration" dialog swallowed save
//! keystrokes this way.
#![cfg(windows)]

use std::time::{Duration, Instant};

use launch_control::{Code, Modifiers};
use windows_sys::Win32::UI::WindowsAndMessaging::FindWindowW;

const POWERSHELL: &str = r"C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe";

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

#[test]
fn a_keystroke_names_the_dialog_blocking_the_app() {
    let title = format!("Blocking Dialog {}", std::process::id());
    // A main window that opens a modal dialog over itself once shown.
    let script = format!(
        "Add-Type -AssemblyName System.Windows.Forms; \
         $main = New-Object Windows.Forms.Form; $main.Text = 'Main'; \
         $main.Add_Shown({{ $d = New-Object Windows.Forms.Form; $d.Text = '{title}'; [void]$d.ShowDialog($main) }}); \
         [Windows.Forms.Application]::Run($main)"
    );
    let mut app = launch_control::Command::new(POWERSHELL)
        .args(["-NoProfile", "-Command", &script])
        .background(true)
        .spawn()
        .expect("launch powershell");

    // Only once the dialog is up: before it, the keystroke would really type.
    let wide_title = wide(&title);
    let deadline = Instant::now() + Duration::from_secs(60);
    while unsafe { FindWindowW(std::ptr::null(), wide_title.as_ptr()) }.is_null() {
        if Instant::now() >= deadline {
            let _ = app.kill();
            panic!("the dialog never opened");
        }
        std::thread::sleep(Duration::from_millis(100));
    }

    let result = app.send_keystroke(Code::KeyS, Modifiers::CONTROL);
    let _ = app.kill();
    match result {
        Err(launch_control::Error::DialogOpen(reported)) => assert_eq!(reported, title),
        other => panic!("expected the dialog to be reported, got {other:?}"),
    }
}
