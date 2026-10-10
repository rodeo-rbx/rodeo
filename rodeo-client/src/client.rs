use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, bail, Result};

use crate::proto;
use crate::studio::StudioBackend;
use crate::transport::Transport;
use crate::dom::Dom;

const POLL_INTERVAL_MS: u64 = 500;
const HEALTH_PROBE_TIMEOUT: Duration = Duration::from_secs(2);

/// The root client — owns the transport shared across all handles it mints.
#[derive(Clone)]
pub struct RodeoClient {
    transport: Arc<Transport>,
}

impl RodeoClient {
    /// Connect to a rodeo master at `host:port`. No network I/O happens until
    /// the first RPC.
    pub fn connect(host: impl Into<String>, port: u16) -> Result<Self> {
        Ok(Self { transport: Arc::new(Transport::new(host, port)?) })
    }

    pub fn host(&self) -> &str { &self.transport.host }
    pub fn port(&self) -> u16 { self.transport.port }

    // -----------------------------------------------------------------------
    // Health & state
    // -----------------------------------------------------------------------

    /// Whether a rodeo master answers on this port. Bounded: something else
    /// holding the port can accept the connection and never reply, which left
    /// every caller (a run deciding whether to start its own serve, the serve
    /// waiting for its master) waiting forever. A master answers in ms.
    pub async fn is_healthy(&self) -> bool {
        let master = self.transport.master();
        let probe = master.health(proto::HealthRequest::default());
        matches!(tokio::time::timeout(HEALTH_PROBE_TIMEOUT, probe).await, Ok(Ok(_)))
    }

    /// Raw health probe. Errors if the master is unreachable.
    pub async fn health(&self) -> Result<proto::HealthResponse> {
        Ok(self.transport
            .master()
            .health(proto::HealthRequest::default())
            .await
            .map_err(|e| anyhow!("health failed: {e}"))?
            .into_owned())
    }

    /// Verify the master's build id equals this client's (`BUILD_ID`). Every
    /// entry point that reuses an already-running master calls this after
    /// the health probe: the probe says "something is listening", this says
    /// "and it is the same build". A mismatch is an error naming both sides
    /// and the fix; `RODEO_SKIP_VERSION_CHECK` downgrades it to a warning.
    pub async fn check_version(&self) -> Result<()> {
        let health = self.health().await?;
        let label = format!("the master at {}:{}", self.host(), self.port());
        match proto::check_peer_version(&label, &health.version) {
            Ok(()) => Ok(()),
            Err(msg) if proto::version_check_skipped() => {
                tracing::warn!("{msg} ({} set, continuing)", proto::SKIP_VERSION_CHECK_ENV);
                Ok(())
            }
            Err(msg) => bail!(
                "{msg}. Stop that serve and start one from this rodeo, or set {}=1 to proceed anyway.",
                proto::SKIP_VERSION_CHECK_ENV
            ),
        }
    }

    /// Poll `is_healthy` up to ~15s. Returns true if the master responded.
    pub async fn wait_for_healthy(&self) -> bool {
        for _ in 0..30 {
            if self.is_healthy().await { return true; }
            tokio::time::sleep(Duration::from_millis(POLL_INTERVAL_MS)).await;
        }
        false
    }

    pub async fn get_state(&self) -> Result<proto::RodeoSnapshot> {
        Ok(self.transport
            .master()
            .get_state(proto::GetStateRequest::default())
            .await
            .map_err(|e| anyhow!("get_state failed: {e}"))?
            .into_owned())
    }

    // -----------------------------------------------------------------------
    // Process management
    // -----------------------------------------------------------------------

    pub async fn list_processes(&self) -> Result<Vec<proto::ProcessInfo>> {
        Ok(self.transport
            .master()
            .list_processes(proto::ListProcessesRequest::default())
            .await
            .map_err(|e| anyhow!("list_processes failed: {e}"))?
            .into_owned()
            .processes)
    }

    pub async fn kill(&self, execution_id: &str) -> Result<()> {
        self.transport
            .master()
            .kill_process(proto::KillProcessRequest {
                execution_id: execution_id.to_string(),
                ..Default::default()
            })
            .await
            .map_err(|e| anyhow!("kill_process({execution_id}) failed: {e}"))?;
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Backend discovery
    // -----------------------------------------------------------------------

    pub async fn list_backends(&self, kind: Option<&str>) -> Result<Vec<proto::BackendInfo>> {
        Ok(self.transport
            .master()
            .list_backends(proto::ListBackendsRequest { kind: kind.map(String::from), ..Default::default() })
            .await
            .map_err(|e| anyhow!("list_backends failed: {e}"))?
            .into_owned()
            .backends)
    }

    /// Poll until a studio backend appears; returns a handle.
    pub async fn get_local_studio(&self) -> Result<StudioBackend> {
        loop {
            let backends = self.list_backends(Some("studio")).await?;
            if let Some(info) = backends.into_iter().next() {
                return Ok(StudioBackend::new(info, self.transport.clone()));
            }
            tokio::time::sleep(Duration::from_millis(POLL_INTERVAL_MS)).await;
        }
    }

    /// Select a studio BACKEND (a machine running a studio backend process)
    /// by id prefix or exact name — not a studio instance; those are
    /// addressed by studio_id via the snapshot.
    pub async fn get_backend(&self, id_or_name: &str) -> Result<StudioBackend> {
        let backends = self.list_backends(Some("studio")).await?;
        backends
            .into_iter()
            .find(|b| b.id.starts_with(id_or_name) || b.name == id_or_name)
            .map(|info| StudioBackend::new(info, self.transport.clone()))
            .ok_or_else(|| anyhow!("studio backend '{id_or_name}' not found"))
    }

    // -----------------------------------------------------------------------
    // DOM discovery
    // -----------------------------------------------------------------------

    pub async fn get_doms(&self) -> Result<Vec<Dom>> {
        let state = self.get_state().await?;
        let mut out = Vec::new();
        for s in &state.studios {
            for v in &s.doms {
                out.push(Dom::from_studio_dom(s, v, self.transport.clone()));
            }
        }
        Ok(out)
    }

    pub async fn get_dom(&self, dom_id: &str) -> Result<Dom> {
        self.get_doms()
            .await?
            .into_iter()
            .find(|v| v.dom_id == dom_id)
            .ok_or_else(|| anyhow!("dom '{dom_id}' not found"))
    }

    pub async fn get_live_servers(&self) -> Result<Vec<Dom>> {
        Ok(self
            .get_doms()
            .await?
            .into_iter()
            .filter(|v| v.mode == "live" && v.dom_kind == "server")
            .collect())
    }

    pub async fn get_live_server_for_place(&self, place_id: i64) -> Result<Dom> {
        self.get_doms()
            .await?
            .into_iter()
            .find(|v| v.mode == "live" && v.dom_kind == "server" && v.place_id == place_id)
            .ok_or_else(|| anyhow!("no live server found for place {place_id}"))
    }

    // -----------------------------------------------------------------------
    // Direct master RPCs (low-level — used by CLI orchestration, not by TS)
    // -----------------------------------------------------------------------

    /// Launch a Studio, blocking until the plugin DOM connects (Ready event).
    /// Returns (backend_id, session_guid). Prefer `StudioBackend::open*` for
    /// higher-level use — this is for CLI orchestration that builds the
    /// LaunchStudioRequest manually.
    pub async fn launch_studio_raw(&self, req: proto::LaunchStudioRequest) -> Result<(String, String)> {
        let mut stream = self.transport
            .master()
            .launch_studio(req)
            .await
            .map_err(|e| anyhow!("failed to launch studio: {e}"))?;
        while let Ok(Some(view)) = stream.message().await {
            let event = view.to_owned_message();
            match event.event {
                Some(proto::launch_studio_event::Event::Ready(ready)) => {
                    return Ok((ready.backend_id, ready.session_guid));
                }
                Some(proto::launch_studio_event::Event::Error(err)) => {
                    bail!("studio launch failed: {}", err.message);
                }
                _ => {}
            }
        }
        bail!("launch studio stream ended without ready event")
    }

    /// Close Studio, blocking until Closed event. `skip_save: true` tells the
    /// backend the caller already performed a verified SavePlace for this
    /// session, so the close-time backstop save is skipped.
    pub async fn close_studio_raw(&self, session_guid: &str, skip_save: bool) -> Result<()> {
        let mut stream = self.transport
            .master()
            .close_studio(proto::CloseStudioRequest {
                session_guid: session_guid.to_string(),
                skip_save: skip_save.then_some(true),
                ..Default::default()
            })
            .await
            .map_err(|e| anyhow!("failed to close studio: {e}"))?;
        while let Ok(Some(view)) = stream.message().await {
            let event = view.to_owned_message();
            if let Some(proto::close_studio_event::Event::Closed(_)) = event.event {
                return Ok(());
            }
        }
        Ok(())
    }

    /// Submit a run with no DOM handle — the serve-wide tier: the master
    /// routes by the opts' mode/dom_kind/context fields (within opts.session
    /// if set) and resolves them to a DOM.
    pub async fn submit_run(&self, opts: crate::run::RunCodeOpts) -> Result<crate::run::RunResult> {
        crate::run::run_buffered_routed(self.transport.clone(), opts).await
    }

    /// Streaming variant of `submit_run` — caller drives the returned stream
    /// and decides how to handle Output events (write to stdio, emit as
    /// structured notifications, etc.).
    pub async fn submit_run_stream(&self, opts: crate::run::RunCodeOpts) -> Result<crate::run::RunStream> {
        crate::run::run_stream_routed(self.transport.clone(), opts).await
    }

    /// Save a studio's place. `session_guid: None` falls back to the only
    /// connected Studio (errors server-side when 0 or ≥2 are connected).
    pub async fn save_place(&self, session_guid: Option<String>) -> Result<proto::SavePlaceResponse> {
        let resp = self.transport
            .master()
            .save_place(proto::SavePlaceRequest { session_guid, ..Default::default() })
            .await
            .map_err(|e| anyhow!("save failed: {e}"))?
            .into_owned();
        if !resp.saved {
            bail!("save failed: {}", resp.error.as_deref().unwrap_or(""));
        }
        Ok(resp)
    }

    /// Save the default (current) studio — for CLI `rodeo save`.
    pub async fn save_default(&self) -> Result<proto::SavePlaceResponse> {
        self.save_place(None).await
    }
}

/// Re-exports from `rodeo_proto` that callers routinely reach for — saves them
/// importing from the proto crate directly for the common cases.
pub use proto::{BackendInfo, ProcessInfo, RodeoSnapshot, DomSnapshot};
