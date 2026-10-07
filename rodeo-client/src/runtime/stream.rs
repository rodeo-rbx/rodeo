use super::{ChildInput, ChildOutput, SharedRpcState, StreamHandler};
use rodeo_proto::runtime_types as rt;
use std::io::{BufRead, Read};

const DEFAULT_CHUNK_SIZE: u32 = 4096;

pub async fn stream_open(state: SharedRpcState, req: &rt::StreamOpenRequest) -> Result<rt::StreamOpenResponse, String> {
    let handler = match req.mode.as_str() {
        "r" => {
            let file = std::fs::File::open(&req.path).map_err(|e| format!("open error: {e}"))?;
            StreamHandler::FileReader {
                reader: Box::new(std::io::BufReader::new(file)),
            }
        }
        "w" => StreamHandler::FileWriter {
            path: req.path.clone(),
            buffer: Vec::new(),
        },
        "a" => {
            let existing = if std::path::Path::new(&req.path).is_file() {
                std::fs::read(&req.path).unwrap_or_default()
            } else {
                Vec::new()
            };
            StreamHandler::FileAppender {
                path: req.path.clone(),
                buffer: existing,
            }
        }
        m => return Err(format!("invalid mode: {m}")),
    };
    state.lock().await.stream_handlers.insert(req.handle.clone(), handler);
    Ok(rt::StreamOpenResponse { handle: req.handle.clone(), ..Default::default() })
}

pub async fn stream_read_chunk(state: SharedRpcState, req: &rt::StreamReadChunkRequest) -> Result<rt::StreamReadChunkResponse, String> {
    let size = req.size.unwrap_or(DEFAULT_CHUNK_SIZE) as usize;

    if req.handle == "stdin" {
        return tokio::task::spawn_blocking(move || {
            let mut buf = vec![0u8; size];
            let n = std::io::stdin().lock().read(&mut buf).map_err(|e| format!("stdin read: {e}"))?;
            Ok(rt::StreamReadChunkResponse {
                data: String::from_utf8_lossy(&buf[..n]).to_string(),
                eof: n == 0,
                ..Default::default()
            })
        })
        .await
        .map_err(|e| format!("task error: {e}"))?;
    }

    if let Some(output) = child_output(&state, &req.handle).await {
        use tokio::io::AsyncReadExt;
        let mut chunk = vec![0u8; size];
        let n = output.lock().await.read(&mut chunk).await.map_err(|e| format!("read error: {e}"))?;
        return Ok(rt::StreamReadChunkResponse {
            data: String::from_utf8_lossy(&chunk[..n]).to_string(),
            eof: n == 0,
            ..Default::default()
        });
    }

    let mut guard = state.lock().await;
    let handler = guard
        .stream_handlers
        .get_mut(&req.handle)
        .ok_or_else(|| format!("no reader for handle: {}", req.handle))?;

    match handler {
        StreamHandler::FileReader { reader } => {
            let mut chunk = vec![0u8; size];
            let n = reader.read(&mut chunk).map_err(|e| format!("read error: {e}"))?;
            Ok(rt::StreamReadChunkResponse {
                data: String::from_utf8_lossy(&chunk[..n]).to_string(),
                eof: n == 0,
                ..Default::default()
            })
        }
        _ => Err(format!("handle not readable: {}", req.handle)),
    }
}

pub async fn stream_read_line(state: SharedRpcState, req: &rt::StreamReadLineRequest) -> Result<rt::StreamReadLineResponse, String> {
    if req.handle == "stdin" {
        return tokio::task::spawn_blocking(|| {
            let mut line = String::new();
            let n = std::io::stdin().lock().read_line(&mut line).map_err(|e| format!("stdin read: {e}"))?;
            strip_newline(&mut line);
            Ok(rt::StreamReadLineResponse { data: line, eof: n == 0, ..Default::default() })
        })
        .await
        .map_err(|e| format!("task error: {e}"))?;
    }

    if let Some(output) = child_output(&state, &req.handle).await {
        return read_line_async(&mut *output.lock().await).await;
    }

    let mut guard = state.lock().await;
    let handler = guard
        .stream_handlers
        .get_mut(&req.handle)
        .ok_or_else(|| format!("no reader for handle: {}", req.handle))?;

    match handler {
        StreamHandler::FileReader { reader } => {
            let mut line = String::new();
            let n = reader.read_line(&mut line).map_err(|e| format!("read error: {e}"))?;
            strip_newline(&mut line);
            Ok(rt::StreamReadLineResponse { data: line, eof: n == 0, ..Default::default() })
        }
        _ => Err(format!("handle not readable: {}", req.handle)),
    }
}

pub async fn stream_read_all(state: SharedRpcState, req: &rt::StreamReadAllRequest) -> Result<rt::StreamReadAllResponse, String> {
    if req.handle == "stdin" {
        return tokio::task::spawn_blocking(|| {
            let mut buf = String::new();
            std::io::stdin().lock().read_to_string(&mut buf).map_err(|e| format!("stdin read: {e}"))?;
            Ok(rt::StreamReadAllResponse { data: buf, ..Default::default() })
        })
        .await
        .map_err(|e| format!("task error: {e}"))?;
    }

    if let Some(output) = child_output(&state, &req.handle).await {
        use tokio::io::AsyncReadExt;
        let mut buf = String::new();
        output.lock().await.read_to_string(&mut buf).await.map_err(|e| format!("read error: {e}"))?;
        return Ok(rt::StreamReadAllResponse { data: buf, ..Default::default() });
    }

    let mut guard = state.lock().await;
    let handler = guard
        .stream_handlers
        .get_mut(&req.handle)
        .ok_or_else(|| format!("no reader for handle: {}", req.handle))?;

    match handler {
        StreamHandler::FileReader { reader } => {
            let mut buf = String::new();
            reader.read_to_string(&mut buf).map_err(|e| format!("read error: {e}"))?;
            Ok(rt::StreamReadAllResponse { data: buf, ..Default::default() })
        }
        _ => Err(format!("handle not readable: {}", req.handle)),
    }
}

pub async fn stream_write(state: SharedRpcState, req: &rt::StreamWriteRequest) -> Result<rt::Ok, String> {
    if let Some(stdin) = child_input(&state, &req.handle).await {
        return write_child_input(&stdin, req.data.as_bytes()).await;
    }
    let mut guard = state.lock().await;
    // Capture the sender up-front — `handler` below holds a mutable borrow
    // on stream_handlers that would conflict with a later `guard.` access.
    let captured_tx = guard.captured_output_tx.clone();
    if let Some(handler) = guard.stream_handlers.get_mut(&req.handle) {
        match handler {
            StreamHandler::Stdout => {
                let _ = captured_tx.send((super::CapturedStreamKind::Stdout, req.data.as_bytes().to_vec()));
            }
            StreamHandler::Stderr => {
                let _ = captured_tx.send((super::CapturedStreamKind::Stderr, req.data.as_bytes().to_vec()));
            }
            StreamHandler::FileWriter { buffer, .. } => {
                buffer.extend_from_slice(req.data.as_bytes());
            }
            StreamHandler::FileAppender { buffer, .. } => {
                buffer.extend_from_slice(req.data.as_bytes());
            }
            _ => {
                tracing::debug!("stream.write: no writer for '{}'", req.handle);
            }
        }
    } else {
        tracing::debug!("stream.write: no handler for '{}'", req.handle);
    }
    Ok(rt::Ok::default())
}

// `size` caps one response so callers can pull a large file in chunks that fit
// the transport (an uncapped read of a big file overflows the connectrpc
// envelope and kills the run stream). `eof` is true when the source is
// exhausted: a read shorter than `size` (read_to_end past a `take` only stops
// early at EOF), or any uncapped read.
pub async fn stream_read_bytes(state: SharedRpcState, req: &rt::StreamReadBytesRequest) -> Result<rt::StreamReadBytesResponse, String> {
    let size = req.size.map(|s| s as u64);
    let response = move |data: Vec<u8>| {
        let eof = match size {
            Some(s) => (data.len() as u64) < s,
            None => true,
        };
        rt::StreamReadBytesResponse { data, eof, ..Default::default() }
    };

    if req.handle == "stdin" {
        return tokio::task::spawn_blocking(move || {
            let mut buf = Vec::new();
            let stdin = std::io::stdin();
            match size {
                Some(s) => stdin.lock().take(s).read_to_end(&mut buf),
                None => stdin.lock().read_to_end(&mut buf),
            }
            .map_err(|e| format!("stdin read: {e}"))?;
            Ok(response(buf))
        })
        .await
        .map_err(|e| format!("task error: {e}"))?;
    }

    if let Some(output) = child_output(&state, &req.handle).await {
        use tokio::io::AsyncReadExt;
        let mut reader = output.lock().await;
        let mut buf = Vec::new();
        match size {
            Some(s) => (&mut *reader).take(s).read_to_end(&mut buf).await,
            None => reader.read_to_end(&mut buf).await,
        }
        .map_err(|e| format!("read error: {e}"))?;
        return Ok(response(buf));
    }

    let mut guard = state.lock().await;
    let handler = guard
        .stream_handlers
        .get_mut(&req.handle)
        .ok_or_else(|| format!("no reader for handle: {}", req.handle))?;

    match handler {
        StreamHandler::FileReader { reader } => {
            let mut buf = Vec::new();
            match size {
                Some(s) => reader.take(s).read_to_end(&mut buf),
                None => reader.read_to_end(&mut buf),
            }
            .map_err(|e| format!("read error: {e}"))?;
            Ok(response(buf))
        }
        _ => Err(format!("handle not readable: {}", req.handle)),
    }
}

pub async fn stream_write_bytes(state: SharedRpcState, req: &rt::StreamWriteBytesRequest) -> Result<rt::Ok, String> {
    if let Some(stdin) = child_input(&state, &req.handle).await {
        return write_child_input(&stdin, &req.data).await;
    }
    let mut guard = state.lock().await;
    let captured_tx = guard.captured_output_tx.clone();
    if let Some(handler) = guard.stream_handlers.get_mut(&req.handle) {
        match handler {
            StreamHandler::Stdout => {
                let _ = captured_tx.send((super::CapturedStreamKind::Stdout, req.data.clone()));
            }
            StreamHandler::Stderr => {
                let _ = captured_tx.send((super::CapturedStreamKind::Stderr, req.data.clone()));
            }
            StreamHandler::FileWriter { buffer, .. } => {
                buffer.extend_from_slice(&req.data);
            }
            StreamHandler::FileAppender { buffer, .. } => {
                buffer.extend_from_slice(&req.data);
            }
            _ => {
                tracing::debug!("stream.writeBytes: no writer for '{}'", req.handle);
            }
        }
    } else {
        tracing::debug!("stream.writeBytes: no handler for '{}'", req.handle);
    }
    Ok(rt::Ok::default())
}

pub async fn stream_close(state: SharedRpcState, req: &rt::StreamCloseRequest) -> Result<rt::Ok, String> {
    let mut guard = state.lock().await;
    if let Some(handler) = guard.stream_handlers.remove(&req.handle) {
        if req.discard { return Ok(rt::Ok::default()); }
        match handler {
            StreamHandler::FileWriter { path, buffer } => {
                std::fs::write(&path, &buffer).map_err(|e| format!("write error: {e}"))?;
            }
            StreamHandler::FileAppender { path, buffer } => {
                std::fs::write(&path, &buffer).map_err(|e| format!("write error: {e}"))?;
            }
            _ => {}
        }
    }
    Ok(rt::Ok::default())
}

/// Remove and return a FileWriter's accumulated (path, buffer) so a consumer
/// can post-process the bytes instead of flushing them verbatim (used by
/// `roblox_export` finalize in place of `stream_close`). This is the only
/// sanctioned way for another module to end a stream handle — keeps
/// `stream_handlers` encapsulated here. A handle of any other kind is left
/// untouched.
pub async fn take_file_writer(state: &SharedRpcState, handle: &str) -> Result<(String, Vec<u8>), String> {
    let mut guard = state.lock().await;
    match guard.stream_handlers.get(handle) {
        Some(StreamHandler::FileWriter { .. }) => {}
        Some(_) => return Err(format!("handle is not an open file writer: {handle}")),
        None => return Err(format!("no open file handle: {handle}")),
    }
    match guard.stream_handlers.remove(handle) {
        Some(StreamHandler::FileWriter { path, buffer }) => Ok((path, buffer)),
        _ => unreachable!("checked above while holding the lock"),
    }
}

// --- helpers ---

/// The pipe behind a child process's stdout or stderr handle. Only the lookup
/// holds the RPC state; the read itself waits on the pipe's own lock.
async fn child_output(state: &SharedRpcState, handle: &str) -> Option<ChildOutput> {
    match state.lock().await.stream_handlers.get(handle) {
        Some(StreamHandler::ProcessStdout { stdout }) => Some(stdout.clone()),
        Some(StreamHandler::ProcessStderr { stderr }) => Some(stderr.clone()),
        _ => None,
    }
}

/// The pipe behind a child process's stdin handle (see [`child_output`]).
async fn child_input(state: &SharedRpcState, handle: &str) -> Option<ChildInput> {
    match state.lock().await.stream_handlers.get(handle) {
        Some(StreamHandler::ProcessStdin { stdin }) => Some(stdin.clone()),
        _ => None,
    }
}

/// Write to a child's stdin. Fails once the child has exited or been killed
/// (a closed pipe), rather than dropping the data silently.
async fn write_child_input(stdin: &ChildInput, data: &[u8]) -> Result<rt::Ok, String> {
    use tokio::io::AsyncWriteExt;
    let mut writer = stdin.lock().await;
    writer.write_all(data).await.map_err(|e| format!("write error: {e}"))?;
    writer.flush().await.map_err(|e| format!("write error: {e}"))?;
    Ok(rt::Ok::default())
}

fn strip_newline(s: &mut String) {
    if s.ends_with('\n') { s.pop(); }
    if s.ends_with('\r') { s.pop(); }
}

async fn read_line_async<R: tokio::io::AsyncRead + Unpin>(reader: &mut R) -> Result<rt::StreamReadLineResponse, String> {
    use tokio::io::AsyncReadExt;
    let mut line = Vec::new();
    let mut byte = [0u8; 1];
    loop {
        match reader.read(&mut byte).await {
            Ok(0) => {
                return Ok(rt::StreamReadLineResponse {
                    data: String::from_utf8_lossy(&line).to_string(),
                    eof: true,
                    ..Default::default()
                });
            }
            Ok(_) => {
                if byte[0] == b'\n' {
                    if line.last() == Some(&b'\r') { line.pop(); }
                    return Ok(rt::StreamReadLineResponse {
                        data: String::from_utf8_lossy(&line).to_string(),
                        eof: false,
                        ..Default::default()
                    });
                }
                line.push(byte[0]);
            }
            Err(e) => return Err(format!("read error: {e}")),
        }
    }
}

#[cfg(test)]
mod close_tests {
    use super::*;
    use crate::runtime::RpcState;
    use std::sync::Arc;
    use tokio::sync::Mutex;

    #[tokio::test]
    async fn discard_releases_writer_without_committing_partial_codec_payload() {
        let path = std::env::temp_dir().join(format!("rodeo-abort-{}", uuid::Uuid::new_v4()));
        std::fs::write(&path, b"original destination").unwrap();
        let (tx, _) = tokio::sync::mpsc::unbounded_channel();
        let state = Arc::new(Mutex::new(RpcState::new(tx)));
        state.lock().await.stream_handlers.insert("codec".into(), StreamHandler::FileWriter {
            path: path.to_string_lossy().into_owned(), buffer: b"incomplete transport packet".to_vec(),
        });
        let request = rt::StreamCloseRequest { handle: "codec".into(), discard: true, ..Default::default() };
        stream_close(state.clone(), &request).await.unwrap();
        assert!(!state.lock().await.stream_handlers.contains_key("codec"));
        assert_eq!(std::fs::read(&path).unwrap(), b"original destination");
        // The encoder may already have consumed the handle before cleanup.
        stream_close(state.clone(), &request).await.unwrap();
        std::fs::remove_file(path).unwrap();
    }
}
