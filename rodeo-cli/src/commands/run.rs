use anyhow::{bail, Context, Result};
use crate::cli::{FflagArgs, PlaceArgs, ServerArgs};
use crate::cli_run::{self, RunRequest};
use rodeo_client::RodeoClient;
use crate::commands::process_source::{self, ProcessedSource};
use rodeo_proto as proto;

/// All run command arguments
pub struct RunArgs {
    pub script: Option<String>,
    pub source: Option<String>,
    pub sourcemap: Option<String>,
    pub output: Option<String>,
    pub return_file: Option<String>,
    pub show_return: bool,
    pub mode: Option<String>,
    pub dom_kind: Option<String>,
    pub context: Option<String>,
    pub studio_id: Option<String>,
    pub no_warn: bool,
    pub no_error: bool,
    pub no_info: bool,
    pub no_print: bool,
    pub no_output: bool,
    pub reload_requires: bool,
    pub script_args: Vec<String>,
    pub server: ServerArgs,
    pub place: PlaceArgs,
    pub fflags: FflagArgs,
    pub verbose: bool,
}

struct ResolvedScript {
    script_path: Option<String>,
    script_content: String,
    instance_path: Option<String>,
}

struct RunConfig {
    script_content: String,
    script_path: Option<String>,
    output_file: Option<String>,
    return_file: Option<String>,
    show_return: bool,
    /// Sparse routing spec (validated in prepare_execution).
    route: crate::shared::target::RouteSpec,
    log_filter: proto::LogFilter,
    reload_requires: bool,
    script_args: Vec<String>,
    verbose: bool,
    instance_path: Option<String>,
    // Launch/server fields
    host: String,
    port: u16,
    place_target: Option<crate::studio_backend::PlaceTarget>,
    should_launch: bool,
    focus: bool,
    save: Option<String>,
    fflags: FflagArgs,
    detached: bool,
    show_widgets: Option<String>,
    // Targeting fields
    dom_id: Option<String>,
    studio_id: Option<String>,
    // Profiling
    profile: Option<std::path::PathBuf>,
}

impl RunConfig {
    /// The resolved route, or None for an empty (default-everything) spec.
    fn resolved(&self) -> Result<Option<crate::shared::target::Resolved>> {
        if self.route.is_empty() {
            Ok(None)
        } else {
            Ok(Some(self.route.resolve()?))
        }
    }

    fn is_play(&self) -> bool {
        self.resolved()
            .ok()
            .flatten()
            .map(|r| r.mode == crate::shared::target::StudioMode::Play)
            .unwrap_or(false)
    }
}

fn args_route(args: &RunArgs) -> Result<crate::shared::target::RouteSpec> {
    crate::shared::target::RouteSpec::from_strings(
        args.mode.as_deref(),
        args.dom_kind.as_deref(),
        args.context.as_deref(),
    )
}

pub async fn main(mut args: RunArgs) -> Result<()> {
    let has_script = args.script.is_some() || args.source.is_some();
    let has_place = args.place.to_target()?.is_some();

    if has_place && !has_script {
        return persistent_mode(args).await;
    }

    let cfg = resolve_script(&mut args)?;
    let cfg = prepare_execution(args, cfg)?;
    let result = submit_and_run(cfg).await?;

    if result.exit_code != 0 {
        std::process::exit(result.exit_code);
    }
    Ok(())
}

/// Persistent mode (`rodeo run --place …` with no script): ensure a serve on
/// the port, launch the place onto it, and — only if we started the serve
/// ourselves — stay alive to host it until Ctrl-C. If a serve is already
/// running we reuse it and return, leaving it and its other studios untouched.
/// Other terminals can `rodeo run --source "..."` against the same port with
/// no flags.
async fn persistent_mode(args: RunArgs) -> Result<()> {
    let port = args.server.port;
    let host = args.server.host.clone();
    let place_target = args.place.to_target()?;

    // Ensure a serve exists on `port`: reuse a healthy one (None — not ours to
    // tear down), otherwise start and own it (Some — we hold it open below).
    let owned: Option<super::serve::ServeHandle> =
        if RodeoClient::connect(&host, port)?.is_healthy().await {
            RodeoClient::connect(&host, port)?.check_version().await?;
            None
        } else {
            Some(super::serve::start_full_serve(port).await?)
        };

    let route = args_route(&args)?;
    let resolved = if route.is_empty() { None } else { Some(route.resolve()?) };
    let is_play = resolved.map(|r| r.mode == crate::shared::target::StudioMode::Play).unwrap_or(false);

    if is_play {
        let resolved = resolved.expect("is_play implies a resolved route");
        let mut play_handles = launch_play_processes(
            &host,
            port,
            &resolved,
            place_target.as_ref(),
            !args.place.focus,
            &args.fflags,
            args.place.show_widgets.clone(),
            args.place.profile.is_some(),
        ).await?;

        if let Some(handle) = owned {
            tracing::info!("Play mode running on port {port}. Press Ctrl-C to stop.");
            handle.wait_for_shutdown().await;
            play_handles.cleanup();
        }
    } else if let Some(target) = place_target {
        let req = build_launch_request(&target, !args.place.focus, args.place.save, args.fflags, args.place.detached, args.place.show_widgets.clone(), args.place.profile.is_some(), &host, port).await?;
        let rc = RodeoClient::connect(&host, port)?;
        if let Some(mut handle) = owned {
            // We own the serve: race the launch against ctrl-c, then hold it open.
            tokio::select! {
                r = rc.launch_studio_raw(req) => { r?; }
                _ = handle.shutdown_rx.recv() => {
                    tracing::info!("Shutting down...");
                    handle.kill_children().await;
                    return Ok(());
                }
            }
            handle.wait_for_shutdown().await;
        } else {
            // Reused an existing serve: launch onto it and return, leaving it up.
            rc.launch_studio_raw(req).await?;
        }
    } else if let Some(handle) = owned {
        // No place target (persistent mode implies --place; defensive), but we
        // started a serve — hold it open.
        handle.wait_for_shutdown().await;
    }

    Ok(())
}

/// Resolve script source: read from file/stdin/--source, process, parse directives.
/// Call `rodeo __process_source` subprocess to bundle + shim + resolve the script.
fn call_process_source(
    script_path: Option<&str>,
    source: Option<&str>,
    sourcemap: Option<&str>,
) -> Result<ProcessedSource> {
    let exe = std::env::current_exe().context("cannot find own binary")?;
    let mut cmd = std::process::Command::new(&exe);
    cmd.arg("__process_source");

    if let Some(file) = script_path {
        cmd.arg(file);
    }
    if let Some(src) = source {
        cmd.args(["--source", src]);
    }
    if let Some(sm) = sourcemap {
        cmd.args(["--sourcemap", sm]);
    }

    let output = cmd.output().context("failed to run __process_source")?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("source processing failed: {}", stderr.trim());
    }

    serde_json::from_slice(&output.stdout)
        .context("failed to parse __process_source output")
}

fn resolve_script(args: &mut RunArgs) -> Result<ResolvedScript> {
    let script_path = match &args.script {
        Some(s) if s == "-" => {
            use std::io::Read;
            let mut buf = String::new();
            std::io::stdin()
                .read_to_string(&mut buf)
                .context("failed to read stdin")?;
            args.source = Some(buf);
            None
        }
        Some(s) => Some(s.clone()),
        None => None,
    };

    if script_path.is_none() && args.source.is_none() {
        bail!("no script source provided. Use a file path, '-' for stdin, or --source for inline execution");
    }

    let processed = call_process_source(
        script_path.as_deref(),
        args.source.as_deref(),
        args.sourcemap.as_deref(),
    )?;

    // Directive merging is done in the parent at main.rs entry before clap
    // even hands us the final RunArgs — by the time we get here, directive
    // flags are already represented in `args` via clap's normal parse path.

    Ok(ResolvedScript {
        script_path: processed.script_path,
        script_content: processed.script,
        instance_path: processed.instance_path,
    })
}

/// Build RunConfig: validate the route, build log filter, assemble config.
fn prepare_execution(args: RunArgs, resolved: ResolvedScript) -> Result<RunConfig> {
    let route = args_route(&args)?;
    // Validate the route now (fast error) and enforce the dom-id pin rule.
    if !route.is_empty() {
        route.resolve()?;
    }
    if args.place.dom_id.is_some()
        && (route.mode.is_some() || route.dom_kind.is_some())
    {
        bail!("--dom-id pins the run to one DOM — mode/dom don't apply (only --context does)");
    }
    if args.studio_id.is_some() && args.place.dom_id.is_some() {
        bail!("--studio-id and --dom-id are mutually exclusive (a DOM already identifies its studio)");
    }

    let log_filter = process_source::execution::build_log_filter(
        args.no_warn, args.no_error, args.no_info, args.no_print, args.no_output,
    );

    let host = args.server.host.clone();
    let port = args.server.port;
    let place_target = args.place.to_target()?;
    let should_launch = place_target.is_some();

    // Always generate a run_id for correlation
    let _run_id = uuid::Uuid::new_v4().to_string().split('-').next().unwrap().to_string();

    // Resolve --profile output dir. Flat — filenames carry the run_id /
    // execution_id + session_guid, so a single shared dir avoids per-run
    // subdir churn and keeps `ls` / grep scoped to one location.
    let profile = args.place.profile.as_ref().map(|p| {
        if p.is_empty() {
            std::path::PathBuf::from(".rodeo/.temp/profiles")
        } else {
            std::path::PathBuf::from(p)
        }
    });

    let fflags = args.fflags;

    Ok(RunConfig {
        script_content: resolved.script_content,
        route,
        log_filter,
        reload_requires: args.reload_requires,
        script_args: args.script_args,
        verbose: args.verbose,
        instance_path: resolved.instance_path,
        // Only a real file path travels as script_path. Inline runs (stdin,
        // --source) used to send the whole source here, which the plugin then
        // assigned as the ModuleScript's Name — Instance.Name caps at 200,000
        // chars, so any inline script over that failed before its first line
        // (issue #16). The plugin names inline runs `inline-<executionId>`.
        script_path: resolved.script_path,
        output_file: args.output,
        return_file: args.return_file,
        show_return: args.show_return,
        host,
        port,
        place_target,
        should_launch,
        focus: args.place.focus,
        save: args.place.save,
        fflags,
        detached: args.place.detached,
        show_widgets: args.place.show_widgets,
        dom_id: args.place.dom_id,
        studio_id: args.studio_id,
        profile,
    })
}

/// The serve on `cfg.port` to launch onto: the running one once it is known to
/// be this build, or a new one this run owns (`Some`). The version check comes
/// first because launching onto another build's serve and only then refusing
/// it would leave the launched Studio behind on that serve.
async fn ensure_serve(cfg: &RunConfig) -> Result<Option<super::serve::ServeHandle>> {
    let client = RodeoClient::connect(&cfg.host, cfg.port)?;
    if client.is_healthy().await {
        client.check_version().await?;
        Ok(None)
    } else {
        Ok(Some(super::serve::start_full_serve(cfg.port).await?))
    }
}

/// Connect to (or launch) the server and execute the script.
async fn submit_and_run(cfg: RunConfig) -> Result<rodeo_client::RunResult> {
    let mut serve_handle: Option<super::serve::ServeHandle> = None;
    let mut _play_handles: Option<PlayHandles> = None;
    let mut launched_studio_id: Option<String> = None;

    if cfg.is_play() {
        // Play mode: ensure the session + requested client(s) exist.
        // NOTE: we do NOT gate on cfg.should_launch — `should_launch` is only
        // true when --place was provided, but `rodeo run --mode play
        // --dom client --clients N -s 'code'` against an already-running
        // play server must still spawn the client.
        serve_handle = ensure_serve(&cfg).await?;

        let resolved = cfg.resolved()?.expect("is_play implies a resolved route");
        _play_handles = Some(launch_play_processes(
            &cfg.host,
            cfg.port,
            &resolved,
            cfg.place_target.as_ref(),
            !cfg.focus,
            &cfg.fflags,
            cfg.show_widgets.clone(),
            cfg.profile.is_some(),
        )
        .await?);
    } else if cfg.place_target.is_some() {
        // `--place` guarantees the place is opened: launch a Studio for it
        // whether or not a serve already exists on the port. No serve →
        // bootstrap one first; existing serve → open an additional studio
        // session on it (the backend supports N concurrent sessions). The
        // run below is pinned to the launched session so the script provably
        // executes in THIS place, not load-balanced across resident studios.
        serve_handle = ensure_serve(&cfg).await?;

        if let Some(ref target) = cfg.place_target {
            let req = build_launch_request(target, !cfg.focus, cfg.save.clone(), cfg.fflags.clone(), cfg.detached, cfg.show_widgets.clone(), cfg.profile.is_some(), &cfg.host, cfg.port).await?;
            // Race launch against shutdown (ctrl-c / SIGTERM)
            if let Some(ref mut handle) = serve_handle {
                let rc = RodeoClient::connect(&cfg.host, cfg.port)?;
                tokio::select! {
                    r = rc.launch_studio_raw(req) => {
                        launched_studio_id = Some(r?.1);
                    }
                    _ = handle.shutdown_rx.recv() => {
                        tracing::info!("Shutting down...");
                        if let Some(mut h) = serve_handle.take() {
                            h.kill_children().await;
                        }
                        return Ok(rodeo_client::RunResult { exit_code: 130, ..Default::default() });
                    }
                }
            } else {
                let (_, studio_id) = RodeoClient::connect(&cfg.host, cfg.port)?.launch_studio_raw(req).await?;
                launched_studio_id = Some(studio_id);
            }
        }
    }

    if cfg.should_launch {
        if !RodeoClient::connect(&cfg.host, cfg.port)?.wait_for_healthy().await {
            bail!("no rodeo server found at {}:{}", cfg.host, cfg.port);
        }
    } else {
        // Brief retry in case server is still starting up
        let mut found = false;
        for _ in 0..6 {
            if RodeoClient::connect(&cfg.host, cfg.port)?.is_healthy().await {
                found = true;
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        }
        if !found {
            bail!("no rodeo server found at {}:{}. Run 'rodeo serve --port {}' first.", cfg.host, cfg.port, cfg.port);
        }
    }
    // The master answering may be another build (a serve left running across
    // an upgrade or rebuild, or another project's on this port). Refuse to
    // run against it rather than let proto drift surface as odd failures.
    RodeoClient::connect(&cfg.host, cfg.port)?.check_version().await?;

    // Resolve --dom-id / --studio-id prefixes against live state.
    let dom_id = match cfg.dom_id.as_deref() {
        Some(prefix) => Some(resolve_dom_id(&cfg.host, cfg.port, prefix).await?),
        None => None,
    };
    // Session pin priority: an explicit --studio-id, else the studio we just
    // launched (so `run --place` executes in THIS place).
    let session = match cfg.studio_id.as_deref() {
        Some(prefix) => Some(resolve_studio_id(&cfg.host, cfg.port, prefix).await?),
        None => launched_studio_id.clone(),
    };

    let is_profiling = cfg.profile.is_some();
    let request = RunRequest {
        script: cfg.script_content,
        route: cfg.route,
        dom_id,
        session,
        log_filter: cfg.log_filter,
        reload_requires: cfg.reload_requires,
        script_args: cfg.script_args,
        return_file: cfg.return_file,
        show_return: cfg.show_return,
        output_file: cfg.output_file,
        verbose: cfg.verbose,
        instance_path: cfg.instance_path,
        script_path: cfg.script_path,
        on_created: None,
        profile: is_profiling,
        profile_dir: cfg.profile.clone(),
    };

    let save_requested = cfg.save.is_some();
    let result = if let Some(mut handle) = serve_handle {
        // Race the run against serve shutdown (Ctrl+C)
        let r = tokio::select! {
            r = cli_run::run_piped(&cfg.host, cfg.port, request) => r,
            _ = handle.shutdown_rx.recv() => {
                tracing::info!("Shutting down...");
                handle.kill_children().await;
                return Ok(rodeo_client::RunResult { exit_code: 130, ..Default::default() });
            }
        };
        if let Some(ref sid) = launched_studio_id {
            finish_launched_studio(&cfg.host, cfg.port, sid, save_requested, cfg.detached, &r).await?;
        }
        drop(handle);
        r?
    } else {
        let r = cli_run::run_piped(&cfg.host, cfg.port, request).await;
        // Same one-shot hygiene when we launched a studio on someone else's
        // serve: save + close after the run (close skipped for --detach).
        if let Some(ref sid) = launched_studio_id {
            finish_launched_studio(&cfg.host, cfg.port, sid, save_requested, cfg.detached, &r).await?;
        }
        r?
    };

    Ok(result)
}

/// Post-run save + close for a Studio this run launched.
///
/// `--save` goes through the same mtime-verified SavePlace path as
/// `rodeo save`, *before* the CLI reports success — the close-time cleanup
/// save is one-shot and can only log its failure, which made a missed save
/// look like exit 0 (issue #7). A verified save arms `skip_save` on the close
/// so the backstop doesn't re-fire against the already-clean document.
///
/// Error precedence: when the run itself failed, a save failure is logged but
/// the run's own error stands; when the run succeeded, a save failure IS the
/// command's failure. `--detach` saves without closing.
async fn finish_launched_studio(
    host: &str,
    port: u16,
    session_guid: &str,
    save_requested: bool,
    detached: bool,
    run_result: &anyhow::Result<rodeo_client::RunResult>,
) -> Result<()> {
    let run_failed = run_result.as_ref().map(|r| r.exit_code != 0).unwrap_or(true);
    let mut saved = false;
    if save_requested {
        match RodeoClient::connect(host, port)?.save_place(Some(session_guid.to_string())).await {
            Ok(_) => saved = true,
            Err(e) if run_failed => {
                tracing::warn!("--save failed after run failure: {e}");
            }
            Err(e) => {
                if !detached {
                    let _ = RodeoClient::connect(host, port)?.close_studio_raw(session_guid, false).await;
                }
                return Err(e);
            }
        }
    }
    if !detached {
        let _ = RodeoClient::connect(host, port)?.close_studio_raw(session_guid, saved).await;
    }
    Ok(())
}


// ---------------------------------------------------------------------------
// Play mode helpers
// ---------------------------------------------------------------------------

/// Resolve a `--dom-id` value (full id or unique prefix) against live state.
async fn resolve_dom_id(host: &str, port: u16, prefix: &str) -> Result<String> {
    let state = RodeoClient::connect(host, port)?.get_state().await?;
    let matches: Vec<String> = state.studios.iter()
        .flat_map(|s| s.doms.iter().map(|d| d.dom_id.clone()))
        .filter(|id| id.starts_with(prefix))
        .collect();
    resolve_unique(matches, prefix, "DOM")
}

/// Resolve a `--studio-id` value (full id or unique prefix) against live state.
async fn resolve_studio_id(host: &str, port: u16, prefix: &str) -> Result<String> {
    let state = RodeoClient::connect(host, port)?.get_state().await?;
    let matches: Vec<String> = state.studios.iter()
        .map(|s| s.studio_id.clone())
        .filter(|id| id.starts_with(prefix))
        .collect();
    resolve_unique(matches, prefix, "studio")
}

fn resolve_unique(mut matches: Vec<String>, prefix: &str, what: &str) -> Result<String> {
    matches.sort();
    matches.dedup();
    match matches.len() {
        1 => Ok(matches.into_iter().next().unwrap()),
        0 => bail!("no connected {what} matches id '{prefix}' (see `rodeo state`)"),
        _ => bail!("id '{prefix}' is ambiguous — matches {} {what}s; use more characters", matches.len()),
    }
}

/// Build a LaunchStudioRequest from CLI args for the LaunchStudio RPC.
pub(crate) async fn build_launch_request(
    target: &crate::studio_backend::PlaceTarget,
    background: bool,
    save: Option<String>,
    fflags: crate::cli::FflagArgs,
    detached: bool,
    show_widgets: Option<String>,
    profile: bool,
    host: &str,
    port: u16,
) -> Result<rodeo_proto::LaunchStudioRequest> {
    let studio_backend = wait_for_studio_backend(host, port).await?;

    Ok(rodeo_proto::LaunchStudioRequest {
        backend: studio_backend.id.clone(),
        place_file: match target {
            crate::studio_backend::PlaceTarget::File(p) => Some(p.clone()),
            _ => None,
        },
        place_id: match target {
            crate::studio_backend::PlaceTarget::PlaceId { place_id, .. } => Some(*place_id),
            _ => None,
        },
        fflags: fflags.fflag_override,
        background,
        detached,
        show_widgets: show_widgets.unwrap_or_default(),
        profile,
        save_path: save,
        fflag_file: fflags.fflag_file,
        ..Default::default()
    })
}

/// How long a launch waits for the serve's studio backend to register. A serve
/// answers on its master port about 100 ms before its studio backend registers,
/// so a run submitted right after `rodeo serve` starts lands in that gap
/// (issue #30). A backend still missing after this is not coming.
const STUDIO_BACKEND_WAIT: std::time::Duration = std::time::Duration::from_secs(5);
const STUDIO_BACKEND_POLL: std::time::Duration = std::time::Duration::from_millis(100);

/// The studio backend registered with the master on `host:port`, waiting up to
/// `STUDIO_BACKEND_WAIT` for one to appear.
async fn wait_for_studio_backend(host: &str, port: u16) -> Result<rodeo_proto::BackendInfo> {
    let client = RodeoClient::connect(host, port)?;
    poll_studio_backend(|| client.list_backends(Some("studio")), STUDIO_BACKEND_WAIT, STUDIO_BACKEND_POLL)
        .await?
        .ok_or_else(|| anyhow::anyhow!(no_studio_backend_message(host, port, STUDIO_BACKEND_WAIT)))
}

/// Poll `list` every `every` until it returns a studio backend (`Some`) or
/// `wait` elapses (`None`). A failing `list` (master gone) fails at once.
async fn poll_studio_backend<F, Fut>(
    mut list: F,
    wait: std::time::Duration,
    every: std::time::Duration,
) -> Result<Option<rodeo_proto::BackendInfo>>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<Vec<rodeo_proto::BackendInfo>>>,
{
    let deadline = tokio::time::Instant::now() + wait;
    loop {
        if let Some(b) = list().await?.into_iter().find(|b| b.kind == "studio") {
            return Ok(Some(b));
        }
        if tokio::time::Instant::now() >= deadline {
            return Ok(None);
        }
        tokio::time::sleep(every).await;
    }
}

/// The error for a serve whose master answers but has no studio backend:
/// either the backend died (the master outlives it) or it never started. The
/// master still holds the port, so a plain `rodeo serve` would fail to bind;
/// name both ways out.
fn no_studio_backend_message(host: &str, port: u16, waited: std::time::Duration) -> String {
    let master_host = if host == "localhost" { String::new() } else { format!(" --master-host {host}") };
    format!(
        "the rodeo serve on {host}:{port} has no studio backend (its master answers, but no studio \
         backend registered within {}s). Stop that serve and start it again with \
         `rodeo serve --port {port}`, or attach a studio backend to it with \
         `rodeo serve --studio{master_host} --master-port {port} --port {}`",
        waited.as_secs(),
        port.saturating_add(1),
    )
}

/// Placeholder retained for the play call sites. Multiplayer-test DOMs are now
/// owned by the studio backend (the edit Studio hosts the in-Studio test via
/// `StudioTestService:ExecuteMultiplayerTestAsync`), so the CLI holds no process
/// handles to clean up.
#[derive(Default)]
pub struct PlayHandles;

impl PlayHandles {
    pub fn cleanup(&mut self) {}
}

async fn launch_play_processes(
    host: &str,
    port: u16,
    route: &crate::shared::target::Resolved,
    place: Option<&crate::studio_backend::PlaceTarget>,
    background: bool,
    fflags: &crate::cli::FflagArgs,
    show_widgets: Option<String>,
    profile: bool,
) -> Result<PlayHandles> {
    use crate::shared::target::DomKind;
    use crate::studio_backend::PlaceTarget;
    use rodeo_client::studio::{OpenOpts, OpenFileOpts, OpenPlaceOpts};

    let _ = port;
    let client = RodeoClient::connect(host, port)?;

    let want_client = matches!(route.dom_kind, DomKind::Client);

    // Existing play session? (a studio that already has a server DOM)
    let snapshot = client.get_state().await.ok();
    let server_studio = snapshot.as_ref().and_then(|s| {
        s.studios.iter().find(|st| st.doms.iter().any(|v| v.dom_kind == "server")).cloned()
    });

    if let Some(st) = server_studio {
        // A multiplayer test is already running:
        //   --dom client  => append one more client (via AddPlayers on the server)
        //   --dom server  => leave the session as-is
        let current = st.doms.iter().filter(|v| v.dom_kind == "client").count() as u32;
        let target_total = if want_client { current + 1 } else { current };
        if target_total > current {
            let add = target_total - current;
            tracing::info!(add, current, target_total, "growing play session via AddPlayers");
            client.submit_run(rodeo_client::RunCodeOpts {
                source: format!("game:GetService(\"StudioTestService\"):AddPlayers({add})\nreturn true"),
                mode: Some("play".to_string()),
                context: Some("server".to_string()),
                ..Default::default()
            }).await.context("AddPlayers run failed")?;
        }
        wait_for_play_session(&client, target_total).await?;
        return Ok(PlayHandles);
    }

    // No play session yet. Fresh test starts with one client (client dom) or
    // none (server dom); more clients are added one at a time via later runs.
    let initial_clients: u32 = if want_client { 1 } else { 0 };

    // We need a place to open the edit Studio that will host the test.
    let Some(place) = place else {
        tracing::info!("waiting for play DOMs to connect...");
        wait_for_play_session(&client, initial_clients).await?;
        return Ok(PlayHandles);
    };

    // Open the edit Studio (profile=true so the multiplayer-test child
    // DataModels inherit the profiler FFlags), then start the in-Studio test.
    // Bounded wait: `get_local_studio` polls forever, which hangs the run when
    // the serve's backend is dead.
    let backend_info = wait_for_studio_backend(host, port).await?;
    let backend = client.get_backend(&backend_info.id).await?;
    let fflag_overrides = fflags.fflag_override.clone();
    let fflag_file = fflags.fflag_file.clone();
    tracing::info!(dom = route.dom_kind.as_str(), initial_clients, "opening edit Studio for multiplayer test");
    let studio = match place {
        PlaceTarget::File(p) => backend.open_file(OpenFileOpts {
            path: p.clone(),
            fflags: fflag_overrides, background, profile,
            save: None, detached: false, fflag_file, show_widgets: show_widgets.clone(),
            timeout: None,
        }).await?,
        PlaceTarget::PlaceId { place_id, .. } => backend.open_place(OpenPlaceOpts {
            place_id: *place_id,
            fflags: fflag_overrides, background, profile,
            save: None, detached: false, fflag_file, show_widgets: show_widgets.clone(),
            timeout: None,
        }).await?,
        PlaceTarget::Empty => backend.open(OpenOpts {
            fflags: fflag_overrides, background, profile,
            save: None, detached: false, fflag_file, show_widgets: show_widgets.clone(),
            timeout: None,
        }).await?,
        PlaceTarget::Content(_) => bail!("Content place target is not supported for play launch"),
    };

    studio.start_multiplayer_test(initial_clients, None).await
        .context("failed to start multiplayer test")?;
    tracing::info!("multiplayer test started");

    Ok(PlayHandles)
}

/// Poll the studio-first state until a play session (a studio with a server DOM
/// and at least `clients` client DOMs) is present.
async fn wait_for_play_session(client: &RodeoClient, clients: u32) -> Result<()> {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
    loop {
        if let Ok(state) = client.get_state().await {
            let ready = state.studios.iter().any(|st| {
                st.doms.iter().any(|v| v.dom_kind == "server")
                    && st.doms.iter().filter(|v| v.dom_kind == "client").count() as u32 >= clients
            });
            if ready {
                return Ok(());
            }
        }
        if std::time::Instant::now() >= deadline {
            bail!("timed out waiting for play session (server + {clients} clients)");
        }
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    }
}

#[cfg(test)]
mod studio_backend_wait_tests {
    use super::*;
    use std::time::Duration;

    fn backend(kind: &str) -> rodeo_proto::BackendInfo {
        rodeo_proto::BackendInfo { id: format!("{kind}-id"), kind: kind.to_string(), ..Default::default() }
    }

    // A serve's master answers before its studio backend registers: the first
    // polls see no backend, a later one does (issue #30).
    #[tokio::test]
    async fn waits_for_a_backend_that_registers_late() {
        let calls = std::cell::Cell::new(0);
        let found = poll_studio_backend(
            || {
                calls.set(calls.get() + 1);
                let n = calls.get();
                async move { Ok(if n < 4 { vec![] } else { vec![backend("player"), backend("studio")] }) }
            },
            Duration::from_secs(5),
            Duration::from_millis(1),
        )
        .await
        .unwrap();
        assert_eq!(found.map(|b| b.id), Some("studio-id".to_string()));
        assert_eq!(calls.get(), 4);
    }

    #[tokio::test]
    async fn gives_up_after_the_wait() {
        let start = std::time::Instant::now();
        let found = poll_studio_backend(
            || async { Ok(vec![backend("player")]) },
            Duration::from_millis(50),
            Duration::from_millis(5),
        )
        .await
        .unwrap();
        assert!(found.is_none());
        assert!(start.elapsed() >= Duration::from_millis(50));
    }

    #[tokio::test]
    async fn a_master_error_fails_at_once() {
        let calls = std::cell::Cell::new(0);
        let err = poll_studio_backend(
            || {
                calls.set(calls.get() + 1);
                async { Err::<Vec<rodeo_proto::BackendInfo>, _>(anyhow::anyhow!("list_backends failed: connection refused")) }
            },
            Duration::from_secs(5),
            Duration::from_millis(1),
        )
        .await
        .unwrap_err();
        assert!(err.to_string().contains("connection refused"));
        assert_eq!(calls.get(), 1);
    }

    // The message names the recovery commands for the port in use; a plain
    // `rodeo serve` alone would fail to bind the port the master still holds.
    #[test]
    fn missing_backend_message_names_the_recovery() {
        let msg = no_studio_backend_message("localhost", 44960, Duration::from_secs(5));
        assert!(msg.contains("the rodeo serve on localhost:44960 has no studio backend"), "{msg}");
        assert!(msg.contains("within 5s"), "{msg}");
        assert!(msg.contains("`rodeo serve --port 44960`"), "{msg}");
        assert!(msg.contains("`rodeo serve --studio --master-port 44960 --port 44961`"), "{msg}");

        let remote = no_studio_backend_message("10.0.0.2", 44872, Duration::from_secs(5));
        assert!(remote.contains("`rodeo serve --studio --master-host 10.0.0.2 --master-port 44872 --port 44873`"), "{remote}");
    }
}
