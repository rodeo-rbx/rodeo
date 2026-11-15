//! `rodeo run --ree`: a run executed by ree (roblox-engine-emulator) that this
//! client starts and answers itself — no serve, no Studio.
//!
//! `ree` runs rodeo's harness (ree/Rodeo.luau), which mounts the built rodeo
//! plugin and calls its ree entry (rodeo-plugin/src/library/ree.luau). The run
//! itself is the plugin's own code — the same `library/run` a Studio's plugin
//! runs — so a script's `@rodeo/*` calls, its log capture and its return value
//! work as they do in Studio. Only the transport differs: where the plugin has
//! a WebSocket to its backend, the engine has a host call
//! (`Context.HostCall`, ree's src/hostcall.rs) to this process. Each call is
//! one `PluginMessage` as JSON in a length-prefixed frame over a loopback TCP
//! connection, answered with one `ServerMessage` frame (empty for a message
//! that has no reply):
//!
//! ```text
//! engine                          this client
//!   studioState (hello)    ->
//!                          <-     run (the RunCommand)
//!   rpc (a ClientRpcCall)  ->     RunEvent::RpcCall to the message loop,
//!                          <-     rpcResponse once it is dispatched
//!   done (ExecutionDone)   ->
//!                          <-     (empty)
//! ```
//!
//! The engine waits for each reply and its clocks stand meanwhile, so one
//! call is in flight at a time. The driver turns the frames into the same
//! `RunEvent`s the master sends, and run.rs's message loop does the rest.
//! One run per engine process: `ree` exits when the run is over.

use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::net::TcpListener;
use tokio::sync::mpsc;

use crate::proto;
use crate::run::{run_on_channel, RunChannel, RunCodeOpts, RunStream};

const HARNESS: &str = include_str!("ree/Rodeo.luau");

/// The environment variables ree's host call reads (ree's src/hostcall.rs).
const HOST_ADDRESS_ENV: &str = "REE_HOST_CALL";
const HOST_TOKEN_ENV: &str = "REE_HOST_TOKEN";

/// How long a started `ree` has to dial back.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);

/// How long the engine has to end a run this client asked it to kill (a
/// script's process.exit) before the process is ended for it.
const KILL_GRACE: Duration = Duration::from_secs(5);

/// The largest frame either side sends (ree's own limit).
const MAX_FRAME: usize = 1 << 30;

/// Virtual seconds a run gets unless the caller says otherwise. `ree` steps
/// frames only while the script waits on its own clock (`task.wait`, a
/// Heartbeat), as fast as the machine goes; waits on the host take none.
pub const DEFAULT_SECONDS: f64 = 3600.0;

/// How to start the engine for one run.
pub struct ReeEngine {
    /// The `ree` executable.
    pub exe: PathBuf,
    /// The built rodeo plugin (`.rbxm`) whose modules run the script.
    pub plugin: Vec<u8>,
    /// Engine options handed to `ree run` as they are (`--roots`,
    /// `--project`, `--mount`, `--mode`, `--run-context`, ...).
    pub args: Vec<String>,
    /// Virtual seconds the run may take before `ree` ends it.
    pub seconds: f64,
}

/// Run `opts` in a new `ree` process. The stream ends with the run.
pub async fn run_stream(engine: ReeEngine, opts: RunCodeOpts) -> Result<RunStream> {
    let print_errors = opts.log_filter.as_ref().is_none_or(|f| f.enable_error);
    let channel = start(engine, opts).await?;
    Ok(run_on_channel(channel, None, print_errors))
}

/// The run's files in a temp dir, removed when the run is over.
struct RunDir(PathBuf);

impl Drop for RunDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

struct ReeChannel {
    events: mpsc::UnboundedReceiver<proto::RunEvent>,
    replies: mpsc::UnboundedSender<proto::RunClientMessage>,
}

impl RunChannel for ReeChannel {
    async fn next(&mut self) -> Result<Option<proto::RunEvent>, String> {
        Ok(self.events.recv().await)
    }

    async fn send(&mut self, message: proto::RunClientMessage) {
        let _ = self.replies.send(message);
    }

    fn closed_reason(&self) -> &'static str {
        "ree's driver ended before the run completed"
    }
}

async fn start(engine: ReeEngine, opts: RunCodeOpts) -> Result<ReeChannel> {
    let run_id = uuid::Uuid::new_v4().simple().to_string();
    let execution_id = run_id[..12].to_string();

    let dir = std::env::temp_dir().join(format!("rodeo-ree-{run_id}"));
    std::fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
    let dir = RunDir(dir);
    let harness = dir.0.join("Rodeo.luau");
    let plugin = dir.0.join("plugin.rbxm");
    std::fs::write(&harness, HARNESS).with_context(|| format!("writing {}", harness.display()))?;
    std::fs::write(&plugin, &engine.plugin).with_context(|| format!("writing {}", plugin.display()))?;

    let listener = TcpListener::bind("127.0.0.1:0").await.context("opening the engine's host port")?;
    let address = listener.local_addr().context("reading the engine's host port")?;
    let token = uuid::Uuid::new_v4().simple().to_string();

    let context = serde_json::json!({ "RodeoPlugin": plugin.to_string_lossy() }).to_string();
    let mut command = tokio::process::Command::new(&engine.exe);
    command
        .arg("run")
        .arg("--harness")
        .arg(&harness)
        .arg("--seconds")
        .arg(engine.seconds.to_string())
        // The run is over when the script is, however long its host takes
        // (a person at stdin): no wall-clock cap. ree still exits when this
        // process is gone (its parent watch).
        .arg("--max-wall-seconds")
        .arg("0")
        .arg("--context")
        .arg(&context)
        .args(&engine.args)
        .env(HOST_ADDRESS_ENV, address.to_string())
        .env(HOST_TOKEN_ENV, &token)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let child = command
        .spawn()
        .map_err(|e| anyhow!("couldn't start ree at {}: {e}", engine.exe.display()))?;

    let run = proto::RunCommand {
        execution_id: execution_id.clone(),
        script: opts.source,
        // The run's code is required on the harness's own thread: the engine
        // is one DataModel whose kind (edit, server, client) its own options
        // set, so there is no other DOM or identity to hop to.
        context: "plugin".to_string(),
        log_filter: opts.log_filter.map(buffa::MessageField::some).unwrap_or_else(buffa::MessageField::none),
        reload_requires: if opts.reload_requires { Some(true) } else { None },
        script_args: opts.script_args,
        return_file: opts.return_file,
        show_return: if opts.show_return { Some(true) } else { None },
        output_file: opts.output_file,
        verbose: if opts.verbose { Some(true) } else { None },
        instance_path: opts.instance_path,
        script_path: opts.script_path,
        ..Default::default()
    };

    let (events_tx, events_rx) = mpsc::unbounded_channel();
    let (replies_tx, replies_rx) = mpsc::unbounded_channel();
    let driver = Driver { child, listener, token, run: Some(run), execution_id, seconds: engine.seconds, events: events_tx, replies: replies_rx, _dir: dir };
    tokio::spawn(driver.drive());
    Ok(ReeChannel { events: events_rx, replies: replies_tx })
}

struct Driver {
    child: tokio::process::Child,
    listener: TcpListener,
    token: String,
    run: Option<proto::RunCommand>,
    execution_id: String,
    seconds: f64,
    events: mpsc::UnboundedSender<proto::RunEvent>,
    replies: mpsc::UnboundedReceiver<proto::RunClientMessage>,
    _dir: RunDir,
}

/// How the engine's side of the run ended.
enum Ending {
    /// The engine sent its ExecutionDone.
    Done(Box<proto::ExecutionDone>),
    /// This client killed it (`process.exit`, a dropped stream).
    Killed,
    /// It hung up, never connected or broke the protocol; the reason so far.
    Lost(Option<String>),
}

impl Driver {
    fn emit(&self, event: proto::run_event::Event) {
        let _ = self.events.send(proto::RunEvent { event: Some(event), ..Default::default() });
    }

    async fn drive(mut self) {
        self.emit(proto::run_event::Event::Created(Box::new(proto::ProcessCreated {
            execution_id: self.execution_id.clone(),
            ..Default::default()
        })));

        let stdout = tokio::spawn(read_all(self.child.stdout.take()));
        let stderr = tokio::spawn(read_all(self.child.stderr.take()));

        let ending = match self.connect().await {
            Ok((reader, writer)) => self.converse(reader, writer).await,
            Err(reason) => Ending::Lost(reason),
        };
        if !matches!(ending, Ending::Done(_)) {
            let _ = self.child.start_kill();
        }
        let status = self.child.wait().await.ok();
        let stdout = stdout.await.unwrap_or_default();
        let stderr = stderr.await.unwrap_or_default();
        let summary: Option<serde_json::Value> = serde_json::from_slice(trim(&stdout)).ok();

        match ending {
            Ending::Done(done) => {
                // What the engine could not do for the script: members it
                // does not emulate report themselves instead of answering.
                for note in missing_notes(summary.as_ref()) {
                    self.note(&format!("\x1b[33mree: {note} is not emulated and answered nil\x1b[0m\n"));
                }
                self.emit(proto::run_event::Event::ExecutionDone(done));
                self.emit(proto::run_event::Event::Complete(Box::new(proto::RunComplete::default())));
            }
            Ending::Killed => {
                self.emit(proto::run_event::Event::ExecutionKilled(Box::new(proto::ExecutionKilled {
                    execution_id: self.execution_id.clone(),
                    ..Default::default()
                })));
                self.emit(proto::run_event::Event::Complete(Box::new(proto::RunComplete::default())));
            }
            Ending::Lost(reason) => {
                let reason = reason.unwrap_or_else(|| lost_reason(status, summary.as_ref(), &stderr, self.seconds));
                self.emit(proto::run_event::Event::Disconnect(reason));
            }
        }
    }

    /// A line for the user on the run's stderr, as a write the message loop
    /// dispatches like the script's own (a notify: no reply comes back here).
    fn note(&self, text: &str) {
        use proto::runtime::client_rpc_call::Req;
        self.emit(proto::run_event::Event::RpcCall(Box::new(proto::runtime::ClientRpcCall {
            id: String::new(),
            execution_id: self.execution_id.clone(),
            req: Some(Req::StreamWrite(Box::new(proto::runtime::StreamWriteRequest {
                handle: "stderr".to_string(),
                data: text.to_string(),
                ..Default::default()
            }))),
            ..Default::default()
        })));
    }

    /// Waits for the engine to dial back and say the token. `Err(None)`: it
    /// exited first, and its exit says why.
    async fn connect(&mut self) -> std::result::Result<(OwnedReadHalf, OwnedWriteHalf), Option<String>> {
        let token = self.token.clone();
        let listener = &self.listener;
        let accept = async move {
            let (stream, _) = listener.accept().await.map_err(|e| format!("the engine's host port failed: {e}"))?;
            let _ = stream.set_nodelay(true);
            let (mut reader, writer) = stream.into_split();
            let said = read_frame(&mut reader).await.map_err(|e| format!("ree connected and hung up: {e}"))?;
            if said != token.as_bytes() {
                return Err("a process that is not this run's ree connected to its host port".to_string());
            }
            Ok((reader, writer))
        };
        tokio::select! {
            connected = accept => connected.map_err(Some),
            _ = self.child.wait() => Err(None),
            _ = tokio::time::sleep(CONNECT_TIMEOUT) => Err(Some(format!(
                "ree did not connect back within {}s. `rodeo run --ree` needs a ree with the host call (rodeo-rbx/roblox-engine-emulator newer than 0.2.0-rodeo.1)",
                CONNECT_TIMEOUT.as_secs()
            ))),
        }
    }

    /// The run's conversation, until the engine hangs up or is killed.
    async fn converse(&mut self, reader: OwnedReadHalf, mut writer: OwnedWriteHalf) -> Ending {
        use proto::plugin_message::Msg;

        // Frames are read on their own task: a read cancelled half-way (a
        // reply arriving meanwhile) would lose the bytes it had taken.
        let (frames_tx, mut frames) = mpsc::unbounded_channel::<Vec<u8>>();
        let reading = tokio::spawn(async move {
            let mut reader = reader;
            while let Ok(frame) = read_frame(&mut reader).await {
                if frames_tx.send(frame).is_err() {
                    break;
                }
            }
        });

        // The call the engine is waiting on, by its RPC id.
        let mut awaiting: Option<String> = None;
        let mut done: Option<Box<proto::ExecutionDone>> = None;
        // The engine said it ended the run on a kill (a process.exit).
        let mut killed = false;
        // A kill this client asked for: the plugin's ree entry ends the run
        // itself a frame later, once the lines the script logged before its
        // process.exit have reached us. Past this, the engine is ended here.
        let mut kill_by: Option<tokio::time::Instant> = None;
        let ending = loop {
            let overdue = async {
                match kill_by {
                    Some(deadline) => tokio::time::sleep_until(deadline).await,
                    None => std::future::pending().await,
                }
            };
            tokio::select! {
                frame = frames.recv() => {
                    let Some(frame) = frame else {
                        // The engine hung up: after its done or killed, the run is over.
                        break match done.take() {
                            Some(done) => Ending::Done(done),
                            None if killed => Ending::Killed,
                            None => Ending::Lost(None),
                        };
                    };
                    let message: proto::PluginMessage = match serde_json::from_slice(&frame) {
                        Ok(message) => message,
                        Err(e) => break Ending::Lost(Some(format!("ree sent a message this rodeo can't read: {e}"))),
                    };
                    let reply: Vec<u8> = match message.msg {
                        Some(Msg::StudioState(hello)) => {
                            if let Err(mismatch) = proto::check_peer_version("the rodeo plugin in ree", &hello.plugin_version) {
                                if !proto::version_check_skipped() {
                                    break Ending::Lost(Some(mismatch));
                                }
                            }
                            match self.run.take() {
                                Some(run) => encode(proto::server_message::Msg::Run(Box::new(run))),
                                None => break Ending::Lost(Some("ree said hello twice".to_string())),
                            }
                        }
                        Some(Msg::Rpc(call)) => {
                            let notify = call.id.is_empty();
                            if !notify {
                                awaiting = Some(call.id.clone());
                            }
                            self.emit(proto::run_event::Event::RpcCall(call));
                            if !notify {
                                // Answered below, once the message loop has dispatched it.
                                continue;
                            }
                            Vec::new()
                        }
                        Some(Msg::Done(finished)) => {
                            done = Some(finished);
                            Vec::new()
                        }
                        Some(Msg::Killed(_)) => {
                            killed = true;
                            Vec::new()
                        }
                        // (mode transitions are a Studio's)
                        Some(Msg::ModeFailed(_)) | None => Vec::new(),
                    };
                    if write_frame(&mut writer, &reply).await.is_err() {
                        // (its hang-up arrives on the frames channel)
                        continue;
                    }
                }
                reply = self.replies.recv() => {
                    use proto::run_client_message::Msg as Reply;
                    match reply.and_then(|message| message.msg) {
                        Some(Reply::RpcResponse(response)) => {
                            // (a notify's response has no one waiting: dropped)
                            if awaiting.as_deref() == Some(response.id.as_str()) {
                                awaiting = None;
                                let frame = encode(proto::server_message::Msg::RpcResponse(response));
                                let _ = write_frame(&mut writer, &frame).await;
                            }
                        }
                        Some(Reply::Kill(_)) => {
                            kill_by.get_or_insert(tokio::time::Instant::now() + KILL_GRACE);
                        }
                        Some(_) => {}
                        // The run's consumer is gone: nobody waits for the rest.
                        None => break Ending::Killed,
                    }
                }
                _ = overdue => break Ending::Killed,
            }
        };
        reading.abort();
        ending
    }
}

/// A ServerMessage as the frame the plugin's ree entry decodes.
fn encode(msg: proto::server_message::Msg) -> Vec<u8> {
    serde_json::to_vec(&proto::ServerMessage { msg: Some(msg), ..Default::default() }).unwrap_or_default()
}

async fn read_frame(reader: &mut OwnedReadHalf) -> std::io::Result<Vec<u8>> {
    let mut length = [0u8; 4];
    reader.read_exact(&mut length).await?;
    let length = u32::from_le_bytes(length) as usize;
    if length > MAX_FRAME {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "frame past the limit"));
    }
    let mut bytes = vec![0u8; length];
    reader.read_exact(&mut bytes).await?;
    Ok(bytes)
}

async fn write_frame(writer: &mut OwnedWriteHalf, bytes: &[u8]) -> std::io::Result<()> {
    writer.write_all(&(bytes.len() as u32).to_le_bytes()).await?;
    writer.write_all(bytes).await?;
    writer.flush().await
}

async fn read_all<R: tokio::io::AsyncRead + Unpin>(pipe: Option<R>) -> Vec<u8> {
    let mut bytes = Vec::new();
    if let Some(mut pipe) = pipe {
        let _ = pipe.read_to_end(&mut bytes).await;
    }
    bytes
}

fn trim(bytes: &[u8]) -> &[u8] {
    let text = std::str::from_utf8(bytes).unwrap_or("");
    text.trim().as_bytes()
}

/// The members the run used that ree does not emulate (its summary's
/// `missing` list: each answered nil instead of doing its work).
fn missing_notes(summary: Option<&serde_json::Value>) -> Vec<String> {
    let members = summary.and_then(|summary| summary["missing"].as_array());
    members
        .into_iter()
        .flatten()
        .filter_map(|entry| {
            let member = entry["member"].as_str()?;
            Some(match entry["count"].as_u64() {
                Some(count) if count > 1 => format!("{member} (used {count} times)"),
                _ => member.to_string(),
            })
        })
        .collect()
}

/// Why a run whose engine went away without its ExecutionDone ended.
fn lost_reason(status: Option<std::process::ExitStatus>, summary: Option<&serde_json::Value>, stderr: &[u8], seconds: f64) -> String {
    let stderr = String::from_utf8_lossy(stderr);
    let stderr = stderr.trim();
    let code = status.and_then(|status| status.code());
    if let Some(summary) = summary {
        // The harness failed (the plugin's entry raised), or the script
        // outlasted the run's virtual time.
        if let Some(failure) = summary["failure"].as_str() {
            return format!("the ree run failed: {failure}");
        }
        let errors: Vec<&str> = summary["output"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|line| line["kind"] == "error")
            .filter_map(|line| line["message"].as_str())
            .collect();
        if !errors.is_empty() {
            return format!("the ree run ended without the script's result: {}", errors.join("\n"));
        }
        if summary["done"] != true {
            return format!(
                "the script was still running after {seconds} virtual seconds, where ree ends a run (it waits on its own clock: task.wait, a Heartbeat). RODEO_REE_SECONDS sets the limit"
            );
        }
        return "the ree run ended without the script's result".to_string();
    }
    let tail: Vec<&str> = stderr.lines().rev().take(8).collect::<Vec<_>>().into_iter().rev().collect();
    match code {
        Some(code) => format!("ree exited with code {code} before the run finished\n{}", tail.join("\n")),
        None => format!("ree ended before the run finished\n{}", tail.join("\n")),
    }
}

/// The `ree` to run: `RODEO_REE` when set, else the one on PATH.
pub fn locate() -> Result<PathBuf> {
    if let Some(path) = std::env::var_os("RODEO_REE").filter(|path| !path.is_empty()) {
        return Ok(PathBuf::from(path));
    }
    let name = if cfg!(windows) { "ree.exe" } else { "ree" };
    let path = std::env::var_os("PATH").unwrap_or_default();
    std::env::split_paths(&path)
        .map(|dir| dir.join(name))
        .find(|candidate| candidate.is_file())
        .ok_or_else(|| {
            anyhow!(
                "--ree needs ree (roblox-engine-emulator) and found none on PATH. Install github:rodeo-rbx/roblox-engine-emulator \
                 (newer than 0.2.0-rodeo.1), or set RODEO_REE to its executable"
            )
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_members_are_noted_with_their_counts() {
        let summary = serde_json::json!({ "output": [{ "kind": "print", "message": "hello" }], "missing": [
            { "member": "EncodingService.Base64Encode", "count": 3 },
            { "member": "Sound.PlaybackLoudness", "count": 1 },
        ] });
        assert_eq!(missing_notes(Some(&summary)), ["EncodingService.Base64Encode (used 3 times)", "Sound.PlaybackLoudness"]);
        assert!(missing_notes(Some(&serde_json::json!({ "output": [] }))).is_empty());
        assert!(missing_notes(None).is_empty());
    }

    #[test]
    fn a_lost_run_says_why() {
        let over = serde_json::json!({ "status": "ok", "output": [] });
        assert!(lost_reason(None, Some(&over), b"", 60.0).contains("still running after 60 virtual seconds"));
        let failed = serde_json::json!({ "status": "error", "failure": "the harness raised", "output": [] });
        assert_eq!(lost_reason(None, Some(&failed), b"", 60.0), "the ree run failed: the harness raised");
        let raised = serde_json::json!({ "status": "ok", "done": true, "output": [{ "kind": "error", "message": "boom" }] });
        assert!(lost_reason(None, Some(&raised), b"", 60.0).ends_with("boom"));
        assert!(lost_reason(None, None, b"first\nree: unknown option --harness", 60.0).contains("ree: unknown option --harness"));
    }
}
