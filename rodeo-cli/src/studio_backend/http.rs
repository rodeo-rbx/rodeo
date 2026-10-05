//! HTTP/WS server for studio backend.
//!
//! Accepts plugin connections (studio_state) and provides /health and /save endpoints.
//! This is separate from master's http.rs which handles CLI clients, backends, etc.

use super::plugin_ws::handle_studio_client;
use crate::master::{BackendState, SharedBackendState};
use rodeo_proto as proto;
use bytes::Bytes;
use http_body_util::Full;
use hyper::body::Incoming;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use serde::Serialize;
use std::convert::Infallible;
use tokio::net::TcpStream;

/// Handle an incoming TCP connection on the studio backend.
pub async fn handle_connection(stream: TcpStream, state: SharedBackendState) {
    let io = TokioIo::new(stream);
    let state_clone = state.clone();

    let service = service_fn(move |req: Request<Incoming>| {
        let state = state_clone.clone();
        async move { handle_request(req, state).await }
    });

    if let Err(e) = http1::Builder::new()
        .serve_connection(io, service)
        .with_upgrades()
        .await
    {
        if !e.is_incomplete_message() {
            tracing::debug!("studio backend connection error: {e}");
        }
    }
}

/// Route: WS upgrade for plugins, or plain HTTP for health/save.
async fn handle_request(
    mut req: Request<Incoming>,
    state: SharedBackendState,
) -> Result<Response<Full<Bytes>>, Infallible> {
    if hyper_tungstenite::is_upgrade_request(&req) {
        // Browsers send Origin on every WebSocket handshake and WebSockets
        // aren't covered by CORS, so any web page could otherwise open a
        // plugin connection to this localhost port, pose as a DOM and receive
        // runs. Studio's WebStreamClient sends no Origin.
        if let Some(origin) = req.headers().get(hyper::header::ORIGIN) {
            tracing::warn!("refused WebSocket upgrade from browser origin {origin:?}");
            return text_response(StatusCode::FORBIDDEN, "Forbidden");
        }
        match hyper_tungstenite::upgrade(&mut req, None) {
            Ok((response, ws_future)) => {
                tokio::spawn(async move {
                    match ws_future.await {
                        Ok(ws_stream) => {
                            handle_websocket(ws_stream, state).await;
                        }
                        Err(e) => {
                            tracing::debug!("WebSocket upgrade error: {e}");
                        }
                    }
                });
                Ok(response)
            }
            Err(_) => text_response(StatusCode::BAD_REQUEST, "WebSocket upgrade failed"),
        }
    } else {
        let method = req.method().clone();
        let path = req.uri().path().to_string();

        match (method, path.as_str()) {
            (hyper::Method::GET, "/health") => {
                let guard = state.lock().await;
                let health = build_health_response(&guard);
                json_response(StatusCode::OK, &health)
            }
            // /save is gone — save flows through master's typed control-stream
            // message `MasterMessage::Save(SaveCommand)` with a routed
            // `BackendMessage::SaveResult` reply.
            _ => text_response(StatusCode::NOT_FOUND, "Not found"),
        }
    }
}

/// Handle WebSocket — studio backend only accepts plugin connections.
async fn handle_websocket(ws_stream: hyper_tungstenite::HyperWebsocketStream, state: SharedBackendState) {
    use futures_util::StreamExt;
    use hyper_tungstenite::tungstenite::Message;

    let (mut ws_tx, mut ws_rx) = ws_stream.split();

    let first_msg = match ws_rx.next().await {
        Some(Ok(Message::Text(text))) => text,
        _ => return,
    };

    let first: serde_json::Value = match serde_json::from_str(&first_msg) {
        Ok(v) => v,
        Err(_) => return,
    };

    // Proto format: { "studioState": {...} }
    // Legacy format: { "type": "studio_state", "state": {...} }
    let is_studio = first.get("studioState").is_some()
        || first["type"].as_str() == Some("studio_state");

    if is_studio {
        handle_studio_client(&mut ws_tx, &mut ws_rx, state, &first).await;
    } else {
        let msg_type = first["type"].as_str().unwrap_or("?");
        tracing::warn!("Unexpected connection type on studio backend: {msg_type}");
    }
}

fn build_health_response(state: &BackendState) -> proto::HealthResponse {
    let mut doms = Vec::new();

    for (dom_id, dom) in &state.doms {
        doms.push(proto::DomInfo {
            rodeo_id: dom_id.clone(),
            active_count: dom.active_count() as u32,
            is_idle: dom.connected && dom.active_runs.is_empty(),
            ..Default::default()
        });
    }

    let total_doms = doms.len() as u32;
    let total_queued = state.pending_runs.len() as u32;

    proto::HealthResponse {
        launched: !state.doms.is_empty(),
        context_count: 0,
        total_doms,
        total_queued,
        contexts: vec![proto::ContextInfo {
            bitset: 0,
            dom_count: total_doms,
            total_queued,
            doms,
            ..Default::default()
        }],
        ..Default::default()
    }
}

fn json_response(
    status: StatusCode,
    body: &impl Serialize,
) -> Result<Response<Full<Bytes>>, Infallible> {
    let body = serde_json::to_string(body).unwrap_or_default();
    Ok(Response::builder()
        .status(status)
        .header("Content-Type", "application/json")
        .body(Full::new(Bytes::from(body)))
        .unwrap())
}

fn text_response(
    status: StatusCode,
    body: &str,
) -> Result<Response<Full<Bytes>>, Infallible> {
    Ok(Response::builder()
        .status(status)
        .header("Content-Type", "text/plain")
        .body(Full::new(Bytes::from(body.to_string())))
        .unwrap())
}
