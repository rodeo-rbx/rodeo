use crate::error::{Error, Result};
use crate::Command;

use std::collections::HashSet;
use std::io;
use std::mem;
use std::process::ExitStatus;
use std::ptr;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use std::os::windows::io::FromRawHandle;

use windows_sys::Win32::Foundation::{
    CloseHandle, SetHandleInformation, BOOL, HANDLE, HANDLE_FLAG_INHERIT, HWND,
    INVALID_HANDLE_VALUE, WAIT_OBJECT_0, WAIT_TIMEOUT,
};
use windows_sys::Win32::Security::SECURITY_ATTRIBUTES;
use windows_sys::Win32::System::Pipes::CreatePipe;
use windows_sys::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
};
use windows_sys::Win32::System::Threading::{
    AttachThreadInput, CreateProcessW, GetCurrentThreadId, GetExitCodeProcess, OpenProcess,
    TerminateProcess, WaitForSingleObject, PROCESS_INFORMATION, STARTF_USESHOWWINDOW,
    STARTF_USESTDHANDLES, STARTUPINFOW,
};
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, SetInformationJobObject,
    JobObjectExtendedLimitInformation, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
    JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    BringWindowToTop, EnumWindows, GetForegroundWindow, GetWindow, GetWindowTextLengthW,
    GetWindowThreadProcessId, IsIconic, IsWindowVisible, SetForegroundWindow, ShowWindow,
    GW_OWNER, SW_RESTORE, SW_SHOWMINNOACTIVE, SW_SHOWNORMAL,
};
use windows_sys::Win32::System::Diagnostics::Debug::ReadProcessMemory;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{keybd_event, KEYEVENTF_KEYUP};
use keyboard_types::{Code, Modifiers};

// Process access rights (literals to avoid feature-gated constant imports).
const PROCESS_TERMINATE: u32 = 0x0001;
const PROCESS_SET_QUOTA: u32 = 0x0100;
const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x1000;
const SYNCHRONIZE: u32 = 0x0010_0000;

/// Serializes keystroke injection process-wide. `keybd_event` writes to the
/// single global input queue and requires the target window foregrounded, so
/// two threads injecting concurrently (e.g. saving several Studios at once)
/// steal focus from each other and interleave their chords — dropping the
/// keystroke entirely. See `WindowsHandle::send_keystroke`.
static KEYSTROKE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// How long to wait for the real GUI process to appear after a bootstrapper
/// handoff before giving up and tracking the originally-launched process.
const ADOPT_DEADLINE: Duration = Duration::from_secs(30);
/// Grace period before concluding "no handoff happened, the launched process
/// IS the app" — only applies when no same-exe sibling has appeared.
const ADOPT_GRACE: Duration = Duration::from_secs(3);

/// Windows-specific handle wrapping a process HANDLE.
pub(crate) struct WindowsHandle {
    process_handle: HANDLE,
    pid: u32,
}

// HANDLE is Send+Sync safe on Windows (opaque kernel object reference).
unsafe impl Send for WindowsHandle {}
unsafe impl Sync for WindowsHandle {}

impl WindowsHandle {
    pub fn focus(&self) -> Result<()> {
        let hwnd = find_main_window_by_pid(self.pid)
            .ok_or_else(|| Error::Platform("no window found for process".into()))?;

        // Re-request foreground every tick. SetForegroundWindow can silently
        // fail under Windows's focus-stealing-prevention heuristics or when
        // other processes are competing for focus. Repeat until
        // GetForegroundWindow confirms our pid owns it.
        //
        // Menu shortcuts (Ctrl+S, etc.) need our window to be foreground;
        // callers that follow focus() with send_keystroke depend on this.
        let deadline = Instant::now() + Duration::from_secs(5);
        let started = Instant::now();
        let mut ticks = 0u32;
        loop {
            let front_pid = force_foreground(hwnd);
            if front_pid == self.pid {
                tracing::debug!(
                    pid = self.pid,
                    ticks,
                    elapsed_ms = started.elapsed().as_millis() as u64,
                    "focus: confirmed foreground",
                );
                return Ok(());
            }
            ticks += 1;
            if ticks % 5 == 0 {
                tracing::debug!(
                    target_pid = self.pid,
                    front_pid,
                    ticks,
                    "focus: still waiting, another process is foreground",
                );
            }
            if Instant::now() >= deadline {
                tracing::warn!(
                    target_pid = self.pid,
                    front_pid,
                    ticks,
                    elapsed_ms = started.elapsed().as_millis() as u64,
                    "focus: gave up after 5s, another process holds foreground",
                );
                return Err(Error::Platform(format!(
                    "focus: pid {} not foreground after 5s (foreground was pid {})",
                    self.pid, front_pid
                )));
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }

    /// Send a keystroke to the process.
    ///
    /// Windows has no per-process key injection like macOS's `CGEventPostToPid`,
    /// so `keybd_event` injects into the foreground input queue. The target must
    /// therefore be foreground: bring it forward and bail (rather than type into
    /// whatever app *is* foreground) if it won't take focus. Callers that already
    /// `focus()` make the foreground step here a cheap no-op.
    pub fn send_keystroke(&self, code: Code, modifiers: Modifiers) -> Result<()> {
        let vk = code_to_vk(code).ok_or(Error::Unsupported)?;
        let hwnd = find_main_window_by_pid(self.pid)
            .ok_or_else(|| Error::Platform("no window found for process".into()))?;

        // Hold the global keystroke lock across the whole focus→inject→restore
        // so concurrent injections (e.g. saving multiple Studios at once) don't
        // steal focus from each other and interleave chords. Recover from
        // poisoning — a panicked prior injection shouldn't wedge all saves.
        let _inject_guard = KEYSTROKE_LOCK.lock().unwrap_or_else(|e| e.into_inner());

        // Remember the prior foreground so we can hand focus back afterward —
        // macOS's CGEventPostToPid never disturbs the foreground at all, and
        // restoring it here keeps this Windows path as close to that contract as
        // the (foreground-only) SendInput mechanism allows.
        let prev_foreground = unsafe { GetForegroundWindow() };

        // Bring the target to the foreground, retrying briefly —
        // SetForegroundWindow can need several attempts on a cold/contended
        // desktop. This retry MUST live here, under KEYSTROKE_LOCK, so that
        // concurrent saves (several Studios at once) can't steal each other's
        // foreground mid-chord. Callers must therefore NOT pre-focus outside
        // the lock: a focus() race outside the lock reintroduces exactly the
        // foreground-steal that drops Ctrl+S. Foreground is a single
        // system-wide resource, so saves are necessarily serialized here.
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut front_pid = force_foreground(hwnd);
        while front_pid != self.pid && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(50));
            front_pid = force_foreground(hwnd);
        }
        if front_pid != self.pid {
            return Err(Error::Platform(format!(
                "send_keystroke: target pid {} not foreground (was {front_pid})",
                self.pid
            )));
        }
        // Let the just-foregrounded window become input-ready before typing.
        std::thread::sleep(Duration::from_millis(120));

        // Chord: modifiers down → key down → key up → modifiers up (reverse).
        // Space the events out — Studio's Qt event loop must observe the
        // modifier as held when the key arrives; a microsecond burst can drop
        // the modifier so the key registers bare (Ctrl+S → a plain "s", no save).
        let mods = modifier_vks(modifiers);
        let gap = Duration::from_millis(40);
        unsafe {
            for &m in &mods {
                keybd_event(m, 0, 0, 0);
                std::thread::sleep(gap);
            }
            keybd_event(vk, 0, 0, 0);
            std::thread::sleep(gap);
            keybd_event(vk, 0, KEYEVENTF_KEYUP, 0);
            std::thread::sleep(gap);
            for &m in mods.iter().rev() {
                keybd_event(m, 0, KEYEVENTF_KEYUP, 0);
                std::thread::sleep(gap);
            }
        }

        // Keep the target foregrounded briefly before yielding focus.
        // keybd_event events are routed to whichever window is foreground when
        // the raw-input thread drains them — NOT to a private per-window queue.
        // So if we restore the previous foreground (or let the next queued save
        // steal focus) before Studio's event loop has processed the Ctrl+S
        // accelerator, the chord lands on the wrong window and the save never
        // fires (the place mtime never changes → caller times out). This hold
        // runs under KEYSTROKE_LOCK, so the next save still can't preempt it.
        std::thread::sleep(Duration::from_millis(400));

        // Hand focus back to whatever was foreground before — best-effort,
        // keeps the steal invisible.
        if !prev_foreground.is_null() && prev_foreground != hwnd {
            force_foreground(prev_foreground);
        }
        Ok(())
    }

    /// Kill returning io::Error (for Child::kill compatibility).
    pub fn kill_io(&self) -> io::Result<()> {
        // Check if still running first
        let wait_result = unsafe { WaitForSingleObject(self.process_handle, 0) };
        if wait_result != WAIT_TIMEOUT {
            return Ok(()); // already dead
        }
        let ok = unsafe { TerminateProcess(self.process_handle, 1) };
        if ok == 0 {
            Err(io::Error::new(io::ErrorKind::Other, "TerminateProcess failed"))
        } else {
            Ok(())
        }
    }

    /// Check if the process has exited. Returns ExitStatus if exited.
    pub fn try_wait(&self) -> io::Result<Option<ExitStatus>> {
        let wait_result = unsafe { WaitForSingleObject(self.process_handle, 0) };
        if wait_result == WAIT_TIMEOUT {
            return Ok(None); // still running
        }
        if wait_result == WAIT_OBJECT_0 {
            let mut exit_code: u32 = 0;
            let ok = unsafe { GetExitCodeProcess(self.process_handle, &mut exit_code) };
            if ok == 0 {
                return Err(io::Error::new(io::ErrorKind::Other, "GetExitCodeProcess failed"));
            }
            // On Windows, ExitStatus wraps a u32 exit code
            Ok(Some(unsafe { std::mem::transmute::<u32, ExitStatus>(exit_code) }))
        } else {
            Err(io::Error::new(io::ErrorKind::Other, "WaitForSingleObject failed"))
        }
    }
}

impl Drop for WindowsHandle {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.process_handle);
        }
    }
}

pub(crate) fn spawn(cmd: &mut Command) -> Result<crate::Child> {
    let exe_path = &cmd.path;
    if !exe_path.exists() {
        return Err(Error::NotFound(exe_path.display().to_string()));
    }

    // Snapshot existing same-exe PIDs *before* launch so adopt_real_process can
    // distinguish a freshly-spawned editor (after a bootstrapper handoff) from
    // pre-existing instances of the same executable.
    let exe_name = exe_path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    let pre_pids = pids_for_exe(&exe_name);

    // Build command line: "exe_path" arg1 arg2 ...
    let mut cmd_line = format!("\"{}\"", exe_path.display());
    for arg in &cmd.args {
        cmd_line.push(' ');
        if arg.contains(' ') || arg.contains('"') {
            cmd_line.push('"');
            cmd_line.push_str(&arg.replace('"', "\\\""));
            cmd_line.push('"');
        } else {
            cmd_line.push_str(arg);
        }
    }

    // Detached shell-open: launch the app *through* explorer by opening its
    // first argument (e.g. a place file) via the shell. The running shell
    // becomes the app's parent, so it's rooted at a persistent process, escapes
    // any job/parent chain we're in, and — for Roblox Studio — opens as a single
    // process with no bootstrapper handoff. The transient `explorer.exe` we
    // spawn hands the open to the running shell and exits; adoption (below)
    // finds the real app by its exe name. See `Command::shell_open`.
    let via_shell = cmd.detached && cmd.shell_open && !cmd.args.is_empty();
    if via_shell {
        cmd_line = format!("explorer.exe \"{}\"", cmd.args[0]);
    }

    let mut si: STARTUPINFOW = unsafe { mem::zeroed() };
    si.cb = mem::size_of::<STARTUPINFOW>() as u32;
    si.dwFlags = STARTF_USESHOWWINDOW;
    si.wShowWindow = if cmd.background {
        SW_SHOWMINNOACTIVE as u16
    } else {
        SW_SHOWNORMAL as u16
    };

    // Wire up stdout/stderr capture when requested — parity with the macOS
    // pty-backed stdio. The child inherits the (inheritable) pipe write ends via
    // STARTF_USESTDHANDLES + bInheritHandles; we keep + drain the read ends.
    //
    // Shell-open is exempt: explorer won't pass our pipes to the shell-launched
    // app anyway, and STARTF_USESTDHANDLES makes the transient explorer open the
    // app itself instead of delegating to the running shell — which reintroduces
    // the parent tie + bootstrapper handoff we're trying to avoid.
    let want_capture = !via_shell && (cmd.stdout_cfg.is_some() || cmd.stderr_cfg.is_some());
    let mut stdout_read: Option<HANDLE> = None;
    let mut stderr_read: Option<HANDLE> = None;
    let mut write_ends: Vec<HANDLE> = Vec::new();
    if want_capture {
        si.dwFlags |= STARTF_USESTDHANDLES;
        si.hStdInput = ptr::null_mut();
        if cmd.stdout_cfg.is_some() {
            if let Some((r, w)) = unsafe { create_inheritable_pipe() } {
                si.hStdOutput = w;
                stdout_read = Some(r);
                write_ends.push(w);
            }
        }
        if cmd.stderr_cfg.is_some() {
            if let Some((r, w)) = unsafe { create_inheritable_pipe() } {
                si.hStdError = w;
                stderr_read = Some(r);
                write_ends.push(w);
            }
        }
    }

    let mut pi: PROCESS_INFORMATION = unsafe { mem::zeroed() };

    // `Command::detached(true)` → DETACHED_PROCESS + CREATE_NEW_PROCESS_GROUP
    // + CREATE_BREAKAWAY_FROM_JOB.
    // DETACHED_PROCESS: child doesn't inherit the parent's console, so console
    // teardown when parent dies doesn't propagate.
    // CREATE_NEW_PROCESS_GROUP: child is in its own process group, so Ctrl+C/
    // Ctrl+Break delivered to the parent group don't reach it.
    // CREATE_BREAKAWAY_FROM_JOB: job membership is inherited, so when the
    // launching process sits inside a kill-on-close job (rodeo's serve
    // supervisor wraps its children in one), the child would die with that job
    // regardless of any detach handling here. Breakaway needs the job to set
    // JOB_OBJECT_LIMIT_BREAKAWAY_OK; if the enclosing job forbids it,
    // CreateProcessW fails ERROR_ACCESS_DENIED and we retry without the flag —
    // launches must not start failing just because the job disallows escape.
    const DETACHED_PROCESS: u32 = 0x00000008;
    const CREATE_NEW_PROCESS_GROUP: u32 = 0x00000200;
    const CREATE_BREAKAWAY_FROM_JOB: u32 = 0x01000000;
    let creation_flags: u32 = if cmd.detached {
        DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP | CREATE_BREAKAWAY_FROM_JOB
    } else {
        0
    };

    let mut ok = 0;
    for flags in [creation_flags, creation_flags & !CREATE_BREAKAWAY_FROM_JOB] {
        // CreateProcessW may write into the command-line buffer, so each
        // attempt gets its own null-terminated UTF-16 copy.
        let mut wide_cmd: Vec<u16> = cmd_line.encode_utf16().collect();
        wide_cmd.push(0);
        ok = unsafe {
            CreateProcessW(
                ptr::null(),
                wide_cmd.as_mut_ptr(),
                ptr::null(),
                ptr::null(),
                // bInheritHandles: TRUE when capturing so the child inherits
                // the pipe write ends (only those are marked inheritable).
                if want_capture { 1 } else { 0 },
                flags,
                ptr::null(),
                ptr::null(),
                &si,
                &mut pi,
            )
        };
        // Success, or nothing left to strip for the retry (non-detached
        // spawns never set the breakaway bit): done. A failed breakaway
        // attempt (job without BREAKAWAY_OK → ERROR_ACCESS_DENIED) falls
        // through to the jobbed retry — a detached child that dies with the
        // job beats no child at all.
        if ok != 0 || flags & CREATE_BREAKAWAY_FROM_JOB == 0 {
            break;
        }
    }

    if ok == 0 {
        unsafe {
            for w in &write_ends {
                CloseHandle(*w);
            }
            if let Some(r) = stdout_read {
                CloseHandle(r);
            }
            if let Some(r) = stderr_read {
                CloseHandle(r);
            }
        }
        return Err(Error::Platform("CreateProcessW failed".into()));
    }

    // Close thread handle immediately (only need the process handle)
    unsafe {
        CloseHandle(pi.hThread);
    }

    // Close our copies of the pipe write ends — the child holds its own. Keeping
    // them open would prevent the read ends from ever seeing EOF.
    for w in &write_ends {
        unsafe { CloseHandle(*w) };
    }

    // The launched process may be a bootstrapper that relaunches the real GUI
    // app as a separate process and then exits — Roblox Studio's
    // RobloxStudioBeta.exe does exactly this. Adopt the real, surviving editor
    // so try_wait/kill/on_exit track it rather than the bootstrapper (whose
    // exit would otherwise look like a launch failure while the app is up).
    // Apps that don't hand off are returned unchanged.
    let (pid, process_handle) =
        adopt_real_process(&exe_name, &pre_pids, pi.dwProcessId, pi.hProcess, via_shell, &cmd.args);

    // Bind non-detached apps to our lifetime job so the OS tears them down when
    // this process exits — even on an uncatchable TerminateProcess, where no
    // Drop/handler runs. `detached` apps are meant to outlive us, so skip them.
    if !cmd.detached {
        bind_to_lifetime_job(process_handle);
    }

    // Drain each captured pipe through the shared line-channel helpers (the same
    // plumbing macOS uses for its pty masters), exposing ChildStdout/ChildStderr.
    // The bootstrapper inherits the pipe write ends and passes them to the
    // relaunched editor, so the real editor's stdout/stderr flow here too.
    let stdout = stdout_read.map(|r| {
        let file = unsafe { std::fs::File::from_raw_handle(r as _) };
        let (tx, rx) = std::sync::mpsc::channel();
        crate::start_drain_thread(file, tx);
        crate::make_child_stdout(rx)
    });
    let stderr = stderr_read.map(|r| {
        let file = unsafe { std::fs::File::from_raw_handle(r as _) };
        let (tx, rx) = std::sync::mpsc::channel();
        crate::start_drain_thread(file, tx);
        crate::make_child_stderr(rx)
    });

    let exit_state = std::sync::Arc::new(std::sync::Mutex::new(crate::ExitState::default()));
    crate::start_exit_watcher(pid, exit_state.clone());

    Ok(crate::Child {
        pid,
        stdout,
        stderr,
        exit_state,
        inner: WindowsHandle {
            process_handle,
            pid,
        },
    })
}

/// Create an anonymous pipe whose WRITE end is inheritable (handed to the child
/// via STARTF_USESTDHANDLES) and whose READ end stays private to us. Returns
/// `(read, write)`.
unsafe fn create_inheritable_pipe() -> Option<(HANDLE, HANDLE)> {
    let mut sa: SECURITY_ATTRIBUTES = mem::zeroed();
    sa.nLength = mem::size_of::<SECURITY_ATTRIBUTES>() as u32;
    sa.bInheritHandle = 1; // both ends inheritable as created...
    let mut read: HANDLE = ptr::null_mut();
    let mut write: HANDLE = ptr::null_mut();
    if CreatePipe(&mut read, &mut write, &sa, 0) == 0 {
        return None;
    }
    // ...then clear inheritance on the read end so the child doesn't get it.
    SetHandleInformation(read, HANDLE_FLAG_INHERIT, 0);
    Some((read, write))
}

/// Enumerate PIDs of all running processes whose image name matches `exe_name`
/// (case-insensitive, e.g. "RobloxStudioBeta.exe").
fn pids_for_exe(exe_name: &str) -> HashSet<u32> {
    let mut set = HashSet::new();
    if exe_name.is_empty() {
        return set;
    }
    unsafe {
        let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snap == INVALID_HANDLE_VALUE {
            return set;
        }
        let mut entry: PROCESSENTRY32W = mem::zeroed();
        entry.dwSize = mem::size_of::<PROCESSENTRY32W>() as u32;
        if Process32FirstW(snap, &mut entry) != 0 {
            loop {
                let name = wide_to_string(&entry.szExeFile);
                if name.eq_ignore_ascii_case(exe_name) {
                    set.insert(entry.th32ProcessID);
                }
                if Process32NextW(snap, &mut entry) == 0 {
                    break;
                }
            }
        }
        CloseHandle(snap);
    }
    set
}

/// Decode a null-terminated UTF-16 buffer (e.g. PROCESSENTRY32W.szExeFile).
fn wide_to_string(buf: &[u16]) -> String {
    let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..len])
}

/// True if the process behind `handle` is still running.
fn process_alive(handle: HANDLE) -> bool {
    unsafe { WaitForSingleObject(handle, 0) == WAIT_TIMEOUT }
}

/// Open a process handle suitable for try_wait (SYNCHRONIZE + query) and kill
/// (terminate). Returns None if the process can't be opened.
fn open_tracked_handle(pid: u32) -> Option<HANDLE> {
    let h = unsafe {
        OpenProcess(
            // PROCESS_SET_QUOTA is required (alongside PROCESS_TERMINATE) for
            // AssignProcessToJobObject — see `bind_to_lifetime_job`.
            SYNCHRONIZE | PROCESS_TERMINATE | PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SET_QUOTA,
            0,
            pid,
        )
    };
    if h.is_null() {
        None
    } else {
        Some(h)
    }
}

/// Process-global job object that non-detached launched apps are bound to.
///
/// Configured `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`: when **this** process exits
/// — gracefully or via an uncatchable `TerminateProcess` — the OS closes the
/// job handle and tears down every app we launched. This is the Windows
/// analogue of the Unix process-group cleanup callers rely on (a parent dies →
/// its launched apps die). Stored as `usize` because a raw `HANDLE` isn't
/// `Send`/`Sync`; `0` means "job creation failed, no-op".
static LIFETIME_JOB: OnceLock<usize> = OnceLock::new();

fn lifetime_job() -> HANDLE {
    let raw = *LIFETIME_JOB.get_or_init(|| unsafe {
        let job = CreateJobObjectW(ptr::null(), ptr::null());
        if job.is_null() {
            return 0;
        }
        let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = mem::zeroed();
        info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        SetInformationJobObject(
            job,
            JobObjectExtendedLimitInformation,
            &info as *const _ as *const _,
            mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
        );
        job as usize
    });
    raw as HANDLE
}

/// Bind a process to the lifetime job so it dies when this process does.
///
/// Studio's bootstrapper relaunches the real editor as a process that breaks
/// away from any *inherited* job, so the editor escapes the job our own parent
/// put us in. An *explicit* `AssignProcessToJobObject` after adoption is not
/// defeated by breakaway and nests under any pre-existing job on Windows 8+.
/// Best-effort: assignment can still fail (e.g. a job in the hierarchy that
/// disallows nesting) — log and continue rather than fail the launch.
fn bind_to_lifetime_job(process_handle: HANDLE) {
    let job = lifetime_job();
    if job.is_null() {
        return;
    }
    let ok = unsafe { AssignProcessToJobObject(job, process_handle) };
    if ok == 0 {
        tracing::debug!(
            error = ?io::Error::last_os_error(),
            "AssignProcessToJobObject failed; launched app won't be auto-killed when this process exits",
        );
    }
}

/// See [`crate::running_instance_cmdlines`]. Matches processes by image name
/// (the `.exe` file name of `app_path`) and reads each raw command line.
pub(crate) fn running_instance_cmdlines(app_path: &std::path::Path) -> Vec<String> {
    let Some(exe_name) = app_path.file_name().and_then(|n| n.to_str()) else { return Vec::new() };
    pids_for_exe(exe_name).into_iter().filter_map(proc_cmdline).collect()
}

/// Read another process's command line via `NtQueryInformationProcess` +
/// `ReadProcessMemory` of the PEB (x64 offsets: `ProcessParameters` at
/// PEB+0x20, `CommandLine` UNICODE_STRING at params+0x70). Returns `None` if
/// the process is gone, unreadable, or mid-exit — callers treat that as
/// "identity unknown", not as a match.
fn proc_cmdline(pid: u32) -> Option<String> {
    #[repr(C)]
    struct ProcessBasicInformation {
        exit_status: isize,
        peb_base: usize,
        affinity: usize,
        base_priority: isize,
        unique_pid: usize,
        parent_pid: usize,
    }
    #[link(name = "ntdll")]
    extern "system" {
        fn NtQueryInformationProcess(
            handle: HANDLE,
            class: u32,
            info: *mut core::ffi::c_void,
            len: u32,
            ret_len: *mut u32,
        ) -> i32;
    }
    const PROCESS_VM_READ: u32 = 0x0010;
    const PROCESS_QUERY_INFORMATION: u32 = 0x0400;

    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_INFORMATION | PROCESS_VM_READ, 0, pid);
        if handle.is_null() {
            return None;
        }
        // Everything below must CloseHandle on the way out.
        let result = (|| {
            let mut pbi: ProcessBasicInformation = mem::zeroed();
            let mut ret_len = 0u32;
            if NtQueryInformationProcess(
                handle,
                0, // ProcessBasicInformation
                &mut pbi as *mut _ as *mut _,
                mem::size_of::<ProcessBasicInformation>() as u32,
                &mut ret_len,
            ) != 0
                || pbi.peb_base == 0
            {
                return None;
            }
            let read = |addr: usize, buf: *mut u8, len: usize| -> bool {
                let mut got = 0usize;
                ReadProcessMemory(handle, addr as *const _, buf as *mut _, len, &mut got) != 0
                    && got == len
            };
            // PEB+0x20 → RTL_USER_PROCESS_PARAMETERS*
            let mut params_ptr = 0usize;
            if !read(pbi.peb_base + 0x20, &mut params_ptr as *mut _ as *mut u8, 8) || params_ptr == 0 {
                return None;
            }
            // params+0x70 → UNICODE_STRING CommandLine { u16 len, u16 cap, pad, ptr at +8 }
            let mut len_bytes = 0u16;
            if !read(params_ptr + 0x70, &mut len_bytes as *mut _ as *mut u8, 2) || len_bytes == 0 {
                return None;
            }
            let mut buf_ptr = 0usize;
            if !read(params_ptr + 0x78, &mut buf_ptr as *mut _ as *mut u8, 8) || buf_ptr == 0 {
                return None;
            }
            let mut wide = vec![0u16; (len_bytes / 2) as usize];
            if !read(buf_ptr, wide.as_mut_ptr() as *mut u8, len_bytes as usize) {
                return None;
            }
            Some(String::from_utf16_lossy(&wide))
        })();
        CloseHandle(handle);
        result
    }
}

/// How long to require cmdline-verified identity before falling back to an
/// unverified windowed sibling (cmdline unreadable / argless launches only).
const VERIFY_FALLBACK: Duration = Duration::from_secs(5);

/// Resolve the real GUI process to track after a possible bootstrapper handoff.
///
/// Strategy: poll for a same-exe process that is new since launch (not in
/// `pre`, and not the bootstrapper `launched_pid`) and owns a top-level window
/// — that's the relaunched editor. Adopt it. If no sibling ever appears within
/// the grace window, the launched process didn't hand off and IS the app, so
/// track it unchanged. Falls back to the launched process on timeout.
///
/// Concurrency safety (mirrors macOS's argv-verified claim): a sibling is only
/// adopted if its command line contains all of `args` — under concurrent
/// launches the snapshot diff alone happily claims a *different* launcher's
/// editor (observed: two legs tracking the same pid; one leg's teardown then
/// killed the other's Studio mid-run and the third Studio leaked). Siblings
/// whose cmdline is readable but mismatched are never adopted; unreadable ones
/// only after `VERIFY_FALLBACK`, preserving the old behavior for argless
/// launches.
fn adopt_real_process(
    exe_name: &str,
    pre: &HashSet<u32>,
    launched_pid: u32,
    launched_handle: HANDLE,
    via_intermediary: bool,
    args: &[String],
) -> (u32, HANDLE) {
    let start = Instant::now();
    let deadline = start + ADOPT_DEADLINE;
    loop {
        let launched_alive = process_alive(launched_handle);

        // Fast path: our own launched process is alive and owns a window — it
        // IS the editor (no handoff happened). Never consider siblings; under
        // concurrent launches they're other launchers' Studios. (Skipped for
        // shell-open: the launched process is a transient explorer.exe, which
        // always has windows.)
        if !via_intermediary && launched_alive && find_window_by_pid(launched_pid).is_some() {
            return (launched_pid, launched_handle);
        }

        let current = pids_for_exe(exe_name);
        let siblings: Vec<u32> = current
            .iter()
            .copied()
            .filter(|p| !pre.contains(p) && *p != launched_pid)
            .collect();

        // A windowed sibling whose cmdline carries our launch args is our
        // relaunched editor — adopt it. Unverifiable (cmdline unreadable)
        // siblings are only eligible after VERIFY_FALLBACK.
        let editor = siblings
            .iter()
            .copied()
            .filter(|&p| find_window_by_pid(p).is_some())
            .find(|&p| match proc_cmdline(p) {
                Some(cmdline) => args.iter().all(|a| cmdline.contains(a.as_str())),
                None => args.is_empty() || start.elapsed() >= VERIFY_FALLBACK,
            });
        if let Some(editor) = editor {
            if let Some(h) = open_tracked_handle(editor) {
                tracing::info!(
                    bootstrapper_pid = launched_pid,
                    editor_pid = editor,
                    elapsed_ms = start.elapsed().as_millis() as u64,
                    "adopted real editor process after bootstrapper handoff",
                );
                unsafe { CloseHandle(launched_handle) };
                return (editor, h);
            }
        }

        // No handoff: launched process is still alive, no same-exe sibling has
        // appeared, and the grace window has elapsed → it IS the app.
        if launched_alive && siblings.is_empty() && start.elapsed() >= ADOPT_GRACE {
            return (launched_pid, launched_handle);
        }

        // Launched process died with no sibling to adopt → genuine early exit.
        // Hand back the (dead) launched handle so the caller observes the exit.
        // Exception: a shell-open launches a transient `explorer.exe` that exits
        // by design once it hands the open to the running shell — keep polling
        // for the real app (up to the deadline) instead of treating it as a fail.
        if !launched_alive && siblings.is_empty() && !via_intermediary {
            return (launched_pid, launched_handle);
        }

        if Instant::now() >= deadline {
            tracing::warn!(
                launched_pid,
                elapsed_ms = start.elapsed().as_millis() as u64,
                "adopt_real_process timed out; tracking originally-launched process",
            );
            return (launched_pid, launched_handle);
        }
        std::thread::sleep(Duration::from_millis(200));
    }
}


/// Find the first top-level window belonging to the given PID.
fn find_window_by_pid(target_pid: u32) -> Option<HWND> {
    struct FindData {
        target_pid: u32,
        found_hwnd: Option<HWND>,
    }

    unsafe extern "system" fn enum_callback(hwnd: HWND, lparam: isize) -> BOOL {
        let data = &mut *(lparam as *mut FindData);
        let mut window_pid: u32 = 0;
        GetWindowThreadProcessId(hwnd, &mut window_pid);
        if window_pid == data.target_pid {
            data.found_hwnd = Some(hwnd);
            return 0; // stop enumeration
        }
        1 // continue
    }

    let mut data = FindData {
        target_pid,
        found_hwnd: None,
    };

    unsafe {
        EnumWindows(Some(enum_callback), &mut data as *mut _ as isize);
    }

    data.found_hwnd
}

/// Find the process's MAIN top-level window: visible, unowned, and titled.
///
/// `find_window_by_pid` returns the *first* enumerated window, which for Studio
/// (Qt) is a hidden, owned helper (`Qt5159QWindowIcon`, 66x39) — focusing or
/// typing into it does nothing. The editor document window is the visible,
/// unowned, titled one (it carries the place path as its title). Used for
/// focus/keystroke; adoption keeps using the looser any-window check.
fn find_main_window_by_pid(target_pid: u32) -> Option<HWND> {
    struct FindData {
        target_pid: u32,
        found_hwnd: Option<HWND>,
    }

    unsafe extern "system" fn enum_callback(hwnd: HWND, lparam: isize) -> BOOL {
        let data = &mut *(lparam as *mut FindData);
        let mut window_pid: u32 = 0;
        GetWindowThreadProcessId(hwnd, &mut window_pid);
        if window_pid == data.target_pid
            && IsWindowVisible(hwnd) != 0
            && GetWindow(hwnd, GW_OWNER).is_null()
            && GetWindowTextLengthW(hwnd) > 0
        {
            data.found_hwnd = Some(hwnd);
            return 0; // stop enumeration
        }
        1 // continue
    }

    let mut data = FindData {
        target_pid,
        found_hwnd: None,
    };

    unsafe {
        EnumWindows(Some(enum_callback), &mut data as *mut _ as isize);
    }

    data.found_hwnd
}

/// Best-effort bring `hwnd` to the foreground and return the pid that owns the
/// foreground afterward. Briefly attaches our input thread to the current
/// foreground thread — the standard trick to bypass Windows' focus-stealing
/// prevention, so `SetForegroundWindow` actually takes effect from a background
/// (console) process competing with another GUI app for the foreground.
fn force_foreground(hwnd: HWND) -> u32 {
    unsafe {
        let fg = GetForegroundWindow();
        let mut fg_pid = 0u32;
        let fg_thread = if fg.is_null() {
            0
        } else {
            GetWindowThreadProcessId(fg, &mut fg_pid)
        };
        let our_thread = GetCurrentThreadId();
        let attach = fg_thread != 0 && fg_thread != our_thread;
        if attach {
            AttachThreadInput(our_thread, fg_thread, 1);
        }
        // Only change the show state if the window is minimized — and then
        // with SW_RESTORE, which honors a maximized previous state.
        // SW_SHOWNORMAL here would un-maximize whatever window we're
        // foregrounding: with the post-keystroke focus hand-back, that meant
        // every save knocked the user's maximized editor down to its
        // "normal" rect.
        if IsIconic(hwnd) != 0 {
            ShowWindow(hwnd, SW_RESTORE);
        }
        BringWindowToTop(hwnd);
        SetForegroundWindow(hwnd);
        if attach {
            AttachThreadInput(our_thread, fg_thread, 0);
        }
        let front = GetForegroundWindow();
        let mut pid = 0u32;
        if !front.is_null() {
            GetWindowThreadProcessId(front, &mut pid);
        }
        pid
    }
}

/// Map a `keyboard_types::Code` to a Windows virtual-key code (`bVk` for
/// `keybd_event`). Letters and digits map to their ASCII values (VK_A..VK_Z =
/// 0x41..0x5A, VK_0..VK_9 = 0x30..0x39). Returns None for unmapped keys.
fn code_to_vk(code: Code) -> Option<u8> {
    use keyboard_types::Code::*;
    Some(match code {
        KeyA => 0x41, KeyB => 0x42, KeyC => 0x43, KeyD => 0x44, KeyE => 0x45,
        KeyF => 0x46, KeyG => 0x47, KeyH => 0x48, KeyI => 0x49, KeyJ => 0x4A,
        KeyK => 0x4B, KeyL => 0x4C, KeyM => 0x4D, KeyN => 0x4E, KeyO => 0x4F,
        KeyP => 0x50, KeyQ => 0x51, KeyR => 0x52, KeyS => 0x53, KeyT => 0x54,
        KeyU => 0x55, KeyV => 0x56, KeyW => 0x57, KeyX => 0x58, KeyY => 0x59,
        KeyZ => 0x5A,
        Digit0 => 0x30, Digit1 => 0x31, Digit2 => 0x32, Digit3 => 0x33,
        Digit4 => 0x34, Digit5 => 0x35, Digit6 => 0x36, Digit7 => 0x37,
        Digit8 => 0x38, Digit9 => 0x39,
        Enter => 0x0D, Tab => 0x09, Space => 0x20, Escape => 0x1B,
        Backspace => 0x08, Delete => 0x2E,
        _ => return None,
    })
}

/// Virtual-key codes for the held modifiers, in press order.
fn modifier_vks(modifiers: Modifiers) -> Vec<u8> {
    let mut v = Vec::new();
    if modifiers.contains(Modifiers::CONTROL) {
        v.push(0x11); // VK_CONTROL
    }
    if modifiers.contains(Modifiers::SHIFT) {
        v.push(0x10); // VK_SHIFT
    }
    if modifiers.contains(Modifiers::ALT) {
        v.push(0x12); // VK_MENU
    }
    if modifiers.contains(Modifiers::META) {
        v.push(0x5B); // VK_LWIN
    }
    v
}
