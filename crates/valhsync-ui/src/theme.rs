//! The look shared by both windows: burnt wood, polished bone, worn brass,
//! northern mist. The palette and the effects follow `docs/palette.md` and
//! the specification document's own stylesheet, so the windows and the paper
//! read as one product.

use eframe::egui::{self, Color32, FontFamily, FontId, Rect, Stroke, TextStyle, TextureWrapMode};

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
/// Text style for short carved labels: statuses, counts, tags.
pub const LABEL: &str = "label";

/// The carved style for a short label.
pub fn label_style() -> TextStyle {
    TextStyle::Name(LABEL.into())
}

/// Cinzel, SIL Open Font License 1.1. A Roman inscriptional face: the closest
/// thing to letters carved in stone, which is what the document's headings
/// imitate. The licence travels with the source in `assets/OFL-Cinzel.txt`.
const CINZEL: &[u8] = include_bytes!("../assets/Cinzel.ttf");

/// Source Serif 4, SIL Open Font License 1.1. The specification document is
/// set in an old-style serif and reads better for it; the windows use the
/// same voice instead of a system sans. Its licence ships with the source in
/// `assets/OFL-SourceSerif.txt`.
const SOURCE_SERIF: &[u8] = include_bytes!("../assets/SourceSerif.ttf");

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

    fonts.font_data.insert(
        "text".to_owned(),
        std::sync::Arc::new(egui::FontData::from_static(SOURCE_SERIF)),
    );
    if let Some(family) = fonts.families.get_mut(&FontFamily::Proportional) {
        family.insert(0, "text".to_owned());
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
    w.inactive.bg_stroke = Stroke::new(1.0, EDGE);
    w.inactive.fg_stroke = Stroke::new(1.0, BONE);
    w.inactive.corner_radius = 3.into();

    w.hovered.bg_fill = EDGE_SOFT;
    w.hovered.weak_bg_fill = EDGE_SOFT;
    w.hovered.bg_stroke = Stroke::new(1.2, GOLD);
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
    style.spacing.item_spacing = egui::vec2(10.0, 9.0);
    style.spacing.window_margin = egui::Margin::same(18);

    style
        .text_styles
        .insert(TextStyle::Heading, display_font(26.0));
    style
        .text_styles
        .insert(TextStyle::Body, FontId::new(15.5, FontFamily::Proportional));
    // Buttons and short labels are carved like the title; only running text
    // and paths stay in the interface face, which is easier to read at length.
    style
        .text_styles
        .insert(TextStyle::Button, display_font(14.5));
    style
        .text_styles
        .insert(TextStyle::Name(LABEL.into()), display_font(13.5));
    style.text_styles.insert(
        TextStyle::Small,
        FontId::new(13.0, FontFamily::Proportional),
    );
    style.text_styles.insert(
        TextStyle::Monospace,
        FontId::new(13.0, FontFamily::Monospace),
    );

    // eframe follows the desktop's light/dark setting unless told otherwise,
    // and switching theme swaps the whole style -- which threw this one away
    // and left default light widgets over our painted ground. Refuse to
    // follow, and register the same style under both themes so nothing can
    // swap it out from under us.
    ctx.set_theme(egui::ThemePreference::Dark);
    ctx.set_style_of(egui::Theme::Dark, style.clone());
    ctx.set_style_of(egui::Theme::Light, style);
}

/// A carved plate: panel ground, a thin edge, and iron at the corners.
pub fn card<R>(ui: &mut egui::Ui, add_contents: impl FnOnce(&mut egui::Ui) -> R) -> R {
    let inner = egui::Frame::new()
        .fill(PANEL)
        .stroke(Stroke::new(1.0, EDGE_SOFT))
        .inner_margin(egui::Margin::same(16))
        .corner_radius(3)
        .show(ui, add_contents);
    brackets(ui.painter(), inner.response.rect.shrink(3.0), EDGE);
    inner.inner
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
    // Torchlight from the top of the window: the effect the eye actually
    // reads. Everything else is beneath it.
    radial_pool(
        painter,
        egui::pos2(rect.center().x, rect.min.y),
        rect.width().max(rect.height()) * 0.85,
        Color32::from_rgba_unmultiplied(0x3A, 0x2E, 0x1E, 170),
    );
    radial_pool(
        painter,
        egui::pos2(rect.center().x, rect.max.y),
        rect.width() * 0.6,
        Color32::from_rgba_unmultiplied(0x22, 0x1C, 0x14, 120),
    );
    dust(ctx, painter, rect);
    vignette(painter, rect);
    watermark(ctx, rect);
}

/// How much of the mark shows through, and how heavily it is cut. Five
/// numbers in one place, because "a little luminous" is a judgement and it
/// will want adjusting.
const MARK_HALO: u8 = 9;
const MARK_BLOOM: u8 = 5;
const MARK_CORE: u8 = 13;
/// Stroke widths. The bloom is the soft spread around each stroke; the core
/// is the cut itself, and carries the weight.
const MARK_BLOOM_WIDTH: f32 = 22.0;
const MARK_CORE_WIDTH: f32 = 7.0;

/// Mannaz across the whole window, at the edge of visible.
///
/// Painted on the layer above the panel rather than beneath it: the plates
/// are opaque, and a mark showing only in the gaps between them would read as
/// four unrelated scratches instead of one shape. Above them, at this alpha,
/// it is felt rather than seen -- if it reads as a picture it is too strong.
fn watermark(ctx: &egui::Context, rect: Rect) {
    let painter = egui::Painter::new(
        ctx.clone(),
        egui::LayerId::new(egui::Order::Middle, egui::Id::new("mannaz-watermark")),
        rect,
    );
    let centre = rect.center();
    let radius = rect.width().min(rect.height()) * 0.34;
    // The halo a lamp has, so the mark sits inside the torchlight rather
    // than on top of it.
    radial_pool(
        &painter,
        centre,
        radius * 2.0,
        Color32::from_rgba_unmultiplied(0xC7, 0xA4, 0x55, MARK_HALO),
    );
    // A wide dim pass for the bloom along each stroke, a narrow bright one
    // for the carved edge: the same two-step the lamps are built from.
    rune(
        &painter,
        centre,
        radius,
        Stroke::new(
            MARK_BLOOM_WIDTH,
            Color32::from_rgba_unmultiplied(0xC7, 0xA4, 0x55, MARK_BLOOM),
        ),
    );
    rune(
        &painter,
        centre,
        radius,
        Stroke::new(
            MARK_CORE_WIDTH,
            Color32::from_rgba_unmultiplied(0xE8, 0xCD, 0x8B, MARK_CORE),
        ),
    );
}

/// A fine dust over the ground, so the surface is a material rather than a
/// flat colour. Deliberately almost invisible: it should read as depth, never
/// as a pattern.
fn dust(ctx: &egui::Context, painter: &egui::Painter, rect: Rect) {
    let texture = dust_texture(ctx);
    let mut mesh = egui::Mesh::with_texture(texture.id());
    let scale = 1.0 / DUST_TILE;
    mesh.add_rect_with_uv(
        rect,
        Rect::from_min_size(
            egui::pos2(0.0, 0.0),
            egui::vec2(rect.width() * scale, rect.height() * scale),
        ),
        Color32::from_white_alpha(12),
    );
    painter.add(egui::Shape::mesh(mesh));
}

const DUST_TILE: f32 = 96.0;

/// Deterministic value noise: the same grain on every machine, and no
/// dependency to draw it.
fn hash_noise(x: usize, y: usize, seed: u32) -> f32 {
    #[allow(clippy::cast_possible_truncation)]
    let mut h = (x as u32)
        .wrapping_mul(0x27d4_eb2d)
        .wrapping_add((y as u32).wrapping_mul(0x1656_67b1))
        .wrapping_add(seed.wrapping_mul(0x9e37_79b9));
    h ^= h >> 15;
    h = h.wrapping_mul(0x2545_f491);
    h ^= h >> 13;
    #[allow(clippy::cast_precision_loss)]
    {
        (h & 0xffff) as f32 / 65535.0
    }
}

#[allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]
fn dust_texture(ctx: &egui::Context) -> egui::TextureHandle {
    let id = egui::Id::new("valhsync-dust");
    if let Some(handle) = ctx.data(|d| d.get_temp::<egui::TextureHandle>(id)) {
        return handle;
    }
    let side = DUST_TILE as usize;
    let mut pixels = Vec::with_capacity(side * side);
    for y in 0..side {
        for x in 0..side {
            let n = hash_noise(x, y, 7);
            pixels.push(Color32::from_white_alpha((n * 42.0) as u8));
        }
    }
    let image = egui::ColorImage {
        size: [side, side],
        pixels,
        source_size: egui::vec2(side as f32, side as f32),
    };
    let handle = ctx.load_texture(
        "valhsync-dust",
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

/// A point of light: a hot core inside a halo that falls away. Used where a
/// flat dot would look like a bullet instead of a lamp.
pub fn lamp(painter: &egui::Painter, centre: egui::Pos2, radius: f32, colour: Color32) {
    radial_pool(painter, centre, radius * 4.2, colour.gamma_multiply(0.20));
    radial_pool(painter, centre, radius * 2.1, colour.gamma_multiply(0.35));
    painter.circle_filled(centre, radius, colour);
    // The filament, a touch brighter than the glass.
    painter.circle_filled(
        centre,
        radius * 0.45,
        Color32::from_rgba_unmultiplied(255, 255, 255, 90),
    );
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

/// Mannaz, the rune of man: two staves bound by a crossing. It is what the
/// launcher does, hold a group of players to one shape. Four strokes, so it
/// stays itself at sixteen pixels.
fn mannaz_strokes(centre: [f32; 2], radius: f32) -> [([f32; 2], [f32; 2]); 4] {
    let at = |x: f32, y: f32| [centre[0] + x * radius, centre[1] + y * radius];
    let (left, right) = (-0.60_f32, 0.60_f32);
    let (top, bottom, waist) = (-0.95_f32, 0.95_f32, 0.18_f32);
    [
        (at(left, top), at(left, bottom)),
        (at(right, top), at(right, bottom)),
        (at(left, top), at(right, waist)),
        (at(right, top), at(left, waist)),
    ]
}

/// Draw Mannaz centred on `centre`.
pub fn rune(painter: &egui::Painter, centre: egui::Pos2, radius: f32, stroke: Stroke) {
    for (a, b) in mannaz_strokes([centre.x, centre.y], radius) {
        painter.line_segment([egui::pos2(a[0], a[1]), egui::pos2(b[0], b[1])], stroke);
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

/// 64x64 window icon: Mannaz in brass on burnt wood.
#[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]
pub fn icon() -> egui::IconData {
    const S: i32 = 64;
    let centre = (S as f32 - 1.0) / 2.0;
    let strokes = mannaz_strokes([centre, centre], S as f32 * 0.34);

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
            let ink = strokes
                .iter()
                .map(|(a, b)| distance_to_segment(p, *a, *b))
                .fold(f32::MAX, f32::min);

            let pixel = if ink < 2.0 {
                [0xE8, 0xCD, 0x8B, 255]
            } else if ink < 2.8 {
                [0xC7, 0xA4, 0x55, 255]
            } else if r > S as f32 * 0.49 {
                [0, 0, 0, 0]
            } else if r > S as f32 * 0.45 {
                [0x4A, 0x3C, 0x27, 255]
            } else {
                [0x17, 0x14, 0x0F, 255]
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

/// A stylised anvil: a heavy face drawn out into a horn, a short waist and a
/// wide foot. Proportions are exaggerated so it still reads at 20 pixels.
pub fn anvil(painter: &egui::Painter, rect: Rect, colour: Color32) {
    let (w, h) = (rect.width(), rect.height());
    let x = |t: f32| rect.min.x + w * t;
    let y = |t: f32| rect.min.y + h * t;
    let block = |points: Vec<egui::Pos2>| {
        painter.add(egui::Shape::convex_polygon(points, colour, Stroke::NONE));
    };

    // Face: a thick slab, tapering into the horn on the left.
    block(vec![
        egui::pos2(x(0.22), y(0.18)),
        egui::pos2(x(1.00), y(0.18)),
        egui::pos2(x(1.00), y(0.42)),
        egui::pos2(x(0.22), y(0.42)),
    ]);
    block(vec![
        egui::pos2(x(0.22), y(0.20)),
        egui::pos2(x(0.22), y(0.42)),
        egui::pos2(x(0.00), y(0.34)),
    ]);
    // Waist.
    block(vec![
        egui::pos2(x(0.46), y(0.42)),
        egui::pos2(x(0.76), y(0.42)),
        egui::pos2(x(0.68), y(0.68)),
        egui::pos2(x(0.54), y(0.68)),
    ]);
    // Foot.
    block(vec![
        egui::pos2(x(0.28), y(1.00)),
        egui::pos2(x(0.94), y(1.00)),
        egui::pos2(x(0.86), y(0.68)),
        egui::pos2(x(0.36), y(0.68)),
    ]);
}

/// A waste bin: lid, body and two slots. Drawn, because the glyphs for this
/// are not in the bundled font.
pub fn trash(painter: &egui::Painter, rect: Rect, colour: Color32) {
    let (w, h) = (rect.width(), rect.height());
    let x = |t: f32| rect.min.x + w * t;
    let y = |t: f32| rect.min.y + h * t;
    let stroke = Stroke::new(1.3, colour);

    // Lid, with the little handle above it.
    painter.line_segment(
        [egui::pos2(x(0.05), y(0.22)), egui::pos2(x(0.95), y(0.22))],
        stroke,
    );
    painter.line_segment(
        [egui::pos2(x(0.36), y(0.22)), egui::pos2(x(0.40), y(0.08))],
        stroke,
    );
    painter.line_segment(
        [egui::pos2(x(0.64), y(0.22)), egui::pos2(x(0.60), y(0.08))],
        stroke,
    );
    painter.line_segment(
        [egui::pos2(x(0.40), y(0.08)), egui::pos2(x(0.60), y(0.08))],
        stroke,
    );

    // Body, tapering slightly towards the bottom.
    painter.line_segment(
        [egui::pos2(x(0.16), y(0.30)), egui::pos2(x(0.24), y(0.94))],
        stroke,
    );
    painter.line_segment(
        [egui::pos2(x(0.84), y(0.30)), egui::pos2(x(0.76), y(0.94))],
        stroke,
    );
    painter.line_segment(
        [egui::pos2(x(0.24), y(0.94)), egui::pos2(x(0.76), y(0.94))],
        stroke,
    );

    // Slots.
    for t in [0.40_f32, 0.60] {
        painter.line_segment(
            [egui::pos2(x(t), y(0.42)), egui::pos2(x(t), y(0.82))],
            Stroke::new(1.1, colour),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The window used to vanish the moment a server was added -- but only on
    /// a desktop set to the light theme.
    ///
    /// eframe follows that setting, and switching theme swaps the whole style,
    /// including `text_styles`. Our carved label lives there under a name, and
    /// egui panics outright when a named style cannot be resolved. Everything
    /// that used it was on a screen you only reach with a server, so the
    /// window looked merely wrong until the first one was added, and then the
    /// process aborted.
    #[test]
    fn the_carved_label_survives_a_theme_switch() {
        let ctx = egui::Context::default();
        apply(&ctx);
        for theme in [egui::Theme::Dark, egui::Theme::Light] {
            assert!(
                ctx.style_of(theme).text_styles.contains_key(&label_style()),
                "the {theme:?} style lost the carved label"
            );
        }
    }
}
