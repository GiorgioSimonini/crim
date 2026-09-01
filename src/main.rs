//! crim — **cr**op & tr**im**: a minimal video editor driven by ffmpeg.
//!
//! Module map:
//! * [`state`]   – data model (trim points, crop rectangle, encoder settings);
//! * [`ffprobe`] – reads the metadata of the opened file;
//! * [`frames`]  – background worker that extracts preview frames;
//! * [`command`] – builds and runs the ffmpeg command line;
//! * [`player`]  – audio+video playback through `ffplay`;
//! * [`ui`]      – the two custom widgets (timeline and crop overlay);
//! * [`app`]     – glue: state + widgets + workers.
//!
//! Usage: `crim [file]`

mod app;
mod command;
mod ffprobe;
mod frames;
mod player;
mod state;
mod ui;

use std::path::PathBuf;

/// Window / taskbar icon, embedded in the binary so the executable stays
/// self-contained (no runtime asset lookup, no install step).
fn load_icon() -> egui::IconData {
    let bytes = include_bytes!("../assets/icon.png");
    match image::load_from_memory(bytes) {
        Ok(image) => {
            let image = image.to_rgba8();
            let (width, height) = image.dimensions();
            egui::IconData { rgba: image.into_raw(), width, height }
        }
        // A broken icon must never prevent the application from starting.
        Err(_) => egui::IconData::default(),
    }
}

fn main() -> eframe::Result<()> {
    // Optional positional argument: open this file at startup.
    let initial_file: Option<PathBuf> = std::env::args().nth(1).map(PathBuf::from);

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1100.0, 780.0])
            .with_min_inner_size([760.0, 560.0])
            .with_title("crim")
            // Wayland matches the .desktop file through the app id (see
            // assets/crim.desktop), which is what shows the icon in the dock.
            .with_app_id("crim")
            .with_icon(load_icon()),
        ..Default::default()
    };

    eframe::run_native(
        "crim",
        options,
        Box::new(|cc| Ok(Box::new(app::EditorApp::new(cc, initial_file)))),
    )
}
