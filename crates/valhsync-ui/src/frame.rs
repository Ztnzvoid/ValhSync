//! Borderless window chrome.
//!
//! Both windows drop the system title bar and draw their own, so the header
//! is part of the design instead of a grey strip above it. That means taking
//! back what the decorations used to provide: dragging, double-click to
//! maximise, the three buttons, and resizing from the edges.

use eframe::egui::viewport::ResizeDirection;
use eframe::egui::{self, Rect, Sense, Stroke, StrokeKind, ViewportCommand};

use crate::theme as th;

/// How close to an edge the pointer must be to start a resize.
const GRAB: f32 = 6.0;
/// Size of one window button.
const BUTTON: egui::Vec2 = egui::vec2(44.0, 30.0);

/// Viewport settings for a borderless window. Use instead of the defaults.
pub fn viewport(title: &str, size: [f32; 2], min_size: [f32; 2]) -> egui::ViewportBuilder {
    egui::ViewportBuilder::default()
        .with_title(title)
        .with_inner_size(size)
        .with_min_inner_size(min_size)
        .with_decorations(false)
        .with_resizable(true)
        .with_icon(th::icon())
}

/// Size the window to what it actually shows, between `min` and `max`.
///
/// Call at the end of a frame. A few pixels of slack stop the window from
/// oscillating when a row appears and disappears; a maximised window is left
/// alone.
pub fn fit_to_content(ctx: &egui::Context, min: egui::Vec2, max: egui::Vec2) {
    if ctx.input(|i| i.viewport().maximized.unwrap_or(false)) {
        return;
    }
    let used = ctx.used_rect().size();
    let want = egui::vec2(
        used.x.clamp(min.x, max.x),
        (used.y + 2.0).clamp(min.y, max.y),
    );
    let have = ctx.screen_rect().size();
    if (want.y - have.y).abs() > 8.0 || (want.x - have.x).abs() > 8.0 {
        ctx.send_viewport_cmd(ViewportCommand::InnerSize(want));
    }
}

/// Draw the window's own edge, on top of everything: with the system frame
/// gone, this hairline is what separates the window from the desktop.
pub fn draw_border(ctx: &egui::Context) {
    let painter = ctx.layer_painter(egui::LayerId::new(
        egui::Order::Foreground,
        egui::Id::new("valhsync-window-border"),
    ));
    painter.rect_stroke(
        ctx.screen_rect().shrink(0.5),
        0.0,
        Stroke::new(1.0, th::EDGE),
        StrokeKind::Inside,
    );
}

/// Resize the window when the pointer grabs an edge. Call once per frame,
/// before anything else draws.
pub fn handle_edge_resize(ctx: &egui::Context) {
    let Some(pos) = ctx.pointer_latest_pos() else {
        return;
    };
    if ctx.input(|i| i.pointer.any_down()) && !ctx.input(|i| i.pointer.primary_pressed()) {
        return; // already dragging something
    }
    let rect = ctx.screen_rect();
    let left = pos.x <= rect.left() + GRAB;
    let right = pos.x >= rect.right() - GRAB;
    let top = pos.y <= rect.top() + GRAB;
    let bottom = pos.y >= rect.bottom() - GRAB;

    let direction = match (left, right, top, bottom) {
        (true, _, true, _) => Some(ResizeDirection::NorthWest),
        (_, true, true, _) => Some(ResizeDirection::NorthEast),
        (true, _, _, true) => Some(ResizeDirection::SouthWest),
        (_, true, _, true) => Some(ResizeDirection::SouthEast),
        (true, ..) => Some(ResizeDirection::West),
        (_, true, ..) => Some(ResizeDirection::East),
        (_, _, true, _) => Some(ResizeDirection::North),
        (_, _, _, true) => Some(ResizeDirection::South),
        _ => None,
    };
    let Some(direction) = direction else {
        return;
    };

    ctx.set_cursor_icon(match direction {
        ResizeDirection::North | ResizeDirection::South => egui::CursorIcon::ResizeVertical,
        ResizeDirection::East | ResizeDirection::West => egui::CursorIcon::ResizeHorizontal,
        ResizeDirection::NorthEast | ResizeDirection::SouthWest => egui::CursorIcon::ResizeNeSw,
        ResizeDirection::NorthWest | ResizeDirection::SouthEast => egui::CursorIcon::ResizeNwSe,
    });
    if ctx.input(|i| i.pointer.primary_pressed()) {
        ctx.send_viewport_cmd(ViewportCommand::BeginResize(direction));
    }
}

/// Make the empty part of a header drag the window, and double-click
/// maximise it, the way a title bar does.
pub fn draggable(ui: &egui::Ui, rect: Rect) {
    let response = ui.interact(rect, ui.id().with("drag-window"), Sense::click_and_drag());
    if response.double_clicked() {
        let maximized = ui.ctx().input(|i| i.viewport().maximized.unwrap_or(false));
        ui.ctx()
            .send_viewport_cmd(ViewportCommand::Maximized(!maximized));
    }
    if response.drag_started_by(egui::PointerButton::Primary) {
        ui.ctx().send_viewport_cmd(ViewportCommand::StartDrag);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Control {
    Minimize,
    Maximize,
    Close,
}

/// Minimise, maximise and close, painted rather than written: the glyphs for
/// these are not in the bundled font.
pub fn window_controls(ui: &mut egui::Ui) {
    for control in [Control::Close, Control::Maximize, Control::Minimize] {
        if control_button(ui, control) {
            let ctx = ui.ctx();
            match control {
                Control::Minimize => ctx.send_viewport_cmd(ViewportCommand::Minimized(true)),
                Control::Maximize => {
                    let maximized = ctx.input(|i| i.viewport().maximized.unwrap_or(false));
                    ctx.send_viewport_cmd(ViewportCommand::Maximized(!maximized));
                }
                Control::Close => ctx.send_viewport_cmd(ViewportCommand::Close),
            }
        }
    }
}

fn control_button(ui: &mut egui::Ui, control: Control) -> bool {
    let (rect, response) = ui.allocate_exact_size(BUTTON, Sense::click());
    let hovered = response.hovered();
    if hovered {
        let fill = if control == Control::Close {
            th::BLOOD
        } else {
            th::EDGE_SOFT
        };
        ui.painter().rect_filled(rect, 0.0, fill);
    }
    let ink = if hovered {
        if control == Control::Close {
            th::BONE
        } else {
            th::GOLD_LIT
        }
    } else {
        th::BONE_DIM
    };
    let stroke = Stroke::new(1.2, ink);
    let c = rect.center();
    let s = 5.0;
    let painter = ui.painter();
    match control {
        Control::Minimize => {
            painter.line_segment([egui::pos2(c.x - s, c.y), egui::pos2(c.x + s, c.y)], stroke);
        }
        Control::Maximize => {
            let maximized = ui.ctx().input(|i| i.viewport().maximized.unwrap_or(false));
            let square = Rect::from_center_size(c, egui::vec2(s * 2.0, s * 2.0));
            painter.rect_stroke(square, 0.0, stroke, StrokeKind::Inside);
            if maximized {
                // The "restore" mark: a second square peeking behind.
                let behind = square.translate(egui::vec2(2.5, -2.5));
                painter.rect_stroke(
                    behind,
                    0.0,
                    Stroke::new(1.0, ink.gamma_multiply(0.55)),
                    StrokeKind::Inside,
                );
            }
        }
        Control::Close => {
            painter.line_segment(
                [egui::pos2(c.x - s, c.y - s), egui::pos2(c.x + s, c.y + s)],
                stroke,
            );
            painter.line_segment(
                [egui::pos2(c.x + s, c.y - s), egui::pos2(c.x - s, c.y + s)],
                stroke,
            );
        }
    }
    response.clicked()
}
