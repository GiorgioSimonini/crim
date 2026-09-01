//! Generation and execution of the ffmpeg command line.
//!
//! The command is the real "document" of this application: the UI only exists
//! to build it, and the text box below the preview always shows exactly what
//! will be executed. Nothing is hidden from the user.

use std::io::Read;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex};

use crate::state::{format_timestamp, is_copy, EditState, VideoInfo};

/// Quotes an argument only when needed, so the displayed command can be pasted
/// into a POSIX shell verbatim.
fn quote(arg: &str) -> String {
    let safe = !arg.is_empty()
        && arg
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "._-/:=,@+".contains(c));
    if safe {
        arg.to_owned()
    } else {
        format!("'{}'", arg.replace('\'', r"'\''"))
    }
}

/// Builds the export command for the current selection.
///
/// Layout of the generated line:
/// `ffmpeg -y -ss <in> -t <dur> -i <input> [-vf crop=w:h:x:y] -c:v ... <output>`
///
/// * `-ss`/`-t` are placed **before** `-i`: ffmpeg then seeks with the demuxer
///   (fast) and, since we re-encode anyway, the cut is frame accurate.
/// * `crop` is applied as a filter, which forces a re-encode — that is why a
///   video codec is always specified instead of `-c copy`.
pub fn build_command(info: &VideoInfo, edit: &EditState) -> String {
    let mut parts: Vec<String> = vec!["ffmpeg".into(), "-y".into()];

    // ---- trim -------------------------------------------------------------
    if edit.start > 0.0 {
        parts.push("-ss".into());
        parts.push(format_timestamp(edit.start));
    }
    let duration = edit.trim_duration();
    if duration > 0.0 && (info.duration - duration) > 0.001 {
        parts.push("-t".into());
        parts.push(format_timestamp(duration));
    }

    parts.push("-i".into());
    parts.push(quote(&info.path.to_string_lossy()));

    // ---- crop -------------------------------------------------------------
    if !edit.crop.is_full_frame() {
        let (w, h, x, y) = edit.crop.to_pixels(info);
        parts.push("-vf".into());
        parts.push(format!("crop={w}:{h}:{x}:{y}"));
    }

    // ---- encoding ---------------------------------------------------------
    let e = &edit.export;
    parts.push("-c:v".into());
    parts.push(e.video_codec.clone());
    // With a stream copy there is no encoder, so quality options must be
    // omitted (ffmpeg would reject them).
    if !is_copy(&e.video_codec) {
        parts.push("-crf".into());
        parts.push(e.crf.to_string());
        parts.push("-preset".into());
        parts.push(e.preset.clone());
        // yuv420p is the pixel format every player understands.
        parts.push("-pix_fmt".into());
        parts.push("yuv420p".into());
    }

    if !info.has_audio || e.mute {
        parts.push("-an".into());
    } else {
        parts.push("-c:a".into());
        parts.push(e.audio_codec.clone());
        if !is_copy(&e.audio_codec) {
            parts.push("-b:a".into());
            parts.push(e.audio_bitrate.clone());
        }
    }

    parts.push(quote(&edit.output.to_string_lossy()));
    parts.join(" ")
}

/// Builds the *preview* command used by the Play button (`ffplay`).
///
/// It mirrors the export settings that are visible on screen (trim + crop) but
/// skips encoding entirely, so playback starts instantly and keeps the audio.
pub fn build_play_command(info: &VideoInfo, edit: &EditState, from: f64) -> Vec<String> {
    let mut args: Vec<String> = vec![
        "-hide_banner".into(),
        "-loglevel".into(),
        "error".into(),
        "-autoexit".into(),
        "-window_title".into(),
        "crim preview".into(),
        "-ss".into(),
        format!("{from:.3}"),
        "-t".into(),
        format!("{:.3}", (edit.end - from).max(0.0)),
    ];
    // Keep the preview consistent with the export: no audio when muted.
    if edit.export.mute || !info.has_audio {
        args.push("-an".into());
    }
    if !edit.crop.is_full_frame() {
        let (w, h, x, y) = edit.crop.to_pixels(info);
        args.push("-vf".into());
        args.push(format!("crop={w}:{h}:{x}:{y}"));
    }
    args.push(info.path.to_string_lossy().to_string());
    args
}

// ---------------------------------------------------------------------------
// Execution
// ---------------------------------------------------------------------------

/// Message sent by the export thread to the GUI.
enum ExportEvent {
    /// Last status line printed by ffmpeg (`frame= ... time= ...`).
    Status(String),
    /// Process ended: `Ok(())` or the tail of stderr.
    Done(Result<(), String>),
}

/// A running export, polled once per frame by the GUI.
pub struct ExportJob {
    rx: Receiver<ExportEvent>,
    child: Arc<Mutex<Option<Child>>>,
    /// Latest ffmpeg status line, shown in the status bar.
    pub status: String,
    /// `None` while running.
    pub result: Option<Result<(), String>>,
}

impl ExportJob {
    /// Parses the (possibly hand-edited) command line and starts it.
    ///
    /// `command_line` must start with the program name, exactly as displayed.
    pub fn start(command_line: &str, on_update: impl Fn() + Send + 'static) -> Result<Self, String> {
        let argv = shell_words::split(command_line).map_err(|e| format!("cannot parse the command: {e}"))?;
        let (program, args) = argv.split_first().ok_or("the command is empty")?;

        let mut child = Command::new(program)
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("cannot start `{program}` ({e})"))?;

        let stderr = child.stderr.take();
        let child = Arc::new(Mutex::new(Some(child)));
        let (tx, rx) = mpsc::channel();

        let thread_child = Arc::clone(&child);
        std::thread::spawn(move || {
            let mut tail = String::new();

            if let Some(mut stderr) = stderr {
                // ffmpeg separates progress updates with '\r', so we cannot use
                // BufRead::lines(); we split on both terminators ourselves.
                let mut buffer = [0u8; 1024];
                let mut current = Vec::new();
                while let Ok(n) = stderr.read(&mut buffer) {
                    if n == 0 {
                        break;
                    }
                    for &byte in &buffer[..n] {
                        if byte == b'\r' || byte == b'\n' {
                            if !current.is_empty() {
                                let line = String::from_utf8_lossy(&current).trim().to_string();
                                current.clear();
                                if !line.is_empty() {
                                    tail.push_str(&line);
                                    tail.push('\n');
                                    let _ = tx.send(ExportEvent::Status(line));
                                    on_update();
                                }
                            }
                        } else {
                            current.push(byte);
                        }
                    }
                }
                if !current.is_empty() {
                    tail.push_str(String::from_utf8_lossy(&current).trim());
                }
            }

            let status = thread_child.lock().ok().and_then(|mut c| c.as_mut().map(|c| c.wait()));
            let result = match status {
                Some(Ok(s)) if s.success() => Ok(()),
                Some(Ok(s)) => Err(format!("ffmpeg exited with {s}\n{}", last_lines(&tail, 8))),
                Some(Err(e)) => Err(e.to_string()),
                None => Err("the process handle was lost".to_string()),
            };
            let _ = tx.send(ExportEvent::Done(result));
            on_update();
        });

        Ok(Self { rx, child, status: "starting…".into(), result: None })
    }

    /// Drains the channel. Call once per GUI frame.
    pub fn poll(&mut self) {
        while let Ok(event) = self.rx.try_recv() {
            match event {
                ExportEvent::Status(line) => self.status = line,
                ExportEvent::Done(result) => {
                    self.result = Some(result);
                    self.status = "finished".into();
                }
            }
        }
    }

    pub fn is_running(&self) -> bool {
        self.result.is_none()
    }

    /// Terminates the encoder (used by the Cancel button and on exit).
    pub fn cancel(&mut self) {
        if let Ok(mut guard) = self.child.lock() {
            if let Some(child) = guard.as_mut() {
                let _ = child.kill();
            }
        }
    }
}

fn last_lines(text: &str, count: usize) -> String {
    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    lines[lines.len().saturating_sub(count)..].join("\n")
}

/// True when `name` can be found and executed (used for the startup check).
pub fn binary_available(name: &str) -> bool {
    Command::new(name)
        .arg("-version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .stdin(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Convenience used by the Play button.
pub fn spawn_detached(program: &str, args: &[String], _cwd: Option<&Path>) -> Result<Child, String> {
    Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("cannot start `{program}` ({e})"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::Crop;
    use std::path::PathBuf;

    fn fixture() -> (VideoInfo, EditState) {
        let info = VideoInfo {
            path: PathBuf::from("/tmp/clip.mp4"),
            width: 1920,
            height: 1080,
            duration: 60.0,
            fps: 25.0,
            has_audio: true,
        };
        let edit = EditState::new(&info);
        (info, edit)
    }

    #[test]
    fn full_selection_has_no_trim_and_no_crop() {
        let (info, edit) = fixture();
        let line = build_command(&info, &edit);
        assert!(!line.contains("-ss"), "{line}");
        assert!(!line.contains("crop="), "{line}");
        assert!(line.contains("-c:v libx264"), "{line}");
    }

    #[test]
    fn trim_and_crop_are_rendered() {
        let (info, mut edit) = fixture();
        edit.start = 1.5;
        edit.end = 4.0;
        edit.crop = Crop { x: 0.25, y: 0.0, w: 0.5, h: 1.0 };
        let line = build_command(&info, &edit);
        assert!(line.contains("-ss 00:00:01.500"), "{line}");
        assert!(line.contains("-t 00:00:02.500"), "{line}");
        assert!(line.contains("crop=960:1080:480:0"), "{line}");
    }

    #[test]
    fn crop_pixels_are_even_and_inside_the_frame() {
        let (info, _) = fixture();
        let crop = Crop { x: 0.333, y: 0.777, w: 0.4, h: 0.3 };
        let (w, h, x, y) = crop.to_pixels(&info);
        assert_eq!(w % 2, 0);
        assert_eq!(h % 2, 0);
        assert!(x + w <= info.width && y + h <= info.height);
    }

    #[test]
    fn generated_command_survives_a_round_trip_through_the_parser() {
        let (info, mut edit) = fixture();
        edit.output = PathBuf::from("/tmp/my clip out.mp4"); // space on purpose
        let line = build_command(&info, &edit);
        let argv = shell_words::split(&line).expect("the displayed command must be parsable");
        assert_eq!(argv.first().unwrap(), "ffmpeg");
        assert_eq!(argv.last().unwrap(), "/tmp/my clip out.mp4");
    }

    #[test]
    fn mute_replaces_the_audio_options() {
        let (info, mut edit) = fixture();
        edit.export.mute = true;
        let line = build_command(&info, &edit);
        assert!(line.contains(" -an"), "{line}");
        assert!(!line.contains("-c:a"), "{line}");
    }

    #[test]
    fn stream_copy_omits_encoder_options() {
        let (info, mut edit) = fixture();
        edit.export.video_codec = "copy".into();
        edit.export.audio_codec = "copy".into();
        let line = build_command(&info, &edit);
        assert!(line.contains("-c:v copy"), "{line}");
        assert!(line.contains("-c:a copy"), "{line}");
        assert!(!line.contains("-crf"), "{line}");
        assert!(!line.contains("-preset"), "{line}");
        assert!(!line.contains("-b:a"), "{line}");
    }

    #[test]
    fn missing_binaries_are_detected() {
        assert!(!binary_available("definitely-not-a-real-binary-xyz"));
    }
}
