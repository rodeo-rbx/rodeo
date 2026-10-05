// Repro for "on_exit callback never fires when an NSWorkspace-launched
// (non-child, reparented to launchd) process is killed externally".
//
// Run: cargo run -p launch-control --example repro_on_exit -- [iters]
// Default 50 iterations.

use std::sync::mpsc;
use std::time::Duration;

fn main() {
    let iters: u32 = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(50);
    let parallel: usize = std::env::args()
        .find_map(|a| a.strip_prefix("--parallel=").map(|s| s.to_string()))
        .and_then(|s| s.parse().ok())
        .unwrap_or(1);

    // Use --piped to exercise spawn_piped (rodeo's actual code path), or
    // default to spawn_simple. Studio launches go through spawn_piped because
    // rodeo configures Stdio::piped() for stdout/stderr capture.
    let piped = std::env::args().any(|a| a == "--piped");
    let app = std::env::args()
        .find(|a| a.starts_with("/") && a.ends_with(".app"))
        .unwrap_or_else(|| "/System/Applications/Calculator.app".into());

    println!("app = {app}");
    println!("piped = {piped}");
    println!("parallel = {parallel}");

    let hangs = std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0));
    let fast = std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0));
    let slow = std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0));

    // Mimic rodeo's failure scenario: spawn N in parallel, but only ONE
    // Studio actually dies. The other N-1 stay alive. If spawn_piped's
    // bundle-ID-diff PID lookup collides, the on_exit on the dying Studio
    // may have been registered for an alive PID and will never fire.
    for i in 1..=iters {
        let kill_idx = (i as usize) % parallel.max(1);
        let app = app.clone();
        let hangs = hangs.clone();
        let fast = fast.clone();
        let slow = slow.clone();
        std::thread::scope(|s| {
            let mut handles = vec![];
            for k in 0..parallel.max(1) {
                let label = format!(".{k}");
                let should_kill = k == kill_idx;
                let app = app.clone();
                let hangs = hangs.clone();
                let fast = fast.clone();
                let slow = slow.clone();
                let h = s.spawn(move || {
                    let mut cmd = launch_control::Command::new(&app);
                    cmd.background(true);
                    if piped {
                        cmd.stdout(std::process::Stdio::piped());
                        cmd.stderr(std::process::Stdio::piped());
                    }
                    let child = match cmd.spawn() {
                        Ok(c) => c,
                        Err(e) => {
                            eprintln!("[{i}{label}] spawn failed: {e}");
                            return None;
                        }
                    };
                    let pid = child.id();
                    let (tx, rx) = mpsc::channel();
                    child.on_exit(move |s| { let _ = tx.send(s); });
                    std::thread::sleep(Duration::from_millis(100));

                    if should_kill {
                        unsafe { libc::kill(pid as i32, libc::SIGKILL); }
                        let killed_at = std::time::Instant::now();
                        match rx.recv_timeout(Duration::from_secs(5)) {
                            Ok(_) => {
                                let elapsed_ms = killed_at.elapsed().as_millis();
                                if elapsed_ms < 200 {
                                    fast.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                                } else {
                                    slow.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                                }
                                println!("[{i}{label}] pid={pid} (killed) fired in {elapsed_ms}ms");
                            }
                            Err(_) => {
                                hangs.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                                println!("[{i}{label}] pid={pid} (killed) HUNG — bug reproduced");
                            }
                        }
                        // Pass child back so siblings can be killed in cleanup
                        Some((pid, child))
                    } else {
                        // Don't kill — we're a "live alongside" peer
                        println!("[{i}{label}] pid={pid} (alive)");
                        Some((pid, child))
                    }
                });
                handles.push(h);
            }

            // Drain results, then clean up all surviving Studios.
            let mut survivors = vec![];
            for h in handles {
                if let Some(c) = h.join().ok().flatten() {
                    survivors.push(c);
                }
            }
            for (pid, _child) in &survivors {
                unsafe { libc::kill(*pid as i32, libc::SIGKILL); }
            }
            // Give them a moment to actually die so the next iter's snapshot
            // doesn't pick them up.
            std::thread::sleep(Duration::from_millis(500));
        });
    }

    let hangs = hangs.load(std::sync::atomic::Ordering::Relaxed);
    let fast = fast.load(std::sync::atomic::Ordering::Relaxed);
    let slow = slow.load(std::sync::atomic::Ordering::Relaxed);

    println!();
    println!("=== summary over {iters} runs (parallel={parallel}) ===");
    println!("  fast (<200ms): {fast}");
    println!("  slow (>=200ms): {slow}");
    println!("  hung (>5s, no callback): {hangs}");
    if hangs > 0 {
        std::process::exit(1);
    }
}
