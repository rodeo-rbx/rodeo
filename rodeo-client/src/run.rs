use std::collections::HashMap;
use std::sync::Arc;

use anyhow::{anyhow, bail, Result};
use tokio::sync::{mpsc, Mutex};
use tokio::task::JoinHandle;

use crate::proto;
use crate::runtime;
use crate::transport::Transport;

/// User-facing options for `run_code` at every tier (client / studio / dom).
///
/// The routing fields (`mode`, `dom_kind`) apply to the routed tiers only —
/// `Dom::run_code` rejects them (a pinned DOM does no routing). `context`
/// applies everywhere: it's the run context the code executes as (cf. Roblox
/// `Script.RunContext`), not a routing selector.
#[derive(Default, Clone)]
pub struct RunCodeOpts {
    pub source: String,
    /// Studio mode to converge to: "edit" | "run" | "test" | "play".
    pub mode: Option<String>,
    /// Which DOM role receives the script: "server" | "client".
    pub dom_kind: Option<String>,
    /// Run context: "plugin" | "server" | "client" | "elevated" | "cmdbar".
    pub context: Option<String>,
    pub show_return: bool,
    pub reload_requires: bool,
    pub verbose: bool,
    pub script_args: Vec<String>,
    pub profile: bool,
    pub log_filter: Option<proto::LogFilter>,
    pub instance_path: Option<String>,
    pub script_path: Option<String>,
    pub return_file: Option<String>,
    pub output_file: Option<String>,
    pub profile_dir: Option<std::path::PathBuf>,
    /// Session filter for target-routed submissions (empty `dom_id`): the
    /// server only matches DOMs belonging to this studio session. Lets a
    /// caller that just launched a Studio pin its run to that Studio instead
    /// of load-balancing across every session on the serve. Ignored on the
    /// `Dom`-handle path, which pins by the DOM's own session.
    pub session: Option<String>,
}

/// Buffered result of `Dom::run_code` — aligns with the TS RunResult shape.
#[derive(Default)]
pub struct RunResult {
    /// Master-minted run id (from the ProcessCreated event). `None` only if
    /// the stream died before the first event arrived.
    pub execution_id: Option<String>,
    pub exit_code: i32,
    pub ok: bool,
    pub output: String,
    pub files: HashMap<String, Vec<u8>>,
    /// JSON-encoded return value from the script, untouched. `None` if the
    /// script didn't return anything, if a return file captured the value
    /// instead (the value is in the file, not on the wire), or if the value
    /// exceeded the 2MiB wire cap with show_return set (printed to stdout,
    /// omitted here; without show_return an over-cap value fails the run).
    /// Clients are expected to `JSON.parse` / `json.deserialize` and surface
    /// as `result.return`.
    pub return_value: Option<String>,
    /// Why the run failed, when it did: the module's own error (with its
    /// trace), the runner's (a context the Studio can't provide, a run that
    /// failed to start, an oversized return), or the stream's. `None` for a
    /// successful or killed run.
    pub error: Option<String>,
}

/// The color the plugin's log capture gives Studio's error lines; a run's
/// error printed here matches them.
const ERROR_COLOR: &str = "\x1b[1m\x1b[38;2;200;0;0m";
const RESET: &str = "\x1b[0m";

/// Streaming event emitted by `Dom::run_code_stream`. Mirrors the JSON-RPC
/// daemon's `stream.data` payload shapes so the daemon can emit these directly.
pub enum RunStreamEvent {
    /// First event on every run: the master-minted run id. Callers that may
    /// need to kill the run (e.g. on cancellation) capture it here.
    Created { execution_id: String },
    /// A chunk of bytes the running script wrote to its stdout or stderr.
    Output { kind: runtime::CapturedStreamKind, chunk: String },
    RpcCall { call: Box<proto::runtime::ClientRpcCall> },
    FileChunk { filename: String, data: Vec<u8>, is_last: bool },
    /// Terminal: execution finished (success, failure, or kill). The result's
    /// `ok`/`exit_code` fields carry the outcome. Killed runs produce
    /// `ok=false, exit_code=1`; disconnects produce `ok=false, exit_code=2`.
    Done { result: RunResult },
}

pub struct RunStream {
    rx: mpsc::UnboundedReceiver<RunStreamEvent>,
    _task: JoinHandle<()>,
}

impl RunStream {
    pub async fn next(&mut self) -> Option<RunStreamEvent> {
        self.rx.recv().await
    }
}

/// Core runner — opens a bidi stream, loops, and feeds events into a channel.
/// Returns the JoinHandle + the receiver for the channel. Used by both the
/// buffered and streaming APIs.
async fn run_inner(
    transport: Arc<Transport>,
    dom_id: &str,
    session_guid: Option<String>,
    opts: RunCodeOpts,
) -> Result<(
    mpsc::UnboundedReceiver<RunStreamEvent>,
    JoinHandle<()>,
)> {
    // A run's error is printed with the run's output unless error output is
    // off (--no-error / --no-output), which also hides Studio's error lines.
    let print_errors = opts.log_filter.as_ref().is_none_or(|f| f.enable_error);
    let client = transport.run_service();
    let mut bidi = client.run().await
        .map_err(|e| anyhow!("failed to open run stream (is 'rodeo serve' running?): {e}"))?;

    // No id in the submit — the master mints the run id and returns it as the
    // first event on the stream (ProcessCreated).
    let submit = proto::SubmitRequest {
        script: opts.source,
        mode: opts.mode,
        dom_kind: opts.dom_kind,
        context: opts.context,
        session: session_guid,
        dom_id: if dom_id.is_empty() { None } else { Some(dom_id.to_string()) },
        log_filter: opts.log_filter.map(buffa::MessageField::some).unwrap_or_else(buffa::MessageField::none),
        reload_requires: if opts.reload_requires { Some(true) } else { None },
        script_args: opts.script_args,
        return_file: opts.return_file,
        show_return: if opts.show_return { Some(true) } else { None },
        output_file: opts.output_file,
        verbose: if opts.verbose { Some(true) } else { None },
        instance_path: opts.instance_path,
        script_path: opts.script_path,
        profile: if opts.profile { Some(true) } else { None },
        client_version: proto::BUILD_ID.to_string(),
        ..Default::default()
    };
    // Large scripts are split across ScriptChunk messages so no hop's
    // envelope grows with bundle size (an oversized single message drops the
    // whole stream on the receiving side — the silent exit 2 of issue #9).
    // The ceiling is a sanity check against accidents, not a real limit.
    let mut submit = submit;
    let script_len = submit.script.len();
    if script_len > rodeo_proto::MAX_SCRIPT_SIZE {
        let entrypoint = submit.script_path.as_deref().unwrap_or("<inline source>");
        bail!(
            "bundled script is absurdly large: {:.1} MB exceeds the {} MB sanity ceiling (entrypoint: {entrypoint}). \
             This usually means the bundle swallowed something unintended. Large data doesn't belong in the \
             bundle regardless — bake it to files and read it at runtime (fs.open + stream.readBytes, or \
             roblox.importInstances for models)",
            script_len as f64 / 1_048_576.0,
            rodeo_proto::MAX_SCRIPT_SIZE / 1_048_576,
        );
    }
    let mut tail_chunks: Vec<proto::ScriptChunk> = Vec::new();
    if script_len > rodeo_proto::SCRIPT_CHUNK_SIZE {
        tracing::warn!(
            "large bundle: {:.1} MB of script source (entrypoint: {}). It will run, but Studio compiles \
             all of it per run — baking data to files and reading them at runtime starts faster",
            script_len as f64 / 1_048_576.0,
            submit.script_path.as_deref().unwrap_or("<inline source>"),
        );
        let full = std::mem::take(&mut submit.script);
        let mut rest = full.as_str();
        let mut pieces: Vec<&str> = Vec::new();
        while rest.len() > rodeo_proto::SCRIPT_CHUNK_SIZE {
            // Back off to a char boundary — a split codepoint would make the
            // chunk invalid UTF-8 and fail serialization.
            let mut cut = rodeo_proto::SCRIPT_CHUNK_SIZE;
            while !rest.is_char_boundary(cut) {
                cut -= 1;
            }
            let (head, next) = rest.split_at(cut);
            pieces.push(head);
            rest = next;
        }
        pieces.push(rest);
        submit.script = pieces[0].to_string();
        submit.script_continues = Some(true);
        let last = pieces.len() - 1;
        for (i, piece) in pieces.iter().enumerate().skip(1) {
            tail_chunks.push(proto::ScriptChunk {
                execution_id: String::new(),
                data: piece.to_string(),
                is_last: i == last,
                ..Default::default()
            });
        }
    }
    bidi.send(proto::RunClientMessage {
        msg: Some(proto::run_client_message::Msg::Submit(Box::new(submit))),
        ..Default::default()
    })
        .await
        .map_err(|e| anyhow!("failed to send submit: {e}"))?;
    for chunk in tail_chunks {
        bidi.send(proto::RunClientMessage {
            msg: Some(proto::run_client_message::Msg::ScriptChunk(Box::new(chunk))),
            ..Default::default()
        })
            .await
            .map_err(|e| anyhow!("failed to send script chunk: {e}"))?;
    }

    let (event_tx, event_rx) = mpsc::unbounded_channel::<RunStreamEvent>();
    let profile_dir = opts.profile_dir.clone();

    let task = tokio::spawn(async move {
        message_loop(&mut bidi, profile_dir, print_errors, event_tx).await;
    });

    Ok((event_rx, task))
}

pub(crate) async fn run_stream(
    transport: Arc<Transport>,
    dom_id: &str,
    session_guid: Option<String>,
    opts: RunCodeOpts,
) -> Result<RunStream> {
    let (rx, task) = run_inner(transport, dom_id, session_guid, opts).await?;
    Ok(RunStream { rx, _task: task })
}

/// Route-matched streaming variant — empty `dom_id` tells the server to route
/// by the opts' mode/dom_kind/context fields (within `opts.session` if set).
/// Used by the client/studio tiers and CLI paths that rely on master-side
/// routing (e.g. `rodeo run --mode play --context server`).
pub(crate) async fn run_stream_routed(
    transport: Arc<Transport>,
    opts: RunCodeOpts,
) -> Result<RunStream> {
    let session = opts.session.clone();
    run_stream(transport, "", session, opts).await
}

pub(crate) async fn run_buffered_routed(
    transport: Arc<Transport>,
    opts: RunCodeOpts,
) -> Result<RunResult> {
    // dom_id empty → server routes by mode/dom_kind (within opts.session if set)
    let session = opts.session.clone();
    run_buffered(transport, "", session, opts).await
}

pub(crate) async fn run_buffered(
    transport: Arc<Transport>,
    dom_id: &str,
    session_guid: Option<String>,
    opts: RunCodeOpts,
) -> Result<RunResult> {
    let (mut rx, task) = run_inner(transport, dom_id, session_guid, opts).await?;
    let mut execution_id: Option<String> = None;
    let mut output = String::new();
    let mut files: HashMap<String, Vec<u8>> = HashMap::new();
    let mut exit_code = 0;
    let mut ok = true;
    let mut return_value: Option<String> = None;
    let mut error: Option<String> = None;
    while let Some(ev) = rx.recv().await {
        match ev {
            RunStreamEvent::Created { execution_id: id } => execution_id = Some(id),
            RunStreamEvent::Output { kind: _, chunk } => output.push_str(&chunk),
            RunStreamEvent::FileChunk { filename, data, is_last: _ } => {
                files.entry(filename).or_default().extend_from_slice(&data);
            }
            RunStreamEvent::Done { result } => {
                output = if result.output.is_empty() { output } else { result.output };
                exit_code = result.exit_code;
                ok = result.ok;
                for (k, v) in result.files { files.insert(k, v); }
                return_value = result.return_value;
                error = result.error;
                break;
            }
            RunStreamEvent::RpcCall { .. } => {}
        }
    }
    let _ = task.await;
    Ok(RunResult { execution_id, exit_code, ok, output, files, return_value, error })
}

/// The bidi message loop — drains incoming events, forwards them as
/// `RunStreamEvent`s, and handles client-side RPC dispatch against the local
/// filesystem/stdio. Returns after `Done`, `Killed`, or `Disconnected`.
async fn message_loop(
    bidi: &mut connectrpc::client::BidiStream<hyper::body::Incoming, proto::RunClientMessage, proto::RunEventView<'static>>,
    profile_dir: Option<std::path::PathBuf>,
    print_errors: bool,
    event_tx: mpsc::UnboundedSender<RunStreamEvent>,
) {
    // Master-minted run id, learned from the first stream event (Created).
    let mut execution_id: Option<String> = None;
    // The runtime unconditionally routes script stdout/stderr writes through
    // this channel — it never touches the process's real std streams. Each
    // consumer (CLI, daemon, programmatic) decides where the bytes go.
    //
    // We drain this channel inline inside the main `select!` (alongside bidi
    // events and rpc responses) rather than in a separate task: with a task,
    // it's possible for Complete to arrive and emit `Done` before the task
    // has forwarded the last captured bytes, leaving trailing Output events
    // stranded. An inline branch plus an explicit drain at ExecutionDone
    // keeps ordering deterministic — all Output events for this execution
    // land in `event_tx` before `Done`.
    let (capture_tx, mut capture_rx) =
        mpsc::unbounded_channel::<(runtime::CapturedStreamKind, Vec<u8>)>();
    let rpc_state = Arc::new(Mutex::new(runtime::RpcState::new(capture_tx)));
    let forward_captured = |kind: runtime::CapturedStreamKind, bytes: Vec<u8>| {
        let chunk = String::from_utf8_lossy(&bytes).into_owned();
        let _ = event_tx.send(RunStreamEvent::Output { kind, chunk });
    };

    let mut rpc_tasks: Vec<JoinHandle<()>> = Vec::new();
    let mut file_buffers: HashMap<String, Vec<u8>> = HashMap::new();
    let mut exit_code = 0;
    let mut ok = true;
    let mut return_value: Option<String> = None;
    let mut run_error: Option<String> = None;

    let (response_tx, mut response_rx) = mpsc::unbounded_channel::<proto::RunClientMessage>();

    loop {
        tokio::select! {
            event = bidi.message() => {
                match event {
                    Ok(Some(view)) => {
                        let event: proto::RunEvent = view.to_owned_message();
                        if let Some(evt) = event.event {
                            match evt {
                                proto::run_event::Event::Created(created) => {
                                    tracing::debug!(id = created.execution_id.as_str(), "process created");
                                    execution_id = Some(created.execution_id.clone());
                                    let _ = event_tx.send(RunStreamEvent::Created {
                                        execution_id: created.execution_id,
                                    });
                                }
                                proto::run_event::Event::RpcCall(call) => {
                                    let state = rpc_state.clone();
                                    let tx = response_tx.clone();
                                    let observer = event_tx.clone();
                                    let call_boxed = call.clone();
                                    let handle = tokio::spawn(async move {
                                        let _ = observer.send(RunStreamEvent::RpcCall { call: Box::new(*call_boxed) });
                                        // process.exit: the script suspends plugin-side after this
                                        // notify — ending the run is the client's job. Reuse the
                                        // kill path (the master forwards it to the DOM); the final
                                        // Killed/Done maps to the recorded exit code below.
                                        let is_exit = matches!(
                                            &call.req,
                                            Some(rodeo_proto::runtime_types::client_rpc_call::Req::ProcessExit(_))
                                        );
                                        let eid = call.execution_id.clone();
                                        let response = runtime::dispatch_client(state, &call).await;
                                        let _ = tx.send(proto::RunClientMessage {
                                            msg: Some(proto::run_client_message::Msg::RpcResponse(Box::new(response))),
                                            ..Default::default()
                                        });
                                        if is_exit {
                                            let _ = tx.send(proto::RunClientMessage {
                                                msg: Some(proto::run_client_message::Msg::Kill(Box::new(proto::KillCommand {
                                                    execution_id: eid,
                                                    ..Default::default()
                                                }))),
                                                ..Default::default()
                                            });
                                        }
                                    });
                                    rpc_tasks.push(handle);
                                }
                                proto::run_event::Event::ExecutionDone(done) => {
                                    if !done.success { exit_code = 1; ok = false; }
                                    let (rpc_exit, exit_requested) = {
                                        let st = rpc_state.lock().await;
                                        (st.exit_code, st.exit_requested)
                                    };
                                    if exit_requested {
                                        exit_code = rpc_exit;
                                        ok = rpc_exit == 0;
                                    } else if rpc_exit != 0 { exit_code = rpc_exit; ok = false; }
                                    return_value = done.return_value.clone();
                                    for task in rpc_tasks.drain(..) { let _ = task.await; }
                                    // After every in-flight stream.write RPC has
                                    // resolved, drain any captured bytes they
                                    // produced so they reach event_tx BEFORE we
                                    // process Complete below.
                                    while let Ok((kind, bytes)) = capture_rx.try_recv() {
                                        forward_captured(kind, bytes);
                                    }
                                    // The run's error, after its output. One Studio
                                    // already logged (the module's own) is in that
                                    // output, so it isn't printed twice.
                                    run_error = done.error.clone().filter(|e| !e.is_empty());
                                    if let Some(err) = run_error.as_deref() {
                                        if print_errors && !done.error_in_output {
                                            let _ = event_tx.send(RunStreamEvent::Output {
                                                kind: runtime::CapturedStreamKind::Stderr,
                                                chunk: format!("{ERROR_COLOR}{err}{RESET}\n"),
                                            });
                                        }
                                    }
                                }
                                proto::run_event::Event::ExecutionKilled(_) => {
                                    // A kill is not an error to the caller — it's just a
                                    // completed run with ok=false. Let the normal
                                    // ExecutionDone/Complete flow carry the result (mirrors
                                    // the pre-refactor run_loop semantics; otherwise
                                    // `await runCode()` rejects and tests that await a
                                    // deliberate kill blow up).
                                    // Exception: a kill the client itself initiated for
                                    // process.exit resolves to the script's requested code.
                                    let (rpc_exit, exit_requested) = {
                                        let st = rpc_state.lock().await;
                                        (st.exit_code, st.exit_requested)
                                    };
                                    if exit_requested {
                                        exit_code = rpc_exit;
                                        ok = rpc_exit == 0;
                                    } else {
                                        exit_code = 1;
                                        ok = false;
                                    }
                                    for task in rpc_tasks.drain(..) { let _ = task.await; }
                                    while let Ok((kind, bytes)) = capture_rx.try_recv() {
                                        forward_captured(kind, bytes);
                                    }
                                }
                                proto::run_event::Event::Disconnect(reason) => {
                                    // Stream is lost — server won't send Complete.
                                    // Surface the reason (the only error channel
                                    // consumers read is the output stream), then
                                    // synthesize a terminal Done with ok=false so
                                    // consumers don't hang waiting.
                                    let _ = event_tx.send(RunStreamEvent::Output {
                                        kind: runtime::CapturedStreamKind::Stderr,
                                        chunk: format!("rodeo: run disconnected: {reason}\n"),
                                    });
                                    let files_out = std::mem::take(&mut file_buffers);
                                    let _ = event_tx.send(RunStreamEvent::Done {
                                        result: RunResult {
                                            execution_id: execution_id.clone(),
                                            exit_code: 2,
                                            ok: false,
                                            output: String::new(),
                                            files: files_out,
                                            return_value: return_value.take(),
                                            error: Some(format!("run disconnected: {reason}")),
                                        },
                                    });
                                    break;
                                }
                                proto::run_event::Event::Complete(_) => {
                                    let files_out = std::mem::take(&mut file_buffers);
                                    // `output` on the Done event is left empty — the capture
                                    // pump is the canonical source of Output events, and
                                    // consumers that care about the string form reconstruct
                                    // it from those (run_buffered accumulates; the CLI shim
                                    // writes through + accumulates).
                                    let _ = event_tx.send(RunStreamEvent::Done {
                                        result: RunResult {
                                            execution_id: execution_id.clone(),
                                            exit_code,
                                            ok,
                                            output: String::new(),
                                            files: files_out,
                                            return_value: return_value.take(),
                                            error: run_error.take(),
                                        },
                                    });
                                    break;
                                }
                                proto::run_event::Event::FileChunk(chunk) => {
                                    let buf = file_buffers.entry(chunk.filename.clone()).or_default();
                                    buf.extend_from_slice(&chunk.data);
                                    let _ = event_tx.send(RunStreamEvent::FileChunk {
                                        filename: chunk.filename.clone(),
                                        data: chunk.data.clone(),
                                        is_last: chunk.is_last,
                                    });
                                    if chunk.is_last {
                                        if let Some(dir) = profile_dir.as_ref() {
                                            let _ = std::fs::create_dir_all(dir);
                                            let tmp = dir.join(format!("{}.tmp", &chunk.filename));
                                            let final_path = dir.join(&chunk.filename);
                                            if std::fs::write(&tmp, buf).is_ok() {
                                                let _ = std::fs::rename(&tmp, &final_path);
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                    // Transport-level endings. `Disconnect` above is the
                    // *server-sent* teardown; these arms are the transport
                    // itself dying — e.g. the master dropping the stream on
                    // receipt of an oversized submission. A silent break here
                    // left run_piped to fabricate a bare exit 2 with empty
                    // output (issue #9), so surface the reason on the output
                    // stream (the only error channel consumers read) and
                    // synthesize a terminal Done.
                    ended => {
                        let reason = match ended {
                            Err(e) => format!("rodeo: run stream failed: {e}\n"),
                            _ => "rodeo: run stream closed before the run completed (the server dropped the connection — an oversized submission exceeds the transport limit)\n".to_string(),
                        };
                        let error = reason.trim().trim_start_matches("rodeo: ").to_string();
                        let _ = event_tx.send(RunStreamEvent::Output {
                            kind: runtime::CapturedStreamKind::Stderr,
                            chunk: reason,
                        });
                        let files_out = std::mem::take(&mut file_buffers);
                        let _ = event_tx.send(RunStreamEvent::Done {
                            result: RunResult {
                                execution_id: execution_id.clone(),
                                exit_code: 2,
                                ok: false,
                                output: String::new(),
                                files: files_out,
                                return_value: return_value.take(),
                                error: Some(error),
                            },
                        });
                        break;
                    }
                }
            }
            msg = response_rx.recv() => {
                if let Some(m) = msg {
                    let _ = bidi.send(m).await;
                }
            }
            captured = capture_rx.recv() => {
                if let Some((kind, bytes)) = captured {
                    forward_captured(kind, bytes);
                }
            }
            // The consumer dropped its RunStream: without this branch the
            // detached task would keep the bidi open and the script would run
            // to completion in Studio. Breaking drops `bidi`, the master sees
            // the stream close, and `disconnect_run` auto-kills the run.
            _ = event_tx.closed() => break,
        }
    }

    // Flush any open stream handlers to disk (matches the CLI's flush_streams).
    let mut guard = rpc_state.lock().await;
    let handles: Vec<String> = guard.stream_handlers.keys().cloned().collect();
    for handle in handles {
        if let Some(handler) = guard.stream_handlers.remove(&handle) {
            match handler {
                runtime::StreamHandler::FileWriter { path, buffer } => { let _ = std::fs::write(&path, &buffer); }
                runtime::StreamHandler::FileAppender { path, buffer } => { let _ = std::fs::write(&path, &buffer); }
                _ => {}
            }
        }
    }
    drop(guard);

    // Clean up any leftover temp files keyed by execution_id (same behavior as
    // CLI). If the stream died before Created arrived, no run was dispatched,
    // so there's no temp dir to clean.
    if let Some(eid) = execution_id.as_deref() {
        if let Some(content_path) = studio_content_path_fallback() {
            let temp_dir = std::path::Path::new(&content_path).join("rodeo-temp").join(eid);
            if temp_dir.exists() { let _ = std::fs::remove_dir_all(&temp_dir); }
        }
    }
}

/// Locate the Studio `content/` directory via `roblox_install`. Same logic as
/// `rbx_control::studio::launch::studio_content_path`.
fn studio_content_path_fallback() -> Option<String> {
    roblox_install::RobloxStudio::locate()
        .ok()
        .map(|s| s.content_path().to_string_lossy().to_string())
}
