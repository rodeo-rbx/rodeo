use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    // Emitting any rerun-if-changed disables cargo's default "rerun on any
    // package file change", so re-add the proto sources explicitly.
    println!("cargo:rerun-if-changed=proto/");
    connectrpc_build::Config::new()
        .files(&["proto/rodeo.proto", "proto/runtime.proto"])
        .includes(&["proto/"])
        .include_file("_connectrpc.rs")
        .compile()
        .expect("failed to compile connectrpc definitions");

    emit_build_id();
}

/// `RODEO_BUILD_ID` = `<rodeo-cli version>[+<7-char git sha>]` — the single
/// compatibility token every hop compares (see `BUILD_ID` in lib.rs).
///
/// The version is read from rodeo-cli/Cargo.toml, the manifest the release
/// bump edits (rodeo-proto / rodeo-client are not versioned on their own;
/// rodeo-plugin/build.luau reads the same file for the plugin side). The sha
/// distinguishes two dev builds of the same version — without it every local
/// build is "1.3.0" and proto drift between them goes unnoticed. It reflects
/// HEAD only, not uncommitted edits. Absent when git is unavailable (source
/// tarball builds), in which case the id is the bare version.
fn emit_build_id() {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let repo_root = Path::new(&manifest_dir).join("..");

    let cli_manifest = repo_root.join("rodeo-cli/Cargo.toml");
    println!("cargo:rerun-if-changed={}", cli_manifest.display());
    let version = std::fs::read_to_string(&cli_manifest)
        .ok()
        .and_then(|s| {
            s.lines().find_map(|l| {
                l.trim()
                    .strip_prefix("version = \"")
                    .and_then(|rest| rest.strip_suffix('"'))
                    .map(str::to_string)
            })
        })
        .expect("rodeo-cli/Cargo.toml: no `version = \"…\"` line");

    // Rerun when HEAD moves: track HEAD itself, the ref it points at, and
    // packed-refs. git resolves where they live: in a worktree, `.git` is a
    // file, HEAD is in the worktree's own git dir and refs are in the shared
    // one (missing that left a worktree build's id stale while the plugin's
    // moved on, and the backend parked its own plugin as a mismatch). Only
    // existing paths — a missing rerun-if-changed target forces a rebuild
    // every time.
    let git_path = |name: &str| -> Option<PathBuf> {
        let out = Command::new("git")
            .args(["rev-parse", "--git-path", name])
            .current_dir(&repo_root)
            .output()
            .ok()
            .filter(|o| o.status.success())?;
        let path = PathBuf::from(String::from_utf8_lossy(&out.stdout).trim());
        let path = if path.is_absolute() { path } else { repo_root.join(path) };
        path.exists().then_some(path)
    };
    if let Some(head) = git_path("HEAD") {
        println!("cargo:rerun-if-changed={}", head.display());
        if let Ok(contents) = std::fs::read_to_string(&head) {
            if let Some(ref_path) = contents.trim().strip_prefix("ref: ").and_then(git_path) {
                println!("cargo:rerun-if-changed={}", ref_path.display());
            }
        }
    }
    if let Some(packed) = git_path("packed-refs") {
        println!("cargo:rerun-if-changed={}", packed.display());
    }

    let sha = Command::new("git")
        .args(["rev-parse", "--short=7", "HEAD"])
        .current_dir(&repo_root)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|s| !s.is_empty());

    let build_id = match sha {
        Some(sha) => format!("{version}+{sha}"),
        None => version,
    };
    println!("cargo:rustc-env=RODEO_BUILD_ID={build_id}");
}
