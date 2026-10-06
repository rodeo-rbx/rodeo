use super::connection;
use crate::master::SharedBackendState;
use rodeo_proto as proto;
use proto::ProcessState;
use crate::runtime::mcp as mcp_service;
use futures_util::{SinkExt, StreamExt};
use hyper_tungstenite::tungstenite::Message;
use serde_json::Value;
use tokio::sync::mpsc;
use tracing::{info, debug, Instrument};

/// Stamp `session_guid` on the given DOM from the plugin's self-reported
/// handshake value. The plugin has `flags.SESSION_GUID` baked in at install
/// time and forwards it on its first `StudioStateMsg`. Master looks up the
/// matching `StudioInstanceInfo` (keyed by session_guid), stamps the DOM
/// synchronously, and transitions the instance to "connected". Returns
/// true if the session was stamped.
fn try_claim_session_from_handshake(guard: &mut crate::master::BackendState, dom_id: &str) -> bool {
    let already_stamped = guard.doms.get(dom_id)
        .map(|dom| dom.session_guid.is_some())
        .unwrap_or(false);
    if already_stamped { return false; }
    let Some(reported) = guard.doms.get(dom_id)
        .and_then(|dom| dom.state.as_ref())
        .and_then(|s| s.session_guid.clone())
    else { return false; };

    // Plugin self-reports an id it baked in at install time. Trust it iff
    // we actually spawned something with that id (studio_instances).
    let matched = guard.studio_instances.contains_key(&reported);
    if !matched { return false; }

    if let Some(inst) = guard.studio_instances.get_mut(&reported) {
        if inst.status == "launching" {
            inst.status = "connected".to_string();
        }
    }
    if let Some(dom) = guard.doms.get_mut(dom_id) {
        dom.session_guid = Some(reported);
    }
    if let Some(ref notify) = guard.snapshot_trigger {
        notify.notify_one();
    }
    true
}

/// Shared handler for plugin-reported run termination (Done or Killed).
/// Both outcomes follow the same shape: check whether the run is local to
/// this backend; if so, forward the typed event to the run client and
/// complete the run locally; otherwise, relay the PluginMessage upstream.
async fn handle_run_finished(
    state: &SharedBackendState,
    dom_id: &str,
    eid: &str,
    new_state: ProcessState,
    pm_for_relay: proto::PluginMessage,
    forward: impl FnOnce(&connection::DomConnection, &str),
) {
    let mut guard = state.lock().await;
    let is_local = guard.doms.get(dom_id)
        .map(|dom| dom.active_runs.contains_key(eid))
        .unwrap_or(false);

    if is_local {
        if let Some(dom) = guard.doms.get(dom_id) {
            forward(dom, eid);
        }
        guard.complete_run(eid, dom_id, new_state);
    } else {
        let relay_tx = guard.relay_tx.clone();
        drop(guard);
        if let Some(tx) = relay_tx {
            let _ = tx.send(wrap_dom_plugin_message(dom_id, pm_for_relay));
        }
    }
}

/// Wrap a typed plugin message as a proto BackendMessage::DomPluginMessage for relay to master.
fn wrap_dom_plugin_message(dom_id: &str, message: proto::PluginMessage) -> proto::BackendMessage {
    proto::BackendMessage {
        msg: Some(proto::backend_message::Msg::DomPluginMessage(Box::new(proto::DomPluginMessage {
            dom_id: dom_id.to_string(),
            message: buffa::MessageField::some(message),
            ..Default::default()
        }))),
        ..Default::default()
    }
}

/// Send a typed ClientRpcResponse to the plugin, JSON-encoded for the websocket.
fn send_rpc_response(
    studio_tx: &mpsc::UnboundedSender<String>,
    resp: proto::runtime_types::ClientRpcResponse,
) {
    let server_msg = proto::ServerMessage {
        msg: Some(proto::server_message::Msg::RpcResponse(Box::new(resp))),
        ..Default::default()
    };
    let _ = studio_tx.send(serde_json::to_string(&server_msg).unwrap());
}

/// Build an error ClientRpcResponse for a given call id.
fn error_response(eid: &str, rpc_id: &str, message: String) -> proto::runtime_types::ClientRpcResponse {
    proto::runtime_types::ClientRpcResponse {
        id: rpc_id.to_string(),
        execution_id: eid.to_string(),
        res: Some(proto::runtime_types::client_rpc_response::Res::Error(message)),
        ..Default::default()
    }
}

/// The error an elevated run gets when its Studio never shows up in
/// StudioMCP's Studio list (the reconciliation loop never resolves the DOM's
/// `mcp_studio_id`). The runner prints it as the run's error, so it names the
/// Studio, for a user with several open, and what connects one: Studio's
/// Assistant opens its MCP socket only once its panel has been opened.
fn no_studio_mcp_message(dom_state: Option<&proto::StudioStateMsg>) -> String {
    let studio = match dom_state {
        Some(s) if !s.game_name.is_empty() => format!("Studio {} ({})", s.studio_id, s.game_name),
        Some(s) if !s.studio_id.is_empty() => format!("Studio {}", s.studio_id),
        _ => "This Studio".to_string(),
    };
    format!(
        "{studio} has no StudioMCP connection (waited 10 s), and --context elevated runs \
         through StudioMCP. Open the AI Assistant panel in that Studio with MCP Server \
         enabled in its settings, or use --context cmdbar (edit DOM of a Studio rodeo launched)."
    )
}

/// First server→plugin message. Carries our build id so the plugin can
/// display a mismatch (the backend enforces it, see the gate below).
fn welcome_msg() -> proto::ServerMessage {
    proto::ServerMessage {
        msg: Some(proto::server_message::Msg::Welcome(Box::new(proto::WelcomeMsg {
            version: proto::BUILD_ID.to_string(),
            ..Default::default()
        }))),
        ..Default::default()
    }
}

/// Handle a studio (plugin) client connection.
/// The first message is the initial studio_state (parsed by caller).
pub async fn handle_studio_client<S, R>(
    ws_tx: &mut S,
    ws_rx: &mut R,
    state: SharedBackendState,
    first_msg: &Value,
) where
    S: SinkExt<Message> + Unpin,
    R: StreamExt<Item = Result<Message, hyper_tungstenite::tungstenite::Error>> + Unpin,
{
    let (studio_tx, mut studio_rx) = mpsc::unbounded_channel::<String>();

    // Parse initial state from the first message (proto format: { "studioState": { ... } })
    let initial_state: Option<proto::StudioStateMsg> = first_msg.get("studioState")
        .and_then(|s| serde_json::from_value(s.clone()).ok());

    let dom_id = initial_state.as_ref()
        .map(|s| s.dom_id.clone())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

    let game_name = initial_state.as_ref()
        .map(|s| s.game_name.clone())
        .unwrap_or_default();
    let place_id = initial_state.as_ref()
        .map(|s| s.place_id)
        .unwrap_or(0);

    let dom_id_short = dom_id[..8.min(dom_id.len())].to_string();
    let dom_kind = initial_state.as_ref().map(|s| if s.dom_kind.is_empty() { "?" } else { s.dom_kind.as_str() }).unwrap_or("?").to_string();
    let span = tracing::info_span!("studio", id = dom_id_short.as_str(), dom_kind = dom_kind.as_str());

    async move {

    let initial_mode = initial_state.as_ref().map(|s| if s.mode.is_empty() { "?" } else { s.mode.as_str() }).unwrap_or("?");
    let initial_mcp_studio_id = initial_state.as_ref().and_then(|s| s.mcp_studio_id.as_deref()).unwrap_or("none");
    info!(
        universe = game_name.as_str(),
        place_id,
        mode = initial_mode,
        mcp_studio_id = &initial_mcp_studio_id[..8.min(initial_mcp_studio_id.len())],
        "dom connected"
    );

    // Version gate. The plugin reports its build id on the first message; a
    // DOM running another build is never registered or routed to, since its
    // RunCommand/RPC shapes may not match ours. The welcome goes out so the
    // plugin can show the mismatch in its widget — and then the socket is
    // parked open rather than closed. A plugin whose socket closes reconnects
    // at once (pre-1.5 plugins with no back-off, immediately in play DOMs, and
    // re-running their per-connection setup each time), which against a
    // backend that will never accept it is a hot loop. Held open, it sits
    // quietly and reconnects only when this backend exits — which is exactly
    // when the port can change hands to a backend that does match.
    let plugin_version = initial_state.as_ref().map(|s| s.plugin_version.as_str()).unwrap_or("");
    if let Err(msg) = proto::check_peer_version("this Studio's rodeo plugin", plugin_version) {
        if proto::version_check_skipped() {
            tracing::warn!("{msg} ({} set, accepting)", proto::SKIP_VERSION_CHECK_ENV);
        } else {
            tracing::warn!("{msg}; parking this connection unregistered (the plugin shows the mismatch and reconnects when this backend exits)");
            let _ = ws_tx.send(Message::Text(serde_json::to_string(&welcome_msg()).unwrap().into())).await;
            while let Some(Ok(msg)) = ws_rx.next().await {
                if matches!(msg, Message::Close(_)) {
                    break;
                }
            }
            return;
        }
    }

    // Register DOM
    {
        let mut guard = state.lock().await;

        if let Some(old_dom) = guard.doms.get_mut(&dom_id) {
            if old_dom.connected {
                old_dom.disconnect();
            }
        }

        let mut conn = connection::DomConnection::new(dom_id.clone(), studio_tx.clone());
        if let Some(ref s) = initial_state {
            conn.update_state(s.clone());
        }
        guard.doms.insert(dom_id.clone(), conn);
        guard.process_pending();

        // Identity pairing: the plugin reports its baked `session_guid` on the
        // first message (see flags.SESSION_GUID). Match it to a known
        // StudioInstanceInfo and stamp synchronously — no polling, no races.
        //
        // Fallback: if the plugin didn't send one (e.g. an older build or a
        // manually-installed plugin that wasn't spawned by rodeo), leave the
        // DOM un-stamped. It still connects; it just isn't scoped to a session
        // for routing. Published-place launches where the plugin IS rodeo-
        // installed always send the guid, so this covers only manual installs.
        let _ = try_claim_session_from_handshake(&mut *guard, &dom_id);

        // A DOM connecting is a moment Studio activates itself: the edit DOM
        // as Studio comes up, a server/client DOM as a session starts. Arm
        // the focus guard on its Studio so that activation is handed back.
        let studio = guard.doms.get(&dom_id)
            .and_then(|d| d.session_guid.as_ref())
            .and_then(|sg| guard.studio_instances.get(sg))
            .and_then(|inst| inst.studio.clone());
        if let Some(studio) = studio {
            tokio::task::spawn_blocking(move || studio.guard_focus(std::time::Duration::from_secs(60)));
        }

        // Relay dom_connect to master
        if let Some(ref relay_tx) = guard.relay_tx {
            let state_json = initial_state.as_ref()
                .map(|s| serde_json::to_string(s).unwrap_or_default())
                .unwrap_or_default();
            let _ = relay_tx.send(proto::BackendMessage {
                msg: Some(proto::backend_message::Msg::DomConnect(Box::new(proto::DomConnect {
                    dom_id: dom_id.clone(),
                    state_json,
                    ..Default::default()
                }))),
                ..Default::default()
            });
        }

        // Send welcome
        let _ = ws_tx.send(Message::Text(serde_json::to_string(&welcome_msg()).unwrap().into())).await;
    }

    // Message loop
    loop {
        tokio::select! {
            // Server → plugin (run commands, rpc responses)
            msg = studio_rx.recv() => {
                match msg {
                    Some(text) => {
                        if ws_tx.send(Message::Text(text.into())).await.is_err() {
                            break;
                        }
                    }
                    None => break,
                }
            }
            // Plugin → server — plugin only sends typed PluginMessage variants post-migration.
            msg = ws_rx.next() => {
                match msg {
                    Some(Ok(Message::Text(text))) => {
                        match serde_json::from_str::<proto::PluginMessage>(&text) {
                            Ok(pm) if pm.msg.is_some() => {
                                handle_plugin_message(pm, &dom_id, &state, &studio_tx).await;
                            }
                            Ok(_) => {
                                debug!("ignoring PluginMessage with empty msg oneof");
                            }
                            Err(e) => {
                                tracing::warn!(error = %e, "failed to decode plugin message");
                            }
                        }
                    }
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Err(e)) => {
                        // A read error (e.g. a message over the size cap, or a
                        // protocol/UTF-8 violation) poisons the stream — break
                        // so the normal disconnect cleanup runs instead of
                        // silently polling a dead connection while the
                        // in-flight run waits forever.
                        tracing::warn!(error = %e, "plugin WS read error; disconnecting dom");
                        break;
                    }
                    _ => {}
                }
            }
        }
    }

    // Disconnect
    {
        let guard = state.lock().await;
        let mode = guard.doms.get(&dom_id).and_then(|dom| dom.mode()).unwrap_or("?");
        let dom_kind = guard.doms.get(&dom_id).and_then(|dom| dom.dom_kind()).unwrap_or("?");
        info!(mode, dom_kind, "dom disconnected");
    }

    let mut guard = state.lock().await;
    if let Some(ref relay_tx) = guard.relay_tx {
        let _ = relay_tx.send(proto::BackendMessage {
            msg: Some(proto::backend_message::Msg::DomDisconnect(Box::new(proto::DomDisconnect {
                dom_id: dom_id.clone(),
                ..Default::default()
            }))),
            ..Default::default()
        });
    }
    if let Some(dom) = guard.doms.get_mut(&dom_id) {
        dom.disconnect();
    }
    guard.doms.remove(&dom_id);
    if let Some(ref notify) = guard.snapshot_trigger {
        notify.notify_one();
    }
    guard.process_pending();

    }.instrument(span).await
}

/// Handle a proto PluginMessage from the plugin.
async fn handle_plugin_message(
    plugin_msg: proto::PluginMessage,
    dom_id: &str,
    state: &SharedBackendState,
    studio_tx: &mpsc::UnboundedSender<String>,
) {
    // Clone for relay before destructuring (move-semantics of the oneof makes the original pm unusable).
    let pm_for_relay = plugin_msg.clone();
    let msg = match plugin_msg.msg {
        Some(m) => m,
        None => return,
    };

    match msg {
        proto::plugin_message::Msg::StudioState(ss) => {
            let mut guard = state.lock().await;
            if let Some(dom) = guard.doms.get_mut(dom_id) {
                if let Some(diff) = dom.update_state(*ss.clone()) {
                    info!(changes = diff.as_str(), "state changed");
                }
            }

            // Subsequent-update path: if the DOM wasn't stamped at handshake
            // (older plugin, or the session_guid field arrived in a later
            // update), retry the handshake claim here.
            try_claim_session_from_handshake(&mut *guard, dom_id);

            guard.process_pending();

            if let Some(ref relay_tx) = guard.relay_tx {
                let _ = relay_tx.send(wrap_dom_plugin_message(dom_id, pm_for_relay));
            }
        }
        proto::plugin_message::Msg::Rpc(call) => {
            // Plugin now sends typed ClientRpcCall directly — no method/params_json translation.
            let eid = call.execution_id.clone();
            let rpc_id = call.id.clone();

            // mcp.call is server-context; dispatch server-side and return a typed response.
            // All other variants forward to the run-client.
            match call.req {
                Some(proto::runtime_types::client_rpc_call::Req::McpCall(_)) => {
                    debug!(method = "mcp.call", eid = &eid[..8.min(eid.len())], "rpc (server)");
                    let state_clone = state.clone();
                    let studio_tx_clone = studio_tx.clone();
                    let dom_id = dom_id.to_string();
                    let call_owned = *call;
                    tokio::spawn(async move {
                        let response = dispatch_mcp(&state_clone, &dom_id, call_owned).await;
                        send_rpc_response(&studio_tx_clone, response);
                    });
                }
                _ => {
                    debug!(eid = &eid[..8.min(eid.len())], "rpc (client)");
                    let guard = state.lock().await;
                    let is_local = guard.doms.get(dom_id)
                        .map(|dom| dom.active_runs.contains_key(eid.as_str()))
                        .unwrap_or(false);

                    if is_local {
                        if let Some(dom) = guard.doms.get(dom_id) {
                            dom.forward_rpc_call(&eid, *call);
                        }
                    } else if let Some(ref relay_tx) = guard.relay_tx {
                        // Multi-node relay: forward the typed PluginMessage.
                        let _ = relay_tx.send(wrap_dom_plugin_message(dom_id, pm_for_relay));
                    } else {
                        tracing::warn!(eid = &eid[..8.min(eid.len())], rpc_id = &rpc_id[..8.min(rpc_id.len())], dom_id, "rpc call with no active run and no relay — dropping");
                    }
                }
            }
        }
        proto::plugin_message::Msg::Done(done) => {
            let eid = done.execution_id.clone();
            let new_state = if done.success { ProcessState::PROCESS_STATE_DONE } else { ProcessState::PROCESS_STATE_ERROR };
            let done_owned = *done;
            handle_run_finished(
                state, dom_id, &eid, new_state, pm_for_relay,
                move |dom, eid| { dom.forward_execution_done(eid, done_owned); },
            ).await;
        }
        proto::plugin_message::Msg::Killed(killed) => {
            let eid = killed.execution_id.clone();
            let killed_owned = *killed;
            handle_run_finished(
                state, dom_id, &eid, ProcessState::PROCESS_STATE_KILLED, pm_for_relay,
                move |dom, eid| { dom.forward_execution_killed(eid, killed_owned); },
            ).await;
        }
    }
}

/// Dispatch a server-context RPC (currently: `mcp.call`). Returns a typed ClientRpcResponse.
async fn dispatch_mcp(
    state: &SharedBackendState,
    dom_id: &str,
    call: proto::runtime_types::ClientRpcCall,
) -> proto::runtime_types::ClientRpcResponse {
    use proto::runtime_types::client_rpc_call::Req;
    use proto::runtime_types::client_rpc_response::Res;

    let eid = call.execution_id.clone();
    let rpc_id = call.id.clone();

    let mcp_req = match call.req {
        Some(Req::McpCall(r)) => r,
        _ => return error_response(&eid, &rpc_id, "dispatch_mcp expects McpCallRequest".into()),
    };

    let mcp_arc = { state.lock().await.mcp.clone() };

    // Wait for MCP unification (mcp_studio_id resolved) up to ~10 seconds.
    let mcp_sid = {
        let guard = state.lock().await;
        guard.doms.get(dom_id).and_then(|dom| dom.state.as_ref()).and_then(|s| s.mcp_studio_id.clone())
    };
    let mcp_sid = match mcp_sid {
        Some(sid) => sid,
        None => {
            debug!("waiting for MCP unification");
            let mut attempts = 0;
            loop {
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                attempts += 1;
                let guard = state.lock().await;
                if let Some(sid) = guard.doms.get(dom_id).and_then(|dom| dom.state.as_ref()).and_then(|s| s.mcp_studio_id.clone()) {
                    break sid;
                }
                if attempts >= 20 {
                    let dom_state = guard.doms.get(dom_id).and_then(|dom| dom.state.as_ref());
                    return error_response(&eid, &rpc_id, no_studio_mcp_message(dom_state));
                }
            }
        }
    };

    let arguments: Value = serde_json::from_str(&mcp_req.arguments_json).unwrap_or(Value::Object(Default::default()));
    match mcp_service::handle_mcp_call(mcp_arc, &mcp_sid, &mcp_req.tool, &arguments).await {
        Ok(text) => proto::runtime_types::ClientRpcResponse {
            id: rpc_id,
            execution_id: eid,
            res: Some(Res::McpCall(Box::new(proto::runtime_types::McpCallResponse { result: text, ..Default::default() }))),
            ..Default::default()
        },
        Err(e) => error_response(&eid, &rpc_id, e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_studio_mcp_message_names_the_studio_and_the_setting() {
        let state = proto::StudioStateMsg {
            studio_id: "3f9a2c".into(),
            game_name: "Baseplate".into(),
            ..Default::default()
        };
        let msg = no_studio_mcp_message(Some(&state));
        assert!(msg.starts_with("Studio 3f9a2c (Baseplate) has no StudioMCP connection"), "{msg}");
        assert!(msg.contains("--context elevated"), "{msg}");
        assert!(msg.contains("Open the AI Assistant panel in that Studio with MCP Server enabled"), "{msg}");

        let unnamed = proto::StudioStateMsg { studio_id: "3f9a2c".into(), ..Default::default() };
        assert!(no_studio_mcp_message(Some(&unnamed)).starts_with("Studio 3f9a2c has no"));
        assert!(no_studio_mcp_message(None).starts_with("This Studio has no"));
    }
}
