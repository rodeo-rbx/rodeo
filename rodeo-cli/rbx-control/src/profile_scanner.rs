//! Profile dump scanner — watches ProfilerCaptures directory for .raw/.html dumps
//! that contain a matching `{label_prefix}:{run_id}` label, and sends them to the caller.
//! Uses `notify-debouncer-full` for deduplicated, stable file detection.
//!
//! Studio's auto-capture writes a dump every 60 frames for as long as a
//! profiled Studio runs, between runs too, so every scanner also deletes
//! auto-capture dumps older than [`DUMP_MAX_AGE`]. Deleting by age rather than
//! on delivery leaves a dump for every scanner that needs it: each serve's
//! backend runs its own scanner over the same machine-wide directory, and one
//! Studio can carry runs from several serves.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};
use notify_debouncer_full::{new_debouncer, DebounceEventResult};
use tokio::sync::mpsc;

/// A completed run stays registered until its Studio's capture has moved past
/// it, but gives up once this long passes with no dump for it: the dump
/// holding a run's last frames lands after the run completes (it's written
/// when its 60-frame window closes, then debounced), and a Studio may have
/// closed or never have captured the run's frames at all.
const DRAIN_QUIET: Duration = Duration::from_secs(5);
/// Upper bound on a completed run's drain however many dumps keep matching.
const DRAIN_MAX: Duration = Duration::from_secs(30);
/// A completed run with no dumps yet waits only if some Studio wrote an
/// auto-capture dump this recently; otherwise nothing is capturing (e.g. a
/// `--profile` run on a Studio launched without the capture flags) and
/// waiting would only delay the run's completion.
const RECENT_CAPTURE: Duration = Duration::from_secs(10);
/// An auto-capture dump older than this is deleted. Scanners read a dump
/// within a second of Studio writing it, so by then every run that needed it
/// has it.
pub const DUMP_MAX_AGE: Duration = Duration::from_secs(5 * 60);
/// How often a scanner looks for dumps past [`DUMP_MAX_AGE`].
const SWEEP_INTERVAL: Duration = Duration::from_secs(60);

/// A matched profile dump ready to be sent.
#[derive(Debug)]
pub struct ProfileDump {
    pub execution_id: String,
    pub filename: String,
    pub data: Vec<u8>,
}

fn modified(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).ok()?.modified().ok()
}

/// What an auto-capture filename says about its dump:
/// `AutoCapture_<capture start>_Frames-<first>-<last>.raw`. The capture-start
/// prefix is per Studio process, and each process writes its dumps in frame
/// order.
#[derive(Debug, PartialEq)]
struct AutoCaptureName<'a> {
    process: &'a str,
    first_frame: u64,
    last_frame: u64,
}

fn parse_auto_capture(filename: &str) -> Option<AutoCaptureName<'_>> {
    let stem = filename.strip_suffix(".raw")?;
    if !stem.starts_with("AutoCapture_") {
        return None;
    }
    let (process, frames) = stem.rsplit_once("_Frames-")?;
    let (first, last) = frames.split_once('-')?;
    Some(AutoCaptureName { process, first_frame: first.parse().ok()?, last_frame: last.parse().ok()? })
}

/// Get the default ProfilerCaptures directory.
fn profiler_captures_dir() -> Option<PathBuf> {
    crate::paths::profiler_captures_dir()
}

/// Delete the auto-capture dumps (`AutoCapture_*.raw`) in `dir` written more
/// than `max_age` before `now`. Only Studio's auto-capture output, which only
/// rodeo's profiling turns on: a dump someone saved by hand stays. Another
/// scanner may delete the same file first; that's not an error.
fn sweep_old_dumps(dir: &Path, max_age: Duration, now: SystemTime) -> usize {
    let Ok(entries) = std::fs::read_dir(dir) else { return 0 };
    let mut removed = 0;
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else { continue };
        if parse_auto_capture(name).is_none() {
            continue;
        }
        let old = modified(&path).and_then(|t| now.duration_since(t).ok()).is_some_and(|age| age > max_age);
        if old && std::fs::remove_file(&path).is_ok() {
            removed += 1;
        }
    }
    removed
}

/// Check if a file's binary content contains the label for `execution_id`.
fn file_contains_label(data: &[u8], execution_id: &str, label_prefix: &str) -> bool {
    let needle = format!("{label_prefix}:{execution_id}");
    data.windows(needle.len()).any(|w| w == needle.as_bytes())
}

/// A registered run.
struct Run {
    tx: mpsc::UnboundedSender<ProfileDump>,
    /// Per Studio process whose dumps carried this run's label: the last
    /// frame of its newest labeled dump, and whether a dump of later frames
    /// without the label has arrived since. The plugin labels every frame
    /// while the run executes, so such a dump means every frame of the run
    /// that process captured is in a dump already sent.
    processes: HashMap<String, Progress>,
    /// Set once the run completes: it is unregistered when its processes
    /// have all moved past it, or at the deadline.
    drain: Option<Drain>,
}

struct Progress {
    labeled_through: u64,
    passed: bool,
}

struct Drain {
    quiet_until: Instant,
    give_up_at: Instant,
}

impl Run {
    fn drained(&self) -> bool {
        self.drain.is_some() && !self.processes.is_empty() && self.processes.values().all(|p| p.passed)
    }
}

/// The scanner's bookkeeping, apart from the watcher, the file reads and the
/// clock.
struct Scanner {
    label_prefix: String,
    runs: HashMap<String, Run>,
    /// Write time of the newest auto-capture dump seen, from any Studio.
    last_capture: Option<SystemTime>,
}

impl Scanner {
    fn new(label_prefix: &str) -> Self {
        Scanner { label_prefix: label_prefix.to_string(), runs: HashMap::new(), last_capture: None }
    }

    fn register(&mut self, execution_id: String, tx: mpsc::UnboundedSender<ProfileDump>) {
        tracing::debug!(execution_id, "profile scanner: registered run");
        self.runs.insert(execution_id, Run { tx, processes: HashMap::new(), drain: None });
    }

    /// The run finished executing. Its last dumps are usually still to come,
    /// so rather than unregistering it now, wait for them (see `Run`).
    fn complete(&mut self, execution_id: &str, now: Instant, wall: SystemTime) {
        let Some(run) = self.runs.get_mut(execution_id) else { return };
        let capturing = self
            .last_capture
            .is_some_and(|t| wall.duration_since(t).map_or(true, |age| age <= RECENT_CAPTURE));
        if run.processes.is_empty() && !capturing {
            self.unregister(execution_id, "no Studio is capturing");
            return;
        }
        run.drain = Some(Drain { quiet_until: now + DRAIN_QUIET, give_up_at: now + DRAIN_MAX });
        if run.drained() {
            self.unregister(execution_id, "capture already past the run");
        }
    }

    /// Note an auto-capture dump's write time; done for every dump, so a run
    /// that completes before any dump matched it knows whether to wait.
    fn saw_capture(&mut self, filename: &str, written: SystemTime) {
        if parse_auto_capture(filename).is_some() && self.last_capture.is_none_or(|t| written > t) {
            self.last_capture = Some(written);
        }
    }

    /// A dump was written. Send it to every run whose label it carries, then
    /// advance the drains it moves along.
    fn dump(&mut self, filename: &str, data: &[u8], now: Instant) {
        let matched: Vec<String> = self
            .runs
            .keys()
            .filter(|id| file_contains_label(data, id, &self.label_prefix))
            .cloned()
            .collect();
        let name = parse_auto_capture(filename);

        for execution_id in &matched {
            tracing::info!(execution_id, filename, size = data.len(), "profile scanner: matched dump");
            let _ = self.runs[execution_id].tx.send(ProfileDump {
                execution_id: execution_id.clone(),
                filename: filename.to_string(),
                data: data.to_vec(),
            });
        }

        let Some(name) = name else { return };
        let mut drained = Vec::new();
        for (execution_id, run) in &mut self.runs {
            if matched.contains(execution_id) {
                let progress = run
                    .processes
                    .entry(name.process.to_string())
                    .or_insert(Progress { labeled_through: 0, passed: false });
                progress.labeled_through = progress.labeled_through.max(name.last_frame);
                progress.passed = false;
                if let Some(drain) = &mut run.drain {
                    drain.quiet_until = now + DRAIN_QUIET;
                }
            } else if let Some(progress) = run.processes.get_mut(name.process) {
                if name.first_frame > progress.labeled_through {
                    progress.passed = true;
                }
            }
            if run.drained() {
                drained.push(execution_id.clone());
            }
        }
        for execution_id in drained {
            self.unregister(&execution_id, "capture moved past the run");
        }
    }

    /// Unregister completed runs whose drain ran out of time.
    fn expire(&mut self, now: Instant) {
        let expired: Vec<String> = self
            .runs
            .iter()
            .filter(|(_, run)| run.drain.as_ref().is_some_and(|d| now >= d.quiet_until || now >= d.give_up_at))
            .map(|(id, _)| id.clone())
            .collect();
        for execution_id in expired {
            self.unregister(&execution_id, "drain timed out");
        }
    }

    /// Drop the run's sender, which closes its receiver once the dumps
    /// already sent are taken.
    fn unregister(&mut self, execution_id: &str, why: &str) {
        if self.runs.remove(execution_id).is_some() {
            tracing::debug!(execution_id, why, "profile scanner: unregistered run");
        }
    }

    /// Handle a watcher event for `path`.
    fn on_path(&mut self, path: &Path, now: Instant) {
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        if ext != "raw" && ext != "html" {
            return;
        }
        let Some(filename) = path.file_name().and_then(|n| n.to_str()) else { return };
        // Gone already (the event for a swept dump's deletion lands here).
        let Some(written) = modified(path) else { return };
        self.saw_capture(filename, written);
        if self.runs.is_empty() {
            return;
        }
        let Ok(data) = std::fs::read(path) else { return };
        self.dump(filename, &data, now);
    }
}

/// Start the profile scanner background task.
///
/// `label_prefix` is the string stamped into dumps by the profiler that pairs
/// captures with a particular execution — callers choose their own prefix and
/// register runs by id. The watcher matches dumps whose binary content contains
/// `{label_prefix}:{execution_id}`; prefix-agnostic otherwise.
pub fn start(label_prefix: &str) -> ProfileScannerHandle {
    let label_prefix = label_prefix.to_string();
    let (cmd_tx, mut cmd_rx) = mpsc::unbounded_channel::<ScannerCommand>();

    tokio::spawn(async move {
        let scan_dir = match profiler_captures_dir() {
            Some(d) => d,
            None => {
                tracing::warn!("could not determine ProfilerCaptures directory");
                return;
            }
        };

        // Ensure the dir exists so the watcher can subscribe. Roblox doesn't
        // create ProfilerCaptures until its first dump, so without this the
        // watch fails on a fresh machine ("path is neither a file nor a
        // directory"), the scanner task exits, and no dumps are ever captured.
        let _ = std::fs::create_dir_all(&scan_dir);

        let (fs_tx, mut fs_rx) = mpsc::unbounded_channel::<PathBuf>();
        let _debouncer = {
            let tx = fs_tx.clone();
            let mut debouncer = match new_debouncer(
                std::time::Duration::from_millis(500),
                None,
                move |result: DebounceEventResult| {
                    if let Ok(events) = result {
                        for event in events {
                            for path in event.paths.iter() {
                                let _ = tx.send(path.clone());
                            }
                        }
                    }
                },
            ) {
                Ok(d) => d,
                Err(e) => {
                    tracing::warn!("failed to create file watcher: {e}");
                    return;
                }
            };
            if let Err(e) = debouncer.watch(&scan_dir, notify::RecursiveMode::NonRecursive) {
                tracing::warn!("failed to watch ProfilerCaptures: {e}");
                return;
            }
            debouncer
        };

        let mut scanner = Scanner::new(&label_prefix);
        let mut drain_tick = tokio::time::interval(Duration::from_millis(250));
        drain_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        // The first tick fires at once, so a backend sweeps as it starts.
        let mut sweep_tick = tokio::time::interval(SWEEP_INTERVAL);
        sweep_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

        loop {
            tokio::select! {
                path = fs_rx.recv() => {
                    let Some(path) = path else { break };
                    scanner.on_path(&path, Instant::now());
                }
                cmd = cmd_rx.recv() => {
                    match cmd {
                        Some(ScannerCommand::Register { execution_id, tx }) => scanner.register(execution_id, tx),
                        Some(ScannerCommand::Complete { execution_id }) => {
                            scanner.complete(&execution_id, Instant::now(), SystemTime::now());
                        }
                        None => break,
                    }
                }
                _ = drain_tick.tick() => scanner.expire(Instant::now()),
                _ = sweep_tick.tick() => {
                    let removed = sweep_old_dumps(&scan_dir, DUMP_MAX_AGE, SystemTime::now());
                    if removed > 0 {
                        tracing::debug!(removed, "profile scanner: deleted old auto-capture dumps");
                    }
                }
            }
        }
    });

    ProfileScannerHandle { cmd_tx }
}

enum ScannerCommand {
    Register {
        execution_id: String,
        tx: mpsc::UnboundedSender<ProfileDump>,
    },
    Complete {
        execution_id: String,
    },
}

/// Handle for interacting with the profile scanner.
#[derive(Clone)]
pub struct ProfileScannerHandle {
    cmd_tx: mpsc::UnboundedSender<ScannerCommand>,
}

impl ProfileScannerHandle {
    /// Register a profiled run. Returns a receiver for matched dumps.
    pub fn register(&self, execution_id: String) -> mpsc::UnboundedReceiver<ProfileDump> {
        let (tx, rx) = mpsc::unbounded_channel();
        let _ = self.cmd_tx.send(ScannerCommand::Register { execution_id, tx });
        rx
    }

    /// The profiled run has completed. The scanner keeps sending it dumps
    /// until its Studio's capture has moved past it (bounded by a deadline),
    /// then closes the receiver.
    pub fn complete(&self, execution_id: &str) {
        let _ = self.cmd_tx.send(ScannerCommand::Complete {
            execution_id: execution_id.to_string(),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::mpsc::error::TryRecvError;

    const RUN: &str = "0123456789ab";
    const OTHER_RUN: &str = "ba9876543210";
    const STUDIO: &str = "AutoCapture_2026-10-05T120000.000Z";
    const OTHER_STUDIO: &str = "AutoCapture_2026-10-05T120000.500Z";

    fn dump_name(process: &str, first: u64) -> String {
        format!("{process}_Frames-{first}-{}.raw", first + 60)
    }

    fn labeled(runs: &[&str]) -> Vec<u8> {
        let mut data = b"frames".to_vec();
        for run in runs {
            data.extend_from_slice(format!("rodeo:{run}").as_bytes());
        }
        data
    }

    struct Fixture {
        dir: PathBuf,
        scanner: Scanner,
        now: Instant,
    }

    impl Fixture {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir().join(format!("rodeo-profile-scanner-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            Fixture { dir, scanner: Scanner::new("rodeo"), now: Instant::now() }
        }

        fn register(&mut self, execution_id: &str) -> mpsc::UnboundedReceiver<ProfileDump> {
            let (tx, rx) = mpsc::unbounded_channel();
            self.scanner.register(execution_id.to_string(), tx);
            rx
        }

        /// Studio writes a dump, and the watcher reports it.
        fn write(&mut self, filename: &str, data: &[u8]) -> PathBuf {
            let path = self.dir.join(filename);
            std::fs::write(&path, data).unwrap();
            self.scanner.on_path(&path, self.now);
            path
        }

        /// A file written `age` ago, not reported to the scanner.
        fn file_aged(&self, filename: &str, age: Duration) -> PathBuf {
            let path = self.dir.join(filename);
            std::fs::write(&path, b"frames").unwrap();
            let file = std::fs::File::options().write(true).open(&path).unwrap();
            file.set_modified(SystemTime::now() - age).unwrap();
            path
        }

        fn complete(&mut self, execution_id: &str) {
            self.scanner.complete(execution_id, self.now, SystemTime::now());
        }

        fn advance(&mut self, by: Duration) {
            self.now += by;
            self.scanner.expire(self.now);
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    /// Unregistered: the scanner dropped the run's sender. Dumps sent before
    /// that are still in the channel.
    fn closed(rx: &mut mpsc::UnboundedReceiver<ProfileDump>) -> bool {
        rx.is_closed()
    }

    fn drained(rx: &mut mpsc::UnboundedReceiver<ProfileDump>) -> bool {
        matches!(rx.try_recv(), Err(TryRecvError::Disconnected))
    }

    #[test]
    fn auto_capture_names_give_the_process_and_frames() {
        assert_eq!(
            parse_auto_capture("AutoCapture_2026-09-14T050530.227Z_Frames-123-183.raw"),
            Some(AutoCaptureName { process: "AutoCapture_2026-09-14T050530.227Z", first_frame: 123, last_frame: 183 })
        );
        assert_eq!(parse_auto_capture("AutoCapture_2026-09-14T050530.227Z_Frames-123-183.html"), None);
        assert_eq!(parse_auto_capture("microprofile-20261005-120000.html"), None);
        assert_eq!(parse_auto_capture("MyCapture_Frames-1-61.raw"), None);
    }

    #[test]
    fn a_dump_goes_to_every_run_whose_label_it_carries() {
        let mut f = Fixture::new("two-runs");
        let mut first = f.register(RUN);
        let mut second = f.register(OTHER_RUN);
        let mut third = f.register("aaaaaaaaaaaa");
        f.write(&dump_name(STUDIO, 1), &labeled(&[RUN, OTHER_RUN]));
        assert_eq!(first.try_recv().unwrap().data, labeled(&[RUN, OTHER_RUN]));
        assert_eq!(second.try_recv().unwrap().data, labeled(&[RUN, OTHER_RUN]));
        assert!(third.try_recv().is_err(), "not this run's dump");
    }

    #[test]
    fn old_auto_capture_dumps_are_swept_and_everything_else_stays() {
        let f = Fixture::new("sweep");
        let past = DUMP_MAX_AGE + Duration::from_secs(60);
        let old = f.file_aged(&dump_name(STUDIO, 1), past);
        let young = f.file_aged(&dump_name(STUDIO, 62), DUMP_MAX_AGE - Duration::from_secs(60));
        let saved = f.file_aged("microprofile-20261005-120000.html", past);
        let renamed = f.file_aged("my-capture.raw", past);

        assert_eq!(sweep_old_dumps(&f.dir, DUMP_MAX_AGE, SystemTime::now()), 1);
        assert!(!old.exists());
        assert!(young.exists(), "another scanner may still need it");
        assert!(saved.exists() && renamed.exists(), "dumps saved by hand are never swept");
    }

    #[test]
    fn a_completed_run_still_gets_its_tail_dump() {
        let mut f = Fixture::new("tail");
        let mut rx = f.register(RUN);
        f.write(&dump_name(STUDIO, 1), &labeled(&[RUN]));
        f.complete(RUN);

        // The dump holding the run's last frames lands after RunCompleted.
        f.advance(Duration::from_secs(1));
        f.write(&dump_name(STUDIO, 62), &labeled(&[RUN]));
        assert!(!closed(&mut rx));

        // Another Studio moving on says nothing about this run's Studio.
        f.write(&dump_name(OTHER_STUDIO, 62), b"frames");
        assert!(!closed(&mut rx));

        // This Studio's next window has no label: the run is fully captured.
        f.write(&dump_name(STUDIO, 123), b"frames");
        assert_eq!(rx.try_recv().unwrap().filename, dump_name(STUDIO, 1));
        assert_eq!(rx.try_recv().unwrap().filename, dump_name(STUDIO, 62));
        assert!(drained(&mut rx), "unregistered once the capture moved past the run");
    }

    #[test]
    fn a_run_whose_capture_already_moved_on_unregisters_at_completion() {
        let mut f = Fixture::new("already-past");
        let mut rx = f.register(RUN);
        f.write(&dump_name(STUDIO, 1), &labeled(&[RUN]));
        f.write(&dump_name(STUDIO, 62), b"frames");
        assert!(!closed(&mut rx), "still running, so still registered");
        f.complete(RUN);
        rx.try_recv().unwrap();
        assert!(drained(&mut rx));
    }

    #[test]
    fn a_completed_run_with_no_dumps_waits_while_a_studio_is_capturing() {
        let mut f = Fixture::new("short-run");
        f.write(&dump_name(STUDIO, 1), b"frames");
        let mut rx = f.register(RUN);
        f.complete(RUN);
        assert!(!closed(&mut rx), "its frames may be in the window still being captured");

        f.advance(Duration::from_secs(1));
        f.write(&dump_name(STUDIO, 62), &labeled(&[RUN]));
        f.write(&dump_name(STUDIO, 123), b"frames");
        rx.try_recv().unwrap();
        assert!(drained(&mut rx));
    }

    #[test]
    fn a_completed_run_unregisters_at_once_when_nothing_is_capturing() {
        let mut f = Fixture::new("no-capture");
        let mut rx = f.register(RUN);
        f.complete(RUN);
        assert!(closed(&mut rx));
    }

    #[test]
    fn a_drain_gives_up_after_a_quiet_period() {
        let mut f = Fixture::new("quiet");
        f.write(&dump_name(STUDIO, 1), b"frames");
        let mut rx = f.register(RUN);
        f.complete(RUN);
        f.advance(DRAIN_QUIET - Duration::from_millis(1));
        assert!(!closed(&mut rx));
        f.advance(Duration::from_millis(1));
        assert!(closed(&mut rx));
    }

    #[test]
    fn matching_dumps_extend_a_drain_up_to_its_limit() {
        let mut f = Fixture::new("extend");
        let mut rx = f.register(RUN);
        f.write(&dump_name(STUDIO, 1), &labeled(&[RUN]));
        f.complete(RUN);
        let mut first = 62;
        let mut waited = Duration::ZERO;
        while waited + DRAIN_QUIET / 2 < DRAIN_MAX {
            f.advance(DRAIN_QUIET / 2);
            waited += DRAIN_QUIET / 2;
            f.write(&dump_name(STUDIO, first), &labeled(&[RUN]));
            first += 61;
            assert!(!closed(&mut rx), "a dump for the run keeps it registered");
            while rx.try_recv().is_ok() {}
        }
        f.advance(DRAIN_MAX - waited);
        assert!(closed(&mut rx), "but not past the limit");
    }
}
