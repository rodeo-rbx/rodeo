# launch-control

Cross-platform application automation for Rust. Provides an API isomorphic to `std::process::Command`/`Child` for spawning and controlling GUI applications — with extras for focus control, background launching, keystroke injection, and output capture.

Currently focused on macOS, with Windows support in progress. The goal is a single API surface that works across all desktop platforms for automating arbitrary GUI applications.

## What it does

- **Launch GUI apps** with arguments, URLs, or in background mode
- **Capture stdout/stderr** via PTY pairs (macOS) without blocking the app
- **Focus and activate** windows by PID without requiring the app to be frontmost
- **Send keystrokes** to specific processes without requiring focus (macOS: `CGEventPostToPSN`)
- **Process lifecycle** — `wait()`, `try_wait()`, `kill()`, matching `std::process::Child`

## Usage

The API mirrors `std::process::Command` so it's immediately familiar:

```rust
use launch_control::Command;
use std::process::Stdio;

// Launch an app (like std::process::Command)
let mut child = Command::new("/Applications/Safari.app")
    .arg("https://example.com")
    .spawn()?;

// With output capture
let mut child = Command::new("/Applications/Foo.app")
    .stdout(Stdio::piped())
    .stderr(Stdio::piped())
    .background(true)
    .spawn()?;

// Read captured output
if let Some(ref stdout) = child.stdout {
    let line = stdout.recv_timeout(Duration::from_secs(5))?;
}

// GUI-specific extras
child.focus()?;
child.send_keystroke(Key::S, Modifier::CMD)?;

// Standard process lifecycle
let status = child.wait()?;
```

## API

### `Command` (builder, mirrors `std::process::Command`)

| Method | Description |
|--------|-------------|
| `new(path)` | Create launcher for app at path (`.app` bundle or binary) |
| `arg(s)` / `args(iter)` | Add launch arguments |
| `stdout(Stdio)` / `stderr(Stdio)` | Configure output capture (`Stdio::piped()` to capture) |
| `url(s)` | Open a URL through the app (custom schemes, https, etc.) |
| `background(bool)` | Launch without stealing focus |
| `spawn()` | Launch and return a `Child` handle |

### `Child` (handle, mirrors `std::process::Child`)

| Method / Field | Description |
|----------------|-------------|
| `id()` | OS process ID |
| `kill()` | Force-terminate (`io::Result<()>`) |
| `wait()` | Block until exit (`io::Result<ExitStatus>`) |
| `try_wait()` | Non-blocking exit check (`io::Result<Option<ExitStatus>>`) |
| `stdout` | `Option<ChildStdout>` — implements `Read` + `BufRead` |
| `stderr` | `Option<ChildStderr>` — implements `Read` + `BufRead` |
| `focus()` | Bring app to foreground |
| `send_keystroke(key, modifier)` | Send a keystroke to the process without requiring focus |

## Platform Support

| Feature | macOS | Windows | Linux |
|---------|-------|---------|-------|
| Launch | NSWorkspace | CreateProcessW | - |
| Kill | SIGKILL | TerminateProcess | - |
| Wait / try_wait | kill(pid, 0) poll | WaitForSingleObject | - |
| Focus | NSRunningApplication | SetForegroundWindow | - |
| Keystroke injection | CGEventPostToPSN | - | - |
| Piped output | PTY pairs via /usr/bin/open | - | - |
| Background | NSWorkspace hide | SW_SHOWMINNOACTIVE | - |
| URL launch | NSWorkspace.openURLs | - | - |
