pub mod grpc;

use crate::studio_backend as studio_crate;
use rbx_control::studio::mcp_client::StudioMcpClient;
use rodeo_proto::ProcessState;
use tracing::info;
use crate::studio_backend::connection::{RunRequest, StudioInstance, DomConnection};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex};

/// Message type for sending to run clients — typed variants only, no JSON tunneling.
pub enum ClientMsg {
    FileChunk(rodeo_proto::FileChunk),
    Complete,
    RpcCall(Box<rodeo_proto::runtime_types::ClientRpcCall>),
    ExecutionDone(Box<rodeo_proto::ExecutionDone>),
    ExecutionKilled(Box<rodeo_proto::ExecutionKilled>),
    Disconnect(String),
}

/// Shared server state
/// A Studio instance managed by a backend.
pub struct StudioInstanceInfo {
    /// Master-minted session identity for this Studio launch. Baked into the
    /// plugin's `flags.SESSION_GUID` so the plugin sends it on handshake and
    /// master stamps `DomConnection.session_guid` synchronously.
    pub session_guid: String,
    pub status: String, // "pending" | "launching" | "connected" | "closing" | "error"
    pub studio: Option<Arc<studio_crate::Studio>>,
    pub error: Option<String>,
    /// StudioMCP's id for this Studio process, resolved asynchronously by
    /// polling `list_roblox_studios` after spawn. Used only at the
    /// elevated-call boundary (the `studio_id` of StudioMCP tool calls) — not
    /// for routing.
    pub mcp_studio_id: Option<String>,
}


pub struct BackendState {
    /// All connected DOMs, keyed by domId
    pub doms: HashMap<String, DomConnection>,
    /// Studios derived from DOMs, keyed by studioId
    pub studios: HashMap<String, StudioInstance>,
    /// Registered remote backends, keyed by backend ID
    pub backends: HashMap<String, grpc::BackendConnection>,
    pub pending_runs: Vec<RunRequest>,
    /// Studio instances managed by this backend, keyed by master-assigned studio_id
    pub studio_instances: HashMap<String, StudioInstanceInfo>,
    pub mcp: Arc<Mutex<Option<StudioMcpClient>>>,
    /// Notify to trigger an immediate state snapshot (event-driven)
    pub snapshot_trigger: Option<Arc<tokio::sync::Notify>>,
    /// When set, plugin_ws relays messages to master via this channel (backend mode).
    pub relay_tx: Option<mpsc::UnboundedSender<rodeo_proto::BackendMessage>>,
    /// Profile scanner for collecting microprofiler dumps
    pub profile_scanner: Option<rbx_control::profile_scanner::ProfileScannerHandle>,
    /// Local port this backend listens on (for Studio plugin connections)
    pub port: u16,
    /// Cancellation token for graceful shutdown — SIGTERM cancels this
    pub shutdown_token: tokio_util::sync::CancellationToken,
    /// Master's bootstrap UUID, learned from `RegisterResponse.master_id`.
    /// Used as a tracing span field so log lines from this backend can be
    /// correlated with master's logs by `jq 'select(.master_id=="…")'`.
    pub master_id: String,
}

impl BackendState {
    pub fn new() -> Self {
        Self {
            doms: HashMap::new(),
            studios: HashMap::new(),
            backends: HashMap::new(),
            pending_runs: Vec::new(),
            studio_instances: HashMap::new(),
            mcp: Arc::new(Mutex::new(None)),
            snapshot_trigger: None,
            relay_tx: None,
            port: 0,
            shutdown_token: tokio_util::sync::CancellationToken::new(),
            profile_scanner: None,
            master_id: String::new(),
        }
    }

    // --- DOM lookup helpers ---

    /// Check if any DOMs are still unresolved by StudioMCP reconciliation
    /// (i.e. haven't had their mcp_studio_id populated yet).
    fn has_unresolved(&self) -> bool {
        self.doms.values().any(|dom| {
            dom.connected && dom.state.as_ref().map_or(true, |s| s.mcp_studio_id.is_none())
        })
    }

    // --- Routing ---

    /// Try to find a matching DOM for a run request.
    /// Priority: --dom-id (direct) > route (mode/dom-kind matching) > any connected DOM.
    fn find_match_for_run(&self, run: &RunRequest) -> Option<String> {
        // Direct DOM targeting by ID
        if let Some(ref wanted_dom) = run.dom_id {
            if let Some(dom) = self.doms.get(wanted_dom) {
                if dom.connected {
                    return Some(wanted_dom.clone());
                }
            }
            return None; // Specific DOM requested but not found/connected
        }

        // Validated at submit; an empty route matches any connected DOM.
        let resolved = if run.route.is_empty() {
            None
        } else {
            run.route.resolve().ok()
        };

        let mut best: Option<(String, usize)> = None;

        for (dom_id, dom) in &self.doms {
            if !dom.connected {
                continue;
            }

            // Studio filter — `run.session` restricts to one studio, matching a
            // DOM by its launch session_guid (owned launch pin, e.g.
            // `run --place`) or its canonical studio_id (`--studio-id`). Applies
            // regardless of route so a scoped run never leaks into another studio.
            if let Some(ref wanted) = run.session {
                if !dom.matches_studio(wanted) {
                    continue;
                }
            }

            if let Some(ref r) = resolved {
                let dom_kind = match dom.dom_kind() {
                    Some(d) => d,
                    None => continue,
                };
                if r.dom_kind.as_str() != dom_kind {
                    continue;
                }
                // The edit DOM exists in every studio mode and always reports
                // mode="edit" (it's the non-running source DataModel), so match
                // it by kind alone — converging the studio to r.mode is driven
                // separately by derive_and_push_targets. Server/client DOMs
                // report the studio's live mode, so match that too.
                if r.dom_kind != crate::shared::target::DomKind::Edit {
                    match dom.mode() {
                        Some(m) if m == r.mode.as_str() => {}
                        _ => continue,
                    }
                }
            }

            // Load balance: prefer DOM with fewest active runs
            let count = dom.active_count();
            match &best {
                Some((_, best_count)) if count >= *best_count => {}
                _ => {
                    best = Some((dom_id.clone(), count));
                }
            }
        }

        best.map(|(dom_id, _)| dom_id)
    }

    /// Complete a run (done/killed) and update state.
    ///
    /// For profiled runs: the run stays in active_runs (draining state) so SendFile
    /// can still forward files to the client. FilesComplete removes it later.
    /// For non-profiled runs: removed immediately, Complete sent to client.
    pub fn complete_run(
        &mut self,
        execution_id: &str,
        dom_id: &str,
        new_state: ProcessState,
    ) {
        let is_profiled = self.doms.get(dom_id)
            .and_then(|dom| dom.active_runs.get(execution_id))
            .map(|run| run.profile == Some(true))
            .unwrap_or(false);

        if is_profiled {
            // Keep run alive for file transfers. Tell backend to stop profiling.
            if let Some(dom) = self.doms.get_mut(dom_id) {
                dom.mark_done(execution_id, &new_state);
            }
            for backend in self.backends.values() {
                let _ = backend.tx.send(rodeo_proto::MasterMessage {
                    msg: Some(rodeo_proto::master_message::Msg::RunCompleted(Box::new(rodeo_proto::RunCompleted {
                        execution_id: execution_id.to_string(),
                        ..Default::default()
                    }))),
                    ..Default::default()
                });
            }
        } else {
            // Non-profiled: remove and send Complete
            if let Some(dom) = self.doms.get_mut(dom_id) {
                if let Some(run) = dom.complete_run(execution_id, &new_state) {
                    let _ = run.client_tx.send(ClientMsg::Complete);
                }
            }
        }

        self.process_pending();
    }

    /// Reactive: re-evaluate all pending runs against current state.
    /// Called after any state change (DOM connect/disconnect, state update, run complete).
    pub fn process_pending(&mut self) {

        if self.pending_runs.is_empty() {
            return;
        }

        // Try to route pending runs to matching DOMs
        let mut routed = Vec::new();
        for (i, run) in self.pending_runs.iter().enumerate() {
            if let Some(dom_id) = self.find_match_for_run(run) {
                routed.push((i, dom_id));
            }
        }

        for (i, dom_id) in routed.into_iter().rev() {
            let run = self.pending_runs.remove(i);
            info!(id = run.execution_id.as_str(), dom = &dom_id[..8.min(dom_id.len())], "routed from queue");
            if let Some(dom) = self.doms.get_mut(&dom_id) {
                dom.start_run(run);
            }
        }

    }

}

pub type SharedBackendState = Arc<Mutex<BackendState>>;

// ---------------------------------------------------------------------------
// MasterState — pure snapshot-based, no per-DOM channels
// ---------------------------------------------------------------------------

pub struct MasterState {
    /// Bootstrap UUID minted at master startup by `util::log_capture::init`.
    /// Advertised to backends via `RegisterResponse.master_id` for cross-host
    /// log correlation (`jq 'select(.master_id=="…")'`).
    pub master_id: String,
    /// Registered remote backends, keyed by backend ID
    pub backends: HashMap<String, grpc::BackendConnection>,
    /// Active runs on DOMs, keyed by execution_id
    pub active_runs: HashMap<String, ActiveRun>,
    /// Pending run requests waiting for a matching DOM
    pub pending_runs: Vec<RunRequest>,
    /// In-flight save RPCs: master sends a typed SaveCommand on the control
    /// stream, studio backend replies with SaveResult, and this map routes the
    /// reply back to the awaiting save_place handler via a one-shot channel
    /// keyed by request_id (a UUID minted per-RPC). Using request_id rather
    /// than session_guid keeps routing independent of payload — session_guid
    /// is optional on the wire (CLI saves without specifying one), so it
    /// can't serve as the routing key.
    pub pending_saves: HashMap<String, tokio::sync::oneshot::Sender<rodeo_proto::SaveResult>>,
    /// Declarative per-studio target as last pushed, keyed by studio (see
    /// `studio_key`). Re-pushed to every DOM of the studio whenever the
    /// target or the studio's DOM set changes: a DOM that just connected
    /// (e.g. the server DOM of a session the target has to end) learns the
    /// target that way. The plugin ignores an unchanged target.
    pub target_modes: HashMap<String, StudioTarget>,
    /// Targets set through SetStudioMode, keyed by studio: (mode,
    /// num_players). Queued runs take precedence; dropped once the studio
    /// reaches the mode (or disconnects), which clears the pushed target.
    pub explicit_targets: HashMap<String, (crate::shared::target::StudioMode, u32)>,
}

/// A studio's target mode as pushed to its DOMs (SetTargetModeMsg).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StudioTarget {
    /// "" (none) | "edit" | "run" | "test" | "play"
    pub mode: String,
    /// Target "play" only: clients the multiplayer test starts with.
    pub num_players: u32,
    /// The studio's connected DOM ids, sorted, at the last push.
    pub doms: Vec<String>,
}

/// The studio a DOM belongs to, as the master groups them for targets and
/// routing: the canonical studio_id, else the launch session_guid, else the
/// DOM's own id (a lone DOM before its studio id lands).
fn studio_key(dom: &rodeo_proto::DomSnapshot) -> String {
    let non_empty = |s: &Option<String>| s.clone().filter(|v| !v.is_empty());
    non_empty(&dom.studio_id)
        .or_else(|| non_empty(&dom.session_guid))
        .unwrap_or_else(|| dom.dom_id.clone())
}

/// Connected DOMs grouped into studios (by `studio_key`).
struct StudioIndex {
    groups: HashMap<String, Vec<rodeo_proto::DomSnapshot>>,
    /// Every identifier a run's `session` filter may carry — a launch
    /// session_guid or a canonical studio_id — mapped to its studio.
    ident_to_group: HashMap<String, String>,
}

impl StudioIndex {
    /// The mode a studio is in: its server DOM's (run/test/play), else a
    /// client DOM's, else edit. "" for an unknown studio.
    fn live_mode(&self, group: &str) -> &str {
        let Some(doms) = self.groups.get(group) else { return "" };
        let mode_of = |kind: &str| {
            doms.iter()
                .find(|d| d.dom_kind.as_deref() == Some(kind))
                .and_then(|d| d.mode.as_deref())
        };
        mode_of("server").or_else(|| mode_of("client")).unwrap_or("edit")
    }

    fn group_of_dom(&self, dom_id: &str) -> Option<String> {
        self.groups.iter()
            .find(|(_, doms)| doms.iter().any(|d| d.dom_id == dom_id))
            .map(|(group, _)| group.clone())
    }

    /// The studios a queued run drives: the one its session filter names, or
    /// every studio for a run with no filter.
    fn groups_driven_by(&self, run: &RunRequest) -> Vec<String> {
        match run.session.as_deref().filter(|s| !s.is_empty()) {
            Some(s) => self.ident_to_group.get(s).cloned().into_iter().collect(),
            None => self.groups.keys().cloned().collect(),
        }
    }
}

/// The studio mode a queued run asks its studio to converge to, if any.
/// Pinned runs and empty routes drive nothing. An omitted `--mode` resolves to
/// edit but only targets the edit DOM, which exists in every mode; an explicit
/// `--mode edit` asks for the studio itself to be in edit, ending a session.
fn transition_mode(run: &RunRequest) -> Option<crate::shared::target::StudioMode> {
    use crate::shared::target::StudioMode;
    if run.dom_id.is_some() || run.route.is_empty() {
        return None;
    }
    let resolved = run.route.resolve().ok()?;
    match resolved.mode {
        StudioMode::Edit if run.route.mode != Some(StudioMode::Edit) => None,
        mode => Some(mode),
    }
}

/// A run that's been routed to a DOM and is executing.
pub struct ActiveRun {
    pub execution_id: String,
    pub dom_id: String,
    /// Resolved route, surfaced in the snapshot's process list. mode/dom_kind
    /// are empty for --dom-id-pinned runs (no routing happened); context is
    /// always set (plugin default).
    pub mode: String,
    pub dom_kind: String,
    pub context: String,
    pub client_tx: mpsc::UnboundedSender<ClientMsg>,
    pub state: ProcessState,
    pub profile: Option<bool>,
    pub created_at: f64,
}

impl MasterState {
    pub fn new(master_id: String) -> Self {
        Self {
            master_id,
            backends: HashMap::new(),
            active_runs: HashMap::new(),
            pending_runs: Vec::new(),
            pending_saves: HashMap::new(),
            target_modes: HashMap::new(),
            explicit_targets: HashMap::new(),
        }
    }

    /// Group the connected DOMs of every backend into studios.
    fn studio_index(&self) -> StudioIndex {
        let mut index = StudioIndex { groups: HashMap::new(), ident_to_group: HashMap::new() };
        for backend in self.backends.values() {
            let snap = backend.state_rx.borrow();
            for dom in &snap.doms {
                if !dom.connected { continue; }
                let key = studio_key(dom);
                for ident in [&dom.studio_id, &dom.session_guid].into_iter().flatten() {
                    if !ident.is_empty() {
                        index.ident_to_group.insert(ident.clone(), key.clone());
                    }
                }
                index.groups.entry(key).or_default().push(dom.clone());
            }
        }
        index
    }

    /// Mint a run id: 12 hex chars, unique among live runs. The master is the
    /// sole id authority — clients never supply one — so uniqueness is a
    /// server guarantee rather than a client promise.
    pub fn mint_execution_id(&self) -> String {
        loop {
            let id = uuid::Uuid::new_v4().simple().to_string()[..12].to_string();
            if !self.active_runs.contains_key(&id)
                && !self.pending_runs.iter().any(|r| r.execution_id == id)
            {
                return id;
            }
        }
    }

    /// Get all DOMs across all backends from their latest snapshots.
    fn all_doms(&self) -> Vec<(String, rodeo_proto::DomSnapshot)> {
        let mut result = Vec::new();
        for (backend_id, backend) in &self.backends {
            let snap = backend.state_rx.borrow();
            for dom in &snap.doms {
                let mut dom = dom.clone();
                dom.backend_id = Some(backend_id.clone());
                result.push((dom.dom_id.clone(), dom));
            }
        }
        result
    }

    /// A DOM's latest snapshot, from whichever backend reports it.
    fn dom_snapshot(&self, dom_id: &str) -> Option<rodeo_proto::DomSnapshot> {
        self.backends.values().find_map(|backend| {
            backend.state_rx.borrow().doms.iter().find(|v| v.dom_id == dom_id).cloned()
        })
    }

    /// Find the backend that owns a DOM (by checking snapshots).
    fn backend_for_dom(&self, dom_id: &str) -> Option<&grpc::BackendConnection> {
        for backend in self.backends.values() {
            let snap = backend.state_rx.borrow();
            if snap.doms.iter().any(|v| v.dom_id == dom_id) {
                return Some(backend);
            }
        }
        None
    }

    /// Send a typed ServerMessage to a DOM's plugin via its backend's control stream.
    pub fn send_to_dom(&self, dom_id: &str, message: rodeo_proto::ServerMessage) {
        let kind = match &message.msg {
            Some(rodeo_proto::server_message::Msg::Welcome(_)) => "welcome",
            Some(rodeo_proto::server_message::Msg::Run(_)) => "run",
            Some(rodeo_proto::server_message::Msg::Kill(_)) => "kill",
            Some(rodeo_proto::server_message::Msg::RpcResponse(_)) => "rpc_response",
            Some(rodeo_proto::server_message::Msg::SetTargetMode(_)) => "set_target_mode",
            Some(rodeo_proto::server_message::Msg::ScriptChunk(_)) => "script_chunk",
            None => "empty",
        };
        let dom_short = &dom_id[..8.min(dom_id.len())];
        if let Some(backend) = self.backend_for_dom(dom_id) {
            let send_res = backend.tx.send(rodeo_proto::MasterMessage {
                msg: Some(rodeo_proto::master_message::Msg::DomServerMessage(Box::new(rodeo_proto::DomServerMessage {
                    dom_id: dom_id.to_string(),
                    message: buffa::MessageField::some(message),
                    ..Default::default()
                }))),
                ..Default::default()
            });
            let backend_short = &backend.id[..8.min(backend.id.len())];
            if let Err(e) = send_res {
                tracing::warn!(dom = dom_short, kind, backend = backend_short, "send_to_dom: backend tx send failed: {e}");
            } else {
                tracing::debug!(dom = dom_short, kind, backend = backend_short, "send_to_dom: forwarded");
            }
        } else {
            tracing::warn!(dom = dom_short, kind, "send_to_dom: no backend found for DOM");
        }
    }

    /// Find a matching DOM for a run request using proto snapshots.
    /// `Ok(None)` keeps the run queued; `Err` means it can never dispatch.
    pub fn find_match_for_run(&self, run: &RunRequest) -> Result<Option<String>, String> {
        // Direct DOM targeting by ID
        if let Some(ref wanted_dom) = run.dom_id {
            // Check if DOM exists in any backend snapshot
            let Some(dom) = self.dom_snapshot(wanted_dom) else {
                return Ok(None);
            };
            // Plugin/elevated fit every DOM. Any other context must fit the
            // pinned DOM's kind — checked here rather than at submit, because
            // the kind is the DOM's, not the caller's (issue #14). A DOM's kind
            // never changes, so a misfit is final; a kind not reported yet
            // (the DOM's state hasn't landed) waits for the next snapshot.
            let Some(context) = run.route.context.filter(|c| c.depends_on_dom_kind()) else {
                return Ok(Some(wanted_dom.clone()));
            };
            let kind = dom.dom_kind.as_deref().and_then(|k| crate::shared::target::DomKind::parse(k).ok());
            return match kind {
                None => Ok(None),
                Some(kind) => crate::shared::target::check_pinned_context(context, kind)
                    .map(|()| Some(wanted_dom.clone()))
                    .map_err(|e| e.to_string()),
            };
        }

        // Validated at submit; an empty route matches any connected DOM.
        let resolved = if run.route.is_empty() {
            None
        } else {
            run.route.resolve().ok()
        };

        let all_doms = self.all_doms();
        let mut best: Option<(String, usize)> = None;

        // An explicit `--mode edit` asks for the studio to be in edit, not
        // just for its edit DOM (which exists in every mode): hold the run
        // while its studio still has a session's DOMs, so
        // derive_and_push_targets ends that session first.
        let studios_in_session: std::collections::HashSet<String> =
            if run.route.mode == Some(crate::shared::target::StudioMode::Edit) {
                all_doms.iter()
                    .filter(|(_, d)| d.connected && matches!(d.dom_kind.as_deref(), Some("server") | Some("client")))
                    .map(|(_, d)| studio_key(d))
                    .collect()
            } else {
                std::collections::HashSet::new()
            };

        for (dom_id, dom) in &all_doms {
            if !dom.connected {
                continue;
            }
            if studios_in_session.contains(&studio_key(dom)) {
                continue;
            }

            // Session filter applies regardless of route — see the sibling
            // find_match_for_run: a session-pinned default-route run must not
            // route into another session's DOMs.
            if let Some(ref wanted) = run.session {
                let matches = dom.session_guid.as_deref() == Some(wanted.as_str())
                    || dom.studio_id.as_deref() == Some(wanted.as_str());
                if !matches {
                    continue;
                }
            }

            if let Some(ref r) = resolved {
                let dom_kind = dom.dom_kind.as_deref().unwrap_or("");
                if r.dom_kind.as_str() != dom_kind {
                    continue;
                }
                // Edit DOM: match by kind alone — it exists in every mode and
                // always reports mode="edit"; studio convergence is separate.
                if r.dom_kind != crate::shared::target::DomKind::Edit {
                    let dom_mode = dom.mode.as_deref().unwrap_or("");
                    if r.mode.as_str() != dom_mode {
                        continue;
                    }
                }
            }

            let count = dom.active_runs as usize;
            match &best {
                Some((_, best_count)) if count >= *best_count => {}
                _ => { best = Some((dom_id.clone(), count)); }
            }
        }

        Ok(best.map(|(dom_id, _)| dom_id))
    }

    /// Build the RunCommand message sequence for a RunRequest. A script over
    /// SCRIPT_CHUNK_SIZE is split: the RunCommand carries the first piece
    /// with `script_continues`, followed by ScriptChunk messages the plugin
    /// reassembles. Each ServerMessage becomes its own WS frame on the
    /// backend→plugin hop, so no frame grows with bundle size.
    fn build_run_command(run: &RunRequest) -> Vec<rodeo_proto::ServerMessage> {
        // Pinned runs carry at most a context; routed runs resolve through the
        // defaults table. The plugin receives only the run context.
        let context = run
            .route
            .context
            .or_else(|| run.route.resolve().ok().map(|r| r.context))
            .unwrap_or(crate::shared::target::RunContext::Plugin);

        let mut pieces: Vec<&str> = Vec::new();
        let mut rest = run.script.as_str();
        while rest.len() > rodeo_proto::SCRIPT_CHUNK_SIZE {
            // Char-boundary backoff: a split codepoint would be invalid UTF-8.
            let mut cut = rodeo_proto::SCRIPT_CHUNK_SIZE;
            while !rest.is_char_boundary(cut) {
                cut -= 1;
            }
            let (head, next) = rest.split_at(cut);
            pieces.push(head);
            rest = next;
        }
        pieces.push(rest);

        let mut messages = vec![rodeo_proto::ServerMessage {
            msg: Some(rodeo_proto::server_message::Msg::Run(Box::new(rodeo_proto::RunCommand {
                execution_id: run.execution_id.clone(),
                script: pieces[0].to_string(),
                context: context.as_str().to_string(),
                log_filter: buffa::MessageField::some(run.log_filter.clone()),
                reload_requires: run.reload_requires,
                script_args: run.script_args.clone().unwrap_or_default(),
                return_file: run.return_file.clone(),
                show_return: run.show_return,
                output_file: run.output_file.clone(),
                verbose: run.verbose,
                instance_path: run.instance_path.clone(),
                script_path: run.script_path.clone(),
                profile: run.profile,
                script_continues: if pieces.len() > 1 { Some(true) } else { None },
                ..Default::default()
            }))),
            ..Default::default()
        }];
        let last = pieces.len() - 1;
        for (i, piece) in pieces.iter().enumerate().skip(1) {
            messages.push(rodeo_proto::ServerMessage {
                msg: Some(rodeo_proto::server_message::Msg::ScriptChunk(Box::new(rodeo_proto::ScriptChunk {
                    execution_id: run.execution_id.clone(),
                    data: piece.to_string(),
                    is_last: i == last,
                    ..Default::default()
                }))),
                ..Default::default()
            });
        }
        messages
    }

    /// Send a run command to a DOM and track it as active.
    fn dispatch_run(&mut self, dom_id: &str, run: RunRequest) {
        // Look up DOM's canonical studio_id from snapshot for log correlation.
        let studio = {
            let mut studio = None;
            for backend in self.backends.values() {
                let snap = backend.state_rx.borrow();
                if let Some(v) = snap.doms.iter().find(|v| v.dom_id == dom_id) {
                    studio = v.session_guid.clone();
                    break;
                }
            }
            studio
        };
        // Resolved route for the snapshot: pinned runs did no routing (mode/
        // dom_kind stay empty; context defaults to plugin), routed runs record
        // the effective defaults-applied values.
        let (mode, dom_kind, context) = if run.dom_id.is_some() {
            let context = run
                .route
                .context
                .unwrap_or(crate::shared::target::RunContext::Plugin);
            (String::new(), String::new(), context.as_str().to_string())
        } else {
            match run.route.resolve() {
                Ok(r) => (
                    r.mode.as_str().to_string(),
                    r.dom_kind.as_str().to_string(),
                    r.context.as_str().to_string(),
                ),
                Err(_) => (String::new(), String::new(), String::new()),
            }
        };
        tracing::info!(
            id = run.execution_id.as_str(),
            dom = &dom_id[..8.min(dom_id.len())],
            mode = mode.as_str(),
            kind = dom_kind.as_str(),
            context = context.as_str(),
            studio = studio.as_deref().map(|s| &s[..8.min(s.len())]).unwrap_or("-"),
            "dispatch"
        );
        for cmd in Self::build_run_command(&run) {
            self.send_to_dom(dom_id, cmd);
        }
        self.active_runs.insert(run.execution_id.clone(), ActiveRun {
            execution_id: run.execution_id.clone(),
            dom_id: dom_id.to_string(),
            mode,
            dom_kind,
            context,
            client_tx: run.client_tx,
            state: ProcessState::PROCESS_STATE_RUNNING,
            profile: run.profile,
            created_at: run.created_at,
        });
    }

    /// Route a run to a matching DOM, queue it as pending, or reject it.
    /// Returns whether it was dispatched now.
    pub fn route_or_queue(&mut self, run: RunRequest) -> bool {
        let id = run.execution_id.clone();
        match self.find_match_for_run(&run) {
            Ok(Some(dom_id)) => {
                self.dispatch_run(&dom_id, run);
                info!(id = id.as_str(), "routed");
                true
            }
            Ok(None) => {
                self.pending_runs.push(run);
                self.reconcile();
                info!(id = id.as_str(), "queued (no matching dom)");
                false
            }
            Err(reason) => {
                Self::reject_run(run, &reason);
                false
            }
        }
    }

    /// End a run that can never dispatch, the same way submit rejects an
    /// invalid route. Dropping the run drops its client channel, which ends
    /// the client's event stream after the Disconnect.
    fn reject_run(run: RunRequest, reason: &str) {
        info!(id = run.execution_id.as_str(), error = reason, "rejected: invalid route");
        let _ = run.client_tx.send(ClientMsg::Disconnect(format!("invalid route: {reason}")));
    }

    /// Single entry point for all event-driven state reconciliation:
    /// re-route pending runs, drain runs targeting dead sessions, then
    /// push updated target_modes to studios' edit-DOM plugins.
    pub fn reconcile(&mut self) {
        self.process_pending();
        self.drain_dead_sessions();
        self.derive_and_push_targets();
    }

    /// Re-evaluate pending runs against current backend snapshots.
    pub fn process_pending(&mut self) {
        if self.pending_runs.is_empty() {
            return;
        }
        let mut decided = Vec::new();
        for (i, run) in self.pending_runs.iter().enumerate() {
            match self.find_match_for_run(run) {
                Ok(Some(dom_id)) => decided.push((i, Ok(dom_id))),
                Ok(None) => {}
                Err(reason) => decided.push((i, Err(reason))),
            }
        }
        for (i, decision) in decided.into_iter().rev() {
            let run = self.pending_runs.remove(i);
            match decision {
                Ok(dom_id) => {
                    info!(id = run.execution_id.as_str(), dom = &dom_id[..8.min(dom_id.len())], "routed from queue");
                    self.dispatch_run(&dom_id, run);
                }
                Err(reason) => Self::reject_run(run, &reason),
            }
        }
    }

    /// Drain pending runs whose target session has no live DOM. Without this,
    /// `runCode()` would hang forever waiting for a studio that's gone.
    /// Called alongside process_pending on every notify tick.
    pub fn drain_dead_sessions(&mut self) {
        if self.pending_runs.is_empty() {
            return;
        }

        // Collect the studio filters pending runs carry: a launch session_guid
        // (`run --place`) or a canonical studio_id (`--studio-id`).
        let targeted_sessions: std::collections::HashSet<String> = self.pending_runs.iter()
            .filter_map(|r| r.session.clone())
            .filter(|s| !s.is_empty())
            .collect();

        for scope_session in targeted_sessions {
            // Alive = some connected DOM matches the filter either way — the
            // same test routing applies (`matches_studio`). Checking only the
            // session_guid killed every `--studio-id` run that had to wait for
            // a mode transition, silently, on the next notify tick.
            let alive = self.backends.values().any(|b| {
                b.state_rx.borrow().doms.iter().any(|v| {
                    v.connected
                        && (v.session_guid.as_deref() == Some(scope_session.as_str())
                            || v.studio_id.as_deref() == Some(scope_session.as_str()))
                })
            });
            if alive {
                continue;
            }

            let (drained, kept): (Vec<_>, Vec<_>) = self.pending_runs
                .drain(..)
                .partition(|r| r.session.as_deref() == Some(scope_session.as_str()));
            self.pending_runs = kept;
            for run in drained {
                let _ = run.client_tx.send(ClientMsg::ExecutionKilled(Box::new(
                    rodeo_proto::ExecutionKilled {
                        execution_id: run.execution_id.clone(),
                        ..Default::default()
                    },
                )));
                let _ = run.client_tx.send(ClientMsg::Complete);
                info!(
                    id = run.execution_id.as_str(),
                    session = &scope_session[..8.min(scope_session.len())],
                    "pending run dropped: target session no longer alive",
                );
            }
        }
    }

    /// For each studio, derive the target mode — from the queued runs
    /// (earliest first), else an explicit SetStudioMode target — and push
    /// SetTargetModeMsg to every DOM of the studio when the target or its DOM
    /// set changed. The plugins converge: the server DOM ends a session that
    /// doesn't satisfy the target, the edit DOM starts the target's session
    /// once the engine is idle and reports a transition it gives up on (see
    /// fail_mode_transition).
    pub fn derive_and_push_targets(&mut self) {
        use crate::shared::target::{DomKind, StudioMode};
        // Studios are grouped by studio_id > session_guid > dom_id (see
        // studio_key). A run's session filter — a launch session_guid or a
        // canonical studio_id — resolves to its studio, so both an owned
        // launch pin and a `--studio-id` run drive the right studio.
        // Session-less/id-less DOMs (a hand-opened Studio before its state
        // lands) still get a standalone group a session-less run can drive.
        let index = self.studio_index();

        let mut derived: HashMap<String, (StudioMode, u32)> = HashMap::new();
        let mut ordered: Vec<&RunRequest> = self.pending_runs.iter().collect();
        ordered.sort_by(|a, b| a.created_at.partial_cmp(&b.created_at).unwrap_or(std::cmp::Ordering::Equal));
        for run in ordered {
            let Some(mode) = transition_mode(run) else { continue };
            // A multiplayer test has client DOMs only if it starts with them.
            let wants_client = mode == StudioMode::Play
                && run.route.resolve().map(|r| r.dom_kind == DomKind::Client).unwrap_or(false);
            for group in index.groups_driven_by(run) {
                let entry = derived.entry(group).or_insert((mode, 0));
                if entry.0 == mode && wants_client {
                    entry.1 = 1;
                }
            }
        }

        // Explicit targets drive a studio no queued run is driving. One the
        // studio has reached is done: dropping it clears the pushed target, so
        // the edit DOM stops working on it.
        self.explicit_targets.retain(|group, (mode, _)| {
            index.groups.contains_key(group) && index.live_mode(group) != mode.as_str()
        });
        for (group, target) in &self.explicit_targets {
            derived.entry(group.clone()).or_insert(*target);
        }

        let mut pushes: Vec<(String, String, u32)> = Vec::new();
        let mut next_state: HashMap<String, StudioTarget> = HashMap::new();
        for (group, doms) in &index.groups {
            let mut dom_ids: Vec<String> = doms.iter().map(|d| d.dom_id.clone()).collect();
            dom_ids.sort();
            let (mode, num_players) = match derived.get(group) {
                Some((mode, n)) => (mode.as_str().to_string(), *n),
                None => (String::new(), 0),
            };
            let next = StudioTarget { mode, num_players, doms: dom_ids };
            let changed = match self.target_modes.get(group) {
                None => !next.mode.is_empty(),
                Some(prev) => *prev != next,
            };
            if changed {
                for dom_id in &next.doms {
                    pushes.push((dom_id.clone(), next.mode.clone(), next.num_players));
                }
            }
            next_state.insert(group.clone(), next);
        }
        self.target_modes = next_state;

        for (dom_id, target, num_players) in pushes {
            info!(dom = &dom_id[..8.min(dom_id.len())], target = target.as_str(), num_players, "push target_mode");
            let msg = rodeo_proto::ServerMessage {
                msg: Some(rodeo_proto::server_message::Msg::SetTargetMode(Box::new(
                    rodeo_proto::SetTargetModeMsg { target_mode: target, num_players, ..Default::default() }
                ))),
                ..Default::default()
            };
            self.send_to_dom(&dom_id, msg);
        }
    }

    /// Explicitly set a studio's target mode (SetStudioMode RPC). `studio` is
    /// its launch session_guid or canonical studio_id. Queued runs keep
    /// precedence; otherwise the target drives the studio until it gets there.
    /// Errors when the studio has no connected DOM: the plugins drive the
    /// transition, so with nothing to send to, nothing would ever happen and a
    /// caller waiting on the resulting state would hang.
    pub fn set_target_mode(&mut self, studio: &str, mode: &str) -> Result<(), String> {
        use crate::shared::target::StudioMode;
        let mode = StudioMode::parse(mode).map_err(|e| e.to_string())?;
        let index = self.studio_index();
        let Some(group) = index.ident_to_group.get(studio).cloned() else {
            return Err(format!(
                "no connected DOM for studio session {studio}: the Studio is not running, \
                 its plugin is disconnected, or it was launched by a different rodeo server"
            ));
        };
        // Studio::set_mode("play") waits for a client DOM, and a multiplayer
        // test has one only if it starts with it.
        let num_players = if mode == StudioMode::Play { 1 } else { 0 };
        info!(target = mode.as_str(), studio = &studio[..8.min(studio.len())], "set target_mode explicitly");
        if index.live_mode(&group) == mode.as_str() {
            // Already there: a pushed target would have nothing to converge.
            self.explicit_targets.remove(&group);
        } else {
            self.explicit_targets.insert(group, (mode, num_players));
        }
        self.derive_and_push_targets();
        Ok(())
    }

    /// The edit DOM of `dom_id`'s studio gave up converging it to
    /// `target_mode`: fail the queued runs that were waiting on that
    /// transition with the plugin's reason — they used to stay queued forever
    /// while the plugin retried (#27, #39). With them gone the studio's target
    /// changes (to the next queued run's mode, or none) and is pushed, so the
    /// plugin stops; a run queued later for the same mode pushes it afresh.
    pub fn fail_mode_transition(&mut self, dom_id: &str, target_mode: &str, error: &str) {
        let index = self.studio_index();
        let Some(group) = index.group_of_dom(dom_id) else {
            tracing::warn!(dom = &dom_id[..8.min(dom_id.len())], target_mode, error, "mode transition failed in an unknown DOM");
            return;
        };
        // A report for a target that has since been superseded is stale: the
        // plugin is already working on the new one.
        let current = self.target_modes.get(&group).map(|t| t.mode.as_str()).unwrap_or("");
        if current != target_mode {
            info!(studio = &group[..8.min(group.len())], target_mode, current, "ignoring a mode transition failure for a superseded target");
            return;
        }
        let reason = format!("Studio {} could not enter {target_mode} mode: {error}", &group[..8.min(group.len())]);
        tracing::warn!("{reason}");

        let (failed, kept): (Vec<_>, Vec<_>) = std::mem::take(&mut self.pending_runs)
            .into_iter()
            .partition(|run| {
                transition_mode(run).map(|m| m.as_str()) == Some(target_mode)
                    && index.groups_driven_by(run).contains(&group)
            });
        self.pending_runs = kept;
        for run in failed {
            info!(id = run.execution_id.as_str(), "pending run failed: its mode transition failed");
            let _ = run.client_tx.send(ClientMsg::Disconnect(reason.clone()));
        }
        if self.explicit_targets.get(&group).is_some_and(|(mode, _)| mode.as_str() == target_mode) {
            self.explicit_targets.remove(&group);
        }
        self.reconcile();
    }

    /// Forward a typed ClientRpcCall from a DOM's plugin to the run client.
    pub fn forward_rpc_call(&self, execution_id: &str, call: rodeo_proto::runtime_types::ClientRpcCall) -> bool {
        if let Some(run) = self.active_runs.get(execution_id) {
            let _ = run.client_tx.send(ClientMsg::RpcCall(Box::new(call)));
            return true;
        }
        false
    }

    /// Forward a typed ExecutionDone event to the run client.
    pub fn forward_execution_done(&self, execution_id: &str, done: rodeo_proto::ExecutionDone) -> bool {
        if let Some(run) = self.active_runs.get(execution_id) {
            let _ = run.client_tx.send(ClientMsg::ExecutionDone(Box::new(done)));
            return true;
        }
        false
    }

    /// Forward a typed ExecutionKilled event to the run client.
    pub fn forward_execution_killed(&self, execution_id: &str, killed: rodeo_proto::ExecutionKilled) -> bool {
        if let Some(run) = self.active_runs.get(execution_id) {
            let _ = run.client_tx.send(ClientMsg::ExecutionKilled(Box::new(killed)));
            return true;
        }
        false
    }

    /// Complete a run (done/killed).
    pub fn complete_run(&mut self, execution_id: &str, new_state: ProcessState) {
        let run = self.active_runs.get(execution_id);
        let is_profiled = run.map(|r| r.profile == Some(true)).unwrap_or(false);

        if is_profiled {
            // Keep run alive until file transfers complete.
            if let Some(run) = self.active_runs.get_mut(execution_id) {
                run.state = new_state;
            }
            // Tell studio backend(s) — profile scanner unregisters on RunCompleted.
            for backend in self.backends.values() {
                let _ = backend.tx.send(rodeo_proto::MasterMessage {
                    msg: Some(rodeo_proto::master_message::Msg::RunCompleted(Box::new(rodeo_proto::RunCompleted {
                        execution_id: execution_id.to_string(),
                        ..Default::default()
                    }))),
                    ..Default::default()
                });
            }
        } else {
            if let Some(run) = self.active_runs.remove(execution_id) {
                let _ = run.client_tx.send(ClientMsg::Complete);
                let state_str = match new_state {
                    ProcessState::PROCESS_STATE_DONE => "done",
                    ProcessState::PROCESS_STATE_ERROR => "error",
                    ProcessState::PROCESS_STATE_KILLED => "killed",
                    _ => "unknown",
                };
                info!(id = run.execution_id.as_str(), state = state_str, "completed");
            }
        }

        self.reconcile();
    }

    /// Handle FilesComplete — all file transfers done; send Complete to client.
    pub fn handle_files_complete(&mut self, execution_id: &str) {
        if let Some(run) = self.active_runs.remove(execution_id) {
            let _ = run.client_tx.send(ClientMsg::Complete);
            tracing::debug!(execution_id, "sent Complete after files drained");
        }
    }

    /// Disconnect a run client: remove from pending, auto-kill if running.
    pub fn disconnect_run(&mut self, execution_id: &str) {
        info!(id = execution_id, "run client disconnected");
        self.pending_runs.retain(|r| r.execution_id != execution_id);

        if let Some(run) = self.active_runs.get(execution_id) {
            // Auto-kill: send kill command to the DOM
            let kill_msg = rodeo_proto::ServerMessage {
                msg: Some(rodeo_proto::server_message::Msg::Kill(Box::new(rodeo_proto::KillCommand {
                    execution_id: execution_id.to_string(),
                    ..Default::default()
                }))),
                ..Default::default()
            };
            let dom_id_owned = run.dom_id.clone();
            self.send_to_dom(&dom_id_owned, kill_msg);
        }
    }

    /// Get the mode for a specific session from backend snapshots.
    pub fn mode_for_session(&self, session_guid: &str) -> Option<String> {
        for backend in self.backends.values() {
            let snap = backend.state_rx.borrow();
            for dom in &snap.doms {
                if dom.session_guid.as_deref() == Some(session_guid) {
                    return dom.mode.clone();
                }
            }
        }
        None
    }

    /// Build a snapshot for GetState RPC from backend snapshots.
    pub fn snapshot(&self) -> rodeo_proto::RodeoSnapshot {
        let backends: Vec<rodeo_proto::BackendInfo> = self.backends.values().map(|b| {
            rodeo_proto::BackendInfo {
                id: b.id.clone(),
                kind: b.kind.clone(),
                name: b.name.clone(),
                ..Default::default()
            }
        }).collect();

        let mut doms = Vec::new();
        let mut instances: std::collections::HashMap<String, rodeo_proto::StudioInstanceState> =
            std::collections::HashMap::new();
        for (backend_id, backend) in &self.backends {
            let snap = backend.state_rx.borrow();
            for dom in &snap.doms {
                let mut dom = dom.clone();
                dom.backend_id = Some(backend_id.clone());
                doms.push(dom);
            }
            for inst in &snap.studio_instances {
                instances.insert(inst.session_guid.clone(), inst.clone());
            }
        }
        // Canonical studio-first state, grouped from the collected DOMs + lifecycle.
        let studios = build_studios(&doms, &instances);

        // Join each active run's dom_id against the studio-first state so the
        // process list carries where the run executes.
        let mut dom_owner: HashMap<&str, &rodeo_proto::StudioState> = HashMap::new();
        for st in &studios {
            for d in &st.doms {
                dom_owner.insert(d.dom_id.as_str(), st);
            }
        }

        let processes: Vec<rodeo_proto::ProcessInfo> = self.active_runs.values()
            .map(|r| {
                let owner = dom_owner.get(r.dom_id.as_str());
                rodeo_proto::ProcessInfo {
                    execution_id: r.execution_id.clone(),
                    state: match r.state {
                        ProcessState::PROCESS_STATE_RUNNING => "running",
                        ProcessState::PROCESS_STATE_DONE => "done",
                        ProcessState::PROCESS_STATE_ERROR => "error",
                        ProcessState::PROCESS_STATE_KILLED => "killed",
                        _ => "queued",
                    }.to_string(),
                    mode: r.mode.clone(),
                    dom_kind: r.dom_kind.clone(),
                    context: r.context.clone(),
                    studio_id: owner.map(|s| s.studio_id.clone()),
                    session_id: owner.and_then(|s| s.session_id.clone()),
                    dom_id: Some(r.dom_id.clone()),
                    created_at: r.created_at,
                    ..Default::default()
                }
            })
            // Queued runs aren't pinned to a DOM yet — only the request is known.
            .chain(self.pending_runs.iter().map(|r| {
                // Pinned runs report as dispatch_run records them: no mode or
                // dom kind (the DOM fixes both), context defaulting to plugin.
                let (mode, dom_kind, context) = if r.dom_id.is_some() {
                    let context = r.route.context.unwrap_or(crate::shared::target::RunContext::Plugin);
                    (String::new(), String::new(), context.as_str().to_string())
                } else {
                    let resolved = r.route.resolve().ok();
                    (
                        resolved.map(|v| v.mode.as_str().to_string()).unwrap_or_default(),
                        resolved.map(|v| v.dom_kind.as_str().to_string()).unwrap_or_default(),
                        resolved.map(|v| v.context.as_str().to_string()).unwrap_or_default(),
                    )
                };
                rodeo_proto::ProcessInfo {
                    execution_id: r.execution_id.clone(),
                    state: "queued".to_string(),
                    mode,
                    dom_kind,
                    context,
                    created_at: r.created_at,
                    ..Default::default()
                }
            }))
            .collect();

        // `doms` is consumed only to derive `studios` (studio-first state); the
        // flat list is no longer part of the client-facing snapshot.
        let _ = &doms;
        rodeo_proto::RodeoSnapshot {
            backends,
            processes,
            studios,
            ..Default::default()
        }
    }
}

/// Derive the canonical studio-first state from the flat DOM list + per-Studio
/// lifecycle. DOMs are grouped by `session_guid` (a session_guid-less DOM — e.g. a
/// manually-installed plugin — becomes its own single-DOM studio keyed by domId).
fn build_studios(
    doms: &[rodeo_proto::DomSnapshot],
    instances: &std::collections::HashMap<String, rodeo_proto::StudioInstanceState>,
) -> Vec<rodeo_proto::StudioState> {
    // BTreeMap for a stable, deterministic studio ordering across snapshots.
    let mut groups: std::collections::BTreeMap<String, Vec<&rodeo_proto::DomSnapshot>> =
        std::collections::BTreeMap::new();
    for dom in doms {
        if !dom.connected {
            continue;
        }
        // Group by the canonical plugin studio id — shared across every DOM of
        // one Studio process, so a manual studio's play children group with its
        // edit DOM the same as an owned studio's do. Fall back to the launch
        // session_guid (available at connect before the first state message),
        // then to the dom id (a lone id-less DOM).
        let key = dom
            .studio_id
            .clone()
            .filter(|s| !s.is_empty())
            .or_else(|| dom.session_guid.clone())
            .unwrap_or_else(|| format!("dom:{}", dom.dom_id));
        groups.entry(key).or_default().push(dom);
    }

    groups
        .into_iter()
        .map(|(id, members)| {
            let edit = members.iter().copied().find(|v| v.dom_kind.as_deref() == Some("edit"));
            // Studio mode: a non-edit DOM's mode (run/test/play) if present, else
            // the edit DOM's mode.
            let active_mode_dom = members
                .iter()
                .copied()
                .find(|v| matches!(v.dom_kind.as_deref(), Some("server") | Some("client")));
            let studio_mode = active_mode_dom
                .or(edit)
                .and_then(|v| v.mode.clone())
                .unwrap_or_default();
            // Representative DOM for place/name/backend: prefer the edit DOM.
            let rep = edit.or_else(|| members.first().copied());
            // The launch session_guid (owned studios only) — the DOMs of one
            // studio share it. Absent for manually-connected studios. Used for
            // the instance-lifecycle lookup and surfaced as `session_id`.
            let session_id = members.iter().find_map(|v| v.session_guid.clone());
            let inst = session_id.as_ref().and_then(|sg| instances.get(sg));

            rodeo_proto::StudioState {
                studio_id: id.clone(),
                backend_id: rep.and_then(|v| v.backend_id.clone()).unwrap_or_default(),
                session_id,
                place_name: rep.and_then(|v| v.game_name.clone()).unwrap_or_default(),
                place_id: rep.and_then(|v| v.place_id).unwrap_or(0),
                status: inst
                    .map(|i| i.status.clone())
                    .unwrap_or_else(|| "connected".to_string()),
                studio_mode,
                edit_dom_id: edit.map(|v| v.dom_id.clone()),
                source_path: inst.and_then(|i| i.source_path.clone()),
                working_path: inst.and_then(|i| i.working_path.clone()),
                doms: members
                    .iter()
                    .map(|v| rodeo_proto::StudioDom {
                        dom_id: v.dom_id.clone(),
                        dom_kind: v.dom_kind.clone().unwrap_or_default(),
                        user_name: v.user_name.clone(),
                        user_id: v.user_id,
                        ..Default::default()
                    })
                    .collect(),
                ..Default::default()
            }
        })
        .collect()
}


pub type SharedMasterState = Arc<Mutex<MasterState>>;

/// Connect to StudioMCP and run the reconciliation loop.
#[tracing::instrument(name = "reconcile", skip_all)]
pub async fn run_reconciliation(state: SharedBackendState) {
    loop {
        match StudioMcpClient::new("rodeo").await {
            Ok(client) => {
                let guard = state.lock().await;
                *guard.mcp.lock().await = Some(client);
                info!("StudioMCP connected");
                break;
            }
            Err(_) => {
                tokio::time::sleep(std::time::Duration::from_secs(3)).await;
            }
        }
    }

    loop {
        let has_unresolved = {
            let guard = state.lock().await;
            guard.has_unresolved()
        };

        tracing::debug!(has_unresolved, "reconciliation tick");

        if has_unresolved {
            let mcp_arc = {
                let guard = state.lock().await;
                guard.mcp.clone()
            };

            tracing::debug!("acquiring mcp lock");
            let mut mcp_guard = mcp_arc.lock().await;
            tracing::debug!(has_mcp = mcp_guard.is_some(), "mcp lock acquired");

            if let Some(mcp) = mcp_guard.as_mut() {
                tracing::debug!("calling list_studios");
                match mcp.list_studios().await {
                    Ok(studios) => {
                        tracing::debug!(count = studios.len(), "list_studios returned");
                        for studio in &studios {
                            tracing::debug!(mcp_studio_id = studio.mcp_studio_id.as_str(), "executing unifier");
                            // Unify code fires the MCP studio id into the plugin so
                            // it populates its state.mcp_studio_id. Note: the event
                            // string keys ("studio_id_from_server"/_client) are kept
                            // as-is for plugin wire compatibility — the VALUE they
                            // carry is an mcp_studio_id.
                            // The edit DOM has no play peers, so it only fires the
                            // bindable. It must be tested first: a Team Create edit
                            // DOM reports IsClient() (it is a client of Roblox's Team
                            // Create server), and a FireServer there would go to that
                            // server — and, unpcall'd, skip the bindable fire.
                            let unify_code = format!(
                                r#"local u = game:GetService("ReplicatedStorage"):FindFirstChild("RODEO_UNIFIER") if not u then return end local RunService = game:GetService("RunService") if RunService:IsEdit() then elseif RunService:IsServer() then u.RemoteEvent:FireAllClients("studio_id_from_server", "{msid}") elseif RunService:IsClient() then u.RemoteEvent:FireServer("studio_id_from_client", "{msid}") end u.BindableEvent:Fire("{msid}")"#,
                                msid = studio.mcp_studio_id,
                            );
                            // StudioMCP requires a datamodel_type ("Edit" /
                            // "Server" / "Client") and only runs in types
                            // available in the Studio's current mode. The
                            // unifier self-branches on RunService, so fire it
                            // into every type; types not present in the current
                            // mode just error and are ignored. This is what
                            // distinguishes a play session's Server/Client DOMs
                            // (otherwise they stay unresolved and `test:*`
                            // targets never route to them).
                            for datamodel_type in ["Edit", "Server", "Client"] {
                                match mcp.execute_luau(&studio.mcp_studio_id, &unify_code, datamodel_type).await {
                                    Ok(r) => tracing::debug!(datamodel_type, result = ?r, "execute_luau ok"),
                                    Err(e) => tracing::trace!(datamodel_type, "execute_luau skipped: {e}"),
                                }
                            }
                        }
                    }
                    Err(e) => {
                        tracing::debug!("list_studios failed: {e}");
                    }
                }
            }
        }

        let delay = if has_unresolved { 1 } else { 5 };
        tokio::time::sleep(std::time::Duration::from_secs(delay)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shared::target::{DomKind, RouteSpec, RunContext, StudioMode};
    use tokio::sync::{mpsc, watch};

    fn edit_dom(dom_id: &str, session_guid: Option<&str>) -> rodeo_proto::DomSnapshot {
        rodeo_proto::DomSnapshot {
            dom_id: dom_id.to_string(),
            mode: Some("edit".to_string()),
            dom_kind: Some("edit".to_string()),
            session_guid: session_guid.map(|s| s.to_string()),
            connected: true,
            ..Default::default()
        }
    }

    /// A connected DOM of studio `studio` (its canonical studio_id).
    fn dom(dom_id: &str, kind: &str, mode: &str, studio: &str) -> rodeo_proto::DomSnapshot {
        rodeo_proto::DomSnapshot {
            dom_id: dom_id.to_string(),
            mode: Some(mode.to_string()),
            dom_kind: Some(kind.to_string()),
            studio_id: Some(studio.to_string()),
            connected: true,
            ..Default::default()
        }
    }

    fn route(mode: Option<StudioMode>, dom_kind: Option<DomKind>, context: Option<RunContext>) -> RouteSpec {
        RouteSpec { mode, dom_kind, context }
    }

    fn queued_run(route: RouteSpec) -> (RunRequest, mpsc::UnboundedReceiver<ClientMsg>) {
        queued_run_for("exec-1", route, None, 0.0)
    }

    fn queued_run_for(
        id: &str,
        route: RouteSpec,
        session: Option<&str>,
        created_at: f64,
    ) -> (RunRequest, mpsc::UnboundedReceiver<ClientMsg>) {
        let (client_tx, client_rx) = mpsc::unbounded_channel();
        let run = RunRequest {
            execution_id: id.to_string(),
            script: String::new(),
            route,
            session: session.map(|s| s.to_string()),
            dom_id: None,
            log_filter: rodeo_proto::LogFilter::default(),
            reload_requires: None,
            script_args: None,
            return_file: None,
            show_return: None,
            output_file: None,
            verbose: None,
            instance_path: None,
            script_path: None,
            profile: None,
            client_tx,
            state: rodeo_proto::ProcessState::PROCESS_STATE_QUEUED,
            created_at,
        };
        (run, client_rx)
    }

    fn state_with_doms(
        doms: Vec<rodeo_proto::DomSnapshot>,
    ) -> (MasterState, mpsc::UnboundedReceiver<rodeo_proto::MasterMessage>) {
        let mut state = MasterState::new("master-test".to_string());
        let (backend_tx, backend_rx) = mpsc::unbounded_channel();
        let (state_tx, state_rx) = watch::channel(rodeo_proto::StateSnapshot {
            doms,
            ..Default::default()
        });
        state.backends.insert(
            "backend-1".to_string(),
            grpc::BackendConnection {
                id: "backend-1".to_string(),
                kind: "studio".to_string(),
                name: "test-studio".to_string(),
                tx: backend_tx,
                state_tx,
                state_rx,
            },
        );
        (state, backend_rx)
    }

    fn state_with_edit_dom(
        dom_id: &str,
        session_guid: Option<&str>,
    ) -> (MasterState, mpsc::UnboundedReceiver<rodeo_proto::MasterMessage>) {
        state_with_doms(vec![edit_dom(dom_id, session_guid)])
    }

    /// Replace the backend's DOM snapshot (a DOM connected or went away).
    fn set_doms(state: &MasterState, doms: Vec<rodeo_proto::DomSnapshot>) {
        let backend = state.backends.get("backend-1").expect("test backend");
        let _ = backend.state_tx.send(rodeo_proto::StateSnapshot { doms, ..Default::default() });
    }

    /// What the master sent to DOMs, in a stable order.
    #[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
    enum Sent {
        /// (dom, target_mode, num_players)
        Target(String, String, u32),
        /// (dom, execution_id)
        Run(String, String),
    }

    fn drain(rx: &mut mpsc::UnboundedReceiver<rodeo_proto::MasterMessage>) -> Vec<Sent> {
        let mut out = Vec::new();
        while let Ok(msg) = rx.try_recv() {
            let Some(rodeo_proto::master_message::Msg::DomServerMessage(dsm)) = msg.msg else { continue };
            let dsm = *dsm;
            let dom = dsm.dom_id.clone();
            match dsm.message.into_option().and_then(|m| m.msg) {
                Some(rodeo_proto::server_message::Msg::SetTargetMode(t)) => {
                    out.push(Sent::Target(dom, t.target_mode.clone(), t.num_players))
                }
                Some(rodeo_proto::server_message::Msg::Run(r)) => out.push(Sent::Run(dom, r.execution_id.clone())),
                _ => {}
            }
        }
        out.sort();
        out
    }

    fn target(dom: &str, mode: &str, n: u32) -> Sent {
        Sent::Target(dom.to_string(), mode.to_string(), n)
    }

    fn disconnect_reason(rx: &mut mpsc::UnboundedReceiver<ClientMsg>) -> Option<String> {
        while let Ok(msg) = rx.try_recv() {
            if let ClientMsg::Disconnect(reason) = msg {
                return Some(reason);
            }
        }
        None
    }

    // Regression: a manually-installed plugin reports SESSION_GUID=nil, so its
    // edit DOM registers with no session_guid. derive_and_push_targets used to
    // skip session-less DOMs entirely, so the master never pushed SetTargetMode
    // to a hand-opened Studio — auto-transition never fired and the run hung in
    // pending_runs forever. A session-less edit DOM must still be driven for a
    // session-less (`session: None`) run.
    #[test]
    fn session_less_edit_dom_is_driven_for_a_session_less_run() {
        let (mut state, mut backend_rx) = state_with_edit_dom("edit-dom", None);
        let (run, _client_rx) = queued_run(route(Some(StudioMode::Test), Some(DomKind::Server), None));
        state.pending_runs.push(run);

        state.derive_and_push_targets();

        // Session-less DOM is keyed by its own dom_id and gets the "test" target.
        assert_eq!(
            state.target_modes.get("edit-dom"),
            Some(&StudioTarget { mode: "test".to_string(), num_players: 0, doms: vec!["edit-dom".to_string()] }),
            "session-less edit DOM must be included in target derivation"
        );
        // And an actual SetTargetMode push must be dispatched to its backend.
        assert_eq!(drain(&mut backend_rx), vec![target("edit-dom", "test", 0)]);
    }

    // Control: a session-bearing edit DOM keeps being keyed by its session.
    #[test]
    fn session_bearing_edit_dom_is_driven_by_session() {
        let (mut state, _backend_rx) = state_with_edit_dom("edit-dom", Some("sess-A"));
        let (run, _client_rx) = queued_run(route(Some(StudioMode::Test), Some(DomKind::Server), None));
        state.pending_runs.push(run);

        state.derive_and_push_targets();

        assert_eq!(
            state.target_modes.get("sess-A").map(|t| t.mode.as_str()),
            Some("test"),
            "session-bearing DOM is keyed by its session_guid"
        );
    }

    fn play_dom(dom_id: &str, kind: Option<&str>) -> rodeo_proto::DomSnapshot {
        rodeo_proto::DomSnapshot {
            dom_id: dom_id.to_string(),
            mode: Some("play".to_string()),
            dom_kind: kind.map(|k| k.to_string()),
            session_guid: Some("sess-A".to_string()),
            connected: true,
            ..Default::default()
        }
    }

    fn pinned_run(dom_id: &str, context: Option<crate::shared::target::RunContext>) -> (RunRequest, mpsc::UnboundedReceiver<ClientMsg>) {
        let (mut run, client_rx) = queued_run(crate::shared::target::RouteSpec { mode: None, dom_kind: None, context });
        run.dom_id = Some(dom_id.to_string());
        (run, client_rx)
    }

    /// The context of the first Run command sent to a DOM, if any.
    fn dispatched_context(backend_rx: &mut mpsc::UnboundedReceiver<rodeo_proto::MasterMessage>) -> Option<(String, String)> {
        while let Ok(msg) = backend_rx.try_recv() {
            if let Some(rodeo_proto::master_message::Msg::DomServerMessage(m)) = msg.msg {
                if let Some(rodeo_proto::server_message::Msg::Run(run)) = m.message.into_option().and_then(|s| s.msg) {
                    return Some((m.dom_id, run.context));
                }
            }
        }
        None
    }

    // Issue #14: a run pinned to a play client or server DOM at that DOM's
    // own context dispatches there.
    #[test]
    fn pinned_run_dispatches_to_a_play_dom_at_its_context() {
        use crate::shared::target::RunContext as C;
        for (kind, context) in [("client", C::Client), ("server", C::Server), ("client", C::Plugin), ("server", C::Elevated)] {
            let (mut state, mut backend_rx) = state_with_edit_dom("edit-dom", Some("sess-A"));
            state.backends.get("backend-1").unwrap().state_tx.send_modify(|s| s.doms.push(play_dom("play-dom", Some(kind))));
            let (run, _client_rx) = pinned_run("play-dom", Some(context));

            state.route_or_queue(run);

            assert!(state.active_runs.contains_key("exec-1"), "{context:?} on {kind}: not dispatched");
            assert_eq!(
                dispatched_context(&mut backend_rx),
                Some(("play-dom".to_string(), context.as_str().to_string())),
                "{context:?} on {kind}"
            );
        }
    }

    // A context the pinned DOM can't host is final (a DOM's kind never
    // changes): the run ends with the reason instead of queueing forever.
    #[test]
    fn pinned_run_at_a_context_the_dom_cannot_host_is_rejected() {
        use crate::shared::target::RunContext as C;
        for (dom, kind, context) in [("play-dom", "client", C::Server), ("play-dom", "server", C::Client), ("edit-dom", "edit", C::Client), ("play-dom", "client", C::Cmdbar)] {
            let (mut state, mut backend_rx) = state_with_edit_dom("edit-dom", Some("sess-A"));
            state.backends.get("backend-1").unwrap().state_tx.send_modify(|s| s.doms.push(play_dom("play-dom", Some("client"))));
            state.backends.get("backend-1").unwrap().state_tx.send_modify(|s| {
                for d in s.doms.iter_mut().filter(|d| d.dom_id == dom) { d.dom_kind = Some(kind.to_string()); }
            });
            let (run, mut client_rx) = pinned_run(dom, Some(context));

            state.route_or_queue(run);

            assert!(state.active_runs.is_empty() && state.pending_runs.is_empty(), "{context:?} on {kind}");
            assert!(dispatched_context(&mut backend_rx).is_none(), "{context:?} on {kind}");
            match client_rx.try_recv() {
                Ok(ClientMsg::Disconnect(reason)) => {
                    let expected = format!("invalid route: context {} cannot run on the pinned DOM, which is a {kind} DOM", context.as_str());
                    assert!(reason.starts_with(&expected), "{reason}");
                }
                _ => panic!("{context:?} on {kind}: expected a Disconnect"),
            }
            // The run was dropped, so the client's stream ends.
            assert!(matches!(client_rx.try_recv(), Err(mpsc::error::TryRecvError::Disconnected)));
        }
    }

    // A pinned DOM whose kind isn't reported yet: a kind-dependent context
    // waits for it; plugin (fits every DOM) dispatches at once as before.
    #[test]
    fn pinned_run_waits_for_an_unreported_dom_kind() {
        use crate::shared::target::RunContext as C;
        let (mut state, mut backend_rx) = state_with_edit_dom("edit-dom", Some("sess-A"));
        state.backends.get("backend-1").unwrap().state_tx.send_modify(|s| s.doms.push(play_dom("play-dom", None)));

        let (run, _client_rx) = pinned_run("play-dom", Some(C::Client));
        state.route_or_queue(run);
        assert_eq!(state.pending_runs.len(), 1);
        assert!(dispatched_context(&mut backend_rx).is_none());

        state.backends.get("backend-1").unwrap().state_tx.send_modify(|s| {
            for d in s.doms.iter_mut().filter(|d| d.dom_id == "play-dom") { d.dom_kind = Some("client".to_string()); }
        });
        state.process_pending();
        assert!(state.pending_runs.is_empty());
        assert_eq!(dispatched_context(&mut backend_rx), Some(("play-dom".to_string(), "client".to_string())));

        let (mut state, mut backend_rx) = state_with_edit_dom("edit-dom", Some("sess-A"));
        state.backends.get("backend-1").unwrap().state_tx.send_modify(|s| s.doms.push(play_dom("play-dom", None)));
        let (run, _client_rx) = pinned_run("play-dom", None);
        state.route_or_queue(run);
        assert_eq!(dispatched_context(&mut backend_rx), Some(("play-dom".to_string(), "plugin".to_string())));
    }

    // #39 item 3: `--mode edit` used to run on the edit DOM and return while
    // the session kept running. It now waits for the studio to be in edit and
    // drives the transition there (the server DOM's EndTest).
    #[test]
    fn explicit_edit_mode_ends_the_session_first() {
        let (mut state, mut backend_rx) = state_with_doms(vec![
            dom("edit", "edit", "edit", "S"),
            dom("server", "server", "test", "S"),
            dom("client", "client", "test", "S"),
        ]);
        let (run, _client_rx) = queued_run_for("r-edit", route(Some(StudioMode::Edit), None, None), Some("S"), 1.0);

        assert!(!state.route_or_queue(run), "held while the studio is in a session");
        assert_eq!(
            drain(&mut backend_rx),
            vec![target("client", "edit", 0), target("edit", "edit", 0), target("server", "edit", 0)],
        );

        // The session ended: its DOMs are gone.
        set_doms(&state, vec![dom("edit", "edit", "edit", "S")]);
        state.reconcile();
        assert_eq!(
            drain(&mut backend_rx),
            vec![target("edit", "", 0), Sent::Run("edit".to_string(), "r-edit".to_string())],
        );
        assert!(state.pending_runs.is_empty());
    }

    // An omitted --mode resolves to edit but still means "the edit DOM, which
    // exists in every mode": it must not end the session.
    #[test]
    fn omitted_mode_runs_on_the_edit_dom_without_ending_the_session() {
        let (mut state, mut backend_rx) = state_with_doms(vec![
            dom("edit", "edit", "edit", "S"),
            dom("server", "server", "test", "S"),
        ]);
        let (run, _client_rx) = queued_run_for("r-plugin", route(None, None, Some(RunContext::Plugin)), Some("S"), 1.0);

        assert!(state.route_or_queue(run));
        assert_eq!(drain(&mut backend_rx), vec![Sent::Run("edit".to_string(), "r-plugin".to_string())]);
    }

    // #39: a play target starts a multiplayer test, with a client when a
    // queued run needs one. (It used to start a solo test, which reports
    // "test" and never satisfies a play run.)
    #[test]
    fn play_target_carries_the_client_count() {
        let (mut state, mut backend_rx) = state_with_doms(vec![
            dom("edit", "edit", "edit", "S"),
            dom("server", "server", "test", "S"),
            dom("client", "client", "test", "S"),
        ]);
        let (server_run, _rx1) = queued_run_for("r-server", route(Some(StudioMode::Play), None, Some(RunContext::Server)), Some("S"), 1.0);
        assert!(!state.route_or_queue(server_run));
        assert_eq!(
            drain(&mut backend_rx),
            vec![target("client", "play", 0), target("edit", "play", 0), target("server", "play", 0)],
        );

        let (client_run, _rx2) = queued_run_for("r-client", route(Some(StudioMode::Play), Some(DomKind::Client), None), Some("S"), 2.0);
        assert!(!state.route_or_queue(client_run));
        assert_eq!(
            drain(&mut backend_rx),
            vec![target("client", "play", 1), target("edit", "play", 1), target("server", "play", 1)],
        );
    }

    // The plugin treats a re-sent target as a no-op, but the master still
    // re-sends it when the DOM set changes so a newly connected DOM (here the
    // run session's server, which the test target must end) learns it.
    #[test]
    fn unchanged_target_is_re_pushed_only_when_the_dom_set_changes() {
        let (mut state, mut backend_rx) = state_with_doms(vec![dom("edit", "edit", "edit", "S")]);
        let (run, _client_rx) = queued_run_for("r-test", route(Some(StudioMode::Test), None, Some(RunContext::Server)), Some("S"), 1.0);
        assert!(!state.route_or_queue(run));
        assert_eq!(drain(&mut backend_rx), vec![target("edit", "test", 0)]);

        state.reconcile();
        assert_eq!(drain(&mut backend_rx), vec![], "same target, same DOMs: nothing re-sent");

        set_doms(&state, vec![dom("edit", "edit", "edit", "S"), dom("server", "server", "run", "S")]);
        state.reconcile();
        assert_eq!(drain(&mut backend_rx), vec![target("edit", "test", 0), target("server", "test", 0)]);
    }

    // #27/#39: when the edit DOM gives up on a transition, the runs waiting on
    // it fail with the plugin's reason instead of staying queued forever.
    // Runs for another mode, or pinned to another studio, keep waiting.
    #[test]
    fn failed_transition_fails_only_the_runs_waiting_on_it() {
        let (mut state, mut backend_rx) = state_with_doms(vec![
            dom("edit-s", "edit", "edit", "S"),
            dom("edit-t", "edit", "edit", "T"),
        ]);
        let (test_s, mut test_s_rx) = queued_run_for("test-s", route(Some(StudioMode::Test), None, Some(RunContext::Client)), Some("S"), 1.0);
        let (run_s, mut run_s_rx) = queued_run_for("run-s", route(Some(StudioMode::Run), None, Some(RunContext::Server)), Some("S"), 2.0);
        let (test_t, mut test_t_rx) = queued_run_for("test-t", route(Some(StudioMode::Test), None, Some(RunContext::Server)), Some("T"), 3.0);
        let (test_any, mut test_any_rx) = queued_run_for("test-any", route(Some(StudioMode::Test), None, Some(RunContext::Server)), None, 4.0);
        for run in [test_s, run_s, test_t, test_any] {
            assert!(!state.route_or_queue(run));
        }
        drain(&mut backend_rx);

        let engine = "StudioTestService:ExecutePlayModeAsync failed 5 times: Failed to start the test because a previous one is still in progress.";
        state.fail_mode_transition("edit-s", "test", engine);

        let reason = disconnect_reason(&mut test_s_rx).expect("the test run on S fails");
        assert_eq!(reason, format!("Studio S could not enter test mode: {engine}"));
        // A session-less run drives every studio, this one included.
        assert!(disconnect_reason(&mut test_any_rx).is_some());
        assert!(disconnect_reason(&mut run_s_rx).is_none());
        assert!(disconnect_reason(&mut test_t_rx).is_none());
        let mut pending: Vec<&str> = state.pending_runs.iter().map(|r| r.execution_id.as_str()).collect();
        pending.sort();
        assert_eq!(pending, vec!["run-s", "test-t"]);

        // S moves on to the next queued run's mode.
        assert_eq!(drain(&mut backend_rx), vec![target("edit-s", "run", 0)]);
    }

    // A report racing a newer target is stale: the plugin already works on
    // the new target, so nothing is failed.
    #[test]
    fn stale_transition_failure_is_ignored() {
        let (mut state, _backend_rx) = state_with_doms(vec![dom("edit", "edit", "edit", "S")]);
        let (run, mut client_rx) = queued_run_for("r-test", route(Some(StudioMode::Test), None, Some(RunContext::Server)), Some("S"), 1.0);
        assert!(!state.route_or_queue(run));

        state.fail_mode_transition("edit", "run", "late report");

        assert!(disconnect_reason(&mut client_rx).is_none());
        assert_eq!(state.pending_runs.len(), 1);
    }

    // After a failure the studio's target is cleared and pushed (the plugin
    // stops), so a run queued later for the same mode re-sends it and the
    // plugin, which gave up, starts a fresh attempt.
    #[test]
    fn same_mode_after_a_failure_is_pushed_again() {
        let (mut state, mut backend_rx) = state_with_doms(vec![dom("edit", "edit", "edit", "S")]);
        let (first, _rx1) = queued_run_for("first", route(Some(StudioMode::Test), None, Some(RunContext::Server)), Some("S"), 1.0);
        assert!(!state.route_or_queue(first));
        drain(&mut backend_rx);
        state.fail_mode_transition("edit", "test", "gave up");
        assert_eq!(drain(&mut backend_rx), vec![target("edit", "", 0)]);

        let (second, _rx2) = queued_run_for("second", route(Some(StudioMode::Test), None, Some(RunContext::Server)), Some("S"), 2.0);
        assert!(!state.route_or_queue(second));
        assert_eq!(drain(&mut backend_rx), vec![target("edit", "test", 0)]);
    }

    // SetStudioMode targets drive the studio until it gets there, then clear,
    // so the edit DOM isn't left working on a target nobody waits for.
    #[test]
    fn explicit_target_holds_until_reached() {
        let mut edit = dom("edit", "edit", "edit", "S");
        edit.session_guid = Some("sess-S".to_string());
        let (mut state, mut backend_rx) = state_with_doms(vec![edit.clone()]);

        state.set_target_mode("sess-S", "run").expect("studio is connected");
        assert_eq!(drain(&mut backend_rx), vec![target("edit", "run", 0)]);
        state.reconcile();
        assert_eq!(drain(&mut backend_rx), vec![], "explicit target persists");

        set_doms(&state, vec![edit, dom("server", "server", "run", "S")]);
        state.reconcile();
        assert_eq!(drain(&mut backend_rx), vec![target("edit", "", 0), target("server", "", 0)]);
        assert!(state.explicit_targets.is_empty());
    }

    #[test]
    fn explicit_target_for_the_current_mode_pushes_nothing() {
        let (mut state, mut backend_rx) = state_with_doms(vec![dom("edit", "edit", "edit", "S")]);
        state.set_target_mode("S", "edit").expect("studio is connected");
        assert_eq!(drain(&mut backend_rx), vec![]);
        assert!(state.explicit_targets.is_empty());

        assert!(state.set_target_mode("nope", "run").is_err(), "an unknown studio is an error");
    }
}
