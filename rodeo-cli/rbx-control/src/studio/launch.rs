//! Generic Roblox Studio process launch.
//!
//! Spawns a single Studio instance (`-task EditPlace` or a place file arg),
//! polls its stdout for the post-login marker, and manages lifecycle —
//! save-on-exit via Cmd+S keystroke, kill on drop, fflag restore. No
//! consumer-specific coupling: no daemon gating, no plugin install,
//! no session-guid stamping. Consumers compose this with their own
//! orchestration.

use anyhow::{bail, Context, Result};
use rbx_dom_weak::{InstanceBuilder, WeakDom};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use crate::fflags::{self, FflagConfig, FflagHandle, FflagTarget};
use crate::studio::layout;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// How to handle the place file on exit.
#[derive(Clone, Debug)]
pub enum SaveMode {
    /// No save — delete temp file on cleanup (default).
    NoSave,
    /// Save in-place — trigger Cmd+S, keep file.
    SaveInPlace,
    /// Save to output path — trigger Cmd+S, keep file at this path.
    SaveToPath(String),
}

/// What place to open in Studio.
#[derive(Clone, Debug)]
pub enum PlaceTarget {
    /// Fresh empty place (caller should pass the temp file path in `PlaceTarget::File`
    /// if they want a prepared place; `Empty` launches Studio with no file).
    Empty,
    /// Local `.rbxl`/`.rbxlx` file. Studio opens this as a local file.
    File(String),
    /// In-memory place bytes (rodeo downloaded them itself for the
    /// multiplayer-test path). Edit-mode launch doesn't support this; caller
    /// should use `File` or `PlaceId` instead.
    Content(Vec<u8>),
    /// Published place by ID. `universe_id` resolved via Roblox API if `None`.
    PlaceId { place_id: u64, universe_id: Option<u64> },
}

/// Options for launching Studio.
#[derive(Clone, Debug, Default)]
pub struct StudioOptions {
    /// Launch without focusing (for parallel/background launches).
    pub background: bool,
    /// How to handle the place file on exit.
    pub save: SaveMode,
    /// FFlag overrides to apply before launch (restored on cleanup).
    pub fflags: FflagConfig,
    /// If true, skip killing Studio on cleanup (Studio survives this process's exit).
    pub detached: bool,
    /// `--show-widgets` allow-list spec: which dock widgets to keep visible by
    /// patching the dock-layout plist before launch (everything else hidden).
    /// `None` = normal Studio (no patch); `Some("none")` = hide everything;
    /// `Some("output,...")` = keep those. Restored on cleanup.
    pub show_widgets: Option<String>,
    /// If set, launch via `-task RunScript -runScriptFile <path>` so Studio runs
    /// this Luau (command-bar identity) right after the place loads. File/PlaceId
    /// targets are passed as `-localPlaceFile`/`-placeId` alongside it. Used by
    /// rodeo to stamp the plugin's bootstrap attributes; `None` keeps the plain
    /// open-the-place launch.
    pub run_script_file: Option<PathBuf>,
}

impl Default for SaveMode {
    fn default() -> Self {
        SaveMode::NoSave
    }
}

// ---------------------------------------------------------------------------
// Studio
// ---------------------------------------------------------------------------

/// Handle to a launched Studio instance.
///
/// Owns the process lifecycle, place file, and FFlag restoration. Drop
/// triggers cleanup: save (if configured) → restore fflags → kill → delete
/// temp files. Cleanup is idempotent — safe to call explicit `cleanup()`
/// before the value drops.
pub struct Studio {
    handle: std::sync::Mutex<Option<launch_control::Child>>,
    /// PID stored separately so `kill()` never blocks on the handle mutex.
    pid: u32,
    place_path: Option<PathBuf>,
    save_mode: SaveMode,
    fflag_handle: Option<FflagHandle>,
    /// Dock-layout plist patch (`--show-widgets`). Restored alongside fflags.
    layout_handle: Option<filepatch::Handle>,
    /// Saved-once flag — cleanup() may be called by both explicit code and Drop.
    saved: AtomicBool,
    /// Cleaned-once flag — guarantees `cleanup()` runs its body only once.
    cleaned: AtomicBool,
    detached: bool,
    /// Text unique to this launch's command line (its RunScript bootstrap,
    /// else its place file), used to find its log. `None` when neither.
    log_marker: Option<String>,
    /// When this Studio was spawned; its log is newer.
    spawned_at: std::time::SystemTime,
}

impl Studio {
    /// Launch a new Studio instance with the given target and options.
    ///
    /// Returns a handle immediately; Studio is still booting. Callers detect
    /// readiness out of band (rodeo waits for the plugin's WebSocket connect).
    pub fn spawn(target: PlaceTarget, opts: StudioOptions) -> Result<Self> {
        // Apply fflags before launching (Studio reads them at startup).
        let fflag_handle = if !opts.fflags.overrides.is_empty() || opts.fflags.file.is_some() {
            fflags::apply(
                FflagTarget::Studio,
                &opts.fflags.overrides,
                opts.fflags.file.as_deref(),
            )?
        } else {
            None
        };

        tracing::info!(show_widgets = ?opts.show_widgets, "Studio::spawn invoked");

        // Apply dock-layout plist patch before launching (Studio reads it at startup).
        let layout_handle = if let Some(spec) = opts.show_widgets.as_deref() {
            let h = layout::apply_show_widgets(spec)
                .context("failed to apply --show-widgets layout patch")?;
            tracing::info!(applied = h.is_some(), "show-widgets: apply returned");
            h
        } else {
            None
        };

        let studio_path = studio_application_path()?;

        // `-parentPid` tells Studio to self-exit when the launching process
        // dies. On Windows it also forces a launch mode that does NOT load user
        // plugins — so the rodeo plugin never loads and no VM ever connects.
        // Omit it there and rely on explicit kill-on-drop (+ the JobObject the
        // serve supervisor wraps children in) for teardown. macOS/Linux keep
        // the parent-death behavior.
        #[cfg(target_os = "windows")]
        let parent_args: Vec<String> = Vec::new();
        // macOS also passes AppKit's per-process user-defaults override that
        // skips window-state restoration. When Studio dies within its first
        // seconds — a crash, or a kill while it is still "reopening windows" —
        // macOS greets the NEXT launch with a modal alert ("unexpectedly quit
        // while reopening windows. Reopen?") before Studio has started
        // logging. A background launch can never dismiss it, so every launch
        // hangs until a human clicks. Measured on macOS 27 / Studio 0.738: the
        // flag is accepted and bypasses the prompt even while it is armed.
        #[cfg(target_os = "macos")]
        let parent_args: Vec<String> = vec![
            "-parentPid".to_string(),
            std::process::id().to_string(),
            "-ApplePersistenceIgnoreState".to_string(),
            "YES".to_string(),
        ];
        #[cfg(not(any(target_os = "windows", target_os = "macos")))]
        let parent_args: Vec<String> =
            vec!["-parentPid".to_string(), std::process::id().to_string()];

        // When a RunScript bootstrap is requested, every target launches via
        // `-task RunScript ... -runScriptFile <path>` (verified to open local
        // files and run the script). Direct-exec rather than shell-open so the
        // explicit args reach Studio intact.
        let run_script_file = opts
            .run_script_file
            .as_ref()
            .map(|p| p.to_string_lossy().to_string());
        let use_run_script = run_script_file.is_some();
        let spawned_at = std::time::SystemTime::now();

        match target {
            PlaceTarget::PlaceId { place_id, universe_id } => {
                if !matches!(opts.save, SaveMode::NoSave) {
                    bail!("save modes cannot be used with PlaceId targets (use Studio's publish flow for cloud places)");
                }
                let uid = match universe_id {
                    Some(uid) => uid,
                    None => resolve_universe_id(place_id)?,
                };
                tracing::info!(place_id, universe_id = uid, "launching Studio for place");

                let mut cmd = launch_control::Command::new(&studio_path);
                let task = if use_run_script { "RunScript" } else { "EditPlace" };
                cmd.args([
                    "-task", task,
                    "-placeId", &place_id.to_string(),
                    "-universeId", &uid.to_string(),
                ]);
                if let Some(ref script) = run_script_file {
                    cmd.args(["-runScriptFile", script]);
                }
                let handle = cmd
                    .args(&parent_args)
                    .background(opts.background)
                    .detached(opts.detached)
                    .stdout(std::process::Stdio::piped())
                    .stderr(std::process::Stdio::piped())
                    .spawn()
                    .context("failed to launch Studio")?;

                let pid = handle.id();
                Ok(Studio {
                    handle: std::sync::Mutex::new(Some(handle)),
                    pid,
                    place_path: None,
                    save_mode: SaveMode::NoSave,
                    fflag_handle,
                    layout_handle,
                    saved: AtomicBool::new(false),
                    cleaned: AtomicBool::new(false),
                    detached: opts.detached,
                    log_marker: run_script_file.clone(),
                    spawned_at,
                })
            }
            PlaceTarget::File(ref path) => {
                let place_path = PathBuf::from(path);
                let abs_place = std::fs::canonicalize(&place_path)
                    .unwrap_or_else(|_| std::env::current_dir().unwrap().join(&place_path));
                // Windows `canonicalize` yields an extended-length path
                // (`\\?\C:\...`). Roblox Studio's launcher doesn't recognize
                // that form as a place file to open — it reports "Launch Intent
                // is None" and opens a blank place, so the rodeo plugin's
                // session-guid gate never matches and it never connects. Strip
                // the prefix to a normal absolute path. No-op on macOS/Linux,
                // where the prefix never appears.
                let place_str = abs_place.to_string_lossy().to_string();
                let place_str = place_str
                    .strip_prefix(r"\\?\")
                    .map(str::to_string)
                    .unwrap_or(place_str);

                tracing::info!(place = %place_str, use_run_script, "launching Studio");
                let mut cmd = launch_control::Command::new(&studio_path);
                if use_run_script {
                    // RunScript + local file: pass the place explicitly and run
                    // the bootstrap. Direct-exec (no shell_open) so the args reach
                    // Studio intact, as verified.
                    cmd.args(["-task", "RunScript", "-localPlaceFile", &place_str]);
                    if let Some(ref script) = run_script_file {
                        cmd.args(["-runScriptFile", script]);
                    }
                } else {
                    // Plain launch: open the place file directly. When detached,
                    // open it through the shell so Studio is rooted at explorer
                    // (persistent) rather than the daemon — otherwise Studio's
                    // launcher-watch reaps it when the daemon dies. No effect when
                    // not detached. (The first arg is the place file explorer opens.)
                    cmd.arg(&place_str).shell_open(true);
                }
                let handle = cmd
                    .args(&parent_args)
                    .background(opts.background)
                    .detached(opts.detached)
                    .stdout(std::process::Stdio::piped())
                    .stderr(std::process::Stdio::piped())
                    .spawn()
                    .context("failed to launch Studio")?;

                let pid = handle.id();
                Ok(Studio {
                    handle: std::sync::Mutex::new(Some(handle)),
                    pid,
                    place_path: Some(place_path),
                    save_mode: opts.save,
                    fflag_handle,
                    layout_handle,
                    saved: AtomicBool::new(false),
                    cleaned: AtomicBool::new(false),
                    detached: opts.detached,
                    log_marker: run_script_file.clone().or(Some(place_str)),
                    spawned_at,
                })
            }
            PlaceTarget::Content(_) => {
                bail!("Content variant is for the multiplayer-test flow; use File or PlaceId for edit-mode launch");
            }
            PlaceTarget::Empty => {
                tracing::info!(use_run_script, "launching Studio with no place file");
                let mut cmd = launch_control::Command::new(&studio_path);
                if use_run_script {
                    cmd.args(["-task", "RunScript"]);
                    if let Some(ref script) = run_script_file {
                        cmd.args(["-runScriptFile", script]);
                    }
                }
                let handle = cmd
                    .args(&parent_args)
                    .background(opts.background)
                    .detached(opts.detached)
                    .stdout(std::process::Stdio::piped())
                    .stderr(std::process::Stdio::piped())
                    .spawn()
                    .context("failed to launch Studio")?;

                let pid = handle.id();
                Ok(Studio {
                    handle: std::sync::Mutex::new(Some(handle)),
                    pid,
                    place_path: None,
                    save_mode: opts.save,
                    fflag_handle,
                    layout_handle,
                    saved: AtomicBool::new(false),
                    cleaned: AtomicBool::new(false),
                    detached: opts.detached,
                    log_marker: run_script_file.clone(),
                    spawned_at,
                })
            }
        }
    }

    /// Check if Studio process is still running.
    pub fn is_running(&self) -> bool {
        match self.handle.lock().unwrap().as_mut() {
            Some(handle) => handle.try_wait().ok().map_or(true, |s| s.is_none()),
            None => false,
        }
    }

    /// Register a callback invoked when the Studio process exits. Event-driven
    /// (no polling). Forwards directly to `launch_control::Child::on_exit` —
    /// the callback fires once, even if exit already happened.
    pub fn on_exit(&self, callback: impl FnOnce(std::process::ExitStatus) + Send + 'static) {
        if let Some(ref handle) = *self.handle.lock().unwrap() {
            handle.on_exit(callback);
        }
    }

    /// Studio process PID.
    pub fn pid(&self) -> u32 {
        self.pid
    }

    /// Path to the place file Studio opened, if any.
    pub fn place_path(&self) -> Option<&Path> {
        self.place_path.as_deref()
    }

    /// Whether this Studio was launched with `detached: true` — when true,
    /// Drop leaves the process running. Explicit `cleanup()` always kills
    /// regardless.
    pub fn detached(&self) -> bool {
        self.detached
    }

    /// Bring Studio to the front of its own display. Keyboard focus only
    /// moves to Studio when the user is working on that display (see
    /// `launch_control::Child::focus`).
    pub fn focus(&self) -> Result<()> {
        if let Some(ref handle) = *self.handle.lock().unwrap() {
            handle.focus().context("failed to focus Studio")?;
        }
        Ok(())
    }

    /// For the next `window`, undo any activation Studio grabs for itself
    /// (it does so when a test session starts or ends). See
    /// `launch_control::Child::guard_focus`.
    pub fn guard_focus(&self, window: std::time::Duration) {
        if let Some(ref handle) = *self.handle.lock().unwrap() {
            handle.guard_focus(window);
        }
    }

    /// One pre-warm probe of Studio's accessibility connection: walk the menu
    /// bar for "File > Save to File" (read-only — nothing is pressed).
    /// Returns true when the caller should stop probing (item answered, or
    /// the handle is gone). The first AX contact with a freshly-launched
    /// Studio can block ~25s while its main thread settles; probing in the
    /// background at launch keeps that out of the save budget so `save()`'s
    /// menu press responds in milliseconds.
    ///
    /// Single-shot by design: the caller loops while holding only a `Weak`
    /// to this Studio — holding the Studio (and with it the daemon
    /// SlotHandle + process handle) across a long warm loop delays teardown
    /// and exhausts launch slots.
    pub fn warm_save_menu_once(&self) -> bool {
        #[cfg(target_os = "macos")]
        {
            let guard = self.handle.lock().unwrap();
            match *guard {
                Some(ref handle) => matches!(handle.find_menu_item("File", "Save to File"), Ok(true)),
                None => true, // handle gone — Studio closing, stop probing
            }
        }
        #[cfg(not(target_os = "macos"))]
        {
            true
        }
    }

    /// Send Cmd+S / Ctrl+S to Studio to trigger save. focus() is best-effort —
    /// if it can't confirm we're frontmost, log a warning but still fire the
    /// keystroke. Rationale: the old ("pre-refactor") save path always fired
    /// Cmd+S unconditionally and it usually worked even when we weren't
    /// definitively frontmost. CGEventPostToPSN delivers to the process's
    /// event queue regardless of focus, and many menu dispatchers still
    /// handle the key equivalent.
    pub fn save(&self) -> Result<()> {
        let started = std::time::Instant::now();

        // macOS primary path: press File > Save to File via Accessibility.
        // This needs no focus and is delivered regardless of activation state,
        // so it avoids both keystroke failure modes (Qt's undrained Carbon
        // queue for background targets; cooperative-activation denials that
        // keep `focus()` from ever making the target frontmost — observed
        // eating ~27s of the save budget and still dropping the chord).
        // Matched by exact title because Studio binds ⌘S internally without
        // exposing AXMenuItemCmdChar on the item. Falls back to the keystroke
        // path if the AX press fails (e.g. no accessibility permission).
        #[cfg(target_os = "macos")]
        {
            let guard = self.handle.lock().unwrap();
            if let Some(ref handle) = *guard {
                match handle.press_menu_item("File", "Save to File") {
                    Ok(()) => {
                        tracing::info!(
                            pid = handle.id(),
                            elapsed_ms = started.elapsed().as_millis() as u64,
                            "save: pressed File > Save to File via accessibility",
                        );
                        return Ok(());
                    }
                    Err(e) => tracing::warn!(
                        pid = handle.id(),
                        "save: AX menu press failed ({e}); falling back to focus+keystroke",
                    ),
                }
            }
        }

        // On Windows, foregrounding happens inside `send_keystroke` under the
        // global keystroke lock. We must NOT pre-focus here: a `focus()` call
        // outside that lock races with a concurrent save's locked injection and
        // steals its foreground mid-chord, dropping the Ctrl+S (the place's
        // mtime never changes and the save hangs to timeout). On macOS,
        // CGEvent delivery wants the window frontmost first and has no
        // foreground-steal race, so activate there — unconditionally, since
        // the chord only reaches the frontmost app — and hand activation
        // back afterwards if the user was working on another display.
        let guard = self.handle.lock().unwrap();
        if let Some(ref handle) = *guard {
            #[cfg(target_os = "macos")]
            let hand_back = match handle.activate() {
                Ok(prev) => {
                    tracing::info!(
                        pid = self.pid,
                        elapsed_ms = started.elapsed().as_millis() as u64,
                        hand_back = ?prev,
                        "save: activated for keystroke",
                    );
                    prev
                }
                Err(e) => {
                    tracing::warn!(
                        pid = self.pid,
                        "save: activation did not confirm (continuing with keystroke anyway): {e}",
                    );
                    None
                }
            };
            tracing::info!(
                pid = handle.id(),
                place = ?self.place_path,
                focus_to_keystroke_ms = started.elapsed().as_millis() as u64,
                "save: sending Cmd+S keystroke",
            );
            // Save shortcut is Cmd+S on macOS, Ctrl+S elsewhere (META is the
            // Windows key on Windows, which would not trigger save).
            #[cfg(target_os = "macos")]
            let save_modifier = launch_control::Modifiers::META;
            #[cfg(not(target_os = "macos"))]
            let save_modifier = launch_control::Modifiers::CONTROL;
            let ks_result = handle
                .send_keystroke(launch_control::Code::KeyS, save_modifier);
            match &ks_result {
                Ok(()) => tracing::info!(pid = handle.id(), "save: send_keystroke returned Ok"),
                Err(e) => tracing::warn!(pid = handle.id(), "save: send_keystroke failed: {e}"),
            }
            #[cfg(target_os = "macos")]
            if let Some(prev) = hand_back {
                // Give the chord a moment to be consumed while Studio is
                // frontmost, then return the user's key window and focus.
                std::thread::sleep(std::time::Duration::from_millis(150));
                handle.restore_focus(prev);
            }
            ks_result.context("failed to send save keystroke to Studio")?;
            Ok(())
        } else {
            tracing::error!("save: no Studio handle available");
            bail!("no Studio handle available for save")
        }
    }

    /// Whether the process is still alive, without taking the handle mutex.
    fn alive(&self) -> bool {
        #[cfg(unix)]
        {
            unsafe { libc::kill(self.pid as i32, 0) == 0 }
        }
        #[cfg(not(unix))]
        {
            self.handle.try_lock().map_or(true, |mut handle| {
                handle.as_mut().is_some_and(|h| h.try_wait().ok().map_or(true, |status| status.is_none()))
            })
        }
    }

    /// Wait until killing this Studio can't lose its sign-in (see
    /// [`log::Credential`]), polling its log and Studio's cookie store: its
    /// sign-in has ended, and any token it renewed is in the store on disk. A
    /// kill before that signs the account out of every Studio. Gives up after
    /// 20 s (a Studio stuck before it signs in). A launch with no log marker
    /// can't be checked and is killed at once.
    fn wait_for_sign_in_to_settle(&self) {
        use super::log;
        const MAX_WAIT: Duration = Duration::from_secs(20);
        let (Some(dir), Some(marker)) = (crate::paths::roblox_logs_dir(), self.log_marker.as_deref()) else {
            return;
        };
        let give_up = Instant::now() + MAX_WAIT;
        let mut waiting = false;
        while self.alive() {
            let state = log::find_log(&dir, marker, self.spawned_at)
                .and_then(|path| std::fs::read_to_string(path).ok())
                .map(|text| log::credential_state(&text));
            let settled = match &state {
                // No log yet, or no finished sign-in: Studio may be renewing the token.
                None | Some(log::Credential::NotSignedIn | log::Credential::SigningIn) => false,
                Some(log::Credential::Renewed { name, at }) => renewed_token_on_disk(name, *at),
                Some(log::Credential::Settled) => true,
            };
            if settled {
                if waiting {
                    tracing::info!(pid = self.pid, "Studio's sign-in is saved; killing it");
                }
                return;
            }
            if Instant::now() >= give_up {
                tracing::warn!(pid = self.pid, ?state, "killing Studio before its sign-in settled; the next Studio may come up signed out");
                return;
            }
            if !waiting {
                tracing::info!(pid = self.pid, ?state, "waiting for Studio to save its sign-in before killing it");
                waiting = true;
            }
            std::thread::sleep(Duration::from_millis(200));
        }
    }

    /// Terminate the Studio process, first letting its sign-in settle (see
    /// [`Self::wait_for_sign_in_to_settle`]).
    /// Uses stored PID directly so it never blocks on the handle mutex.
    pub fn kill(&self) {
        if self.alive() {
            self.wait_for_sign_in_to_settle();
        }
        // Reap StudioTestService child processes first. `ExecuteMultiplayerTestAsync`
        // spawns `-task StartServer`/`StartClient` Studios as children of this edit
        // Studio (they carry `editpid <our pid>` in argv). rodeo does NOT own those
        // handles, and `EndTest` does not reliably terminate them, so killing only
        // this process would orphan them. Kill them explicitly by the editpid marker
        // (which survives the reparenting that happens when this process dies).
        reap_test_children(self.pid);

        #[cfg(unix)]
        unsafe {
            libc::kill(self.pid as i32, libc::SIGKILL);
        }
        #[cfg(not(unix))]
        if let Some(ref mut handle) = *self.handle.lock().unwrap() {
            let _ = handle.kill();
        }
    }

    /// Full cleanup: save (if configured and not yet saved) → restore fflags →
    /// kill → delete temp place file (if NoSave). Idempotent.
    ///
    /// Always kills the Studio process — the `detached` option only governs
    /// what happens when the `Studio` handle is dropped (without an explicit
    /// cleanup call). Calling this function is interpreted as the caller
    /// taking down the process.
    /// Mark the place as already saved so `cleanup()` skips its close-time
    /// backstop save. Callers set this after a *verified* save (mtime
    /// confirmed) — the backstop is one-shot and unverifiable, and re-firing
    /// it against an already-clean document just burns the mtime wait.
    pub fn mark_saved(&self) {
        self.saved.store(true, Ordering::Relaxed);
    }

    pub fn cleanup(&self) {
        if self.cleaned.swap(true, Ordering::Relaxed) {
            return;
        }

        // Save (one shot): trigger Cmd+S and wait for mtime change
        if !matches!(self.save_mode, SaveMode::NoSave)
            && !self.saved.swap(true, Ordering::Relaxed)
            && self.is_running()
        {
            tracing::info!("Saving Studio place...");
            let mtime_before = self
                .place_path
                .as_ref()
                .and_then(|p| std::fs::metadata(p).ok())
                .and_then(|m| m.modified().ok());

            if let Err(e) = self.save() {
                tracing::error!("Save failed: {e}");
            } else if let (Some(ref path), Some(before)) = (&self.place_path, mtime_before) {
                tracing::debug!(path = %path.display(), mtime_before = ?before, "waiting for save");
                let deadline = Instant::now() + Duration::from_secs(30);
                loop {
                    std::thread::sleep(Duration::from_millis(200));
                    if Instant::now() > deadline {
                        tracing::warn!("Save timed out after 30s");
                        break;
                    }
                    if let Ok(meta) = std::fs::metadata(path) {
                        if let Ok(now) = meta.modified() {
                            if now != before {
                                tracing::info!("Save complete");
                                break;
                            }
                        }
                    } else {
                        tracing::warn!("Place file disappeared during save poll");
                        break;
                    }
                }
            } else {
                std::thread::sleep(Duration::from_secs(2));
            }
        }

        // Restore fflags (always — system-wide state).
        if let Some(ref handle) = self.fflag_handle {
            handle.restore();
        }

        // Restore Studio dock-layout plist patch (--show-widgets).
        if let Some(ref handle) = self.layout_handle {
            handle.restore();
        }

        self.kill();
        // Only delete the place file when not saving (it was a temp).
        if matches!(self.save_mode, SaveMode::NoSave) {
            if let Some(ref path) = self.place_path {
                let _ = std::fs::remove_file(path);
                let lock_path = path.with_file_name(format!(
                    "{}.lock",
                    path.file_name().unwrap_or_default().to_string_lossy()
                ));
                let _ = std::fs::remove_file(lock_path);
            }
        }
    }
}

/// Kill the StudioTestService child processes (`-task StartServer`/`StartClient`
/// spawned by `ExecuteMultiplayerTestAsync`) belonging to the edit Studio
/// `edit_pid`. They carry an `editpid <edit_pid>` token in their command line,
/// stable even after the edit Studio dies and they reparent to launchd/init.
/// Best-effort and fire-and-forget — never blocks Studio teardown.
///
/// The marker MUST be matched on a token boundary. A bare substring match
/// (`editpid {edit_pid}` anywhere) ALSO matches `editpid {edit_pid}{digit}…` —
/// i.e. the StudioTestService children of *other* live sessions whose edit pid
/// merely has this pid as a numeric prefix (reaper pid `4761` matches a live
/// server carrying `editpid 47612`). SIGKILLing those drops their plugin
/// WebSocket ("Connection reset without closing handshake") and silently breaks
/// unrelated multiplayer tests. So we require the pid to be followed by a
/// non-digit or the end of the command line.
///
/// NOTE: a boundary match still cannot distinguish a *reused* pid — if
/// `edit_pid` is freed and the OS later assigns it to a different edit Studio, a
/// stale reap can exact-match the new session's children. Closing that fully
/// needs tracking the real child PIDs captured at `ExecuteMultiplayerTestAsync`
/// time rather than re-deriving them from the argv marker here.
fn reap_test_children(edit_pid: u32) {
    #[cfg(unix)]
    {
        // `([^0-9]|$)` anchors the pid to a token boundary (see above) so a
        // shorter pid does not match a longer pid that shares its prefix.
        let needle = format!("editpid {edit_pid}([^0-9]|$)");
        let Ok(out) = std::process::Command::new("pgrep").arg("-f").arg(&needle).output() else {
            return;
        };
        for line in String::from_utf8_lossy(&out.stdout).lines() {
            if let Ok(pid) = line.trim().parse::<i32>() {
                if pid as u32 != edit_pid {
                    unsafe { libc::kill(pid, libc::SIGKILL) };
                    tracing::info!(edit_pid, child_pid = pid, "reaped StudioTestService child process");
                }
            }
        }
    }
    #[cfg(windows)]
    {
        // Best-effort: terminate processes whose command line carries the editpid
        // marker. taskkill can't filter by command line, so use WMIC (deprecated
        // but still present on supported Windows versions). SQL LIKE has no regex,
        // so anchor the token boundary with two patterns: the pid followed by a
        // space (another arg follows) or sitting at the end of the command line.
        let filter = format!(
            "(CommandLine like '%editpid {edit_pid} %' or CommandLine like '%editpid {edit_pid}')"
        );
        let _ = std::process::Command::new("wmic")
            .args(["process", "where", filter.as_str(), "call", "terminate"])
            .output();
        let _ = edit_pid;
    }
}

/// Where Studio's cookie store can't be read (Windows), how long after a token
/// renewal to assume Studio wrote it to disk. Measured at 2.4 s on macOS.
const UNVERIFIED_SAVE_WAIT: Duration = Duration::from_secs(5);

/// Whether the token Studio renewed at `at`, stored as cookie record `name`,
/// is in its cookie store on disk.
fn renewed_token_on_disk(name: &str, at: std::time::SystemTime) -> bool {
    use super::cookies;
    match cookies::store_path() {
        // The store keeps whole seconds, so this save's record is created no
        // earlier than `at` rounded down; the previous one is minutes older.
        Some(path) => std::fs::read(path)
            .ok()
            .and_then(|file| cookies::created_at(&file, name))
            .is_some_and(|created| created + Duration::from_secs(1) > at),
        None => std::time::SystemTime::now() >= at + UNVERIFIED_SAVE_WAIT,
    }
}

impl Drop for Studio {
    fn drop(&mut self) {
        tracing::debug!(pid = self.pid, detached = self.detached, "rbx_control::Studio::Drop");
        if self.detached {
            // Caller asked Studio to survive parent exit. Restore system-wide
            // state (fflags, layout plist) so we don't leak overrides, but
            // leave the Studio process running and don't delete its place file.
            // Explicit `cleanup()` calls bypass this and always tear down.
            if !self.cleaned.swap(true, Ordering::Relaxed) {
                if let Some(ref handle) = self.fflag_handle { handle.restore(); }
                if let Some(ref handle) = self.layout_handle { handle.restore(); }
            }
        } else {
            self.cleanup();
        }
    }
}

// ---------------------------------------------------------------------------
// Place helpers
// ---------------------------------------------------------------------------

/// Create a minimal empty place DOM (DataModel + Workspace).
pub fn create_minimal_place() -> WeakDom {
    let mut dom = WeakDom::new(InstanceBuilder::new("DataModel"));
    let root = dom.root_ref();
    let workspace = InstanceBuilder::new("Workspace");
    dom.insert(root, workspace);
    dom
}

/// Serialize a `WeakDom` to `.rbxl` binary format.
pub fn serialize_place(dom: &WeakDom) -> Result<Vec<u8>> {
    let mut buf = Vec::new();
    rbx_binary::to_writer(&mut buf, dom, dom.root().children())
        .context("failed to serialize place")?;
    Ok(buf)
}

/// Write a DOM to a place file, choosing binary (`.rbxl`) or XML (`.rbxlx`) format
/// based on the file extension.
pub fn write_place(dom: &WeakDom, path: &Path) -> Result<()> {
    let refs = dom.root().children();
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
    if ext == "rbxlx" {
        let mut buf = Vec::new();
        let options = rbx_xml::EncodeOptions::new()
            .property_behavior(rbx_xml::EncodePropertyBehavior::WriteUnknown);
        rbx_xml::to_writer(&mut buf, dom, refs, options)
            .context("failed to serialize XML place")?;
        std::fs::write(path, buf).context("failed to write place file")
    } else {
        let mut buf = Vec::new();
        rbx_binary::to_writer(&mut buf, dom, refs)
            .context("failed to serialize binary place")?;
        std::fs::write(path, buf).context("failed to write place file")
    }
}

// ---------------------------------------------------------------------------
// Universe ID resolution
// ---------------------------------------------------------------------------

/// Resolve a place ID to its universe ID via the public Roblox API.
pub fn resolve_universe_id(place_id: u64) -> Result<u64> {
    let url = format!("https://apis.roblox.com/universes/v1/places/{place_id}/universe");
    let resp = reqwest::blocking::get(&url).context("failed to reach Roblox API")?;
    if !resp.status().is_success() {
        bail!(
            "Roblox API returned {} for place {place_id} — verify the place ID exists and is published",
            resp.status()
        );
    }
    let body: serde_json::Value = resp.json().context("failed to parse Roblox API response")?;
    body["universeId"]
        .as_u64()
        .context("Roblox API response missing universeId")
}

// ---------------------------------------------------------------------------
// Studio binary discovery
// ---------------------------------------------------------------------------

/// Path to the Roblox Studio application.
/// On macOS, returns the `.app` bundle path (for `NSWorkspace`).
/// On Windows, returns the executable path.
pub fn studio_application_path() -> Result<String> {
    let studio =
        roblox_install::RobloxStudio::locate().context("could not locate Roblox Studio")?;

    #[cfg(target_os = "macos")]
    {
        let app_path = studio
            .application_path()
            .ancestors()
            .find(|p| p.extension().is_some_and(|e| e == "app"))
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|| studio.application_path().to_string_lossy().to_string());
        Ok(app_path)
    }

    #[cfg(not(target_os = "macos"))]
    {
        Ok(studio.application_path().to_string_lossy().to_string())
    }
}

/// Path to the Roblox Studio `content/` directory, if available.
pub fn studio_content_path() -> Option<String> {
    roblox_install::RobloxStudio::locate()
        .ok()
        .map(|s| s.content_path().to_string_lossy().to_string())
}


/// Self-heal Roblox Studio discovery when the registry pointer is stale.
///
/// `roblox_install`'s Windows lookup trusts `HKCU\…\RobloxStudio\ContentFolder`,
/// which Studio's auto-update leaves pointing at the *deleted* previous version
/// dir — so `locate()` resolves a `RobloxStudioBeta.exe` that no longer exists
/// and every launch fails in milliseconds. The crate exposes no public way to
/// force its Versions/ scan except the documented `ROBLOX_STUDIO_PATH` escape
/// hatch: point it at the Roblox root and it finds the latest installed version
/// itself.
///
/// Call once at process start, before spawning threads. No-op when the user
/// already set `ROBLOX_STUDIO_PATH` or when the registry pointer is valid, so
/// the happy path and explicit overrides are untouched. Child processes inherit
/// the variable, so a single call at the top-level entry covers the whole tree.
pub fn ensure_studio_env() {
    // Respect an explicit override — the user (or a test harness) knows best.
    if std::env::var_os("ROBLOX_STUDIO_PATH").is_some() {
        return;
    }
    // Happy path: the registry pointer already resolves to a real executable.
    if let Ok(studio) = roblox_install::RobloxStudio::locate() {
        if studio.application_path().is_file() {
            return;
        }
    }
    // Stale/missing pointer: fall back to the Roblox root so the crate scans
    // Versions/ for a live install. Windows-only — this failure mode is specific
    // to the Windows ContentFolder registry heuristic.
    #[cfg(target_os = "windows")]
    {
        if let Some(root) =
            std::env::var_os("LOCALAPPDATA").map(|p| PathBuf::from(p).join("Roblox"))
        {
            if root.join("Versions").is_dir() {
                // Runs before tracing is initialized (main → here → tokio →
                // subscriber), so tracing::warn! would be dropped; use stderr.
                eprintln!(
                    "rodeo: RobloxStudio registry pointer is stale (deleted version dir, \
                     likely after a Studio auto-update); scanning {}\\Versions for a live \
                     install",
                    root.display()
                );
                std::env::set_var("ROBLOX_STUDIO_PATH", root);
            }
        }
    }
}
