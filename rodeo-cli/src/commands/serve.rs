use anyhow::{Context, Result};
use rodeo_client::RodeoClient;
use crate::util::config;
use crate::master::BackendState;
use crate::studio_backend::http::handle_connection as handle_studio_connection;
use std::net::SocketAddr;
use std::process::Stdio;
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio::sync::Mutex;
use process_wrap::tokio::{CommandWrap, ChildWrapper};
#[cfg(unix)]
use process_wrap::tokio::ProcessGroup;
#[cfg(windows)]
use rbx_control::job::KillOnCloseJob;

/// Which role(s) this serve instance runs.
pub enum ServeMode {
    /// Master + studio backend (default)
    Combined,
    /// Master only — accepts backends + CLI clients
    Master,
    /// Studio backend only — connects outbound to master; listens on `port`
    Studio { port: u16, master_host: String, master_port: u16 },
}

/// Ensure MCP Server is enabled in Studio's AI Assistant settings for all users.
#[cfg(any(target_os = "macos", target_os = "windows"))]
pub fn ensure_mcp_enabled() {
    let _studio = match roblox_install::RobloxStudio::locate() {
        Ok(s) => s,
        Err(_) => return,
    };

    #[cfg(target_os = "macos")]
    let assistant_dir = {
        let home = match std::env::var("HOME") {
            Ok(h) => std::path::PathBuf::from(h),
            Err(_) => return,
        };
        home.join("Library").join("Roblox").join("AssistantSettings")
    };

    #[cfg(target_os = "windows")]
    let assistant_dir = {
        let roblox_dir = _studio.application_path()
            .ancestors()
            .find(|p| p.file_name().map_or(false, |n| n == "Roblox"))
            .map(|p| p.to_path_buf());
        match roblox_dir {
            Some(dir) => dir.join("AssistantSettings"),
            None => return,
        }
    };

    let entries = match std::fs::read_dir(&assistant_dir) {
        Ok(e) => e,
        Err(_) => return,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let Ok(content) = std::fs::read_to_string(&path) else {
            continue;
        };
        let Ok(mut val) = serde_json::from_str::<serde_json::Value>(&content) else {
            continue;
        };
        if val.get("mcp-server").and_then(|m| m.get("enabled")).and_then(|e| e.as_bool()) == Some(true) {
            continue;
        }
        if let Some(obj) = val.as_object_mut() {
            obj.insert("mcp-server".into(), serde_json::json!({"enabled": true}));
            let _ = std::fs::write(&path, serde_json::to_string_pretty(&val).unwrap_or_default());
        }
    }
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub fn ensure_mcp_enabled() {}

// ---------------------------------------------------------------------------
// Role entry points (used by internal subcommands and public serve flags)
// ---------------------------------------------------------------------------

/// Run the master server. Blocks until the process exits.
///
/// `master_id` is the bootstrap UUID assigned by `util::log_capture::init`
/// at master startup; it's advertised to backends via `RegisterResponse` so
/// distributed logs can be correlated across hosts.
pub async fn run_master(port: u16, master_id: String) -> Result<()> {
    let state = Arc::new(Mutex::new(crate::master::MasterState::new(master_id)));

    let addr: SocketAddr = ([127, 0, 0, 1], port).into();
    let router = crate::master::grpc::build_router(state.clone());

    tracing::info!("Master serving on port {port}");

    let service = connectrpc::ConnectRpcService::new(router).with_limits(
        connectrpc::Limits::default()
            .max_message_size(rodeo_proto::MAX_RPC_MESSAGE_SIZE)
            .max_request_body_size(rodeo_proto::MAX_RPC_MESSAGE_SIZE),
    );
    if let Err(e) = connectrpc::Server::from_service(service).serve(addr).await {
        tracing::error!("master server error: {e}");
    }

    Ok(())
}

/// Run a studio backend. Blocks until the process exits.
pub async fn run_studio_backend(port: u16, master_host: &str, master_port: u16) -> Result<()> {
    ensure_mcp_enabled();

    // Self-heal leaked filepatch patches from crashed/hard-killed runs before
    // accepting connections. A `--profile` run that didn't restore leaves
    // Studio's microprofiler-autocapture FFlag enabled *globally* — every
    // subsequent Studio (rodeo or manual) then captures dumps and fills the
    // disk; a killed `--show-widgets` run leaves a stripped dock layout. Each
    // sweep reverts only locks whose owner process is dead, so a concurrent
    // active patch is left untouched.
    if let Err(e) = rbx_control::fflags::sweep_stale_leak(rbx_control::fflags::FflagTarget::Studio) {
        tracing::warn!("fflag stale-lock sweep failed: {e}");
    }
    if let Err(e) = rbx_control::studio::layout::sweep_stale_leak() {
        tracing::warn!("layout stale-lock sweep failed: {e}");
    }

    // Two backends on the same port are already prevented by the socket bind.

    let state = Arc::new(Mutex::new(BackendState::new()));
    state.lock().await.port = port;

    {
        let scanner = rbx_control::profile_scanner::start("rodeo");
        state.lock().await.profile_scanner = Some(scanner);
    }

    tokio::spawn(crate::master::run_reconciliation(state.clone()));

    let addr: SocketAddr = ([127, 0, 0, 1], port).into();
    let listener = TcpListener::bind(&addr)
        .await
        .context(format!("failed to bind to {addr}"))?;

    // Bound: this port is ours. Sweep plugin files left behind by dead
    // backends, and install this backend's own — one file per running
    // backend, named by build and port, so serves of different builds or
    // ports never overwrite each other's plugin (issue #12). Installing only
    // after the bind means the file never describes a backend that failed to
    // start. The write is byte-idempotent, so a same-build serve returning to
    // this port leaves Studios that still hold the plugin untouched.
    //
    // The sweep runs alongside startup rather than before it: it probes the
    // masters of other serves' files, which can take seconds (a dead port is
    // slow to refuse on Windows), and nothing here waits on it — it never
    // touches this backend's file (too new, and its master answers).
    tokio::spawn(crate::studio_backend::plugin_sweep::sweep());
    match crate::studio_backend::launch::install_plugin(port) {
        Ok(path) => {
            // A file an earlier backend left on exit is this backend's now.
            crate::studio_backend::plugin_sweep::clear_kept(&path);
            tracing::info!(path = %path.display(), "installed rodeo plugin");
        }
        // An error, not a warning: without its plugin no Studio this backend
        // launches can connect, so a launch then fails only as "the plugin
        // never loaded" — this line is the cause, and quiet runs show errors.
        Err(e) => tracing::error!("failed to install rodeo plugin: {e}"),
    }

    let accept_state = state.clone();
    tokio::spawn(async move {
        loop {
            match listener.accept().await {
                Ok((stream, _addr)) => {
                    let conn_state = accept_state.clone();
                    tokio::spawn(handle_studio_connection(stream, conn_state));
                }
                Err(e) => {
                    tracing::error!("Accept error: {e}");
                }
            }
        }
    });

    tracing::info!("Studio backend on port {port}, connecting to master at {master_host}:{master_port}");

    #[cfg(unix)]
    let mut sigterm = tokio::signal::unix::signal(
        tokio::signal::unix::SignalKind::terminate(),
    ).expect("failed to register SIGTERM handler");

    let master_fut = async {
        match crate::studio_backend::backend::connect_to_master(master_host, master_port, Some(port)).await {
            Ok((client, backend_id, master_id, bidi)) => {
                state.lock().await.master_id = master_id;
                crate::studio_backend::backend::run_master_loop(client, backend_id, bidi, state.clone()).await;
            }
            Err(e) => {
                tracing::error!("studio backend failed to connect to master: {e}");
            }
        }
    };

    #[cfg(unix)]
    tokio::select! {
        _ = master_fut => {}
        _ = sigterm.recv() => {
            tracing::info!("studio backend shutting down...");
            // Cancel all spawned tasks (launch, monitor, etc.)
            state.lock().await.shutdown_token.cancel();
        }
    }
    #[cfg(not(unix))]
    master_fut.await;

    // Explicitly clean up all Studio instances. Studios launched with
    // `detached: true` survive parent exit — skip kill for those and let
    // their Drop impl restore fflag/layout state without touching the
    // process.
    {
        let mut guard = state.lock().await;
        let count = guard.studio_instances.len();
        tracing::info!("cleaning up {count} studio instance(s)...");
        for (id, inst) in guard.studio_instances.drain() {
            if let Some(studio) = inst.studio {
                if studio.detached() {
                    tracing::info!(studio_id = id.as_str(), "studio is detached, skipping kill (process will survive)");
                    // Arc drop runs rodeo::Studio::Drop → rbx_control::Studio::Drop,
                    // both respect `detached` and only restore fflags/layout.
                    continue;
                }
                tracing::info!(studio_id = id.as_str(), "killing Studio");
                studio.kill();
            }
        }

        // This backend's plugin file stays installed: the next serve of this
        // build on this port finds it unchanged and launches Studio without
        // waiting for a fresh file to settle, and a detached Studio may still
        // run it. The record lets other backends' sweeps leave it for
        // plugin_sweep::KEEP_FOR; they remove it after that.
        let kept = crate::studio_backend::launch::plugin_path(port)
            .and_then(|path| Ok(crate::studio_backend::plugin_sweep::mark_kept(&path)?));
        match kept {
            Ok(()) => tracing::info!("left this backend's plugin file installed for the next serve on this port"),
            Err(e) => tracing::warn!("couldn't record that this backend's plugin file stays installed: {e}"),
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Combined serve: spawns master + backends as child processes
// ---------------------------------------------------------------------------

/// Handle returned by `start_full_serve` for lifecycle management.
pub struct ServeHandle {
    pub shutdown_rx: tokio::sync::mpsc::Receiver<()>,
    /// The master, then the studio backend.
    children: Vec<Box<dyn ChildWrapper>>,
    /// Windows: one kill-on-close job per child. Held (not read) so the job
    /// handles live exactly as long as this handle — dropping them, or this
    /// process dying however abruptly, tears down each child's tree except
    /// breakaway (`--detach`) Studios.
    #[cfg(windows)]
    #[allow(dead_code)]
    jobs: Vec<KillOnCloseJob>,
}

impl ServeHandle {
    fn spawn(&mut self, exe: &std::path::Path, args: &[&str]) -> Result<()> {
        #[cfg(windows)]
        {
            let (child, job) = spawn_in_group(exe, args)?;
            self.children.push(child);
            self.jobs.push(job);
        }
        #[cfg(not(windows))]
        self.children.push(spawn_in_group(exe, args)?);
        Ok(())
    }

    /// Startup waits for the master to answer and the backend to register. A
    /// child that exited instead never will (the master returns when it can't
    /// bind its port, the backend when it can't reach the master): fail
    /// rather than wait forever. Its error is on stderr even in a quiet run.
    fn check_started(&mut self) -> Result<()> {
        for (child, role) in self.children.iter_mut().zip(["master", "studio backend"]) {
            if let Some(status) = child.try_wait().with_context(|| format!("checking rodeo's {role}"))? {
                let logs = std::env::var("RODEO_LOG_DIR").unwrap_or_else(|_| ".rodeo/.temp/logs".to_string());
                anyhow::bail!("rodeo's {role} exited while starting ({status}); its log is in {logs}");
            }
        }
        Ok(())
    }

    pub async fn wait_for_shutdown(mut self) {
        self.shutdown_rx.recv().await;
        tracing::info!("Shutting down...");
        self.kill_children().await;
    }

    pub async fn kill_children(&mut self) {
        // Send SIGTERM to each process group so children can clean up (e.g. kill Studio)
        #[cfg(unix)]
        for child in &self.children {
            let _ = child.signal(libc::SIGTERM);
        }
        // Windows has no SIGTERM to ask with. A studio backend runs the same
        // cleanup when its master connection closes, so stop the master and
        // let the backend finish before forcing whatever is left.
        #[cfg(windows)]
        {
            self.stop_master();
            let deadline = tokio::time::Instant::now() + BACKEND_WIND_DOWN;
            for child in &mut self.children {
                if tokio::time::timeout_at(deadline, child.wait()).await.is_err() {
                    let _ = child.start_kill();
                }
            }
        }
        for child in &mut self.children {
            let _ = child.wait().await;
        }
    }

    /// Kill the master, which `start_full_serve` spawns first.
    #[cfg(windows)]
    fn stop_master(&mut self) {
        if let Some(master) = self.children.first_mut() {
            let _ = master.start_kill();
        }
    }

    /// Let the studio backend hold its own job open. If this process dies
    /// without stopping it (a hard kill), the master dies at once with its
    /// job, which closes the backend's master connection and runs its
    /// shutdown; the backend's job kills what is left only once the backend
    /// has exited, instead of killing it mid-shutdown.
    #[cfg(windows)]
    fn share_backend_job(&self) -> Result<()> {
        let (Some(backend), Some(job)) = (self.children.get(1), self.jobs.get(1)) else {
            return Ok(());
        };
        let pid = backend.id().context("studio backend has no pid")?;
        job.share_with(pid).context("failed to share the studio backend's job with it")
    }
}

/// How long a studio backend gets, once its master is gone, to finish its own
/// shutdown (close the Studios it launched, record its plugin file) before it
/// is killed.
#[cfg(windows)]
const BACKEND_WIND_DOWN: std::time::Duration = std::time::Duration::from_secs(10);

/// Exit the studio backend when `ppid`, the serve that spawned it, exits. On
/// Unix the parent-exit watch SIGTERMs this process, which runs the backend's
/// shutdown. Windows has no signal to send; there the serve's death takes the
/// master with it, and the closed master connection runs the same shutdown —
/// so only exit if that hasn't finished within the wind-down window.
pub fn exit_backend_with_parent(ppid: u32) {
    #[cfg(not(windows))]
    parent_exit::on_parent_exit(ppid);
    #[cfg(windows)]
    parent_exit::on_pid_exit(ppid, || {
        std::thread::sleep(BACKEND_WIND_DOWN);
        std::process::exit(0);
    });
}

impl Drop for ServeHandle {
    fn drop(&mut self) {
        #[cfg(unix)]
        for child in &self.children {
            let _ = child.signal(libc::SIGTERM);
        }
        // As in `kill_children`, but waiting here: the jobs, dropped right
        // after, would kill the backend mid-cleanup.
        #[cfg(windows)]
        {
            self.stop_master();
            let deadline = std::time::Instant::now() + BACKEND_WIND_DOWN;
            for child in &mut self.children {
                while matches!(child.try_wait(), Ok(None)) && std::time::Instant::now() < deadline {
                    std::thread::sleep(std::time::Duration::from_millis(50));
                }
                let _ = child.start_kill();
            }
        }
    }
}

#[cfg(windows)]
type SpawnedChild = (Box<dyn ChildWrapper>, KillOnCloseJob);
#[cfg(not(windows))]
type SpawnedChild = Box<dyn ChildWrapper>;

fn spawn_in_group(exe: &std::path::Path, args: &[&str]) -> Result<SpawnedChild> {
    let mut wrap = CommandWrap::with_new(exe, |cmd| {
        cmd.args(args)
            .stdin(Stdio::null())
            .stderr(Stdio::inherit());
        // A console Ctrl+C reaches every process attached to the console
        // unless it is in another process group, so give each child its own,
        // as on Unix: only this process sees it and stops the children in
        // order (`kill_children`), instead of each dying mid-whatever.
        #[cfg(windows)]
        {
            const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
            cmd.creation_flags(CREATE_NEW_PROCESS_GROUP);
        }
    });
    // Put each child in its own kill group so termination cascades to
    // grandchildren (e.g. Studio): a Unix process group, or a Windows job
    // object (which kills the whole tree when the job handle is closed).
    // The job is ours, not process-wrap's: its JobObject never sets
    // BREAKAWAY_OK, which would trap `--detach` Studios in the job and kill
    // them at serve exit before any detach logic could matter.
    #[cfg(unix)]
    wrap.wrap(ProcessGroup::leader());
    let child = wrap
        .spawn()
        .context("failed to spawn child process")?;
    #[cfg(windows)]
    {
        let job = KillOnCloseJob::new().context("failed to create child job object")?;
        let pid = child.id().context("spawned child has no pid")?;
        job.assign_pid(pid).context("failed to assign child to job object")?;
        Ok((child, job))
    }
    #[cfg(not(windows))]
    Ok(child)
}

/// Spawn master + studio backend as child processes.
/// Each child is in its own process group so kill propagates to grandchildren (e.g. Studio).
/// Waits for the studio backend to register before returning.
pub async fn start_full_serve(port: u16) -> Result<ServeHandle> {
    let exe = std::env::current_exe().context("cannot find own binary")?;
    let ppid = std::process::id().to_string();
    let (shutdown_tx, shutdown_rx) = tokio::sync::mpsc::channel::<()>(1);
    // Built first so a startup failure drops it, which stops the children
    // that did start.
    let mut handle = ServeHandle {
        shutdown_rx,
        children: Vec::new(),
        #[cfg(windows)]
        jobs: Vec::new(),
    };

    // Spawn master and wait for it to be healthy
    handle.spawn(&exe, &["__master", "--port", &port.to_string(), "--ppid", &ppid])?;
    let rc = RodeoClient::connect("localhost", port)?;
    while !rc.is_healthy().await {
        handle.check_started()?;
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }

    // Spawn studio backend and wait for it to register
    let studio_port = port + 1;
    let studio_port_str = studio_port.to_string();
    let port_str = port.to_string();
    handle.spawn(&exe, &["__studio-backend", "--port", &studio_port_str, "--master-host", "localhost", "--master-port", &port_str, "--ppid", &ppid])?;
    #[cfg(windows)]
    if let Err(e) = handle.share_backend_job() {
        tracing::warn!("{e:#}; if this process is killed, the studio backend dies without its shutdown");
    }
    loop {
        let backends = rc.list_backends(None).await.unwrap_or_default();
        if backends.iter().any(|b| b.kind == "studio") { break; }
        handle.check_started()?;
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }

    tracing::info!("Serving on port {port}");

    // Signal handler
    tokio::spawn(async move {
        #[cfg(unix)]
        {
            let mut sigterm = tokio::signal::unix::signal(
                tokio::signal::unix::SignalKind::terminate(),
            ).expect("failed to register SIGTERM handler");
            tokio::select! {
                _ = tokio::signal::ctrl_c() => {}
                _ = sigterm.recv() => {}
            }
        }
        #[cfg(not(unix))]
        {
            let _ = tokio::signal::ctrl_c().await;
        }
        let _ = shutdown_tx.try_send(());
    });

    Ok(handle)
}

// ---------------------------------------------------------------------------
// Public serve command entry point
// ---------------------------------------------------------------------------

pub async fn main(
    port: Option<u16>,
    mode: ServeMode,
) -> Result<()> {
    match mode {
        ServeMode::Combined | ServeMode::Master => {
            let port = port.unwrap_or(config::SERVE_PORT);
            let handle = start_full_serve(port).await?;
            handle.wait_for_shutdown().await;
        }
        ServeMode::Studio { port, master_host, master_port } => {
            run_studio_backend(port, &master_host, master_port).await?;
        }
    }

    Ok(())
}
