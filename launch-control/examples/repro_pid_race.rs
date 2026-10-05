// Demonstrates the PID-assignment race in spawn_piped's bundle-ID-diff
// approach when multiple processes are spawned concurrently for the same
// bundle. Each thread takes a "before" snapshot, spawns via `open`, then
// looks for the first new PID with that bundle ID. With concurrent spawns,
// multiple threads see the same "new" PID, while other actually-launched
// PIDs go unclaimed.

use std::sync::mpsc;
use std::time::Duration;

fn list_pids_for(bundle_id: &str) -> Vec<i32> {
    use objc2_app_kit::NSRunningApplication;
    NSRunningApplication::runningApplicationsWithBundleIdentifier(
        &objc2_foundation::NSString::from_str(bundle_id),
    )
    .iter()
    .map(|app| app.processIdentifier())
    .collect()
}

fn main() {
    let parallel: usize = std::env::args()
        .find_map(|a| a.strip_prefix("--parallel=").map(|s| s.to_string()))
        .and_then(|s| s.parse().ok())
        .unwrap_or(3);
    let iters: u32 = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(5);

    let app = "/System/Applications/Calculator.app";
    let bundle_id = "com.apple.calculator";

    for i in 1..=iters {
        let before = list_pids_for(bundle_id);
        println!("[iter {i}] before pids = {:?}", before);

        let claims = std::sync::Arc::new(std::sync::Mutex::new(Vec::<u32>::new()));

        std::thread::scope(|s| {
            let mut handles = vec![];
            for k in 0..parallel {
                let claims = claims.clone();
                let h = s.spawn(move || {
                    let mut cmd = launch_control::Command::new(app);
                    cmd.background(true);
                    cmd.stdout(std::process::Stdio::piped());
                    cmd.stderr(std::process::Stdio::piped());
                    let child = match cmd.spawn() {
                        Ok(c) => c,
                        Err(e) => {
                            eprintln!("[iter {i}.{k}] spawn err: {e}");
                            return None;
                        }
                    };
                    let pid = child.id();
                    println!("[iter {i}.{k}] spawn returned pid={pid}");
                    claims.lock().unwrap().push(pid);

                    // Hold child alive briefly so we can poll the PID list.
                    std::thread::sleep(Duration::from_millis(800));
                    Some((pid, child))
                });
                handles.push(h);
            }

            // Let all threads finish their PID lookup before we measure.
            std::thread::sleep(Duration::from_millis(500));
            let mid_snapshot = list_pids_for(bundle_id);
            let new_pids: Vec<i32> = mid_snapshot.iter()
                .filter(|p| !before.contains(p))
                .copied()
                .collect();
            let claimed = claims.lock().unwrap().clone();
            println!("[iter {i}] real new pids = {:?}", new_pids);
            println!("[iter {i}] claimed pids  = {:?}", claimed);

            let unique_claims: std::collections::HashSet<u32> = claimed.iter().copied().collect();
            let orphans: Vec<i32> = new_pids.iter()
                .filter(|p| !unique_claims.contains(&(**p as u32)))
                .copied()
                .collect();
            let phantom: Vec<u32> = claimed.iter()
                .filter(|p| !new_pids.contains(&(**p as i32)))
                .copied()
                .collect();
            if !orphans.is_empty() {
                println!("[iter {i}] *** ORPHANS (launched but not claimed): {:?}", orphans);
            }
            if !phantom.is_empty() {
                println!("[iter {i}] *** PHANTOM (claimed but not running): {:?}", phantom);
            }
            if claimed.len() != unique_claims.len() {
                println!("[iter {i}] *** PID COLLISION (multiple threads claim same PID)");
            }

            // Cleanup: kill all real new PIDs and join threads.
            for p in &new_pids {
                unsafe { libc::kill(*p, libc::SIGKILL); }
            }
            for h in handles {
                let _ = h.join();
            }
        });

        std::thread::sleep(Duration::from_millis(400));
    }
}
