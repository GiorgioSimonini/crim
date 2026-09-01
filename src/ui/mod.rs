//! Custom widgets.
//!
//! Both widgets are written directly against `egui`'s painter + response API
//! instead of composing existing widgets: they need pixel-precise handles and
//! their own hit-testing, which is exactly the case where immediate-mode
//! drawing is simpler than assembling stock components.
//!
//! Each widget follows the same contract:
//! `fn widget(ui, …, state) -> bool` where the returned bool means
//! "the model changed, the preview should be refreshed".

pub mod preview;
pub mod timeline;

/// Dropdown for a codec name, with an escape hatch for anything not listed.
///
/// Selecting `custom…` empties the value and reveals a text field, so the UI
/// stays short for the 99 % case without hiding the remaining ffmpeg encoders.
/// Returns `true` when the value changed.
pub fn codec_combo(
    ui: &mut egui::Ui,
    id: &str,
    value: &mut String,
    options: &[&str],
    width: f32,
) -> bool {
    let known = options.iter().any(|option| *option == value.as_str());
    let shown = if known { value.clone() } else { "custom…".to_owned() };
    let mut changed = false;

    egui::ComboBox::from_id_salt(id)
        .selected_text(shown)
        .width(width)
        .show_ui(ui, |ui| {
            for option in options {
                if ui.selectable_label(value.as_str() == *option, *option).clicked() {
                    *value = (*option).to_owned();
                    changed = true;
                }
            }
            ui.separator();
            if ui.selectable_label(!known, "custom…").clicked() && known {
                value.clear();
                changed = true;
            }
        });

    if !known {
        changed |= ui
            .add(
                egui::TextEdit::singleline(value)
                    .desired_width(110.0)
                    .hint_text("encoder name"),
            )
            .changed();
    }
    changed
}

/// Shared colour palette, kept in one place so the look can be tuned quickly.
pub mod theme {
    use egui::Color32;

    pub const TRACK: Color32 = Color32::from_rgb(48, 50, 56);
    /// Teal, taken from the application icon.
    pub const SELECTION: Color32 = Color32::from_rgb(64, 190, 186);
    pub const HANDLE: Color32 = Color32::from_rgb(232, 236, 242);
    pub const HANDLE_HOVER: Color32 = Color32::from_rgb(255, 196, 92);
    pub const PLAYHEAD: Color32 = Color32::from_rgb(240, 92, 92);
    pub const CROP_BORDER: Color32 = Color32::from_rgb(255, 196, 92);
    pub const CROP_GUIDE: Color32 = Color32::from_rgba_premultiplied(180, 180, 180, 90);
    pub const OUTSIDE_MASK: Color32 = Color32::from_rgba_premultiplied(0, 0, 0, 130);
}
