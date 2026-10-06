use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{anyhow, bail, Result};

use crate::proto;
use crate::transport::Transport;
use crate::dom::Dom;

const DOM_POLL_INTERVAL_MS: u64 = 200;

/// Describe a wait failure: the deadline and what the studio actually looked
/// like at that moment, so the caller can tell "Studio never entered play"
/// from "no client DOM connected" without a separate state query.
fn wait_timeout_error(session_guid: &str, timeout: Duration, last: Option<&proto::StudioState>) -> anyhow::Error {
    let seen = match last {
        Some(s) => {
            let kinds: Vec<&str> = s.doms.iter().map(|v| v.dom_kind.as_str()).collect();
            format!("status={} mode={} doms=[{}]", s.status, s.studio_mode, kinds.join(","))
        }
        None => "studio not present in state".to_string(),
    };
    anyhow!(
        "timed out after {:?} waiting for studio instance {} to reach expected state ({seen})",
        timeout, session_guid
    )
}


// ---------------------------------------------------------------------------
// StudioBackend
// ---------------------------------------------------------------------------

/// Options for `StudioBackend::open` — empty place, edit mode.
#[derive(Default, Clone)]
pub struct OpenOpts {
    pub fflags: Vec<String>,
    pub background: bool,
    pub profile: bool,
    pub save: Option<String>,
    pub detached: bool,
    pub fflag_file: Option<String>,
    /// `--show-widgets` allow-list spec (`None` = normal; `Some("none")` = hide
    /// all; `Some("output,...")` = keep those).
    pub show_widgets: Option<String>,
    /// Bound on waiting for the edit DOM to connect after launch. `None`
    /// (the default) waits indefinitely.
    pub timeout: Option<Duration>,
}

/// Options for `StudioBackend::open_place` — open by place ID.
#[derive(Clone)]
pub struct OpenPlaceOpts {
    pub place_id: u64,
    pub fflags: Vec<String>,
    pub background: bool,
    pub profile: bool,
    pub save: Option<String>,
    pub detached: bool,
    pub fflag_file: Option<String>,
    /// `--show-widgets` allow-list spec (`None` = normal; `Some("none")` = hide
    /// all; `Some("output,...")` = keep those).
    pub show_widgets: Option<String>,
    /// Bound on waiting for the edit DOM to connect after launch. `None`
    /// waits indefinitely.
    pub timeout: Option<Duration>,
}

/// Options for `StudioBackend::open_file` — open by file path.
#[derive(Clone)]
pub struct OpenFileOpts {
    pub path: String,
    pub fflags: Vec<String>,
    pub background: bool,
    pub profile: bool,
    pub save: Option<String>,
    pub detached: bool,
    pub fflag_file: Option<String>,
    /// `--show-widgets` allow-list spec (`None` = normal; `Some("none")` = hide
    /// all; `Some("output,...")` = keep those).
    pub show_widgets: Option<String>,
    /// Bound on waiting for the edit DOM to connect after launch. `None`
    /// waits indefinitely.
    pub timeout: Option<Duration>,
}

#[derive(Clone)]
pub struct StudioBackend {
    pub id: String,
    pub name: String,
    transport: Arc<Transport>,
}

impl StudioBackend {
    pub(crate) fn new(info: proto::BackendInfo, transport: Arc<Transport>) -> Self {
        Self { id: info.id, name: info.name, transport }
    }

    pub async fn open(&self, opts: OpenOpts) -> Result<Studio> {
        self.launch(proto::LaunchStudioRequest {
            backend: self.id.clone(),
            fflags: opts.fflags,
            background: opts.background,
            detached: opts.detached,
            profile: opts.profile,
            save_path: opts.save,
            fflag_file: opts.fflag_file,
            show_widgets: opts.show_widgets.unwrap_or_default(),
            ..Default::default()
        }, opts.timeout).await
    }

    pub async fn open_place(&self, opts: OpenPlaceOpts) -> Result<Studio> {
        self.launch(proto::LaunchStudioRequest {
            backend: self.id.clone(),
            place_id: Some(opts.place_id),
            fflags: opts.fflags,
            background: opts.background,
            detached: opts.detached,
            profile: opts.profile,
            save_path: opts.save,
            fflag_file: opts.fflag_file,
            show_widgets: opts.show_widgets.unwrap_or_default(),
            ..Default::default()
        }, opts.timeout).await
    }

    pub async fn open_file(&self, opts: OpenFileOpts) -> Result<Studio> {
        // Resolve against THIS process's cwd before sending: the backend that
        // opens the file may run elsewhere and would resolve a relative path
        // against its own cwd (it now fails loudly rather than opening a blank
        // place, but the error should name the caller's intent, not the
        // backend's directory). Also fails fast here if the file is missing.
        let path = std::fs::canonicalize(&opts.path)
            .map_err(|e| anyhow!("place file '{}' not found: {e}", opts.path))?
            .to_string_lossy()
            .into_owned();
        self.launch(proto::LaunchStudioRequest {
            backend: self.id.clone(),
            place_file: Some(path),
            fflags: opts.fflags,
            background: opts.background,
            detached: opts.detached,
            profile: opts.profile,
            save_path: opts.save,
            fflag_file: opts.fflag_file,
            show_widgets: opts.show_widgets.unwrap_or_default(),
            ..Default::default()
        }, opts.timeout).await
    }

    /// Low-level launch — use when building the LaunchStudioRequest outside the
    /// typed OpenOpts/OpenPlaceOpts/OpenFileOpts helpers (e.g. for the CLI
    /// commands/run.rs orchestration). `timeout` bounds the wait for the edit
    /// DOM after the backend reports the Studio connected; `None` waits
    /// indefinitely.
    pub async fn launch(&self, req: proto::LaunchStudioRequest, timeout: Option<Duration>) -> Result<Studio> {
        let mut stream = self.transport.master()
            .launch_studio(req)
            .await
            .map_err(|e| anyhow!("failed to launch studio: {e}"))?;
        while let Ok(Some(view)) = stream.message().await {
            let event = view.to_owned_message();
            match event.event {
                Some(proto::launch_studio_event::Event::Ready(ready)) => {
                    let session_guid = ready.session_guid.clone();
                    let backend_id = ready.backend_id.clone();
                    let mut studio = Studio {
                        session_guid: session_guid.clone(),
                        backend_id,
                        transport: self.transport.clone(),
                        edit_dom: None,
                        server_dom: None,
                        client_dom: None,
                    };
                    // Invariant: when open resolves, the edit DOM is connected
                    // under this Studio instance (waited via the canonical
                    // studio-first state).
                    let inst = studio.wait_for_instance(
                        |s| s.doms.iter().any(|v| v.dom_kind == "edit"),
                        timeout,
                    ).await?;
                    studio.edit_dom = studio.dom_by_kind(&inst, "edit");
                    return Ok(studio);
                }
                Some(proto::launch_studio_event::Event::Error(err)) => {
                    bail!("studio launch failed: {}", err.message);
                }
                _ => {}
            }
        }
        bail!("launch stream ended without ready event")
    }
}

// ---------------------------------------------------------------------------
// Studio
// ---------------------------------------------------------------------------

pub struct Studio {
    pub session_guid: String,
    pub backend_id: String,
    transport: Arc<Transport>,
    /// Populated when open resolves.
    pub edit_dom: Option<Dom>,
    /// Populated by `set_mode("run"/"test"/"play")`.
    pub server_dom: Option<Dom>,
    /// Populated by `set_mode("test"/"play")`.
    pub client_dom: Option<Dom>,
}

impl Studio {
    /// Returns the edit DOM, panicking if `open` never resolved it (impossible
    /// unless constructed by hand). Matches the TS `editDom!` non-null assert.
    pub fn edit_dom(&self) -> &Dom {
        self.edit_dom.as_ref().expect("edit_dom must be set after open")
    }

    /// Execute code somewhere in THIS studio — the session-scoped tier. The
    /// master matches the opts' mode/dom_kind/context among this studio's
    /// DOMs (auto-transitioning its mode when needed) and picks one.
    pub async fn run_code(&self, mut opts: crate::run::RunCodeOpts) -> Result<crate::run::RunResult> {
        opts.session = Some(self.session_guid.clone());
        crate::run::run_buffered_routed(self.transport.clone(), opts).await
    }

    /// Streaming variant of [`Self::run_code`].
    pub async fn run_code_stream(&self, mut opts: crate::run::RunCodeOpts) -> Result<crate::run::RunStream> {
        opts.session = Some(self.session_guid.clone());
        crate::run::run_stream_routed(self.transport.clone(), opts).await
    }

    /// This Studio's canonical state from the master's studio-first snapshot
    /// (by `session_guid`), or None if not yet present.
    pub async fn instance_state(&self) -> Result<Option<proto::StudioState>> {
        let state = self.transport.master()
            .get_state(proto::GetStateRequest::default())
            .await
            .map_err(|e| anyhow!("get_state failed: {e}"))?
            .into_owned();
        Ok(state.studios.into_iter().find(|s| s.session_id.as_deref() == Some(self.session_guid.as_str())))
    }

    /// Poll the canonical studio-first state until this Studio satisfies `pred`.
    /// Replaces per-DOM polling — callers express the whole-instance condition
    /// they expect (e.g. "a server DOM and N client DOMs are present").
    ///
    /// `timeout` is opt-in: `None` polls until the predicate holds or the
    /// master becomes unreachable. Pass `Some` to fail after that long.
    pub async fn wait_for_instance<F>(&self, pred: F, timeout: Option<Duration>) -> Result<proto::StudioState>
    where
        F: Fn(&proto::StudioState) -> bool,
    {
        let deadline = timeout.map(|t| Instant::now() + t);
        loop {
            let current = self.instance_state().await?;
            if let Some(s) = current.as_ref() {
                if pred(s) {
                    return Ok(current.unwrap());
                }
            }
            if let (Some(deadline), Some(timeout)) = (deadline, timeout) {
                if Instant::now() >= deadline {
                    return Err(wait_timeout_error(&self.session_guid, timeout, current.as_ref()));
                }
            }
            tokio::time::sleep(Duration::from_millis(DOM_POLL_INTERVAL_MS)).await;
        }
    }

    /// Resolve the first DOM in `studio` with the given `dom` to a DOM handle.
    pub(crate) fn dom_by_kind(&self, studio: &proto::StudioState, dom: &str) -> Option<Dom> {
        studio.doms.iter()
            .find(|v| v.dom_kind == dom)
            .map(|v| Dom::from_studio_dom(studio, v, self.transport.clone()))
    }

    /// Transition this Studio to `mode` and wait for the member DOMs that mode
    /// implies to connect. `timeout` is opt-in: `None` waits indefinitely.
    pub async fn set_mode(&mut self, mode: &str, timeout: Option<Duration>) -> Result<()> {
        self.transport.master()
            .set_studio_mode(proto::SetStudioModeRequest {
                session_guid: self.session_guid.clone(),
                mode: mode.to_string(),
                ..Default::default()
            })
            .await
            .map_err(|e| anyhow!("set_studio_mode failed: {e}"))?;

        if mode == "edit" {
            self.server_dom = None;
            self.client_dom = None;
            return Ok(());
        }

        // Wait on the canonical studio state for the expected member DOMs to
        // appear. We gate on `dom` presence rather than per-DOM mode strings —
        // the studio reaching the requested config is what matters, and the
        // test/play distinction is a post-hoc reconciliation detail.
        let want_client = mode == "test" || mode == "play";
        let inst = self.wait_for_instance(
            move |s| {
                let has_server = s.doms.iter().any(|v| v.dom_kind == "server");
                let has_client = !want_client || s.doms.iter().any(|v| v.dom_kind == "client");
                has_server && has_client
            },
            timeout,
        ).await?;

        self.server_dom = self.dom_by_kind(&inst, "server");
        self.client_dom = if want_client { self.dom_by_kind(&inst, "client") } else { None };
        Ok(())
    }

    pub async fn get_mode(&self) -> Result<String> {
        let resp = self.transport.master()
            .set_studio_mode(proto::SetStudioModeRequest {
                session_guid: self.session_guid.clone(),
                mode: String::new(),
                ..Default::default()
            })
            .await
            .map_err(|e| anyhow!("get_mode failed: {e}"))?
            .into_owned();
        Ok(resp.mode)
    }

    pub async fn save(&self) -> Result<proto::SavePlaceResponse> {
        let resp = self.transport.master()
            .save_place(proto::SavePlaceRequest {
                backend: Some(String::new()),
                session_guid: Some(self.session_guid.clone()),
                ..Default::default()
            })
            .await
            .map_err(|e| anyhow!("save failed: {e}"))?
            .into_owned();
        if !resp.saved {
            bail!("save failed: {}", resp.error.as_deref().unwrap_or(""));
        }
        Ok(resp)
    }

    pub async fn close(&self) -> Result<()> {
        let mut stream = self.transport.master()
            .close_studio(proto::CloseStudioRequest {
                session_guid: self.session_guid.clone(),
                ..Default::default()
            })
            .await
            .map_err(|e| anyhow!("close failed: {e}"))?;
        while let Ok(Some(view)) = stream.message().await {
            let event = view.to_owned_message();
            if let Some(proto::close_studio_event::Event::Closed(_)) = event.event {
                return Ok(());
            }
        }
        Ok(())
    }

    pub async fn get_doms(&self) -> Result<Vec<Dom>> {
        match self.instance_state().await? {
            Some(s) => Ok(s.doms.iter()
                .map(|v| Dom::from_studio_dom(&s, v, self.transport.clone()))
                .collect()),
            None => Ok(Vec::new()),
        }
    }

    /// Start an in-Studio multiplayer test via
    /// `StudioTestService:ExecuteMultiplayerTestAsync` with `num_players` client
    /// DataModels. Runs the start snippet on the edit DOM (fire-and-forget — the
    /// API yields for the session's life), then waits on the canonical studio
    /// state for the server + N client DOMs to register. `timeout` is opt-in:
    /// `None` waits indefinitely.
    pub async fn start_multiplayer_test(&self, num_players: u32, timeout: Option<Duration>) -> Result<MultiplayerTest> {
        let edit = self.edit_dom.as_ref()
            .ok_or_else(|| anyhow!("start_multiplayer_test requires an open edit Studio"))?;

        // `rodeoSession` tags the session's kind, as the plugin's mode driver
        // does for the sessions it starts: the play server reads it back
        // through GetTestArgs, so it knows to end this session for a `test`
        // target even with no client to report the kind.
        let snippet = format!(
            "local sts = game:GetService(\"StudioTestService\")\n\
             task.spawn(function()\n\
             \tsts:ExecuteMultiplayerTestAsync({num_players}, {{ rodeo = true, rodeoSession = \"play\" }})\n\
             end)\n\
             return \"started\""
        );
        edit.run_code(crate::run::RunCodeOpts { source: snippet, ..Default::default() }).await?;

        let n = num_players as usize;
        let inst = self.wait_for_instance(
            move |s| {
                s.doms.iter().any(|v| v.dom_kind == "server")
                    && s.doms.iter().filter(|v| v.dom_kind == "client").count() == n
            },
            timeout,
        ).await?;

        let server = self.dom_by_kind(&inst, "server")
            .ok_or_else(|| anyhow!("multiplayer test started but no server DOM appeared"))?;
        let clients: Vec<Dom> = inst.doms.iter()
            .filter(|v| v.dom_kind == "client")
            .map(|v| Dom::from_studio_dom(&inst, v, self.transport.clone()))
            .collect();

        Ok(MultiplayerTest {
            server,
            clients,
            session_guid: self.session_guid.clone(),
            transport: self.transport.clone(),
        })
    }
}

// ---------------------------------------------------------------------------
// MultiplayerTest — a running in-Studio multiplayer test (one server DataModel +
// N client DataModels, all under the edit Studio's session). Control is just
// `run_code` against the right DOM (StudioTestService:AddPlayers/EndTest/LeaveTest).
// ---------------------------------------------------------------------------

pub struct MultiplayerTest {
    pub server: Dom,
    pub clients: Vec<Dom>,
    session_guid: String,
    transport: Arc<Transport>,
}

impl MultiplayerTest {
    pub fn server(&self) -> &Dom { &self.server }
    pub fn clients(&self) -> &[Dom] { &self.clients }

    /// Connect one more client DataModel (`StudioTestService:AddPlayers(1)` on
    /// the server), wait for it to register, and return its Dom handle.
    /// `timeout` is opt-in: `None` waits indefinitely.
    pub async fn connect_client(&mut self, timeout: Option<Duration>) -> Result<Dom> {
        let known: std::collections::HashSet<String> =
            self.clients.iter().map(|v| v.dom_id.clone()).collect();
        self.server.run_code(crate::run::RunCodeOpts {
            source: "game:GetService(\"StudioTestService\"):AddPlayers(1)\nreturn true".to_string(),
            ..Default::default()
        }).await?;
        let known_pred = known.clone();
        let inst = self.wait_for_instance(move |s| {
            s.doms.iter().any(|v| v.dom_kind == "client" && !known_pred.contains(&v.dom_id))
        }, timeout).await?;
        self.clients = inst.doms.iter()
            .filter(|v| v.dom_kind == "client")
            .map(|v| Dom::from_studio_dom(&inst, v, self.transport.clone()))
            .collect();
        self.clients.iter()
            .find(|v| !known.contains(&v.dom_id))
            .cloned()
            .ok_or_else(|| anyhow!("new client DOM did not appear in studio state"))
    }

    /// Disconnect one client (`StudioTestService:LeaveTest` on that client DOM),
    /// identified by its domId.
    pub async fn disconnect_client(&mut self, dom_id: &str) -> Result<()> {
        let index = self.clients.iter().position(|v| v.dom_id == dom_id)
            .ok_or_else(|| anyhow!("no client with domId {dom_id} in this test"))?;
        self.clients[index].run_code(crate::run::RunCodeOpts {
            source: "local s = game:GetService(\"StudioTestService\")\n\
                     if s:CanLeaveTest() then s:LeaveTest() end\nreturn true".to_string(),
            ..Default::default()
        }).await?;
        self.clients.remove(index);
        Ok(())
    }

    /// End the whole session (`StudioTestService:EndTest` on the server).
    pub async fn close(&self) -> Result<()> {
        self.server.run_code(crate::run::RunCodeOpts {
            source: "game:GetService(\"StudioTestService\"):EndTest(\"end\")\nreturn true".to_string(),
            ..Default::default()
        }).await?;
        Ok(())
    }

    /// Poll this test's studio (by session_guid) until `pred` holds. `timeout`
    /// is opt-in: `None` polls until the predicate holds.
    async fn wait_for_instance<F>(&self, pred: F, timeout: Option<Duration>) -> Result<proto::StudioState>
    where
        F: Fn(&proto::StudioState) -> bool,
    {
        let deadline = timeout.map(|t| Instant::now() + t);
        loop {
            let state = self.transport.master()
                .get_state(proto::GetStateRequest::default())
                .await
                .map_err(|e| anyhow!("get_state failed: {e}"))?
                .into_owned();
            let current = state.studios.into_iter().find(|s| s.session_id.as_deref() == Some(self.session_guid.as_str()));
            if let Some(s) = current.as_ref() {
                if pred(s) {
                    return Ok(current.unwrap());
                }
            }
            if let (Some(deadline), Some(timeout)) = (deadline, timeout) {
                if Instant::now() >= deadline {
                    return Err(wait_timeout_error(&self.session_guid, timeout, current.as_ref()));
                }
            }
            tokio::time::sleep(Duration::from_millis(DOM_POLL_INTERVAL_MS)).await;
        }
    }
}

