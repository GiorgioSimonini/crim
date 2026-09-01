//! Thin wrapper around the `ffprobe` binary.
//!
//! We ask ffprobe for JSON and pick only the few fields the editor needs.
//! Shelling out (instead of linking libav*) keeps the build dependency-free;
//! see `tech_summary.md` for the reasoning.

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::state::VideoInfo;

/// Runs `ffprobe` on `path` and extracts the metadata we care about.
pub fn probe(path: &Path) -> Result<VideoInfo, String> {
    let output = Command::new("ffprobe")
        .args([
            "-v", "error",
            "-print_format", "json",
            "-show_format",
            "-show_streams",
        ])
        .arg(path)
        .output()
        .map_err(|e| format!("cannot run `ffprobe` ({e}). Is ffmpeg installed and in PATH?"))?;

    if !output.status.success() {
        return Err(format!(
            "ffprobe failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }

    let json: serde_json::Value =
        serde_json::from_slice(&output.stdout).map_err(|e| format!("invalid ffprobe JSON: {e}"))?;

    let streams = json["streams"].as_array().cloned().unwrap_or_default();

    let video = streams
        .iter()
        .find(|s| s["codec_type"] == "video")
        .ok_or_else(|| "the file contains no video stream".to_string())?;

    let width = video["width"].as_u64().unwrap_or(0) as u32;
    let height = video["height"].as_u64().unwrap_or(0) as u32;
    if width == 0 || height == 0 {
        return Err("could not read the video resolution".into());
    }

    // Duration may live on the stream or only on the container.
    let duration = video["duration"]
        .as_str()
        .and_then(|s| s.parse::<f64>().ok())
        .or_else(|| json["format"]["duration"].as_str().and_then(|s| s.parse::<f64>().ok()))
        .unwrap_or(0.0);

    // `avg_frame_rate` is a rational string such as "30000/1001".
    let fps = video["avg_frame_rate"]
        .as_str()
        .and_then(parse_rational)
        .filter(|v| *v > 0.0)
        .or_else(|| video["r_frame_rate"].as_str().and_then(parse_rational))
        .unwrap_or(25.0);

    let has_audio = streams.iter().any(|s| s["codec_type"] == "audio");

    Ok(VideoInfo {
        path: PathBuf::from(path),
        width,
        height,
        duration: if duration > 0.0 { duration } else { 1.0 },
        fps,
        has_audio,
    })
}

/// Parses ffprobe's `num/den` rational notation.
fn parse_rational(text: &str) -> Option<f64> {
    let (num, den) = text.split_once('/')?;
    let num: f64 = num.trim().parse().ok()?;
    let den: f64 = den.trim().parse().ok()?;
    if den == 0.0 {
        None
    } else {
        Some(num / den)
    }
}
