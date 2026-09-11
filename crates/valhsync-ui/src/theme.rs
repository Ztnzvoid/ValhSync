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
    wood(ctx, painter, rect);
    // Torchlight falls from the top of the window onto the plank.
    radial_pool(
        painter,
        egui::pos2(rect.center().x, rect.min.y),
        rect.width().max(rect.height()) * 0.85,
        Color32::from_rgba_unmultiplied(0x3A, 0x2E, 0x1E, 150),
    );
    vignette(painter, rect);
}

/// Lay the wood over `rect`. The tile repeats, so a window of any size is
/// one continuous plank.
fn wood(ctx: &egui::Context, painter: &egui::Painter, rect: Rect) {
    let texture = wood_texture(ctx);
    let mut mesh = egui::Mesh::with_texture(texture.id());
    let scale = 1.0 / WOOD_TILE;
    mesh.add_rect_with_uv(
        rect,
        Rect::from_min_size(
            egui::pos2(0.0, 0.0),
            egui::vec2(rect.width() * scale, rect.height() * scale),
        ),
        Color32::WHITE,
    );
    painter.add(egui::Shape::mesh(mesh));
}

const WOOD_TILE: f32 = 256.0;

/// Deterministic value noise: the same plank on every machine, and no
/// dependency to draw it.
fn hash_noise(x: i32, y: i32, seed: u32) -> f32 {
    #[allow(clippy::cast_sign_loss)]
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

/// Smoothed noise at a point, wrapping on the tile so the texture repeats
/// without a seam.
fn smooth_noise(x: f32, y: f32, period: i32, seed: u32) -> f32 {
    #[allow(clippy::cast_possible_truncation)]
    let (xi, yi) = (x.floor() as i32, y.floor() as i32);
    let (xf, yf) = (x - x.floor(), y - y.floor());
    // Smoothstep, so the bands curve instead of creasing.
    let (u, v) = (xf * xf * (3.0 - 2.0 * xf), yf * yf * (3.0 - 2.0 * yf));
    let at = |dx: i32, dy: i32| {
        hash_noise(
            (xi + dx).rem_euclid(period),
            (yi + dy).rem_euclid(period),
            seed,
        )
    };
    let top = at(0, 0) * (1.0 - u) + at(1, 0) * u;
    let bottom = at(0, 1) * (1.0 - u) + at(1, 1) * u;
    top * (1.0 - v) + bottom * v
}

/// A plank: long grain along x, knots and darker rings from layered noise.
/// Kept very low in contrast; it is a ground for text, not a wallpaper.
#[allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]
fn wood_texture(ctx: &egui::Context) -> egui::TextureHandle {
    let id = egui::Id::new("valhsync-wood");
    if let Some(handle) = ctx.data(|d| d.get_temp::<egui::TextureHandle>(id)) {
        return handle;
    }
    let side = WOOD_TILE as usize;
    let mut pixels = Vec::with_capacity(side * side);
    for y in 0..side {
        for x in 0..side {
            let (fx, fy) = (x as f32, y as f32);
            // Grain runs along x: stretch the noise horizontally.
            let warp = smooth_noise(fx / 48.0, fy / 12.0, 8, 1) * 2.2
                + smooth_noise(fx / 16.0, fy / 5.0, 24, 2) * 0.7;
            let rings = ((fy / 9.0 + warp) * std::f32::consts::TAU).sin() * 0.5 + 0.5;
            let fibre = smooth_noise(fx / 2.0, fy / 1.2, 128, 3);
            let knots = smooth_noise(fx / 70.0, fy / 70.0, 4, 4);

            // Two browns, far apart in the source and brought close here.
            let t = (rings * 0.55 + fibre * 0.25 + knots * 0.20).clamp(0.0, 1.0);
            let lift = 0.55 + t * 0.45;
            let r = (0x2A as f32 * lift) as u8;
            let g = (0x22 as f32 * lift) as u8;
            let b = (0x18 as f32 * lift) as u8;
            pixels.push(Color32::from_rgba_unmultiplied(r, g, b, 235));
        }
    }
    let image = egui::ColorImage {
        size: [side, side],
        pixels,
        source_size: egui::vec2(side as f32, side as f32),
    };
    let handle = ctx.load_texture(
        "valhsync-wood",
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

/// A stylised anvil, drawn from three blocks: the face with its horn, the
/// waist, and the base. Used for the repair action.
pub fn anvil(painter: &egui::Painter, rect: Rect, colour: Color32) {
    let (w, h) = (rect.width(), rect.height());
    let x = |t: f32| rect.min.x + w * t;
    let y = |t: f32| rect.min.y + h * t;

    // Face, with the horn drawn out to the left.
    painter.add(egui::Shape::convex_polygon(
        vec![
            egui::pos2(x(0.16), y(0.30)),
            egui::pos2(x(0.92), y(0.30)),
            egui::pos2(x(0.92), y(0.46)),
            egui::pos2(x(0.16), y(0.46)),
        ],
        colour,
        Stroke::NONE,
    ));
    painter.add(egui::Shape::convex_polygon(
        vec![
            egui::pos2(x(0.16), y(0.30)),
            egui::pos2(x(0.16), y(0.46)),
            egui::pos2(x(0.02), y(0.40)),
        ],
        colour,
        Stroke::NONE,
    ));
    // Waist.
    painter.add(egui::Shape::convex_polygon(
        vec![
            egui::pos2(x(0.42), y(0.46)),
            egui::pos2(x(0.70), y(0.46)),
            egui::pos2(x(0.62), y(0.74)),
            egui::pos2(x(0.50), y(0.74)),
        ],
        colour,
        Stroke::NONE,
    ));
    // Base.
    painter.add(egui::Shape::convex_polygon(
        vec![
            egui::pos2(x(0.30), y(0.86)),
            egui::pos2(x(0.82), y(0.86)),
            egui::pos2(x(0.78), y(0.74)),
            egui::pos2(x(0.34), y(0.74)),
        ],
        colour,
        Stroke::NONE,
    ));
}
