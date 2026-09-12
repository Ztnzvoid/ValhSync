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

/// Shrink a window that opened taller than the screen it landed on.
///
/// The default sizes are chosen so a whole page is visible at once. On a
/// shorter display that same number puts the bottom bar under the taskbar,
/// where the save button cannot be reached at all -- worse than a scroll.
/// Runs once, as soon as the monitor's size is known.
pub fn clamp_to_display(ctx: &egui::Context) {
    let id = egui::Id::new("valhsync-clamped");
    if ctx.memory(|m| m.data.get_temp::<bool>(id)).is_some() {
        return;
    }
    let Some(monitor) = ctx.input(|i| i.viewport().monitor_size) else {
        return; // not known yet; try again next frame
    };
    ctx.memory_mut(|m| m.data.insert_temp(id, true));
    // Room for a taskbar and the window's own trim.
    let usable = monitor * 0.92;
    let have = ctx.screen_rect().size();
    if have.x > usable.x || have.y > usable.y {
        ctx.send_viewport_cmd(ViewportCommand::InnerSize(egui::vec2(
            have.x.min(usable.x),
            have.y.min(usable.y),
        )));
    }
}

/// Size the window to what it actually shows, between `min` and `max`.
///
/// Call at the end of a frame. A few pixels of slack stop the window from
/// oscillating when a row appears and disappears; a maximised window is left
/// alone.
pub fn fit_to_content(ctx: &egui::Context, wanted_height: f32, min: egui::Vec2, max: egui::Vec2) {
    if ctx.input(|i| i.viewport().maximized.unwrap_or(false)) {
        return;
    }
    let have = ctx.screen_rect().size();
    let id = egui::Id::new("valhsync-fit");
    let asked: Option<egui::Vec2> = ctx.memory(|m| m.data.get_temp(id));

    // A size we did not ask for means the person dragged an edge. From then on
    // the window is theirs: measuring the content is a first guess, not a rule
    // to enforce every frame -- enforcing it fought the drag and snapped the
    // window back, which is what made resizing look broken.
    //
    // Width was invisible to this check: `want.x` is copied from `have.x`, so
    // what we asked for always agreed with what we had and dragging the sides
    // never registered. A drag says so for itself now, in `handle_edge_resize`
    // -- this comparison is only a backstop for a size the system changed
    // without us, such as a snap to half the screen.
    if let Some(asked) = asked
        && (asked - have).abs().max_elem() > 24.0
    {
        ctx.memory_mut(|m| m.data.insert_temp(id, MANUAL));
    }
    if asked == Some(MANUAL) {
        return;
    }

    let want = egui::vec2(
        have.x.clamp(min.x, max.x),
        wanted_height.clamp(min.y, max.y),
    );
    if (want - have).abs().max_elem() > 8.0 {
        ctx.send_viewport_cmd(ViewportCommand::InnerSize(want));
    }
    ctx.memory_mut(|m| m.data.insert_temp(id, want));
}

/// Stands in for "the person resized this window"; no real size is negative.
const MANUAL: egui::Vec2 = egui::vec2(-1.0, -1.0);

/// Hand the window over: from here on it is sized by the person, not by its
/// contents. Called the moment a drag starts, because during the drag the
/// system is resizing the window and a measurement sent in the same frame
/// pulls it straight back.
pub fn release_to_user(ctx: &egui::Context) {
    ctx.memory_mut(|m| m.data.insert_temp(egui::Id::new("valhsync-fit"), MANUAL));
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
        release_to_user(ctx);
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
