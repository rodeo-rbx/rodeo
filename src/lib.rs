//! Cross-platform native app automation for Rust.
//!
//! Launch, focus, control, and capture output from GUI applications programmatically.
//! The API mirrors `std::process::Command`/`Child` for familiarity, with additional
//! GUI-specific methods for window focus, keystroke injection, and background launching.
//!
//! # Example
//!
//! ```no_run
//! use launch_control::Command;
//! use std::process::Stdio;
//!
//! // Simple launch (like std::process::Command)
//! let mut child = Command::new("/Applications/Safari.app")
//!     .arg("https://example.com")
//!     .spawn()?;
//!
//! // With output capture
//! let mut child = Command::new("/Applications/Foo.app")
//!     .stdout(Stdio::piped())
//!     .stderr(Stdio::piped())
//!     .background(true)
//!     .spawn()?;
//!
//! // GUI-specific extras
//! child.focus()?;
//! child.kill()?;
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

mod error;

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod windows;

pub use error::{Error, Result};
pub use keyboard_types::{Code, Modifiers};

use std::ffi::OsStr;
use std::io::{self, BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{ExitStatus, Stdio};
use std::sync::{mpsc, Arc, Mutex};

/// Internal fire-once state for exit notification.
/// Transitions from `Pending(callbacks)` → `Exited(status)` exactly once.
enum ExitState {
    Pending(Vec<Box<dyn FnOnce(ExitStatus) + Send>>),
    Exited(ExitStatus),
}

impl Default for ExitState {
    fn default() -> Self {
        Self::Pending(Vec::new())
    }
}

pub(crate) type ExitStateShared = Arc<Mutex<ExitState>>;

/// Start the OS-native exit watcher for a PID. When the process exits, fire
/// all registered callbacks. Delegates to `parent_exit::on_pid_exit` which
/// uses kqueue on macOS/BSD, waitpid on Linux, and WaitForSingleObject on
/// Windows — single source of truth for process-exit primitives.
///
/// We can't read the real exit code of non-child processes on macOS (NSWorkspace
/// launches are reparented to launchd), so the status passed to callbacks is a
/// synthesized successful exit. Callers that need the real exit code should use
/// `Child::try_wait()` / `wait()` instead.
pub(crate) fn start_exit_watcher(pid: u32, state: ExitStateShared) {
    parent_exit::on_pid_exit(pid, move || {
        let status = synth_exit_status();
        let mut guard = state.lock().unwrap();
        let old = std::mem::replace(&mut *guard, ExitState::Exited(status));
        drop(guard);
        if let ExitState::Pending(callbacks) = old {
            for cb in callbacks {
                cb(status);
            }
        }
    });
}

#[cfg(unix)]
fn synth_exit_status() -> ExitStatus {
    use std::os::unix::process::ExitStatusExt;
    ExitStatus::from_raw(0)
}

#[cfg(windows)]
fn synth_exit_status() -> ExitStatus {
    use std::os::windows::process::ExitStatusExt;
    ExitStatus::from_raw(0)
}

/// If the `tokio` feature is on AND the registering thread has a current Tokio
/// runtime, wrap the callback so it dispatches onto that runtime when the
/// process exits. Otherwise return the callback unchanged.
///
/// Without this, callbacks fire from `parent-exit`'s OS-event thread (kqueue
/// on macOS, waitpid on Linux, WaitForSingleObject on Windows), which has no
/// Tokio context — `tokio::spawn(...)` inside the callback panics with
/// "no reactor running".
fn wrap_with_runtime_dispatch(
    callback: impl FnOnce(ExitStatus) + Send + 'static,
) -> Box<dyn FnOnce(ExitStatus) + Send> {
    #[cfg(feature = "tokio")]
    if let Ok(handle) = tokio::runtime::Handle::try_current() {
        return Box::new(move |status| {
            // `spawn(async move { ... })` puts the callback in async-task
            // context: `tokio::spawn`, `Handle::current`, runtime-aware
            // primitives (`Mutex::lock().await` etc.) all work inside.
            handle.spawn(async move { callback(status) });
        });
    }
    Box::new(callback)
}

/// Builder for launching an application. Mirrors `std::process::Command`.
///
/// On macOS, the path should be an `.app` bundle (e.g. `/Applications/Safari.app`).
/// A binary path inside `.app/Contents/MacOS/` is also accepted and will be resolved
/// to the bundle automatically.
///
/// On Windows, the path should be the executable path.
pub struct Command {
    pub(crate) path: PathBuf,
    pub(crate) args: Vec<String>,
    pub(crate) background: bool,
    /// URL to open via the app (e.g. custom scheme URLs).
    /// When set, uses NSWorkspace.openURLs on macOS instead of openApplicationAtURL.
    pub(crate) url: Option<String>,
    pub(crate) stdout_cfg: Option<Stdio>,
    pub(crate) stderr_cfg: Option<Stdio>,
    /// When true, the launched app survives the calling process's death —
    /// SIGKILL of the parent doesn't take the app with it. See
    /// [`Command::detached`] for the platform-specific implementation notes.
    pub(crate) detached: bool,
    /// Windows-only, detached-only: open the launch through the shell (explorer)
    /// rather than spawning the exe directly. The app is then rooted at the
    /// persistent shell instead of the caller — the Windows analogue of macOS's
    /// NSWorkspace→launchd reparenting. For Roblox Studio this also avoids the
    /// bootstrapper handoff, so it stays a single process we can adopt. Opens
    /// `args[0]` (e.g. a place file) via `explorer.exe`; the app to adopt is
    /// still identified by `path`'s exe name. No-op when not detached / non-Windows.
    pub(crate) shell_open: bool,
}

impl Command {
    /// Create a new command for the application at the given path.
    pub fn new(path: impl AsRef<Path>) -> Self {
        Self {
            path: path.as_ref().to_path_buf(),
            args: Vec::new(),
            background: false,
            url: None,
            stdout_cfg: None,
            stderr_cfg: None,
            detached: false,
            shell_open: false,
        }
    }

    /// Add a single argument.
    pub fn arg(&mut self, arg: impl AsRef<OsStr>) -> &mut Self {
        self.args.push(arg.as_ref().to_string_lossy().to_string());
        self
    }

    /// Add multiple arguments.
    pub fn args(&mut self, args: impl IntoIterator<Item = impl AsRef<OsStr>>) -> &mut Self {
        for a in args {
            self.args.push(a.as_ref().to_string_lossy().to_string());
        }
        self
    }

    /// Open a URL via this application (e.g. custom scheme or `https://` URLs).
    /// On macOS, uses `NSWorkspace.openURLs` to route the URL through the app.
    pub fn url(&mut self, url: impl Into<String>) -> &mut Self {
        self.url = Some(url.into());
        self
    }

    /// If true, launch hidden without taking focus (background mode).
    /// If false, the app is raised on its own display once it has a window
    /// (see [`Child::focus`] for how keyboard focus is decided on macOS).
    /// Default: false.
    pub fn background(&mut self, bg: bool) -> &mut Self {
        self.background = bg;
        self
    }

    /// Configure stdout handling. Use `Stdio::piped()` to capture output.
    pub fn stdout(&mut self, cfg: Stdio) -> &mut Self {
        self.stdout_cfg = Some(cfg);
        self
    }

    /// Configure stderr handling. Use `Stdio::piped()` to capture output.
    pub fn stderr(&mut self, cfg: Stdio) -> &mut Self {
        self.stderr_cfg = Some(cfg);
        self
    }

    /// Decouple the launched app from the calling process. When true, a SIGKILL
    /// (or any other ungraceful exit) of the spawning process leaves the app
    /// running. Mirrors `start_new_session=True` (Python) / `Setsid=true` (Go) /
    /// `DETACHED_PROCESS` (Windows `CreateProcess`) — same idea, unified across
    /// platforms.
    ///
    /// Stdio capture (`Stdio::piped()`) keeps working in detached mode on every
    /// platform: implementation hides the platform-specific machinery
    /// (wrapper process on macOS, native creation flag on Windows, `setsid` on
    /// Linux).
    ///
    /// Explicit `Child::kill()` always kills the app regardless of this flag —
    /// detached only governs implicit propagation when the parent dies.
    pub fn detached(&mut self, detached: bool) -> &mut Self {
        self.detached = detached;
        self
    }

    /// Open a detached launch through the shell (explorer) instead of spawning
    /// the exe directly. See the [`shell_open`](Command::shell_open) field. Only
    /// affects detached launches on Windows; a no-op otherwise.
    pub fn shell_open(&mut self, shell_open: bool) -> &mut Self {
        self.shell_open = shell_open;
        self
    }

    /// Spawn the application and return a handle for lifecycle management.
    pub fn spawn(&mut self) -> Result<Child> {
        platform_spawn(self)
    }
}

/// Handle to a running application process. Mirrors `std::process::Child`.
///
/// GUI-specific methods (`focus`, `send_save_keystroke`) are available in addition
/// to the standard process lifecycle methods.
pub struct Child {
    pid: u32,
    /// Captured stdout (only present when launched with `.stdout(Stdio::piped())`).
    pub stdout: Option<ChildStdout>,
    /// Captured stderr (only present when launched with `.stderr(Stdio::piped())`).
    pub stderr: Option<ChildStderr>,
    /// Shared exit state for `on_exit` callbacks. Transitions once.
    pub(crate) exit_state: ExitStateShared,
    #[cfg(target_os = "macos")]
    inner: Option<macos::MacOSHandle>,
    #[cfg(target_os = "windows")]
    inner: windows::WindowsHandle,
}

impl Child {
    /// The OS process identifier. Mirrors `std::process::Child::id()`.
    pub fn id(&self) -> u32 {
        self.pid
    }

    /// Force-terminate the application. Mirrors `std::process::Child::kill()`.
    pub fn kill(&mut self) -> io::Result<()> {
        #[cfg(target_os = "macos")]
        {
            let pid = self.pid;
            let ret = unsafe { libc::kill(pid as i32, libc::SIGKILL) };
            if ret != 0 {
                let err = io::Error::last_os_error();
                if err.raw_os_error() == Some(libc::ESRCH) {
                    return Ok(()); // already dead
                }
                return Err(err);
            }
            return Ok(());
        }
        #[cfg(target_os = "windows")]
        {
            return self.inner.kill_io();
        }
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        {
            Err(io::Error::new(io::ErrorKind::Unsupported, "unsupported platform"))
        }
    }

    /// Wait for the process to exit. Mirrors `std::process::Child::wait()`.
    ///
    /// On macOS, this polls process liveness since GUI apps launched via NSWorkspace
    /// are not direct children (so `waitpid` is not available).
    pub fn wait(&mut self) -> io::Result<ExitStatus> {
        loop {
            if let Some(status) = self.try_wait()? {
                return Ok(status);
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
    }

    /// Check if the process has exited without blocking. Mirrors `std::process::Child::try_wait()`.
    ///
    /// Returns `Ok(Some(status))` if exited, `Ok(None)` if still running.
    pub fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
        #[cfg(target_os = "macos")]
        {
            return macos::try_wait(self.pid);
        }
        #[cfg(target_os = "windows")]
        {
            return self.inner.try_wait();
        }
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        {
            Err(io::Error::new(io::ErrorKind::Unsupported, "unsupported platform"))
        }
    }

    // -- Exit observation (multi-observer, event-driven, no polling) --

    /// Register a callback that fires when the process exits.
    ///
    /// Event-driven (OS-native primitives — kqueue on macOS, WaitForSingleObject
    /// on Windows). Does not require `&mut self` — multiple callbacks may be
    /// registered from independent code paths, and none interferes with `kill()`
    /// or ownership of the `Child` handle.
    ///
    /// If the process has already exited, the callback fires immediately on the
    /// calling thread. Otherwise it fires on a dedicated watcher thread.
    ///
    /// **With the `tokio` feature enabled**, registration captures the calling
    /// thread's current Tokio runtime handle (if any) and dispatches the
    /// callback through it — making `tokio::spawn(...)` and other runtime-
    /// dependent calls inside the callback work correctly. Without the feature
    /// (or without a current runtime at registration), the callback runs on
    /// the OS-event watcher thread, which has no Tokio context — calling
    /// `tokio::spawn(...)` in that case panics with "no reactor running".
    ///
    /// Callback receives the process's `ExitStatus`. On macOS, apps launched via
    /// NSWorkspace are not direct children, so we cannot read a real exit code;
    /// the status will be a synthesized successful exit. On Windows, the real
    /// `GetExitCodeProcess` value is reported.
    pub fn on_exit(&self, callback: impl FnOnce(ExitStatus) + Send + 'static) {
        let callback = wrap_with_runtime_dispatch(callback);
        let mut state = self.exit_state.lock().unwrap();
        match &mut *state {
            ExitState::Pending(cbs) => cbs.push(callback),
            ExitState::Exited(status) => {
                let status = *status;
                drop(state);
                callback(status);
            }
        }
    }

    // -- GUI-specific extras (not on std::process::Child) --

    /// Bring the application to the front of its own display.
    ///
    /// macOS: keyboard focus moves to the app only when the user's focused
    /// window is on the same display. When it is on another display, the
    /// app's windows are raised where they are and activation is handed
    /// straight back to the previous app (key window only), so typing on
    /// the other display is not interrupted. Windows: `SetForegroundWindow`
    /// (always takes focus).
    pub fn focus(&self) -> Result<()> {
        #[cfg(target_os = "macos")]
        {
            return match self.inner {
                Some(ref inner) => inner.focus(),
                None => Err(Error::Platform("no GUI handle available".into())),
            };
        }
        #[cfg(target_os = "windows")]
        {
            return self.inner.focus();
        }
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        {
            Err(Error::Unsupported)
        }
    }

    /// Activate the application unconditionally, keyboard focus included —
    /// for callers about to inject keystrokes, which only reach the
    /// frontmost app. On macOS returns the pid to pass to `restore_focus`
    /// afterwards when the user was working on a different display; `None`
    /// when nothing needs handing back (and always on Windows).
    pub fn activate(&self) -> Result<Option<u32>> {
        #[cfg(target_os = "macos")]
        {
            return match self.inner {
                Some(ref inner) => inner.activate(),
                None => Err(Error::Platform("no GUI handle available".into())),
            };
        }
        #[cfg(target_os = "windows")]
        {
            return self.inner.focus().map(|()| None);
        }
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        {
            Err(Error::Unsupported)
        }
    }

    /// For the next `window`, undo any activation the app grabs for itself
    /// (macOS; no-op elsewhere). See `MacOSHandle::guard_focus`: an app that
    /// activates itself mid-task — Studio does when a test session starts
    /// or ends — has activation handed straight back to the app the user
    /// was in, unless the user deliberately switched to it. Re-arming
    /// extends the window.
    pub fn guard_focus(&self, window: std::time::Duration) {
        #[cfg(target_os = "macos")]
        {
            if let Some(ref inner) = self.inner {
                inner.guard_focus(window);
            }
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = window;
        }
    }

    /// Hand activation back to `prev` (a pid returned by `activate`),
    /// bringing only its key window forward. No-op off macOS.
    pub fn restore_focus(&self, prev: u32) {
        #[cfg(target_os = "macos")]
        {
            if let Some(ref inner) = self.inner {
                inner.restore_focus(prev);
            }
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = prev;
        }
    }

    /// Press the menu item bound to ⌘+`ch` via the Accessibility API
    /// (macOS only). Targets the process directly with no focus requirement —
    /// more reliable than synthetic keystrokes for menu-bound shortcuts.
    pub fn press_menu_cmd_char(&self, ch: char) -> Result<()> {
        #[cfg(target_os = "macos")]
        {
            return match self.inner {
                Some(ref inner) => inner.press_menu_cmd_char(ch),
                None => Err(Error::Platform("no GUI handle available".into())),
            };
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = ch;
            Err(Error::Unsupported)
        }
    }

    /// Press the menu item with the exact title `item_title` in the menu-bar
    /// menu titled `menu_title` via the Accessibility API (macOS only). For
    /// apps that bind shortcuts internally without exposing the key
    /// equivalent on the menu item (e.g. Qt apps like Roblox Studio).
    pub fn press_menu_item(&self, menu_title: &str, item_title: &str) -> Result<()> {
        #[cfg(target_os = "macos")]
        {
            return match self.inner {
                Some(ref inner) => inner.press_menu_item(menu_title, item_title),
                None => Err(Error::Platform("no GUI handle available".into())),
            };
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = (menu_title, item_title);
            Err(Error::Unsupported)
        }
    }

    /// Read-only probe for a menu item's existence via the Accessibility API
    /// (macOS only). Walks the same path as `press_menu_item` without
    /// pressing — use at launch to pre-warm the target's accessibility
    /// connection (first AX contact with a fresh app can block for seconds).
    pub fn find_menu_item(&self, menu_title: &str, item_title: &str) -> Result<bool> {
        #[cfg(target_os = "macos")]
        {
            return match self.inner {
                Some(ref inner) => inner.find_menu_item(menu_title, item_title),
                None => Err(Error::Platform("no GUI handle available".into())),
            };
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = (menu_title, item_title);
            Err(Error::Unsupported)
        }
    }

    /// Send a keystroke to the process. Does not require the app to be focused.
    ///
    /// Uses `CGEventPostToPSN` on macOS to post directly to the process's event queue.
    pub fn send_keystroke(&self, code: Code, modifiers: Modifiers) -> Result<()> {
        #[cfg(target_os = "macos")]
        {
            return match self.inner {
                Some(ref inner) => inner.send_keystroke(code, modifiers),
                None => Err(Error::Platform("no GUI handle available".into())),
            };
        }
        #[cfg(target_os = "windows")]
        {
            return self.inner.send_keystroke(code, modifiers);
        }
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        {
            let _ = (code, modifiers);
            Err(Error::Unsupported)
        }
    }
}

/// Captured stdout from a launched application. Implements `Read` and `BufRead`.
///
/// Backed by a background drain thread that reads eagerly from the underlying
/// PTY/pipe to prevent the application from blocking when buffers fill.
/// If this value is dropped, the drain thread continues consuming data.
pub struct ChildStdout {
    rx: mpsc::Receiver<String>,
    buf: Vec<u8>,
    pos: usize,
}

impl ChildStdout {
    /// Receive a single line with a timeout. Convenience for callers that need
    /// deadline-based reading (e.g. waiting for startup markers).
    pub fn recv_timeout(&self, timeout: std::time::Duration) -> std::result::Result<String, mpsc::RecvTimeoutError> {
        self.rx.recv_timeout(timeout)
    }

    /// Non-blocking line receive. `Err(Empty)` if no line is queued yet,
    /// `Err(Disconnected)` once the drain thread has exited (EOF).
    pub fn try_recv(&self) -> std::result::Result<String, mpsc::TryRecvError> {
        self.rx.try_recv()
    }
}

impl Read for ChildStdout {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.pos >= self.buf.len() {
            match self.rx.recv() {
                Ok(line) => {
                    self.buf = format!("{line}\n").into_bytes();
                    self.pos = 0;
                }
                Err(_) => return Ok(0), // EOF — drain thread exited
            }
        }
        let n = std::cmp::min(buf.len(), self.buf.len() - self.pos);
        buf[..n].copy_from_slice(&self.buf[self.pos..self.pos + n]);
        self.pos += n;
        Ok(n)
    }
}

impl BufRead for ChildStdout {
    fn fill_buf(&mut self) -> io::Result<&[u8]> {
        if self.pos >= self.buf.len() {
            match self.rx.recv() {
                Ok(line) => {
                    self.buf = format!("{line}\n").into_bytes();
                    self.pos = 0;
                }
                Err(_) => {
                    self.buf.clear();
                    self.pos = 0;
                    return Ok(&[]);
                }
            }
        }
        Ok(&self.buf[self.pos..])
    }

    fn consume(&mut self, amt: usize) {
        self.pos += amt;
    }
}

/// Captured stderr from a launched application. Implements `Read` and `BufRead`.
///
/// Identical to `ChildStdout` — separate type for API clarity.
pub struct ChildStderr {
    rx: mpsc::Receiver<String>,
    buf: Vec<u8>,
    pos: usize,
}

impl ChildStderr {
    pub fn recv_timeout(&self, timeout: std::time::Duration) -> std::result::Result<String, mpsc::RecvTimeoutError> {
        self.rx.recv_timeout(timeout)
    }

    /// Non-blocking line receive. `Err(Empty)` if no line is queued yet,
    /// `Err(Disconnected)` once the drain thread has exited (EOF).
    pub fn try_recv(&self) -> std::result::Result<String, mpsc::TryRecvError> {
        self.rx.try_recv()
    }
}

impl Read for ChildStderr {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.pos >= self.buf.len() {
            match self.rx.recv() {
                Ok(line) => {
                    self.buf = format!("{line}\n").into_bytes();
                    self.pos = 0;
                }
                Err(_) => return Ok(0),
            }
        }
        let n = std::cmp::min(buf.len(), self.buf.len() - self.pos);
        buf[..n].copy_from_slice(&self.buf[self.pos..self.pos + n]);
        self.pos += n;
        Ok(n)
    }
}

impl BufRead for ChildStderr {
    fn fill_buf(&mut self) -> io::Result<&[u8]> {
        if self.pos >= self.buf.len() {
            match self.rx.recv() {
                Ok(line) => {
                    self.buf = format!("{line}\n").into_bytes();
                    self.pos = 0;
                }
                Err(_) => {
                    self.buf.clear();
                    self.pos = 0;
                    return Ok(&[]);
                }
            }
        }
        Ok(&self.buf[self.pos..])
    }

    fn consume(&mut self, amt: usize) {
        self.pos += amt;
    }
}

/// Start a background thread that reads lines and sends to channel.
/// If receiver is dropped, continues draining to prevent pipe blocking.
pub(crate) fn start_drain_thread(reader: impl Read + Send + 'static, tx: mpsc::Sender<String>) {
    std::thread::spawn(move || {
        let reader = BufReader::new(reader);
        for line in reader.lines() {
            match line {
                Ok(l) => { let _ = tx.send(l); }
                Err(_) => break,
            }
        }
    });
}

pub(crate) fn make_child_stdout(rx: mpsc::Receiver<String>) -> ChildStdout {
    ChildStdout { rx, buf: Vec::new(), pos: 0 }
}

pub(crate) fn make_child_stderr(rx: mpsc::Receiver<String>) -> ChildStderr {
    ChildStderr { rx, buf: Vec::new(), pos: 0 }
}

// -- Platform dispatch --

#[cfg(target_os = "macos")]
fn platform_spawn(cmd: &mut Command) -> Result<Child> {
    macos::spawn(cmd)
}

#[cfg(target_os = "windows")]
fn platform_spawn(cmd: &mut Command) -> Result<Child> {
    windows::spawn(cmd)
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn platform_spawn(_cmd: &mut Command) -> Result<Child> {
    Err(Error::Unsupported)
}

/// Entry point for the standalone `launch-control` helper binary. Parses
/// args from `std::env::args().skip(1)` and dispatches to the helper main.
///
/// Most callers use one of three integration modes:
///
/// 1. **Standalone**: deploy the `[[bin]] launch-control` next to your
///    binary; `Command::spawn` resolves it as a sibling of `current_exe`.
///    No code change needed.
///
/// 2. **Subcommand** (recommended for multi-call binaries): expose a
///    hidden subcommand in your own `main` that dispatches to
///    [`run_main_with_args`], then call [`set_helper_invocation`] at
///    startup to point launch-control at your binary + that subcommand.
///    Single binary, no separate helper file.
///
/// 3. **Embed**: `include_bytes!` the helper at compile time, unpack to a
///    cache dir at startup, then call [`set_helper_invocation`] with the
///    unpacked path and empty prefix args. Single binary distribution
///    without restructuring `main`.
///
/// `run_main` itself is for mode 1 — the `[[bin]]` target's `fn main`.
#[cfg(target_os = "macos")]
pub fn run_main() -> ! {
    run_main_with_args(std::env::args().skip(1))
}

#[cfg(not(target_os = "macos"))]
pub fn run_main() -> ! {
    eprintln!("launch-control: helper not supported on this platform");
    std::process::exit(2)
}

/// Same as [`run_main`] but takes the helper's args from an explicit
/// iterator. Lets a consumer's `main` strip its own dispatch arg (e.g.
/// the subcommand name) before handing the rest off to launch-control.
///
/// Diverges — does not return.
#[cfg(target_os = "macos")]
pub fn run_main_with_args(args: impl Iterator<Item = String>) -> ! {
    macos::run_helper_main_with_args(args)
}

#[cfg(not(target_os = "macos"))]
pub fn run_main_with_args(_args: impl Iterator<Item = String>) -> ! {
    eprintln!("launch-control: helper not supported on this platform");
    std::process::exit(2)
}

/// Configure how `Command::spawn` invokes the helper subprocess. When
/// set, spawning runs `<bin> <prefix_args...> <helper-flags...>` instead
/// of looking for a sibling binary or honoring `LAUNCH_CONTROL_BIN`.
///
/// Use this to dispatch the helper as a subcommand of your own binary —
/// see mode 2 in [`run_main`]'s doc. Call once at startup, before any
/// `Command::spawn(...)`. First call wins; later calls are silently
/// ignored.
///
/// # Example
///
/// ```no_run
/// // In your `main`, before `Command::spawn`:
/// let exe = std::env::current_exe()?;
/// launch_control::set_helper_invocation(exe, vec!["__launch-control".into()]);
///
/// // And register the subcommand dispatch at the very top of `main`:
/// if std::env::args().nth(1).as_deref() == Some("__launch-control") {
///     launch_control::run_main_with_args(std::env::args().skip(2));
/// }
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
pub fn set_helper_invocation(bin: std::path::PathBuf, prefix_args: Vec<String>) {
    let _ = HELPER_INVOCATION.set(HelperInvocation { bin, prefix_args });
}

#[derive(Debug, Clone)]
pub(crate) struct HelperInvocation {
    pub(crate) bin: std::path::PathBuf,
    pub(crate) prefix_args: Vec<String>,
}

pub(crate) static HELPER_INVOCATION: std::sync::OnceLock<HelperInvocation> = std::sync::OnceLock::new();

/// Bring an already-running app to the front of its own display by pid —
/// the same display-aware behavior as [`Child::focus`], for apps this
/// process did not launch. macOS only.
pub fn focus_pid(pid: u32) -> Result<()> {
    #[cfg(target_os = "macos")]
    {
        return macos::focus_pid(pid);
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = pid;
        Err(Error::Unsupported)
    }
}
