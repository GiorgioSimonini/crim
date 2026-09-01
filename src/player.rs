//! Audio + video playback of the current selection, delegated to `ffplay`.
//!
//! Decoding video *and* audio in-process (with synchronisation, resampling and
//! an audio output device) would be by far the largest part of this program.
//! `ffplay` ships with ffmpeg, already does all of that, and accepts the very
//! same `-ss/-t/-vf crop` options we show in the command box — so playback is
//! guaranteed to match what will be exported.
//!
//! While the child runs we advance the playhead with the wall clock, which is
//! accurate enough for a preview and costs nothing.

use std::process::Child;
use std::time::Instant;

use crate::command::{build_play_command, spawn_detached};
use crate::state::{EditState, VideoInfo};

#[derive(Default)]
pub struct Player {
    child: Option<Child>,
    /// When playback started, and from which position, to derive the playhead.
    started_at: Option<Instant>,
    started_from: f64,
}

impl Player {
    pub fn is_playing(&self) -> bool {
        self.child.is_some()
    }

    /// Starts `ffplay` at `from` seconds, honouring the current crop and the
    /// trim-out point.
    pub fn play(&mut self, info: &VideoInfo, edit: &EditState, from: f64) -> Result<(), String> {
        self.stop();
        let args = build_play_command(info, edit, from);
        let child = spawn_detached("ffplay", &args, None)
            .map_err(|e| format!("{e}. `ffplay` is part of the ffmpeg suite."))?;
        self.child = Some(child);
        self.started_at = Some(Instant::now());
        self.started_from = from;
        Ok(())
    }

    /// Kills the player window if it is still open.
    pub fn stop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        self.started_at = None;
    }

    /// Call once per GUI frame.
    ///
    /// Returns the estimated playback position while playing, so the caller can
    /// move the playhead cursor. Automatically clears the state when `ffplay`
    /// exits (end of the selection, or the user closed its window).
    pub fn update(&mut self, edit: &EditState) -> Option<f64> {
        let finished = match self.child.as_mut() {
            Some(child) => matches!(child.try_wait(), Ok(Some(_)) | Err(_)),
            None => return None,
        };
        if finished {
            self.stop();
            return None;
        }

        let elapsed = self.started_at?.elapsed().as_secs_f64();
        let position = self.started_from + elapsed;
        if position >= edit.end {
            self.stop();
            return Some(edit.end);
        }
        Some(position)
    }
}

impl Drop for Player {
    fn drop(&mut self) {
        self.stop();
    }
}
