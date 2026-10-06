use super::{stream, SharedRpcState};
use rodeo_proto::runtime_types as rt;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

/// Finalize a `roblox.exportInstances`. The plugin has streamed the binary bytes from
/// `SerializeInstancesAsync` into a FileWriter via chunked
/// `stream.writeBytes`; this consumes the handle in place of `stream_close`.
/// If the destination ends in `.rbxmx`/`.rbxlx`, re-serialize the binary DOM
/// as XML via rbx-binary → rbx-xml; otherwise write the binary bytes through.
/// Writes atomically (`.tmp` + rename) so a failed export leaves no partial
/// file.
pub async fn roblox_export(state: SharedRpcState, req: &rt::RobloxExportRequest) -> Result<rt::Ok, String> {
    let (path, buffer) = stream::take_file_writer(&state, &req.handle).await?;

    let lower = path.to_lowercase();
    let is_xml = lower.ends_with(".rbxmx") || lower.ends_with(".rbxlx");

    let bytes_to_write: Vec<u8> = if is_xml {
        let dom = rbx_binary::from_reader(buffer.as_slice())
            .map_err(|e| format!("rbx-binary decode: {e}"))?;
        let root_refs: Vec<_> = dom.root().children().to_vec();
        let mut out = Vec::new();
        rbx_xml::to_writer_default(&mut out, &dom, &root_refs)
            .map_err(|e| format!("rbx-xml encode: {e}"))?;
        out
    } else {
        buffer
    };

    if let Some(parent) = std::path::Path::new(&path).parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("create parent dirs for {}: {e}", parent.display()))?;
        }
    }

    let tmp = format!("{path}.tmp");
    std::fs::write(&tmp, &bytes_to_write).map_err(|e| format!("write {tmp}: {e}"))?;
    std::fs::rename(&tmp, &path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        format!("rename {tmp} -> {path}: {e}")
    })?;

    Ok(rt::Ok::default())
}

/// Finalize a capture: `rgba` is the `source_width` x `source_height` RGBA8
/// frame decoded from the engine's capture file, `(width, height)` the
/// viewport it must be a whole multiple of and the size written to `output`.
///
/// On a high-DPI display the frame is the viewport times the display scale
/// (2x on Retina); a frame from before a viewport change is not a multiple at
/// all — the stale-frame case, an error rather than a retry. The image is
/// resampled to exactly the viewport, so a capture has the same pixel size on
/// every machine and offset-based UI maps 1:1 onto pixels, then PNG-encoded
/// and written atomically.
fn finalize_pixels(
    rgba: Vec<u8>,
    source_width: u32,
    source_height: u32,
    width: u32,
    height: u32,
    keep_frame: bool,
    output: &str,
) -> Result<rt::RobloxCaptureCollectResponse, String> {
    use fast_image_resize::images::Image;
    use fast_image_resize::{FilterType, PixelType, ResizeAlg, ResizeOptions, Resizer};

    if width == 0 || height == 0 {
        return Err(format!("capture finalize: invalid viewport {width}x{height}"));
    }
    let expected_len = source_width as usize * source_height as usize * 4;
    if source_width == 0 || source_height == 0 || rgba.len() != expected_len {
        return Err(format!(
            "capture finalize: frame buffer is {} bytes, expected {expected_len} for a {source_width}x{source_height} RGBA8 frame",
            rgba.len()
        ));
    }

    // A genuine frame is the viewport times one display scale on both axes.
    let rx = source_width as f64 / width as f64;
    let ry = source_height as f64 / height as f64;
    if rx < 0.995 || ry < 0.995 || (rx - ry).abs() > 0.02 {
        return Err(format!(
            "captured frame is {source_width}x{source_height}, not a whole multiple of the {width}x{height} viewport \
             ({rx:.2}x by {ry:.2}x). Studio handed back a frame rendered before the viewport changed; \
             raise `settle` so the new size has rendered before the capture."
        ));
    }

    // The engine can report success for a frame it never rendered: on macOS,
    // Studio 0.739 returned solo play-test captures all zero (issue #17; a
    // real frame by 0.741.19), and v1.3.0 wrote those as PNGs that read as
    // "my scene is black". Refuse an all-black frame rather than write it.
    if rgba.chunks_exact(4).all(|p| p[0] == 0 && p[1] == 0 && p[2] == 0) {
        return Err(
            "captured frame is entirely black: every pixel Studio returned is black, so nothing was \
             written. Either Studio did not render this capture (older Studio builds returned black \
             frames in solo play-test sessions on macOS) or everything in view is black."
                .to_string(),
        );
    }

    let pixels: Vec<u8> = if keep_frame || (source_width, source_height) == (width, height) {
        rgba
    } else {
        let src = Image::from_vec_u8(source_width, source_height, rgba, PixelType::U8x4)
            .map_err(|e| format!("capture resize source: {e}"))?;
        let mut dst = Image::new(width, height, PixelType::U8x4);
        Resizer::new()
            .resize(
                &src,
                &mut dst,
                &ResizeOptions::new().resize_alg(ResizeAlg::Convolution(FilterType::Lanczos3)),
            )
            .map_err(|e| format!("capture resize: {e}"))?;
        dst.into_vec()
    };

    // `keep_frame` writes the engine's frame as rendered (the viewport times the
    // display scale, or whatever scale a caller-driven simulator produced);
    // the default resamples to the viewport so pixels map 1:1 onto UI offsets.
    let (out_width, out_height) = if keep_frame { (source_width, source_height) } else { (width, height) };
    write_png_atomic(&pixels, out_width, out_height, output)?;

    Ok(rt::RobloxCaptureCollectResponse {
        width: out_width,
        height: out_height,
        source_width,
        source_height,
        ..Default::default()
    })
}

// ---------------------------------------------------------------------------
// Captures: the frame from the engine's capture directory.
//
// The engine writes every CaptureService capture as a PNG to a per-user
// directory, named `wob-<pid><6-digit counter>` after the capturing process.
// That file is the frame for every capture, in every DOM: promoting the
// capture's temporary texture into an EditableImage is refused in play DOMs
// ("cannot currently create editable image from temporary texture id"), is
// capped at 8192 pixels a side everywhere (a 2x display's 8K frame is 15360
// wide), and would read the whole RGBA frame through Luau. Files there
// outlive the process, so a frame is identified by a snapshot taken before
// the capture, not by "newest file": exactly one complete PNG that appeared
// or changed since, sized like the viewport at one display scale. Two such
// files (another Studio or run capturing at the same moment) is reported,
// never guessed at. Measured on Studio 0.739: a window-sized frame lands
// within ~2s of the callback, the simulator's largest (23466x13200) in ~7s.
// ---------------------------------------------------------------------------

/// What the capture directory held when a capture began: file name ->
/// (length, mtime). This capture's frame is a file not in here, or whose
/// length or mtime differs.
#[derive(Debug, Clone, Default)]
pub struct CaptureSnapshot {
    pub dir: PathBuf,
    pub files: HashMap<String, (u64, SystemTime)>,
}

/// How long to wait for the engine to write the frame after the callback.
/// Measured at ~2s for a window-sized frame; a 15360x8640 frame on a slow
/// disk or GPU can take far longer to encode, so this is generous. It only
/// fires when the callback ran and no file ever came, which is not a case
/// the engine has shown.
const CAPTURE_FILE_TIMEOUT: Duration = Duration::from_secs(60);

/// The engine's per-user capture directory. `RODEO_CAPTURE_DIR` overrides it
/// (tests, unusual installs).
pub fn capture_dir() -> Result<PathBuf, String> {
    if let Ok(dir) = std::env::var("RODEO_CAPTURE_DIR") {
        if !dir.is_empty() {
            return Ok(PathBuf::from(dir));
        }
    }
    if cfg!(target_os = "macos") {
        let home = std::env::var("HOME").map_err(|_| "capture: HOME is not set".to_string())?;
        Ok(Path::new(&home).join("Library").join("Roblox").join("tmp-capture-storage"))
    } else if cfg!(target_os = "windows") {
        let local = std::env::var("LOCALAPPDATA").map_err(|_| "capture: LOCALAPPDATA is not set".to_string())?;
        Ok(Path::new(&local).join("Roblox").join("tmp-capture-storage"))
    } else {
        Err("roblox.captureViewport in a running session reads the engine's capture directory, \
             which rodeo knows only on macOS and Windows"
            .to_string())
    }
}

fn list_dir(dir: &Path) -> HashMap<String, (u64, SystemTime)> {
    let mut out = HashMap::new();
    let Ok(read_dir) = std::fs::read_dir(dir) else {
        return out;
    };
    for entry in read_dir.flatten() {
        let Ok(meta) = entry.metadata() else {
            continue;
        };
        if !meta.is_file() {
            continue;
        }
        let mtime = meta.modified().unwrap_or(SystemTime::UNIX_EPOCH);
        out.insert(entry.file_name().to_string_lossy().into_owned(), (meta.len(), mtime));
    }
    out
}

/// `roblox.captureBegin`: snapshot the capture directory before the plugin
/// calls CaptureScreenshot, so the matching collect can tell this capture's
/// file from everything that was already there.
pub async fn roblox_capture_begin(
    state: SharedRpcState,
    _req: &rt::RobloxCaptureBeginRequest,
) -> Result<rt::RobloxCaptureBeginResponse, String> {
    let dir = capture_dir()?;
    let files = {
        let dir = dir.clone();
        tokio::task::spawn_blocking(move || list_dir(&dir))
            .await
            .map_err(|e| format!("capture begin task failed: {e}"))?
    };
    let mut guard = state.lock().await;
    guard.next_capture_token += 1;
    let token = format!("capture-{}", guard.next_capture_token);
    guard.capture_snapshots.insert(token.clone(), CaptureSnapshot { dir, files });
    Ok(rt::RobloxCaptureBeginResponse { token, ..Default::default() })
}

/// `roblox.captureCollect`: wait for this capture's frame to appear in the
/// snapshotted directory, then finalize it (scale check, all-black rejection,
/// resample to the viewport, PNG).
pub async fn roblox_capture_collect(
    state: SharedRpcState,
    req: &rt::RobloxCaptureCollectRequest,
) -> Result<rt::RobloxCaptureCollectResponse, String> {
    let snapshot = state
        .lock()
        .await
        .capture_snapshots
        .remove(&req.token)
        .ok_or_else(|| format!("capture collect: unknown capture token {}", req.token))?;
    let (width, height, keep_frame, output) = (req.width, req.height, req.keep_frame, req.output.clone());
    let deadline = tokio::time::Instant::now() + CAPTURE_FILE_TIMEOUT;
    loop {
        let snap = snapshot.clone();
        let scanned = tokio::task::spawn_blocking(move || scan_for_frame(&snap, width, height))
            .await
            .map_err(|e| format!("capture collect task failed: {e}"))?;
        match scanned {
            Err(e) => return Err(e),
            Ok(Some(frame)) => {
                let out = output.clone();
                return tokio::task::spawn_blocking(move || {
                    finalize_pixels(frame.rgba, frame.width, frame.height, width, height, keep_frame, &out)
                })
                .await
                .map_err(|e| format!("capture finalize task failed: {e}"))?;
            }
            Ok(None) => {}
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(format!(
                "roblox.captureViewport: the capture callback fired but no new frame appeared in {} within {}s \
                 (expected a PNG about {width}x{height} at the display scale). In a running session rodeo \
                 reads the frame Studio writes to that directory; make sure the Studio window is rendering.",
                snapshot.dir.display(),
                CAPTURE_FILE_TIMEOUT.as_secs()
            ));
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// A decoded frame from the capture directory.
#[derive(Debug)]
struct Frame {
    rgba: Vec<u8>,
    width: u32,
    height: u32,
}

/// A file in the capture directory as seen while collecting. `png_size` is
/// the IHDR size when the file starts like a PNG.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaptureEntry {
    pub name: String,
    pub len: u64,
    pub mtime: SystemTime,
    pub png_size: Option<(u32, u32)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Selection {
    None,
    One(String),
    Ambiguous(Vec<String>),
}

/// Which of `entries` is this capture's frame: not in `snapshot` (or changed
/// since), a PNG, and sized like the `width` x `height` viewport at one
/// display scale. Pure, so the rule is unit-tested without a Studio.
pub fn select_capture_file(
    snapshot: &HashMap<String, (u64, SystemTime)>,
    entries: &[CaptureEntry],
    width: u32,
    height: u32,
) -> Selection {
    let mut names: Vec<String> = entries
        .iter()
        .filter(|e| {
            let is_new = match snapshot.get(&e.name) {
                None => true,
                Some(&(len, mtime)) => len != e.len || mtime != e.mtime,
            };
            is_new && e.png_size.is_some_and(|(fw, fh)| frame_matches_viewport(fw, fh, width, height))
        })
        .map(|e| e.name.clone())
        .collect();
    names.sort();
    match names.len() {
        0 => Selection::None,
        1 => Selection::One(names.remove(0)),
        _ => Selection::Ambiguous(names),
    }
}

/// A genuine frame is the viewport times one display scale on both axes (2x on
/// Retina); the same tolerance `finalize_pixels` applies.
fn frame_matches_viewport(frame_width: u32, frame_height: u32, width: u32, height: u32) -> bool {
    if width == 0 || height == 0 {
        return false;
    }
    let rx = frame_width as f64 / width as f64;
    let ry = frame_height as f64 / height as f64;
    rx >= 0.995 && ry >= 0.995 && (rx - ry).abs() <= 0.02
}

const PNG_SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
/// A complete PNG ends with its IEND chunk: zero length, "IEND", fixed CRC.
const PNG_IEND: [u8; 12] = [0, 0, 0, 0, b'I', b'E', b'N', b'D', 0xAE, 0x42, 0x60, 0x82];

/// Width and height from a PNG's IHDR, when `bytes` start like a PNG.
pub fn png_ihdr_size(bytes: &[u8]) -> Option<(u32, u32)> {
    if bytes.len() < 24 || bytes[..8] != PNG_SIGNATURE || &bytes[12..16] != b"IHDR" {
        return None;
    }
    let width = u32::from_be_bytes([bytes[16], bytes[17], bytes[18], bytes[19]]);
    let height = u32::from_be_bytes([bytes[20], bytes[21], bytes[22], bytes[23]]);
    Some((width, height))
}

fn read_ihdr(path: &Path) -> Option<(u32, u32)> {
    use std::io::Read;
    let mut file = std::fs::File::open(path).ok()?;
    let mut head = [0u8; 33];
    let n = file.read(&mut head).ok()?;
    png_ihdr_size(&head[..n])
}

/// One pass over the capture directory. `Ok(Some(frame))` when exactly one new
/// matching PNG is present and complete; `Ok(None)` when none has appeared yet
/// or the one candidate is still being written; `Err` when several appeared.
fn scan_for_frame(snapshot: &CaptureSnapshot, width: u32, height: u32) -> Result<Option<Frame>, String> {
    let entries: Vec<CaptureEntry> = list_dir(&snapshot.dir)
        .into_iter()
        .map(|(name, (len, mtime))| {
            // Only files new since the snapshot are worth opening.
            let unchanged = snapshot.files.get(&name) == Some(&(len, mtime));
            let png_size = if unchanged { None } else { read_ihdr(&snapshot.dir.join(&name)) };
            CaptureEntry { name, len, mtime, png_size }
        })
        .collect();
    match select_capture_file(&snapshot.files, &entries, width, height) {
        Selection::None => Ok(None),
        Selection::Ambiguous(names) => Err(format!(
            "roblox.captureViewport: {} captures appeared in {} at the same time ({}); another Studio or run \
             captured concurrently, so this run's frame cannot be told apart. Retry the capture.",
            names.len(),
            snapshot.dir.display(),
            names.join(", ")
        )),
        Selection::One(name) => {
            let path = snapshot.dir.join(&name);
            let Ok(bytes) = std::fs::read(&path) else {
                return Ok(None);
            };
            // Still being written: no IEND yet. Try again.
            if !bytes.ends_with(&PNG_IEND) {
                return Ok(None);
            }
            // The crate's default allocation cap is 512 MiB, which a 2x 8K frame
            // just clears and the simulator's 3x frames (up to 23466x13200,
            // 929 MB of RGB) do not; the file comes from the engine and its
            // size is bounded by the simulator, so decode without a cap. A
            // complete file the decoder still rejects is an error, not a wait.
            let mut reader = image::ImageReader::new(std::io::Cursor::new(&bytes))
                .with_guessed_format()
                .map_err(|e| format!("roblox.captureViewport: could not read {}: {e}", path.display()))?;
            reader.limits(image::Limits::no_limits());
            let decoded = reader
                .decode()
                .map_err(|e| format!("roblox.captureViewport: the engine's capture file {} did not decode: {e}", path.display()))?;
            let (width, height) = (decoded.width(), decoded.height());
            Ok(Some(Frame { rgba: decoded.into_rgba8().into_raw(), width, height }))
        }
    }
}

/// Encode `width` x `height` RGBA8 pixels as PNG and write them to `output`
/// atomically (`.tmp` + rename), creating parent directories. Fast compression:
/// these files are large and consumed locally.
fn write_png_atomic(pixels: &[u8], width: u32, height: u32, output: &str) -> Result<(), String> {
    use image::ImageEncoder;

    let mut png = Vec::new();
    image::codecs::png::PngEncoder::new_with_quality(
        &mut png,
        image::codecs::png::CompressionType::Fast,
        image::codecs::png::FilterType::Adaptive,
    )
    .write_image(pixels, width, height, image::ExtendedColorType::Rgba8)
    .map_err(|e| format!("encode png: {e}"))?;

    if let Some(parent) = std::path::Path::new(output).parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("create parent dirs for {}: {e}", parent.display()))?;
        }
    }
    let tmp = format!("{output}.tmp");
    std::fs::write(&tmp, &png).map_err(|e| format!("write {tmp}: {e}"))?;
    std::fs::rename(&tmp, output).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        format!("rename {tmp} -> {output}: {e}")
    })?;
    Ok(())
}

/// `roblox.exportEditableImage`: the plugin streamed an EditableImage's RGBA8
/// pixels into a FileWriter on the output path; consume the handle and write
/// the file as PNG. Only `.png` is supported (an EditableImage always carries
/// alpha, and PNG is what the Roblox side round-trips losslessly).
pub async fn roblox_image_encode(state: SharedRpcState, req: &rt::RobloxImageEncodeRequest) -> Result<rt::Ok, String> {
    let (path, rgba) = stream::take_file_writer(&state, &req.handle).await?;
    let (width, height) = (req.width, req.height);
    if !path.to_lowercase().ends_with(".png") {
        return Err(format!("exportEditableImage: only .png output is supported (got '{path}')"));
    }
    encode_rgba_to_png(rgba, width, height, &path)
}

fn encode_rgba_to_png(rgba: Vec<u8>, width: u32, height: u32, output: &str) -> Result<rt::Ok, String> {
    let expected_len = width as usize * height as usize * 4;
    if width == 0 || height == 0 || rgba.len() != expected_len {
        return Err(format!(
            "exportEditableImage: pixel buffer is {} bytes, expected {expected_len} for a {width}x{height} RGBA8 image",
            rgba.len()
        ));
    }
    write_png_atomic(&rgba, width, height, output)?;
    Ok(rt::Ok::default())
}

/// `roblox.importEditableImage`: decode the image at `path` (PNG or JPEG) to
/// RGBA8 and register the caller-minted `handle` as a reader over those bytes,
/// so the plugin pulls them with ordinary chunked `stream.readBytes` (a single
/// response could not carry a large image) and then closes the handle.
pub async fn roblox_image_decode(
    state: SharedRpcState,
    req: &rt::RobloxImageDecodeRequest,
) -> Result<rt::RobloxImageDecodeResponse, String> {
    let path = req.path.clone();
    let (rgba, width, height) = tokio::task::spawn_blocking(move || decode_image_file(&path))
        .await
        .map_err(|e| format!("image decode task failed: {e}"))??;

    let mut guard = state.lock().await;
    if guard.stream_handlers.contains_key(&req.handle) {
        return Err(format!("handle already open: {}", req.handle));
    }
    guard.stream_handlers.insert(
        req.handle.clone(),
        super::StreamHandler::FileReader { reader: Box::new(std::io::Cursor::new(rgba)) },
    );
    Ok(rt::RobloxImageDecodeResponse { width, height, ..Default::default() })
}

fn decode_image_file(path: &str) -> Result<(Vec<u8>, u32, u32), String> {
    let bytes = std::fs::read(path).map_err(|e| format!("importEditableImage: read {path}: {e}"))?;
    let decoded = image::load_from_memory(&bytes)
        .map_err(|e| format!("importEditableImage: decode {path}: {e} (PNG and JPEG are supported)"))?;
    let (width, height) = (decoded.width(), decoded.height());
    Ok((decoded.into_rgba8().into_raw(), width, height))
}

#[cfg(test)]
mod image_codec_tests {
    use super::*;

    fn pattern(w: u32, h: u32) -> Vec<u8> {
        (0..(w * h * 4) as usize).map(|i| ((i * 7) % 256) as u8).collect()
    }

    #[test]
    fn png_round_trips_pixels_exactly() {
        let dir = std::env::temp_dir().join(format!("rodeo-image-codec-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let out = dir.join("nested").join("img.png");
        let out_s = out.to_string_lossy().into_owned();
        encode_rgba_to_png(pattern(8, 4), 8, 4, &out_s).expect("encode");
        let (rgba, w, h) = decode_image_file(&out_s).expect("decode");
        assert_eq!((w, h), (8, 4));
        assert_eq!(rgba, pattern(8, 4));
    }

    #[test]
    fn encode_rejects_a_short_buffer() {
        let err = encode_rgba_to_png(vec![0; 10], 8, 4, "/nonexistent/x.png").expect_err("short");
        assert!(err.contains("10 bytes") && err.contains("128"), "{err}");
    }

    #[test]
    fn decode_reports_a_missing_file() {
        let err = decode_image_file("/definitely/not/here.png").expect_err("missing");
        assert!(err.contains("not/here.png"), "{err}");
    }
}

#[cfg(test)]
mod capture_finalize_tests {
    use super::*;

    fn frame(w: u32, h: u32) -> Vec<u8> {
        let mut v = Vec::with_capacity((w * h * 4) as usize);
        for y in 0..h {
            for x in 0..w {
                v.extend_from_slice(&[(x % 256) as u8, (y % 256) as u8, 128, 255]);
            }
        }
        v
    }

    fn scratch(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("rodeo-capture-finalize-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn finalize(dir: &std::path::Path, sw: u32, sh: u32, w: u32, h: u32) -> Result<rt::RobloxCaptureCollectResponse, String> {
        let output = dir.join("nested").join("out.png");
        finalize_pixels(frame(sw, sh), sw, sh, w, h, false, &output.to_string_lossy())
    }

    #[test]
    fn retina_frame_is_resampled_to_the_viewport() {
        let dir = scratch("retina");
        let res = finalize(&dir, 400, 200, 200, 100).expect("2x frame finalizes");
        assert_eq!((res.width, res.height, res.source_width, res.source_height), (200, 100, 400, 200));
        let out = image::open(dir.join("nested/out.png")).unwrap();
        assert_eq!((out.width(), out.height()), (200, 100));
    }

    #[test]
    fn exact_frame_is_encoded_as_is() {
        let dir = scratch("exact");
        let res = finalize(&dir, 200, 100, 200, 100).expect("1x frame finalizes");
        assert_eq!((res.width, res.height), (200, 100));
        let out = image::open(dir.join("nested/out.png")).unwrap().into_rgba8();
        assert_eq!((out.width(), out.height()), (200, 100));
        assert_eq!(out.into_raw(), frame(200, 100), "1x pixels round-trip untouched");
    }

    #[test]
    fn stale_frame_with_wrong_aspect_is_an_error() {
        let dir = scratch("stale");
        let err = finalize(&dir, 400, 200, 300, 300).expect_err("non-multiple frame must fail");
        assert!(err.contains("400x200") && err.contains("300x300") && err.contains("settle"), "{err}");
        assert!(!dir.join("nested/out.png").exists(), "nothing written on error");
    }

    #[test]
    fn frame_smaller_than_viewport_is_an_error() {
        let dir = scratch("small");
        let err = finalize(&dir, 100, 50, 200, 100).expect_err("upscaling is never silent");
        assert!(err.contains("100x50"), "{err}");
    }

    #[test]
    fn buffer_length_must_match_the_frame() {
        let dir = scratch("len");
        let output = dir.join("out.png");
        let err = finalize_pixels(vec![0; 100], 10, 10, 10, 10, false, &output.to_string_lossy()).expect_err("short buffer");
        assert!(err.contains("100 bytes") && err.contains("400"), "{err}");
    }
}

#[cfg(test)]
mod capture_collect_tests {
    use super::*;

    fn entry(name: &str, len: u64, secs: u64, size: Option<(u32, u32)>) -> CaptureEntry {
        CaptureEntry {
            name: name.to_string(),
            len,
            mtime: SystemTime::UNIX_EPOCH + Duration::from_secs(secs),
            png_size: size,
        }
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("rodeo-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn ihdr_size_is_read_from_a_png_header() {
        let mut bytes = PNG_SIGNATURE.to_vec();
        bytes.extend_from_slice(&13u32.to_be_bytes());
        bytes.extend_from_slice(b"IHDR");
        bytes.extend_from_slice(&2860u32.to_be_bytes());
        bytes.extend_from_slice(&1686u32.to_be_bytes());
        bytes.extend_from_slice(&[8, 2, 0, 0, 0]);
        assert_eq!(png_ihdr_size(&bytes), Some((2860, 1686)));
        assert_eq!(png_ihdr_size(&bytes[..20]), None);
        assert_eq!(png_ihdr_size(b"not a png at all, definitely not one"), None);
    }

    #[test]
    fn a_new_file_at_the_viewport_scale_is_selected() {
        let snapshot = HashMap::from([("wob-1000".to_string(), (10u64, SystemTime::UNIX_EPOCH))]);
        let entries = vec![
            entry("wob-1000", 10, 0, Some((2860, 1686))),
            entry("wob-1001", 20, 5, Some((2860, 1686))),
        ];
        assert_eq!(select_capture_file(&snapshot, &entries, 1430, 843), Selection::One("wob-1001".into()));
    }

    #[test]
    fn snapshot_files_other_sizes_and_non_pngs_are_ignored() {
        let snapshot = HashMap::from([("wob-1000".to_string(), (10u64, SystemTime::UNIX_EPOCH))]);
        let entries = vec![
            entry("wob-1000", 10, 0, Some((2860, 1686))),
            entry("wob-2000", 30, 5, Some((2548, 1464))),
            entry("notes.txt", 3, 5, None),
        ];
        assert_eq!(select_capture_file(&snapshot, &entries, 1430, 843), Selection::None);
    }

    #[test]
    fn a_snapshot_file_rewritten_since_counts_as_new() {
        let snapshot = HashMap::from([("wob-1000".to_string(), (10u64, SystemTime::UNIX_EPOCH))]);
        let entries = vec![entry("wob-1000", 10, 7, Some((1430, 843)))];
        assert_eq!(select_capture_file(&snapshot, &entries, 1430, 843), Selection::One("wob-1000".into()));
    }

    #[test]
    fn two_new_matching_files_are_ambiguous_not_guessed() {
        let entries = vec![
            entry("wob-2000", 20, 5, Some((2860, 1686))),
            entry("wob-1001", 20, 5, Some((2860, 1686))),
        ];
        assert_eq!(
            select_capture_file(&HashMap::new(), &entries, 1430, 843),
            Selection::Ambiguous(vec!["wob-1001".into(), "wob-2000".into()])
        );
    }

    #[test]
    fn scan_waits_on_a_partial_png_and_decodes_a_complete_one() {
        let dir = scratch("capture-scan");
        let snapshot = CaptureSnapshot { dir: dir.clone(), files: list_dir(&dir) };
        assert!(scan_for_frame(&snapshot, 4, 2).unwrap().is_none(), "empty directory");

        // An 8x4 frame for a 4x2 viewport (2x scale) with one red pixel.
        let mut rgba = vec![0u8; 8 * 4 * 4];
        rgba[0] = 255;
        rgba[3] = 255;
        let path = dir.join("wob-4200000000");
        write_png_atomic(&rgba, 8, 4, path.to_str().unwrap()).unwrap();
        let png = std::fs::read(&path).unwrap();

        std::fs::write(&path, &png[..png.len() - 6]).unwrap();
        assert!(scan_for_frame(&snapshot, 4, 2).unwrap().is_none(), "still being written");

        std::fs::write(&path, &png).unwrap();
        let frame = scan_for_frame(&snapshot, 4, 2).unwrap().expect("complete frame");
        assert_eq!((frame.width, frame.height), (8, 4));
        assert_eq!(&frame.rgba[..4], &[255, 0, 0, 255]);

        // A second new frame of the same size is a concurrent capture: an error.
        std::fs::write(dir.join("wob-4300000000"), &png).unwrap();
        let err = scan_for_frame(&snapshot, 4, 2).expect_err("ambiguous");
        assert!(err.contains("2 captures appeared"), "{err}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn keep_frame_writes_the_engine_frame_at_its_own_size() {
        let dir = scratch("capture-keep");
        let out = dir.join("keep.png");
        let mut rgba = vec![0u8; 8 * 4 * 4];
        rgba[0] = 255;
        rgba[3] = 255;
        // An 8x4 frame for a 4x2 viewport: resampled by default, kept as-is with keep_frame.
        let res = finalize_pixels(rgba.clone(), 8, 4, 4, 2, true, out.to_str().unwrap()).unwrap();
        assert_eq!((res.width, res.height, res.source_width, res.source_height), (8, 4, 8, 4));
        let written = image::open(&out).unwrap();
        assert_eq!((written.width(), written.height()), (8, 4));
        let res = finalize_pixels(rgba, 8, 4, 4, 2, false, out.to_str().unwrap()).unwrap();
        assert_eq!((res.width, res.height), (4, 2));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_complete_but_undecodable_file_is_an_error_not_a_wait() {
        let dir = scratch("capture-corrupt");
        let snapshot = CaptureSnapshot { dir: dir.clone(), files: list_dir(&dir) };
        let mut bytes = PNG_SIGNATURE.to_vec();
        bytes.extend_from_slice(&13u32.to_be_bytes());
        bytes.extend_from_slice(b"IHDR");
        bytes.extend_from_slice(&8u32.to_be_bytes());
        bytes.extend_from_slice(&4u32.to_be_bytes());
        bytes.extend_from_slice(&[8, 6, 0, 0, 0, 0, 0, 0, 0]);
        bytes.extend_from_slice(b"garbage where the image data should be");
        bytes.extend_from_slice(&PNG_IEND);
        std::fs::write(dir.join("wob-4400000000"), &bytes).unwrap();
        let err = scan_for_frame(&snapshot, 4, 2).expect_err("complete but corrupt");
        assert!(err.contains("did not decode"), "{err}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn finalize_refuses_an_all_black_frame_and_writes_nothing() {
        let dir = scratch("capture-black");
        let out = dir.join("black.png");
        let err = finalize_pixels(vec![0u8; 4 * 2 * 4], 4, 2, 4, 2, false, out.to_str().unwrap()).expect_err("black frame");
        assert!(err.contains("entirely black"), "{err}");
        assert!(!out.exists(), "nothing written for a black frame");

        let mut rgba = vec![0u8; 4 * 2 * 4];
        rgba[5] = 1;
        finalize_pixels(rgba, 4, 2, 4, 2, false, out.to_str().unwrap()).expect("one lit pixel is a frame");
        assert!(out.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
