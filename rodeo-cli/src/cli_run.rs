//! CLI-flavored run execution: streams events from `rodeo_client` and copies
//! captured script stdout/stderr to this process's real stdio in real time.
//!
//! The rodeo-client crate never writes to real stdio — it emits
//! `RunStreamEvent::Output` and lets each consumer decide what to do with
//! those bytes. For the `rodeo run` CLI specifically, the expected UX is
//! live passthrough to the user's terminal, which is what this helper does.
//! Other consumers (the `__spawn_canonical_client` daemon, programmatic
//! library users) route the same events differently.

use std::collections::HashMap;
use std::io::Write;

use anyhow::Result;
use rodeo_client::run::RunStreamEvent;
use rodeo_client::runtime::CapturedStreamKind;
use rodeo_client::{RodeoClient, RunCodeOpts, RunResult};
use rodeo_proto as proto;

/// All inputs the CLI assembles for a single run. Translated to a
/// `RunCodeOpts` inside `run_piped`. Mirrors the old `client::RunRequest`
/// shape since this is how `commands/run.rs` and `commands/mcp.rs` already
/// describe a run.
pub struct RunRequest {
    pub script: String,
    /// Sparse routing spec (mode/dom_kind/context).
    pub route: crate::shared::target::RouteSpec,
    pub dom_id: Option<String>,
    /// Pin route-matched execution to this studio session (e.g. the Studio
    /// this run just launched via `--place`, or an explicit `--studio-id`).
    pub session: Option<String>,
    pub log_filter: proto::LogFilter,
    pub reload_requires: bool,
    pub script_args: Vec<String>,
    pub return_file: Option<String>,
    pub show_return: bool,
    pub output_file: Option<String>,
    pub verbose: bool,
    pub instance_path: Option<String>,
    pub script_path: Option<String>,
    pub profile: bool,
    pub profile_dir: Option<std::path::PathBuf>,
    /// Fired with the master-minted run id when the Created event arrives.
    /// Lets callers (e.g. the MCP server) kill the run on cancellation.
    pub on_created: Option<tokio::sync::oneshot::Sender<String>>,
}

/// Execute a run against a live server. Writes script stdout/stderr to the
/// CLI's real stdio as chunks arrive and returns a final `RunResult` whose
/// `output` field holds the merged captured text.
pub async fn run_piped(host: &str, port: u16, mut request: RunRequest) -> Result<RunResult> {
    let client = RodeoClient::connect(host, port)?;
    let mut on_created = request.on_created.take();
    let opts = RunCodeOpts {
        source: request.script,
        mode: request.route.mode.map(|m| m.as_str().to_string()),
        dom_kind: request.route.dom_kind.map(|d| d.as_str().to_string()),
        context: request.route.context.map(|c| c.as_str().to_string()),
        show_return: request.show_return,
        reload_requires: request.reload_requires,
        verbose: request.verbose,
        script_args: request.script_args,
        profile: request.profile,
        log_filter: Some(request.log_filter),
        instance_path: request.instance_path,
        script_path: request.script_path,
        return_file: request.return_file,
        output_file: request.output_file,
        profile_dir: request.profile_dir,
        session: request.session,
    };

    let mut stream = match request.dom_id.as_deref() {
        Some(dom_id) if !dom_id.is_empty() => {
            let dom = client.get_dom(dom_id).await?;
            dom.run_code_stream(opts).await?
        }
        _ => client.submit_run_stream(opts).await?,
    };

    let mut buffered_output = String::new();
    let mut files: HashMap<String, Vec<u8>> = HashMap::new();
    let mut final_result: Option<RunResult> = None;
    while let Some(ev) = stream.next().await {
        match ev {
            RunStreamEvent::Created { execution_id } => {
                if let Some(tx) = on_created.take() {
                    let _ = tx.send(execution_id);
                }
            }
            RunStreamEvent::Output { kind, chunk } => {
                match kind {
                    CapturedStreamKind::Stdout => {
                        let _ = std::io::stdout().write_all(chunk.as_bytes());
                        let _ = std::io::stdout().flush();
                    }
                    CapturedStreamKind::Stderr => {
                        let _ = std::io::stderr().write_all(chunk.as_bytes());
                        let _ = std::io::stderr().flush();
                    }
                }
                buffered_output.push_str(&chunk);
            }
            RunStreamEvent::FileChunk { filename, data, is_last: _ } => {
                files.entry(filename).or_default().extend_from_slice(&data);
            }
            RunStreamEvent::Done { result } => { final_result = Some(result); break; }
            RunStreamEvent::RpcCall { .. } => {}
        }
    }

    let mut result = final_result.unwrap_or_else(|| {
        // The stream task synthesizes a Done for every ending it can see
        // (Complete, Disconnect, transport error/EOF), so reaching here means
        // it died without one. Say so — a bare exit 2 with no output is
        // indistinguishable from "never ran" (issue #9).
        let error = "run ended without a result (stream task terminated unexpectedly)";
        eprintln!("rodeo: {error}");
        RunResult {
            execution_id: None,
            exit_code: 2, ok: false, output: String::new(), files: HashMap::new(), return_value: None,
            error: Some(error.to_string()),
        }
    });
    if result.output.is_empty() { result.output = buffered_output; }
    for (k, v) in files { result.files.entry(k).or_insert(v); }
    Ok(result)
}
