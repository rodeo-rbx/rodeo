//! Internal: `rodeo __spawn_canonical_client --host H --port P`
//!
//! Long-lived JSON-RPC 2.0 daemon over newline-delimited JSON on stdin/stdout.
//! Wrappers in other languages (TypeScript, Luau) spawn this subprocess and
//! talk to it instead of re-implementing connectrpc client logic.
//!
//! **Load-bearing principle: this file is a thin envelope over `rodeo_client`.**
//! Every method is a short adapter:
//!   (1) look up handles in the daemon-local maps,
//!   (2) call the corresponding `rodeo_client` method,
//!   (3) insert any newly-minted handles into the maps,
//!   (4) serialize the return to JSON.
//!
//! If you find yourself writing business logic here, move it to `rodeo-client`
//! — that's the canonical impl that CLI / TS / Luau all share.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use anyhow::{anyhow, Result};
use rodeo_client::{
    MultiplayerTest, RodeoClient, RunCodeOpts,
    Studio, StudioBackend, Dom,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::Mutex;

// ---------------------------------------------------------------------------
// Wire types
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct JsonRpcRequest {
    #[allow(dead_code)]
    jsonrpc: Option<String>,
    method: String,
    #[serde(default)]
    params: Value,
    id: Option<Value>,
}

#[derive(Serialize)]
struct JsonRpcResponse {
    jsonrpc: &'static str,
    id: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<JsonRpcErrorObj>,
}

#[derive(Serialize)]
struct JsonRpcErrorObj {
    code: i32,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    data: Option<Value>,
}

#[derive(Serialize)]
struct JsonRpcNotification {
    jsonrpc: &'static str,
    method: &'static str,
    params: Value,
}

// ---------------------------------------------------------------------------
// Daemon state
// ---------------------------------------------------------------------------

type Handles<T> = Mutex<HashMap<String, Arc<T>>>;

struct State {
    client: RodeoClient,
    backends: Handles<StudioBackend>,
    // `Studio` is mutable (setMode mutates its DOM handles). Wrap in an extra
    // Mutex so method handlers can take an exclusive borrow even when the
    // Arc itself is cloned out of the map.
    studios: Mutex<HashMap<String, Arc<Mutex<Studio>>>>,
    /// Running in-Studio multiplayer tests, keyed by a minted `mp` handle.
    /// `MultiplayerTest` is mutable (connectClient/disconnectClient update its
    /// client handles), so wrap each in a Mutex.
    mp_tests: Mutex<HashMap<String, Arc<Mutex<MultiplayerTest>>>>,
    next_handle: AtomicU64,
    /// Cancel channels keyed by the client-provided streamId — `dom.cancelRun`
    /// drops the sender to signal the runCode task to stop.
    streams: Mutex<HashMap<String, tokio::sync::oneshot::Sender<()>>>,
    /// Serializes stdout writes across concurrent tasks.
    stdout: Mutex<tokio::io::Stdout>,
}

impl State {
    fn mint_handle(&self, prefix: &str) -> String {
        let n = self.next_handle.fetch_add(1, Ordering::Relaxed);
        format!("{prefix}-{n}")
    }
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

pub async fn main(host: String, port: u16) -> Result<()> {
    let client = RodeoClient::connect(&host, port)?;
    let state = Arc::new(State {
        client,
        backends: Default::default(),
        studios: Default::default(),
        mp_tests: Default::default(),
        next_handle: AtomicU64::new(1),
        streams: Default::default(),
        stdout: Mutex::new(tokio::io::stdout()),
    });

    let stdin = tokio::io::stdin();
    let mut lines = BufReader::new(stdin).lines();
    while let Some(line) = lines.next_line().await? {
        let line = line.trim().to_string();
        if line.is_empty() { continue; }
        // Handle inline so the response is written before we consume the next
        // line. Long-running work (dom.runCode streaming) spawns its own
        // background task for emitting notifications — the initial RPC call
        // returns quickly with a streamId, so this doesn't block throughput.
        handle_line(state.clone(), line).await;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Frame dispatch
// ---------------------------------------------------------------------------

async fn handle_line(state: Arc<State>, line: String) {
    let req: JsonRpcRequest = match serde_json::from_str(&line) {
        Ok(r) => r,
        Err(e) => {
            write_error(&state, Value::Null, -32700, &format!("parse error: {e}"), None).await;
            return;
        }
    };
    let id = req.id.clone().unwrap_or(Value::Null);
    match dispatch(state.clone(), &req.method, req.params).await {
        Ok(result) => write_response(&state, id, result).await,
        Err(e) => write_error(&state, id, -32603, &e.to_string(), None).await,
    }
}

async fn dispatch(state: Arc<State>, method: &str, params: Value) -> Result<Value> {
    match method {
        // client.*
        "client.isHealthy" => Ok(json!(state.client.is_healthy().await)),
        // Version handshake for wrappers: `ok` is the verdict (honours
        // RODEO_SKIP_VERSION_CHECK), `message` explains a mismatch. Wrappers
        // call it once isHealthy is true and refuse to proceed on !ok.
        "client.checkVersion" => {
            use rodeo_client::proto::{check_peer_version, version_check_skipped, SKIP_VERSION_CHECK_ENV};
            let health = state.client.health().await?;
            let label = format!("the master at {}:{}", state.client.host(), state.client.port());
            let (ok, message) = match check_peer_version(&label, &health.version) {
                Ok(()) => (true, None),
                Err(msg) => (
                    version_check_skipped(),
                    Some(format!("{msg}. Stop that serve and start one from this rodeo, or set {SKIP_VERSION_CHECK_ENV}=1 to proceed anyway.")),
                ),
            };
            Ok(json!({
                "ok": ok,
                "server": health.version,
                "client": rodeo_client::BUILD_ID,
                "message": message,
            }))
        }
        "client.getState" => {
            let mut snapshot = serde_json::to_value(state.client.get_state().await?)?;
            // proto3 JSON omits empty `repeated` fields, but the wire contract
            // (StateSnapshotDTO) is "studios is always an array" — every consumer
            // does state.studios.{find,filter,map} unguarded. Materialize it so an
            // empty snapshot (e.g. right after the last Studio closes) doesn't
            // crash callers with "undefined is not an object".
            if let Some(obj) = snapshot.as_object_mut() {
                obj.entry("studios").or_insert_with(|| Value::Array(Vec::new()));
            }
            Ok(snapshot)
        }
        "client.listBackends" => {
            let kind = params.get("kind").and_then(|v| v.as_str()).map(String::from);
            let list = state.client.list_backends(kind.as_deref()).await?;
            Ok(serde_json::to_value(list)?)
        }
        "client.getLocalStudio" => {
            let backend = state.client.get_local_studio().await?;
            let info = serde_json::json!({ "id": backend.id, "name": backend.name });
            let handle = state.mint_handle("b");
            state.backends.lock().await.insert(handle.clone(), Arc::new(backend));
            Ok(json!({ "backendHandle": handle, "info": info }))
        }
        "client.getBackend" => {
            let id_or_name = params.get("idOrName").and_then(|v| v.as_str())
                .ok_or_else(|| anyhow!("idOrName required"))?;
            let backend = state.client.get_backend(id_or_name).await?;
            let info = serde_json::json!({ "id": backend.id, "name": backend.name });
            let handle = state.mint_handle("b");
            state.backends.lock().await.insert(handle.clone(), Arc::new(backend));
            Ok(json!({ "backendHandle": handle, "info": info }))
        }
        "client.getDoms" => {
            let doms = state.client.get_doms().await?;
            Ok(serde_json::to_value(doms.iter().map(dom_snapshot).collect::<Vec<_>>())?)
        }
        "client.runCode" => client_run_code(state, params).await,
        "client.listProcesses" => {
            let list = state.client.list_processes().await?;
            Ok(serde_json::to_value(list)?)
        }
        "client.kill" => {
            let id = params.get("executionId").and_then(|v| v.as_str())
                .ok_or_else(|| anyhow!("executionId required"))?;
            state.client.kill(id).await?;
            Ok(Value::Null)
        }

        // studio.*
        "studio.open" => studio_open(state, params).await,
        "studio.openPlace" => studio_open_place(state, params).await,
        "studio.openFile" => studio_open_file(state, params).await,
        "studio.setMode" => studio_set_mode(state, params).await,
        "studio.getMode" => studio_get_mode(state, params).await,
        "studio.save" => studio_save(state, params).await,
        "studio.close" => studio_close(state, params).await,
        "studio.getDoms" => studio_get_doms(state, params).await,
        "studio.runCode" => studio_run_code(state, params).await,
        "studio.startMultiplayerTest" => studio_start_multiplayer_test(state, params).await,

        // mp.* — in-Studio multiplayer test lifecycle, keyed by an `mpHandle`
        // returned from studio.startMultiplayerTest. The server/client DOMs are
        // run via dom.runCode by domId like any other DOM.
        "mp.connectClient" => mp_connect_client(state, params).await,
        "mp.disconnectClient" => mp_disconnect_client(state, params).await,
        "mp.close" => mp_close(state, params).await,

        // dom.*
        "dom.runCode" => dom_run_code(state, params).await,
        "dom.cancelRun" => dom_cancel_run(state, params).await,

        _ => anyhow::bail!("unknown method: {method}"),
    }
}

// ---------------------------------------------------------------------------
// studio.* adapters
// ---------------------------------------------------------------------------

async fn lookup_backend(state: &State, params: &Value) -> Result<Arc<StudioBackend>> {
    let h = params.get("backendHandle").and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("backendHandle required"))?;
    state.backends.lock().await.get(h).cloned()
        .ok_or_else(|| anyhow!("unknown backendHandle: {h}"))
}

async fn lookup_studio(state: &State, params: &Value) -> Result<Arc<Mutex<Studio>>> {
    let h = params.get("studioHandle").and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("studioHandle required"))?;
    state.studios.lock().await.get(h).cloned()
        .ok_or_else(|| anyhow!("unknown studioHandle: {h}"))
}

async fn insert_studio(state: &State, studio: Studio) -> (String, Arc<Mutex<Studio>>) {
    let handle = state.mint_handle("s");
    let arc = Arc::new(Mutex::new(studio));
    state.studios.lock().await.insert(handle.clone(), arc.clone());
    (handle, arc)
}

/// Optional `timeoutMs` param shared by every call that waits on studio
/// state (open*, setMode, startMultiplayerTest, mp.connectClient). Absent =
/// wait indefinitely.
fn parse_timeout(params: &Value) -> Option<std::time::Duration> {
    params.get("timeoutMs").and_then(|v| v.as_u64()).map(std::time::Duration::from_millis)
}

fn parse_open_opts(params: &Value) -> rodeo_client::studio::OpenOpts {
    rodeo_client::studio::OpenOpts {
        fflags: params.get("fflags").and_then(|v| v.as_array())
            .map(|a| a.iter().filter_map(|s| s.as_str().map(String::from)).collect())
            .unwrap_or_default(),
        background: params.get("background").and_then(|v| v.as_bool()).unwrap_or(false),
        profile: params.get("profile").and_then(|v| v.as_bool()).unwrap_or(false),
        save: params.get("save").and_then(|v| v.as_str()).map(String::from),
        detached: params.get("detached").and_then(|v| v.as_bool()).unwrap_or(false),
        fflag_file: params.get("fflagFile").and_then(|v| v.as_str()).map(String::from),
        show_widgets: params.get("showWidgets").and_then(|v| v.as_str()).map(String::from),
        timeout: parse_timeout(params),
    }
}

async fn studio_open(state: Arc<State>, params: Value) -> Result<Value> {
    let backend = lookup_backend(&state, &params).await?;
    let studio = backend.open(parse_open_opts(&params)).await?;
    let session_guid = studio.session_guid.clone();
    let edit_dom_id = studio.edit_dom().dom_id.clone();
    let (handle, _arc) = insert_studio(&state, studio).await;
    Ok(json!({ "studioHandle": handle, "sessionGuid": session_guid, "editDomId": edit_dom_id }))
}

async fn studio_open_place(state: Arc<State>, params: Value) -> Result<Value> {
    let backend = lookup_backend(&state, &params).await?;
    let opts = parse_open_opts(&params);
    let place_id = params.get("placeId").and_then(|v| v.as_u64())
        .ok_or_else(|| anyhow!("placeId required"))?;
    let studio = backend.open_place(rodeo_client::studio::OpenPlaceOpts {
        place_id,
        fflags: opts.fflags,
        background: opts.background,
        profile: opts.profile,
        save: opts.save,
        detached: opts.detached,
        fflag_file: opts.fflag_file,
        show_widgets: opts.show_widgets,
        timeout: opts.timeout,
    }).await?;
    let session_guid = studio.session_guid.clone();
    let edit_dom_id = studio.edit_dom().dom_id.clone();
    let (handle, _arc) = insert_studio(&state, studio).await;
    Ok(json!({ "studioHandle": handle, "sessionGuid": session_guid, "editDomId": edit_dom_id }))
}

async fn studio_open_file(state: Arc<State>, params: Value) -> Result<Value> {
    let backend = lookup_backend(&state, &params).await?;
    let opts = parse_open_opts(&params);
    let path = params.get("path").and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("path required"))?
        .to_string();
    let studio = backend.open_file(rodeo_client::studio::OpenFileOpts {
        path,
        fflags: opts.fflags,
        background: opts.background,
        profile: opts.profile,
        save: opts.save,
        detached: opts.detached,
        fflag_file: opts.fflag_file,
        show_widgets: opts.show_widgets,
        timeout: opts.timeout,
    }).await?;
    let session_guid = studio.session_guid.clone();
    let edit_dom_id = studio.edit_dom().dom_id.clone();
    let (handle, _arc) = insert_studio(&state, studio).await;
    Ok(json!({ "studioHandle": handle, "sessionGuid": session_guid, "editDomId": edit_dom_id }))
}

async fn studio_set_mode(state: Arc<State>, params: Value) -> Result<Value> {
    let studio = lookup_studio(&state, &params).await?;
    let mode = params.get("mode").and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("mode required"))?
        .to_string();
    let timeout = parse_timeout(&params);
    let mut guard = studio.lock().await;
    guard.set_mode(&mode, timeout).await?;
    Ok(json!({
        "serverDomId": guard.server_dom.as_ref().map(|v| v.dom_id.clone()),
        "clientDomId": guard.client_dom.as_ref().map(|v| v.dom_id.clone()),
    }))
}

async fn studio_get_mode(state: Arc<State>, params: Value) -> Result<Value> {
    let studio = lookup_studio(&state, &params).await?;
    let mode = studio.lock().await.get_mode().await?;
    Ok(json!(mode))
}

async fn studio_save(state: Arc<State>, params: Value) -> Result<Value> {
    let studio = lookup_studio(&state, &params).await?;
    let resp = studio.lock().await.save().await?;
    Ok(serde_json::to_value(resp)?)
}

async fn studio_close(state: Arc<State>, params: Value) -> Result<Value> {
    let handle = params.get("studioHandle").and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("studioHandle required"))?
        .to_string();
    let studio = lookup_studio(&state, &params).await?;
    studio.lock().await.close().await?;
    state.studios.lock().await.remove(&handle);
    Ok(Value::Null)
}

async fn studio_get_doms(state: Arc<State>, params: Value) -> Result<Value> {
    let studio = lookup_studio(&state, &params).await?;
    let doms = studio.lock().await.get_doms().await?;
    Ok(serde_json::to_value(doms.iter().map(dom_snapshot).collect::<Vec<_>>())?)
}

async fn studio_start_multiplayer_test(state: Arc<State>, params: Value) -> Result<Value> {
    let studio = lookup_studio(&state, &params).await?;
    // Start the session with `numPlayers` clients UP FRONT (single
    // ExecuteMultiplayerTestAsync(numPlayers) call). Growing a *running* session
    // afterwards with StudioTestService:AddPlayers (mp.connectClient) crashes the
    // Studio server on some engine versions (observed on 0.726: SIGSEGV / null
    // deref on a worker thread the moment AddPlayers runs). Callers that know
    // their client count should request it here and read the returned
    // clientDomIds, rather than starting at 0 and growing via mp.connectClient.
    // Defaults to 0 for backwards compatibility.
    let num_players = params.get("numPlayers").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
    let timeout = parse_timeout(&params);
    let mp = studio.lock().await.start_multiplayer_test(num_players, timeout).await?;
    let server_dom_id = mp.server.dom_id.clone();
    let client_dom_ids: Vec<String> = mp.clients().iter().map(|v| v.dom_id.clone()).collect();
    let handle = state.mint_handle("mp");
    state.mp_tests.lock().await.insert(handle.clone(), Arc::new(Mutex::new(mp)));
    Ok(json!({ "mpHandle": handle, "serverDomId": server_dom_id, "clientDomIds": client_dom_ids }))
}

async fn lookup_mp(state: &State, params: &Value) -> Result<Arc<Mutex<MultiplayerTest>>> {
    let h = params.get("mpHandle").and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("mpHandle required"))?;
    state.mp_tests.lock().await.get(h).cloned()
        .ok_or_else(|| anyhow!("unknown mpHandle: {h}"))
}

async fn mp_connect_client(state: Arc<State>, params: Value) -> Result<Value> {
    let mp = lookup_mp(&state, &params).await?;
    let timeout = parse_timeout(&params);
    let client = mp.lock().await.connect_client(timeout).await?;
    Ok(json!({ "clientDomId": client.dom_id }))
}

async fn mp_disconnect_client(state: Arc<State>, params: Value) -> Result<Value> {
    let mp = lookup_mp(&state, &params).await?;
    let dom_id = params.get("domId").and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("domId required"))?
        .to_string();
    mp.lock().await.disconnect_client(&dom_id).await?;
    Ok(Value::Null)
}

async fn mp_close(state: Arc<State>, params: Value) -> Result<Value> {
    let handle = params.get("mpHandle").and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("mpHandle required"))?
        .to_string();
    let mp = lookup_mp(&state, &params).await?;
    mp.lock().await.close().await?;
    state.mp_tests.lock().await.remove(&handle);
    Ok(Value::Null)
}

// ---------------------------------------------------------------------------
// dom.* adapters
// ---------------------------------------------------------------------------

/// Read a RunCodeOpts from JSON-RPC params, including the routing fields
/// (mode/dom_kind/context). The DOM tier omits the routing fields at
/// the type level, so they arrive absent → None.
fn read_run_opts(params: &Value) -> RunCodeOpts {
    // Parse logFilter from params — missing fields default to "enabled" to
    // match the old TS client's behavior (all message types captured unless
    // explicitly disabled). Passing an unset field on the wire causes the
    // plugin to treat the filter as all-off, which silently hides print/warn.
    let log_filter = {
        let lf = params.get("logFilter").cloned().unwrap_or(Value::Object(Default::default()));
        fn get_bool(v: &Value, k: &str, default: bool) -> bool {
            v.get(k).and_then(|x| x.as_bool()).unwrap_or(default)
        }
        rodeo_proto::LogFilter {
            enable_warn: get_bool(&lf, "enableWarn", true),
            enable_error: get_bool(&lf, "enableError", true),
            enable_info: get_bool(&lf, "enableInfo", true),
            enable_output: get_bool(&lf, "enableOutput", true),
            enable_logs: get_bool(&lf, "enableLogs", true),
            ..Default::default()
        }
    };
    // Accept directory paths from the JSON-RPC caller (same-machine) and hand
    // them to rodeo-client, which writes profile/log files directly to disk.
    let profile_dir = params.get("profileDir").and_then(|v| v.as_str()).map(std::path::PathBuf::from);
    let str_opt = |k: &str| params.get(k).and_then(|v| v.as_str()).map(String::from);

    RunCodeOpts {
        source: params.get("source").and_then(|v| v.as_str()).unwrap_or("").to_string(),
        mode: str_opt("mode"),
        dom_kind: str_opt("domKind"),
        context: str_opt("context"),
        show_return: params.get("showReturn").and_then(|v| v.as_bool()).unwrap_or(false),
        reload_requires: params.get("reloadRequires").and_then(|v| v.as_bool()).unwrap_or(false),
        verbose: params.get("verbose").and_then(|v| v.as_bool()).unwrap_or(false),
        script_args: params.get("scriptArgs").and_then(|v| v.as_array())
            .map(|a| a.iter().filter_map(|s| s.as_str().map(String::from)).collect())
            .unwrap_or_default(),
        profile: profile_dir.is_some(),
        log_filter: Some(log_filter),
        instance_path: str_opt("instancePath"),
        script_path: str_opt("scriptPath"),
        return_file: str_opt("returnFile"),
        output_file: str_opt("outputFile"),
        profile_dir,
        session: None,
    }
}

async fn dom_run_code(state: Arc<State>, params: Value) -> Result<Value> {
    let dom_id = params.get("domId").and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("domId required"))?
        .to_string();
    let stream_id = require_stream_id(&params)?;
    let opts = read_run_opts(&params);
    let dom = state.client.get_dom(&dom_id).await?;
    let stream = dom.run_code_stream(opts).await?;
    pump_stream(state, stream_id, stream).await
}

async fn studio_run_code(state: Arc<State>, params: Value) -> Result<Value> {
    let studio = lookup_studio(&state, &params).await?;
    let stream_id = require_stream_id(&params)?;
    let opts = read_run_opts(&params);
    let stream = {
        let guard = studio.lock().await;
        guard.run_code_stream(opts).await?
    };
    pump_stream(state, stream_id, stream).await
}

async fn client_run_code(state: Arc<State>, params: Value) -> Result<Value> {
    let stream_id = require_stream_id(&params)?;
    let opts = read_run_opts(&params);
    let stream = state.client.submit_run_stream(opts).await?;
    pump_stream(state, stream_id, stream).await
}

/// Client-provided streamId (mandatory). Eliminates the race where a
/// server-minted ID could appear in a notification before the response
/// reaches the client: the caller allocates it and registers its callback
/// before sending, so any notification routes correctly.
fn require_stream_id(params: &Value) -> Result<String> {
    Ok(params.get("streamId").and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("streamId required (client-allocated)"))?
        .to_string())
}

/// Drive a RunStream, forwarding events to the JSON-RPC caller as
/// stream.data / stream.done / stream.error notifications. Registers a cancel
/// channel keyed by streamId. Shared by the dom / studio / client run tiers.
async fn pump_stream(
    state: Arc<State>,
    stream_id: String,
    mut stream: rodeo_client::run::RunStream,
) -> Result<Value> {
    let (cancel_tx, mut cancel_rx) = tokio::sync::oneshot::channel::<()>();
    state.streams.lock().await.insert(stream_id.clone(), cancel_tx);

    let state2 = state.clone();
    let sid = stream_id.clone();
    tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = &mut cancel_rx => {
                    emit_stream(&state2, "stream.error", json!({
                        "streamId": &sid,
                        "error": "canceled",
                    })).await;
                    break;
                }
                ev = stream.next() => {
                    let Some(ev) = ev else { break; };
                    match ev {
                        rodeo_client::run::RunStreamEvent::Created { .. } => {
                            // The run id also arrives on stream.done (from
                            // RunResult); no separate notification needed.
                        }
                        rodeo_client::run::RunStreamEvent::Output { kind, chunk } => {
                            let wire_kind = match kind {
                                rodeo_client::runtime::CapturedStreamKind::Stdout => "stdout",
                                rodeo_client::runtime::CapturedStreamKind::Stderr => "stderr",
                            };
                            emit_stream(&state2, "stream.data", json!({
                                "streamId": &sid, "kind": wire_kind, "chunk": chunk,
                            })).await;
                        }
                        rodeo_client::run::RunStreamEvent::FileChunk { .. } => {
                            // rodeo-client writes files directly to disk via
                            // profile_dir passed in opts. Nothing to forward to
                            // the JSON-RPC caller.
                        }
                        rodeo_client::run::RunStreamEvent::RpcCall { .. } => {
                            // Handled entirely inside the daemon. Not surfaced to
                            // the JSON-RPC client. (A `delegateRpcs` opt-in flag
                            // could expose these later if a wrapper wants them.)
                        }
                        rodeo_client::run::RunStreamEvent::Done { result } => {
                            emit_stream(&state2, "stream.done", json!({
                                "streamId": &sid,
                                "result": {
                                    "ok": result.ok,
                                    "exitCode": result.exit_code,
                                    "output": result.output,
                                    // Master-minted run id (from ProcessCreated).
                                    "executionId": result.execution_id,
                                    // JSON-encoded return value (as emitted by the
                                    // plugin runner). Carried through verbatim;
                                    // clients parse it as `result.return`.
                                    "returnValue": result.return_value,
                                    // Why the run failed, when it did.
                                    "error": result.error,
                                },
                            })).await;
                            break;
                        }
                    }
                }
            }
        }
        state2.streams.lock().await.remove(&sid);
    });

    // Response just acks the streamId. The TS wrapper doesn't use it for
    // routing — it already registered the callback — but returning it makes
    // the RPC semantics unambiguous for any future (non-TS) caller.
    Ok(json!({ "streamId": stream_id }))
}

async fn dom_cancel_run(state: Arc<State>, params: Value) -> Result<Value> {
    let stream_id = params.get("streamId").and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("streamId required"))?;
    if let Some(tx) = state.streams.lock().await.remove(stream_id) {
        let _ = tx.send(());
    }
    Ok(Value::Null)
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn dom_snapshot(v: &Dom) -> serde_json::Value {
    json!({
        "domId": v.dom_id,
        "backendId": v.backend_id,
        "mode": v.mode,
        "domKind": v.dom_kind,
        "sessionGuid": v.session_guid,
        "placeId": v.place_id,
        "gameName": v.game_name,
        "connected": v.connected,
        "activeRuns": v.active_runs,
        "userName": v.user_name,
        "userId": v.user_id,
    })
}

async fn write_line(state: &State, line: String) {
    let mut guard = state.stdout.lock().await;
    if guard.write_all(line.as_bytes()).await.is_err() { return; }
    let _ = guard.write_all(b"\n").await;
    let _ = guard.flush().await;
}

async fn write_response(state: &State, id: Value, result: Value) {
    let resp = JsonRpcResponse { jsonrpc: "2.0", id, result: Some(result), error: None };
    if let Ok(s) = serde_json::to_string(&resp) {
        write_line(state, s).await;
    }
}

async fn write_error(state: &State, id: Value, code: i32, message: &str, data: Option<Value>) {
    let resp = JsonRpcResponse {
        jsonrpc: "2.0",
        id,
        result: None,
        error: Some(JsonRpcErrorObj { code, message: message.to_string(), data }),
    };
    if let Ok(s) = serde_json::to_string(&resp) {
        write_line(state, s).await;
    }
}

async fn emit_stream(state: &State, method: &'static str, params: Value) {
    let n = JsonRpcNotification { jsonrpc: "2.0", method, params };
    if let Ok(s) = serde_json::to_string(&n) {
        write_line(state, s).await;
    }
}
