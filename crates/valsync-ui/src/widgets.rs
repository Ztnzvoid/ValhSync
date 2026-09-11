//! Small building blocks both windows use, so a heading, a field or a status
//! dot looks the same everywhere.

use eframe::egui::{self, Color32, RichText};

use crate::theme as th;

/// The product header: name in carved gold, one line of subtitle.
pub fn header(ui: &mut egui::Ui, title: &str, subtitle: &str) {
    ui.vertical(|ui| {
        ui.label(RichText::new(title).size(24.0).strong().color(th::GOLD));
        ui.label(RichText::new(subtitle).small().color(th::BONE_DIM));
    });
}

/// Section title inside a card.
pub fn section(ui: &mut egui::Ui, text: &str) {
    ui.label(RichText::new(text).size(16.0).strong().color(th::GOLD_LIT));
    ui.add_space(2.0);
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

/// The one primary action of a screen: gold plate, dark text.
pub fn primary_button(
    ui: &mut egui::Ui,
    label: &str,
    enabled: bool,
    size: egui::Vec2,
) -> egui::Response {
    ui.add_enabled(
        enabled,
        egui::Button::new(RichText::new(label).size(18.0).strong().color(th::NIGHT))
            .fill(th::GOLD)
            .min_size(size),
    )
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

/// A warning or error plate: leather ground, coloured rule, coloured text.
pub fn notice(ui: &mut egui::Ui, color: Color32, text: &str) {
    th::callout(color).show(ui, |ui| {
        ui.label(RichText::new(text).color(color));
    });
}

/// Monospace read-only text the user is meant to copy (invite codes, paths).
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
