//! The Valheim-flavoured look, from `docs/palette.md`: burnt wood, polished
//! bone, worn brass, northern mist. Same tokens as the spec document.

use eframe::egui::{self, Color32, FontFamily, FontId, Stroke, TextStyle};

pub const NIGHT: Color32 = Color32::from_rgb(0x0F, 0x0D, 0x0B);
pub const WOOD: Color32 = Color32::from_rgb(0x17, 0x14, 0x0F);
pub const PANEL: Color32 = Color32::from_rgb(0x1E, 0x19, 0x12);
pub const LEATHER: Color32 = Color32::from_rgb(0x26, 0x20, 0x17);
pub const EDGE: Color32 = Color32::from_rgb(0x4A, 0x3C, 0x27);
pub const EDGE_SOFT: Color32 = Color32::from_rgb(0x33, 0x2A, 0x1D);
pub const BONE: Color32 = Color32::from_rgb(0xE0, 0xD5, 0xBC);
pub const BONE_DIM: Color32 = Color32::from_rgb(0xAD, 0xA0, 0x89);
pub const GOLD: Color32 = Color32::from_rgb(0xC7, 0xA4, 0x55);
pub const GOLD_LIT: Color32 = Color32::from_rgb(0xE8, 0xCD, 0x8B);
pub const RUNE: Color32 = Color32::from_rgb(0x7E, 0x9A, 0xA7);
pub const BLOOD: Color32 = Color32::from_rgb(0x9A, 0x34, 0x21);
pub const BLOOD_LIT: Color32 = Color32::from_rgb(0xD2, 0x72, 0x4F);
pub const MOSS: Color32 = Color32::from_rgb(0x7E, 0x91, 0x55);

pub fn apply(ctx: &egui::Context) {
    let mut style = (*ctx.style()).clone();

    let v = &mut style.visuals;
    *v = egui::Visuals::dark();
    v.panel_fill = NIGHT;
    v.window_fill = PANEL;
    v.extreme_bg_color = WOOD;
    v.faint_bg_color = LEATHER;
    v.code_bg_color = WOOD;
    v.override_text_color = Some(BONE);
    v.hyperlink_color = GOLD;
    v.warn_fg_color = GOLD_LIT;
    v.error_fg_color = BLOOD_LIT;
    v.window_stroke = Stroke::new(1.0, EDGE);
    v.window_shadow = egui::epaint::Shadow::NONE;
    v.popup_shadow = egui::epaint::Shadow::NONE;
    v.selection.bg_fill = GOLD.gamma_multiply(0.45);
    v.selection.stroke = Stroke::new(1.0, GOLD_LIT);

    let w = &mut v.widgets;
    w.noninteractive.bg_fill = LEATHER;
    w.noninteractive.weak_bg_fill = LEATHER;
    w.noninteractive.bg_stroke = Stroke::new(1.0, EDGE_SOFT);
    w.noninteractive.fg_stroke = Stroke::new(1.0, BONE_DIM);

    w.inactive.bg_fill = LEATHER;
    w.inactive.weak_bg_fill = LEATHER;
    w.inactive.bg_stroke = Stroke::new(1.0, EDGE_SOFT);
    w.inactive.fg_stroke = Stroke::new(1.0, BONE);

    w.hovered.bg_fill = EDGE_SOFT;
    w.hovered.weak_bg_fill = EDGE_SOFT;
    w.hovered.bg_stroke = Stroke::new(1.0, GOLD);
    w.hovered.fg_stroke = Stroke::new(1.5, GOLD_LIT);

    w.active.bg_fill = EDGE;
    w.active.weak_bg_fill = EDGE;
    w.active.bg_stroke = Stroke::new(1.0, GOLD_LIT);
    w.active.fg_stroke = Stroke::new(1.5, GOLD_LIT);

    w.open.bg_fill = LEATHER;
    w.open.weak_bg_fill = LEATHER;
    w.open.bg_stroke = Stroke::new(1.0, GOLD);
    w.open.fg_stroke = Stroke::new(1.0, BONE);

    style.spacing.button_padding = egui::vec2(14.0, 7.0);
    style.spacing.item_spacing = egui::vec2(10.0, 8.0);
    style.spacing.window_margin = egui::Margin::same(16);

    style.text_styles.insert(
        TextStyle::Heading,
        FontId::new(26.0, FontFamily::Proportional),
    );
    style
        .text_styles
        .insert(TextStyle::Body, FontId::new(15.0, FontFamily::Proportional));
    style.text_styles.insert(
        TextStyle::Button,
        FontId::new(15.0, FontFamily::Proportional),
    );
    style.text_styles.insert(
        TextStyle::Small,
        FontId::new(12.5, FontFamily::Proportional),
    );
    style.text_styles.insert(
        TextStyle::Monospace,
        FontId::new(13.0, FontFamily::Monospace),
    );

    ctx.set_style(style);
}

/// A carved-plate frame for cards.
pub fn card() -> egui::Frame {
    egui::Frame::new()
        .fill(PANEL)
        .stroke(Stroke::new(1.0, EDGE_SOFT))
        .inner_margin(egui::Margin::same(16))
        .corner_radius(4)
}

/// Frame for an error or warning callout, with the blood-red rule.
pub fn callout(color: Color32) -> egui::Frame {
    egui::Frame::new()
        .fill(LEATHER)
        .stroke(Stroke::new(1.0, color.gamma_multiply(0.6)))
        .inner_margin(egui::Margin::symmetric(12, 10))
        .corner_radius(3)
}

/// 32x32 window icon: a gold circle on night with a dark "V" notch.
/// Drawn at start-up so the binary ships no asset.
#[allow(clippy::cast_precision_loss)] // pixel coordinates
pub fn icon() -> egui::IconData {
    const S: usize = 32;
    const SIDE: u32 = 32;
    let mut rgba = Vec::with_capacity(S * S * 4);
    let c = (S as f32 - 1.0) / 2.0;
    for y in 0..S {
        for x in 0..S {
            let dx = x as f32 - c;
            let dy = y as f32 - c;
            let r = (dx * dx + dy * dy).sqrt();
            // V: two diagonal strokes meeting near the bottom
            let in_v = {
                let fy = y as f32;
                let left = (dx + (fy - 8.0) * 0.55).abs() < 2.2 && (8.0..=24.0).contains(&fy);
                let right = (dx - (fy - 8.0) * 0.55).abs() < 2.2 && (8.0..=24.0).contains(&fy);
                left || right
            };
            let px = if r > 15.5 {
                [0, 0, 0, 0]
            } else if r > 14.0 {
                [0x4A, 0x3C, 0x27, 255]
            } else if in_v {
                [0x0F, 0x0D, 0x0B, 255]
            } else {
                [0xC7, 0xA4, 0x55, 255]
            };
            rgba.extend_from_slice(&px);
        }
    }
    egui::IconData {
        rgba,
        width: SIDE,
        height: SIDE,
    }
}
