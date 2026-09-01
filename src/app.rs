//! Application shell: owns the state, wires the widgets together and keeps the
//! ffmpeg command in sync with the user's edits.
//!
//! Data flow, once per frame:
//! 1. collect results from the background workers (frame extractor, exporter,
//!    player);
//! 2. draw the widgets, which mutate [`EditState`] and report what changed;
//! 3. if something changed, rebuild the command text and/or ask for a new
//!    preview frame.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use egui::{Color32, RichText, TextureHandle, TextureOptions};

use crate::command::{self, ExportJob};
use crate::ffprobe;
use crate::frames::{FrameExtractor, FrameRequest};
use crate::player::Player;
use crate::state::{
    format_timestamp, is_copy, EditState, VideoInfo, AUDIO_BITRATES, AUDIO_CODECS, PRESETS,
    VIDEO_CODECS,
};
use crate::ui::{
    codec_combo,
    preview::{preview, CropDrag},
    timeline::{timeline, TimelineHandle},
};

/// Preview frames are downscaled to this width: enough to place a crop
/// precisely, cheap to decode and to upload to the GPU.
const PREVIEW_WIDTH: u32 = 960;
/// Minimum delay between two extraction requests while dragging.
const SEEK_THROTTLE: Duration = Duration::from_millis(70);

pub struct EditorApp {
    // --- document ---------------------------------------------------------
    video: Option<VideoInfo>,
    edit: Option<EditState>,

    // --- preview ----------------------------------------------------------
    extractor: FrameExtractor,
    texture: Option<TextureHandle>,
    frame_aspect: f32,
    /// Timestamp of the frame currently on screen (may lag behind the
    /// playhead while a new one is being decoded).
    displayed_time: f64,
    /// Position waiting to be decoded (set while the user scrubs).
    pending_seek: Option<f64>,
    last_seek_request: Instant,

    // --- widget state -----------------------------------------------------
    timeline_drag: Option<TimelineHandle>,
    crop_drag: CropDrag,

    // --- command ----------------------------------------------------------
    command_text: String,
    /// True once the user typed in the command box: we then stop overwriting it.
    command_edited: bool,

    // --- processes --------------------------------------------------------
    export: Option<ExportJob>,
    player: Player,

    // --- messages ---------------------------------------------------------
    message: Option<(String, bool)>, // (text, is_error)
    /// Clone of the egui context, so worker threads can request a repaint.
    egui_ctx: egui::Context,
    ffmpeg_missing: bool,
    ffplay_missing: bool,
}

impl EditorApp {
    pub fn new(cc: &eframe::CreationContext<'_>, initial_file: Option<PathBuf>) -> Self {
        cc.egui_ctx.set_visuals(egui::Visuals::dark());

        // The worker wakes the UI thread up when a frame is ready, so we can
        // stay on egui's default "repaint only on input" behaviour.
        let ctx = cc.egui_ctx.clone();
        let extractor = FrameExtractor::new(move || ctx.request_repaint());

        let mut app = Self {
            video: None,
            edit: None,
            extractor,
            texture: None,
            frame_aspect: 16.0 / 9.0,
            displayed_time: 0.0,
            pending_seek: None,
            last_seek_request: Instant::now() - SEEK_THROTTLE,
            timeline_drag: None,
            crop_drag: CropDrag::default(),
            command_text: String::new(),
            command_edited: false,
            export: None,
            player: Player::default(),
            message: None,
            egui_ctx: cc.egui_ctx.clone(),
            ffmpeg_missing: !command::binary_available("ffmpeg"),
            ffplay_missing: !command::binary_available("ffplay"),
        };

        if let Some(path) = initial_file {
            app.open(&path);
        }
        app
    }

    // -- document ----------------------------------------------------------

    /// Probes a file and resets the editing state.
    fn open(&mut self, path: &Path) {
        match ffprobe::probe(path) {
            Ok(info) => {
                self.frame_aspect = info.width as f32 / info.height as f32;
                let edit = EditState::new(&info);
                self.pending_seek = Some(edit.playhead);
                self.texture = None;
                self.command_edited = false;
                self.message = Some((
                    format!("{}×{} · {:.2} fps · {}", info.width, info.height, info.fps,
                            format_timestamp(info.duration)),
                    false,
                ));
                self.video = Some(info);
                self.edit = Some(edit);
                self.rebuild_command();
            }
            Err(error) => self.message = Some((error, true)),
        }
    }

    fn pick_file(&mut self) {
        if let Some(path) = rfd::FileDialog::new()
            .add_filter("video", &["mp4", "mkv", "mov", "avi", "webm", "m4v", "mpg", "mpeg", "wmv", "flv", "ts"])
            .add_filter("all files", &["*"])
            .pick_file()
        {
            self.open(&path);
        }
    }

    /// Regenerates the command text unless the user edited it by hand.
    fn rebuild_command(&mut self) {
        if self.command_edited {
            return;
        }
        if let (Some(info), Some(edit)) = (&self.video, &self.edit) {
            self.command_text = command::build_command(info, edit);
        }
    }

    /// Schedules a preview refresh at the current playhead.
    fn request_frame(&mut self) {
        if let Some(edit) = &self.edit {
            self.pending_seek = Some(edit.playhead);
        }
    }

    /// Sends the pending request, respecting the throttle.
    fn flush_seek(&mut self) {
        let Some(time) = self.pending_seek else { return };
        if self.last_seek_request.elapsed() < SEEK_THROTTLE {
            return;
        }
        let Some(info) = &self.video else { return };
        self.extractor.request(FrameRequest {
            path: info.path.clone(),
            // Asking for a frame exactly at EOF returns nothing; step back a
            // little to stay inside the stream.
            time: time.min(info.duration - info.frame_step()).max(0.0),
            scale_width: (info.width > PREVIEW_WIDTH).then_some(PREVIEW_WIDTH),
        });
        self.pending_seek = None;
        self.last_seek_request = Instant::now();
    }

    /// Uploads a decoded frame as a texture.
    fn accept_frame(&mut self, ctx: &egui::Context, frame: crate::frames::DecodedFrame) {
        let image = egui::ColorImage::from_rgba_unmultiplied([frame.width, frame.height], &frame.rgba);
        self.frame_aspect = frame.width as f32 / frame.height as f32;
        self.displayed_time = frame.time;
        match &mut self.texture {
            // Reusing the handle avoids allocating a new GPU texture per frame.
            Some(texture) => texture.set(image, TextureOptions::LINEAR),
            None => self.texture = Some(ctx.load_texture("preview", image, TextureOptions::LINEAR)),
        }
    }

    // -- panels ------------------------------------------------------------

    fn toolbar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            if ui.button("Open video…").clicked() {
                self.pick_file();
            }

            let Some(info) = self.video.clone() else {
                ui.label(RichText::new("no file loaded").weak());
                return;
            };

            ui.separator();

            // --- transport ---
            let playing = self.player.is_playing();
            let label = if playing { "⏹ Stop" } else { "▶ Play" };
            if ui.add_enabled(!self.ffplay_missing, egui::Button::new(label)).clicked() {
                if playing {
                    self.player.stop();
                } else if let Some(edit) = &self.edit {
                    let from = if edit.playhead >= edit.end - 0.05 { edit.start } else { edit.playhead };
                    if let Err(error) = self.player.play(&info, edit, from) {
                        self.message = Some((error, true));
                    }
                }
            }

            // --- mute ---
            // Toggling it rewrites the command (-an) and, if a preview is
            // running, restarts ffplay so what you hear matches the command.
            let muted = self.edit.as_ref().is_some_and(|edit| edit.export.mute);
            let mute_label = if muted { "Muted" } else { "Audio" };
            let mute_button = ui.add_enabled(
                info.has_audio,
                egui::SelectableLabel::new(muted, mute_label),
            );
            if mute_button
                .on_hover_text(if info.has_audio {
                    "drop the audio track from the export and the preview (-an)"
                } else {
                    "this file has no audio track"
                })
                .clicked()
            {
                if let Some(edit) = &mut self.edit {
                    edit.export.mute = !muted;
                }
                self.rebuild_command();
                if self.player.is_playing() {
                    if let Some(edit) = &self.edit {
                        let from = edit.playhead;
                        let edit = edit.clone();
                        if let Err(error) = self.player.play(&info, &edit, from) {
                            self.message = Some((error, true));
                        }
                    }
                }
            }

            let step = info.frame_step();
            if ui.button("⏴").on_hover_text("previous frame").clicked() {
                self.nudge(-step);
            }
            if ui.button("⏵").on_hover_text("next frame").clicked() {
                self.nudge(step);
            }

            ui.separator();

            // --- trim shortcuts ---
            if ui.button("Set in").on_hover_text("trim in at the playhead").clicked() {
                if let Some(edit) = &mut self.edit {
                    edit.start = edit.playhead.min(edit.end - step);
                }
                self.rebuild_command();
            }
            if ui.button("Set out").on_hover_text("trim out at the playhead").clicked() {
                if let Some(edit) = &mut self.edit {
                    edit.end = edit.playhead.max(edit.start + step);
                }
                self.rebuild_command();
            }

            ui.separator();

            if ui.button("Reset crop").clicked() {
                if let Some(edit) = &mut self.edit {
                    edit.crop = Default::default();
                }
                self.rebuild_command();
            }

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if let (Some(info), Some(edit)) = (&self.video, &self.edit) {
                    let (w, h, x, y) = edit.crop.to_pixels(info);
                    // Compact status text: it is reference information, so it
                    // is deliberately smaller than the interactive widgets.
                    ui.label(
                        RichText::new(format!(
                            "{}  ·  frame {}  ·  crop {w}×{h}+{x}+{y}  ·  selection {}",
                            info.path.file_name().unwrap_or_default().to_string_lossy(),
                            format_timestamp(self.displayed_time),
                            format_timestamp(edit.trim_duration()),
                        ))
                        .monospace()
                        .size(9.5)
                        .weak(),
                    );
                }
            });
        });
    }

    /// Moves the playhead by `delta` seconds, keeping it inside the selection.
    fn nudge(&mut self, delta: f64) {
        if let Some(edit) = &mut self.edit {
            edit.playhead = (edit.playhead + delta).clamp(edit.start, edit.end);
        }
        // Manual stepping should feel immediate: bypass the throttle.
        self.last_seek_request = Instant::now() - SEEK_THROTTLE;
        self.request_frame();
    }

    fn bottom_panel(&mut self, ui: &mut egui::Ui) {
        // --- timeline ---
        if let (Some(info), Some(edit)) = (self.video.clone(), self.edit.as_mut()) {
            let change = timeline(ui, &info, edit, &mut self.timeline_drag);
            if change.seek {
                self.request_frame();
            }
            if change.trim {
                self.rebuild_command();
            }
        }

        ui.add_space(4.0);

        // --- encoding options ---
        if let Some(edit) = self.edit.as_mut() {
            let has_audio = self.video.as_ref().is_some_and(|info| info.has_audio);
            ui.horizontal(|ui| {
                ui.label("video");
                codec_combo(ui, "video_codec", &mut edit.export.video_codec, VIDEO_CODECS, 110.0);

                // A stream copy has no encoder, so quality controls would be
                // meaningless: hide them instead of generating invalid options.
                if is_copy(&edit.export.video_codec) {
                    if !edit.crop.is_full_frame() {
                        ui.label(
                            RichText::new("⚠ cropping needs a re-encode")
                                .color(Color32::from_rgb(255, 196, 92)),
                        )
                        .on_hover_text(
                            "`-c:v copy` cannot apply a filter: pick a video encoder or reset the crop",
                        );
                    }
                } else {
                    ui.label("crf");
                    ui.add(egui::DragValue::new(&mut edit.export.crf).range(0..=51))
                        .on_hover_text("lower = better quality, bigger file (18–24 is typical)");
                    ui.label("preset");
                    egui::ComboBox::from_id_salt("preset")
                        .selected_text(edit.export.preset.clone())
                        .width(90.0)
                        .show_ui(ui, |ui| {
                            for preset in PRESETS {
                                ui.selectable_value(
                                    &mut edit.export.preset,
                                    (*preset).to_owned(),
                                    *preset,
                                );
                            }
                        });
                }

                ui.separator();

                ui.label("audio");
                if !has_audio {
                    ui.label(RichText::new("none in the source").weak());
                } else if edit.export.mute {
                    ui.label(RichText::new("muted (-an)").weak());
                } else {
                    codec_combo(ui, "audio_codec", &mut edit.export.audio_codec, AUDIO_CODECS, 110.0);
                    if !is_copy(&edit.export.audio_codec) {
                        codec_combo(
                            ui,
                            "audio_bitrate",
                            &mut edit.export.audio_bitrate,
                            AUDIO_BITRATES,
                            70.0,
                        );
                    }
                }

                ui.separator();
                if ui.button("Output…").clicked() {
                    if let Some(path) = rfd::FileDialog::new()
                        .set_file_name(edit.output.file_name().unwrap_or_default().to_string_lossy())
                        .save_file()
                    {
                        edit.output = path;
                    }
                }
                ui.label(RichText::new(edit.output.to_string_lossy()).monospace().weak());
            });
            // Any of the widgets above may have changed a setting; regenerating
            // the command every frame is cheap (a few string pushes).
            self.rebuild_command();
        }

        ui.add_space(4.0);

        // --- the command itself ---
        ui.horizontal(|ui| {
            ui.label(RichText::new("ffmpeg command").strong());
            if self.command_edited {
                ui.label(RichText::new("(edited by hand)").color(Color32::from_rgb(255, 196, 92)));
                if ui.button("Regenerate").clicked() {
                    self.command_edited = false;
                    self.rebuild_command();
                }
            }
            if ui.button("Copy").clicked() {
                let text = self.command_text.clone();
                ui.output_mut(|o| o.copied_text = text);
            }
        });

        let response = ui.add(
            egui::TextEdit::multiline(&mut self.command_text)
                .font(egui::TextStyle::Monospace)
                .desired_rows(3)
                .desired_width(f32::INFINITY),
        );
        if response.changed() {
            self.command_edited = true;
        }

        ui.add_space(4.0);

        // --- export ---
        ui.horizontal(|ui| {
            let running = self.export.as_ref().is_some_and(|job| job.is_running());
            if running {
                if ui.button("Cancel").clicked() {
                    if let Some(job) = &mut self.export {
                        job.cancel();
                    }
                }
                ui.spinner();
            } else if ui
                .add_enabled(self.video.is_some() && !self.ffmpeg_missing, egui::Button::new("Export"))
                .clicked()
            {
                self.start_export();
            }

            if let Some(job) = &self.export {
                match &job.result {
                    None => {
                        ui.label(RichText::new(&job.status).monospace().weak());
                    }
                    Some(Ok(())) => {
                        ui.label(RichText::new("✔ export finished").color(Color32::LIGHT_GREEN));
                    }
                    Some(Err(error)) => {
                        ui.label(RichText::new(format!("✖ {error}")).color(Color32::LIGHT_RED));
                    }
                }
            } else if let Some((text, is_error)) = &self.message {
                let color = if *is_error { Color32::LIGHT_RED } else { ui.visuals().weak_text_color() };
                ui.label(RichText::new(text).color(color));
            }
        });
    }

    fn start_export(&mut self) {
        let ctx_command = self.command_text.clone();
        let ctx = self.egui_ctx.clone();
        match ExportJob::start(&ctx_command, move || ctx.request_repaint()) {
            Ok(job) => {
                self.export = Some(job);
                self.message = None;
            }
            Err(error) => self.message = Some((error, true)),
        }
    }
}

impl eframe::App for EditorApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // 1. background results ------------------------------------------------
        if let Some(result) = self.extractor.poll() {
            match result {
                Ok(frame) => self.accept_frame(ctx, frame),
                Err(error) => self.message = Some((error, true)),
            }
        }
        if let Some(job) = &mut self.export {
            job.poll();
        }
        if let Some(edit) = &mut self.edit {
            if let Some(position) = self.player.update(edit) {
                edit.playhead = position;
                // ffplay draws its own window: we only follow with the cursor.
                ctx.request_repaint_after(Duration::from_millis(33));
            }
        }

        // 2. widgets ------------------------------------------------------------
        egui::TopBottomPanel::top("toolbar").show(ctx, |ui| {
            ui.add_space(4.0);
            self.toolbar(ui);
            ui.add_space(4.0);
            if self.ffmpeg_missing {
                ui.label(
                    RichText::new("ffmpeg/ffprobe not found in PATH — see INSTALL.md")
                        .color(Color32::LIGHT_RED),
                );
            }
        });

        egui::TopBottomPanel::bottom("controls").show(ctx, |ui| {
            ui.add_space(6.0);
            self.bottom_panel(ui);
            ui.add_space(6.0);
        });

        egui::CentralPanel::default()
            .frame(egui::Frame::none())
            .show(ctx, |ui| {
                let mut crop = self.edit.as_ref().map(|e| e.crop).unwrap_or_default();
                let changed = preview(ui, self.texture.as_ref(), self.frame_aspect, &mut crop, &mut self.crop_drag);
                if changed {
                    if let Some(edit) = &mut self.edit {
                        edit.crop = crop;
                    }
                    self.rebuild_command();
                }
            });

        // 3. drag & drop --------------------------------------------------------
        let dropped = ctx.input(|i| i.raw.dropped_files.clone());
        if let Some(path) = dropped.into_iter().find_map(|f| f.path) {
            self.open(&path);
        }

        // 4. deferred work ------------------------------------------------------
        self.flush_seek();
        if self.pending_seek.is_some() || self.extractor.is_busy() {
            // Keep the loop alive until the pending frame has been delivered.
            ctx.request_repaint_after(SEEK_THROTTLE);
        }
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.player.stop();
        if let Some(job) = &mut self.export {
            job.cancel();
        }
    }
}
