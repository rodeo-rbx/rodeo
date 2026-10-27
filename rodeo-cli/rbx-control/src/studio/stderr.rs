//! What a launched Studio writes to stderr.
//!
//! Studio's own log file doesn't record why it aborted: a failed Metal
//! assertion, for one, only reaches stderr ("MTLTextureDescriptor has height
//! (32776) greater than the maximum allowed size of 32768", Studio 0.745 on a
//! large capture). Every line goes to the debug log as it arrives, which the
//! serve's log file always records, and the last few are kept: a crash is
//! reported once at warn level with them, when rodeo kills the crashed Studio
//! or when Studio closes stderr, whichever comes first. (A crashed Studio's
//! DataModels disconnect, which can end a run and close its Studio before
//! anything waiting on the process exit gets to run.)
//!
//! Studio's crash reporter (Crashpad) logs two lines when Studio starts it
//! (from `StudioCrashpadHandler.cpp`) and a few hundred more, from other
//! files, only while it handles a crash. Those would push what Studio wrote
//! before it died out of the kept lines, so Crashpad's lines aren't kept (nor
//! the line after one ending in ':', which continues it: the startup lines
//! are followed by the handler's whole command line); one
//! from anywhere but the startup file marks the Studio as crashed. That is
//! the only crash signal on macOS, where Studio isn't rodeo's child process
//! and its exit status can't be read.

use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Read};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// How many of Studio's last stderr lines are kept.
const KEPT_LINES: usize = 12;

/// The Crashpad source file that logs when Studio starts it, not on a crash.
const CRASHPAD_STARTUP_SOURCE: &str = "StudioCrashpadHandler.cpp";

struct Kept {
    lines: Mutex<VecDeque<String>>,
    crashed: AtomicBool,
    /// Set once Studio closes stderr. A tail nothing watches starts closed.
    closed: AtomicBool,
    /// Set once the crash has been reported.
    reported: AtomicBool,
    /// The last line was Crashpad's and ended in ':', so the next continues it.
    continues: AtomicBool,
}

impl Default for Kept {
    fn default() -> Self {
        Self { lines: Mutex::default(), crashed: AtomicBool::new(false), closed: AtomicBool::new(true), reported: AtomicBool::new(false), continues: AtomicBool::new(false) }
    }
}

/// The last lines a Studio wrote to stderr, Crashpad's left out.
#[derive(Clone, Default)]
pub struct StderrTail(Arc<Kept>);

impl StderrTail {
    /// Read `stderr` on a thread of its own until Studio closes it, logging
    /// each line and keeping the last few.
    pub fn watch(stderr: impl Read + Send + 'static, pid: u32) -> Self {
        let tail = Self::default();
        tail.0.closed.store(false, Ordering::Relaxed);
        let kept = tail.clone();
        std::thread::spawn(move || {
            for line in BufReader::new(stderr).lines() {
                let Ok(line) = line else { break };
                tracing::debug!(pid, "studio stderr: {line}");
                kept.push(line);
            }
            kept.0.closed.store(true, Ordering::Relaxed);
            kept.report_crash(pid);
        });
        tail
    }

    /// If Studio crashed, warn once with the last lines it wrote.
    pub fn report_crash(&self, pid: u32) {
        if !self.crashed() || self.0.reported.swap(true, Ordering::Relaxed) {
            return;
        }
        let lines = self.lines();
        if lines.is_empty() {
            tracing::warn!(pid, "Studio crashed");
        } else {
            tracing::warn!(pid, "Studio crashed; the last lines it wrote to stderr:\n{}", lines.join("\n"));
        }
    }

    fn push(&self, line: String) {
        if let Some(source) = crashpad_source(&line) {
            if source != CRASHPAD_STARTUP_SOURCE {
                self.0.crashed.store(true, Ordering::Relaxed);
            }
            self.0.continues.store(line.ends_with(':'), Ordering::Relaxed);
            return;
        }
        if self.0.continues.swap(false, Ordering::Relaxed) {
            return;
        }
        let mut lines = self.0.lines.lock().unwrap();
        if lines.len() == KEPT_LINES {
            lines.pop_front();
        }
        lines.push_back(line);
    }

    /// The kept lines, oldest first.
    pub fn lines(&self) -> Vec<String> {
        self.0.lines.lock().unwrap().iter().cloned().collect()
    }

    /// Whether Studio's crash reporter handled a crash.
    pub fn crashed(&self) -> bool {
        self.0.crashed.load(Ordering::Relaxed)
    }

    /// Wait up to `timeout` for Studio to close stderr, so the lines it wrote
    /// last have been read.
    pub fn wait_closed(&self, timeout: Duration) {
        let deadline = Instant::now() + timeout;
        while !self.0.closed.load(Ordering::Relaxed) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

/// The source file of a Crashpad line, which starts
/// `[pid:tid:yyyymmdd,hhmmss.micros:LEVEL file:line]`; `None` for any other line.
fn crashpad_source(line: &str) -> Option<&str> {
    let prefix = line.strip_prefix('[')?.split_once(']')?.0;
    let mut fields = prefix.splitn(4, ':');
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    let (pid, tid, when, source) = (fields.next()?, fields.next()?, fields.next()?, fields.next()?);
    let (date, time) = when.split_once(',')?;
    if !(digits(pid) && digits(tid) && digits(date) && time.split('.').all(digits)) {
        return None;
    }
    let (_level, file) = source.split_once(' ')?;
    Some(file.rsplit_once(':').map_or(file, |(file, _line)| file))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_crashpad_sources() {
        assert_eq!(crashpad_source("[6479:32753560:20261027,155854.919308:INFO CrashpadCallback.inl:45] Add attribute: UserId"), Some("CrashpadCallback.inl"));
        assert_eq!(crashpad_source("[7398:32763004:20261027,160119.062181:INFO StudioCrashpadHandler.cpp:104] Handler executable command line:"), Some("StudioCrashpadHandler.cpp"));
        assert_eq!(crashpad_source("-[MTLTextureDescriptorInternal validateWithDevice:]:1421: failed assertion `Texture Descriptor Validation"), None);
        assert_eq!(crashpad_source("MTLTextureDescriptor has height (32776) greater than the maximum allowed size of 32768."), None);
        assert_eq!(crashpad_source("2026-10-27 16:01:28.185 RobloxStudio[7479:32763752] ApplePersistenceIgnoreState: Existing state will not be touched."), None);
        assert_eq!(crashpad_source("[rodeo] Starting execution..."), None);
        assert_eq!(crashpad_source(""), None);
    }

    fn watch_lines(lines: &[String]) -> StderrTail {
        let input = lines.join("\n") + "\n";
        let tail = StderrTail::watch(std::io::Cursor::new(input.into_bytes()), 1);
        tail.wait_closed(Duration::from_secs(5));
        assert!(tail.0.closed.load(Ordering::Relaxed), "reader never finished");
        tail
    }

    #[test]
    fn a_crash_keeps_what_studio_wrote_before_crashpad() {
        let mut lines = vec![
            "[7398:32763004:20261027,160119.062181:INFO StudioCrashpadHandler.cpp:104] Handler executable command line:".to_string(),
            "/Applications/RobloxStudio.app/Contents/MacOS/RobloxCrashHandler --no-rate-limit".to_string(),
            "-[MTLTextureDescriptorInternal validateWithDevice:]:1421: failed assertion `Texture Descriptor Validation".to_string(),
        ];
        lines.extend((0..200).map(|i| format!("[6479:32753560:20261027,155854.{i}:INFO CrashpadCallback.inl:45] attribute {i}")));
        let tail = watch_lines(&lines);
        assert!(tail.crashed());
        assert_eq!(tail.lines(), vec![lines[2].clone()]);
        // The watcher reported it on close; rodeo's kill doesn't again.
        assert!(tail.0.reported.load(Ordering::Relaxed));
    }

    #[test]
    fn crashpads_startup_lines_are_not_a_crash() {
        let tail = watch_lines(&[
            "[7398:32763004:20261027,160119.062181:INFO StudioCrashpadHandler.cpp:104] Handler executable command line:".to_string(),
            "/Applications/RobloxStudio.app/Contents/MacOS/RobloxCrashHandler --no-rate-limit --crashCounter Mac-ROBLOXStudio-Crash".to_string(),
            "[7398:32763004:20261027,160119.062665:INFO StudioCrashpadHandler.cpp:140] Running the crashpad handler main function with 26 arguments:".to_string(),
            "/Applications/RobloxStudio.app/Contents/MacOS/RobloxCrashHandler --no-rate-limit --attachment=attachment_0.745.0.7451137".to_string(),
            "2026-10-27 16:01:19.085 RobloxStudio[7396:32762928] ApplePersistenceIgnoreState: Existing state will not be touched.".to_string(),
        ]);
        assert!(!tail.crashed());
        assert_eq!(tail.lines(), vec!["2026-10-27 16:01:19.085 RobloxStudio[7396:32762928] ApplePersistenceIgnoreState: Existing state will not be touched.".to_string()]);
        tail.report_crash(1);
        assert!(!tail.0.reported.load(Ordering::Relaxed));
    }

    #[test]
    fn keeps_at_most_the_last_twelve() {
        let tail = StderrTail::default();
        for i in 0..20 {
            tail.push(format!("line {i}"));
        }
        let kept = tail.lines();
        assert_eq!(kept.len(), KEPT_LINES);
        assert_eq!(kept.first().map(String::as_str), Some("line 8"));
        assert_eq!(kept.last().map(String::as_str), Some("line 19"));
    }
}
