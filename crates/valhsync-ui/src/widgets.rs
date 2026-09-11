//! Small building blocks both windows use, so a heading, a field or a status
//! dot looks the same everywhere.

use eframe::egui::{self, Color32, RichText};

use crate::theme as th;

/// The product header: the valknut and the name, carved in gold over a
/// faint glow. The glow is the document's `text-shadow` on `h1`.
pub fn header(ui: &mut egui::Ui, title: &str) {
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

/// Dimmed explanatory line under a control.
pub fn hint(ui: &mut egui::Ui, text: &str) {
    ui.label(RichText::new(text).small().color(th::BONE_DIM));
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

/// A coloured dot followed by a label: the status vocabulary of both windows.
pub fn status_dot(ui: &mut egui::Ui, color: Color32, text: &str) {
    ui.horizontal(|ui| {
        dot(ui, color);
        ui.label(RichText::new(text).strong().color(th::BONE));
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
