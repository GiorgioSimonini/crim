//! Frame preview with an interactive crop rectangle drawn on top of it.

use egui::{Color32, CursorIcon, Pos2, Rect, Rounding, Sense, Stroke, TextureHandle, Ui, Vec2};

use super::theme;
use crate::state::Crop;

/// Part of the crop rectangle grabbed by the pointer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CropHandle {
    TopLeft,
    Top,
    TopRight,
    Right,
    BottomRight,
    Bottom,
    BottomLeft,
    Left,
    /// Dragging the inside moves the whole rectangle.
    Inside,
}

impl CropHandle {
    fn cursor(self) -> CursorIcon {
        match self {
            CropHandle::TopLeft | CropHandle::BottomRight => CursorIcon::ResizeNwSe,
            CropHandle::TopRight | CropHandle::BottomLeft => CursorIcon::ResizeNeSw,
            CropHandle::Top | CropHandle::Bottom => CursorIcon::ResizeVertical,
            CropHandle::Left | CropHandle::Right => CursorIcon::ResizeHorizontal,
            CropHandle::Inside => CursorIcon::Move,
        }
    }
}

/// Mutable state the widget needs between frames.
#[derive(Default)]
pub struct CropDrag {
    pub handle: Option<CropHandle>,
    /// Pointer position at drag start, in normalised image coordinates.
    grab_origin: Vec2,
    /// Crop rectangle at drag start (used for a stable "move" gesture).
    grab_crop: Crop,
}

const HANDLE_RADIUS: f32 = 6.0;
/// Hit-test tolerance around handles, in screen pixels.
const GRAB: f32 = 10.0;

/// Draws the frame (letterboxed inside the available area) and the crop
/// overlay. Returns `true` when the crop changed.
pub fn preview(
    ui: &mut Ui,
    texture: Option<&TextureHandle>,
    aspect: f32,
    crop: &mut Crop,
    drag: &mut CropDrag,
) -> bool {
    let mut changed = false;

    let area = ui.available_size();
    let (outer, response) = ui.allocate_exact_size(area, Sense::click_and_drag());
    let painter = ui.painter_at(outer);
    painter.rect_filled(outer, Rounding::ZERO, Color32::from_gray(18));

    // ---- fit the frame inside the panel, preserving the aspect ratio -------
    let aspect = if aspect > 0.0 { aspect } else { 16.0 / 9.0 };
    let mut size = Vec2::new(outer.width(), outer.width() / aspect);
    if size.y > outer.height() {
        size = Vec2::new(outer.height() * aspect, outer.height());
    }
    let image_rect = Rect::from_center_size(outer.center(), size);

    match texture {
        Some(texture) => {
            painter.image(
                texture.id(),
                image_rect,
                // Full UV range: we always draw the whole extracted frame and
                // let the crop overlay live on top of it.
                Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                Color32::WHITE,
            );
        }
        None => {
            painter.text(
                outer.center(),
                egui::Align2::CENTER_CENTER,
                "no frame",
                egui::FontId::proportional(14.0),
                Color32::from_gray(120),
            );
            return false;
        }
    }

    // Conversions between normalised image space and screen space.
    let to_screen = |x: f32, y: f32| {
        Pos2::new(image_rect.left() + x * image_rect.width(), image_rect.top() + y * image_rect.height())
    };
    let to_norm = |pos: Pos2| {
        Vec2::new(
            ((pos.x - image_rect.left()) / image_rect.width()).clamp(0.0, 1.0),
            ((pos.y - image_rect.top()) / image_rect.height()).clamp(0.0, 1.0),
        )
    };

    let crop_rect =
        Rect::from_min_max(to_screen(crop.x, crop.y), to_screen(crop.x + crop.w, crop.y + crop.h));

    // ---- interaction ------------------------------------------------------
    if response.drag_started() {
        if let Some(pos) = response.interact_pointer_pos() {
            drag.handle = hit_test(crop_rect, pos);
            drag.grab_origin = to_norm(pos);
            drag.grab_crop = *crop;
        }
    }
    if response.drag_stopped() {
        drag.handle = None;
    }

    if let (Some(handle), Some(pos)) = (drag.handle, response.interact_pointer_pos()) {
        let p = to_norm(pos);
        let base = drag.grab_crop;

        // Work with edges; recompute x/y/w/h at the end.
        let (mut left, mut top) = (crop.x, crop.y);
        let (mut right, mut bottom) = (crop.x + crop.w, crop.y + crop.h);

        match handle {
            CropHandle::Inside => {
                let delta = p - drag.grab_origin;
                left = base.x + delta.x;
                top = base.y + delta.y;
                right = left + base.w;
                bottom = top + base.h;
                // Moving must not resize: clamp the whole rectangle instead.
                if left < 0.0 {
                    right -= left;
                    left = 0.0;
                }
                if top < 0.0 {
                    bottom -= top;
                    top = 0.0;
                }
                if right > 1.0 {
                    left -= right - 1.0;
                    right = 1.0;
                }
                if bottom > 1.0 {
                    top -= bottom - 1.0;
                    bottom = 1.0;
                }
            }
            _ => {
                if matches!(handle, CropHandle::TopLeft | CropHandle::Left | CropHandle::BottomLeft) {
                    left = p.x;
                }
                if matches!(handle, CropHandle::TopRight | CropHandle::Right | CropHandle::BottomRight) {
                    right = p.x;
                }
                if matches!(handle, CropHandle::TopLeft | CropHandle::Top | CropHandle::TopRight) {
                    top = p.y;
                }
                if matches!(handle, CropHandle::BottomLeft | CropHandle::Bottom | CropHandle::BottomRight)
                {
                    bottom = p.y;
                }
            }
        }

        let mut next = Crop {
            x: left.min(right),
            y: top.min(bottom),
            w: (right - left).abs(),
            h: (bottom - top).abs(),
        };
        next.clamp();

        if (next.x - crop.x).abs() > f32::EPSILON
            || (next.y - crop.y).abs() > f32::EPSILON
            || (next.w - crop.w).abs() > f32::EPSILON
            || (next.h - crop.h).abs() > f32::EPSILON
        {
            *crop = next;
            changed = true;
        }
        ui.ctx().set_cursor_icon(handle.cursor());
    } else if let Some(pos) = response.hover_pos() {
        if let Some(handle) = hit_test(crop_rect, pos) {
            ui.ctx().set_cursor_icon(handle.cursor());
        }
    }

    // ---- overlay ----------------------------------------------------------
    // Darken everything outside the crop, with four rectangles.
    let c = crop_rect;
    for rect in [
        Rect::from_min_max(image_rect.min, Pos2::new(image_rect.right(), c.top())),
        Rect::from_min_max(Pos2::new(image_rect.left(), c.bottom()), image_rect.max),
        Rect::from_min_max(Pos2::new(image_rect.left(), c.top()), Pos2::new(c.left(), c.bottom())),
        Rect::from_min_max(Pos2::new(c.right(), c.top()), Pos2::new(image_rect.right(), c.bottom())),
    ] {
        if rect.is_positive() {
            painter.rect_filled(rect, Rounding::ZERO, theme::OUTSIDE_MASK);
        }
    }

    // Rule-of-thirds guides.
    for i in 1..3 {
        let f = i as f32 / 3.0;
        let x = c.left() + c.width() * f;
        let y = c.top() + c.height() * f;
        painter.line_segment(
            [Pos2::new(x, c.top()), Pos2::new(x, c.bottom())],
            Stroke::new(1.0_f32, theme::CROP_GUIDE),
        );
        painter.line_segment(
            [Pos2::new(c.left(), y), Pos2::new(c.right(), y)],
            Stroke::new(1.0_f32, theme::CROP_GUIDE),
        );
    }

    painter.rect_stroke(c, Rounding::ZERO, Stroke::new(1.5_f32, theme::CROP_BORDER));
    for pos in handle_positions(c) {
        painter.circle_filled(pos, HANDLE_RADIUS, theme::CROP_BORDER);
        painter.circle_stroke(pos, HANDLE_RADIUS, Stroke::new(1.0_f32, Color32::from_gray(30)));
    }

    changed
}

/// The eight resize handles, in the same order as [`hit_test`] checks them.
fn handle_positions(r: Rect) -> [Pos2; 8] {
    [
        r.left_top(),
        Pos2::new(r.center().x, r.top()),
        r.right_top(),
        Pos2::new(r.right(), r.center().y),
        r.right_bottom(),
        Pos2::new(r.center().x, r.bottom()),
        r.left_bottom(),
        Pos2::new(r.left(), r.center().y),
    ]
}

/// Corners and edges win over the inside area.
fn hit_test(crop_rect: Rect, pos: Pos2) -> Option<CropHandle> {
    const ORDER: [CropHandle; 8] = [
        CropHandle::TopLeft,
        CropHandle::Top,
        CropHandle::TopRight,
        CropHandle::Right,
        CropHandle::BottomRight,
        CropHandle::Bottom,
        CropHandle::BottomLeft,
        CropHandle::Left,
    ];
    for (handle, handle_pos) in ORDER.iter().zip(handle_positions(crop_rect)) {
        if (handle_pos - pos).length() <= GRAB + HANDLE_RADIUS {
            return Some(*handle);
        }
    }
    crop_rect.contains(pos).then_some(CropHandle::Inside)
}
