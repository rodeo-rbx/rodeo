use super::{ChildProcess, SharedRpcState, StreamHandler};
use rodeo_proto::runtime_types as rt;

/// Locate the Studio `content/` directory via `roblox_install`. Same logic as
/// `rbx_control::studio::launch::studio_content_path`, duplicated here so
/// rodeo-client doesn't take a dep on rbx-control.
fn studio_content_path() -> Option<String> {
    roblox_install::RobloxStudio::locate()
        .ok()
        .map(|s| s.content_path().to_string_lossy().to_string())
}

fn format_process_output(output: &std::process::Output) -> rt::ProcessRunResponse {
    rt::ProcessRunResponse {
        out: String::from_utf8_lossy(&output.stdout).to_string(),
        err: String::from_utf8_lossy(&output.stderr).to_string(),
        ..format_status(output.status)
    }
}

/// How a process ended: its exit code, or -1 and the signal that ended it.
fn format_status(status: std::process::ExitStatus) -> rt::ProcessRunResponse {
    #[cfg(unix)]
    let signal = std::os::unix::process::ExitStatusExt::signal(&status);
    #[cfg(not(unix))]
    let signal = None;
    rt::ProcessRunResponse {
        ok: status.success(),
        exitcode: status.code().unwrap_or(-1),
        signal,
        ..Default::default()
    }
}

pub fn process_get_info(_req: &rt::ProcessGetInfoRequest) -> Result<rt::ProcessGetInfoResponse, String> {
    Ok(rt::ProcessGetInfoResponse {
        cwd: std::env::current_dir()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default(),
        // HOME on Unix; Windows normally has no HOME, so fall back to USERPROFILE.
        homedir: std::env::var("HOME")
            .or_else(|_| std::env::var("USERPROFILE"))
            .unwrap_or_default(),
        execpath: std::env::current_exe()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default(),
        env: std::env::vars().collect(),
        platform: Some(std::env::consts::OS.to_string()),
        arch: Some(std::env::consts::ARCH.to_string()),
        studio_content_path: studio_content_path(),
        ..Default::default()
    })
}

pub async fn process_exit(state: SharedRpcState, req: &rt::ProcessExitRequest) -> Result<rt::Ok, String> {
    let mut guard = state.lock().await;
    guard.exit_code = req.code;
    guard.exit_requested = true;
    Ok(rt::Ok::default())
}

fn build_command(program: &str, args: &[String], opts: Option<&rt::ProcessOptions>) -> tokio::process::Command {
    let mut cmd = tokio::process::Command::new(program);
    cmd.args(args);
    if let Some(o) = opts {
        if let Some(cwd) = &o.cwd {
            cmd.current_dir(cwd);
        }
        cmd.envs(&o.env);
    }
    cmd
}

/// Run `cmd` to completion with stdout and stderr captured. With `input`, the
/// child's stdin gets those bytes and is then closed; without, stdin is null,
/// so a child that reads it sees end-of-file at once.
async fn output_with_input(mut cmd: tokio::process::Command, input: Option<&[u8]>) -> std::io::Result<std::process::Output> {
    use tokio::io::AsyncWriteExt;
    cmd.stdout(std::process::Stdio::piped());
    cmd.stderr(std::process::Stdio::piped());
    let Some(input) = input else {
        return cmd.output().await;
    };
    cmd.stdin(std::process::Stdio::piped());
    let mut child = cmd.spawn()?;
    let mut stdin = child.stdin.take().expect("stdin is piped");
    let input = input.to_vec();
    // Write while the output is read: a child that answers as it reads (cat)
    // would otherwise fill its stdout pipe and stall both sides.
    let write = async move {
        let written = stdin.write_all(&input).await;
        drop(stdin);
        written
    };
    let (written, output) = tokio::join!(write, child.wait_with_output());
    match written {
        // A child may exit without reading all of its input.
        Err(e) if e.kind() != std::io::ErrorKind::BrokenPipe => Err(e),
        _ => output,
    }
}

pub async fn process_run(req: &rt::ProcessRunRequest) -> Result<rt::ProcessRunResponse, String> {
    if req.args.is_empty() {
        return Err("empty args".to_string());
    }
    let program = &req.args[0];
    let program_args = &req.args[1..];
    let options = req.options.as_option();
    let cmd = build_command(program, program_args, options);
    let output = output_with_input(cmd, options.and_then(|o| o.input.as_deref()))
        .await
        .map_err(|e| format!("run error: {e}"))?;
    Ok(format_process_output(&output))
}

pub async fn process_system(req: &rt::ProcessSystemRequest) -> Result<rt::ProcessRunResponse, String> {
    // Shell out via the platform's shell: `sh -c` on Unix, `cmd /C` on Windows
    // (there is no `sh` on a stock Windows install), unless `shell` names one.
    let (default_shell, flag) = if cfg!(windows) { ("cmd", "/C") } else { ("sh", "-c") };
    let options = req.options.as_option();
    let shell = options.and_then(|o| o.shell.as_deref()).unwrap_or(default_shell);
    let cmd = build_command(shell, &[flag.to_string(), req.command.clone()], options);
    let output = output_with_input(cmd, options.and_then(|o| o.input.as_deref()))
        .await
        .map_err(|e| format!("system error: {e}"))?;
    Ok(format_process_output(&output))
}

pub async fn process_create(state: SharedRpcState, req: &rt::ProcessCreateRequest) -> Result<rt::ProcessCreateResponse, String> {
    if req.args.is_empty() {
        return Err("empty args".to_string());
    }
    if req.options.as_option().is_some_and(|o| o.input.is_some()) {
        return Err("input is for run and system; write to the handle's stdin stream instead".to_string());
    }
    let program = &req.args[0];
    let program_args = &req.args[1..];
    let is_piped = req
        .options
        .as_option()
        .and_then(|o| o.stdio.as_deref())
        .map(|s| s == "piped")
        .unwrap_or(false);

    let mut cmd = build_command(program, program_args, req.options.as_option());
    if is_piped {
        cmd.stdin(std::process::Stdio::piped());
        cmd.stdout(std::process::Stdio::piped());
        cmd.stderr(std::process::Stdio::piped());
    }
    let mut child = cmd.spawn().map_err(|e| format!("create error: {e}"))?;

    let mut guard = state.lock().await;
    guard.next_pid += 1;
    let pid = guard.next_pid.to_string();

    let mut resp = rt::ProcessCreateResponse {
        pid: pid.clone(),
        ..Default::default()
    };

    if is_piped {
        let stdin_handle = format!("proc:{pid}:stdin");
        let stdout_handle = format!("proc:{pid}:stdout");
        let stderr_handle = format!("proc:{pid}:stderr");

        let (stdin, stdout, stderr) = (child.stdin.take(), child.stdout.take(), child.stderr.take());
        let (Some(stdin), Some(stdout), Some(stderr)) = (stdin, stdout, stderr) else {
            return Err("create error: piped stdio not available".to_string());
        };
        let pipe = |reader: Box<dyn tokio::io::AsyncRead + Send + Unpin>| std::sync::Arc::new(tokio::sync::Mutex::new(reader));
        guard.stream_handlers.insert(stdin_handle.clone(), StreamHandler::ProcessStdin { stdin: std::sync::Arc::new(tokio::sync::Mutex::new(stdin)) });
        guard.stream_handlers.insert(stdout_handle.clone(), StreamHandler::ProcessStdout { stdout: pipe(Box::new(stdout)) });
        guard.stream_handlers.insert(stderr_handle.clone(), StreamHandler::ProcessStderr { stderr: pipe(Box::new(stderr)) });

        resp.stdin_handle = Some(stdin_handle);
        resp.stdout_handle = Some(stdout_handle);
        resp.stderr_handle = Some(stderr_handle);
    }

    guard.child_processes.insert(pid, watch_child(child));
    Ok(resp)
}

/// Hand `child` to a task that waits for it, and kills it when asked.
fn watch_child(mut child: tokio::process::Child) -> ChildProcess {
    let (kill, mut kill_requests) = tokio::sync::mpsc::unbounded_channel::<()>();
    let (exited, exit) = tokio::sync::watch::channel(None);
    tokio::spawn(async move {
        let status = tokio::select! {
            status = child.wait() => status,
            // Disabled, not fired, once every sender is gone (the run ended):
            // the child is left running, as before.
            Some(()) = kill_requests.recv() => {
                let _ = child.start_kill();
                child.wait().await
            }
        };
        let _ = exited.send(Some(status.map_err(|e| format!("wait error: {e}"))));
    });
    ChildProcess { kill, exit }
}

/// Wait for a child started by `create` to exit. Any number of callers may
/// wait, before or after it exits. Its output is not captured here: a piped
/// child's output is read through its stream handles, and an unpiped child
/// writes straight to this process's stdout and stderr.
pub async fn process_run_handle(state: SharedRpcState, req: &rt::ProcessRunHandleRequest) -> Result<rt::ProcessRunResponse, String> {
    let mut exit = {
        let guard = state.lock().await;
        guard
            .child_processes
            .get(&req.pid)
            .ok_or_else(|| format!("unknown pid: {}", req.pid))?
            .exit
            .clone()
    };
    let exited = exit.wait_for(Option::is_some).await.map_err(|_| "wait error: child task ended".to_string())?;
    let status = exited.as_ref().expect("waited for Some").clone()?;
    Ok(format_status(status))
}

pub async fn process_kill(state: SharedRpcState, req: &rt::ProcessKillRequest) -> Result<rt::Ok, String> {
    let guard = state.lock().await;
    if let Some(child) = guard.child_processes.get(&req.pid) {
        let _ = child.kill.send(());
    }
    Ok(rt::Ok::default())
}
