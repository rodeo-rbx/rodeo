//! Studio backend mode: connects to master via connectrpc gRPC.
//!
//! Uses connectrpc-generated client stubs for typed RPC. Plugin DOMs are uplifted
//! to master via the Control bidirectional stream. File transfers (profile dumps)
//! use the SendFile client streaming RPC.

use anyhow::{Context, Result};
use rodeo_proto as proto;
use crate::master::SharedBackendState;
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::{info, warn};

/// Connect to master via connectrpc gRPC and register as a studio backend.
/// Returns (client, backend_id, master_id, bidi_stream) for the Control RPC.
/// `master_id` is master's bootstrap UUID — empty string if master is too old
/// to advertise it.
pub async fn connect_to_master(
    master_host: &str,
    master_port: u16,
    local_port: Option<u16>,
) -> Result<(
    proto::BackendServiceClient<connectrpc::client::HttpClient>,
    String,
    String,
    connectrpc::client::BidiStream<hyper::body::Incoming, proto::BackendMessage, proto::MasterMessageView<'static>>,
)> {
    let url = format!("http://{master_host}:{}", master_port);
    info!(url = url.as_str(), "connecting to master via connectrpc");

    let http = connectrpc::client::HttpClient::plaintext();
    let config = connectrpc::client::ClientConfig::new(url.parse().context("invalid master URL")?)
        .default_max_message_size(rodeo_proto::MAX_RPC_MESSAGE_SIZE);
    let client = proto::BackendServiceClient::new(http, config);

    let hostname = hostname::get()
        .map(|h| h.to_string_lossy().to_string())
        .unwrap_or_else(|_| "unknown".to_string());

    // Open Control bidi stream
    let mut bidi = client.control().await
        .map_err(|e| anyhow::anyhow!("control stream failed: {e}"))?;

    // Send Register as first message
    bidi.send(proto::BackendMessage {
        msg: Some(proto::backend_message::Msg::Register(Box::new(proto::RegisterRequest {
            kind: "studio".to_string(),
            name: hostname,
            port: local_port.map(|p| p as u32),
            version: proto::BUILD_ID.to_string(),
            ..Default::default()
        }))),
        ..Default::default()
    }).await.map_err(|e| anyhow::anyhow!("failed to send register: {e}"))?;

    // Read RegisterResponse (first MasterMessage)
    let first = bidi.message().await
        .map_err(|e| anyhow::anyhow!("expected RegisterResponse: {e}"))?
        .ok_or_else(|| anyhow::anyhow!(
            "stream closed before RegisterResponse (a master from another rodeo build rejects registration; its log names both versions)"
        ))?;
    let first_owned = first.to_owned_message();
    let (backend_id, master_id) = match first_owned.msg {
        Some(proto::master_message::Msg::Registered(r)) => (r.id, r.master_id),
        _ => anyhow::bail!("expected RegisterResponse as first message"),
    };

    info!(
        backend_id = &backend_id[..8.min(backend_id.len())],
        master_id = &master_id[..8.min(master_id.len())],
        "registered with master",
    );

    Ok((client, backend_id, master_id, bidi))
}

/// Run the master connection loop via connectrpc Control bidi stream.
pub async fn run_master_loop(
    client: proto::BackendServiceClient<connectrpc::client::HttpClient>,
    _backend_id: String,
    mut bidi: connectrpc::client::BidiStream<hyper::body::Incoming, proto::BackendMessage, proto::MasterMessageView<'static>>,
    state: SharedBackendState,
) {
    // Relay channel: plugin_ws sends proto::BackendMessage, forwarded to bidi stream
    let (relay_tx, mut relay_rx) = mpsc::unbounded_channel::<proto::BackendMessage>();

    // Spawn task to forward relay messages to bidi stream
    let (outgoing_tx, mut outgoing_rx) = mpsc::unbounded_channel::<proto::BackendMessage>();
    let relay_out_tx = outgoing_tx.clone();
    tokio::spawn(async move {
        while let Some(msg) = relay_rx.recv().await {
            if relay_out_tx.send(msg).is_err() { break; }
        }
    });

    {
        let mut guard = state.lock().await;
        guard.relay_tx = Some(relay_tx.clone());
    }

    // Send dom_connect for already-connected DOMs
    {
        let guard = state.lock().await;
        for (dom_id, dom) in &guard.doms {
            if dom.connected {
                let state_json = serde_json::to_string(&dom.state).unwrap_or_default();
                let _ = bidi.send(proto::BackendMessage {
                    msg: Some(proto::backend_message::Msg::DomConnect(Box::new(proto::DomConnect {
                        dom_id: dom_id.clone(), state_json,
                        ..Default::default()
                    }))),
                    ..Default::default()
                }).await;
            }
        }
    }

    // State snapshots: periodic (2s) + event-driven via snapshot_trigger
    {
        let snapshot_state = state.clone();
        let snapshot_tx = outgoing_tx.clone();
        let trigger = Arc::new(tokio::sync::Notify::new());
        {
            let mut guard = state.lock().await;
            guard.snapshot_trigger = Some(trigger.clone());
        }
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = tokio::time::sleep(std::time::Duration::from_secs(2)) => {}
                    _ = trigger.notified() => {}
                }
                let snapshot = build_state_snapshot(&snapshot_state).await;
                let msg = proto::BackendMessage {
                    msg: Some(proto::backend_message::Msg::StateSnapshot(Box::new(snapshot))),
                    ..Default::default()
                };
                if snapshot_tx.send(msg).is_err() { break; }
            }
        });
    }

    // Spawn task to send outgoing messages on bidi stream
    // We need a separate task because bidi.send() requires &mut bidi
    // but we also need to receive from bidi concurrently.
    // Use a channel-based approach instead.
    let loop_state = state.clone();

    loop {
        tokio::select! {
            // Forward outgoing messages to bidi stream
            msg = outgoing_rx.recv() => {
                match msg {
                    Some(m) => {
                        if !guard_relay_size(&m, &loop_state).await { continue; }
                        if bidi.send(m).await.is_err() { break; }
                    }
                    None => break,
                }
            }
            // Receive incoming master messages
            result = bidi.message() => {
                match result {
                    Ok(Some(view)) => {
                        let master_msg = view.to_owned_message();
                        if let Some(msg) = master_msg.msg {
                            handle_master_msg(msg, &client, &outgoing_tx, &loop_state).await;
                        }
                    }
                    _ => break,
                }
            }
        }
    }

    { let mut guard = state.lock().await; guard.relay_tx = None; }
    info!("master connection closed");
}

/// Covers connect framing and the DomPluginMessage wrapper around a relayed
/// message, so a payload sized exactly at the cap still fits.
const RELAY_GUARD_MARGIN: usize = 4096;

/// Refuse to relay any single BackendMessage that would exceed the master's
/// connectrpc envelope cap. The receive side can't skip an oversized message —
/// it drops the whole control stream, killing every Studio and run on this
/// backend (issue #7's failure mode). Rejecting at the sender scopes the
/// failure to the one message: a plugin RPC gets an error response back (the
/// run's rpc.call raises, actionable); anything else is dropped with a
/// warning. Returns whether the message may be sent.
async fn guard_relay_size(m: &proto::BackendMessage, state: &SharedBackendState) -> bool {
    use buffa::Message;
    let size = m.compute_size() as usize;
    if size + RELAY_GUARD_MARGIN <= rodeo_proto::MAX_RPC_MESSAGE_SIZE {
        return true;
    }

    if let Some(proto::backend_message::Msg::DomPluginMessage(ref dpm)) = m.msg {
        if let Some(proto::plugin_message::Msg::Rpc(ref call)) =
            dpm.message.as_option().and_then(|pm| pm.msg.as_ref())
        {
            warn!(
                rpc_id = call.id.as_str(),
                execution_id = call.execution_id.as_str(),
                size,
                limit = rodeo_proto::MAX_RPC_MESSAGE_SIZE,
                "refusing to relay oversized rpc; erroring the call instead of killing the control stream"
            );
            let guard = state.lock().await;
            if let Some(dom) = guard.doms.get(&dpm.dom_id) {
                let server_msg = proto::ServerMessage {
                    msg: Some(proto::server_message::Msg::RpcResponse(Box::new(
                        rodeo_proto::runtime_types::ClientRpcResponse {
                            id: call.id.clone(),
                            execution_id: call.execution_id.clone(),
                            res: Some(rodeo_proto::runtime_types::client_rpc_response::Res::Error(format!(
                                "rpc message ({size} bytes) exceeds the relay envelope limit ({} bytes); send the data in smaller chunks",
                                rodeo_proto::MAX_RPC_MESSAGE_SIZE,
                            ))),
                            ..Default::default()
                        },
                    ))),
                    ..Default::default()
                };
                let _ = dom.studio_tx.send(serde_json::to_string(&server_msg).unwrap_or_default());
            }
            return false;
        }
    }

    warn!(
        size,
        limit = rodeo_proto::MAX_RPC_MESSAGE_SIZE,
        "dropping oversized backend message instead of killing the control stream"
    );
    false
}

/// Mark a launching session as failed. Drives the lifecycle transition
/// through the snapshot — sets the instance row to status="error" rather
/// than removing it, because the master's launch_studio watcher only
/// resolves on a terminal status; absence would leave it (and the run
/// client blocked on it) hanging forever. SessionExited is still emitted
/// so the master reconciles pending runs scoped to the dead session.
///
/// Only a launch that hasn't connected yet is failed, checked under the same
/// lock as the transition so a plugin connecting at that moment wins. Returns
/// whether it failed the launch.
async fn fail_launch(
    state: &SharedBackendState,
    outgoing_tx: &mpsc::UnboundedSender<proto::BackendMessage>,
    session_guid: &str,
    reason: String,
) -> bool {
    {
        let mut guard = state.lock().await;
        let Some(inst) = guard.studio_instances.get_mut(session_guid) else { return false };
        if inst.status != "launching" && inst.status != "pending" {
            return false;
        }
        inst.status = "error".to_string();
        inst.error = Some(reason.clone());
        if let Some(ref notify) = guard.snapshot_trigger { notify.notify_one(); }
    }
    let _ = outgoing_tx.send(proto::BackendMessage {
        msg: Some(proto::backend_message::Msg::SessionExited(
            Box::new(proto::SessionExited {
                session_guid: session_guid.to_string(),
                reason,
                ..Default::default()
            })
        )),
        ..Default::default()
    });
    true
}

/// How long a launched Studio has to connect its plugin before the launch
/// fails: `RODEO_LAUNCH_TIMEOUT` seconds, default 120. `0` means no limit.
fn launch_timeout() -> Option<std::time::Duration> {
    const DEFAULT_SECS: u64 = 120;
    let secs = match std::env::var("RODEO_LAUNCH_TIMEOUT") {
        Ok(v) => v.trim().parse::<u64>().unwrap_or_else(|_| {
            warn!("RODEO_LAUNCH_TIMEOUT={v:?} is not a whole number of seconds; using {DEFAULT_SECS}");
            DEFAULT_SECS
        }),
        Err(_) => DEFAULT_SECS,
    };
    (secs > 0).then(|| std::time::Duration::from_secs(secs))
}

/// Sign-in state from the log of the Studio launched for `session_guid`
/// (matched by its RunScript bootstrap path). `None` if the log isn't found.
async fn launched_studio_sign_in(
    session_guid: &str,
    since: std::time::SystemTime,
) -> Option<rbx_control::studio::log::SignIn> {
    let marker = format!("rodeo-bootstrap-{session_guid}");
    tokio::task::spawn_blocking(move || {
        let dir = rbx_control::paths::roblox_logs_dir()?;
        let log = std::fs::read_to_string(rbx_control::studio::log::find_log(&dir, &marker, since)?).ok()?;
        Some(rbx_control::studio::log::sign_in_state(&log))
    })
    .await
    .ok()
    .flatten()
}

async fn handle_master_msg(
    msg: proto::master_message::Msg,
    client: &proto::BackendServiceClient<connectrpc::client::HttpClient>,
    outgoing_tx: &mpsc::UnboundedSender<proto::BackendMessage>,
    state: &SharedBackendState,
) {
    match msg {
        proto::master_message::Msg::DomServerMessage(dom_server) => {
            let dom_id = dom_server.dom_id.clone();
            let server_msg = match dom_server.message.into_option() {
                Some(m) => m,
                None => return,
            };

            // If profiled or logged run, wire up file collection.
            // Match directly on the typed oneof — no JSON parsing.
            if let Some(proto::server_message::Msg::Run(ref run_cmd)) = server_msg.msg {
                let execution_id = run_cmd.execution_id.clone();

                // Profile dumps: register scanner and stream via gRPC.
                // Scanner is keyed by execution_id (label embedded in dump file).
                if run_cmd.profile == Some(true) {
                    let guard = state.lock().await;
                    let session_guid = guard.doms.get(&dom_id)
                        .and_then(|dom| dom.session_guid.clone())
                        .unwrap_or_default();
                    if let Some(ref scanner) = guard.profile_scanner {
                        let mut dump_rx = scanner.register(execution_id.clone());
                        let eid = execution_id.clone();
                        let sg = session_guid.clone();
                        let dump_client = client.clone();
                        let files_done_tx = outgoing_tx.clone();
                        tokio::spawn(async move {
                            while let Some(dump) = dump_rx.recv().await {
                                // Flat, self-describing filename:
                                //   profile-<execution_id>-<session_guid>-<ts>-<raw>
                                // raw carries the scanner's original name (which includes
                                // its own sequence / sample marker).
                                let ts = crate::util::log_capture::filename_timestamp();
                                let filename = if sg.is_empty() {
                                    format!("profile-{}-{}-{}", &eid, ts, &dump.filename)
                                } else {
                                    format!("profile-{}-{}-{}-{}", &eid, &sg, ts, &dump.filename)
                                };
                                let chunks = stream_file_chunks(&filename, &eid, &dump.data);
                                if let Err(e) = dump_client.send_file(chunks).await {
                                    tracing::warn!("profile send failed: {e}");
                                } else {
                                    tracing::debug!(filename = filename.as_str(), size = dump.data.len(), "profile: sent via gRPC");
                                }
                            }
                            // Scanner unregistered, dump_rx closed — all files sent
                            let _ = files_done_tx.send(proto::BackendMessage {
                                msg: Some(proto::backend_message::Msg::FilesComplete(Box::new(proto::FilesComplete {
                                    execution_id: eid,
                                    ..Default::default()
                                }))),
                                ..Default::default()
                            });
                        });
                        tracing::debug!(execution_id, "profile: registered run on backend scanner");
                    }
                    drop(guard);
                }
            }

            // Serialize the typed ServerMessage once and forward to the DOM's plugin
            // over the WebSocket (which still speaks JSON-encoded proto).
            let kind = match &server_msg.msg {
                Some(proto::server_message::Msg::Welcome(_)) => "welcome",
                Some(proto::server_message::Msg::Run(_)) => "run",
                Some(proto::server_message::Msg::Kill(_)) => "kill",
                Some(proto::server_message::Msg::RpcResponse(_)) => "rpc_response",
                Some(proto::server_message::Msg::SetTargetMode(_)) => "set_target_mode",
                Some(proto::server_message::Msg::ScriptChunk(_)) => "script_chunk",
                None => "empty",
            };
            let dom_short = &dom_id[..8.min(dom_id.len())];
            let json = serde_json::to_string(&server_msg).unwrap();
            let guard = state.lock().await;
            if let Some(dom_conn) = guard.doms.get(&dom_id) {
                // A mode transition makes Studio activate itself when the
                // session starts (and again when it ends), stealing keyboard
                // focus from whatever the user is in — on any display. Arm
                // the focus guard on that Studio so the activation is handed
                // straight back; every transition re-arms it.
                if matches!(server_msg.msg, Some(proto::server_message::Msg::SetTargetMode(_))) {
                    let studio = dom_conn
                        .session_guid
                        .as_ref()
                        .and_then(|sg| guard.studio_instances.get(sg))
                        .and_then(|inst| inst.studio.clone());
                    if let Some(studio) = studio {
                        tokio::task::spawn_blocking(move || {
                            studio.guard_focus(std::time::Duration::from_secs(60))
                        });
                    }
                }
                match dom_conn.studio_tx.send(json) {
                    Ok(_) => tracing::debug!(dom = dom_short, kind, "backend → plugin: forwarded"),
                    Err(e) => tracing::warn!(dom = dom_short, kind, "backend → plugin: channel closed: {e}"),
                }
            } else {
                tracing::warn!(dom = dom_short, kind, "backend → plugin: dom not found in backend state");
            }
        }
        proto::master_message::Msg::Save(cmd) => {
            // Resolve which Studio to save, then fire Cmd+S + wait for mtime
            // + reply with SaveResult. Routing key is request_id; session_guid
            // is payload (may be absent — fall back to the only connected
            // Studio when there's exactly one; error otherwise).
            let request_id = cmd.request_id.clone();
            let caller_sg = cmd.session_guid.clone();
            let resolution = {
                let guard = state.lock().await;
                match caller_sg.as_deref().filter(|s| !s.is_empty()) {
                    Some(sg) => {
                        match guard.studio_instances.get(sg)
                            .filter(|inst| inst.status == "connected")
                            .and_then(|inst| inst.studio.clone())
                        {
                            Some(studio) => Ok((sg.to_string(), studio)),
                            None => Err(format!("no connected Studio for session_guid={sg}")),
                        }
                    }
                    None => {
                        // Fallback: only connected Studio wins.
                        let connected: Vec<_> = guard.studio_instances.iter()
                            .filter(|(_, inst)| inst.status == "connected")
                            .filter_map(|(sg, inst)| inst.studio.as_ref().map(|s| (sg.clone(), s.clone())))
                            .collect();
                        match connected.len() {
                            1 => Ok(connected.into_iter().next().unwrap()),
                            0 => Err("no connected Studio".to_string()),
                            n => Err(format!(
                                "{n} connected Studios; specify session_guid to disambiguate"
                            )),
                        }
                    }
                }
            };

            let outgoing = outgoing_tx.clone();
            let request_id_for_err = request_id.clone();
            let (session_guid, studio) = match resolution {
                Ok(r) => r,
                Err(err) => {
                    tracing::warn!("save: {err}");
                    let _ = outgoing.send(proto::BackendMessage {
                        msg: Some(proto::backend_message::Msg::SaveResult(Box::new(proto::SaveResult {
                            request_id: request_id_for_err,
                            session_guid: caller_sg,
                            saved: false,
                            path: None,
                            error: Some(err),
                            ..Default::default()
                        }))),
                        ..Default::default()
                    });
                    return;
                }
            };
            let sg_short: String = session_guid[..8.min(session_guid.len())].to_string();

            tokio::spawn(async move {
                let place_path = studio.place_path().map(|p| p.to_path_buf());
                let mtime_before = place_path.as_ref()
                    .and_then(|p| std::fs::metadata(p).ok())
                    .and_then(|m| m.modified().ok());
                tracing::info!(
                    session_guid = sg_short.as_str(),
                    place = ?place_path.as_ref().map(|p| p.display().to_string()),
                    "save: triggering studio.save()"
                );
                // Confirm the save by watching the place file's mtime, re-firing
                // the Ctrl+S keystroke until it lands. On Windows a concurrent or
                // background keystroke save can silently drop the chord (foreground
                // is a single system-wide resource — see launch-control's
                // send_keystroke), so a single fire is unreliable; retry until the
                // mtime changes or we hit the overall deadline.
                let (saved, path, error) = match (place_path, mtime_before) {
                    (Some(path), Some(before)) => {
                        // 60s: generous because a save attempt against a busy /
                        // still-settling Studio can stall ~25s on its first AX
                        // or focus contact (worse under suite load); the
                        // pre-warm usually absorbs that, but the budget must
                        // tolerate the un-warmed worst case plus retries.
                        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
                        let mut changed = false;
                        let mut ticks = 0u32;
                        let mut attempts = 0u32;
                        let mut last_err: Option<String> = None;
                        'retry: while std::time::Instant::now() < deadline {
                            attempts += 1;
                            if let Err(e) = studio.save() {
                                last_err = Some(e.to_string());
                                tracing::warn!(session_guid = sg_short.as_str(), attempts, "save: keystroke attempt failed: {e}");
                            }
                            // Give this attempt up to ~6s to land, bounded by the
                            // overall deadline, before re-firing.
                            let attempt_deadline = (std::time::Instant::now()
                                + std::time::Duration::from_secs(6))
                                .min(deadline);
                            while std::time::Instant::now() < attempt_deadline {
                                tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                                ticks += 1;
                                if let Ok(meta) = std::fs::metadata(&path) {
                                    if let Ok(now) = meta.modified() {
                                        if now != before { changed = true; break 'retry; }
                                    }
                                }
                            }
                        }
                        if changed {
                            tracing::info!(session_guid = sg_short.as_str(), ticks, attempts, "save: mtime changed, success");
                            (true, Some(path.to_string_lossy().into_owned()), None)
                        } else {
                            tracing::warn!(session_guid = sg_short.as_str(), ticks, attempts, "save: timed out waiting for mtime change");
                            (false, Some(path.to_string_lossy().into_owned()),
                             last_err.or_else(|| Some("mtime did not change within 60s".to_string())))
                        }
                    }
                    // No place path / no baseline mtime — can't confirm via mtime.
                    // Fire once and report the keystroke's own result.
                    (maybe_path, _) => match studio.save() {
                        Ok(()) => (true, maybe_path.map(|p| p.to_string_lossy().into_owned()), None),
                        Err(e) => {
                            tracing::error!(session_guid = sg_short.as_str(), "save: studio.save() failed: {e}");
                            (false, maybe_path.map(|p| p.to_string_lossy().into_owned()), Some(e.to_string()))
                        }
                    },
                };
                let _ = outgoing.send(proto::BackendMessage {
                    msg: Some(proto::backend_message::Msg::SaveResult(Box::new(proto::SaveResult {
                        request_id,
                        session_guid: Some(session_guid),
                        saved,
                        path,
                        error,
                        ..Default::default()
                    }))),
                    ..Default::default()
                });
            });
        }
        proto::master_message::Msg::RunCompleted(rc) => {
            let execution_id = rc.execution_id.clone();
            let guard = state.lock().await;
            // Profile scanner: unregister by execution_id.
            if let Some(ref scanner) = guard.profile_scanner {
                scanner.unregister(&execution_id);
                tracing::debug!(execution_id, "backend: unregistered scanner on RunCompleted");
            }
        }
        proto::master_message::Msg::LaunchStudio(cmd) => {
            let cmd = *cmd;
            let ls = state.clone();
            let out_tx = outgoing_tx.clone();
            let session_guid = cmd.session_guid.clone();
            {
                let mut guard = ls.lock().await;
                guard.studio_instances.insert(session_guid.clone(), crate::master::StudioInstanceInfo {
                    session_guid: session_guid.clone(),
                    status: "pending".to_string(),
                    studio: None,
                    error: None,
                    mcp_studio_id: None,
                });
                if let Some(ref notify) = guard.snapshot_trigger {
                    notify.notify_one();
                }
            }
            tokio::spawn(async move {
                use crate::studio_backend::{Studio, StudioOptions, FflagConfig, PlaceTarget};
                let target = if let Some(file) = cmd.place_file {
                    PlaceTarget::File(file)
                } else if cmd.place_id.unwrap_or(0) > 0 {
                    PlaceTarget::PlaceId { place_id: cmd.place_id.unwrap(), universe_id: None }
                } else {
                    PlaceTarget::Empty
                };
                let guard = ls.lock().await;
                let port = guard.port;
                drop(guard);
                let mut fflags = FflagConfig { overrides: cmd.fflags, file: cmd.fflag_file };
                if cmd.profile {
                    fflags = crate::studio_backend::launch::inject_profile_fflags(fflags);
                }
                let opts = StudioOptions {
                    port,
                    background: cmd.background,
                    save: crate::studio_backend::parse_save_mode(cmd.save_path),
                    fflags,
                    detached: cmd.detached,
                    show_widgets: if cmd.show_widgets.is_empty() { None } else { Some(cmd.show_widgets.clone()) },
                    session_guid: session_guid.clone(),
                };
                // Transition pending → launching
                {
                    let mut guard = ls.lock().await;
                    if let Some(inst) = guard.studio_instances.get_mut(&session_guid) {
                        inst.status = "launching".to_string();
                    }
                    if let Some(ref notify) = guard.snapshot_trigger {
                        notify.notify_one();
                    }
                }
                let sg_short = &session_guid[..8.min(session_guid.len())];
                tracing::info!(session_guid = sg_short, "Launching Studio on port {port}...");

                // Side-channel: snapshot StudioMCP's Studio list before spawn so we can
                // identify the new mcp_studio_id by diffing after spawn. Skipped if MCP
                // isn't initialized — launch-order correlation remains as fallback.
                tracing::info!(session_guid = sg_short, "launch: taking pre-spawn MCP snapshot");
                let pre_ids: std::collections::HashSet<String> = {
                    let mcp_arc = { ls.lock().await.mcp.clone() };
                    let mut mcp_guard = mcp_arc.lock().await;
                    match mcp_guard.as_mut() {
                        Some(client) => match client.list_studios().await {
                            Ok(entries) => entries.into_iter().map(|e| e.mcp_studio_id).collect(),
                            Err(e) => {
                                tracing::debug!(error = %e, "list_roblox_studios pre-snapshot failed; side-channel disabled for this launch");
                                std::collections::HashSet::new()
                            }
                        },
                        None => std::collections::HashSet::new(),
                    }
                };
                tracing::info!(session_guid = sg_short, pre_count = pre_ids.len(), "launch: pre-spawn MCP snapshot done, entering spawn_blocking");

                // Spawn Studio (blocking). Handle is available immediately;
                // readiness is signaled by the plugin's WS connect.
                let launch_started = std::time::SystemTime::now();
                let spawn_result = tokio::task::spawn_blocking(move || Studio::spawn(target, opts)).await;
                tracing::info!(session_guid = sg_short, ok = spawn_result.is_ok(), "launch: spawn_blocking returned");
                let instance = match spawn_result {
                    Ok(Ok(inst)) => std::sync::Arc::new(inst),
                    Ok(Err(e)) => {
                        tracing::error!("Studio spawn failed: {e}");
                        fail_launch(&ls, &out_tx, &session_guid, format!("launch_failed: {e}")).await;
                        return;
                    }
                    Err(e) => {
                        tracing::error!("Studio spawn task failed: {e}");
                        fail_launch(&ls, &out_tx, &session_guid, format!("launch_failed: {e}")).await;
                        return;
                    }
                };

                // Store handle IMMEDIATELY — SIGTERM cleanup can now kill Studio
                {
                    let mut guard = ls.lock().await;
                    if let Some(inst) = guard.studio_instances.get_mut(&session_guid) {
                        inst.studio = Some(instance.clone());
                    }
                }

                // Side-channel: poll list_roblox_studios for the new mcp_studio_id.
                // First new entry not already claimed by another studio_instance is
                // attributed here. Timeout after 30s; launch-order correlation
                // remains as fallback.
                {
                    let poll_state = ls.clone();
                    let poll_session_guid = session_guid.clone();
                    tokio::spawn(async move {
                        let start = std::time::Instant::now();
                        let timeout = std::time::Duration::from_secs(30);
                        let interval = std::time::Duration::from_millis(200);
                        loop {
                            tokio::time::sleep(interval).await;
                            if start.elapsed() > timeout {
                                tracing::debug!(session_guid = %poll_session_guid, "mcp_studio_id side-channel timed out");
                                return;
                            }
                            let mcp_arc = { poll_state.lock().await.mcp.clone() };
                            let entries_opt = {
                                let mut mcp_guard = mcp_arc.lock().await;
                                match mcp_guard.as_mut() {
                                    Some(client) => client.list_studios().await.ok(),
                                    None => None,
                                }
                            };
                            let Some(entries) = entries_opt else { continue };
                            let mut guard = poll_state.lock().await;
                            // Already claimed? (e.g. a reconnect raced us)
                            if guard.studio_instances.get(&poll_session_guid).and_then(|i| i.mcp_studio_id.clone()).is_some() {
                                return;
                            }
                            let claimed: std::collections::HashSet<String> = guard.studio_instances.values()
                                .filter_map(|i| i.mcp_studio_id.clone())
                                .collect();
                            let picked = entries.into_iter()
                                .find(|e| !pre_ids.contains(&e.mcp_studio_id) && !claimed.contains(&e.mcp_studio_id));
                            let Some(entry) = picked else { continue };
                            if let Some(inst) = guard.studio_instances.get_mut(&poll_session_guid) {
                                tracing::info!(session_guid = %poll_session_guid, mcp_studio_id = %entry.mcp_studio_id, "paired session with mcp_studio_id via side-channel");
                                inst.mcp_studio_id = Some(entry.mcp_studio_id);
                                if let Some(ref notify) = guard.snapshot_trigger { notify.notify_one(); }
                            }
                            return;
                        }
                    });
                }

                // Pre-warm Studio's accessibility connection in the background
                // so the first save's AX menu press doesn't absorb the ~25s
                // first-contact settle inside the save budget. Holds only a
                // Weak ref between probes: the Studio owns the daemon
                // SlotHandle and process handle, and pinning the Arc for the
                // warm window delays teardown and exhausts launch slots
                // (observed: processCleanup pid lingering + Studio opens
                // queueing behind phantom slots).
                {
                    let warm_weak = std::sync::Arc::downgrade(&instance);
                    let warm_sg = sg_short.to_string();
                    tokio::spawn(async move {
                        let started = std::time::Instant::now();
                        let deadline = started + std::time::Duration::from_secs(60);
                        while std::time::Instant::now() < deadline {
                            let Some(studio) = warm_weak.upgrade() else { return };
                            let done = tokio::task::spawn_blocking(move || studio.warm_save_menu_once())
                                .await
                                .unwrap_or(true);
                            if done {
                                tracing::debug!(
                                    session_guid = warm_sg.as_str(),
                                    elapsed_ms = started.elapsed().as_millis() as u64,
                                    "save menu pre-warm: ready",
                                );
                                return;
                            }
                            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                        }
                        tracing::debug!(session_guid = warm_sg.as_str(), "save menu pre-warm: gave up after 60s");
                    });
                }

                // Launch deadline (#13). Studio shows its sign-in prompt before
                // it loads plugins, so a signed-out Studio never connects and
                // the launch, with every run pinned to it, would wait forever.
                // Past the deadline, fail the launch and close the Studio. Holds
                // only a Weak ref, like the pre-warm above.
                if let Some(timeout) = launch_timeout() {
                    let deadline_state = ls.clone();
                    let deadline_out_tx = out_tx.clone();
                    let deadline_session = session_guid.clone();
                    let deadline_studio = std::sync::Arc::downgrade(&instance);
                    tokio::spawn(async move {
                        use rbx_control::studio::log::SignIn;
                        tokio::time::sleep(timeout).await;
                        let launching = deadline_state.lock().await.studio_instances
                            .get(&deadline_session)
                            .is_some_and(|i| i.status == "launching");
                        if !launching {
                            return;
                        }
                        // Let a sign-in that is under way finish first: killing
                        // Studio mid sign-in can leave the account signed out.
                        let mut sign_in = launched_studio_sign_in(&deadline_session, launch_started).await;
                        let sign_in_wait = std::time::Instant::now() + std::time::Duration::from_secs(15);
                        while sign_in == Some(SignIn::InProgress) && std::time::Instant::now() < sign_in_wait {
                            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                            sign_in = launched_studio_sign_in(&deadline_session, launch_started).await;
                        }
                        let secs = timeout.as_secs();
                        let session = &deadline_session[..8.min(deadline_session.len())];
                        let reason = if sign_in == Some(SignIn::Failed) {
                            format!("launch_failed: Studio (session {session}) did not connect within {secs}s: it is waiting for you to sign in to Roblox (its saved sign-in expired). Sign in to Roblox Studio, then run again")
                        } else {
                            format!("launch_failed: Studio (session {session}) did not connect within {secs}s: the rodeo plugin never loaded in it. Studio may be showing a dialog or still loading the place; set RODEO_LAUNCH_TIMEOUT to allow longer")
                        };
                        if !fail_launch(&deadline_state, &deadline_out_tx, &deadline_session, reason.clone()).await {
                            return;
                        }
                        warn!(session_guid = %deadline_session, "{reason}");
                        let Some(studio) = deadline_studio.upgrade() else { return };
                        // Never save: the place may not have loaded.
                        tokio::task::spawn_blocking(move || {
                            studio.mark_saved();
                            studio.cleanup();
                            studio.kill();
                        })
                        .await
                        .ok();
                    });
                }

                // Event-driven exit handler — replaces the old 2-second polling
                // monitor. `Child::on_exit` fires via OS-level wait/kqueue, so we
                // learn about death immediately. The callback synthesizes a
                // SessionExited message; master handles per-DOM run cleanup via
                // the existing DomDisconnect path (the OS closes the plugin's
                // WebSocket when the process dies → WS reader fires DomDisconnect).
                let exit_state = ls.clone();
                let exit_session_guid = session_guid.clone();
                let exit_out_tx = out_tx.clone();
                let exit_instance = instance.clone();
                instance.on_exit(move |_status| {
                    // Determine reason: was status still "launching" when we died?
                    // (i.e. plugin never connected) — call it launch_failed so
                    // the master's launch_studio stream gets a meaningful Error.
                    let session_guid = exit_session_guid.clone();
                    let exit_state = exit_state.clone();
                    let exit_out_tx = exit_out_tx.clone();
                    let exit_instance = exit_instance.clone();
                    tokio::spawn(async move {
                        let status = {
                            let guard = exit_state.lock().await;
                            guard.studio_instances.get(&session_guid).map(|i| i.status.clone())
                        };
                        // A launch the deadline already failed has reported its
                        // reason and SessionExited; keep that reason.
                        if status.as_deref() == Some("error") {
                            exit_instance.cleanup();
                            return;
                        }
                        let was_launching = matches!(status.as_deref(), Some("launching" | "pending"));
                        let reason = if was_launching {
                            "launch_failed: Studio process exited during launch".to_string()
                        } else {
                            "exited".to_string()
                        };
                        tracing::info!(session_guid = %session_guid, reason = %reason, "Studio process exited (on_exit)");
                        exit_instance.cleanup();
                        let _ = exit_out_tx.send(proto::BackendMessage {
                            msg: Some(proto::backend_message::Msg::SessionExited(
                                Box::new(proto::SessionExited {
                                    session_guid: session_guid.clone(),
                                    reason: reason.clone(),
                                    ..Default::default()
                                })
                            )),
                            ..Default::default()
                        });
                        // Drive the lifecycle transition through the snapshot —
                        // this is the same surface plugin-connect mutates via
                        // try_claim_session_from_handshake. Removing the row
                        // would lose the terminal state and leave any
                        // launch_studio watcher hanging (Studio crashes during
                        // plugin load → no Ready, no Error, just absence).
                        let mut guard = exit_state.lock().await;
                        if let Some(inst) = guard.studio_instances.get_mut(&session_guid) {
                            inst.status = "error".to_string();
                            inst.error = Some(reason);
                        }
                        if let Some(ref notify) = guard.snapshot_trigger {
                            notify.notify_one();
                        }
                    });
                });
            });
        }
        proto::master_message::Msg::CloseStudio(cmd) => {
            let session_guid = cmd.session_guid.clone();
            let skip_save = cmd.skip_save == Some(true);
            let studio_to_cleanup = {
                let mut guard = state.lock().await;
                let trigger = guard.snapshot_trigger.clone();
                let studio = guard.studio_instances.get_mut(&session_guid).and_then(|inst| {
                    inst.status = "closing".to_string();
                    inst.studio.clone()
                });

                // Eagerly prune DOMs belonging to the closing Studio so master's routing
                // view excludes them before the SIGTERM actually lands. Without this, the
                // window between kill-issue and WebSocket EOF allows new runs to match and
                // dispatch to a plugin that's about to die (and never reply).
                let dying: Vec<String> = guard.doms.iter()
                    .filter(|(_, dom)| dom.session_guid.as_deref() == Some(session_guid.as_str()))
                    .map(|(id, _)| id.clone())
                    .collect();
                for dom_id in &dying {
                    if let Some(dom) = guard.doms.get_mut(dom_id) {
                        dom.disconnect();
                    }
                    guard.doms.remove(dom_id);
                    if let Some(ref relay_tx) = guard.relay_tx {
                        let _ = relay_tx.send(proto::BackendMessage {
                            msg: Some(proto::backend_message::Msg::DomDisconnect(Box::new(proto::DomDisconnect {
                                dom_id: dom_id.clone(),
                                ..Default::default()
                            }))),
                            ..Default::default()
                        });
                    }
                }
                if !dying.is_empty() {
                    tracing::info!(session_guid = session_guid.as_str(), count = dying.len(), "pruned doms for closing studio");
                }

                if let Some(ref notify) = trigger {
                    notify.notify_one();
                }
                studio
            };
            if let Some(studio) = studio_to_cleanup {
                tracing::info!(session_guid = session_guid.as_str(), "closing Studio");
                // Run full cleanup off the async runtime — save() can block up
                // to 30s waiting for file mtime change. cleanup() skips the
                // kill for --detach studios (detach = survive serve exit), but
                // an explicit CloseStudio means the user wants the process
                // gone regardless, so kill unconditionally after (no-op when
                // cleanup already killed).
                let ls = state.clone();
                tokio::spawn(async move {
                    tokio::task::spawn_blocking(move || {
                        if skip_save {
                            studio.mark_saved();
                        }
                        studio.cleanup();
                        studio.kill();
                    }).await.ok();
                    let mut guard = ls.lock().await;
                    guard.studio_instances.remove(&session_guid);
                    if let Some(ref notify) = guard.snapshot_trigger {
                        notify.notify_one();
                    }
                });
            }
        }
        _ => {}
    }
}

/// Break file data into FileChunk messages for gRPC streaming.
pub fn stream_file_chunks(filename: &str, execution_id: &str, data: &[u8]) -> Vec<proto::FileChunk> {
    const CHUNK_SIZE: usize = 64 * 1024;
    let mut chunks = Vec::new();
    for (i, chunk) in data.chunks(CHUNK_SIZE).enumerate() {
        let is_last = (i + 1) * CHUNK_SIZE >= data.len();
        chunks.push(proto::FileChunk {
            filename: filename.to_string(),
            execution_id: execution_id.to_string(),
            data: chunk.to_vec(),
            is_last,
            ..Default::default()
        });
    }
    if chunks.is_empty() {
        chunks.push(proto::FileChunk {
            filename: filename.to_string(),
            execution_id: execution_id.to_string(),
            data: vec![],
            is_last: true,
            ..Default::default()
        });
    }
    chunks
}

async fn build_state_snapshot(state: &SharedBackendState) -> proto::StateSnapshot {
    let guard = state.lock().await;
    let doms = guard.doms.iter().map(|(dom_id, dom)| proto::DomSnapshot {
        dom_id: dom_id.clone(),
        mode: dom.state.as_ref().map(|s| s.mode.clone()),
        dom_kind: dom.state.as_ref().map(|s| s.dom_kind.clone()),
        session_guid: dom.session_guid.clone(),
        // Canonical studio id from the plugin (empty until first state msg).
        studio_id: dom.state.as_ref()
            .map(|s| s.studio_id.clone())
            .filter(|s| !s.is_empty()),
        place_id: dom.state.as_ref().map(|s| s.place_id),
        game_name: dom.state.as_ref().map(|s| s.game_name.clone()),
        active_runs: dom.active_count() as u32,
        connected: dom.connected,
        // Player identity for client DOMs, so master can populate
        // StudioDom.user_name / user_id.
        user_name: dom.state.as_ref()
            .and_then(|s| s.client_info.clone().into_option())
            .map(|ci| ci.name),
        user_id: dom.state.as_ref()
            .and_then(|s| s.client_info.clone().into_option())
            .map(|ci| ci.user_id),
        ..Default::default()
    }).collect();
    let studios = guard.studios.iter().map(|(_, s)| proto::StudioSnapshot {
        session_guid: s.session_guid.clone(), mode: s.mode.clone(), name: s.name.clone(), place_id: s.place_id,
        ..Default::default()
    }).collect();
    let studio_instances = guard.studio_instances.values().map(|inst| proto::StudioInstanceState {
        session_guid: inst.session_guid.clone(),
        status: inst.status.clone(),
        error: inst.error.clone(),
        source_path: inst.studio.as_ref().and_then(|s| s.source_path().map(str::to_string)),
        working_path: inst.studio.as_ref().and_then(|s| s.working_path().map(str::to_string)),
        ..Default::default()
    }).collect();
    proto::StateSnapshot { doms, studios, studio_instances, ..Default::default() }
}

