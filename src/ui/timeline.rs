//! Timeline strip: two trim cursors plus a draggable playhead between them.

use egui::{Align2, Color32, FontId, Pos2, Rect, Rounding, Sense, Stroke, Ui, Vec2};

use super::theme;
use crate::state::{format_timestamp, EditState, VideoInfo};

/// Which element the pointer grabbed when the drag started.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimelineHandle {
    Start,
    End,
    Playhead,
}

/// What changed during this frame.
#[derive(Default, Debug, Clone, Copy)]
pub struct TimelineChange {
    /// The playhead moved: a new preview frame is needed.
    pub seek: bool,
    /// A trim cursor moved: the ffmpeg command must be rebuilt.
    pub trim: bool,
}

/// Total height of the strip: 18 px for the in/out labels, 30 px of track,
/// then room for the playhead knob and for the current-time label underneath.
const HEIGHT: f32 = 86.0;
const TRACK_TOP: f32 = 20.0;
const TRACK_HEIGHT: f32 = 30.0;
const SIDE_MARGIN: f32 = 10.0;
const HANDLE_WIDTH: f32 = 9.0;
/// Pointer distance (px) within which a handle is considered grabbed.
const GRAB_RADIUS: f32 = 12.0;
const LABEL_SIZE: f32 = 11.0;

/// Draws the timeline and applies the user's drags to `edit`.
///
/// `drag` holds the handle currently being dragged; it must live in the
/// application state (a widget in immediate mode is stateless by itself).
pub fn timeline(
    ui: &mut Ui,
    info: &VideoInfo,
    edit: &mut EditState,
    drag: &mut Option<TimelineHandle>,
) -> TimelineChange {
    let mut change = TimelineChange::default();

    let (outer, response) =
        ui.allocate_exact_size(Vec2::new(ui.available_width(), HEIGHT), Sense::click_and_drag());
    let painter = ui.painter_at(outer);

    // Track geometry: leave room on both sides so the handles stay reachable
    // even when start == 0 or end == duration.
    let track = Rect::from_min_max(
        Pos2::new(outer.left() + SIDE_MARGIN, outer.top() + TRACK_TOP),
        Pos2::new(outer.right() - SIDE_MARGIN, outer.top() + TRACK_TOP + TRACK_HEIGHT),
    );

    let duration = info.duration.max(0.001);
    // Conversions between seconds and screen x.
    let to_x = |t: f64| track.left() + (t / duration) as f32 * track.width();
    let to_time = |x: f32| {
        (((x - track.left()) / track.width().max(1.0)) as f64 * duration).clamp(0.0, duration)
    };

    // ---- interaction ------------------------------------------------------
    if response.drag_started() {
        if let Some(pos) = response.interact_pointer_pos() {
            // The playhead wins whenever it is within reach, even if a trim
            // cursor sits at the very same position: it is the cursor the user
            // moves constantly, while trim points are set once (and can always
            // be set with the "Set in"/"Set out" buttons).
            *drag = if (to_x(edit.playhead) - pos.x).abs() <= GRAB_RADIUS {
                Some(TimelineHandle::Playhead)
            } else {
                [
                    (TimelineHandle::Start, to_x(edit.start)),
                    (TimelineHandle::End, to_x(edit.end)),
                ]
                .iter()
                .map(|(handle, x)| (*handle, (x - pos.x).abs()))
                .filter(|(_, distance)| *distance <= GRAB_RADIUS)
                .min_by(|a, b| a.1.total_cmp(&b.1))
                .map(|(handle, _)| handle)
                // Clicking anywhere else on the track just moves the playhead.
                .or(Some(TimelineHandle::Playhead))
            };
        }
    }
    if response.drag_stopped() {
        *drag = None;
    }

    if let (Some(handle), Some(pos)) = (*drag, response.interact_pointer_pos()) {
        let t = to_time(pos.x);
        // One frame is the smallest sensible gap between the two trim cursors.
        let min_gap = info.frame_step();
        match handle {
            TimelineHandle::Start => {
                edit.start = t.min(edit.end - min_gap).max(0.0);
                change.trim = true;
            }
            TimelineHandle::End => {
                edit.end = t.max(edit.start + min_gap).min(duration);
                change.trim = true;
            }
            TimelineHandle::Playhead => {
                edit.playhead = t;
                change.seek = true;
            }
        }
        // The playhead is always inside the kept region.
        let clamped = edit.playhead.clamp(edit.start, edit.end);
        if (clamped - edit.playhead).abs() > f64::EPSILON {
            edit.playhead = clamped;
            change.seek = true;
        }
    }

    if response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
    }

    // ---- painting ---------------------------------------------------------
    let rounding = Rounding::same(4.0);
    painter.rect_filled(track, rounding, theme::TRACK);

    // Kept region.
    let selection = Rect::from_min_max(
        Pos2::new(to_x(edit.start), track.top()),
        Pos2::new(to_x(edit.end), track.bottom()),
    );
    painter.rect_filled(selection, rounding, theme::SELECTION);

    // Trim handles.
    for (handle, time) in [(TimelineHandle::Start, edit.start), (TimelineHandle::End, edit.end)] {
        let x = to_x(time);
        let rect = Rect::from_center_size(
            Pos2::new(x, track.center().y),
            Vec2::new(HANDLE_WIDTH, track.height() + 10.0),
        );
        let active = *drag == Some(handle);
        let color = if active { theme::HANDLE_HOVER } else { theme::HANDLE };
        painter.rect_filled(rect, Rounding::same(3.0), color);
        // Two engraved lines, the usual affordance for a grabbable handle.
        for offset in [-2.0, 2.0] {
            painter.line_segment(
                [
                    Pos2::new(x + offset, rect.top() + 8.0),
                    Pos2::new(x + offset, rect.bottom() - 8.0),
                ],
                Stroke::new(1.0_f32, Color32::from_gray(110)),
            );
        }
    }

    // Playhead.
    let px = to_x(edit.playhead);
    painter.line_segment(
        [Pos2::new(px, track.top() - 6.0), Pos2::new(px, track.bottom() + 6.0)],
        Stroke::new(2.0_f32, theme::PLAYHEAD),
    );
    painter.circle_filled(Pos2::new(px, track.top() - 8.0), 5.0, theme::PLAYHEAD);

    // Labels: trim in/out on the sides, current position under the playhead.
    let font = FontId::monospace(LABEL_SIZE);
    let text_color = ui.visuals().weak_text_color();
    painter.text(
        Pos2::new(track.left(), outer.top() + 2.0),
        Align2::LEFT_TOP,
        format!("in {}", format_timestamp(edit.start)),
        font.clone(),
        text_color,
    );
    painter.text(
        Pos2::new(track.right(), outer.top() + 2.0),
        Align2::RIGHT_TOP,
        format!("out {}", format_timestamp(edit.end)),
        font.clone(),
        text_color,
    );
    // The label is centred on the playhead but kept fully inside the widget,
    // so it is never clipped at the borders.
    let half_label = 42.0;
    painter.text(
        Pos2::new(
            px.clamp(outer.left() + half_label, outer.right() - half_label),
            track.bottom() + 10.0,
        ),
        Align2::CENTER_TOP,
        format_timestamp(edit.playhead),
        font,
        theme::PLAYHEAD,
    );

    change
}
