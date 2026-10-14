pub mod adapters;
pub mod bundle;
pub mod directive;
pub mod execution;
pub mod source;
pub mod sourcemap;

use anyhow::{bail, Result};
use serde::{Serialize, Deserialize};

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcessedSource {
    pub script: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub script_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub instance_path: Option<String>,
}

/// Process source in-process. Used by `rodeo run` and by the client daemon
/// (`__spawn_canonical_client`) for the TypeScript and Luau clients' runs.
///
/// Runs on a thread with a large stack: both darklua's bundler and full_moon's
/// parser recurse with the require-graph / AST depth, which overflows Windows'
/// 1 MB default main-thread stack (macOS/Linux default to 8 MB) — a
/// STATUS_STACK_OVERFLOW process crash, not a catchable error. The generous
/// stack makes deep-but-finite scripts bundle identically on every platform.
pub fn process(
    script: Option<String>,
    source_arg: Option<String>,
    sourcemap: Option<String>,
) -> Result<ProcessedSource> {
    std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(move || process_impl(script, source_arg, sourcemap))
        .map_err(|e| anyhow::anyhow!("failed to spawn process_source thread: {e}"))?
        .join()
        .map_err(|_| anyhow::anyhow!("process_source thread panicked"))?
}

fn process_impl(
    script: Option<String>,
    source_arg: Option<String>,
    sourcemap: Option<String>,
) -> Result<ProcessedSource> {
    if let Some(ref src) = source_arg {
        // Materialize inline source to a temp `.luau` file under CWD so
        // darklua's bundle pass has an anchor for relative-path require
        // resolution. Same pipeline as file-mode: bundle (inlines fs deps)
        // → inline_shims (resolves @rodeo/@lune) → ensure_return.
        let tmp_path = std::env::current_dir()
            .unwrap_or_else(|_| std::path::PathBuf::from("."))
            .join(format!(".rodeo-inline-{}.luau", uuid::Uuid::new_v4()));
        std::fs::write(&tmp_path, src)
            .map_err(|e| anyhow::anyhow!("cannot write inline-source temp file: {e}"))?;

        let opts = bundle::BundleOptions {
            sourcemap: sourcemap.clone(),
            verbose: false,
        };
        let bundle_result = bundle::bundle(tmp_path.to_str().unwrap_or(""), &opts);
        let _ = std::fs::remove_file(&tmp_path);
        // Bundling only inlines filesystem requires; inline `--source` runs
        // rarely have any, so a bundle failure shouldn't abort the run. darklua
        // emits no output for source it can't parse (e.g. a syntax error), and
        // that failure differs by platform: macOS passes the invalid source
        // through, Windows produces nothing. Fall back to the raw source so
        // Studio runs it and reports the real error (syntax error, unresolved
        // require, ...) as a not-ok result — identically on every platform.
        let bundled = match bundle_result {
            Ok(b) => b,
            Err(e) => {
                tracing::warn!("bundle failed, running raw source: {e:#}");
                src.clone()
            }
        };

        let shimmed = bundle::inline_shims(&bundled)?;
        let script = source::ensure_return(&shimmed);
        Ok(ProcessedSource {
            script,
            script_path: None,
            instance_path: None,
        })
    } else if let Some(ref file) = script {
        // File: resolve path, bundle, shim, ensure_return, resolve instance.
        // Directive parsing happens in the parent before subprocess invocation
        // (see main.rs splice phase) — clap re-parses with directive tokens
        // inlined into argv, so directives stay in lockstep with the CLI grammar.
        let resolved = directive::resolve_script_path(file);

        // Bundle
        let opts = bundle::BundleOptions {
            sourcemap: sourcemap.clone(),
            verbose: false,
        };
        let bundled = bundle::bundle(&resolved, &opts)?;

        // Shim injection + ensure_return
        let shimmed = bundle::inline_shims(&bundled)?;
        let script = source::ensure_return(&shimmed);

        // Instance resolution
        let script_path = Some(execution::compute_relative_path(&resolved));
        let instance_path = execution::resolve_instance_path(
            sourcemap.as_deref(),
            Some(&resolved),
        );

        Ok(ProcessedSource {
            script,
            script_path,
            instance_path,
        })
    } else {
        bail!("either a script file or --source must be provided");
    }
}
