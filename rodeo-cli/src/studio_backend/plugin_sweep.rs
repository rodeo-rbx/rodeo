//! Start-time sweep of per-backend plugin files.
//!
//! Each studio backend installs `rodeo-<build>-<port>.rbxm` into Studio's
//! plugins directory once it has bound its port. On a clean exit it leaves the
//! file installed and records that it did (`mark_kept`): the next serve of the
//! same build on that port finds the file unchanged and launches Studio at
//! once, instead of waiting out a freshly written file (see
//! `launch::settle_plugin_file`). Every backend runs this sweep right after
//! binding, so the files of backends that are gone and that nothing still
//! needs get cleaned up by the next backend to start.
//!
//! Only files matching that name shape are considered. The legacy `rodeo.rbxm`
//! that pre-1.5 rodeo versions write is left alone: those versions and their
//! open Studios depend on it, and they may be running on this machine.
//!
//! Deleting a plugin file unloads the plugin from every open Studio at once,
//! so a file goes only when no backend answers on its master port (the plugin
//! port minus one), no running Studio was launched against that port, *and*
//! its backend did not leave it less than [`KEEP_FOR`] ago (a crashed backend
//! leaves no record, so its file goes at once). A backend of a different build
//! answering there means the port has changed hands, and the file is stale
//! regardless: the new backend's own plugin takes those Studios over by port.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::Duration;

use rodeo_client::RodeoClient;

use super::launch::{parse_plugin_file_name, plugins_dir, remove_plugin_file, sweep_orphaned_ide_state};

/// Files younger than this are skipped: a sibling backend may be mid-start,
/// its file written but its master not yet answering.
pub const MIN_AGE: Duration = Duration::from_secs(5);
/// How long a master gets to answer a health probe before its port counts as
/// unanswered.
const PROBE_TIMEOUT: Duration = Duration::from_secs(2);
/// The shared plugin file pre-1.5 versions write. Never read, written, or
/// removed by this build.
pub const LEGACY_PLUGIN_FILE: &str = "rodeo.rbxm";
/// How long a plugin file its backend left on a clean exit stays installed
/// for the next serve on its port. Until then, Studios opened meanwhile also
/// load it, and it redials the stopped port.
pub const KEEP_FOR: Duration = Duration::from_secs(60 * 60);

/// The record that a backend left `plugin_file` installed on a clean exit:
/// an empty file whose modification time is when. Kept outside the plugins
/// directory, since any write there makes every open Studio reload plugins.
fn kept_marker(plugin_file: &Path) -> Option<PathBuf> {
    let name = plugin_file.file_name()?;
    Some(dirs::cache_dir()?.join("rodeo").join("kept-plugins").join(name))
}

/// Record that this backend is exiting and leaves `plugin_file` installed.
pub fn mark_kept(plugin_file: &Path) -> std::io::Result<()> {
    let Some(marker) = kept_marker(plugin_file) else { return Ok(()) };
    if let Some(dir) = marker.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(marker, b"")
}

/// Drop the record for `plugin_file`: a backend owns it again, or it is gone.
pub fn clear_kept(plugin_file: &Path) {
    if let Some(marker) = kept_marker(plugin_file) {
        let _ = std::fs::remove_file(marker);
    }
}

/// How long ago a backend left `plugin_file` on a clean exit, if it did.
fn kept_for(plugin_file: &Path) -> Option<Duration> {
    let marker = kept_marker(plugin_file)?;
    std::fs::metadata(marker).and_then(|m| m.modified()).ok()?.elapsed().ok()
}

/// One `rodeo-<build>-<port>.rbxm` found in the plugins directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub path: PathBuf,
    pub build: String,
    pub port: u16,
    pub age: Duration,
    /// How long ago its backend left it on a clean exit; `None` if no backend
    /// did (it crashed, or the file predates the record).
    pub kept_for: Option<Duration>,
}

/// What answered a master's health probe.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Probe {
    Healthy { version: String },
    NoAnswer,
}

/// The master port for a plugin (studio backend) port: a serve is a master
/// on `p` with its backend on `p + 1`.
fn master_port(plugin_port: u16) -> u16 {
    plugin_port.saturating_sub(1)
}

/// Which files to delete. `probes` is keyed by master port; a port missing
/// from it counts as unanswered. `live_ports` are plugin ports still
/// referenced by a running Studio's launch bootstrap. A file its backend left
/// less than [`KEEP_FOR`] ago stays for the next serve on its port.
pub fn sweep_decisions(
    entries: &[Entry],
    probes: &HashMap<u16, Probe>,
    live_ports: &HashSet<u16>,
) -> Vec<PathBuf> {
    entries
        .iter()
        .filter(|e| e.age >= MIN_AGE)
        .filter(|e| match probes.get(&master_port(e.port)) {
            Some(Probe::Healthy { version }) => version != &e.build,
            Some(Probe::NoAnswer) | None => {
                !live_ports.contains(&e.port) && !e.kept_for.is_some_and(|k| k < KEEP_FOR)
            }
        })
        .map(|e| e.path.clone())
        .collect()
}

/// Bootstrap files named on running Studios' command lines. Every rodeo
/// launch passes `-runScriptFile <cwd>/.rodeo/.temp/rodeo-bootstrap-<session>.luau`;
/// matching on that fixed shape keeps directories with spaces intact.
pub fn bootstrap_paths(cmdlines: &[String]) -> Vec<PathBuf> {
    let re = regex::Regex::new(
        r#"-runScriptFile\s+"?(.+?[/\\]\.rodeo[/\\]\.temp[/\\]rodeo-bootstrap-[0-9A-Za-z-]+\.luau)"#,
    )
    .expect("static regex");
    cmdlines
        .iter()
        .filter_map(|c| re.captures(c))
        .map(|c| PathBuf::from(&c[1]))
        .collect()
}

/// The plugin port a bootstrap stamps (`ws:SetAttribute("rodeoPort", N)`).
pub fn port_from_bootstrap(source: &str) -> Option<u16> {
    let re = regex::Regex::new(r#"SetAttribute\("rodeoPort",\s*(\d+)\)"#).expect("static regex");
    re.captures(source)?.get(1)?.as_str().parse().ok()
}

/// Plugin ports still in use by a running Studio.
fn live_ports() -> HashSet<u16> {
    let Ok(app) = rbx_control::studio::launch::studio_application_path() else {
        return HashSet::new();
    };
    let cmdlines = launch_control::running_instance_cmdlines(Path::new(&app));
    bootstrap_paths(&cmdlines)
        .into_iter()
        .filter_map(|p| std::fs::read_to_string(p).ok())
        .filter_map(|s| port_from_bootstrap(&s))
        .collect()
}

fn scan(dir: &Path) -> Vec<Entry> {
    let Ok(read) = std::fs::read_dir(dir) else { return Vec::new() };
    read.filter_map(|e| e.ok())
        .filter_map(|e| {
            let name = e.file_name();
            let (build, port) = parse_plugin_file_name(name.to_str()?)?;
            let age = e
                .metadata()
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| t.elapsed().ok())
                .unwrap_or(MIN_AGE);
            let path = e.path();
            let kept_for = kept_for(&path);
            Some(Entry { path, build, port, age, kept_for })
        })
        .collect()
}

async fn probe(master_port: u16) -> Probe {
    let Ok(client) = RodeoClient::connect("localhost", master_port) else {
        return Probe::NoAnswer;
    };
    match tokio::time::timeout(PROBE_TIMEOUT, client.health()).await {
        Ok(Ok(health)) => Probe::Healthy { version: health.version },
        _ => Probe::NoAnswer,
    }
}

/// Remove plugin files whose backend is gone and that no Studio still uses.
/// Failures are logged, never fatal: a leftover file costs a parked
/// connection, not a broken serve.
pub async fn sweep() {
    let dir = match plugins_dir() {
        Ok(dir) => dir,
        Err(e) => {
            tracing::debug!("plugin sweep skipped: {e}");
            return;
        }
    };
    if dir.join(LEGACY_PLUGIN_FILE).exists() {
        tracing::info!(
            "legacy plugin file {LEGACY_PLUGIN_FILE} present: older rodeo versions use it and this build never touches it; delete it by hand once no pre-1.5 rodeo remains"
        );
    }
    let entries = scan(&dir);
    if entries.is_empty() {
        let orphaned = sweep_orphaned_ide_state(&dir);
        if orphaned > 0 {
            tracing::info!(count = orphaned, "removed Studio IDE-state files for plugin files that no longer exist");
        }
        return;
    }

    let mut probes = HashMap::new();
    for entry in entries.iter().filter(|e| e.age >= MIN_AGE) {
        let mp = master_port(entry.port);
        if !probes.contains_key(&mp) {
            probes.insert(mp, probe(mp).await);
        }
    }
    let live = tokio::task::spawn_blocking(live_ports).await.unwrap_or_default();

    for path in sweep_decisions(&entries, &probes, &live) {
        match remove_plugin_file(&path) {
            Ok(()) => tracing::info!(path = %path.display(), "removed stale plugin file (its backend is gone and nothing still uses it)"),
            Err(e) => tracing::warn!(path = %path.display(), "failed to remove stale plugin file: {e}"),
        }
    }

    // Studio's per-plugin IDE-state files outlive a plugin file when the
    // backend crashed (or predate this cleanup); drop the ones whose plugin
    // file is gone.
    let orphaned = sweep_orphaned_ide_state(&dir);
    if orphaned > 0 {
        tracing::info!(count = orphaned, "removed Studio IDE-state files for plugin files that no longer exist");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(build: &str, port: u16, age_secs: u64) -> Entry {
        Entry {
            path: PathBuf::from(format!("rodeo-{build}-{port}.rbxm")),
            build: build.to_string(),
            port,
            age: Duration::from_secs(age_secs),
            kept_for: None,
        }
    }

    fn kept(mut e: Entry, kept_secs: u64) -> Entry {
        e.kept_for = Some(Duration::from_secs(kept_secs));
        e
    }

    #[test]
    fn decisions_cover_every_case() {
        let entries = vec![
            entry("A", 46001, 60), // live: same build answers → keep
            entry("B", 46011, 60), // a different build answers on the port → delete
            entry("C", 46021, 60), // no answer, a Studio still references it → keep
            entry("D", 46031, 60), // no answer, nothing references it → delete
            entry("E", 46041, 1),  // too young to judge → keep
        ];
        let probes = HashMap::from([
            (46000, Probe::Healthy { version: "A".into() }),
            (46010, Probe::Healthy { version: "Z".into() }),
            (46020, Probe::NoAnswer),
            // 46030 absent: counts as unanswered
        ]);
        let live = HashSet::from([46021]);
        let deleted = sweep_decisions(&entries, &probes, &live);
        assert_eq!(
            deleted,
            vec![PathBuf::from("rodeo-B-46011.rbxm"), PathBuf::from("rodeo-D-46031.rbxm")]
        );
    }

    #[test]
    fn a_file_left_on_exit_stays_for_the_keep_window() {
        let window = KEEP_FOR.as_secs();
        let entries = vec![
            kept(entry("A", 46101, 60), 60),          // left a minute ago → keep
            kept(entry("B", 46111, 60), window + 60), // left past the window → delete
            kept(entry("C", 46121, 60), 60),          // left recently, but the port changed hands → delete
        ];
        let probes = HashMap::from([
            (46100, Probe::NoAnswer),
            (46110, Probe::NoAnswer),
            (46120, Probe::Healthy { version: "Z".into() }),
        ]);
        let deleted = sweep_decisions(&entries, &probes, &HashSet::new());
        assert_eq!(
            deleted,
            vec![PathBuf::from("rodeo-B-46111.rbxm"), PathBuf::from("rodeo-C-46121.rbxm")]
        );
    }

    #[test]
    fn bootstrap_paths_survive_spaces_and_quotes() {
        let cmdlines = vec![
            "/Applications/RobloxStudio.app/Contents/MacOS/RobloxStudio -task RunScript -localPlaceFile /tmp/x.rbxl -runScriptFile /Users/me/My Projects/game/.rodeo/.temp/rodeo-bootstrap-2b1c-4e5f.luau -parentPid 42".to_string(),
            r#""C:\Program Files\Roblox\RobloxStudioBeta.exe" -task RunScript -runScriptFile "C:\Users\me\my game\.rodeo\.temp\rodeo-bootstrap-abc.luau""#.to_string(),
            "/Applications/RobloxStudio.app/Contents/MacOS/RobloxStudio".to_string(), // hand-opened
        ];
        let paths = bootstrap_paths(&cmdlines);
        assert_eq!(
            paths,
            vec![
                PathBuf::from("/Users/me/My Projects/game/.rodeo/.temp/rodeo-bootstrap-2b1c-4e5f.luau"),
                PathBuf::from(r"C:\Users\me\my game\.rodeo\.temp\rodeo-bootstrap-abc.luau"),
            ]
        );
    }

    #[test]
    fn port_parses_from_the_bootstrap_this_build_writes() {
        let src = super::super::launch::bootstrap_source("sess", 46801, "1.5.0+abc1234");
        assert_eq!(port_from_bootstrap(&src), Some(46801));
        assert_eq!(port_from_bootstrap("print('no port here')"), None);
    }
}
