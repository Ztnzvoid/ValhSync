//! Small building blocks both windows use, so a heading, a field or a status
//! dot looks the same everywhere.

use eframe::egui::{self, Color32, RichText};

use crate::theme as th;

/// The product header: the valknut and the name, carved in gold over a
/// faint glow. The glow is the document's `text-shadow` on `h1`.
///
/// `tag` is the stage of the thing, set beside the name and not in gold:
/// somebody who has this window open should be able to see, without going
/// looking for it, that they are running something early.
pub fn header(ui: &mut egui::Ui, title: &str, tag: Option<&str>) {
    ui.horizontal(|ui| {
        // Algiz, struck like a maker's mark to the left of the name.
        let mark = 30.0;
        let (mark_rect, _) = ui.allocate_exact_size(egui::vec2(mark, mark), egui::Sense::hover());
        th::glow(ui.painter(), mark_rect, th::GOLD.gamma_multiply(0.12));
        th::rune(
            ui.painter(),
            mark_rect.center(),
            mark * 0.50,
            egui::Stroke::new(1.8, th::GOLD),
        );
        ui.add_space(10.0);
        let galley =
            ui.painter()
                .layout_no_wrap(title.to_owned(), th::display_font(23.0), th::GOLD);
        let (rect, _) = ui.allocate_exact_size(galley.size(), egui::Sense::hover());
        th::glow(ui.painter(), rect, th::GOLD.gamma_multiply(0.18));
        ui.painter().galley(rect.min, galley, th::GOLD);
        if let Some(tag) = tag {
            ui.add_space(10.0);
            let galley =
                ui.painter()
                    .layout_no_wrap(tag.to_owned(), th::display_font(12.0), th::BONE_DIM);
            // Sat on the baseline of the name rather than centred on it: level
            // with the name it qualifies, the way a version sits on a spine.
            let (rect, _) = ui.allocate_exact_size(galley.size(), egui::Sense::hover());
            let drop = egui::vec2(0.0, 9.0);
            ui.painter().galley(rect.min + drop, galley, th::BONE_DIM);
        }
    });
}

/// Section title inside a card: the number in northern mist, the words in
/// brass small-caps, a hairline underneath. The document's `h2`, in a window.
pub fn section(ui: &mut egui::Ui, text: &str) {
    let (number, words) = text.split_once('·').unwrap_or(("", text));
    ui.horizontal(|ui| {
        if !number.is_empty() {
            ui.label(
                RichText::new(number.trim())
                    .font(th::display_font(15.0))
                    .color(th::RUNE),
            );
        }
        ui.label(
            RichText::new(words.trim().to_uppercase())
                .font(th::display_font(15.0))
                .strong()
                .color(th::GOLD),
        );
    });
    ui.add_space(3.0);
    th::hairline(ui);
    ui.add_space(7.0);
}

/// The window's tabs, carved in the display face over a hairline: the open
/// one lit and underlined, the others waiting in bone.
///
/// `items` is `(value, label)`; clicking one writes it into `current`.
pub fn tabs<T: Copy + PartialEq>(ui: &mut egui::Ui, current: &mut T, items: &[(T, &str)]) {
    const SIZE: f32 = 15.0;
    const PAD: egui::Vec2 = egui::vec2(16.0, 8.0);

    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        for (value, label) in items {
            let open = *current == *value;
            let galley = ui.painter().layout_no_wrap(
                (*label).to_owned(),
                th::display_font(SIZE),
                th::BONE_DIM,
            );
            let (rect, response) =
                ui.allocate_exact_size(galley.size() + PAD * 2.0, egui::Sense::click());
            let hovered = response.hovered();
            let ink = match (open, hovered) {
                (true, _) => th::GOLD_LIT,
                (false, true) => th::BONE,
                (false, false) => th::BONE_DIM,
            };
            if open {
                th::glow(ui.painter(), rect, th::GOLD.gamma_multiply(0.14));
            }
            ui.painter().galley(rect.min + PAD, galley, ink);

            // The lit bar under the open tab, tapered at both ends so it reads
            // as struck rather than drawn.
            let base = rect.bottom() - 1.0;
            let bar = egui::Rect::from_min_max(
                egui::pos2(rect.left() + PAD.x * 0.5, base - 2.0),
                egui::pos2(rect.right() - PAD.x * 0.5, base),
            );
            if open {
                th::glow(ui.painter(), bar.expand(3.0), th::GOLD.gamma_multiply(0.30));
                ui.painter().rect_filled(bar, 1.0, th::GOLD);
            } else if hovered {
                ui.painter().rect_filled(bar, 1.0, th::EDGE);
            }
            if response.clicked() {
                *current = *value;
            }
        }
    });
    th::hairline(ui);
}

/// A modal panel: the body scrolls if it has to, so whatever the caller draws
/// after it -- the buttons that dismiss it -- is always on screen.
///
/// Windows can be small, and a fingerprint to confirm is a lot of text; pushing
/// Trust and Cancel past the bottom edge leaves no way out of the dialog.
pub fn dialog_body<R>(ui: &mut egui::Ui, add_contents: impl FnOnce(&mut egui::Ui) -> R) -> R {
    egui::ScrollArea::vertical()
        .max_height(dialog_room(ui, 190.0))
        .auto_shrink([false, true])
        .show(ui, add_contents)
        .inner
}

/// How tall a dialog's scrolling part may be: the window, less `reserve` for
/// the title bar, the buttons and the margins. A height fixed in pixels looks
/// right on the window it was written for and overflows every smaller one.
#[must_use]
pub fn dialog_room(ui: &egui::Ui, reserve: f32) -> f32 {
    // A share of the window as well as a margin off it. Subtracting a fixed
    // reserve was tuned when a note could not pass two thousand characters;
    // at four times that, on a tall screen, the result was a dialog taller
    // than the window it belongs to -- text running off the top and the
    // bottom with nothing to scroll, because the thing overflowing was the
    // dialog itself and not its contents.
    let screen = ui.ctx().screen_rect().height();
    (screen - reserve).clamp(120.0, screen * 0.72)
}

/// Dimmed explanatory line under a control.
pub fn hint(ui: &mut egui::Ui, text: &str) {
    ui.label(RichText::new(text).small().color(th::BONE_DIM));
}

/// A lamp on the current row: `height` is the text it sits beside, so the two
/// share a centre line.
pub fn lamp(ui: &mut egui::Ui, colour: Color32, height: f32) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(height, height), egui::Sense::hover());
    th::lamp(ui.painter(), rect.center(), height * 0.20, colour);
}

/// A progress bar cut like the rest of the window: a carved track, a lit
/// fill, and a sheen travelling along it while the work is live.
///
/// egui's own is a grey rounded rectangle and looks like it came from another
/// program, which on the one screen somebody watches while they wait is the
/// wrong impression to leave.
///
/// `fraction` outside 0..=1 is clamped rather than refused: a download that
/// reports more bytes than it promised is a bug in the count, not a reason to
/// stop drawing.
pub fn progress(ui: &mut egui::Ui, fraction: f32, height: f32) {
    let width = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::hover());
    let round = egui::CornerRadius::same(2);

    // The track: the same cut as a field, so a bar and an input read as parts
    // of one object rather than two borrowed widgets.
    ui.painter().rect_filled(rect, round, th::NIGHT);
    ui.painter().rect_stroke(
        rect,
        round,
        egui::Stroke::new(1.0, th::EDGE_SOFT),
        egui::StrokeKind::Inside,
    );

    let done = fraction.clamp(0.0, 1.0);
    if done <= f32::EPSILON {
        return;
    }
    let mut lit = rect;
    lit.set_width(rect.width() * done);
    // Under the fill, so the glow reads as the metal being hot rather than as
    // a shape drawn on top of it.
    th::glow(ui.painter(), lit, th::GOLD.gamma_multiply(0.30));
    ui.painter().rect_filled(lit, round, th::GOLD);

    // The leading edge, brighter: a bar with a lit end reads as moving even
    // in a still frame.
    let edge = egui::Rect::from_min_max(
        egui::pos2((lit.max.x - 18.0).max(lit.min.x), lit.min.y),
        lit.max,
    );
    ui.painter().rect_filled(edge, round, th::GOLD_LIT);

    // And a sheen travelling across the filled part while there is work left.
    if done < 1.0 {
        let t = ui.ctx().input(|i| i.time % 3600.0);
        #[allow(clippy::cast_possible_truncation)]
        let phase = ((t as f32) * 0.55).fract();
        let x = lit.min.x + lit.width() * phase;
        let sheen = egui::Rect::from_min_max(
            egui::pos2((x - 26.0).max(lit.min.x), lit.min.y),
            egui::pos2((x + 26.0).min(lit.max.x), lit.max.y),
        );
        if sheen.width() > 1.0 {
            ui.painter()
                .rect_filled(sheen, round, Color32::from_white_alpha(26));
        }
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(40));
    }
}

/// The widest a column of text is allowed to get, whatever the window does.
///
/// A window maximised on an ultrawide is four thousand pixels of line length,
/// and a sentence that wide cannot be read: the eye loses the start of the
/// next line. Everything in these windows is prose, lists and rows of two
/// things far apart, all of which want a column rather than a canvas.
pub const COLUMN: f32 = 860.0;

/// Lay content out in a centred column no wider than [`COLUMN`].
///
/// Narrow windows are unaffected -- the column is simply the window. It is
/// the wide ones this exists for, where the alternative is a name on the far
/// left, a status on the far right, and a metre of nothing between them.
pub fn column<R>(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    let available = ui.available_width();
    let width = available.min(COLUMN);
    let pad = ((available - width) * 0.5).max(0.0);
    let mut out = None;
    ui.horizontal_top(|ui| {
        ui.add_space(pad);
        ui.allocate_ui_with_layout(
            egui::vec2(width, 0.0),
            egui::Layout::top_down(egui::Align::Min),
            |ui| {
                ui.set_width(width);
                out = Some(add(ui));
            },
        );
    });
    out.expect("the column body always runs")
}

/// A section that can be folded away, with its own memory.
///
/// One mechanism for "there is too much of this to show at once", used
/// everywhere, instead of a scroll area here, a `show all` button opening a
/// dialog there, and a hard cut somewhere else. `count` goes in the header so
/// the size is known before it is opened, and the body is bounded and scrolls
/// past `max_body` so that expanding something enormous still leaves the rest
/// of the window reachable.
///
/// Returns whether it is open, for a caller that wants to skip expensive work
/// while it is not.
pub fn collapsible(
    ui: &mut egui::Ui,
    id: impl std::hash::Hash,
    title: &str,
    count: Option<usize>,
    default_open: bool,
    max_body: f32,
    body: impl FnOnce(&mut egui::Ui),
) -> bool {
    let id = ui.make_persistent_id(id);
    let mut open = ui.data_mut(|d| *d.get_persisted_mut_or(id, default_open));

    let header = match count {
        Some(n) => format!("{title}  ({n})"),
        None => title.to_string(),
    };
    // The whole row is the target, not a chevron six pixels wide.
    let response = ui
        .scope(|ui| {
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(if open { "\u{25BE}" } else { "\u{25B8}" })
                        .color(th::GOLD)
                        .monospace(),
                );
                ui.label(
                    RichText::new(header)
                        .font(th::display_font(13.0))
                        .color(th::GOLD_LIT),
                );
            });
        })
        .response
        .interact(egui::Sense::click());
    if response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    if response.clicked() {
        open = !open;
        ui.data_mut(|d| d.insert_persisted(id, open));
    }

    if open {
        ui.add_space(4.0);
        egui::ScrollArea::vertical()
            .id_salt(id)
            .max_height(max_body)
            .auto_shrink([false, true])
            .show(ui, body);
    }
    open
}

/// The width every line on a card indents by, so their text shares one left
/// edge whether the line opens with a lamp, a dot or nothing at all.
///
/// Three different left edges on three consecutive lines reads as a mistake
/// even to somebody who could not say what is wrong with it.
pub const GUTTER: f32 = 26.0;

/// A dot centred in the gutter, for a line that sits under one opened by a
/// lamp.
pub fn gutter_dot(ui: &mut egui::Ui, color: Color32) {
    let (rect, _) = ui.allocate_exact_size(
        egui::vec2(GUTTER, ui.text_style_height(&egui::TextStyle::Body)),
        egui::Sense::hover(),
    );
    let size = ui.text_style_height(&egui::TextStyle::Body) * 0.45;
    ui.painter()
        .circle_filled(rect.center(), size * 0.62, color);
}

/// A name cut the way the product's own mark is cut: capitals with air
/// between them.
///
/// The header spells VALHSYNC with spaces in the literal because egui has no
/// letter spacing. A server's name is written by its admin, so it gets the
/// same treatment here rather than being the one piece of display type on
/// the window set solid.
#[must_use]
pub fn spaced(name: &str) -> String {
    // A thin space, not a full one: this is letter spacing, not words.
    name.trim()
        .chars()
        .map(|c| c.to_string())
        .collect::<Vec<_>>()
        .join("\u{2009}")
}

/// A filled dot, painted rather than written: the bullet glyph is missing
/// from egui's bundled font and would show up as an empty box.
pub fn dot(ui: &mut egui::Ui, color: Color32) {
    let size = ui.text_style_height(&egui::TextStyle::Body) * 0.45;
    let (rect, _) = ui.allocate_exact_size(
        egui::vec2(size * 2.0, ui.text_style_height(&egui::TextStyle::Body)),
        egui::Sense::hover(),
    );
    ui.painter()
        .circle_filled(rect.center(), size * 0.62, color);
}

/// A lamp followed by a carved label: the strongest form of the status row,
/// for the one or two states a window is really about.
pub fn status_dot_lit(ui: &mut egui::Ui, color: Color32, text: &str) {
    ui.horizontal(|ui| {
        lamp(ui, color, 22.0);
        ui.label(
            RichText::new(text)
                .text_style(th::label_style())
                .color(th::BONE),
        );
    });
}

/// A coloured dot followed by a label: the status vocabulary of both windows.
pub fn status_dot(ui: &mut egui::Ui, color: Color32, text: &str) {
    ui.horizontal(|ui| {
        dot(ui, color);
        ui.label(
            RichText::new(text)
                .text_style(th::label_style())
                .color(th::BONE),
        );
    });
}

/// A labelled single-line field, label above, monospace input below.
pub fn field(ui: &mut egui::Ui, label: &str, value: &mut String, width: f32) -> egui::Response {
    ui.label(RichText::new(label).small().color(th::BONE_DIM));
    ui.add(
        egui::TextEdit::singleline(value)
            .desired_width(width)
            .font(egui::TextStyle::Monospace),
    )
}

/// A warning or error plate: leather ground, coloured rule down the left,
/// coloured text.
pub fn notice(ui: &mut egui::Ui, color: Color32, text: &str) {
    th::callout(ui, color, |ui| {
        ui.label(RichText::new(text).color(color));
    });
}

/// Monospace text the user is meant to copy, on the dark ground and with the
/// blood rule of the document's log block.
pub fn code_block(ui: &mut egui::Ui, text: &str) {
    let mut owned = text.to_string();
    ui.add(
        egui::TextEdit::multiline(&mut owned)
            .desired_rows(2)
            .desired_width(f32::INFINITY)
            .font(egui::TextStyle::Monospace)
            .interactive(true),
    );
}

/// A square button holding a drawn icon rather than a glyph. `paint` receives
/// the area the icon should fill and the colour it should use.
pub fn icon_button(
    ui: &mut egui::Ui,
    enabled: bool,
    tooltip: &str,
    paint: impl FnOnce(&egui::Painter, egui::Rect, Color32),
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(40.0, 40.0), egui::Sense::click());
    let hovered = response.hovered() && enabled;
    ui.painter().rect(
        rect,
        3.0,
        if hovered { th::EDGE_SOFT } else { th::LEATHER },
        egui::Stroke::new(1.0, if hovered { th::GOLD } else { th::EDGE_SOFT }),
        egui::StrokeKind::Inside,
    );
    let ink = match (enabled, hovered) {
        (false, _) => th::BONE_DIM.gamma_multiply(0.45),
        (true, false) => th::BONE_DIM,
        (true, true) => th::GOLD_LIT,
    };
    paint(ui.painter(), rect.shrink(11.0), ink);
    let response = response.on_hover_text(tooltip);
    if enabled {
        response
    } else {
        response.on_disabled_hover_text(tooltip)
    }
}
