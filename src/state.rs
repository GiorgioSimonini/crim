//! Pure data model of the editor.
//!
//! This module deliberately contains **no** GUI and **no** process handling:
//! it only describes *what* the user selected. Every other module either
//! renders this state (`ui::*`) or turns it into an ffmpeg invocation
//! (`command`). Keeping it isolated makes the state easy to extend
//! (e.g. adding rotation, fade, speed change) without touching the widgets.

use std::path::PathBuf;

/// Metadata of the currently open file, as reported by `ffprobe`.
#[derive(Debug, Clone)]
pub struct VideoInfo {
    pub path: PathBuf,
    /// Coded frame size in pixels (before any cropping).
    pub width: u32,
    pub height: u32,
    /// Container duration in seconds.
    pub duration: f64,
    /// Average frame rate, used to step frame-by-frame. Falls back to 25.0.
    pub fps: f64,
    /// True when ffprobe found at least one audio stream (drives `-an` / `-c:a`).
    pub has_audio: bool,
}

impl VideoInfo {
    /// Duration of a single frame, in seconds.
    pub fn frame_step(&self) -> f64 {
        if self.fps > 0.0 {
            1.0 / self.fps
        } else {
            0.04
        }
    }
}

/// Crop rectangle stored in **normalised** coordinates (0.0..=1.0) relative to
/// the full frame.
///
/// Normalised units keep the model independent from both the preview scale and
/// the source resolution, so the same rectangle stays valid if we later load a
/// different file or change the preview quality. Pixels are only computed at
/// export time, in [`Crop::to_pixels`].
#[derive(Debug, Clone, Copy)]
pub struct Crop {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Default for Crop {
    /// The whole frame.
    fn default() -> Self {
        Self { x: 0.0, y: 0.0, w: 1.0, h: 1.0 }
    }
}

impl Crop {
    pub fn is_full_frame(&self) -> bool {
        self.x <= 0.0005 && self.y <= 0.0005 && self.w >= 0.9995 && self.h >= 0.9995
    }

    /// Keeps the rectangle inside the frame and never smaller than ~2 %.
    pub fn clamp(&mut self) {
        const MIN: f32 = 0.02;
        self.w = self.w.clamp(MIN, 1.0);
        self.h = self.h.clamp(MIN, 1.0);
        self.x = self.x.clamp(0.0, 1.0 - self.w);
        self.y = self.y.clamp(0.0, 1.0 - self.h);
    }

    /// Converts to integer pixels for the ffmpeg `crop` filter.
    ///
    /// Width/height/offsets are rounded to **even** numbers because the default
    /// 4:2:0 chroma subsampling (`yuv420p`) cannot represent odd dimensions;
    /// libx264 would otherwise refuse to encode.
    pub fn to_pixels(&self, info: &VideoInfo) -> (u32, u32, u32, u32) {
        let even = |v: f32| ((v as u32) / 2) * 2;

        let mut w = even((self.w * info.width as f32).round()).max(2);
        let mut h = even((self.h * info.height as f32).round()).max(2);
        let x = even((self.x * info.width as f32).round());
        let y = even((self.y * info.height as f32).round());

        // Guard against rounding pushing the rectangle past the frame border.
        w = w.min(info.width.saturating_sub(x)).max(2);
        h = h.min(info.height.saturating_sub(y)).max(2);
        (w, h, x, y)
    }
}

/// Everything the user can adjust for the current file.
#[derive(Debug, Clone)]
pub struct EditState {
    /// Trim-in point, seconds.
    pub start: f64,
    /// Trim-out point, seconds.
    pub end: f64,
    /// Position of the scrubbing cursor, always kept inside `[start, end]`.
    pub playhead: f64,
    pub crop: Crop,
    pub export: ExportSettings,
    pub output: PathBuf,
}

impl EditState {
    /// Fresh state for a newly opened file: full length, full frame.
    pub fn new(info: &VideoInfo) -> Self {
        Self {
            start: 0.0,
            end: info.duration,
            playhead: 0.0,
            crop: Crop::default(),
            export: ExportSettings::default(),
            output: default_output_path(&info.path),
        }
    }

    pub fn trim_duration(&self) -> f64 {
        (self.end - self.start).max(0.0)
    }
}

/// Codecs offered by the dropdowns. The first entries are the common cases;
/// `"copy"` means "remux without re-encoding" and any other value can still be
/// typed in the `custom…` field.
pub const VIDEO_CODECS: &[&str] = &[
    "libx264",     // H.264, the safe default
    "libx265",     // H.265/HEVC, ~30 % smaller, slower
    "libvpx-vp9",  // VP9 (WebM)
    "libsvtav1",   // AV1, fast encoder
    "mpeg4",       // legacy, very fast
    "h264_nvenc",  // NVIDIA hardware
    "h264_vaapi",  // Intel/AMD hardware on Linux
    "copy",        // no re-encode (incompatible with cropping)
];

pub const AUDIO_CODECS: &[&str] = &[
    "aac",         // the safe default for MP4
    "libmp3lame",  // MP3
    "libopus",     // best quality per bit (WebM/MKV)
    "ac3",
    "flac",        // lossless
    "pcm_s16le",   // uncompressed
    "copy",        // keep the original stream
];

pub const AUDIO_BITRATES: &[&str] = &["96k", "128k", "160k", "192k", "256k", "320k"];

pub const PRESETS: &[&str] =
    &["ultrafast", "superfast", "veryfast", "faster", "fast", "medium", "slow", "veryslow"];

/// True when the codec name means "stream copy", i.e. no encoding parameters
/// (crf, preset, bitrate) may be passed for that stream.
pub fn is_copy(codec: &str) -> bool {
    codec.eq_ignore_ascii_case("copy")
}

/// Encoder knobs exposed in the UI. They only affect the *generated* command;
/// the user can always override the final command text by hand.
#[derive(Debug, Clone)]
pub struct ExportSettings {
    pub video_codec: String,
    /// Constant Rate Factor: lower = better quality / bigger file.
    pub crf: u32,
    pub preset: String,
    pub audio_codec: String,
    pub audio_bitrate: String,
    /// Drop the audio stream entirely (`-an`). Also mutes the ffplay preview.
    pub mute: bool,
}

impl Default for ExportSettings {
    fn default() -> Self {
        Self {
            video_codec: "libx264".to_owned(),
            crf: 20,
            preset: "medium".to_owned(),
            audio_codec: "aac".to_owned(),
            audio_bitrate: "192k".to_owned(),
            mute: false,
        }
    }
}

/// `/path/movie.mkv` -> `/path/movie_edit.mp4`
pub fn default_output_path(input: &std::path::Path) -> PathBuf {
    let stem = input.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "output".into());
    let dir = input.parent().map(|p| p.to_path_buf()).unwrap_or_default();
    dir.join(format!("{stem}_edit.mp4"))
}

/// Formats seconds as `HH:MM:SS.mmm`, the form ffmpeg accepts for `-ss`/`-t`.
pub fn format_timestamp(seconds: f64) -> String {
    let s = seconds.max(0.0);
    let hours = (s / 3600.0).floor() as u64;
    let minutes = ((s % 3600.0) / 60.0).floor() as u64;
    let secs = s % 60.0;
    format!("{hours:02}:{minutes:02}:{secs:06.3}")
}
