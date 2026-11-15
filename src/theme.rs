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
    pub dark: bool,
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
    dark: false,
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
    dark: true,
};

pub const RADIUS_CARD: u8 = 12;
pub const RADIUS_CONTROL: u8 = 8;

pub fn palette(ctx: &egui::Context) -> Palette {
    if ctx.theme() == egui::Theme::Dark { DARK } else { LIGHT }
}

/// Applies the app look to both the light and dark styles egui keeps.
pub fn apply(ctx: &egui::Context) {
    for (theme, p) in [(egui::Theme::Light, LIGHT), (egui::Theme::Dark, DARK)] {
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
            v.window_fill = p.card;
            v.window_stroke = Stroke::new(1.0, p.stroke);
            v.window_corner_radius = CornerRadius::same(RADIUS_CARD);
            v.menu_corner_radius = CornerRadius::same(10);
            v.extreme_bg_color = p.card;
            v.faint_bg_color = p.fill_secondary;
            v.text_edit_bg_color = Some(p.card);
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
            w.noninteractive.bg_fill = p.card;
            w.noninteractive.weak_bg_fill = p.card;
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
