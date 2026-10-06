use std::collections::HashMap;
use tokio::sync::mpsc;

use rodeo_proto::{self as proto, ProcessState, StudioStateMsg};

/// A script execution request — single source of truth for a run's lifecycle.
#[derive(Debug, Clone)]
pub struct RunRequest {
    pub execution_id: String,
    pub script: String,
    /// Routing spec (sparse, as submitted). Routed runs are validated via
    /// `resolve()` at submit time. Pinned runs (`dom_id`) carry at most a
    /// context, which `resolve()` may reject (it assumes edit mode); the
    /// context is checked against the pinned DOM's kind at dispatch.
    pub route: crate::shared::target::RouteSpec,
    /// Studio filter — restricts route matching to one studio. Matches a DOM
    /// whose launch `session_guid` (owned-studio launch pin, e.g. `run --place`)
    /// OR canonical `studio_id` (`--studio-id`, Studio.runCode) equals this.
    pub session: Option<String>,
    /// Direct DOM targeting (bypasses route matching; only `route.context`
    /// applies)
    pub dom_id: Option<String>,
    pub log_filter: proto::LogFilter,
    pub reload_requires: Option<bool>,
    pub script_args: Option<Vec<String>>,
    pub return_file: Option<String>,
    pub show_return: Option<bool>,
    pub output_file: Option<String>,
    pub verbose: Option<bool>,
    pub instance_path: Option<String>,
    pub script_path: Option<String>,
    /// Whether this run has profiling enabled
    pub profile: Option<bool>,
    /// Channel to send output back to the run client
    pub client_tx: mpsc::UnboundedSender<crate::master::ClientMsg>,

    // Process metadata
    pub state: ProcessState,
    pub created_at: f64,
}

/// Produce a human-readable diff between two StudioStateMsg snapshots.
/// Returns None if nothing meaningful changed.
pub fn diff_state(new: &StudioStateMsg, old: &StudioStateMsg) -> Option<String> {
    let mut changes = Vec::new();

    fn opt(s: &str) -> &str { if s.is_empty() { "?" } else { s } }

    if new.mode != old.mode {
        changes.push(format!("mode: {} → {}", opt(&old.mode), opt(&new.mode)));
    }
    if new.dom_kind != old.dom_kind {
        changes.push(format!("domKind: {} → {}", opt(&old.dom_kind), opt(&new.dom_kind)));
    }
    if new.mcp_studio_id != old.mcp_studio_id {
        let short = |s: &Option<String>| s.as_deref().map(|v| &v[..8.min(v.len())]).unwrap_or("none").to_string();
        changes.push(format!("mcp_studio_id: {} → {}", short(&old.mcp_studio_id), short(&new.mcp_studio_id)));
    }
    if new.is_server != old.is_server {
        changes.push(format!("is_server: {} → {}", old.is_server, new.is_server));
    }
    if new.is_client != old.is_client {
        changes.push(format!("is_client: {} → {}", old.is_client, new.is_client));
    }
    if new.is_running != old.is_running {
        changes.push(format!("is_running: {} → {}", old.is_running, new.is_running));
    }
    if new.is_edit != old.is_edit {
        changes.push(format!("is_edit: {} → {}", old.is_edit, new.is_edit));
    }

    if changes.is_empty() { None } else { Some(changes.join(", ")) }
}

/// A Studio instance, derived from connected DOMs grouped by session_guid.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StudioInstance {
    /// Master-minted session identity. Same value as the `session_guid` stamped
    /// on each DOM belonging to this Studio.
    pub session_guid: String,
    /// "edit" | "run" | "test" | "play"
    pub mode: String,
    pub name: String,
    pub place_id: i64,
    pub universe_id: i64,
    pub edit_dom: Option<String>,
    pub server_dom: Option<String>,
    pub client_doms: Vec<String>,
}

/// Per-DOM connection state, keyed by domId.
pub struct DomConnection {
    #[allow(dead_code)]
    pub dom_id: String,
    pub studio_tx: mpsc::UnboundedSender<String>,
    pub connected: bool,
    pub active_runs: HashMap<String, RunRequest>,
    /// Canonicalized state from the plugin (updated via studio_state messages)
    pub state: Option<StudioStateMsg>,
    /// Master-minted session identity, stamped on the DOM when the plugin
    /// connects to a launched `StudioInstance`. This is the single routing key
    /// across master, backends, and clients (replaces the old dual
    /// `canonical_studio_id` + `session_uid` identities).
    pub session_guid: Option<String>,
}

impl DomConnection {
    pub fn new(dom_id: String, studio_tx: mpsc::UnboundedSender<String>) -> Self {
        Self {
            dom_id,
            studio_tx,
            connected: true,
            active_runs: HashMap::new(),
            state: None,
            session_guid: None,
        }
    }

    /// Update the DOM state from a studio_state message.
    /// Returns a diff string if anything meaningful changed.
    pub fn update_state(&mut self, new_state: StudioStateMsg) -> Option<String> {
        let diff = self.state.as_ref().and_then(|old| diff_state(&new_state, old));
        self.state = Some(new_state);
        diff
    }

    /// Start a run on this DOM
    pub fn start_run(&mut self, mut run: RunRequest) {
        // Pinned runs (dom_id) carry at most a context; routed runs resolve
        // through the defaults table. Either way the plugin receives only the
        // run context — mode/dom-kind were consumed when this DOM was picked.
        let context = run
            .route
            .context
            .or_else(|| run.route.resolve().ok().map(|r| r.context))
            .unwrap_or(crate::shared::target::RunContext::Plugin);

        let run_cmd = proto::ServerMessage {
            msg: Some(proto::server_message::Msg::Run(Box::new(
                proto::RunCommand {
                    execution_id: run.execution_id.clone(),
                    script: run.script.clone(),
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
                    ..Default::default()
                },
            ))),
            ..Default::default()
        };

        let msg = serde_json::to_string(&run_cmd).unwrap();

        tracing::info!(id = run.execution_id.as_str(), context = context.as_str(), "executing");

        run.state = ProcessState::PROCESS_STATE_RUNNING;
        let _ = self.studio_tx.send(msg);
        self.active_runs
            .insert(run.execution_id.clone(), run);
    }

    /// Complete a run, removing it from active_runs and returning it.
    pub fn complete_run(&mut self, execution_id: &str, state: &ProcessState) -> Option<RunRequest> {
        if let Some(run) = self.active_runs.remove(execution_id) {
            tracing::info!(id = execution_id, state = crate::master::grpc::process_state_str(state), "completed");
            Some(run)
        } else {
            None
        }
    }

    /// Mark a run as done without removing it (keeps client_tx alive for file transfers).
    /// Returns true if the run was found and marked.
    pub fn mark_done(&mut self, execution_id: &str, state: &ProcessState) -> bool {
        if let Some(run) = self.active_runs.get_mut(execution_id) {
            tracing::info!(id = execution_id, state = crate::master::grpc::process_state_str(state), "completed (draining files)");
            run.state = state.clone();
            true
        } else {
            false
        }
    }

    /// Forward a typed RPC call to the run client. Returns true if the execution was found.
    pub fn forward_rpc_call(&self, execution_id: &str, call: rodeo_proto::runtime_types::ClientRpcCall) -> bool {
        if let Some(run) = self.active_runs.get(execution_id) {
            let _ = run.client_tx.send(crate::master::ClientMsg::RpcCall(Box::new(call)));
            true
        } else {
            false
        }
    }

    /// Forward a typed ExecutionDone to the run client.
    pub fn forward_execution_done(&self, execution_id: &str, done: rodeo_proto::ExecutionDone) -> bool {
        if let Some(run) = self.active_runs.get(execution_id) {
            let _ = run.client_tx.send(crate::master::ClientMsg::ExecutionDone(Box::new(done)));
            true
        } else {
            false
        }
    }

    /// Forward a typed ExecutionKilled to the run client.
    pub fn forward_execution_killed(&self, execution_id: &str, killed: rodeo_proto::ExecutionKilled) -> bool {
        if let Some(run) = self.active_runs.get(execution_id) {
            let _ = run.client_tx.send(crate::master::ClientMsg::ExecutionKilled(Box::new(killed)));
            true
        } else {
            false
        }
    }

    /// Handle Studio disconnection
    pub fn disconnect(&mut self) {
        self.connected = false;

        for (eid, run) in &self.active_runs {
            let _ = run.client_tx.send(crate::master::ClientMsg::Disconnect(format!("studio disconnected (eid={eid})")));
        }

        self.active_runs.clear();
    }

    pub fn active_count(&self) -> usize {
        self.active_runs.len()
    }

    /// Get the mode from DOM state, if available.
    pub fn mode(&self) -> Option<&str> {
        self.state.as_ref().map(|s| if s.mode.is_empty() { None } else { Some(s.mode.as_str()) }).flatten()
    }

    /// Get the dom from DOM state, if available.
    pub fn dom_kind(&self) -> Option<&str> {
        self.state.as_ref().map(|s| if s.dom_kind.is_empty() { None } else { Some(s.dom_kind.as_str()) }).flatten()
    }

    /// The canonical plugin studio id, if this DOM has reported its state.
    pub fn studio_id(&self) -> Option<&str> {
        self.state.as_ref().map(|s| if s.studio_id.is_empty() { None } else { Some(s.studio_id.as_str()) }).flatten()
    }

    /// Does this DOM belong to the studio identified by `wanted`? A run's
    /// studio filter matches on either the launch `session_guid` (owned-studio
    /// launch pin) or the canonical `studio_id` (`--studio-id`).
    pub fn matches_studio(&self, wanted: &str) -> bool {
        self.session_guid.as_deref() == Some(wanted) || self.studio_id() == Some(wanted)
    }
}
