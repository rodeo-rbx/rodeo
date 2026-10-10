//! Per-subprocess log capture.
//!
//! Each long-running subprocess (master, studio-backend, player-backend,
//! studio-daemon) initialises tracing with BOTH a stderr layer (existing
//! human-readable output) and a text file layer. The file lands at
//! `.rodeo/.temp/logs/<role>-<bootstrap_id>-<ts>.log`.
//!
//! Same text format as stderr (ANSI colours stripped) — readable by
//! `cat`/`less`/`grep`. Tracing spans still emit their fields as
//! `key=value` pairs, so `grep master_id=5014db7a logs/*.log` works for
//! cross-subprocess correlation.

use std::io::IsTerminal;

use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{EnvFilter, Layer};

/// Produce the ISO-8601 compact UTC timestamp used in filenames, e.g.
/// `20260421T153042Z`.
pub fn filename_timestamp() -> String {
    chrono::Utc::now().format("%Y%m%dT%H%M%SZ").to_string()
}

/// Initialise tracing for a subprocess. Sets up:
///   - stderr layer: human-readable (preserves existing behaviour)
///   - JSON file layer: `.rodeo/.temp/logs/<role>-<bootstrap_id>-<ts>.log`
///
/// Call once at subprocess entry after argv parse.
///
/// Environment:
///   RUST_LOG — standard env filter for both layers (falls back to rodeo=info)
///   RODEO_VERBOSE=1 — bump rodeo crate to debug when RUST_LOG is unset
///   NO_COLOR / FORCE_COLOR — ANSI colour behaviour on stderr
///   RODEO_NO_TIMESTAMPS — strip timestamps from stderr (file layer keeps them)
///   RODEO_LOG_DIR — override the log directory (default `.rodeo/.temp/logs`)
pub fn init(role: &'static str, bootstrap_id: &str) {
    let verbose = std::env::var("RODEO_VERBOSE").is_ok();
    // Set by the parent rodeo-run process when this subprocess was
    // auto-spawned for a run (not an interactive serve). The run's terminal
    // carries the run's own output: stderr gets errors only. Whatever a run
    // needs to know reaches it through the run itself (launch and mode
    // failures, disconnects); a serve's warnings are about the serve. The
    // file layer still runs at debug, so everything is at .rodeo/.temp/logs/.
    let quiet = std::env::var("RODEO_QUIET").is_ok();
    let no_color = std::env::var("NO_COLOR").is_ok_and(|v| !v.is_empty());
    let force_color = std::env::var("FORCE_COLOR").is_ok_and(|v| !v.is_empty());
    let use_ansi = !no_color && (force_color || std::io::stderr().is_terminal());
    let no_timestamps = std::env::var("RODEO_NO_TIMESTAMPS").is_ok();

    let log_dir = std::env::var("RODEO_LOG_DIR")
        .unwrap_or_else(|_| ".rodeo/.temp/logs".to_string());
    let log_dir = std::path::PathBuf::from(log_dir);

    // Best-effort: if the file layer can't be created, fall back to stderr-only.
    let file_writer = match std::fs::create_dir_all(&log_dir) {
        Ok(()) => {
            let filename = format!("{role}-{bootstrap_id}-{ts}.log", ts = filename_timestamp());
            match std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(log_dir.join(&filename))
            {
                Ok(f) => Some(f),
                Err(e) => {
                    eprintln!("log_capture: failed to open {filename}: {e}");
                    None
                }
            }
        }
        Err(e) => {
            eprintln!("log_capture: failed to create {}: {e}", log_dir.display());
            None
        }
    };

    // Capture our crate + rbx_control + launch_control. Each holds a chunk of
    // Studio-side instrumentation (focus/save/keystroke in launch_control,
    // launch lifecycle in rbx_control, everything else in rodeo itself).
    // RUST_LOG still takes precedence if set.
    let stderr_filter = EnvFilter::try_from_env("RUST_LOG").unwrap_or_else(|_| {
        if verbose {
            EnvFilter::new("rodeo=debug,rbx_control=debug,launch_control=debug")
        } else if quiet {
            EnvFilter::new("rodeo=error,rbx_control=error,launch_control=error")
        } else {
            EnvFilter::new("rodeo=info,rbx_control=info,launch_control=info")
        }
    });
    // File layer is always debug-level so we capture more detail than stderr.
    let file_filter = EnvFilter::try_from_env("RUST_LOG")
        .unwrap_or_else(|_| EnvFilter::new("rodeo=debug,rbx_control=debug,launch_control=debug"));

    let stderr_layer = if no_timestamps {
        tracing_subscriber::fmt::layer()
            .with_writer(std::io::stderr)
            .with_ansi(use_ansi)
            .with_target(false)
            .without_time()
            .with_filter(stderr_filter)
            .boxed()
    } else {
        tracing_subscriber::fmt::layer()
            .with_writer(std::io::stderr)
            .with_ansi(use_ansi)
            .with_target(false)
            .with_filter(stderr_filter)
            .boxed()
    };

    let file_layer = file_writer.map(|file| {
        tracing_subscriber::fmt::layer()
            .with_writer(std::sync::Mutex::new(file))
            .with_ansi(false)
            .with_target(false)
            .with_filter(file_filter)
            .boxed()
    });

    tracing_subscriber::registry()
        .with(stderr_layer)
        .with(file_layer)
        .init();
}
