use egui::{Color32, CornerRadius, FontFamily, FontId, Margin, Stroke, TextStyle, Vec2};

#[derive(Clone, Copy, Debug)]
pub struct Palette {
    pub window: Color32,
    pub sidebar: Color32,
    pub card: Color32,
    pub card_hover: Color32,
    pub stroke: Color32,
    pub separator: Color32,
    pub text: Color32,
    pub text_secondary: Color32,
    pub text_tertiary: Color32,
    pub accent: Color32,
    pub accent_text: Color32,
    pub green: Color32,
    pub red: Color32,
    pub orange: Color32,
    pub fill_secondary: Color32,
    pub fill_tertiary: Color32,
    pub toggle_off: Color32,
    pub selection: Color32,
    /// Windows, sheets, menus and tooltips: whatever floats over the page.
    pub float: Color32,
    /// Text fields and other wells.
    pub field: Color32,
    /// The sliding knob of a segmented control.
    pub knob: Color32,
    pub dark: bool,
    /// Liquid Glass: `window`, `sidebar` and `card` are the panel marker and `float` the
    /// floating one, which [`crate::liquid_glass::glassify`] swaps for real glass.
    pub liquid: bool,
}

pub const LIGHT: Palette = Palette {
    window: Color32::from_rgb(0xF5, 0xF5, 0xF7),
    sidebar: Color32::from_rgb(0xEB, 0xEB, 0xEE),
    card: Color32::WHITE,
    card_hover: Color32::from_rgb(0xFA, 0xFA, 0xFC),
    stroke: Color32::from_black_alpha(14),
    separator: Color32::from_black_alpha(18),
    text: Color32::from_rgb(0x1D, 0x1D, 0x1F),
    text_secondary: Color32::from_rgb(0x6E, 0x6E, 0x73),
    text_tertiary: Color32::from_rgb(0xAE, 0xAE, 0xB2),
    accent: Color32::from_rgb(0x00, 0x7A, 0xFF),
    accent_text: Color32::WHITE,
    green: Color32::from_rgb(0x34, 0xC7, 0x59),
    red: Color32::from_rgb(0xFF, 0x3B, 0x30),
    orange: Color32::from_rgb(0xFF, 0x95, 0x00),
    fill_secondary: Color32::from_black_alpha(12),
    fill_tertiary: Color32::from_black_alpha(22),
    toggle_off: Color32::from_rgb(0xE9, 0xE9, 0xEB),
    selection: Color32::from_rgba_premultiplied(0, 12, 25, 40),
    float: Color32::WHITE,
    field: Color32::WHITE,
    knob: Color32::WHITE,
    dark: false,
    liquid: false,
};

pub const DARK: Palette = Palette {
    window: Color32::from_rgb(0x1C, 0x1C, 0x1E),
    sidebar: Color32::from_rgb(0x16, 0x16, 0x18),
    card: Color32::from_rgb(0x2C, 0x2C, 0x2E),
    card_hover: Color32::from_rgb(0x33, 0x33, 0x36),
    stroke: Color32::from_rgba_premultiplied(18, 18, 18, 18),
    separator: Color32::from_rgba_premultiplied(22, 22, 22, 22),
    text: Color32::from_rgb(0xF5, 0xF5, 0xF7),
    text_secondary: Color32::from_rgb(0x98, 0x98, 0x9D),
    text_tertiary: Color32::from_rgb(0x63, 0x63, 0x66),
    accent: Color32::from_rgb(0x0A, 0x84, 0xFF),
    accent_text: Color32::WHITE,
    green: Color32::from_rgb(0x30, 0xD1, 0x58),
    red: Color32::from_rgb(0xFF, 0x45, 0x3A),
    orange: Color32::from_rgb(0xFF, 0x9F, 0x0A),
    fill_secondary: Color32::from_rgba_premultiplied(20, 20, 20, 20),
    fill_tertiary: Color32::from_rgba_premultiplied(36, 36, 36, 36),
    toggle_off: Color32::from_rgb(0x39, 0x39, 0x3D),
    selection: Color32::from_rgba_premultiplied(2, 31, 60, 60),
    float: Color32::from_rgb(0x2C, 0x2C, 0x2E),
    field: Color32::from_rgb(0x2C, 0x2C, 0x2E),
    knob: Color32::from_rgb(0x4A, 0x4A, 0x4C),
    dark: true,
    liquid: false,
};

/// A translucent colour from straight (un-premultiplied) RGBA, usable in a `const`.
const fn glass(r: u8, g: u8, b: u8, a: u8) -> Color32 {
    Color32::from_rgba_premultiplied(
        ((r as u16 * a as u16 + 127) / 255) as u8,
        ((g as u16 * a as u16 + 127) / 255) as u8,
        ((b as u16 * a as u16 + 127) / 255) as u8,
        a,
    )
}

/// What marks a glass panel and a floating glass surface, dark then light. Chosen so
/// that, painted as they are (on a GPU that can't draw the glass), they're plain
/// translucent panels.
const LIQUID_PANEL: [Color32; 2] = [glass(28, 30, 38, 171), glass(246, 247, 250, 171)];
const LIQUID_FLOAT: [Color32; 2] = [glass(44, 46, 58, 182), glass(225, 230, 245, 182)];

/// Liquid Glass, dark. Fields and controls are grey at a low opacity, which reads on any
/// glass.
pub const LIQUID_DARK: Palette = Palette {
    window: LIQUID_PANEL[0],
    sidebar: LIQUID_PANEL[0],
    card: LIQUID_PANEL[0],
    card_hover: glass(255, 255, 255, 16),
    stroke: glass(255, 255, 255, 26),
    separator: glass(255, 255, 255, 28),
    text: Color32::from_rgb(242, 242, 245),
    text_secondary: Color32::from_rgb(190, 193, 200),
    text_tertiary: Color32::from_rgb(140, 143, 152),
    accent: Color32::from_rgb(64, 140, 255),
    accent_text: Color32::WHITE,
    green: Color32::from_rgb(0x30, 0xD1, 0x58),
    red: Color32::from_rgb(0xFF, 0x45, 0x3A),
    orange: Color32::from_rgb(0xFF, 0x9F, 0x0A),
    fill_secondary: glass(118, 118, 128, 61),
    fill_tertiary: glass(118, 118, 128, 92),
    toggle_off: glass(118, 118, 128, 110),
    selection: Color32::from_rgba_premultiplied(16, 35, 64, 64),
    float: LIQUID_FLOAT[0],
    field: glass(118, 118, 128, 61),
    knob: glass(170, 172, 184, 120),
    dark: true,
    liquid: true,
};

/// Liquid Glass, light: white glass, dark ink.
pub const LIQUID_LIGHT: Palette = Palette {
    window: LIQUID_PANEL[1],
    sidebar: LIQUID_PANEL[1],
    card: LIQUID_PANEL[1],
    card_hover: Color32::from_black_alpha(12),
    stroke: Color32::from_black_alpha(18),
    separator: Color32::from_black_alpha(24),
    text: Color32::from_rgb(28, 28, 31),
    text_secondary: Color32::from_rgb(88, 90, 98),
    text_tertiary: Color32::from_rgb(128, 130, 140),
    accent: Color32::from_rgb(10, 110, 235),
    accent_text: Color32::WHITE,
    green: Color32::from_rgb(0x28, 0xB4, 0x4C),
    red: Color32::from_rgb(0xF0, 0x38, 0x2E),
    orange: Color32::from_rgb(0xF0, 0x8A, 0x00),
    fill_secondary: glass(118, 118, 128, 36),
    fill_tertiary: glass(118, 118, 128, 58),
    toggle_off: glass(118, 118, 128, 80),
    selection: Color32::from_rgba_premultiplied(2, 22, 47, 48),
    float: LIQUID_FLOAT[1],
    field: glass(118, 118, 128, 36),
    knob: glass(255, 255, 255, 215),
    dark: false,
    liquid: true,
};

pub const RADIUS_CARD: u8 = 12;
pub const RADIUS_CONTROL: u8 = 8;

/// How round the two glass panels are, and the gap around them, under Liquid Glass.
pub const RADIUS_PANEL: u8 = 20;
pub const PANEL_GAP: i8 = 10;

/// Whether the Liquid Glass theme is on.
static LIQUID: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub fn is_liquid() -> bool {
    LIQUID.load(std::sync::atomic::Ordering::Relaxed)
}

/// Turns Liquid Glass on or off and restyles the app to match.
pub fn set_liquid(ctx: &egui::Context, on: bool) {
    LIQUID.store(on, std::sync::atomic::Ordering::Relaxed);
    apply(ctx);
}

fn palette_for(dark: bool) -> Palette {
    match (is_liquid(), dark) {
        (false, false) => LIGHT,
        (false, true) => DARK,
        (true, false) => LIQUID_LIGHT,
        (true, true) => LIQUID_DARK,
    }
}

pub fn palette(ctx: &egui::Context) -> Palette {
    palette_for(ctx.theme() == egui::Theme::Dark)
}

/// Under Liquid Glass, the colours that mark its panels and floating surfaces this frame.
pub fn liquid_markers(ctx: &egui::Context) -> Option<crate::liquid_glass::Markers> {
    is_liquid().then(|| liquid_markers_for(ctx.theme() == egui::Theme::Light))
}

/// Liquid Glass's markers, light or dark, whatever theme is active.
pub(crate) fn liquid_markers_for(light: bool) -> crate::liquid_glass::Markers {
    let i = light as usize;
    crate::liquid_glass::Markers { panel: LIQUID_PANEL[i], float: LIQUID_FLOAT[i], light }
}

/// The colour the window is cleared to: under Liquid Glass, what the flowing colours
/// blend into.
pub fn base_color(ctx: &egui::Context) -> Color32 {
    let p = palette(ctx);
    match (p.liquid, p.dark) {
        (false, _) => p.window,
        (true, true) => Color32::from_rgb(12, 13, 18),
        (true, false) => Color32::from_rgb(226, 231, 242),
    }
}

/// Paints what Liquid Glass sits on: colours flowing slowly behind every panel. Call
/// first in the frame, before any panel is drawn. Does nothing under the other themes.
pub fn paint_backdrop(ctx: &egui::Context) {
    let p = palette(ctx);
    if !p.liquid {
        return;
    }
    let rect = ctx.content_rect();
    let painter = ctx.layer_painter(egui::LayerId::background());
    let base = base_color(ctx);
    painter.rect_filled(rect, 0.0, base);

    let t = ctx.input(|i| i.time) as f32;
    let fields = flow_fields(t, rect, p.dark);
    let base = [base.r() as f32, base.g() as f32, base.b() as f32];
    const COLS: u32 = 36;
    const ROWS: u32 = 24;
    let mut mesh = egui::Mesh::default();
    for j in 0..=ROWS {
        for i in 0..=COLS {
            let pos = egui::pos2(
                rect.left() + rect.width() * i as f32 / COLS as f32,
                rect.top() + rect.height() * j as f32 / ROWS as f32,
            );
            mesh.colored_vertex(pos, flow_color(pos, base, &fields));
        }
    }
    for j in 0..ROWS {
        for i in 0..COLS {
            let a = j * (COLS + 1) + i;
            mesh.add_triangle(a, a + 1, a + COLS + 1);
            mesh.add_triangle(a + 1, a + COLS + 2, a + COLS + 1);
        }
    }
    painter.add(egui::Shape::mesh(mesh));
    // The colours only need to move while someone is looking.
    if ctx.input(|i| i.focused) {
        ctx.request_repaint_after(std::time::Duration::from_millis(50));
    }
}

/// One of the flowing colour fields: where its centre is, how far it reaches, its colour
/// and how strongly it's laid.
struct Field {
    center: egui::Pos2,
    radius: f32,
    color: [f32; 3],
    strength: f32,
}

/// The fields at time `t` over `rect`.
fn flow_fields(t: f32, rect: egui::Rect, dark: bool) -> Vec<Field> {
    // Electric blue, violet, magenta, amber, teal, sky.
    const COLORS: [[f32; 3]; 6] = [
        [40.0, 105.0, 255.0],
        [138.0, 70.0, 255.0],
        [255.0, 60.0, 160.0],
        [255.0, 150.0, 50.0],
        [0.0, 195.0, 170.0],
        [70.0, 190.0, 255.0],
    ];
    let span = rect.width().max(rect.height()).max(1.0);
    COLORS
        .iter()
        .enumerate()
        .map(|(i, &c)| {
            let i = i as u32;
            let speed = 0.035 + 0.04 * hash01(i, 31);
            let phase = hash01(i, 32) * std::f32::consts::TAU;
            let (ax, ay) = (0.12 + 0.76 * hash01(i, 33), 0.12 + 0.76 * hash01(i, 34));
            let orbit = 0.16 + 0.1 * hash01(i, 35);
            let center = egui::pos2(
                rect.left() + rect.width() * (ax + orbit * (t * speed + phase).cos()),
                rect.top() + rect.height() * (ay + orbit * 0.8 * (t * speed * 1.3 + phase).sin()),
            );
            // Light: the same hues, washed toward white.
            let color = if dark { c } else { c.map(|v| v + (255.0 - v) * 0.35) };
            Field { center, radius: span * (0.26 + 0.1 * hash01(i, 36)), color, strength: if dark { 0.85 } else { 0.75 } }
        })
        .collect()
}

/// The flowing colour at `pos`: each field laid over `base` by how near its centre `pos`
/// is (a Gaussian falloff).
fn flow_color(pos: egui::Pos2, base: [f32; 3], fields: &[Field]) -> Color32 {
    let mut c = base;
    for f in fields {
        let d2 = (pos - f.center).length_sq() / (f.radius * f.radius);
        let w = (-d2 * 1.6).exp() * f.strength;
        for (ck, fk) in c.iter_mut().zip(f.color) {
            *ck += (fk - *ck) * w;
        }
    }
    Color32::from_rgb(c[0].round() as u8, c[1].round() as u8, c[2].round() as u8)
}

/// Cheap integer hash to `[0, 1)`, so each field keeps the same path from run to run.
fn hash01(i: u32, salt: u32) -> f32 {
    let mut h = i
        .wrapping_mul(0x2c1b_3c6d)
        .wrapping_add(salt.wrapping_mul(0x2745_7d83))
        .wrapping_add(0x9e37_79b9);
    h ^= h >> 15;
    h = h.wrapping_mul(0x85eb_ca6b);
    h ^= h >> 13;
    h = h.wrapping_mul(0xc2b2_ae35);
    h ^= h >> 16;
    (h as f32) / (u32::MAX as f32)
}

/// Applies the app look to both the light and dark styles egui keeps.
pub fn apply(ctx: &egui::Context) {
    for (theme, p) in [(egui::Theme::Light, palette_for(false)), (egui::Theme::Dark, palette_for(true))] {
        ctx.style_mut_of(theme, |style| {
            style.text_styles = [
                (TextStyle::Small, FontId::new(12.0, FontFamily::Proportional)),
                (TextStyle::Body, FontId::new(14.0, FontFamily::Proportional)),
                (TextStyle::Button, FontId::new(14.0, FontFamily::Proportional)),
                (TextStyle::Heading, FontId::new(26.0, FontFamily::Proportional)),
                (TextStyle::Monospace, FontId::new(13.0, FontFamily::Monospace)),
            ]
            .into();

            let sp = &mut style.spacing;
            sp.item_spacing = Vec2::new(8.0, 8.0);
            sp.button_padding = Vec2::new(14.0, 7.0);
            sp.interact_size = Vec2::new(40.0, 30.0);
            sp.window_margin = Margin::same(16);
            sp.menu_margin = Margin::same(8);
            sp.indent = 16.0;
            sp.scroll.bar_width = 6.0;
            sp.scroll.floating = true;
            sp.scroll.bar_inner_margin = 4.0;

            // Text in lists is a label, not a document: selecting it fights with dragging
            // rows and makes the list scroll to follow the selection.
            style.interaction.selectable_labels = false;

            let v = &mut style.visuals;
            v.dark_mode = p.dark;
            v.override_text_color = Some(p.text);
            v.panel_fill = p.window;
            v.window_fill = p.float;
            // Glass has its own bright rim.
            v.window_stroke = if p.liquid { Stroke::NONE } else { Stroke::new(1.0, p.stroke) };
            v.window_corner_radius = CornerRadius::same(RADIUS_CARD);
            v.menu_corner_radius = CornerRadius::same(10);
            v.extreme_bg_color = p.field;
            v.faint_bg_color = p.fill_secondary;
            v.text_edit_bg_color = Some(p.field);
            v.hyperlink_color = p.accent;
            v.selection.bg_fill = p.selection;
            v.selection.stroke = Stroke::new(1.0, p.accent);
            v.window_shadow = egui::epaint::Shadow {
                offset: [0, 8],
                blur: 32,
                spread: 0,
                color: Color32::from_black_alpha(if p.dark { 120 } else { 40 }),
            };
            v.popup_shadow = egui::epaint::Shadow {
                offset: [0, 6],
                blur: 20,
                spread: 0,
                color: Color32::from_black_alpha(if p.dark { 110 } else { 30 }),
            };
            v.striped = false;
            v.slider_trailing_fill = true;

            let w = &mut v.widgets;
            let r = CornerRadius::same(RADIUS_CONTROL);
            w.noninteractive.bg_fill = p.field;
            w.noninteractive.weak_bg_fill = p.field;
            w.noninteractive.bg_stroke = Stroke::new(1.0, p.stroke);
            w.noninteractive.fg_stroke = Stroke::new(1.0, p.text_secondary);
            w.noninteractive.corner_radius = r;

            w.inactive.bg_fill = p.fill_secondary;
            w.inactive.weak_bg_fill = p.fill_secondary;
            w.inactive.bg_stroke = Stroke::NONE;
            w.inactive.fg_stroke = Stroke::new(1.0, p.text);
            w.inactive.corner_radius = r;

            w.hovered.bg_fill = p.fill_tertiary;
            w.hovered.weak_bg_fill = p.fill_tertiary;
            w.hovered.bg_stroke = Stroke::NONE;
            w.hovered.fg_stroke = Stroke::new(1.0, p.text);
            w.hovered.corner_radius = r;
            w.hovered.expansion = 0.0;

            w.active.bg_fill = p.fill_tertiary;
            w.active.weak_bg_fill = p.fill_tertiary;
            w.active.bg_stroke = Stroke::NONE;
            w.active.fg_stroke = Stroke::new(1.0, p.text);
            w.active.corner_radius = r;
            w.active.expansion = 0.0;

            w.open.bg_fill = p.fill_tertiary;
            w.open.weak_bg_fill = p.fill_tertiary;
            w.open.bg_stroke = Stroke::NONE;
            w.open.fg_stroke = Stroke::new(1.0, p.text);
            w.open.corner_radius = r;
        });
    }
}

pub const FAMILY_SEMIBOLD: &str = "semibold";
pub const FAMILY_BOLD: &str = "bold";
pub const FAMILY_ICONS: &str = "icons";

pub fn semibold() -> FontFamily {
    FontFamily::Name(FAMILY_SEMIBOLD.into())
}
pub fn bold() -> FontFamily {
    FontFamily::Name(FAMILY_BOLD.into())
}
pub fn icons() -> FontFamily {
    FontFamily::Name(FAMILY_ICONS.into())
}

/// The same icons as symbols from the fonts bundled with egui, for systems without Segoe
/// Fluent Icons.
#[cfg(not(windows))]
pub mod icon {
    pub const LIBRARY: &str = "\u{1F4DA}";
    pub const LIST: &str = "\u{2630}";
    pub const SETTINGS: &str = "\u{2699}";
    pub const PLAY: &str = "\u{25B6}";
    pub const ADD: &str = "\u{2795}";
    pub const SEARCH: &str = "\u{1F50D}";
    pub const MORE: &str = "\u{2026}";
    pub const FOLDER: &str = "\u{1F4C1}";
    pub const CHEVRON_UP: &str = "\u{23F6}";
    pub const CHEVRON_DOWN: &str = "\u{23F7}";
    pub const WARNING: &str = "\u{26A0}";
    pub const CHECK: &str = "\u{2714}";
    pub const REFRESH: &str = "\u{1F504}";
    pub const GAME: &str = "\u{1F3AE}";
    pub const CLOSE: &str = "\u{2716}";
    pub const LOCK: &str = "\u{1F512}";
    pub const DOWNLOAD: &str = "\u{1F4E5}";
    pub const OPEN: &str = "\u{1F517}";
}

/// Segoe Fluent Icons glyphs (falls back to nothing if the font is missing).
#[cfg(windows)]
pub mod icon {
    pub const LIBRARY: &str = "\u{E8F1}";
    pub const LIST: &str = "\u{E8FD}";
    pub const SETTINGS: &str = "\u{E713}";
    pub const PLAY: &str = "\u{E768}";
    pub const ADD: &str = "\u{E710}";
    pub const SEARCH: &str = "\u{E721}";
    pub const MORE: &str = "\u{E712}";
    pub const FOLDER: &str = "\u{E8B7}";
    pub const CHEVRON_UP: &str = "\u{E70E}";
    pub const CHEVRON_DOWN: &str = "\u{E70D}";
    pub const WARNING: &str = "\u{E7BA}";
    pub const CHECK: &str = "\u{E73E}";
    pub const REFRESH: &str = "\u{E72C}";
    pub const GAME: &str = "\u{E7FC}";
    pub const CLOSE: &str = "\u{E711}";
    pub const LOCK: &str = "\u{E72E}";
    pub const DOWNLOAD: &str = "\u{E896}";
    pub const OPEN: &str = "\u{E8A7}";
}

/// The Segoe fonts only exist on Windows. Elsewhere the app's own font families point at
/// the fonts bundled with egui, so every weight and icon still has something to draw with.
#[cfg(not(windows))]
pub fn install_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    let bundled = fonts.families.get(&FontFamily::Proportional).cloned().unwrap_or_default();
    for family in [semibold(), bold(), icons()] {
        fonts.families.insert(family, bundled.clone());
    }
    ctx.set_fonts(fonts);
}

/// Uses the Windows UI fonts so text and icons look native instead of egui's bundled face.
#[cfg(windows)]
pub fn install_fonts(ctx: &egui::Context) {
    use egui::epaint::text::{FontData, FontInsert, FontPriority, InsertFontFamily};
    let add = |ctx: &egui::Context, path: &str, name: &str, family: FontFamily, fallback_prop: bool| {
        let Ok(bytes) = std::fs::read(path) else { return };
        let mut families = vec![InsertFontFamily { family, priority: FontPriority::Highest }];
        if fallback_prop {
            families.push(InsertFontFamily {
                family: FontFamily::Proportional,
                priority: FontPriority::Lowest,
            });
        }
        ctx.add_font(FontInsert::new(name, FontData::from_owned(bytes), families));
    };
    add(ctx, r"C:\Windows\Fonts\segoeui.ttf", "segoe-ui", FontFamily::Proportional, false);
    add(ctx, r"C:\Windows\Fonts\seguisb.ttf", FAMILY_SEMIBOLD, semibold(), false);
    add(ctx, r"C:\Windows\Fonts\segoeuib.ttf", FAMILY_BOLD, bold(), false);
    add(ctx, r"C:\Windows\Fonts\SegoeIcons.ttf", FAMILY_ICONS, icons(), true);
}
