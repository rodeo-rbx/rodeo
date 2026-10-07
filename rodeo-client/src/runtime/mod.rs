pub mod fs;
pub mod process;
pub mod roblox;
pub mod mesh;
pub mod scene;
pub mod stream;

use rodeo_proto::runtime_types as rt;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::Instrument;

// ---------------------------------------------------------------------------
// Service context routing
// ---------------------------------------------------------------------------

/// Where a service handler executes.
pub enum ServiceContext {
    /// Handle on the serve process (has access to server state, StudioMCP, etc.)
    Server,
    /// Forward to exec client (has access to local CWD, env, terminal, filesystem)
    Client,
}

/// Route an RPC method to its execution context.
/// This is the single place that declares where each service runs.
/// Moving a service between contexts = changing one line here.
pub fn context(method: &str) -> ServiceContext {
    match method {
        "mcp.call" => ServiceContext::Server,
        // All fs, stream, process operations need the client's local environment
        _ => ServiceContext::Client,
    }
}

/// Handle RPCs that can be served directly by the server (no run client needed).
/// Returns None for methods that require a client.
pub fn handle_server_dispatch(method: &str, _params: &Value) -> Option<Result<Value, String>> {
    match method {
        "process.getInfo" => {
            let resp = process::process_get_info(&rt::ProcessGetInfoRequest::default())
                .map(|r| serde_json::to_value(&r).unwrap_or(Value::Null));
            Some(resp)
        }
        _ => None,
    }
}

/// Policy enforcement — called by server BEFORE routing to either context.
/// Currently a no-op. Future: check session permissions.
pub fn check_policy(
    _method: &str,
    _params: &Value,
) -> Result<(), String> {
    // Future: session.policy.check(method, params)
    Ok(())
}

// ---------------------------------------------------------------------------
// Client-side RPC state (used by exec client for client-context services)
// ---------------------------------------------------------------------------

/// Stream handler operations. Client-internal state; not a wire type.
pub enum StreamHandler {
    // Boxed so a reader can also be an in-memory cursor (roblox.importEditableImage
    // hands decoded pixels to the plugin through the ordinary chunked reads).
    FileReader {
        reader: Box<dyn std::io::BufRead + Send>,
    },
    FileWriter {
        path: String,
        buffer: Vec<u8>,
    },
    FileAppender {
        path: String,
        buffer: Vec<u8>,
    },
    Stdout,
    Stderr,
    Stdin,
    ProcessStdin {
        stdin: ChildInput,
    },
    ProcessStdout {
        stdout: ChildOutput,
    },
    ProcessStderr {
        stderr: ChildOutput,
    },
}

/// A child process's stdout or stderr. Each pipe has its own lock: a read
/// waiting for output must not hold `RpcState`, or every other call of the
/// run (writing the child's stdin, printing) would wait with it.
pub type ChildOutput = Arc<Mutex<Box<dyn tokio::io::AsyncRead + Send + Unpin>>>;

/// A child process's stdin, locked on its own for the same reason.
pub type ChildInput = Arc<Mutex<tokio::process::ChildStdin>>;

/// A child started by `process.create`. A task owns the process and waits for
/// it: `kill` asks that task to kill it, and `exit` reports how it ended, so
/// waiting and killing work in any order and from any number of callers.
pub struct ChildProcess {
    pub kill: tokio::sync::mpsc::UnboundedSender<()>,
    pub exit: tokio::sync::watch::Receiver<Option<Result<std::process::ExitStatus, String>>>,
}

/// Which std stream a captured write came from. `StreamHandler::Stdout` /
/// `Stderr` writes are ALWAYS routed through the capture channel — consumers
/// of the resulting `RunStreamEvent::Output` decide where the bytes go
/// (real stdio for the CLI, `stream.data` notifications for the daemon,
/// `RunResult.output` for programmatic callers). The runtime itself never
/// touches `std::io::stdout()` — conflating "script output" with "process
/// stdout" was a legacy CLI-only assumption that broke every non-CLI consumer.
#[derive(Clone, Copy, Debug)]
pub enum CapturedStreamKind { Stdout, Stderr }

pub type CapturedOutputSender = tokio::sync::mpsc::UnboundedSender<(CapturedStreamKind, Vec<u8>)>;

/// Shared RPC state (per-execution, client-side).
pub struct RpcState {
    pub stream_handlers: HashMap<String, StreamHandler>,
    pub child_processes: HashMap<String, ChildProcess>,
    pub exit_code: i32,
    /// Set when the script called process.exit — the run ends via a
    /// client-initiated kill and the final result maps to `exit_code`
    /// (which may legitimately be 0).
    pub exit_requested: bool,
    pub next_pid: u32,
    /// Where `stream.write("stdout" | "stderr", ...)` bytes go.
    pub captured_output_tx: CapturedOutputSender,
    /// Engine capture-directory snapshots taken by `roblox.captureBegin`,
    /// keyed by token, consumed by the matching `roblox.captureCollect`.
    pub capture_snapshots: HashMap<String, roblox::CaptureSnapshot>,
    pub next_capture_token: u64,
}

impl RpcState {
    pub fn new(captured_output_tx: CapturedOutputSender) -> Self {
        let mut state = Self {
            stream_handlers: HashMap::new(),
            child_processes: HashMap::new(),
            exit_code: 0,
            exit_requested: false,
            next_pid: 0,
            captured_output_tx,
            capture_snapshots: HashMap::new(),
            next_capture_token: 0,
        };
        state.stream_handlers.insert("stdout".to_string(), StreamHandler::Stdout);
        state.stream_handlers.insert("stderr".to_string(), StreamHandler::Stderr);
        state.stream_handlers.insert("stdin".to_string(), StreamHandler::Stdin);
        state
    }
}

pub type SharedRpcState = Arc<Mutex<RpcState>>;

// ---------------------------------------------------------------------------
// Typed dispatch — the drift boundary.
//
// Takes a typed ClientRpcCall (oneof), dispatches to the matching handler,
// returns a typed ClientRpcResponse. Missing variants are compile errors.
// ---------------------------------------------------------------------------

fn box_res<T>(
    result: Result<T, String>,
    ctor: fn(Box<T>) -> rt::client_rpc_response::Res,
) -> Option<rt::client_rpc_response::Res> {
    Some(match result {
        Ok(v) => ctor(Box::new(v)),
        Err(e) => rt::client_rpc_response::Res::Error(e),
    })
}

fn sync_arm<T>(
    method: &'static str,
    id: &str,
    handler: impl FnOnce() -> Result<T, String>,
    ctor: fn(Box<T>) -> rt::client_rpc_response::Res,
) -> Option<rt::client_rpc_response::Res> {
    let span = tracing::debug_span!("rpc", method, id);
    box_res(span.in_scope(handler), ctor)
}

async fn async_arm<T>(
    method: &'static str,
    id: &str,
    fut: impl std::future::Future<Output = Result<T, String>>,
    ctor: fn(Box<T>) -> rt::client_rpc_response::Res,
) -> Option<rt::client_rpc_response::Res> {
    let span = tracing::debug_span!("rpc", method, id);
    box_res(fut.instrument(span).await, ctor)
}

pub async fn dispatch_client(
    state: SharedRpcState,
    call: &rt::ClientRpcCall,
) -> rt::ClientRpcResponse {
    use rt::client_rpc_call::Req;
    use rt::client_rpc_response::Res;

    let id = call.id.as_str();
    let res = match &call.req {
        // fs
        Some(Req::FsExists(r))  => sync_arm("fs.exists",  id, || fs::fs_exists(r),  Res::FsExists),
        Some(Req::FsStat(r))    => sync_arm("fs.stat",    id, || fs::fs_stat(r),    Res::FsStat),
        Some(Req::FsType(r))    => sync_arm("fs.type",    id, || fs::fs_type(r),    Res::FsType),
        Some(Req::FsMkdir(r))   => sync_arm("fs.mkdir",   id, || fs::fs_mkdir(r),   Res::FsMkdir),
        Some(Req::FsListdir(r)) => sync_arm("fs.listdir", id, || fs::fs_listdir(r), Res::FsListdir),
        Some(Req::FsRemove(r))  => sync_arm("fs.remove",  id, || fs::fs_remove(r),  Res::FsRemove),
        Some(Req::FsRmdir(r))   => sync_arm("fs.rmdir",   id, || fs::fs_rmdir(r),   Res::FsRmdir),
        Some(Req::FsCopy(r))    => sync_arm("fs.copy",    id, || fs::fs_copy(r),    Res::FsCopy),

        // stream
        Some(Req::StreamOpen(r))      => async_arm("stream.open",      id, stream::stream_open(state.clone(), r),       Res::StreamOpen).await,
        Some(Req::StreamReadChunk(r)) => async_arm("stream.readChunk", id, stream::stream_read_chunk(state.clone(), r), Res::StreamReadChunk).await,
        Some(Req::StreamReadLine(r))  => async_arm("stream.readLine",  id, stream::stream_read_line(state.clone(), r),  Res::StreamReadLine).await,
        Some(Req::StreamReadAll(r))   => async_arm("stream.readAll",   id, stream::stream_read_all(state.clone(), r),   Res::StreamReadAll).await,
        Some(Req::StreamWrite(r))     => async_arm("stream.write",     id, stream::stream_write(state.clone(), r),      Res::StreamWrite).await,
        Some(Req::StreamClose(r))     => async_arm("stream.close",     id, stream::stream_close(state.clone(), r),      Res::StreamClose).await,
        Some(Req::StreamReadBytes(r)) => async_arm("stream.readBytes", id, stream::stream_read_bytes(state.clone(), r), Res::StreamReadBytes).await,
        Some(Req::StreamWriteBytes(r)) => async_arm("stream.writeBytes", id, stream::stream_write_bytes(state.clone(), r), Res::StreamWriteBytes).await,

        // process
        Some(Req::ProcessGetInfo(r))   => sync_arm ("process.getInfo",    id, || process::process_get_info(r),                          Res::ProcessGetInfo),
        Some(Req::ProcessExit(r))      => async_arm("process.exit",       id, process::process_exit(state.clone(), r),                  Res::ProcessExit).await,
        Some(Req::ProcessRun(r))       => async_arm("process.run",        id, process::process_run(r),                                  Res::ProcessRun).await,
        Some(Req::ProcessSystem(r))    => async_arm("process.system",     id, process::process_system(r),                               Res::ProcessSystem).await,
        Some(Req::ProcessCreate(r))    => async_arm("process.create",     id, process::process_create(state.clone(), r),                Res::ProcessCreate).await,
        Some(Req::ProcessRunHandle(r)) => async_arm("process.run_handle", id, process::process_run_handle(state.clone(), r),            Res::ProcessRunHandle).await,
        Some(Req::ProcessKill(r))      => async_arm("process.kill",       id, process::process_kill(state.clone(), r),                  Res::ProcessKill).await,

        // mcp.call is server-context; the server dispatches it directly before forwarding to run-client.
        // Getting here means the routing was wrong.
        Some(Req::McpCall(_)) => Some(Res::Error("mcp.call must be dispatched server-side".to_string())),

        // roblox
        Some(Req::RobloxExport(r)) => async_arm("roblox.exportInstances", id, roblox::roblox_export(state.clone(), r), Res::RobloxExport).await,
        Some(Req::RobloxCaptureBegin(r)) => async_arm("roblox.captureBegin", id, roblox::roblox_capture_begin(state.clone(), r), Res::RobloxCaptureBegin).await,
        Some(Req::RobloxCaptureCollect(r)) => async_arm("roblox.captureCollect", id, roblox::roblox_capture_collect(state.clone(), r), Res::RobloxCaptureCollect).await,
        Some(Req::RobloxImageEncode(r)) => async_arm("roblox.imageEncode", id, roblox::roblox_image_encode(state.clone(), r), Res::RobloxImageEncode).await,
        Some(Req::RobloxImageDecode(r)) => async_arm("roblox.imageDecode", id, roblox::roblox_image_decode(state.clone(), r), Res::RobloxImageDecode).await,
        Some(Req::RobloxMeshEncode(r)) => async_arm("roblox.meshEncode", id, mesh::roblox_mesh_encode(state.clone(), r), Res::RobloxMeshEncode).await,
        Some(Req::RobloxMeshDecode(r)) => async_arm("roblox.meshDecode", id, mesh::roblox_mesh_decode(state.clone(), r), Res::RobloxMeshDecode).await,
        Some(Req::RobloxSceneDecode(r)) => async_arm("roblox.sceneDecode", id, scene::decode(state.clone(), r), Res::RobloxSceneDecode).await,
        Some(Req::RobloxSceneEncode(r)) => async_arm("roblox.sceneEncode", id, scene::encode(state.clone(), r), Res::RobloxSceneEncode).await,
        // Simulator sessions are handled inside the plugin (plugin identity);
        // one reaching the run client means the plugin predates them.
        Some(Req::RobloxSimulatorApply(_)) | Some(Req::RobloxSimulatorRestore(_)) => Some(Res::Error(
            "roblox.captureViewport device/viewportSize need a rodeo plugin that handles simulator RPCs; restart serve so the plugin matches this rodeo".to_string(),
        )),

        None => Some(Res::Error("missing req".to_string())),
    };

    guard_relay_size(rt::ClientRpcResponse {
        id: call.id.clone(),
        execution_id: call.execution_id.clone(),
        res,
        ..Default::default()
    })
}

/// Covers connect framing and the outer stream-message wrapper around a
/// guarded response, so a payload sized exactly at the cap still fits.
const RELAY_GUARD_MARGIN: usize = 4096;

/// Refuse to send any RPC response that would exceed the connectrpc envelope
/// cap. The receive side can't skip an oversized message — it drops the whole
/// stream, which tears down the run (transport death, no diagnostic). Rejecting
/// at the sender turns that into a run-scoped error the script sees at the
/// call site. Handlers that can return payloads that grow with user data must
/// chunk (see stream.luau's READ/WRITE_CHUNK); this guard is the backstop for
/// paths that don't yet.
fn guard_relay_size(mut response: rt::ClientRpcResponse) -> rt::ClientRpcResponse {
    use buffa::Message;
    let size = response.compute_size() as usize;
    if size + RELAY_GUARD_MARGIN <= rodeo_proto::MAX_RPC_MESSAGE_SIZE {
        return response;
    }
    tracing::warn!(
        id = response.id.as_str(),
        size,
        limit = rodeo_proto::MAX_RPC_MESSAGE_SIZE,
        "rpc response exceeds the relay envelope cap; erroring the call instead of killing the run stream"
    );
    response.res = Some(rt::client_rpc_response::Res::Error(format!(
        "rpc response ({size} bytes) exceeds the relay envelope limit ({} bytes); read or return the data in smaller chunks",
        rodeo_proto::MAX_RPC_MESSAGE_SIZE,
    )));
    response
}

