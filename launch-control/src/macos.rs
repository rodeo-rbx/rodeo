use crate::error::{Error, Result};
use crate::Command;

use objc2::rc::Retained;
use objc2_app_kit::{
    NSApplicationActivationOptions, NSRunningApplication, NSWorkspace,
    NSWorkspaceOpenConfiguration,
};
use objc2_foundation::{NSArray, NSString, NSURL};

use std::io;
use std::process::ExitStatus;
use std::sync::{mpsc, Mutex};
use std::time::{Duration, Instant};

/// Serializes the snapshot → open → PID-claim window in `spawn_piped`.
/// The lookup uses NSRunningApplication's bundle-ID set diff, which is
/// inherently racy under concurrent spawns of the same bundle: multiple
/// callers see the same "first new PID" and the rest are orphaned (never
/// associated with any callback / Child handle). Holding this lock across
/// the snapshot/launch/claim ensures each caller is paired with a unique
/// new PID.
static SPAWN_PIPED_LOCK: Mutex<()> = Mutex::new(());

/// macOS-specific handle wrapping NSRunningApplication.
pub(crate) struct MacOSHandle {
    app: Retained<NSRunningApplication>,
    pid: u32,
    guard: std::sync::Arc<FocusGuard>,
}

impl MacOSHandle {
    pub(crate) fn new(app: Retained<NSRunningApplication>, pid: u32) -> Self {
        Self { app, pid, guard: Default::default() }
    }

    /// For the next `window`, undo any activation the app grabs for itself.
    /// Studio activates itself when a test session starts or ends, which
    /// pulls keyboard focus away from whatever the user is typing into —
    /// on any display. While the guard is armed, the moment the app becomes
    /// frontmost without a deliberate switch (see
    /// `user_switched_deliberately`), activation is handed back to the app
    /// the user was in — key window only, so the app stays where it was
    /// raised. Re-arming extends the window; a deliberate switch to the app
    /// ends it early.
    pub fn guard_focus(&self, window: Duration) {
        use std::sync::atomic::Ordering;
        let until = Instant::now() + window;
        {
            let mut d = self.guard.deadline.lock().unwrap_or_else(|e| e.into_inner());
            *d = Some(d.map_or(until, |cur| cur.max(until)));
        }
        if self.guard.running.swap(true, Ordering::SeqCst) {
            tracing::debug!(pid = self.pid, window_secs = window.as_secs(), "focus guard: extended");
            return; // already watching; deadline extended above
        }
        tracing::info!(
            pid = self.pid, window_secs = window.as_secs(),
            ax_trusted = ax::is_trusted(), ax_front = ?ax::focused_app_pid(), window_front = ?frontmost_pid_by_window_order(),
            "focus guard: armed"
        );
        let guard = self.guard.clone();
        let pid = self.pid;
        let _ = std::thread::Builder::new()
            .name("launch-control-focus-guard".into())
            .spawn(move || {
                let mut prev = frontmost_pid().filter(|&p| p != pid);
                loop {
                    let deadline = *guard.deadline.lock().unwrap_or_else(|e| e.into_inner());
                    if deadline.map_or(true, |d| Instant::now() >= d) {
                        break;
                    }
                    match frontmost_pid() {
                        Some(p) if p == pid => {
                            if user_switched_deliberately() {
                                tracing::info!(pid, "focus guard: user switched to the app; standing down");
                                break;
                            }
                            match prev {
                                Some(prev_pid) => {
                                    reactivate_key_window_only(prev_pid);
                                    tracing::info!(pid, prev = prev_pid, "focus guard: app activated itself; activation handed back");
                                }
                                None => tracing::debug!(pid, "focus guard: app activated itself; nothing to hand back to"),
                            }
                        }
                        Some(p) => prev = Some(p),
                        None => {}
                    }
                    std::thread::sleep(Duration::from_millis(100));
                }
                guard.running.store(false, Ordering::SeqCst);
            });
    }

    /// Bring the app to the front of its own display. Keyboard focus moves
    /// to it only when the user is already working on that display; when
    /// their focused window is on another display, the app is raised where
    /// it is and activation is handed straight back, so a Studio on the
    /// second monitor never pulls typing away from the editor on the first.
    pub fn focus(&self) -> Result<()> {
        self.focus_relative_to(UserFocus::capture())
    }

    /// `focus` against a user position captured earlier (before a launch,
    /// when the app may since have activated itself on startup).
    pub(crate) fn focus_relative_to(&self, user: UserFocus) -> Result<()> {
        if self.app.isTerminated() {
            return Err(Error::Terminated);
        }
        // Unhide first — activation can fail on hidden apps.
        if self.app.isHidden() {
            self.app.unhide();
        }
        let mine = display::of_app_window(self.pid);
        let same_display = match (mine, user.display) {
            (Some(a), Some(b)) => a == b,
            _ => true, // geometry unknown: behave like a plain activation
        };
        let hand_back = if same_display || display::is_single() {
            None
        } else {
            user.pid.filter(|&p| p != self.pid)
        };
        let activated = frontmost_pid() == Some(self.pid) || self.activate_all_windows();
        match hand_back {
            Some(prev) => {
                // Activation has landed (windows ordered front); now put the
                // user's key window back above them and return their focus.
                reactivate_key_window_only(prev);
                tracing::info!(
                    pid = self.pid, prev, activated, display = ?mine, user_display = ?user.display,
                    "focus: raised on its own display, activation handed back"
                );
            }
            None => tracing::info!(pid = self.pid, activated, display = ?mine, "focus: activated"),
        }
        if !activated && hand_back.is_none() {
            return Err(Error::Platform(format!("focus: pid {} did not become frontmost", self.pid)));
        }
        Ok(())
    }

    /// Activate with every window ordered front, and confirm it. The
    /// AppKit call's return value is unreliable from CLI processes (often
    /// false when it worked), and cooperative activation can also refuse it
    /// outright — so check who is frontmost and fall back to the AX route,
    /// which is honored for any AX-trusted process.
    fn activate_all_windows(&self) -> bool {
        let current = NSRunningApplication::currentApplication();
        self.app.activateFromApplication_options(
            &current,
            NSApplicationActivationOptions::ActivateAllWindows,
        );
        if wait_frontmost(self.pid, Duration::from_millis(400)) {
            return true;
        }
        let ax_err = ax::set_frontmost_app(self.pid);
        let ok = wait_frontmost(self.pid, Duration::from_millis(400));
        tracing::debug!(pid = self.pid, ax_err, ok, "activate: AppKit activation not honored, used AX");
        ok
    }

    /// Activate unconditionally, keyboard focus included — for callers about
    /// to inject keystrokes, which only reach the frontmost app. Returns the
    /// pid to hand activation back to afterwards (`restore_focus`) when the
    /// user was working on a different display, else `None`.
    pub fn activate(&self) -> Result<Option<u32>> {
        if self.app.isTerminated() {
            return Err(Error::Terminated);
        }
        if self.app.isHidden() {
            self.app.unhide();
        }
        let user = UserFocus::capture();
        let mine = display::of_app_window(self.pid);
        let hand_back = match (mine, user.display) {
            (Some(a), Some(b)) if a != b && !display::is_single() => user.pid.filter(|&p| p != self.pid),
            _ => None,
        };
        if frontmost_pid() != Some(self.pid) && !self.activate_all_windows() {
            return Err(Error::Platform(format!("activate: pid {} did not become frontmost", self.pid)));
        }
        Ok(hand_back)
    }

    /// Hand activation back to `prev` (from `activate`), key window only.
    pub fn restore_focus(&self, prev: u32) {
        reactivate_key_window_only(prev);
    }

    /// Send a keystroke directly to this process without requiring focus.
    /// Uses CGEventPostToPSN to post keyboard events to the process's event queue.
    pub fn send_keystroke(&self, code: keyboard_types::Code, modifiers: keyboard_types::Modifiers) -> Result<()> {
        let keycode = code_to_macos_keycode(code)
            .ok_or_else(|| Error::Platform(format!("unsupported key: {code:?}")))?;
        let flags = modifiers_to_cg_flags(modifiers);
        send_keystroke_to_pid(self.pid, keycode, flags)
    }

    /// Press the menu item whose key equivalent is ⌘+`ch` via the
    /// Accessibility API (AXPress). Unlike keystroke injection this targets
    /// the process directly, requires no focus, and is immune to the
    /// Qt event-queue drain problem — the menu action runs even when the
    /// app is backgrounded or hidden.
    pub fn press_menu_cmd_char(&self, ch: char) -> Result<()> {
        ax::press_menu_item_by_cmd_char(self.pid, ch)
    }

    /// Press the menu item with the exact title `item_title` in the menu-bar
    /// menu titled `menu_title` via the Accessibility API (AXPress). For apps
    /// that bind shortcuts internally without exposing AXMenuItemCmdChar.
    pub fn press_menu_item(&self, menu_title: &str, item_title: &str) -> Result<()> {
        ax::press_menu_item_by_title(self.pid, menu_title, item_title)
    }

    /// Read-only probe: walk the menu bar and report whether the item exists.
    /// Useful to pre-warm the target's accessibility connection — the first
    /// AX contact with a freshly-launched app can block for many seconds
    /// while it settles; doing that walk in the background at launch makes
    /// later `press_menu_item` calls respond in milliseconds.
    pub fn find_menu_item(&self, menu_title: &str, item_title: &str) -> Result<bool> {
        ax::find_menu_item_by_title(self.pid, menu_title, item_title)
    }
}

// CoreGraphics FFI for posting keyboard events.
#[allow(non_camel_case_types, non_upper_case_globals, dead_code)]
mod cg_ffi {
    use std::ffi::c_void;

    pub type CGEventRef = *mut c_void;
    pub type CGEventSourceRef = *mut c_void;
    pub type CGEventFlags = u64;
    pub type CGKeyCode = u16;

    #[repr(C)]
    #[derive(Clone, Copy, Debug)]
    pub struct CGPoint { pub x: f64, pub y: f64 }
    #[repr(C)]
    #[derive(Clone, Copy, Debug)]
    pub struct CGSize { pub width: f64, pub height: f64 }
    #[repr(C)]
    #[derive(Clone, Copy, Debug)]
    pub struct CGRect { pub origin: CGPoint, pub size: CGSize }

    // CGEventTapLocation values — where the event enters the event stream.
    // kCGSessionEventTap (1) is the window-server session queue; NSApp of the
    // frontmost app drains it continuously, which is why synthetic Cmd+S via
    // this path triggers Qt shortcuts even when the app was already Active.
    pub const K_CG_HID_EVENT_TAP: u32 = 0;
    pub const K_CG_SESSION_EVENT_TAP: u32 = 1;
    #[allow(dead_code)]
    pub const K_CG_ANNOTATED_SESSION_EVENT_TAP: u32 = 2;

    unsafe extern "C" {
        pub fn CGEventCreateKeyboardEvent(
            source: CGEventSourceRef,
            virtual_key: CGKeyCode,
            key_down: bool,
        ) -> CGEventRef;

        pub fn CGEventSetFlags(event: CGEventRef, flags: CGEventFlags);

        // Post to the session event tap — same path hardware events take.
        // Routes to whichever app is currently frontmost. Use this when the
        // target is already frontmost so NSApp's normal event pump delivers
        // the event (vs CGEventPostToPid which writes to the target's private
        // Carbon queue, drained by Qt only on inactive→active transitions).
        pub fn CGEventPost(tap: u32, event: CGEventRef);

        // Post a CGEvent to a specific process by PID. macOS 10.11+.
        // Preferred over CGEventPostToPSN which requires a Carbon PSN lookup
        // via GetProcessForPID — both deprecated, and the PSN path fails for
        // background-launched processes that haven't registered with Launch
        // Services yet. The trade-off: events land in the target's private
        // Carbon queue, which Qt only drains on activation-state transitions,
        // so this is unreliable when the target is already frontmost.
        pub fn CGEventPostToPid(pid: libc::pid_t, event: CGEventRef);

        pub fn CFRelease(cf: *mut c_void);

        // Display geometry in the global CG space (origin at the top-left of
        // the main display, y down) — the same space AX reports window
        // positions in, so the two compare without any flipping.
        pub fn CGGetActiveDisplayList(max: u32, displays: *mut u32, count: *mut u32) -> i32;
        pub fn CGDisplayBounds(display: u32) -> CGRect;
        // Cursor location in that same space. CGEventCreate(NULL) snapshots
        // the HID state from any thread; NSEvent.mouseLocation would need the
        // main thread and a coordinate flip.
        pub fn CGEventCreate(source: CGEventSourceRef) -> CGEventRef;
        pub fn CGEventGetLocation(event: CGEventRef) -> CGPoint;
        // User-input recency, for telling a self-activation apart from a
        // deliberate switch. State id 0 = kCGEventSourceStateCombinedSessionState.
        pub fn CGEventSourceSecondsSinceLastEventType(state_id: i32, event_type: u32) -> f64;
        pub fn CGEventSourceKeyState(state_id: i32, key: CGKeyCode) -> bool;
    }
    #[link(name = "CoreGraphics", kind = "framework")]
    unsafe extern "C" {
        // On-screen windows, front to back. No TCC gate, unlike AX.
        pub fn CGWindowListCopyWindowInfo(option: u32, relative_to: u32) -> *const c_void;
    }
    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        pub fn CFArrayGetCount(array: *const c_void) -> isize;
        pub fn CFArrayGetValueAtIndex(array: *const c_void, idx: isize) -> *const c_void;
        pub fn CFDictionaryGetValue(dict: *const c_void, key: *const c_void) -> *const c_void;
        pub fn CFNumberGetValue(number: *const c_void, the_type: isize, out: *mut c_void) -> u8;
        pub fn CFStringCreateWithCString(alloc: *const c_void, c_str: *const i8, encoding: u32) -> *const c_void;
    }
    pub const K_CG_WINDOW_LIST_OPTION_ON_SCREEN_ONLY: u32 = 1 << 0;
    pub const K_CG_WINDOW_LIST_EXCLUDE_DESKTOP_ELEMENTS: u32 = 1 << 4;
    pub const K_CG_EVENT_LEFT_MOUSE_DOWN: u32 = 1;
    pub const K_CG_EVENT_RIGHT_MOUSE_DOWN: u32 = 3;
    pub const K_CG_EVENT_OTHER_MOUSE_DOWN: u32 = 25;
    pub const K_VK_COMMAND: CGKeyCode = 0x37;
}

/// Owner pid of the frontmost normal (layer 0) on-screen window — the
/// active app's key window in practice. Public CoreGraphics, no TCC gate,
/// and live: the fallback for processes the AX server will not talk to.
fn frontmost_pid_by_window_order() -> Option<u32> {
    use cg_ffi::*;
    let list = unsafe {
        CGWindowListCopyWindowInfo(
            K_CG_WINDOW_LIST_OPTION_ON_SCREEN_ONLY | K_CG_WINDOW_LIST_EXCLUDE_DESKTOP_ELEMENTS,
            0,
        )
    };
    if list.is_null() {
        return None;
    }
    let key = |name: &'static str| unsafe {
        CFStringCreateWithCString(std::ptr::null(), name.as_ptr() as *const i8, 0x0800_0100)
    };
    let (k_layer, k_pid) = (key("kCGWindowLayer\0"), key("kCGWindowOwnerPID\0"));
    let mut found = None;
    let n = unsafe { CFArrayGetCount(list) };
    for i in 0..n {
        let dict = unsafe { CFArrayGetValueAtIndex(list, i) };
        if dict.is_null() {
            continue;
        }
        let mut layer: i32 = -1;
        let mut pid: i32 = 0;
        let l = unsafe { CFDictionaryGetValue(dict, k_layer) };
        let p = unsafe { CFDictionaryGetValue(dict, k_pid) };
        if l.is_null() || p.is_null() {
            continue;
        }
        unsafe {
            CFNumberGetValue(l, 3, &mut layer as *mut _ as *mut _); // kCFNumberSInt32Type
            CFNumberGetValue(p, 3, &mut pid as *mut _ as *mut _);
        }
        if layer == 0 && pid > 0 {
            found = Some(pid as u32);
            break;
        }
    }
    unsafe {
        CFRelease(k_layer as *mut _);
        CFRelease(k_pid as *mut _);
        CFRelease(list as *mut _);
    }
    found
}

/// Did the user just act in a way that explains an app becoming frontmost —
/// a mouse-down in the last half second (a click on its window) or ⌘ held
/// right now (⌘-Tab)? Typing in another app is deliberately *not* counted:
/// that is exactly what the user is doing when Studio steals focus.
fn user_switched_deliberately() -> bool {
    use cg_ffi::*;
    let recent = |t: u32| unsafe { CGEventSourceSecondsSinceLastEventType(0, t) } < 0.5;
    recent(K_CG_EVENT_LEFT_MOUSE_DOWN)
        || recent(K_CG_EVENT_RIGHT_MOUSE_DOWN)
        || recent(K_CG_EVENT_OTHER_MOUSE_DOWN)
        || unsafe { CGEventSourceKeyState(0, K_VK_COMMAND) }
}

/// Shared state of a handle's focus guard (see `MacOSHandle::guard_focus`).
#[derive(Default)]
pub(crate) struct FocusGuard {
    deadline: Mutex<Option<Instant>>,
    running: std::sync::atomic::AtomicBool,
}

/// Which display a point, an app's front window, or the cursor is on.
/// Ids are CGDirectDisplayIDs; nothing here needs the main thread.
mod display {
    use super::cg_ffi::*;

    fn active() -> Vec<(u32, CGRect)> {
        let mut ids = [0u32; 16];
        let mut count = 0u32;
        let err = unsafe { CGGetActiveDisplayList(ids.len() as u32, ids.as_mut_ptr(), &mut count) };
        if err != 0 {
            return Vec::new();
        }
        ids[..count as usize]
            .iter()
            .map(|&id| (id, unsafe { CGDisplayBounds(id) }))
            .collect()
    }

    pub fn for_point(x: f64, y: f64) -> Option<u32> {
        active()
            .into_iter()
            .find(|(_, b)| {
                x >= b.origin.x
                    && x < b.origin.x + b.size.width
                    && y >= b.origin.y
                    && y < b.origin.y + b.size.height
            })
            .map(|(id, _)| id)
    }

    pub fn under_cursor() -> Option<u32> {
        let ev = unsafe { CGEventCreate(std::ptr::null_mut()) };
        if ev.is_null() {
            return None;
        }
        let p = unsafe { CGEventGetLocation(ev) };
        unsafe { CFRelease(ev) };
        for_point(p.x, p.y)
    }

    /// Display holding the centre of the app's focused (else main, else
    /// first) window.
    pub fn of_app_window(pid: u32) -> Option<u32> {
        let (x, y) = super::ax::front_window_center(pid)?;
        for_point(x, y)
    }

    pub fn is_single() -> bool {
        active().len() <= 1
    }
}

/// The user's working position, captured before anything gets activated:
/// the frontmost app and the display holding its focused window (the
/// cursor's display when that app has no window to ask).
#[derive(Clone, Copy, Debug)]
pub(crate) struct UserFocus {
    pub pid: Option<u32>,
    pub display: Option<u32>,
}

impl UserFocus {
    pub fn capture() -> Self {
        let pid = frontmost_pid();
        let display = pid.and_then(display::of_app_window).or_else(display::under_cursor);
        Self { pid, display }
    }
}

/// Pid of the frontmost app — live via AX; NSWorkspace only as a fallback
/// (see `ax::focused_app_pid` for why it cannot be trusted in-process).
fn frontmost_pid() -> Option<u32> {
    ax::focused_app_pid()
        .or_else(frontmost_pid_by_window_order)
        .or_else(|| {
            NSWorkspace::sharedWorkspace()
                .frontmostApplication()
                .map(|a| a.processIdentifier() as u32)
        })
}

fn wait_frontmost(pid: u32, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    loop {
        if frontmost_pid() == Some(pid) {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Hand activation back to `pid`, bringing only its key window forward.
/// `activateWithOptions(0)` orders just the key window front, whereas AX
/// `AXFrontmost` and `ActivateAllWindows` order every window of the app
/// front (verified on macOS 26, two displays) — the difference is what lets
/// a window raised on the other display stay visible after the hand-back.
/// Falls back to the AX route if cooperative activation ignores the call.
#[allow(deprecated)]
fn reactivate_key_window_only(pid: u32) {
    if let Some(app) = NSRunningApplication::runningApplicationWithProcessIdentifier(pid as libc::pid_t) {
        if app.isTerminated() {
            return;
        }
        app.activateWithOptions(NSApplicationActivationOptions::empty());
        if wait_frontmost(pid, Duration::from_millis(300)) {
            return;
        }
        tracing::debug!(pid, "hand-back: activateWithOptions not honored, using AX");
    }
    let _ = ax::set_frontmost_app(pid);
}

/// `Child::focus` for an app this process did not launch.
pub(crate) fn focus_pid(pid: u32) -> Result<()> {
    let app = NSRunningApplication::runningApplicationWithProcessIdentifier(pid as libc::pid_t)
        .ok_or_else(|| Error::NotFound(format!("no running application with pid {pid}")))?;
    MacOSHandle::new(app, pid).focus()
}

/// A non-background launch raises the app through the display-aware
/// `focus` once it has a window — never through Launch Services' own
/// activation, which is unconditional and would pull focus from another
/// display before the window even exists. Polls AX with short timeouts so
/// an app that has not started answering AX yet cannot block.
fn schedule_deferred_focus(pid: u32, user: UserFocus) {
    let _ = std::thread::Builder::new()
        .name("launch-control-focus".into())
        .spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(30);
            loop {
                if ax::has_window(pid) {
                    break;
                }
                let alive = NSRunningApplication::runningApplicationWithProcessIdentifier(pid as libc::pid_t)
                    .map(|a| !a.isTerminated())
                    .unwrap_or(false);
                if !alive || Instant::now() >= deadline {
                    tracing::debug!(pid, alive, "deferred focus: no window appeared; leaving app as is");
                    return;
                }
                std::thread::sleep(Duration::from_millis(200));
            }
            let Some(app) = NSRunningApplication::runningApplicationWithProcessIdentifier(pid as libc::pid_t) else {
                return;
            };
            let handle = MacOSHandle::new(app, pid);
            if let Err(e) = handle.focus_relative_to(user) {
                tracing::warn!(pid, "deferred focus failed: {e}");
            }
        });
}

/// Map keyboard_types::Code to macOS virtual keycodes.
fn code_to_macos_keycode(code: keyboard_types::Code) -> Option<u16> {
    use keyboard_types::Code::*;
    Some(match code {
        KeyA => 0, KeyS => 1, KeyD => 2, KeyF => 3, KeyH => 4, KeyG => 5,
        KeyZ => 6, KeyX => 7, KeyC => 8, KeyV => 9, KeyB => 11, KeyQ => 12,
        KeyW => 13, KeyE => 14, KeyR => 15, KeyY => 16, KeyT => 17, KeyO => 31,
        KeyU => 32, KeyI => 34, KeyP => 35, KeyL => 37, KeyJ => 38, KeyK => 40,
        KeyN => 45, KeyM => 46,
        Enter => 36, Tab => 48, Space => 49, Backspace => 51, Escape => 53,
        Digit1 => 18, Digit2 => 19, Digit3 => 20, Digit4 => 21, Digit5 => 23,
        Digit6 => 22, Digit7 => 26, Digit8 => 28, Digit9 => 25, Digit0 => 29,
        Minus => 27, Equal => 24, BracketLeft => 33, BracketRight => 30,
        Backslash => 42, Semicolon => 41, Quote => 39, Backquote => 50,
        Comma => 43, Period => 47, Slash => 44,
        F1 => 122, F2 => 120, F3 => 99, F4 => 118, F5 => 96, F6 => 97,
        F7 => 98, F8 => 100, F9 => 101, F10 => 109, F11 => 103, F12 => 111,
        ArrowLeft => 123, ArrowRight => 124, ArrowDown => 125, ArrowUp => 126,
        Home => 115, End => 119, PageUp => 116, PageDown => 121, Delete => 117,
        _ => return None,
    })
}

/// Map keyboard_types::Modifiers to macOS CGEventFlags.
fn modifiers_to_cg_flags(modifiers: keyboard_types::Modifiers) -> u64 {
    let mut flags: u64 = 0;
    if modifiers.contains(keyboard_types::Modifiers::META) {
        flags |= 0x100000; // kCGEventFlagMaskCommand
    }
    if modifiers.contains(keyboard_types::Modifiers::SHIFT) {
        flags |= 0x020000; // kCGEventFlagMaskShift
    }
    if modifiers.contains(keyboard_types::Modifiers::ALT) {
        flags |= 0x080000; // kCGEventFlagMaskAlternate
    }
    if modifiers.contains(keyboard_types::Modifiers::CONTROL) {
        flags |= 0x040000; // kCGEventFlagMaskControl
    }
    flags
}

fn send_keystroke_to_pid(pid: u32, keycode: u16, flags: u64) -> Result<()> {
    use cg_ffi::*;

    // Route by frontmost-ness:
    //
    // - `CGEventPostToPid` writes to the target process's private Carbon
    //   event queue. A Qt app only drains that queue on an inactive→active
    //   transition, so posting here when the target is already frontmost
    //   results in silently dropped events (observed: foreground Studio never
    //   receiving Cmd+S).
    // - `CGEventPost(kCGSessionEventTap)` posts to the window server's
    //   session queue — the same queue hardware events take. NSApp of the
    //   frontmost app drains it continuously, so a synthetic Cmd+S there
    //   triggers shortcuts exactly like a real key press.
    //
    // Picked PostToPid for backgrounded targets because session-tap events
    // only go to whichever app is frontmost; we'd otherwise send Cmd+S to
    // the wrong app. Apple's own guidance (NSEvent docs): "CGEventPostToPSN
    // if you want to target a specific process, or post to kCGSessionEventTap
    // if you want to target the app with focus."
    let actual_frontmost_pid = frontmost_pid();
    let frontmost = actual_frontmost_pid == Some(pid);
    tracing::info!(
        pid,
        frontmost,
        actual_frontmost_pid = ?actual_frontmost_pid,
        route = if frontmost { "session_tap" } else { "post_to_pid" },
        "send_keystroke routing"
    );

    unsafe {
        let key_down = CGEventCreateKeyboardEvent(std::ptr::null_mut(), keycode, true);
        if key_down.is_null() {
            return Err(Error::Platform("failed to create key-down event".into()));
        }
        CGEventSetFlags(key_down, flags);
        if frontmost {
            CGEventPost(K_CG_SESSION_EVENT_TAP, key_down);
        } else {
            CGEventPostToPid(pid as libc::pid_t, key_down);
        }
        CFRelease(key_down);

        let key_up = CGEventCreateKeyboardEvent(std::ptr::null_mut(), keycode, false);
        if key_up.is_null() {
            return Err(Error::Platform("failed to create key-up event".into()));
        }
        CGEventSetFlags(key_up, flags);
        if frontmost {
            CGEventPost(K_CG_SESSION_EVENT_TAP, key_up);
        } else {
            CGEventPostToPid(pid as libc::pid_t, key_up);
        }
        CFRelease(key_up);
    }

    Ok(())
}

/// Accessibility (AXUIElement) bindings for invoking menu items directly.
///
/// Menu actions performed through AXPress run in the target regardless of
/// focus or activation state — the window server delivers them to the app's
/// NSMenu machinery, not through the keyboard event queue. This sidesteps
/// both failure modes of synthetic keystrokes:
///  - CGEventPostToPid lands in the target's private Carbon queue, which Qt
///    only drains on inactive→active transitions (dropped while background).
///  - CGEventPost(kCGSessionEventTap) goes to whichever app is frontmost,
///    which a CLI process often cannot make the target (cooperative
///    activation on modern macOS denies focus steals).
///
/// Uses the same TCC Accessibility right as CGEvent injection.
mod ax {
    use crate::error::{Error, Result};
    use std::ffi::c_void;

    type CFTypeRef = *const c_void;
    type CFStringRef = *const c_void;
    type CFArrayRef = *const c_void;
    type CFIndex = isize;
    type AXUIElementRef = *const c_void;
    type AXError = i32;
    type Boolean = u8;

    const K_CF_STRING_ENCODING_UTF8: u32 = 0x0800_0100;
    const K_CF_NUMBER_SINT32_TYPE: CFIndex = 3;
    const K_AX_ERROR_SUCCESS: AXError = 0;

    #[link(name = "ApplicationServices", kind = "framework")]
    unsafe extern "C" {
        fn AXIsProcessTrusted() -> Boolean;
        fn AXUIElementCreateApplication(pid: libc::pid_t) -> AXUIElementRef;
        fn AXUIElementCopyAttributeValue(
            element: AXUIElementRef,
            attribute: CFStringRef,
            value: *mut CFTypeRef,
        ) -> AXError;
        fn AXUIElementPerformAction(element: AXUIElementRef, action: CFStringRef) -> AXError;
        fn AXUIElementSetMessagingTimeout(element: AXUIElementRef, timeout_secs: f32) -> AXError;
        fn AXUIElementCreateSystemWide() -> AXUIElementRef;
        fn AXUIElementSetAttributeValue(
            element: AXUIElementRef,
            attribute: CFStringRef,
            value: CFTypeRef,
        ) -> AXError;
        fn AXValueGetValue(value: CFTypeRef, value_type: u32, out: *mut c_void) -> Boolean;
        fn AXUIElementGetPid(element: AXUIElementRef, pid: *mut libc::pid_t) -> AXError;
    }

    const K_AX_VALUE_TYPE_CGPOINT: u32 = 1;
    const K_AX_VALUE_TYPE_CGSIZE: u32 = 2;

    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        static kCFBooleanTrue: CFTypeRef;
    }

    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        fn CFStringCreateWithCString(
            alloc: *const c_void,
            c_str: *const i8,
            encoding: u32,
        ) -> CFStringRef;
        fn CFStringGetCString(
            string: CFStringRef,
            buffer: *mut i8,
            buffer_size: CFIndex,
            encoding: u32,
        ) -> Boolean;
        fn CFArrayGetCount(array: CFArrayRef) -> CFIndex;
        fn CFArrayGetValueAtIndex(array: CFArrayRef, idx: CFIndex) -> *const c_void;
        fn CFNumberGetValue(number: CFTypeRef, the_type: CFIndex, value_ptr: *mut c_void) -> Boolean;
        fn CFBooleanGetValue(boolean: CFTypeRef) -> Boolean;
        fn CFRetain(cf: CFTypeRef) -> CFTypeRef;
        // CFRelease is declared (with a *mut signature) in cg_ffi; reuse it.
    }

    use super::cg_ffi::CFRelease;

    /// Owned CFTypeRef released on drop. Null-safe.
    struct CFOwned(CFTypeRef);
    impl Drop for CFOwned {
        fn drop(&mut self) {
            if !self.0.is_null() {
                unsafe { CFRelease(self.0 as *mut c_void) };
            }
        }
    }

    /// NUL-terminated static names for the AX string constants we need —
    /// avoids depending on the framework-exported CFString globals.
    fn cf_string(name: &'static str) -> CFOwned {
        debug_assert!(name.ends_with('\0'));
        CFOwned(unsafe {
            CFStringCreateWithCString(
                std::ptr::null(),
                name.as_ptr() as *const i8,
                K_CF_STRING_ENCODING_UTF8,
            )
        })
    }

    /// Copy an attribute value; Ok(None) when the attribute is unsupported
    /// or empty for this element (common: separators have no cmd char).
    fn copy_attr(element: AXUIElementRef, attribute: &'static str) -> Option<CFOwned> {
        let attr = cf_string(attribute);
        let mut value: CFTypeRef = std::ptr::null();
        let err = unsafe { AXUIElementCopyAttributeValue(element, attr.0, &mut value) };
        if err == K_AX_ERROR_SUCCESS && !value.is_null() {
            Some(CFOwned(value))
        } else {
            None
        }
    }

    /// Like `copy_attr` but surfaces the AXError code — use where "the app
    /// didn't respond" (kAXErrorCannotComplete = -25204, common on a busy or
    /// still-launching target) must be distinguishable from "attribute
    /// genuinely absent".
    fn copy_attr_err(element: AXUIElementRef, attribute: &'static str) -> std::result::Result<CFOwned, AXError> {
        let attr = cf_string(attribute);
        let mut value: CFTypeRef = std::ptr::null();
        let err = unsafe { AXUIElementCopyAttributeValue(element, attr.0, &mut value) };
        if err == K_AX_ERROR_SUCCESS && !value.is_null() {
            Ok(CFOwned(value))
        } else {
            Err(err)
        }
    }

    fn bool_of(value: &CFOwned) -> Option<bool> {
        if value.0.is_null() {
            return None;
        }
        Some(unsafe { CFBooleanGetValue(value.0) } != 0)
    }

    /// Unhide the app via NSRunningApplication (no activation / focus steal).
    /// Menu items of hidden apps can report AXEnabled=false; unhiding lets
    /// the app validate its menus without taking focus from the user.
    fn unhide_app(pid: u32) {
        use objc2_app_kit::NSRunningApplication;
        if let Some(app) = NSRunningApplication::runningApplicationWithProcessIdentifier(pid as libc::pid_t) {
            if app.isHidden() {
                app.unhide();
            }
        }
    }

    /// PID of the currently frontmost application, if any (live).
    fn frontmost_pid() -> Option<u32> {
        super::frontmost_pid()
    }

    pub(super) fn is_trusted() -> bool {
        unsafe { AXIsProcessTrusted() != 0 }
    }

    /// Pid of the app that currently has focus, read live from the AX
    /// server. `NSWorkspace.frontmostApplication` is a cached value that
    /// only refreshes when the process pumps its main run loop — which a
    /// CLI or a Tokio backend never does — so it reports whatever was true
    /// at first use, indefinitely.
    pub(super) fn focused_app_pid() -> Option<u32> {
        let system = CFOwned(unsafe { AXUIElementCreateSystemWide() } as CFTypeRef);
        if system.0.is_null() {
            return None;
        }
        unsafe { AXUIElementSetMessagingTimeout(system.0 as AXUIElementRef, 0.5) };
        let app = copy_attr(system.0 as AXUIElementRef, "AXFocusedApplication\0")?;
        let mut pid: libc::pid_t = 0;
        let err = unsafe { AXUIElementGetPid(app.0 as AXUIElementRef, &mut pid) };
        (err == K_AX_ERROR_SUCCESS && pid > 0).then_some(pid as u32)
    }

    /// The app's AX element with a messaging timeout, so a target that is
    /// busy or still launching bounds how long each attribute read blocks.
    fn app_element(pid: u32, timeout_secs: f32) -> Option<CFOwned> {
        let app = CFOwned(unsafe { AXUIElementCreateApplication(pid as libc::pid_t) } as CFTypeRef);
        if app.0.is_null() {
            return None;
        }
        unsafe { AXUIElementSetMessagingTimeout(app.0 as AXUIElementRef, timeout_secs) };
        Some(app)
    }

    /// The app's focused window, else its main window, else its first.
    fn front_window(app: AXUIElementRef) -> Option<CFOwned> {
        if let Some(w) = copy_attr(app, "AXFocusedWindow\0") {
            return Some(w);
        }
        if let Some(w) = copy_attr(app, "AXMainWindow\0") {
            return Some(w);
        }
        let windows = copy_attr(app, "AXWindows\0")?;
        if unsafe { CFArrayGetCount(windows.0) } == 0 {
            return None;
        }
        let first = unsafe { CFArrayGetValueAtIndex(windows.0, 0) };
        if first.is_null() {
            return None;
        }
        Some(CFOwned(unsafe { CFRetain(first) }))
    }

    /// Centre of the app's front window in global CG coordinates.
    pub(super) fn front_window_center(pid: u32) -> Option<(f64, f64)> {
        let app = app_element(pid, 1.0)?;
        let win = front_window(app.0 as AXUIElementRef)?;
        let pos = copy_attr(win.0 as AXUIElementRef, "AXPosition\0")?;
        let size = copy_attr(win.0 as AXUIElementRef, "AXSize\0")?;
        let mut p = super::cg_ffi::CGPoint { x: 0.0, y: 0.0 };
        let mut s = super::cg_ffi::CGSize { width: 0.0, height: 0.0 };
        let ok_p = unsafe { AXValueGetValue(pos.0, K_AX_VALUE_TYPE_CGPOINT, &mut p as *mut _ as *mut c_void) };
        let ok_s = unsafe { AXValueGetValue(size.0, K_AX_VALUE_TYPE_CGSIZE, &mut s as *mut _ as *mut c_void) };
        if ok_p == 0 || ok_s == 0 {
            return None;
        }
        Some((p.x + s.width / 2.0, p.y + s.height / 2.0))
    }

    /// Whether the app has a window yet. Short AX timeout: this polls a
    /// launching app, whose first AX contact can otherwise block for seconds.
    pub(super) fn has_window(pid: u32) -> bool {
        match app_element(pid, 0.5) {
            Some(app) => front_window(app.0 as AXUIElementRef).is_some(),
            None => false,
        }
    }

    /// Activate an app by setting kAXFrontmostAttribute on its AX application
    /// element. Unlike `NSRunningApplication.activate` — which cooperative
    /// activation (macOS 14+) silently denies for CLI callers that are not
    /// themselves frontmost — the accessibility route is honored for any
    /// AX-trusted process (it is how window managers switch apps).
    pub(super) fn set_frontmost_app(pid: u32) -> AXError {
        let app = CFOwned(unsafe { AXUIElementCreateApplication(pid as libc::pid_t) } as CFTypeRef);
        if app.0.is_null() {
            return -1;
        }
        let attr = cf_string("AXFrontmost\0");
        unsafe {
            AXUIElementSetAttributeValue(app.0 as AXUIElementRef, attr.0, kCFBooleanTrue)
        }
    }

    fn string_of(value: &CFOwned) -> Option<String> {
        let mut buf = [0i8; 64];
        let ok = unsafe {
            CFStringGetCString(value.0, buf.as_mut_ptr(), buf.len() as CFIndex, K_CF_STRING_ENCODING_UTF8)
        };
        if ok == 0 {
            return None;
        }
        let cstr = unsafe { std::ffi::CStr::from_ptr(buf.as_ptr()) };
        cstr.to_str().ok().map(|s| s.to_string())
    }

    fn i32_of(value: &CFOwned) -> Option<i32> {
        let mut out: i32 = 0;
        let ok = unsafe {
            CFNumberGetValue(value.0, K_CF_NUMBER_SINT32_TYPE, &mut out as *mut i32 as *mut c_void)
        };
        (ok != 0).then_some(out)
    }

    /// Walk every (menu-bar title, menu item) pair in the app's menu bar,
    /// calling `pred(bar_title, item)` until it returns true — then run
    /// `on_match(item, title)` and return Ok(Some(its result)). Ok(None) when
    /// nothing matched.
    ///
    /// Reliability measures (each one is load-bearing, observed in practice):
    /// - 1s messaging timeout: AX queries to a busy/launching app otherwise
    ///   block ~6s per call (a single press was observed taking 25s); fail
    ///   fast instead and let the caller's retry loop re-enter.
    /// - kAXErrorCannotComplete is surfaced distinctly: it means "app not
    ///   responding to accessibility", not "menu absent".
    unsafe fn walk_first_matching<R>(
        pid: u32,
        mut pred: impl FnMut(&str, AXUIElementRef) -> bool,
        on_match: impl FnOnce(AXUIElementRef, &str) -> R,
    ) -> Result<Option<R>> {
        unsafe {
            if AXIsProcessTrusted() == 0 {
                return Err(Error::Platform(
                    "accessibility permission not granted (AXIsProcessTrusted=false)".into(),
                ));
            }
            let app = CFOwned(AXUIElementCreateApplication(pid as libc::pid_t) as CFTypeRef);
            if app.0.is_null() {
                return Err(Error::Platform("AXUIElementCreateApplication returned null".into()));
            }
            // Fail fast on unresponsive targets; the caller retries. Set on the
            // system-wide element: per-app timeouts don't propagate to element
            // refs copied out of attributes (observed: a query against a
            // still-launching Studio blocked ~25s despite an app-element
            // timeout), while the system-wide element changes this process's
            // default for all AX messaging.
            let systemwide = CFOwned(AXUIElementCreateSystemWide() as CFTypeRef);
            let _ = AXUIElementSetMessagingTimeout(systemwide.0 as AXUIElementRef, 1.0);
            let _ = AXUIElementSetMessagingTimeout(app.0 as AXUIElementRef, 1.0);

            let menubar = match copy_attr_err(app.0, "AXMenuBar\0") {
                Ok(v) => v,
                Err(-25204) => {
                    return Err(Error::Platform(
                        "app not responding to accessibility queries (kAXErrorCannotComplete)".into(),
                    ))
                }
                Err(code) => {
                    return Err(Error::Platform(format!(
                        "app has no accessible menu bar (AXError {code})"
                    )))
                }
            };
            let bar_items = copy_attr(menubar.0, "AXChildren\0")
                .ok_or_else(|| Error::Platform("menu bar has no children".into()))?;

            for i in 0..CFArrayGetCount(bar_items.0) {
                // Borrowed (Get-rule) — do not release array elements.
                let bar_item = CFArrayGetValueAtIndex(bar_items.0, i);
                let bar_title = copy_attr(bar_item, "AXTitle\0")
                    .and_then(|t| string_of(&t))
                    .unwrap_or_default();
                let Some(menus) = copy_attr(bar_item, "AXChildren\0") else { continue };
                for j in 0..CFArrayGetCount(menus.0) {
                    let menu = CFArrayGetValueAtIndex(menus.0, j);
                    let Some(items) = copy_attr(menu, "AXChildren\0") else { continue };
                    for k in 0..CFArrayGetCount(items.0) {
                        let item = CFArrayGetValueAtIndex(items.0, k);
                        if !pred(&bar_title, item) {
                            continue;
                        }
                        let title = copy_attr(item, "AXTitle\0")
                            .and_then(|t| string_of(&t))
                            .unwrap_or_default();
                        return Ok(Some(on_match(item, &title)));
                    }
                }
            }
            Ok(None)
        }
    }

    /// AXPress gate + action for a matched item. Disabled items swallow
    /// AXPress silently (observed: hidden Studio's File > Save to File
    /// no-ops). Two recovery steps before giving up, cheapest first:
    /// 1. unhide (no focus steal) — a hidden app may not validate menus.
    /// 2. activation flash via kAXFrontmostAttribute — Qt apps never
    ///    validate their native menu bar until their FIRST activation
    ///    (observed macOS 26 / Studio 0.729: a background-launched Studio
    ///    reports AXEnabled=false on every File item indefinitely, and
    ///    AXPress on them is silently swallowed). Cooperative activation
    ///    denies `NSRunningApplication.activate` from CLI callers, but the
    ///    AX route is honored for AX-trusted processes. Validation persists
    ///    after deactivation, so the flash happens at most once per app;
    ///    the previously frontmost app is restored right after the press.
    unsafe fn press_first_matching(
        pid: u32,
        desc: &str,
        pred: impl FnMut(&str, AXUIElementRef) -> bool,
    ) -> Result<()> {
        let pressed = unsafe {
            walk_first_matching(pid, pred, |item, title| {
                let enabled = |item: AXUIElementRef| {
                    copy_attr(item, "AXEnabled\0").and_then(|v| bool_of(&v))
                };
                let mut restore_front: Option<u32> = None;
                if enabled(item) == Some(false) {
                    unhide_app(pid);
                    std::thread::sleep(std::time::Duration::from_millis(300));
                }
                if enabled(item) == Some(false) {
                    let prev = frontmost_pid().filter(|&p| p != pid);
                    let ax_err = set_frontmost_app(pid);
                    tracing::info!(
                        pid, title, ax_err, prev_frontmost = ?prev,
                        "ax: item disabled — activation flash to force menu validation ({desc})"
                    );
                    restore_front = prev;
                    // Menu validation lands within a few hundred ms of
                    // activation; poll briefly instead of a fixed sleep.
                    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
                    while enabled(item) == Some(false) && std::time::Instant::now() < deadline {
                        std::thread::sleep(std::time::Duration::from_millis(100));
                    }
                    if enabled(item) == Some(false) {
                        if let Some(prev) = restore_front {
                            super::reactivate_key_window_only(prev);
                        }
                        return Err(Error::Platform(format!(
                            "menu item '{title}' is disabled (even after unhide + activation flash)"
                        )));
                    }
                }
                let action = cf_string("AXPress\0");
                let err = AXUIElementPerformAction(item, action.0);
                // Hand focus back to whoever had it before the flash. Done
                // after the press: the press itself needs no focus once the
                // item is validated, but restoring first would re-order the
                // app switch behind the pending AXPress on some runloops.
                if let Some(prev) = restore_front {
                    super::reactivate_key_window_only(prev);
                }
                if err == K_AX_ERROR_SUCCESS {
                    tracing::info!(pid, title, "ax: pressed menu item ({desc})");
                    Ok(())
                } else {
                    Err(Error::Platform(format!(
                        "AXPress on menu item '{title}' failed (AXError {err})"
                    )))
                }
            })?
        };
        match pressed {
            Some(result) => result,
            None => Err(Error::Platform(format!("no menu item matching {desc} found"))),
        }
    }

    /// AXPress the item titled `item_title` inside the menu titled
    /// `menu_title`. Exact title matches — the reliable route for apps (like
    /// Qt's Roblox Studio) that bind shortcuts internally without exposing
    /// AXMenuItemCmdChar on the item.
    pub(super) fn press_menu_item_by_title(pid: u32, menu_title: &str, item_title: &str) -> Result<()> {
        unsafe {
            press_first_matching(pid, &format!("'{menu_title}' > '{item_title}'"), |bar, item| {
                bar == menu_title
                    && copy_attr(item, "AXTitle\0")
                        .and_then(|t| string_of(&t))
                        .as_deref()
                        == Some(item_title)
            })
        }
    }

    /// Read-only existence probe for a menu item (no AXPress). Walks the same
    /// path as the press, so it exercises (and warms) the target's
    /// accessibility connection.
    pub(super) fn find_menu_item_by_title(pid: u32, menu_title: &str, item_title: &str) -> Result<bool> {
        unsafe {
            walk_first_matching(
                pid,
                |bar, item| {
                    bar == menu_title
                        && copy_attr(item, "AXTitle\0")
                            .and_then(|t| string_of(&t))
                            .as_deref()
                            == Some(item_title)
                },
                |_, _| (),
            )
            .map(|found| found.is_some())
        }
    }

    /// Find the menu item whose key equivalent is plain ⌘+`ch`
    /// (AXMenuItemCmdModifiers == 0 means "Command only") anywhere in the
    /// app's menu bar, and AXPress it.
    pub(super) fn press_menu_item_by_cmd_char(pid: u32, ch: char) -> Result<()> {
        let want = ch.to_ascii_uppercase().to_string();
        unsafe {
            press_first_matching(pid, &format!("plain ⌘{ch} key equivalent"), |_, item| {
                let char_matches = copy_attr(item, "AXMenuItemCmdChar\0")
                    .and_then(|c| string_of(&c))
                    .as_deref()
                    == Some(want.as_str());
                if !char_matches {
                    return false;
                }
                // 0 = Command with no extra modifiers; ⇧⌘ etc. are nonzero,
                // and we only want the plain chord.
                copy_attr(item, "AXMenuItemCmdModifiers\0")
                    .and_then(|m| i32_of(&m))
                    .unwrap_or(-1)
                    == 0
            })
        }
    }
}

/// Check if a process has exited. Returns synthetic ExitStatus.
///
/// macOS GUI apps launched via NSWorkspace are not our child processes,
/// so `waitpid` is not available. We use `kill(pid, 0)` to check liveness.
pub(crate) fn try_wait(pid: u32) -> io::Result<Option<ExitStatus>> {
    let ret = unsafe { libc::kill(pid as i32, 0) };
    if ret == 0 {
        // Process is still running
        Ok(None)
    } else {
        let err = io::Error::last_os_error();
        if err.raw_os_error() == Some(libc::ESRCH) {
            // Process does not exist — it has exited.
            // Synthesize a successful exit status. We can't get the real exit code
            // from a process that isn't our child.
            Ok(Some(make_exit_status(0)))
        } else {
            Err(err)
        }
    }
}

/// Create an ExitStatus from a raw wait status.
/// On Unix, ExitStatus wraps a libc wait status — use `__WEXITSTATUS` encoding.
fn make_exit_status(code: i32) -> ExitStatus {
    // Encode as a normal exit: bits 15:8 = exit code, bits 7:0 = 0 (no signal)
    let wait_status = (code & 0xFF) << 8;
    // SAFETY: ExitStatus on Unix wraps a c_int wait status
    unsafe { std::mem::transmute::<i32, ExitStatus>(wait_status) }
}


/// Single spawn function — handles both piped and non-piped based on Command's stdio config.
pub(crate) fn spawn(cmd: &mut Command) -> Result<crate::Child> {
    let path = &cmd.path;

    if !path.exists() {
        return Err(Error::NotFound(path.display().to_string()));
    }

    let needs_piped = cmd.stdout_cfg.is_some() || cmd.stderr_cfg.is_some();

    if needs_piped {
        spawn_piped(cmd)
    } else {
        spawn_simple(cmd)
    }
}

/// Launch via NSWorkspace (no stdio capture).
fn spawn_simple(cmd: &mut Command) -> Result<crate::Child> {
    let path = &cmd.path;

    // NSWorkspace expects an .app bundle URL. If the user passed the inner
    // binary (e.g. .../Contents/MacOS/RobloxStudio), walk up to find it.
    let bundle_path = find_app_bundle(path)
        .ok_or_else(|| Error::NotFound(format!("no .app bundle found for {}", path.display())))?;

    let path_str = NSString::from_str(&bundle_path.to_string_lossy());
    let url = NSURL::fileURLWithPath(&path_str);

    // Where the user is working, before the launch can disturb it.
    let user = UserFocus::capture();

    // Configure launch options. Never let Launch Services activate: that is
    // unconditional and lands before the app has a window. Foreground
    // launches are raised by `schedule_deferred_focus` instead.
    let config = NSWorkspaceOpenConfiguration::new();
    config.setCreatesNewApplicationInstance(true);
    config.setActivates(false);
    if cmd.background {
        config.setHides(true);
    }

    if !cmd.args.is_empty() {
        let ns_args: Vec<Retained<NSString>> =
            cmd.args.iter().map(|a| NSString::from_str(a)).collect();
        let ns_array = NSArray::from_retained_slice(&ns_args);
        config.setArguments(&ns_array);
    }

    // Bridge async completion handler -> synchronous channel
    let (tx, rx) =
        mpsc::channel::<std::result::Result<Retained<NSRunningApplication>, String>>();

    let block = block2::RcBlock::new(
        move |app: *mut NSRunningApplication, error: *mut objc2_foundation::NSError| {
            if app.is_null() {
                let msg = if !error.is_null() {
                    unsafe { &*error }.localizedDescription().to_string()
                } else {
                    "unknown launch error".to_string()
                };
                let _ = tx.send(Err(msg));
            } else {
                let retained = unsafe { Retained::retain(app) }
                    .expect("NSRunningApplication should not be null");
                let _ = tx.send(Ok(retained));
            }
        },
    );

    let workspace = NSWorkspace::sharedWorkspace();

    if let Some(ref open_url) = cmd.url {
        // URL mode: open a URL (e.g. custom://scheme) with the specified app
        let url_str = NSString::from_str(open_url);
        let open_nsurl = NSURL::URLWithString(&url_str)
            .ok_or_else(|| Error::Platform(format!("invalid URL: {open_url}")))?;
        let urls = NSArray::from_retained_slice(&[open_nsurl]);
        workspace.openURLs_withApplicationAtURL_configuration_completionHandler(
            &urls,
            &url,
            &config,
            Some(&block),
        );
    } else {
        // App mode: launch the application directly
        workspace.openApplicationAtURL_configuration_completionHandler(
            &url,
            &config,
            Some(&block),
        );
    }

    // Wait for completion (Studio launch is typically fast, but give it time)
    let result = rx
        .recv_timeout(std::time::Duration::from_secs(30))
        .map_err(|_| Error::Platform("launch timed out after 30s".into()))?
        .map_err(Error::Platform)?;

    let pid = result.processIdentifier();
    if pid < 0 {
        return Err(Error::Platform("process identifier unavailable".into()));
    }

    // Extra hide call in case the app self-activates during startup
    if cmd.background {
        result.hide();
    } else {
        schedule_deferred_focus(pid as u32, user);
    }

    let exit_state = std::sync::Arc::new(std::sync::Mutex::new(crate::ExitState::default()));
    crate::start_exit_watcher(pid as u32, exit_state.clone());

    Ok(crate::Child {
        pid: pid as u32,
        stdout: None,
        stderr: None,
        exit_state,
        inner: Some(MacOSHandle::new(result, pid as u32)),
    })
}

/// Spawn an application with piped stdout/stderr via the helper subprocess.
///
/// The helper (`launch-control`, this crate's binary target) holds the pty
/// masters in its own process. This makes the launched app's stdio survive
/// the calling process's death — for `Command::detached(true)`, when the
/// caller dies, the helper survives and the app keeps running with stdio
/// captured. For non-detached, the helper exits when its parent dies, the
/// pty masters close, and the app receives SIGHUP and dies (orderly cascade).
///
/// Uses `open -g -j` (inside the helper) for clean background launch (no
/// focus steal, no Dock activation). Returns an `NSRunningApplication`-backed
/// `Child` for focus/kill/keystroke support.
/// Read a process's argv via `sysctl KERN_PROCARGS2`. Returns None if the
/// process is gone, unreadable (different user), or mid-exec.
///
/// Buffer layout: `argc: i32 | exec_path\0 | \0 padding... | argv[0]\0 ...`.
fn proc_argv(pid: i32) -> Option<Vec<String>> {
    let mut mib = [libc::CTL_KERN, libc::KERN_PROCARGS2, pid];
    let mut size: libc::size_t = 0;
    let rc = unsafe {
        libc::sysctl(mib.as_mut_ptr(), 3, std::ptr::null_mut(), &mut size, std::ptr::null_mut(), 0)
    };
    if rc != 0 || size < 4 {
        return None;
    }
    let mut buf = vec![0u8; size];
    let rc = unsafe {
        libc::sysctl(
            mib.as_mut_ptr(),
            3,
            buf.as_mut_ptr() as *mut libc::c_void,
            &mut size,
            std::ptr::null_mut(),
            0,
        )
    };
    if rc != 0 || size < 4 {
        return None;
    }
    buf.truncate(size);

    let argc = i32::from_ne_bytes(buf[0..4].try_into().ok()?) as usize;
    let rest = &buf[4..];
    // Skip exec_path (first NUL-terminated string), then the NUL padding run.
    let exec_end = rest.iter().position(|&b| b == 0)?;
    let mut idx = exec_end;
    while idx < rest.len() && rest[idx] == 0 {
        idx += 1;
    }
    let mut args = Vec::with_capacity(argc);
    let mut start = idx;
    for i in idx..rest.len() {
        if rest[i] == 0 {
            args.push(String::from_utf8_lossy(&rest[start..i]).into_owned());
            start = i + 1;
            if args.len() == argc {
                break;
            }
        }
    }
    Some(args)
}

/// See [`crate::running_instance_cmdlines`]. Enumerates the bundle's running
/// instances the same way `spawn_piped` does and reads each argv via
/// `proc_argv`.
pub(crate) fn running_instance_cmdlines(app_path: &std::path::Path) -> Vec<String> {
    let Some(bundle_path) = find_app_bundle(app_path) else { return Vec::new() };
    let Some(bundle_id) = bundle_id_from_path(&bundle_path) else { return Vec::new() };
    let apps = NSRunningApplication::runningApplicationsWithBundleIdentifier(
        &NSString::from_str(&bundle_id),
    );
    apps.iter()
        .map(|app| app.processIdentifier())
        .filter(|&pid| pid > 0)
        .filter_map(proc_argv)
        .map(|argv| argv.join(" "))
        .collect()
}

/// Whether `haystack` (a process argv) contains `needle` as a contiguous
/// subsequence. `open --args` forwards our args verbatim, so the launched
/// instance's argv ends with exactly the args we passed.
fn argv_contains_args(haystack: &[String], needle: &[String]) -> bool {
    if needle.is_empty() {
        return true;
    }
    haystack
        .windows(needle.len())
        .any(|w| w == needle)
}

fn spawn_piped(cmd: &mut Command) -> Result<crate::Child> {
    let path = &cmd.path;

    if !path.exists() {
        return Err(Error::NotFound(path.display().to_string()));
    }

    // Resolve to .app bundle (the helper's `open` invocation requires it)
    let bundle_path = find_app_bundle(path)
        .ok_or_else(|| Error::NotFound(format!("no .app bundle found for {}", path.display())))?;
    let bundle_id = bundle_id_from_path(&bundle_path)
        .ok_or_else(|| Error::Platform("could not determine bundle ID for PID lookup".into()))?;

    // Find the helper binary. Two modes (in priority order):
    //   1. set_helper_invocation(bin, prefix_args) — caller-configured.
    //      Used both for subcommand integration (bin = consumer's own
    //      exe, prefix = ["__sub"]) and for embed-and-unpack (bin =
    //      unpacked path, no prefix).
    //   2. sibling-of-current_exe — standalone deployment.
    let (helper_bin, helper_prefix) = resolve_helper()?;

    // Hold the spawn lock across the snapshot → spawn-helper → PID-claim
    // window. NSRunningApplication's set-diff is racy under concurrent spawns
    // of the same bundle.
    let _spawn_guard = SPAWN_PIPED_LOCK.lock().unwrap_or_else(|e| e.into_inner());

    // Snapshot existing PIDs for this bundle to detect the new one after launch
    let before_pids: Vec<i32> = NSRunningApplication::runningApplicationsWithBundleIdentifier(
        &objc2_foundation::NSString::from_str(&bundle_id),
    ).iter().map(|app| app.processIdentifier()).collect();

    // Spawn the helper. The helper does the actual NSWorkspace+pty work
    // internally and forwards the app's stdio to its own stdout/stderr.
    // Where the user is working, before the launch can disturb it.
    let user = UserFocus::capture();

    let mut helper_cmd = std::process::Command::new(&helper_bin);
    helper_cmd.args(&helper_prefix);
    helper_cmd.arg("--bundle").arg(&bundle_path);
    if cmd.background {
        helper_cmd.arg("--background");
    } else {
        // `open` must not activate either; see `schedule_deferred_focus`.
        helper_cmd.arg("--no-activate");
    }
    if cmd.detached {
        helper_cmd.arg("--detached");
    }
    if !cmd.args.is_empty() {
        helper_cmd.arg("--");
        helper_cmd.args(&cmd.args);
    }
    helper_cmd.stdin(std::process::Stdio::null());
    helper_cmd.stdout(std::process::Stdio::piped());
    helper_cmd.stderr(std::process::Stdio::piped());

    let mut helper_child = helper_cmd.spawn()
        .map_err(|e| Error::Platform(format!("failed to spawn launch-control helper ({} {}): {e}", helper_bin.display(), helper_prefix.join(" "))))?;

    // Find the new PID by diffing against the snapshot, VERIFIED by argv.
    // The in-process SPAWN_PIPED_LOCK can't serialize claims across processes
    // (each `rodeo run` is its own process), and the bare "first unseen pid"
    // diff mis-claims under concurrent same-bundle launches — observed: two
    // launchers claiming the same Studio pid, leaving the third Studio
    // orphaned and a sibling's Studio killed by the wrong owner's cleanup.
    // Our args are launcher-unique in practice (unique temp place path,
    // -parentPid <our pid>), so requiring the candidate's argv to contain
    // exactly the args we passed makes the claim deterministic — including
    // against Studios launched concurrently by other processes or humans.
    // Fall back to the unverified diff only after the verified search has
    // had ample time (covers argless launches and argv-read failures).
    let pid = (|| -> Option<u32> {
        let mut unverified_fallback: Option<u32> = None;
        for attempt in 0..50 {
            let current = NSRunningApplication::runningApplicationsWithBundleIdentifier(
                &objc2_foundation::NSString::from_str(&bundle_id),
            );
            for app in current.iter() {
                let p = app.processIdentifier();
                if p <= 0 || before_pids.contains(&p) {
                    continue;
                }
                if cmd.args.is_empty() {
                    return Some(p as u32);
                }
                match proc_argv(p) {
                    Some(argv) if argv_contains_args(&argv, &cmd.args) => {
                        return Some(p as u32);
                    }
                    _ => {
                        // New pid for our bundle but not our argv (sibling
                        // launcher's instance, or argv unreadable). Remember
                        // it as a last-resort fallback.
                        unverified_fallback = Some(p as u32);
                    }
                }
            }
            // After 5s without an argv-verified match, accept the unverified
            // candidate rather than failing the launch outright.
            if attempt >= 25 {
                if let Some(p) = unverified_fallback {
                    tracing::warn!(
                        pid = p,
                        "spawn_piped: claiming pid WITHOUT argv verification (no candidate matched our args)",
                    );
                    return Some(p);
                }
            }
            // Helper bails fast on bad args; if it died before the app
            // appears, give up rather than wait the full 10s.
            if let Ok(Some(_status)) = helper_child.try_wait() {
                return None;
            }
            std::thread::sleep(std::time::Duration::from_millis(200));
        }
        None
    })()
    .ok_or_else(|| {
        // Drain helper stderr for diagnostics
        let mut helper_stderr = String::new();
        if let Some(mut stderr) = helper_child.stderr.take() {
            use std::io::Read;
            let _ = stderr.read_to_string(&mut helper_stderr);
        }
        Error::Platform(format!("helper failed to launch app: {helper_stderr}"))
    })?;

    // PID claimed — release the spawn lock.
    drop(_spawn_guard);

    let app = NSRunningApplication::runningApplicationWithProcessIdentifier(pid as i32);

    // Hide app if background mode — prevents Dock icon from appearing
    if cmd.background {
        if let Some(ref app) = app {
            app.hide();
        }
    } else {
        schedule_deferred_focus(pid, user);
    }

    let inner = app.map(|app| MacOSHandle::new(app, pid));

    // Drain helper subprocess stdout/stderr (each carries the app's
    // corresponding stream, forwarded by the helper's pty drain). These are
    // real OS pipes between us and the helper — no pty involved at this
    // layer, so we don't get SIGHUP'd if the helper dies later.
    let (stdout_tx, stdout_rx) = std::sync::mpsc::channel();
    let (stderr_tx, stderr_rx) = std::sync::mpsc::channel();
    if let Some(stdout) = helper_child.stdout.take() {
        crate::start_drain_thread(stdout, stdout_tx);
    }
    if let Some(stderr) = helper_child.stderr.take() {
        crate::start_drain_thread(stderr, stderr_tx);
    }

    let exit_state = std::sync::Arc::new(std::sync::Mutex::new(crate::ExitState::default()));
    crate::start_exit_watcher(pid, exit_state.clone());

    // Forget the helper handle — we don't want std::process::Child::Drop
    // killing the helper when this scope ends. Helper lifecycle is tied to
    // the app's via parent-exit + pty SIGHUP; explicit Child::kill() on our
    // returned handle goes via the app's NSRunningApplication, and the
    // helper sees the app exit and tears itself down.
    std::mem::forget(helper_child);

    Ok(crate::Child {
        pid,
        stdout: Some(crate::make_child_stdout(stdout_rx)),
        stderr: Some(crate::make_child_stderr(stderr_rx)),
        exit_state,
        inner,
    })
}

/// Resolve how to invoke the helper. Returns `(binary, prefix_args)` so
/// callers can run `<binary> <prefix_args...> <helper-flags...>`.
///
/// Priority:
///   1. [`crate::set_helper_invocation`] — caller configured. Used for
///      both subcommand integration (bin = consumer's exe, prefix = the
///      hidden subcommand name) and embed-and-unpack (bin = unpacked
///      path, prefix empty).
///   2. Sibling-of-`current_exe` — `<dir-of-consumer-bin>/launch-control`,
///      the typical "deploy the helper next to your binary" layout.
fn resolve_helper() -> Result<(std::path::PathBuf, Vec<String>)> {
    if let Some(inv) = crate::HELPER_INVOCATION.get() {
        if !inv.bin.exists() {
            return Err(Error::Platform(format!(
                "configured helper bin does not exist: {}",
                inv.bin.display()
            )));
        }
        return Ok((inv.bin.clone(), inv.prefix_args.clone()));
    }
    let exe = std::env::current_exe()
        .map_err(|e| Error::Platform(format!("current_exe failed: {e}")))?;
    let dir = exe.parent()
        .ok_or_else(|| Error::Platform("current_exe has no parent dir".into()))?;
    let candidate = dir.join("launch-control");
    if candidate.exists() {
        return Ok((candidate, Vec::new()));
    }
    Err(Error::Platform(format!(
        "launch-control helper not found at {} (call set_helper_invocation, or deploy the binary as a sibling)",
        candidate.display()
    )))
}

/// Body of the `launch-control` helper binary. Lives here so it's testable
/// lib code; the binary target is a 3-line shim that calls this.
///
/// Reads args:
///   `--bundle <path>` (required) — `.app` bundle path
///   `--background`               — pass `-g -j` to `open` (no focus steal)
///   `--no-activate`              — pass `-g` to `open`; the parent raises the app itself
///   `--detached`                 — survive parent death (don't exit on
///                                   parent-pid disappearance)
///   `--`  ... app args ...
///
/// Behavior:
///   1. Ignore SIGPIPE so writes to a closed parent stdout/stderr don't kill
///      us in detached mode.
///   2. Create stdout/stderr ptys, spawn `/usr/bin/open` with `--stdout`/
///      `--stderr` set to the slave paths.
///   3. Drain pty masters → forward each line to helper's own stdout/stderr.
///      When the app exits, its pty slave closes → master gets EIO → drain
///      thread ends.
///   4. If `!--detached`, watch the parent PID via `parent-exit`; on parent
///      death, exit (which closes the pty masters → app gets SIGHUP → orderly
///      cascade kill).
///   5. Block on the drain threads. When both finish (app exited), exit 0.
pub(crate) fn run_helper_main_with_args(args: impl Iterator<Item = String>) -> ! {
    // 1. Become our own session leader (`setsid`) so the helper is detached
    //    from the parent's session and process group. Without this, when the
    //    session leader (typically the shell / test runner that spawned the
    //    consumer process) dies, the kernel delivers SIGHUP to every process
    //    in the session — helper included — which kills detached mode.
    //
    //    Also explicitly ignore SIGPIPE (writes to a closed parent stdio
    //    pipe shouldn't kill us — we drain to void instead) and SIGHUP
    //    (defense in depth in case a controlling terminal we don't know
    //    about hangs up).
    unsafe {
        libc::setsid();
        libc::signal(libc::SIGPIPE, libc::SIG_IGN);
        libc::signal(libc::SIGHUP, libc::SIG_IGN);
    }

    // Parse args
    let mut bundle: Option<std::path::PathBuf> = None;
    let mut background = false;
    let mut no_activate = false;
    let mut detached = false;
    let mut app_args: Vec<String> = Vec::new();
    let mut iter = args;
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--bundle" => {
                bundle = iter.next().map(std::path::PathBuf::from);
            }
            "--background" => background = true,
            "--no-activate" => no_activate = true,
            "--detached" => detached = true,
            "--" => {
                app_args.extend(iter.by_ref());
                break;
            }
            other => {
                eprintln!("launch-control: unknown arg: {other}");
                std::process::exit(2);
            }
        }
    }

    let bundle = match bundle {
        Some(b) => b,
        None => {
            eprintln!("launch-control: --bundle <path> required");
            std::process::exit(2);
        }
    };

    let log_path = std::env::temp_dir().join(format!("launch-control-{}.log", std::process::id()));
    fn log_to(path: &std::path::Path, msg: &str) {
        use std::io::Write;
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs_f64())
                .unwrap_or(0.0);
            let _ = writeln!(f, "[{:.3}] [{}] {}", now, std::process::id(), msg);
        }
    }
    log_to(&log_path, &format!(
        "starting pid={} ppid={} detached={} background={} no_activate={} bundle={}",
        std::process::id(),
        unsafe { libc::getppid() },
        detached,
        background,
        no_activate,
        bundle.display(),
    ));

    // 2. Create ptys + spawn `open`
    let (stdout_master, stdout_slave_path) = match create_pty() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("launch-control: create_pty(stdout) failed: {e}");
            std::process::exit(1);
        }
    };
    let (stderr_master, stderr_slave_path) = match create_pty() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("launch-control: create_pty(stderr) failed: {e}");
            std::process::exit(1);
        }
    };

    let mut open_cmd = std::process::Command::new("/usr/bin/open");
    open_cmd.arg("-n");
    if background {
        open_cmd.args(["-g", "-j"]);
    } else if no_activate {
        open_cmd.arg("-g");
    }
    open_cmd.arg("-a").arg(&bundle);
    open_cmd.arg("--stdout").arg(&stdout_slave_path);
    open_cmd.arg("--stderr").arg(&stderr_slave_path);
    if !app_args.is_empty() {
        open_cmd.arg("--args");
        open_cmd.args(&app_args);
    }
    open_cmd.stdout(std::process::Stdio::null());
    open_cmd.stderr(std::process::Stdio::piped());

    let mut open_child = match open_cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("launch-control: failed to spawn /usr/bin/open: {e}");
            std::process::exit(1);
        }
    };
    let open_status = open_child.wait().ok();
    if !open_status.map(|s| s.success()).unwrap_or(false) {
        let mut open_stderr = String::new();
        if let Some(mut stderr) = open_child.stderr.take() {
            use std::io::Read;
            let _ = stderr.read_to_string(&mut open_stderr);
        }
        eprintln!("launch-control: open failed: {open_stderr}");
        std::process::exit(1);
    }

    log_to(&log_path, "open completed; about to find Studio PID via NSRunningApplication");

    // Find the Studio PID (just for logging; not strictly needed by helper)
    let bundle_id = bundle_id_from_path(&bundle).unwrap_or_default();
    let studio_pid: Option<i32> = if !bundle_id.is_empty() {
        let apps = NSRunningApplication::runningApplicationsWithBundleIdentifier(
            &objc2_foundation::NSString::from_str(&bundle_id),
        );
        let mut newest: Option<(std::time::SystemTime, i32)> = None;
        for app in apps.iter() {
            let p = app.processIdentifier();
            if p > 0 {
                newest = match newest {
                    None => Some((std::time::SystemTime::now(), p)),
                    Some((t, _)) => Some((t, p)),
                };
            }
        }
        newest.map(|(_, p)| p)
    } else {
        None
    };
    log_to(&log_path, &format!("found studio_pid={studio_pid:?}; starting drain threads"));

    // Watch Studio's exit so we log when it dies and can correlate timing
    if let Some(pid) = studio_pid {
        let lp = log_path.clone();
        parent_exit::on_pid_exit(pid as u32, move || {
            log_to(&lp, &format!("studio_pid={pid} EXITED"));
        });
    }

    // 3. Drain pty masters → helper's stdout/stderr (real pipes to parent)
    let stdout_handle = std::thread::spawn(move || {
        relay_pty_to(stdout_master, std::io::stdout());
    });
    let stderr_handle = std::thread::spawn(move || {
        relay_pty_to(stderr_master, std::io::stderr());
    });

    // 4. Watch parent PID. On Unix `getppid()` returns the immediate parent;
    // for our use case (spawned by launch-control's `Command::spawn`), that's
    // the consumer process (e.g. rodeo serve). If non-detached and parent
    // dies, exit — that closes our pty masters, app gets SIGHUP, dies.
    if !detached {
        let parent_pid = unsafe { libc::getppid() } as u32;
        log_to(&log_path, &format!("non-detached: registering parent_exit watch on pid={parent_pid}"));
        if parent_pid > 1 {
            let log_path_for_handler = log_path.clone();
            parent_exit::on_pid_exit(parent_pid, move || {
                log_to(&log_path_for_handler, "parent_exit handler fired; calling exit(0)");
                std::process::exit(0);
            });
        }
    } else {
        log_to(&log_path, "detached: skipping parent_exit registration");
    }

    // 5. Block on drains. When both end (pty EOF — app exited), helper exits.
    log_to(&log_path, "blocking on drain threads");
    let _ = stdout_handle.join();
    log_to(&log_path, "stdout drain finished");
    let _ = stderr_handle.join();
    log_to(&log_path, "stderr drain finished; exiting 0");
    std::process::exit(0);
}

/// Drain a pty master into a writer (helper's stdout/stderr) line-buffered.
/// If the writer fails (parent stdio closed), keep draining the pty into the
/// void so the app doesn't fill its pty buffer and stall.
fn relay_pty_to<W: std::io::Write>(reader: std::fs::File, mut writer: W) {
    use std::io::{BufRead, BufReader, Read};
    let log_path = std::env::temp_dir().join(format!("launch-control-{}.log", std::process::id()));
    let log = |msg: &str| {
        use std::io::Write;
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&log_path) {
            let _ = writeln!(f, "[{}] relay_pty: {msg}", std::process::id());
        }
    };
    let mut buf_reader = BufReader::new(reader);
    let mut line = String::new();
    let mut sink = false;
    let mut byte_buf = [0u8; 4096];
    loop {
        if sink {
            // Parent closed; keep pty draining via raw reads.
            line.clear();
            match buf_reader.read(&mut byte_buf) {
                Ok(0) => { log("EOF (sink mode)"); return; }
                Err(e) => { log(&format!("read err in sink: {e}")); return; }
                Ok(_) => continue,
            }
        }
        line.clear();
        match buf_reader.read_line(&mut line) {
            Ok(0) => { log("EOF (line mode)"); return; }
            Ok(_) => {
                if writer.write_all(line.as_bytes()).is_err() {
                    let _ = writer.flush();
                    sink = true;
                    log("write err → entering sink mode");
                    continue;
                }
                let _ = writer.flush();
            }
            Err(e) => { log(&format!("read_line err: {e}")); return; }
        }
    }
}

/// Create a pty pair and return (master File, slave device path).
/// The slave path (e.g. `/dev/ttys042`) can be passed to `open --stdout/--stderr`.
fn create_pty() -> Result<(std::fs::File, std::path::PathBuf)> {
    use std::os::unix::io::FromRawFd;

    let mut master: libc::c_int = 0;
    let mut slave: libc::c_int = 0;
    let ret = unsafe {
        libc::openpty(
            &mut master,
            &mut slave,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };
    if ret != 0 {
        return Err(Error::Platform(format!(
            "openpty failed: {}",
            std::io::Error::last_os_error()
        )));
    }

    // Get slave device path via ptsname
    let slave_path = unsafe {
        let ptr = libc::ptsname(master);
        if ptr.is_null() {
            libc::close(master);
            libc::close(slave);
            return Err(Error::Platform("ptsname failed".into()));
        }
        std::ffi::CStr::from_ptr(ptr)
            .to_string_lossy()
            .into_owned()
    };

    // Close slave fd — `open` will open the device path itself
    unsafe { libc::close(slave); }

    let master_file = unsafe { std::fs::File::from_raw_fd(master) };
    Ok((master_file, std::path::PathBuf::from(slave_path)))
}

/// Read the bundle identifier from an .app's Info.plist.
fn bundle_id_from_path(bundle_path: &std::path::Path) -> Option<String> {
    let plist_path = bundle_path.join("Contents/Info.plist");
    let content = std::fs::read(&plist_path).ok()?;
    let content_str = String::from_utf8_lossy(&content);
    // Simple XML parse for CFBundleIdentifier
    let key = "<key>CFBundleIdentifier</key>";
    let idx = content_str.find(key)?;
    let after = &content_str[idx + key.len()..];
    let start = after.find("<string>")? + 8;
    let end = after[start..].find("</string>")?;
    Some(after[start..start + end].to_string())
}

/// Walk up from a binary path to find the .app bundle directory.
/// E.g., /Applications/Foo.app/Contents/MacOS/Foo -> /Applications/Foo.app
fn find_app_bundle(path: &std::path::Path) -> Option<std::path::PathBuf> {
    if path.extension().is_some_and(|e| e == "app") {
        return Some(path.to_path_buf());
    }
    let mut current = path.to_path_buf();
    while let Some(parent) = current.parent() {
        if parent.extension().is_some_and(|e| e == "app") {
            return Some(parent.to_path_buf());
        }
        if parent == current {
            break;
        }
        current = parent.to_path_buf();
    }
    None
}
