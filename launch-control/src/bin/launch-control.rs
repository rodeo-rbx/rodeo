//! Helper binary used by `launch_control::Command::spawn` on macOS to hold
//! the launched app's pty FDs in a separate OS process. Lets
//! `Command::detached(true)` work: when the spawning process dies, this
//! helper survives and the app keeps running with stdio captured.
//!
//! Real implementation lives in the lib so it's testable; this binary is a
//! thin shim.

fn main() {
    launch_control::run_main()
}
