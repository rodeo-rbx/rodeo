//! Rodeo-specific Studio launch wrappers.
//!
//! Composes [`rbx_control::studio::launch::Studio`] with rodeo orchestration:
//! installs this backend's rodeo plugin file, generates the RunScript
//! bootstrap that stamps `rodeoSession`/`rodeoPort` onto the Workspace at
//! launch (so the plugin routes to this launch), and binds the log scanner.
//! No place-file mutation: the original file opens unchanged.

use anyhow::{bail, Context, Result};
use rbx_dom_weak::{InstanceBuilder, WeakDom};
use std::path::{Path, PathBuf};

use crate::studio_backend::plugin_embed;

// Re-exports so call sites that reach `crate::studio_backend::{SaveMode, PlaceTarget, FflagConfig}`
// (via `pub use launch::*;` in mod.rs) keep working unchanged.
pub use rbx_control::fflags::FflagConfig;
pub use rbx_control::studio::launch::{PlaceTarget, SaveMode};

/// Parse a rodeo CLI `--save` argument into a `SaveMode`.
/// - `None` → `NoSave`
/// - `Some("")` (bare `--save` flag) → `SaveInPlace`
/// - `Some(path)` → `SaveToPath(path)`
///
/// The empty-string-means-SaveInPlace convention is rodeo's CLI shape, not
/// general Studio automation semantics, so it lives here rather than in
/// rbx-control.
pub fn parse_save_mode(save: Option<String>) -> SaveMode {
    match save {
        None => SaveMode::NoSave,
        Some(s) if s.is_empty() => SaveMode::SaveInPlace,
        Some(path) => SaveMode::SaveToPath(path),
    }
}

/// Append microprofiler auto-capture fflags to an existing `FflagConfig`
/// when the launch request asks for profiling. Centralized here so every
/// launch path (Studio, MP-test) and every client (CLI, rodeo-client,
/// rodeo-client-ts, rodeo-client-lune) gets identical fflag injection without
/// each having to know the magic FFlag names. Skips any fflag the caller
/// already set so explicit user overrides win.
///
/// Reads `RODEO_PROFILE_FRAME_INTERVAL` and `RODEO_PROFILE_NUM_FRAMES` from
/// the backend host's environment for tuning; both default to 60.
pub fn inject_profile_fflags(fflags: FflagConfig) -> FflagConfig {
    let interval = std::env::var("RODEO_PROFILE_FRAME_INTERVAL")
        .unwrap_or_else(|_| "60".to_string());
    let num_frames = std::env::var("RODEO_PROFILE_NUM_FRAMES")
        .unwrap_or_else(|_| "60".to_string());

    let injected = [
        "FFlagDebugMicroProfilerAutoCaptureRawEnabled=true".to_string(),
        format!("FIntDebugMicroProfilerAutoCaptureRawInterval={interval}"),
        format!("FIntDebugMicroProfilerAutoCaptureRawNumFrames={num_frames}"),
    ];

    let mut out = fflags;
    for flag in &injected {
        let key = flag.split('=').next().unwrap();
        if !out.overrides.iter().any(|f| f.starts_with(key)) {
            out.overrides.push(flag.clone());
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Rodeo StudioOptions — adds session_guid + plugin port on top of generic opts.
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
pub struct StudioOptions {
    /// Port the rodeo plugin connects to. Stamped as the `rodeoPort` Workspace
    /// attribute by the RunScript bootstrap so the static plugin connects here.
    pub port: u16,
    pub background: bool,
    pub save: SaveMode,
    pub fflags: FflagConfig,
    pub detached: bool,
    /// `--show-widgets` allow-list spec (`None` = normal Studio; `Some("none")`
    /// = hide all; `Some("output,...")` = keep those). Restored on cleanup.
    pub show_widgets: Option<String>,
    /// Master-minted session identity. Stamped as the `rodeoSession` Workspace
    /// attribute by the RunScript bootstrap; the plugin sends it on the WS
    /// handshake so master correlates the connecting DOM to this launch.
    pub session_guid: String,
}

// ---------------------------------------------------------------------------
// Studio — wraps rbx_control::studio::launch::Studio with rodeo state.
// ---------------------------------------------------------------------------

/// Handle to a rodeo-managed Studio instance. Composes:
/// - `inner`: generic process + fflag + save-on-exit mechanics
/// - `log_path`: paired by the log scanner shortly after launch
///
/// The rodeo plugin is a permanent static install (shared with manual use), so
/// there's no per-launch plugin file to track or delete.
pub struct Studio {
    /// Master-minted session identity.
    session_guid: String,
    /// Inner generic Studio. Drop runs save + kill + fflag restore.
    inner: rbx_control::studio::launch::Studio,
    /// The place file the caller asked to open, absolutized. None for
    /// place-id and blank-place launches. Surfaced in `rodeo state`.
    source_path: Option<String>,
    /// The file Studio actually has open (temp copy for NoSave, the source
    /// itself for SaveInPlace, the output path for SaveToPath), absolutized.
    /// None for place-id launches. Surfaced in `rodeo state`.
    working_path: Option<String>,
}

impl Studio {
    /// Spawn a new Studio instance. Installs the rodeo plugin, prepares the
    /// place file (stamping session_guid), and launches Studio. Readiness is
    /// signaled by the plugin's WebSocket connection, not by this handle.
    pub fn spawn(target: PlaceTarget, opts: StudioOptions) -> Result<Self> {
        let session_guid = opts.session_guid.clone();
        let sg_short = &session_guid[..8.min(session_guid.len())];

        // The plugin is installed once at studio-backend startup (see
        // `commands::serve::run_studio_backend`), not per launch — a launch
        // always goes through a running backend that already ensured it.

        // Generate the RunScript bootstrap. It stamps `rodeoSession`/`rodeoPort`
        // onto the Workspace once Studio is up, so the static plugin routes to
        // this launch's serve port and master can correlate the DOM. No place
        // mutation needed.
        tracing::info!(session_guid = sg_short, "spawn: writing RunScript bootstrap");
        let bootstrap = write_bootstrap_script(&session_guid, opts.port)?;

        // Prepare the place file (temp copy for NoSave, original for SaveInPlace,
        // output copy for SaveToPath; synthesize a minimal place for Empty so
        // RunScript has a file to open). For PlaceId targets, skip prep — Studio
        // downloads the file from Roblox.
        let mut source_path: Option<String> = None;
        let mut working_path: Option<String> = None;
        let prepared_target = match &target {
            PlaceTarget::PlaceId { .. } => target.clone(),
            PlaceTarget::Content(_) => {
                bail!("Content variant is for the multiplayer-test flow; edit-mode launch should receive File or PlaceId");
            }
            PlaceTarget::File(_) | PlaceTarget::Empty => {
                let place_str = match &target {
                    PlaceTarget::File(p) => Some(p.as_str()),
                    _ => None,
                };
                tracing::info!(session_guid = sg_short, "spawn: preparing place file");
                let place_path = prepare_place(place_str, &opts.save)?;
                source_path = place_str.map(|p| absolutize(Path::new(p)));
                working_path = Some(absolutize(&place_path));
                PlaceTarget::File(place_path.to_string_lossy().to_string())
            }
        };

        settle_plugin_file(opts.port);

        tracing::info!(session_guid = sg_short, "spawn: calling rbx_control Studio::spawn");
        let inner = rbx_control::studio::launch::Studio::spawn(
            prepared_target,
            rbx_control::studio::launch::StudioOptions {
                background: opts.background,
                save: opts.save,
                fflags: opts.fflags,
                detached: opts.detached,
                show_widgets: opts.show_widgets.as_deref().map(|s| expand_show_widgets(s, opts.port)),
                run_script_file: Some(bootstrap),
            },
        )?;
        tracing::info!(session_guid = sg_short, pid = inner.pid(), "spawn: Studio process spawned");
        if opts.background {
            // A background Studio activates itself when its first session
            // starts (the initial mode is baked into the bootstrap, so no
            // SetTargetMode is relayed for it). Guard from spawn so that
            // activation is handed straight back; transitions re-arm later.
            inner.guard_focus(std::time::Duration::from_secs(120));
        }

        Ok(Studio {
            session_guid,
            inner,
            source_path,
            working_path,
        })
    }

    pub fn source_path(&self) -> Option<&str> { self.source_path.as_deref() }
    pub fn working_path(&self) -> Option<&str> { self.working_path.as_deref() }

    // -- Delegates to inner --

    pub fn detached(&self) -> bool { self.inner.detached() }
    /// Event-driven exit notification. See `launch_control::Child::on_exit`.
    pub fn on_exit(&self, callback: impl FnOnce(std::process::ExitStatus) + Send + 'static) {
        self.inner.on_exit(callback);
    }
    pub fn place_path(&self) -> Option<&Path> { self.inner.place_path() }
    pub fn save(&self) -> Result<()> { self.inner.save() }
    pub fn mark_saved(&self) { self.inner.mark_saved() }
    /// Undo activation Studio grabs for itself during a mode transition
    /// (see `rbx_control::studio::Studio::guard_focus`).
    pub fn guard_focus(&self, window: std::time::Duration) { self.inner.guard_focus(window) }
    pub fn warm_save_menu_once(&self) -> bool { self.inner.warm_save_menu_once() }
    pub fn kill(&self) { self.inner.kill() }

    /// Full cleanup — delegates to inner (save + kill + fflag restore). The
    /// static plugin is a permanent install and is intentionally left in place.
    pub fn cleanup(&self) {
        self.inner.cleanup();
    }
}

impl Drop for Studio {
    fn drop(&mut self) {
        let sg_short = &self.session_guid[..8.min(self.session_guid.len())];
        let detached = self.inner.detached();
        tracing::info!(
            session_guid = sg_short,
            pid = self.inner.pid(),
            detached,
            "Studio::Drop fired"
        );
        if detached {
            // Caller asked Studio to survive parent exit. Skip cleanup —
            // inner Drop handles fflag/layout restore and leaves the Studio
            // process alone. The static plugin is a permanent install, so
            // there's nothing launch-specific to remove either way.
            return;
        }
        self.cleanup();
    }
}

// ---------------------------------------------------------------------------
// Rodeo-specific helpers: static plugin install, RunScript bootstrap, place prep
// ---------------------------------------------------------------------------

/// Studio's local plugins directory (`~/Documents/Roblox/Plugins` on macOS).
pub(crate) fn plugins_dir() -> Result<PathBuf> {
    let studio = roblox_install::RobloxStudio::locate()
        .context("failed to locate Roblox Studio install")?;
    Ok(studio.plugins_path().to_path_buf())
}

/// File name of the plugin a studio backend on `port` installs.
///
/// One file per running backend, named by build and port, so serves of
/// different builds or ports never overwrite each other's plugin (issue #12).
/// The build id is verbatim (`1.4.0-rc.4+b493edb`, dashes included), so the
/// name parses from the right: the port is the last `-` segment.
pub(crate) fn plugin_file_name(build: &str, port: u16) -> String {
    format!("rodeo-{build}-{port}.rbxm")
}

/// Inverse of [`plugin_file_name`]. `None` for anything else in the plugins
/// directory — including the legacy `rodeo.rbxm` that pre-1.5 versions write,
/// which nothing in this build reads, writes, or removes.
pub(crate) fn parse_plugin_file_name(name: &str) -> Option<(String, u16)> {
    let stem = name.strip_prefix("rodeo-")?.strip_suffix(".rbxm")?;
    let (build, port) = stem.rsplit_once('-')?;
    if build.is_empty() {
        return None;
    }
    Some((build.to_string(), port.parse().ok()?))
}

/// Where this build's plugin for a backend on `port` lives.
pub(crate) fn plugin_path(port: u16) -> Result<PathBuf> {
    Ok(plugins_dir()?.join(plugin_file_name(rodeo_proto::BUILD_ID, port)))
}

/// Install this backend's plugin file, with the build id and `port` baked in
/// so the plugin knows which backend is its own. Call it only once the
/// backend has bound `port`: the file must never describe a backend that
/// failed to start. Byte-idempotent (see `plugin_embed::write_plugin`).
pub(crate) fn install_plugin(port: u16) -> Result<PathBuf> {
    let path = plugin_path(port)?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).context("failed to create plugins directory")?;
    }
    plugin_embed::write_plugin(&path, rodeo_proto::BUILD_ID, port)?;
    Ok(path)
}

/// How long after writing a plugin file it is safe to launch a Studio that
/// will load it. macOS delivers the file-system "add" event roughly 1.2 s
/// after the write; a Studio spawned inside that window starts watching the
/// plugins folder just in time to receive it, and hot-reloads the plugin
/// while its edit DataModel is still initializing. In testing that reload
/// crashed Studio in roughly one fresh launch in eight, and every crash was
/// a launch that had it. Waiting out the window lets the event land before
/// the Studio exists.
const PLUGIN_SETTLE: std::time::Duration = std::time::Duration::from_millis(2500);

/// Block until this backend's plugin file is at least [`PLUGIN_SETTLE`] old.
/// A no-op unless the file was just written. A backend leaves its file
/// installed when it exits (see `plugin_sweep::KEEP_FOR`), so the next serve of
/// the same build on that port finds it unchanged and doesn't wait: only a new
/// build or port, or a serve after the keep window, pays ~2.5 s, on its first
/// launch. Runs on the launch's blocking thread.
fn settle_plugin_file(port: u16) {
    let Ok(path) = plugin_path(port) else { return };
    let age = std::fs::metadata(&path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.elapsed().ok());
    if let Some(age) = age {
        if age < PLUGIN_SETTLE {
            let wait = PLUGIN_SETTLE - age;
            tracing::info!(wait_ms = wait.as_millis() as u64, "spawn: plugin file is fresh; letting its file-system event land before Studio starts");
            std::thread::sleep(wait);
        }
    }
}

/// Remove a per-backend plugin file together with the IDE-state files Studio
/// keeps for it and the record that its backend left it installed. Missing
/// files are not an error. Studio unloads a plugin the moment its file
/// disappears, so only the start-time sweep (`plugin_sweep`) calls this.
pub(crate) fn remove_plugin_file(path: &Path) -> std::io::Result<()> {
    match std::fs::remove_file(path) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e),
    }
    super::plugin_sweep::clear_kept(path);
    if let (Some(dir), Some(name)) = (plugin_ide_state_dir(), path.file_name().and_then(|n| n.to_str())) {
        remove_ide_state_in(&dir, name);
    }
    Ok(())
}

/// Studio's per-plugin IDE-state directory.
///
/// For every local plugin file Studio writes `pluginIDEState_user_<file>_<n>.xml`
/// (which of the plugin's scripts are open in the script editor) and
/// `pluginIDEState_user_<file>_<n>_DebuggerData.xml` (breakpoints set in
/// them) when the plugin unloads, and restores them when it loads, so a
/// plugin developer keeps tabs and breakpoints across reloads. rodeo's plugin
/// scripts are never opened, so for us the pair is two empty shells — but
/// Studio writes one pair per plugin *file name*, and per-backend names would
/// accumulate them forever, so they go when the plugin file goes.
fn plugin_ide_state_dir() -> Option<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        std::env::var_os("HOME").map(|h| PathBuf::from(h).join("Library/Roblox/pluginIDEState"))
    }
    #[cfg(target_os = "windows")]
    {
        std::env::var_os("LOCALAPPDATA").map(|p| PathBuf::from(p).join("Roblox").join("pluginIDEState"))
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        None
    }
}

/// The per-backend plugin file an IDE-state file belongs to:
/// `pluginIDEState_user_rodeo-<build>-<port>.rbxm_0_DebuggerData.xml` →
/// `rodeo-<build>-<port>.rbxm`. `None` for anything else, including the
/// legacy `rodeo.rbxm`'s files, which stay Studio's business.
pub(crate) fn plugin_file_of_ide_state(name: &str) -> Option<String> {
    let rest = name.strip_prefix("pluginIDEState_user_")?;
    let end = rest.find(".rbxm_")? + ".rbxm".len();
    let file = &rest[..end];
    parse_plugin_file_name(file)?;
    Some(file.to_string())
}

/// Remove the IDE-state files Studio wrote for `plugin_file_name`. Returns
/// how many were removed.
fn remove_ide_state_in(dir: &Path, plugin_file_name: &str) -> usize {
    let prefix = format!("pluginIDEState_user_{plugin_file_name}_");
    let Ok(read) = std::fs::read_dir(dir) else { return 0 };
    read.flatten()
        .filter(|e| e.file_name().to_str().is_some_and(|n| n.starts_with(&prefix)))
        .filter(|e| std::fs::remove_file(e.path()).is_ok())
        .count()
}

/// Remove IDE-state files for per-backend plugin files that no longer exist
/// in `plugins_dir` — left by a crashed backend, or from before this cleanup
/// existed. Returns how many were removed.
pub(crate) fn sweep_orphaned_ide_state(plugins_dir: &Path) -> usize {
    plugin_ide_state_dir().map_or(0, |dir| sweep_orphaned_ide_state_in(&dir, plugins_dir))
}

fn sweep_orphaned_ide_state_in(ide_dir: &Path, plugins_dir: &Path) -> usize {
    let Ok(read) = std::fs::read_dir(ide_dir) else { return 0 };
    read.flatten()
        .filter(|e| {
            e.file_name()
                .to_str()
                .and_then(plugin_file_of_ide_state)
                .is_some_and(|file| !plugins_dir.join(file).exists())
        })
        .filter(|e| std::fs::remove_file(e.path()).is_ok())
        .count()
}

/// The Qtitan dock-panel id Studio assigns this backend's plugin widget, for
/// `--show-widgets`. Studio derives it from the plugin file name and the id
/// the plugin passes to `CreateDockWidgetPluginGui` (`Rodeo-<port>`), both of
/// which now vary per backend — hence the `rodeo` alias below.
pub(crate) fn plugin_panel_id(port: u16) -> String {
    format!("edit_user_{}_Rodeo-{}", plugin_file_name(rodeo_proto::BUILD_ID, port), port)
}

/// Expand the `rodeo` alias in a `--show-widgets` list to this backend's own
/// dock-panel id. Everything else passes through to rbx-control's parser.
fn expand_show_widgets(spec: &str, port: u16) -> String {
    spec.split(',')
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(|item| {
            if item.eq_ignore_ascii_case("rodeo") {
                plugin_panel_id(port)
            } else {
                item.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join(",")
}

/// Write the per-launch RunScript bootstrap to the temp dir. Run by Studio at
/// launch (command-bar identity), it stamps `rodeoSession`/`rodeoPort` onto the
/// Workspace so the static plugin connects to this launch's serve port and
/// reports its session on the WS handshake, and installs the command-bar
/// bridge that `--context cmdbar` runs through.
fn write_bootstrap_script(session_guid: &str, port: u16) -> Result<PathBuf> {
    // Absolute path: Studio's working directory differs from rodeo's, so
    // `-runScriptFile` must be absolute (a relative path makes Studio report
    // "Failed to read script file"). current_dir().join avoids the Windows
    // `\\?\` prefix that canonicalize would add.
    let temp_dir = std::env::current_dir()
        .context("failed to resolve current dir")?
        .join(".rodeo/.temp");
    std::fs::create_dir_all(&temp_dir).context("failed to create temp dir")?;
    let path = temp_dir.join(format!("rodeo-bootstrap-{session_guid}.luau"));
    // session_guid is a master-minted UUID (no quotes/backslashes), so embedding
    // it in a string literal is safe.
    std::fs::write(&path, bootstrap_source(session_guid, port, rodeo_proto::BUILD_ID))
        .context("failed to write bootstrap script")?;
    Ok(path)
}

/// Name of the BindableFunction the bootstrap parents under CoreGui. The
/// plugin's runner looks it up by this name for `--context cmdbar`.
const CMDBAR_BRIDGE_NAME: &str = "rodeoCmdbar";

/// The bootstrap's Luau source. Studio runs `-runScriptFile` once in the edit
/// DOM at command-bar identity (4 — verified on 0.738: `DebuggerManager()`
/// works there, `script`/`plugin` are nil, yields work). Two jobs:
///
/// 1. Stamp the launch attributes the static plugin reads.
/// 2. Install the command-bar bridge: a BindableFunction whose `OnInvoke`
///    keeps its creator's identity when the plugin (identity 5) invokes it, so
///    `--context cmdbar` runs the user module at command-bar identity with no
///    StudioMCP hop. The edit DOM's identities share one Luau VM, so the
///    result is handed back through the plugin's namespace in `_G`
///    (`_G.__rodeo_plugins[<build>-<port>].cmdbar`) rather than the Bindable
///    return (which deep-copies tables and rejects functions).
///    `Archivable = false` under CoreGui keeps it out of saves and out of
///    play-mode clones.
pub(crate) fn bootstrap_source(session_guid: &str, port: u16, build: &str) -> String {
    format!(
        r#"local ws = game:GetService("Workspace")
ws:SetAttribute("rodeoSession", "{session_guid}")
ws:SetAttribute("rodeoPort", {port})

local bridge = Instance.new("BindableFunction")
bridge.Name = "{bridge}"
bridge.Archivable = false
bridge:SetAttribute("rodeoSession", "{session_guid}")
bridge:SetAttribute("rodeoBuild", "{build}")
bridge.OnInvoke = function(module, executionId)
	local ok, result = xpcall(require, function(err)
		return tostring(err) .. "\n" .. debug.traceback(nil, 2)
	end, module)
	local plugins = _G.__rodeo_plugins or {{}}
	_G.__rodeo_plugins = plugins
	local ns = plugins["{key}"] or {{ procs = {{}}, cmdbar = {{}} }}
	plugins["{key}"] = ns
	ns.cmdbar[executionId] = {{ ok = ok, result = result }}
	return ok
end
bridge.Parent = game:GetService("CoreGui")
"#,
        bridge = CMDBAR_BRIDGE_NAME,
        key = plugin_key(build, port),
    )
}

/// The plugin instance's key in the edit VM's `_G.__rodeo_plugins`, which
/// the bootstrap's bridge uses to hand cmdbar results to the right plugin.
/// Must match `constants.PLUGIN_KEY` in the plugin: `<build>-<port>`.
fn plugin_key(build: &str, port: u16) -> String {
    format!("{build}-{port}")
}

#[cfg(test)]
mod bootstrap_tests {
    use super::*;

    #[test]
    fn bootstrap_stamps_attributes_and_installs_cmdbar_bridge() {
        let src = bootstrap_source("abc-123", 44901, "1.2.3+abcdef0");
        assert!(src.contains(r#"ws:SetAttribute("rodeoSession", "abc-123")"#), "{src}");
        assert!(src.contains(r#"ws:SetAttribute("rodeoPort", 44901)"#), "{src}");
        assert!(src.contains(r#"bridge.Name = "rodeoCmdbar""#), "{src}");
        assert!(src.contains(r#"bridge:SetAttribute("rodeoBuild", "1.2.3+abcdef0")"#), "{src}");
        // The bridge hands results to the plugin instance's namespace, keyed
        // exactly as constants.PLUGIN_KEY builds it.
        assert!(src.contains(r#"plugins["1.2.3+abcdef0-44901"]"#), "{src}");
        assert!(src.contains("bridge.Archivable = false"), "{src}");
        assert!(src.contains(r#"bridge.Parent = game:GetService("CoreGui")"#), "{src}");
        // The Luau source must carry a real "\n" escape, not a raw newline.
        assert!(src.contains(r#".. "\n" .."#), "{src}");
    }
}

/// Absolutize a path against the backend's CWD for display in `rodeo state`.
/// `current_dir().join` rather than `canonicalize` — same reason as the
/// bootstrap path: canonicalize adds the Windows `\\?\` verbatim prefix.
fn absolutize(p: &Path) -> String {
    if p.is_absolute() {
        return p.to_string_lossy().to_string();
    }
    std::env::current_dir()
        .map(|cwd| cwd.join(p))
        .unwrap_or_else(|_| p.to_path_buf())
        .to_string_lossy()
        .to_string()
}

/// Prepare a place file for Studio based on SaveMode. The place contents are
/// never mutated — routing happens at runtime via the RunScript bootstrap, so
/// the original file opens unchanged.
fn prepare_place(place: Option<&str>, save: &SaveMode) -> Result<PathBuf> {
    // A non-empty path is a request to open THAT file. Refuse anything that
    // isn't one before touching the filesystem: this used to fall through to
    // the empty-place branch, so a path the backend couldn't see (relative to
    // the client's cwd, a typo, a directory) silently opened a blank place and
    // the script failed later on a missing DataModel (issue #1). The CLI and
    // rodeo-client absolutize paths before sending; this catches everything
    // else (hand-rolled clients, MCP) and any drift.
    let place = place.filter(|p| !p.is_empty());
    if let Some(p) = place {
        let path = Path::new(p);
        if path.is_dir() {
            bail!("place path '{p}' is a directory, expected an .rbxl/.rbxlx file");
        }
        if !path.is_file() {
            let cwd = std::env::current_dir()
                .map(|c| c.display().to_string())
                .unwrap_or_else(|_| "?".to_string());
            let hint = if path.is_absolute() {
                String::new()
            } else {
                format!(" (relative paths resolve against the studio backend's cwd, {cwd}; pass an absolute path)")
            };
            bail!("place file '{p}' not found{hint}");
        }
        // The old DOM-parse-and-stamp step implicitly rejected non-place
        // files; without it a corrupted file would copy fine and Studio would
        // hang opening garbage instead of failing fast.
        validate_place_file(p)?;
    }
    let has_place = place.is_some();

    let temp_dir = Path::new(".rodeo/.temp");
    std::fs::create_dir_all(temp_dir).context("failed to create temp dir")?;

    match save {
        SaveMode::NoSave => {
            // Copy to a temp file so any in-Studio edits never touch the original.
            let ext = if has_place {
                Path::new(place.unwrap())
                    .extension()
                    .and_then(|e| e.to_str())
                    .unwrap_or("rbxl")
            } else {
                "rbxl"
            };
            let temp_path = temp_dir.join(format!("rodeo-{}.{}", uuid::Uuid::new_v4(), ext));
            if has_place {
                std::fs::copy(place.unwrap(), &temp_path)
                    .context("failed to copy place file")?;
            } else {
                let dom = create_minimal_place();
                rbx_control::studio::launch::write_place(&dom, &temp_path)?;
            }
            Ok(temp_path)
        }
        SaveMode::SaveInPlace => {
            if has_place {
                // Open the user's file directly so Studio saves back to it.
                Ok(PathBuf::from(place.unwrap()))
            } else {
                let temp_path = temp_dir.join(format!("rodeo-{}.rbxl", uuid::Uuid::new_v4()));
                let dom = create_minimal_place();
                rbx_control::studio::launch::write_place(&dom, &temp_path)?;
                Ok(temp_path)
            }
        }
        SaveMode::SaveToPath(out) => {
            let out_path = PathBuf::from(out);
            if let Some(parent) = out_path.parent() {
                if !parent.as_os_str().is_empty() {
                    std::fs::create_dir_all(parent)
                        .context("failed to create output directory")?;
                }
            }
            if has_place {
                std::fs::copy(place.unwrap(), &out_path)
                    .context("failed to copy place file")?;
            } else {
                let dom = create_minimal_place();
                rbx_control::studio::launch::write_place(&dom, &out_path)?;
            }
            Ok(out_path)
        }
    }
}

#[cfg(test)]
mod plugin_file_tests {
    use super::*;

    #[test]
    fn plugin_file_name_round_trips_builds_with_dashes_and_plus() {
        for (build, port) in [("1.4.0-rc.4+b493edb", 44873u16), ("2.0.0", 46001), ("1.5.0-rc.1+abc-def", 1)] {
            let name = plugin_file_name(build, port);
            assert_eq!(parse_plugin_file_name(&name), Some((build.to_string(), port)), "{name}");
        }
    }

    #[test]
    fn parse_ignores_everything_else_in_the_plugins_directory() {
        for name in ["rodeo.rbxm", "rodeo-44881.lock", "RojoManagedPlugin.rbxm", "rodeo-.rbxm", "rodeo-1.2.3-notaport.rbxm", "rodeo-1.2.3-44873.rbxmx", ".DS_Store"] {
            assert_eq!(parse_plugin_file_name(name), None, "{name}");
        }
    }

    #[test]
    fn ide_state_files_map_back_to_our_plugin_files_only() {
        assert_eq!(
            plugin_file_of_ide_state("pluginIDEState_user_rodeo-1.4.0-rc.4+05b88e1-46299.rbxm_0.xml").as_deref(),
            Some("rodeo-1.4.0-rc.4+05b88e1-46299.rbxm")
        );
        assert_eq!(
            plugin_file_of_ide_state("pluginIDEState_user_rodeo-1.4.0-rc.4+05b88e1-46299.rbxm_0_DebuggerData.xml").as_deref(),
            Some("rodeo-1.4.0-rc.4+05b88e1-46299.rbxm")
        );
        for other in [
            "pluginIDEState_user_rodeo.rbxm_0.xml",            // legacy shared plugin: Studio's business
            "pluginIDEState_user_RojoManagedPlugin.rbxm_0.xml", // someone else's plugin
            "pluginIDEState_cloud_12345_0.xml",
            ".DS_Store",
        ] {
            assert_eq!(plugin_file_of_ide_state(other), None, "{other}");
        }
    }

    #[test]
    fn ide_state_cleanup_removes_only_the_named_plugins_files() {
        let base = std::env::temp_dir().join(format!("rodeo-ide-state-test-{}", std::process::id()));
        let ide = base.join("pluginIDEState");
        let plugins = base.join("Plugins");
        std::fs::create_dir_all(&ide).unwrap();
        std::fs::create_dir_all(&plugins).unwrap();
        let touch = |dir: &Path, name: &str| std::fs::write(dir.join(name), b"x").unwrap();
        touch(&ide, "pluginIDEState_user_rodeo-1.2.3+abc-46001.rbxm_0.xml");
        touch(&ide, "pluginIDEState_user_rodeo-1.2.3+abc-46001.rbxm_0_DebuggerData.xml");
        touch(&ide, "pluginIDEState_user_rodeo-1.2.3+abc-46003.rbxm_0.xml");
        touch(&ide, "pluginIDEState_user_rodeo-1.2.3+abc-46003.rbxm_0_DebuggerData.xml");
        touch(&ide, "pluginIDEState_user_rodeo.rbxm_0.xml");
        touch(&ide, "pluginIDEState_user_RojoManagedPlugin.rbxm_0.xml");

        // Removing one plugin's state leaves every other file alone.
        assert_eq!(remove_ide_state_in(&ide, "rodeo-1.2.3+abc-46001.rbxm"), 2);
        assert_eq!(std::fs::read_dir(&ide).unwrap().count(), 4);

        // The orphan sweep removes state only for our plugin files that are
        // gone: 46003 has no plugin file, so its pair goes; the legacy and
        // foreign files stay regardless.
        touch(&plugins, "rodeo-1.2.3+abc-46005.rbxm");
        touch(&ide, "pluginIDEState_user_rodeo-1.2.3+abc-46005.rbxm_0.xml");
        assert_eq!(sweep_orphaned_ide_state_in(&ide, &plugins), 2);
        let left: Vec<String> = std::fs::read_dir(&ide).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
        assert_eq!(left.len(), 3, "{left:?}");
        assert!(left.iter().any(|n| n.contains("46005")), "{left:?}");
        assert!(left.iter().any(|n| n == "pluginIDEState_user_rodeo.rbxm_0.xml"), "{left:?}");
        assert!(left.iter().any(|n| n.contains("RojoManagedPlugin")), "{left:?}");
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn show_widgets_rodeo_alias_expands_to_this_backends_panel() {
        let expanded = expand_show_widgets("output, Rodeo ,commandbar", 46001);
        assert_eq!(expanded, format!("output,{},commandbar", plugin_panel_id(46001)));
        assert!(plugin_panel_id(46001).starts_with("edit_user_rodeo-"), "{}", plugin_panel_id(46001));
        assert!(plugin_panel_id(46001).ends_with("-46001.rbxm_Rodeo-46001"), "{}", plugin_panel_id(46001));
        // Untouched when the alias is absent.
        assert_eq!(expand_show_widgets("output,explorer", 46001), "output,explorer");
    }
}

#[cfg(test)]
mod prepare_place_tests {
    use super::*;

    #[test]
    fn missing_place_file_is_an_error_not_a_blank_place() {
        let err = prepare_place(Some("/definitely/not/here.rbxl"), &SaveMode::NoSave)
            .expect_err("missing place must fail");
        assert!(err.to_string().contains("not found"), "{err}");
    }

    #[test]
    fn relative_missing_path_hints_at_backend_cwd() {
        let err = prepare_place(Some("build.rbxl"), &SaveMode::NoSave)
            .expect_err("missing place must fail");
        let msg = err.to_string();
        assert!(msg.contains("not found") && msg.contains("absolute path"), "{msg}");
    }

    #[test]
    fn directory_is_rejected() {
        let dir = std::env::temp_dir();
        let err = prepare_place(Some(dir.to_str().unwrap()), &SaveMode::NoSave)
            .expect_err("directory must fail");
        assert!(err.to_string().contains("is a directory"), "{err}");
    }
}

/// Create a minimal DataModel with an empty Workspace, for empty-place launches
/// (gives RunScript a file to open).
fn create_minimal_place() -> WeakDom {
    let workspace = InstanceBuilder::new("Workspace");
    WeakDom::new(InstanceBuilder::new("DataModel").with_child(workspace))
}

/// Cheap, read-only sanity check that a path looks like a Roblox place file —
/// binary `rbxl` (magic `<roblox!`) or XML `rbxlx` (`<roblox` / `<?xml`). Just
/// the header, no full DOM parse: enough to fail fast on a corrupted/non-place
/// file rather than hand garbage to Studio and hang waiting for a connection.
fn validate_place_file(path: &str) -> Result<()> {
    use std::io::Read;
    let mut head = [0u8; 8];
    let mut f = std::fs::File::open(path).context("failed to open place file")?;
    let n = f.read(&mut head).context("failed to read place file")?;
    let head = &head[..n];
    if head.starts_with(b"<roblox") || head.starts_with(b"<?xml") {
        Ok(())
    } else {
        bail!("failed to parse place file (not a valid rbxl/rbxlx): {path}");
    }
}
