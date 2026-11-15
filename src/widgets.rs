use crate::theme::{self, Palette};
use egui::{
    Align2, Color32, CornerRadius, FontId, Frame, Id, Margin, Pos2, Rect, Response, RichText, Sense,
    Stroke, Ui, Vec2, pos2, vec2,
};

pub fn heading(text: &str) -> RichText {
    RichText::new(text).font(FontId::new(26.0, theme::bold()))
}

pub fn title(text: &str) -> RichText {
    RichText::new(text).font(FontId::new(15.0, theme::semibold()))
}

pub fn section(text: &str, p: &Palette) -> RichText {
    RichText::new(text)
        .font(FontId::new(12.5, theme::semibold()))
        .color(p.text_secondary)
}

pub fn secondary(text: &str, p: &Palette) -> RichText {
    RichText::new(text).size(12.5).color(p.text_secondary)
}

pub fn icon(glyph: &str, size: f32, color: Color32) -> RichText {
    RichText::new(glyph).font(FontId::new(size, theme::icons())).color(color)
}

// ---------------------------------------------------------------------------

/// iOS-style animated switch.
pub fn toggle(ui: &mut Ui, on: &mut bool) -> Response {
    let p = theme::palette(ui.ctx());
    let size = vec2(40.0, 24.0);
    let (rect, mut resp) = ui.allocate_exact_size(size, Sense::click());
    if resp.clicked() {
        *on = !*on;
        resp.mark_changed();
    }
    let t = ui.ctx().animate_bool_with_time(resp.id, *on, 0.16);
    let bg = p.toggle_off.lerp_to_gamma(p.green, t);
    let radius = rect.height() / 2.0;
    let painter = ui.painter();
    painter.rect_filled(rect, radius, bg);
    let knob_r = radius - 2.0;
    let x = egui::lerp(rect.left() + radius..=rect.right() - radius, t);
    let center = pos2(x, rect.center().y);
    painter.circle_filled(center + vec2(0.0, 1.0), knob_r, Color32::from_black_alpha(40));
    painter.circle_filled(center, knob_r, Color32::WHITE);
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// Small locked indicator shown in place of a toggle for things that can't be changed.
pub fn locked_toggle(ui: &mut Ui) -> Response {
    let p = theme::palette(ui.ctx());
    let (rect, resp) = ui.allocate_exact_size(vec2(40.0, 24.0), Sense::hover());
    let radius = rect.height() / 2.0;
    ui.painter().rect_filled(rect, radius, p.green.gamma_multiply(0.45));
    ui.painter().text(
        rect.center(),
        Align2::CENTER_CENTER,
        theme::icon::LOCK,
        FontId::new(11.0, theme::icons()),
        Color32::WHITE,
    );
    resp
}

// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum PillStyle {
    Primary,
    Secondary,
    Danger,
    Ghost,
}

/// Rounded button with optional leading icon.
pub fn pill(ui: &mut Ui, icon_glyph: Option<&str>, label: &str, style: PillStyle) -> Response {
    let p = theme::palette(ui.ctx());
    let enabled = ui.is_enabled();
    let (fill, text_color) = match style {
        PillStyle::Primary => (p.accent, p.accent_text),
        PillStyle::Secondary => (p.fill_secondary, p.text),
        PillStyle::Danger => (p.red, Color32::WHITE),
        PillStyle::Ghost => (Color32::TRANSPARENT, p.accent),
    };
    let font = FontId::new(13.5, theme::semibold());
    let galley = ui.painter().layout_no_wrap(label.to_owned(), font, text_color);
    let icon_w = if icon_glyph.is_some() { 20.0 } else { 0.0 };
    let pad = vec2(14.0, 0.0);
    let size = vec2(galley.size().x + icon_w + pad.x * 2.0, 32.0);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    let hovered = resp.hovered();
    let pressed = resp.is_pointer_button_down_on();
    let mut fill = fill;
    if !enabled {
        fill = fill.gamma_multiply(0.5);
    } else if pressed {
        fill = fill.gamma_multiply(0.85).blend(Color32::from_black_alpha(20));
    } else if hovered {
        fill = if style == PillStyle::Ghost {
            p.fill_secondary
        } else if p.dark {
            fill.blend(Color32::from_white_alpha(18))
        } else {
            fill.blend(Color32::from_black_alpha(14))
        };
    }
    let painter = ui.painter();
    painter.rect_filled(rect, rect.height() / 2.0, fill);
    let mut x = rect.left() + pad.x;
    if let Some(g) = icon_glyph {
        painter.text(
            pos2(x + 6.0, rect.center().y),
            Align2::CENTER_CENTER,
            g,
            FontId::new(13.0, theme::icons()),
            text_color,
        );
        x += icon_w;
    }
    painter.galley(
        pos2(x, rect.center().y - galley.size().y / 2.0),
        galley,
        text_color,
    );
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// Round icon-only button (like the "…" menu trigger).
pub fn icon_button(ui: &mut Ui, glyph: &str, tooltip: &str) -> Response {
    let p = theme::palette(ui.ctx());
    let (rect, resp) = ui.allocate_exact_size(vec2(28.0, 28.0), Sense::click());
    if resp.hovered() || resp.is_pointer_button_down_on() {
        ui.painter().rect_filled(rect, 14.0, p.fill_secondary);
    }
    ui.painter().text(
        rect.center(),
        Align2::CENTER_CENTER,
        glyph,
        FontId::new(13.0, theme::icons()),
        p.text_secondary,
    );
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(tooltip)
}

// ---------------------------------------------------------------------------

pub fn card_frame(p: &Palette) -> Frame {
    Frame::new()
        .fill(p.card)
        .stroke(Stroke::new(1.0, p.stroke))
        .corner_radius(CornerRadius::same(theme::RADIUS_CARD))
        .inner_margin(Margin::symmetric(16, 12))
}

/// White rounded container, the building block of every page.
pub fn card<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
    let p = theme::palette(ui.ctx());
    card_frame(&p).show(ui, add).inner
}

/// The hovered row inside a grouped card, painted by the group once its full extent is known
/// so the highlight can follow the card's rounded corners.
#[derive(Clone, Copy)]
struct HoverRow {
    bg: egui::layers::ShapeIdx,
    rect: Rect,
}

fn group_key(ui: &Ui) -> Id {
    ui.id().with("grouped")
}

/// Card whose rows are separated by hairlines, like a grouped list in System Settings.
pub fn grouped<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
    let p = theme::palette(ui.ctx());
    Frame::new()
        .fill(p.card)
        .stroke(Stroke::new(1.0, p.stroke))
        .corner_radius(CornerRadius::same(theme::RADIUS_CARD))
        .inner_margin(Margin::ZERO)
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            let key = group_key(ui);
            ui.data_mut(|d| d.insert_temp(key, true));
            let result = add(ui);
            let hovered: Option<HoverRow> = ui.data_mut(|d| {
                d.remove::<bool>(key);
                let h = d.get_temp(key.with("hover"));
                d.remove::<HoverRow>(key.with("hover"));
                h
            });
            if let Some(h) = hovered {
                let inner = ui.min_rect();
                let r = theme::RADIUS_CARD.saturating_sub(1);
                let mut corners = CornerRadius::ZERO;
                if h.rect.top() <= inner.top() + 0.5 {
                    corners.nw = r;
                    corners.ne = r;
                }
                if h.rect.bottom() >= inner.bottom() - 0.5 {
                    corners.sw = r;
                    corners.se = r;
                }
                ui.painter()
                    .set(h.bg, egui::Shape::rect_filled(h.rect, corners, p.card_hover));
            }
            result
        })
        .inner
}

pub fn hairline(ui: &mut Ui) {
    let p = theme::palette(ui.ctx());
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
    let r = Rect::from_min_max(pos2(rect.left() + 16.0, rect.top()), rect.max);
    ui.painter().rect_filled(r, 0.0, p.separator);
}

/// A row inside a grouped card. Returns the row response (clickable) and the content result.
pub fn row<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> (Response, R) {
    row_with_sense(ui, Sense::hover(), None, false, add)
}

/// A row that acts as one big button. Only use for rows without their own controls,
/// since the row's click area is registered on top of its contents.
pub fn row_button<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> (Response, R) {
    row_with_sense(ui, Sense::click(), None, false, add)
}

/// A row that can be picked up and dragged to reorder the list. Clicks still go to the
/// controls inside it; only a press that moves becomes a drag. `key` must stay the same
/// for the item wherever it ends up, so egui can follow the drag while the list scrolls.
/// `lifted` paints it as the one being carried.
pub fn row_drag<R>(ui: &mut Ui, key: &str, lifted: bool, add: impl FnOnce(&mut Ui) -> R) -> (Response, R) {
    let id = ui.id().with("drag").with(key);
    let (resp, inner) = row_with_sense(ui, Sense::drag(), Some(id), lifted, add);
    let resp = if lifted { resp } else { resp.on_hover_cursor(egui::CursorIcon::Grab) };
    (resp, inner)
}

fn row_with_sense<R>(ui: &mut Ui, sense: Sense, id: Option<Id>, lifted: bool, add: impl FnOnce(&mut Ui) -> R) -> (Response, R) {
    let p = theme::palette(ui.ctx());
    let bg = ui.painter().add(egui::Shape::Noop);
    let inner = Frame::new()
        .inner_margin(Margin::symmetric(16, 10))
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing = vec2(10.0, 4.0);
            ui.horizontal(|ui| add(ui)).inner
        });
    let rect = inner.response.rect;
    let id = id.unwrap_or_else(|| ui.id().with(rect.min.y as i64));
    let resp = ui.interact(rect, id, sense);
    if lifted {
        ui.painter().set(bg, egui::Shape::rect_filled(rect, 0.0, p.card_hover));
        let bar = Rect::from_min_max(rect.left_top(), pos2(rect.left() + 3.0, rect.bottom()));
        ui.painter().rect_filled(bar, 0.0, p.accent);
    } else if resp.contains_pointer() && ui.ctx().dragged_id().is_none() {
        let key = group_key(ui);
        if ui.data(|d| d.get_temp::<bool>(key).is_some()) {
            // Let the enclosing group paint it with the right corner rounding.
            ui.data_mut(|d| d.insert_temp(key.with("hover"), HoverRow { bg, rect }));
        } else {
            ui.painter().set(bg, egui::Shape::rect_filled(rect, 0.0, p.card_hover));
        }
    }
    (resp, inner.inner)
}

/// The line showing where a dragged row will land, drawn across the card at `y`.
pub fn drop_marker(ui: &Ui, x_range: egui::Rangef, y: f32) {
    let p = theme::palette(ui.ctx());
    let (left, right) = (x_range.min, x_range.max);
    let painter = ui.painter();
    painter.line_segment([pos2(left + 12.0, y), pos2(right - 12.0, y)], Stroke::new(2.0, p.accent));
    painter.circle_filled(pos2(left + 12.0, y), 4.0, p.accent);
}

// ---------------------------------------------------------------------------

pub fn badge(ui: &mut Ui, text: &str, color: Color32) {
    let font = FontId::new(11.0, theme::semibold());
    let galley = ui.painter().layout_no_wrap(text.to_owned(), font, color);
    let size = galley.size() + vec2(14.0, 6.0);
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    ui.painter().rect_filled(rect, rect.height() / 2.0, color.gamma_multiply(0.14));
    ui.painter().galley(
        pos2(rect.left() + 7.0, rect.top() + 3.0),
        galley,
        color,
    );
}

pub fn dot(ui: &mut Ui, color: Color32) {
    let (rect, _) = ui.allocate_exact_size(vec2(8.0, 8.0), Sense::hover());
    ui.painter().circle_filled(rect.center(), 4.0, color);
}

// ---------------------------------------------------------------------------

pub fn search_field(ui: &mut Ui, text: &mut String, width: f32) -> Response {
    let p = theme::palette(ui.ctx());
    let height = 32.0;
    let (rect, _) = ui.allocate_exact_size(vec2(width, height), Sense::hover());
    ui.painter().rect_filled(rect, height / 2.0, p.fill_secondary);
    ui.painter().text(
        pos2(rect.left() + 16.0, rect.center().y),
        Align2::CENTER_CENTER,
        theme::icon::SEARCH,
        FontId::new(13.0, theme::icons()),
        p.text_secondary,
    );
    let edit_rect = Rect::from_min_max(
        pos2(rect.left() + 30.0, rect.top() + 6.0),
        pos2(rect.right() - 12.0, rect.bottom() - 6.0),
    );
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(edit_rect));
    let resp = child.add(
        egui::TextEdit::singleline(text)
            .hint_text(RichText::new("Search").color(p.text_tertiary))
            .frame(Frame::NONE)
            .desired_width(edit_rect.width()),
    );
    if !text.is_empty() {
        let clear = Rect::from_center_size(pos2(rect.right() - 16.0, rect.center().y), vec2(20.0, 20.0));
        let c = ui.interact(clear, resp.id.with("clear"), Sense::click());
        ui.painter().text(
            clear.center(),
            Align2::CENTER_CENTER,
            theme::icon::CLOSE,
            FontId::new(9.0, theme::icons()),
            p.text_secondary,
        );
        if c.clicked() {
            text.clear();
        }
    }
    resp
}

/// Apple-style segmented control. Returns true if the selection changed.
pub fn segmented(ui: &mut Ui, selected: &mut usize, options: &[&str]) -> bool {
    let p = theme::palette(ui.ctx());
    let seg_w = if options.len() > 3 { 78.0 } else { 96.0 };
    let h = 28.0;
    let total = vec2(seg_w * options.len() as f32 + 4.0, h + 4.0);
    let (rect, _) = ui.allocate_exact_size(total, Sense::hover());
    ui.painter().rect_filled(rect, 8.0, p.fill_secondary);
    let mut changed = false;
    let id = ui.id().with("segmented");
    let target_x = rect.left() + 2.0 + seg_w * *selected as f32;
    let x = ui.ctx().animate_value_with_time(id, target_x, 0.15);
    let knob = Rect::from_min_size(pos2(x, rect.top() + 2.0), vec2(seg_w, h));
    ui.painter().rect_filled(knob.translate(vec2(0.0, 1.0)), 7.0, Color32::from_black_alpha(18));
    ui.painter().rect_filled(knob, 6.0, p.knob);
    for (i, opt) in options.iter().enumerate() {
        let r = Rect::from_min_size(pos2(rect.left() + 2.0 + seg_w * i as f32, rect.top() + 2.0), vec2(seg_w, h));
        let resp = ui.interact(r, id.with(i), Sense::click());
        if resp.clicked() && *selected != i {
            *selected = i;
            changed = true;
        }
        let font = if *selected == i {
            FontId::new(13.0, theme::semibold())
        } else {
            FontId::new(13.0, egui::FontFamily::Proportional)
        };
        ui.painter().text(r.center(), Align2::CENTER_CENTER, opt, font, p.text);
    }
    changed
}

pub fn spinner(ui: &mut Ui, color: Color32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(16.0), Sense::hover());
    let t = ui.input(|i| i.time) as f32;
    let n = 8;
    for k in 0..n {
        let a = k as f32 / n as f32 * std::f32::consts::TAU;
        let phase = ((t * 1.2 * n as f32) as usize + k) % n;
        let alpha = 1.0 - phase as f32 / n as f32;
        let c: Pos2 = rect.center() + vec2(a.cos(), a.sin()) * 5.5;
        ui.painter().circle_filled(c, 1.6, color.gamma_multiply(alpha.max(0.15)));
    }
    ui.ctx().request_repaint();
}
