//! `cargo run --example focus_pid -- <pid>` — display-aware focus of a
//! running app, printing who is frontmost before and after.
fn main() {
    let pid: u32 = std::env::args().nth(1).and_then(|a| a.parse().ok()).expect("usage: focus_pid <pid>");
    tracing_subscriber::fmt().with_env_filter("debug").init();
    launch_control::focus_pid(pid).expect("focus failed");
}
