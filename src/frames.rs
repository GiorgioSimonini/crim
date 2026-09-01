//! Background extraction of single preview frames.
//!
//! Seeking is done by spawning `ffmpeg -ss <t> -i <file> -frames:v 1 -f image2pipe`
//! and reading a PNG from its stdout. That call costs tens of milliseconds, far
//! too much to run inside the GUI thread while the user drags the playhead, so
//! it lives in a dedicated worker thread.
//!
//! The worker **coalesces** requests: while it is busy decoding, newer requests
//! pile up in the channel and only the most recent one is honoured. This is what
//! makes scrubbing feel responsive instead of lagging behind by a queue of stale
//! frames.

use std::io::Read;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};

/// "Please decode the frame at `time` of `path`."
#[derive(Debug, Clone)]
pub struct FrameRequest {
    pub path: PathBuf,
    pub time: f64,
    /// Downscale the preview to this width (keeps decoding + upload cheap).
    /// `None` means native resolution.
    pub scale_width: Option<u32>,
}

/// A decoded RGBA frame, ready to be uploaded as a texture.
pub struct DecodedFrame {
    pub time: f64,
    pub width: usize,
    pub height: usize,
    pub rgba: Vec<u8>,
}

/// Handle owned by the GUI: pushes requests, polls results.
pub struct FrameExtractor {
    tx: Sender<FrameRequest>,
    rx: Receiver<Result<DecodedFrame, String>>,
    /// Set while a request is in flight, so the UI can show a subtle hint.
    pending: bool,
}

impl FrameExtractor {
    /// Spawns the worker thread. `on_frame_ready` is called from the worker
    /// whenever a result is available; the GUI uses it to wake up the event
    /// loop (`egui::Context::request_repaint`).
    pub fn new(on_frame_ready: impl Fn() + Send + 'static) -> Self {
        let (req_tx, req_rx) = mpsc::channel::<FrameRequest>();
        let (res_tx, res_rx) = mpsc::channel::<Result<DecodedFrame, String>>();

        std::thread::Builder::new()
            .name("frame-extractor".into())
            .spawn(move || {
                // Blocking recv: the thread sleeps when there is nothing to do.
                while let Ok(first) = req_rx.recv() {
                    // Drop every superseded request (keep only the newest).
                    let mut request = first;
                    loop {
                        match req_rx.try_recv() {
                            Ok(newer) => request = newer,
                            Err(TryRecvError::Empty) => break,
                            Err(TryRecvError::Disconnected) => return,
                        }
                    }

                    let result = extract(&request);
                    if res_tx.send(result).is_err() {
                        return; // GUI is gone
                    }
                    on_frame_ready();
                }
            })
            .expect("failed to spawn frame extractor thread");

        Self { tx: req_tx, rx: res_rx, pending: false }
    }

    /// Asks for a frame. Cheap: it only pushes into a channel.
    pub fn request(&mut self, request: FrameRequest) {
        if self.tx.send(request).is_ok() {
            self.pending = true;
        }
    }

    /// Non-blocking poll. Returns the newest result, if any.
    pub fn poll(&mut self) -> Option<Result<DecodedFrame, String>> {
        let mut last = None;
        while let Ok(result) = self.rx.try_recv() {
            last = Some(result);
        }
        if last.is_some() {
            self.pending = false;
        }
        last
    }

    pub fn is_busy(&self) -> bool {
        self.pending
    }
}

/// Runs ffmpeg once and decodes the PNG it writes on stdout.
fn extract(request: &FrameRequest) -> Result<DecodedFrame, String> {
    let mut cmd = Command::new("ffmpeg");
    cmd.args(["-hide_banner", "-loglevel", "error"])
        // `-ss` *before* `-i` enables fast (indexed) seeking; since ffmpeg 2.1
        // it is still frame-accurate because the decoder re-decodes from the
        // previous keyframe.
        .arg("-ss")
        .arg(format!("{:.3}", request.time.max(0.0)))
        .arg("-i")
        .arg(&request.path)
        .args(["-frames:v", "1"]);

    if let Some(width) = request.scale_width {
        // -2 keeps the aspect ratio and rounds to an even height.
        cmd.args(["-vf", &format!("scale={width}:-2")]);
    }

    // image2pipe + png writes one raw PNG on stdout, no temporary file needed.
    cmd.args(["-f", "image2pipe", "-vcodec", "png", "-"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .stdin(Stdio::null());

    let mut child = cmd
        .spawn()
        .map_err(|e| format!("cannot run `ffmpeg` ({e}). Is it installed and in PATH?"))?;

    let mut png = Vec::new();
    if let Some(mut stdout) = child.stdout.take() {
        stdout.read_to_end(&mut png).map_err(|e| e.to_string())?;
    }
    let mut stderr = String::new();
    if let Some(mut err) = child.stderr.take() {
        let _ = err.read_to_string(&mut stderr);
    }
    let _ = child.wait();

    if png.is_empty() {
        return Err(if stderr.trim().is_empty() {
            "ffmpeg returned no frame at this position".to_string()
        } else {
            stderr.trim().to_string()
        });
    }

    let image = image::load_from_memory(&png)
        .map_err(|e| format!("cannot decode the extracted frame: {e}"))?
        .to_rgba8();

    Ok(DecodedFrame {
        time: request.time,
        width: image.width() as usize,
        height: image.height() as usize,
        rgba: image.into_raw(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// End-to-end check of the extraction pipeline: builds a synthetic clip
    /// with ffmpeg, then decodes one frame out of it. Skipped when ffmpeg is
    /// not installed.
    #[test]
    fn extracts_a_scaled_frame_from_a_generated_clip() {
        if !crate::command::binary_available("ffmpeg") {
            eprintln!("ffmpeg not available, skipping");
            return;
        }
        let clip = std::env::temp_dir().join("crim_test_clip.mp4");
        let status = Command::new("ffmpeg")
            .args(["-y", "-f", "lavfi", "-i", "testsrc=size=640x360:rate=25:duration=2",
                   "-pix_fmt", "yuv420p"])
            .arg(&clip)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .expect("ffmpeg should run");
        assert!(status.success());

        let frame = extract(&FrameRequest { path: clip, time: 1.0, scale_width: Some(320) })
            .expect("a frame should be decoded");
        assert_eq!(frame.width, 320);
        assert_eq!(frame.height, 180);
        assert_eq!(frame.rgba.len(), 320 * 180 * 4);
    }
}
