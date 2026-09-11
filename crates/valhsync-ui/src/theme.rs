//! The look shared by both windows: burnt wood, polished bone, worn brass,
//! northern mist. The palette and the effects follow `docs/palette.md` and
//! the specification document's own stylesheet, so the windows and the paper
//! read as one product.

use eframe::egui::{
    self, Color32, FontFamily, FontId, Rect, Stroke, StrokeKind, TextStyle, TextureWrapMode,
};

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

/// Font family used for headings.
pub const DISPLAY: &str = "cinzel";

/// Cinzel, SIL Open Font License 1.1. A Roman inscriptional face: the closest
/// thing to letters carved in stone, which is what the document's headings
/// imitate. The licence travels with the source in `assets/OFL-Cinzel.txt`.
const CINZEL: &[u8] = include_bytes!("../assets/Cinzel.ttf");

/// The machine's own interface font, for controls and body text. Cinzel is a
/// display face: a whole form set in it would be unreadable.
fn system_ui_font() -> Option<Vec<u8>> {
    const CANDIDATES: &[&str] = &[
        r"C:\Windows\Fonts\segoeui.ttf",
        "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
        "/usr/share/fonts/TTF/DejaVuSans.ttf",
        "/System/Library/Fonts/Supplemental/Helvetica.ttc",
    ];
    CANDIDATES
        .iter()
        .find_map(|p| std::fs::read(p).ok().filter(|b| !b.is_empty()))
}

fn install_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();

    fonts.font_data.insert(
        DISPLAY.to_owned(),
        std::sync::Arc::new(egui::FontData::from_static(CINZEL)),
    );
    let mut display = fonts
        .families
        .get(&FontFamily::Proportional)
        .cloned()
        .unwrap_or_default();
    display.insert(0, DISPLAY.to_owned());
    fonts
        .families
        .insert(FontFamily::Name(DISPLAY.into()), display);

    if let Some(bytes) = system_ui_font() {
        fonts.font_data.insert(
            "ui".to_owned(),
            std::sync::Arc::new(egui::FontData::from_owned(bytes)),
        );
        if let Some(family) = fonts.families.get_mut(&FontFamily::Proportional) {
            family.insert(0, "ui".to_owned());
        }
    }
    ctx.set_fonts(fonts);
}

/// A heading font at `size`, in the carved face.
pub fn display_font(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(DISPLAY.into()))
}

pub fn apply(ctx: &egui::Context) {
    install_fonts(ctx);
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
    w.inactive.corner_radius = 3.into();

    w.hovered.bg_fill = EDGE_SOFT;
    w.hovered.weak_bg_fill = EDGE_SOFT;
    w.hovered.bg_stroke = Stroke::new(1.0, GOLD);
    w.hovered.fg_stroke = Stroke::new(1.5, GOLD_LIT);
    w.hovered.corner_radius = 3.into();

    w.active.bg_fill = EDGE;
    w.active.weak_bg_fill = EDGE;
    w.active.bg_stroke = Stroke::new(1.0, GOLD_LIT);
    w.active.fg_stroke = Stroke::new(1.5, GOLD_LIT);
    w.active.corner_radius = 3.into();

    w.open.bg_fill = LEATHER;
    w.open.weak_bg_fill = LEATHER;
    w.open.bg_stroke = Stroke::new(1.0, GOLD);
    w.open.fg_stroke = Stroke::new(1.0, BONE);

    style.spacing.button_padding = egui::vec2(14.0, 7.0);
    style.spacing.item_spacing = egui::vec2(10.0, 8.0);
    style.spacing.window_margin = egui::Margin::same(18);

    style
        .text_styles
        .insert(TextStyle::Heading, display_font(26.0));
    style
        .text_styles
        .insert(TextStyle::Body, FontId::new(15.0, FontFamily::Proportional));
    style.text_styles.insert(
        TextStyle::Button,
        FontId::new(14.5, FontFamily::Proportional),
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

/// A leather plate with a thick coloured rule down its left edge, exactly
/// like the `.rule` blocks of the specification document.
pub fn callout<R>(
    ui: &mut egui::Ui,
    color: Color32,
    add_contents: impl FnOnce(&mut egui::Ui) -> R,
) -> R {
    let inner = egui::Frame::new()
        .fill(LEATHER)
        .inner_margin(egui::Margin {
            left: 16,
            right: 12,
            top: 10,
            bottom: 10,
        })
        .corner_radius(2)
        .show(ui, add_contents);
    let rect = inner.response.rect;
    let bar = Rect::from_min_max(rect.min, egui::pos2(rect.min.x + 4.0, rect.max.y));
    ui.painter().rect_filled(bar, 0.0, color);
    inner.inner
}

// ---- background -----------------------------------------------------------

/// The ground of every window: burnt wood, the torch halo of the document, a
/// grain so the surface is a material rather than a flat colour, and a
/// vignette pulling the corners into shadow.
pub fn backdrop(ctx: &egui::Context, painter: &egui::Painter, rect: Rect) {
    painter.rect_filled(rect, 0.0, NIGHT);
    radial_pool(
        painter,
        egui::pos2(rect.center().x, rect.min.y),
        rect.width().max(rect.height()) * 0.85,
        Color32::from_rgb(0x24, 0x1D, 0x14),
    );
    radial_pool(
        painter,
        egui::pos2(rect.center().x, rect.max.y),
        rect.width() * 0.6,
        Color32::from_rgb(0x1A, 0x16, 0x10),
    );
    grain(ctx, painter, rect);
    vignette(painter, rect);
}

/// Tile a small noise texture over `rect` at low opacity.
fn grain(ctx: &egui::Context, painter: &egui::Painter, rect: Rect) {
    let texture = grain_texture(ctx);
    let mut mesh = egui::Mesh::with_texture(texture.id());
    let scale = 1.0 / 96.0;
    mesh.add_rect_with_uv(
        rect,
        Rect::from_min_size(
            egui::pos2(0.0, 0.0),
            egui::vec2(rect.width() * scale, rect.height() * scale),
        ),
        Color32::from_white_alpha(14),
    );
    painter.add(egui::Shape::mesh(mesh));
}

/// A deterministic noise tile, built once and kept in the context.
#[allow(clippy::cast_precision_loss)]
fn grain_texture(ctx: &egui::Context) -> egui::TextureHandle {
    const SIDE: usize = 96;
    let id = egui::Id::new("valhsync-grain");
    if let Some(handle) = ctx.data(|d| d.get_temp::<egui::TextureHandle>(id)) {
        return handle;
    }
    // A tiny xorshift: the same grain on every machine, and no dependency.
    let mut state: u32 = 0x1234_5678;
    let mut pixels = Vec::with_capacity(SIDE * SIDE);
    for _ in 0..SIDE * SIDE {
        state ^= state << 13;
        state ^= state >> 17;
        state ^= state << 5;
        #[allow(clippy::cast_possible_truncation)]
        let n = (state >> 24) as u8;
        // Mostly transparent, a few brighter specks: dust on wood.
        pixels.push(Color32::from_white_alpha(n / 6));
    }
    let image = egui::ColorImage {
        size: [SIDE, SIDE],
        pixels,
        source_size: egui::vec2(SIDE as f32, SIDE as f32),
    };
    let handle = ctx.load_texture(
        "valhsync-grain",
        image,
        egui::TextureOptions {
            wrap_mode: TextureWrapMode::Repeat,
            ..egui::TextureOptions::LINEAR
        },
    );
    ctx.data_mut(|d| d.insert_temp(id, handle.clone()));
    handle
}

/// Darken the four edges, so the eye falls to the middle of the window.
fn vignette(painter: &egui::Painter, rect: Rect) {
    let depth = (rect.width().min(rect.height()) * 0.22).max(40.0);
    let shade = Color32::from_rgba_unmultiplied(0, 0, 0, 120);
    let clear = Color32::from_rgba_unmultiplied(0, 0, 0, 0);
    let edges = [
        (
            Rect::from_min_max(rect.min, egui::pos2(rect.max.x, rect.min.y + depth)),
            true,
            true,
        ),
        (
            Rect::from_min_max(egui::pos2(rect.min.x, rect.max.y - depth), rect.max),
            true,
            false,
        ),
        (
            Rect::from_min_max(rect.min, egui::pos2(rect.min.x + depth, rect.max.y)),
            false,
            true,
        ),
        (
            Rect::from_min_max(egui::pos2(rect.max.x - depth, rect.min.y), rect.max),
            false,
            false,
        ),
    ];
    for (area, vertical, from_start) in edges {
        let mut mesh = egui::Mesh::default();
        let (a, b) = if from_start {
            (shade, clear)
        } else {
            (clear, shade)
        };
        if vertical {
            mesh.colored_vertex(area.left_top(), a);
            mesh.colored_vertex(area.right_top(), a);
            mesh.colored_vertex(area.left_bottom(), b);
            mesh.colored_vertex(area.right_bottom(), b);
        } else {
            mesh.colored_vertex(area.left_top(), a);
            mesh.colored_vertex(area.left_bottom(), a);
            mesh.colored_vertex(area.right_top(), b);
            mesh.colored_vertex(area.right_bottom(), b);
        }
        mesh.add_triangle(0, 1, 2);
        mesh.add_triangle(1, 2, 3);
        painter.add(egui::Shape::mesh(mesh));
    }
}

/// One radial gradient, as a triangle fan from an opaque centre to a
/// transparent rim. egui has no gradient primitive, so we build the mesh.
fn radial_pool(painter: &egui::Painter, centre: egui::Pos2, radius: f32, colour: Color32) {
    const STEPS: usize = 48;
    let mut mesh = egui::Mesh::default();
    mesh.colored_vertex(centre, colour);
    let rim = Color32::from_rgba_unmultiplied(colour.r(), colour.g(), colour.b(), 0);
    for i in 0..=STEPS {
        #[allow(clippy::cast_precision_loss)]
        let angle = i as f32 / STEPS as f32 * std::f32::consts::TAU;
        mesh.colored_vertex(
            centre + egui::vec2(angle.cos() * radius, angle.sin() * radius),
            rim,
        );
    }
    for i in 1..=STEPS {
        #[allow(clippy::cast_possible_truncation)]
        mesh.add_triangle(0, i as u32, i as u32 + 1);
    }
    painter.add(egui::Shape::mesh(mesh));
}

/// A soft halo behind a heading, standing in for the document's text-shadow.
pub fn glow(painter: &egui::Painter, rect: Rect, colour: Color32) {
    radial_pool(
        painter,
        rect.center(),
        rect.width().max(rect.height()),
        colour,
    );
}

/// A hairline under a heading, the way `h2` is ruled in the document.
pub fn hairline(ui: &mut egui::Ui) {
    let width = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(egui::vec2(width, 1.0), egui::Sense::hover());
    ui.painter().rect_filled(rect, 0.0, EDGE_SOFT);
}

// ---- Norse marks ----------------------------------------------------------

/// The three triangles of a valknut, as points. Shared by the painter and the
/// icon rasteriser so the mark is identical everywhere.
fn valknut_triangles(centre: [f32; 2], radius: f32) -> Vec<[[f32; 2]; 3]> {
    let mut out = Vec::with_capacity(3);
    for k in 0..3 {
        #[allow(clippy::cast_precision_loss)]
        let spin = k as f32 * std::f32::consts::TAU / 3.0 - std::f32::consts::FRAC_PI_2;
        // Offset and size stay close: further apart the triangles stop
        // touching, closer together they read as a single shape.
        let hub = [
            centre[0] + spin.cos() * radius * 0.48,
            centre[1] + spin.sin() * radius * 0.48,
        ];
        let mut points = [[0.0; 2]; 3];
        for (corner, point) in points.iter_mut().enumerate() {
            #[allow(clippy::cast_precision_loss)]
            let angle = -std::f32::consts::FRAC_PI_2 + corner as f32 * std::f32::consts::TAU / 3.0;
            *point = [
                hub[0] + angle.cos() * radius * 0.54,
                hub[1] + angle.sin() * radius * 0.54,
            ];
        }
        out.push(points);
    }
    out
}

/// The valknut: three interlocking triangles, the knot of the slain. Drawn
/// from line segments, so it stays crisp at any size and ships as no asset.
pub fn valknut(painter: &egui::Painter, centre: egui::Pos2, radius: f32, stroke: Stroke) {
    for triangle in valknut_triangles([centre.x, centre.y], radius) {
        for i in 0..3 {
            let a = triangle[i];
            let b = triangle[(i + 1) % 3];
            painter.line_segment([egui::pos2(a[0], a[1]), egui::pos2(b[0], b[1])], stroke);
        }
    }
}

/// Carved brackets at the corners of a plate, like the iron at the corners of
/// a chest.
pub fn brackets(painter: &egui::Painter, rect: Rect, colour: Color32) {
    let arm = 10.0_f32.min(rect.width() * 0.12).min(rect.height() * 0.3);
    let stroke = Stroke::new(1.2, colour);
    let corners = [
        (rect.left_top(), egui::vec2(1.0, 0.0), egui::vec2(0.0, 1.0)),
        (
            rect.right_top(),
            egui::vec2(-1.0, 0.0),
            egui::vec2(0.0, 1.0),
        ),
        (
            rect.left_bottom(),
            egui::vec2(1.0, 0.0),
            egui::vec2(0.0, -1.0),
        ),
        (
            rect.right_bottom(),
            egui::vec2(-1.0, 0.0),
            egui::vec2(0.0, -1.0),
        ),
    ];
    for (corner, along, down) in corners {
        painter.line_segment([corner, corner + along * arm], stroke);
        painter.line_segment([corner, corner + down * arm], stroke);
    }
}

/// 64x64 window icon: the valknut in brass on burnt wood.
#[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]
pub fn icon() -> egui::IconData {
    const S: i32 = 64;
    let centre = (S as f32 - 1.0) / 2.0;
    let radius = S as f32 * 0.40;

    let segments: Vec<([f32; 2], [f32; 2])> = valknut_triangles([centre, centre], radius)
        .into_iter()
        .flat_map(|t| [(t[0], t[1]), (t[1], t[2]), (t[2], t[0])])
        .collect();

    let distance_to_segment = |p: [f32; 2], a: [f32; 2], b: [f32; 2]| -> f32 {
        let (vx, vy) = (b[0] - a[0], b[1] - a[1]);
        let (wx, wy) = (p[0] - a[0], p[1] - a[1]);
        let len2 = vx.mul_add(vx, vy * vy);
        let t = if len2 <= f32::EPSILON {
            0.0
        } else {
            (wx.mul_add(vx, wy * vy) / len2).clamp(0.0, 1.0)
        };
        let (dx, dy) = (vx.mul_add(-t, wx), vy.mul_add(-t, wy));
        dx.mul_add(dx, dy * dy).sqrt()
    };

    let mut rgba = Vec::with_capacity((S * S * 4) as usize);
    for y in 0..S {
        for x in 0..S {
            let p = [x as f32, y as f32];
            let dx = p[0] - centre;
            let dy = p[1] - centre;
            let r = dx.mul_add(dx, dy * dy).sqrt();
            let ink = segments
                .iter()
                .map(|(a, b)| distance_to_segment(p, *a, *b))
                .fold(f32::MAX, f32::min);

            let pixel = if ink < 1.5 {
                [0xE8, 0xCD, 0x8B, 255] // brass, lit
            } else if ink < 2.3 {
                [0xC7, 0xA4, 0x55, 255] // brass
            } else if r > S as f32 * 0.49 {
                [0, 0, 0, 0] // outside the disc
            } else if r > S as f32 * 0.45 {
                [0x4A, 0x3C, 0x27, 255] // rim
            } else {
                [0x17, 0x14, 0x0F, 255] // burnt wood
            };
            rgba.extend_from_slice(&pixel);
        }
    }
    egui::IconData {
        rgba,
        width: S as u32,
        height: S as u32,
    }
}

/// A rounded plate with a soft outer glow: the one action a screen is built
/// around.
pub fn glowing_plate(painter: &egui::Painter, rect: Rect, colour: Color32, glow_alpha: f32) {
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let alpha = (glow_alpha * 255.0).clamp(0.0, 255.0) as u8;
    radial_pool(
        painter,
        rect.center(),
        rect.width() * 0.75,
        Color32::from_rgba_unmultiplied(colour.r(), colour.g(), colour.b(), alpha),
    );
    painter.rect(
        rect,
        4.0,
        colour,
        Stroke::new(1.0, GOLD_LIT),
        StrokeKind::Inside,
    );
}
