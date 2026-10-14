//! The runtime bridge (`--adapter`): the real Lune or Lute behind a script's
//! `@lune/*` / `@lute/*` requires.
//!
//! The first `adapter.call` for a runtime starts that runtime's bridge for the
//! run: its executable (runtimes.rs) runs rodeo-adapters/_bridge/server.luau,
//! which dials back over a localhost websocket. Each call's message goes out
//! as one frame, `[u32 id][message]`, and the frame with the same id comes
//! back as its reply. Messages are the bridge's own format (codec.luau); the
//! relay only pairs them up. The process's stdout and stderr are the run's,
//! so a child it starts with inherited stdio prints where the script does.
//!
//! A runtime gets its calls in the order the script made them: the run loop
//! queues each one as it arrives (`enqueue`), before the task that answers it
//! starts, since those tasks run in any order.

use super::{runtimes, CapturedOutputSender, CapturedStreamKind, SharedRpcState};
use futures::{SinkExt, StreamExt};
use rodeo_proto::runtime_types as rt;
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;
use tokio::io::AsyncReadExt;
use tokio::sync::{mpsc, oneshot, watch};
use tokio_tungstenite::tungstenite::{self, Message};

const SERVER: &str = include_str!("../../../rodeo-adapters/_bridge/server.luau");
const CODEC: &str = include_str!("../../../rodeo-adapters/_bridge/codec.luau");

/// How long a started runtime has to connect back.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);

/// How long a runtime has to exit once its connection closes, before it's
/// killed: one that exits by itself flushes the output it buffered (Lute's
/// print, into a pipe).
const EXIT_GRACE: Duration = Duration::from_secs(2);

type Reply = Result<Vec<u8>, String>;
type Call = (Vec<u8>, oneshot::Sender<Reply>);
type Socket = tokio_tungstenite::WebSocketStream<tokio::net::TcpStream>;

/// The run's bridges, each started by its runtime's first call. `stop` (or
/// dropping this) ends them: their processes are killed and the calls still
/// waiting fail.
pub struct Bridges {
    queues: Mutex<HashMap<&'static str, mpsc::UnboundedSender<Call>>>,
    /// Calls `enqueue` queued, by RPC id, for the task answering each.
    queued: Mutex<HashMap<String, oneshot::Receiver<Reply>>>,
    output: CapturedOutputSender,
    stop: watch::Sender<bool>,
}

impl Bridges {
    pub fn new(output: CapturedOutputSender) -> Self {
        Self {
            queues: Mutex::default(),
            queued: Mutex::default(),
            output,
            stop: watch::channel(false).0,
        }
    }

    /// Queues `call` for its runtime now, if it's an adapter call.
    pub fn enqueue(&self, call: &rt::ClientRpcCall) {
        if let Some(rt::client_rpc_call::Req::AdapterCall(req)) = &call.req {
            tracing::trace!(id = call.id.as_str(), runtime = req.runtime.as_str(), bytes = req.payload.len(), "adapter call queued");
            let replied = self.submit(&req.runtime, req.payload.clone());
            self.queued.lock().unwrap().insert(call.id.clone(), replied);
        }
    }

    fn submit(&self, runtime: &str, payload: Vec<u8>) -> oneshot::Receiver<Reply> {
        let (reply, replied) = oneshot::channel();
        let Some(runtime) = runtimes::by_name(runtime) else {
            let _ = reply.send(Err(format!("unknown runtime '{runtime}'")));
            return replied;
        };
        let mut queues = self.queues.lock().unwrap();
        let queue = queues.entry(runtime.name).or_insert_with(|| {
            let (queue, calls) = mpsc::unbounded_channel();
            tokio::spawn(run(runtime, calls, self.output.clone(), self.stop.subscribe()));
            queue
        });
        if let Err(mpsc::error::SendError((_, reply))) = queue.send((payload, reply)) {
            let _ = reply.send(Err(format!("the {} bridge has stopped", runtime.name)));
        }
        replied
    }

    pub fn stop(&self) {
        self.stop.send_replace(true);
    }
}

impl Drop for Bridges {
    fn drop(&mut self) {
        self.stop();
    }
}

pub async fn adapter_call(state: SharedRpcState, id: &str, req: &rt::AdapterCallRequest) -> Result<rt::AdapterCallResponse, String> {
    let bridges = state.lock().await.bridges.clone();
    let queued = bridges.queued.lock().unwrap().remove(id);
    tracing::trace!(id, was_queued = queued.is_some(), "adapter call waiting");
    let replied = queued.unwrap_or_else(|| bridges.submit(&req.runtime, req.payload.clone()));
    let payload = replied.await.map_err(|_| format!("the {} bridge has stopped", req.runtime))??;
    tracing::trace!(id, bytes = payload.len(), "adapter call answered");
    Ok(rt::AdapterCallResponse { payload, ..Default::default() })
}

/// One runtime's bridge for the run: starts it, then relays its calls.
async fn run(
    runtime: &'static runtimes::Runtime,
    mut calls: mpsc::UnboundedReceiver<Call>,
    output: CapturedOutputSender,
    mut stop: watch::Receiver<bool>,
) {
    // Dropping a start the run ended in the middle of kills what it started.
    let started = tokio::select! {
        started = start(runtime, output) => started,
        _ = stopped(&mut stop) => return,
    };
    match started {
        Ok(bridge) => relay(runtime.name, bridge, calls, stop).await,
        // Every call gets why the bridge isn't there.
        Err(e) => loop {
            tokio::select! {
                call = calls.recv() => match call {
                    Some((_, reply)) => { let _ = reply.send(Err(e.clone())); }
                    None => return,
                },
                _ = stopped(&mut stop) => return,
            }
        },
    }
}

/// A started bridge: its connection, process and server sources.
struct Started {
    socket: Socket,
    child: tokio::process::Child,
    // Lute resolves every require from the requiring script's file, so the
    // server's sources stay until the bridge ends.
    _dir: RemoveDir,
}

async fn start(runtime: &'static runtimes::Runtime, output: CapturedOutputSender) -> Result<Started, String> {
    let fail = |e: String| format!("starting the {} bridge: {e}", runtime.name);
    let exe = runtime.executable().await.map_err(fail)?;

    let dir = std::env::temp_dir().join(format!("rodeo-bridge-{}", uuid::Uuid::new_v4().simple()));
    let cleanup = RemoveDir(dir.clone());
    std::fs::create_dir_all(&dir).map_err(|e| fail(format!("creating {}: {e}", dir.display())))?;
    for (name, source) in [("server.luau", SERVER), ("codec.luau", CODEC)] {
        std::fs::write(dir.join(name), source).map_err(|e| fail(format!("writing {name}: {e}")))?;
    }

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.map_err(|e| fail(e.to_string()))?;
    let port = listener.local_addr().map_err(|e| fail(e.to_string()))?.port();
    let token = uuid::Uuid::new_v4().simple().to_string();

    let mut child = tokio::process::Command::new(&exe)
        .arg("run")
        .arg(dir.join("server.luau"))
        .env("RODEO_BRIDGE_URL", format!("ws://127.0.0.1:{port}/{token}"))
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| fail(format!("running {}: {e}", exe.display())))?;
    if let Some(stdout) = child.stdout.take() {
        tokio::spawn(forward(stdout, CapturedStreamKind::Stdout, output.clone()));
    }
    if let Some(stderr) = child.stderr.take() {
        tokio::spawn(forward(stderr, CapturedStreamKind::Stderr, output));
    }

    // Connected once the runtime says hello: a request sent before then can
    // arrive with the handshake, and Lute's client loses it.
    let connect = async {
        let mut socket = accept(&listener, &token).await?;
        hello(&mut socket).await?;
        Ok::<_, String>(socket)
    };
    let socket = tokio::select! {
        socket = connect => socket.map_err(fail)?,
        status = child.wait() => {
            let status = status.map(|s| s.to_string()).unwrap_or_else(|e| e.to_string());
            return Err(fail(format!("{} exited before connecting ({status})", exe.display())));
        }
        _ = tokio::time::sleep(CONNECT_TIMEOUT) => {
            return Err(fail(format!("{} didn't connect within {}s", exe.display(), CONNECT_TIMEOUT.as_secs())));
        }
    };
    tracing::debug!(runtime = runtime.name, exe = %exe.display(), "bridge connected");
    Ok(Started { socket, child, _dir: cleanup })
}

/// Accepts the runtime's connection: the first one asking for `/<token>`.
async fn accept(listener: &tokio::net::TcpListener, token: &str) -> Result<Socket, String> {
    use tungstenite::handshake::server::{ErrorResponse, Request, Response};
    let expected = format!("/{token}");
    loop {
        let (stream, _) = listener.accept().await.map_err(|e| e.to_string())?;
        let check = |request: &Request, response: Response| {
            if request.uri().path() == expected {
                Ok(response)
            } else {
                let mut refused = ErrorResponse::new(None);
                *refused.status_mut() = tungstenite::http::StatusCode::FORBIDDEN;
                Err(refused)
            }
        };
        // No size limits on this local connection: a reply too big to send
        // on fails its call at the RPC relay (guard_relay_size), where it
        // leaves the bridge up.
        let config = tungstenite::protocol::WebSocketConfig::default().max_message_size(None).max_frame_size(None);
        if let Ok(socket) = tokio_tungstenite::accept_hdr_async_with_config(stream, check, Some(config)).await {
            return Ok(socket);
        }
    }
}

/// Waits for the runtime's hello: a frame of id 0 alone.
async fn hello(socket: &mut Socket) -> Result<(), String> {
    loop {
        match socket.next().await {
            Some(Ok(Message::Binary(bytes))) if bytes[..] == [0, 0, 0, 0] => return Ok(()),
            Some(Ok(Message::Close(_))) | None => return Err("the runtime closed the connection".to_string()),
            Some(Err(e)) => return Err(e.to_string()),
            Some(Ok(_)) => continue,
        }
    }
}

/// Sends calls in order and pairs replies with them until the socket closes
/// or the run ends; then fails what's still waiting, closes the socket, and
/// kills the process if it doesn't exit within EXIT_GRACE.
async fn relay(
    runtime: &'static str,
    bridge: Started,
    mut calls: mpsc::UnboundedReceiver<Call>,
    mut stop: watch::Receiver<bool>,
) {
    let Started { mut socket, mut child, _dir } = bridge;
    let mut pending: HashMap<u32, oneshot::Sender<Reply>> = HashMap::new();
    let mut next_id: u32 = 0;
    let reason = loop {
        tokio::select! {
            call = calls.recv() => {
                let Some((payload, reply)) = call else { break "the run ended".to_string() };
                next_id = next_id.wrapping_add(1);
                let mut frame = Vec::with_capacity(4 + payload.len());
                frame.extend_from_slice(&next_id.to_le_bytes());
                frame.extend_from_slice(&payload);
                if let Err(e) = socket.send(Message::Binary(frame.into())).await {
                    let reason = format!("the {runtime} bridge's connection failed: {e}");
                    let _ = reply.send(Err(reason.clone()));
                    break reason;
                }
                tracing::trace!(runtime, frame = next_id, "bridge frame sent");
                pending.insert(next_id, reply);
            }
            message = socket.next() => {
                let frame = match message {
                    Some(Ok(Message::Binary(bytes))) => bytes.to_vec(),
                    Some(Ok(Message::Text(text))) => text.as_bytes().to_vec(),
                    Some(Ok(Message::Close(_))) | None => break format!("the {runtime} bridge exited"),
                    Some(Ok(_)) => continue,
                    Some(Err(e)) => break format!("the {runtime} bridge's connection failed: {e}"),
                };
                if frame.len() < 4 {
                    continue;
                }
                let id = u32::from_le_bytes([frame[0], frame[1], frame[2], frame[3]]);
                tracing::trace!(runtime, frame = id, known = pending.contains_key(&id), "bridge frame received");
                if let Some(reply) = pending.remove(&id) {
                    let _ = reply.send(Ok(frame[4..].to_vec()));
                }
            }
            _ = stopped(&mut stop) => break "the run ended".to_string(),
        }
    };
    tracing::debug!(runtime, reason, "bridge stopped");
    for (_, reply) in pending.drain() {
        let _ = reply.send(Err(reason.clone()));
    }
    calls.close();
    while let Ok((_, reply)) = calls.try_recv() {
        let _ = reply.send(Err(reason.clone()));
    }
    let _ = socket.close(None).await;
    let _ = tokio::time::timeout(EXIT_GRACE, child.wait()).await;
}

/// Resolves once the run's bridges are stopped (or `Bridges` is dropped).
async fn stopped(stop: &mut watch::Receiver<bool>) {
    let _ = stop.wait_for(|stopped| *stopped).await;
}

/// Copies one of the process's output pipes into the run's output.
async fn forward(mut pipe: impl tokio::io::AsyncRead + Unpin, kind: CapturedStreamKind, output: CapturedOutputSender) {
    let mut buf = vec![0u8; 8192];
    while let Ok(n) = pipe.read(&mut buf).await {
        if n == 0 || output.send((kind, buf[..n].to_vec())).is_err() {
            break;
        }
    }
}

struct RemoveDir(std::path::PathBuf);

impl Drop for RemoveDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
