//! A colour field: a swatch that opens a picker (saturation/value square, hue and alpha
//! bars, RGBA values) and the colour's hex code, editable so a colour can be pasted in.
//!
//! egui's own picker has a numeric-mode toggle and a copy button whose clipboard glyph our
//! fonts don't have; the hex field covers copying.

use egui::ecolor::{Hsva, HsvaGamma};
use egui::epaint::{CircleShape, Mesh};
use egui::{
    Color32, DragValue, FontId, Popup, PopupCloseBehavior, Rect, Response, Sense, Stroke, StrokeKind, TextEdit, Ui,
    lerp, pos2, remap_clamp, vec2,
};

/// Edits `rgba` (unmultiplied sRGB with alpha). Returns whether it changed.
pub fn color_edit(ui: &mut Ui, rgba: &mut [u8; 4]) -> bool {
    let mut changed = false;
    let popup_id = ui.auto_id_with("color_popup");
    let open = Popup::is_id_open(ui.ctx(), popup_id);
    let button = swatch(ui, *rgba, open).on_hover_text("Pick a colour");
    Popup::menu(&button).id(popup_id).close_behavior(PopupCloseBehavior::CloseOnClickOutside).show(|ui| {
        changed |= picker(ui, popup_id, rgba);
    });
    changed |= hex_field(ui, rgba);
    changed
}

/// `#rrggbb`, or `#rrggbbaa` when not opaque.
pub fn hex(rgba: [u8; 4]) -> String {
    view::style::color_hex(render::Color(rgba))
}

/// A hex code as typed or pasted: `#` optional, surrounding blanks ignored.
pub fn parse_hex(text: &str) -> Option<[u8; 4]> {
    let t = text.trim();
    let t = t.strip_prefix('#').unwrap_or(t);
    view::style::parse_color_hex(&format!("#{t}")).map(|c| c.0)
}

/// The hex code. While focused it keeps the typed text and applies it whenever it's a
/// valid colour; otherwise it shows the current colour.
fn hex_field(ui: &mut Ui, rgba: &mut [u8; 4]) -> bool {
    let id = ui.auto_id_with("color_hex");
    let key = id.with("text");
    let mut text = ui.data(|d| d.get_temp::<String>(key)).unwrap_or_else(|| hex(*rgba));
    let font = FontId::monospace(ui.style().text_styles[&egui::TextStyle::Body].size);
    let r = ui.add(TextEdit::singleline(&mut text).id(id).font(font).desired_width(86.0).char_limit(12));
    let mut changed = false;
    if r.has_focus() {
        if r.changed()
            && let Some(c) = parse_hex(&text)
            && c != *rgba
        {
            *rgba = c;
            changed = true;
        }
        ui.data_mut(|d| d.insert_temp(key, text));
    } else {
        ui.data_mut(|d| d.remove::<String>(key));
    }
    changed
}

/// The colour on a button (its opaque version on the right half when translucent).
fn swatch(ui: &mut Ui, [r, g, b, a]: [u8; 4], open: bool) -> Response {
    let size = vec2(ui.spacing().interact_size.x, ui.spacing().interact_size.y + 4.0);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    let visuals = if open { &ui.visuals().widgets.open } else { ui.style().interact(&response) };
    let painter = ui.painter();
    let inner = rect.shrink(2.0);
    let opaque = Color32::from_rgb(r, g, b);
    if a == 255 {
        painter.rect_filled(inner, 3.0, opaque);
    } else {
        paint_checkers(painter, inner);
        let (left, right) = inner.split_left_right_at_fraction(0.5);
        painter.rect_filled(left, 0.0, Color32::from_rgba_unmultiplied(r, g, b, a));
        painter.rect_filled(right, 0.0, opaque);
    }
    painter.rect_stroke(rect, 4.0, Stroke::new(1.5, visuals.bg_fill), StrokeKind::Inside);
    response
}

fn to_rgba(h: HsvaGamma) -> [u8; 4] {
    Hsva::from(h).to_srgba_unmultiplied()
}

fn contrast(c: Color32) -> Color32 {
    let l = 0.299 * c.r() as f32 + 0.587 * c.g() as f32 + 0.114 * c.b() as f32;
    if l < 128.0 { Color32::WHITE } else { Color32::BLACK }
}

const WIDTH: f32 = 240.0;
const BAR: f32 = 18.0;
/// Grid steps of the gradients (a multiple of 6 to hit the pure hues).
const STEPS: usize = 36;

fn picker(ui: &mut Ui, id: egui::Id, rgba: &mut [u8; 4]) -> bool {
    // The HSV value is kept while the popup is open, so the hue survives greys and black.
    let key = id.with("hsva");
    let mut h = ui
        .data(|d| d.get_temp::<HsvaGamma>(key))
        .filter(|h| to_rgba(*h) == *rgba)
        .unwrap_or_else(|| HsvaGamma::from(Hsva::from_srgba_unmultiplied(*rgba)));
    let before = *rgba;
    ui.spacing_mut().item_spacing.y = 8.0;

    let opaque = HsvaGamma { a: 1.0, ..h };
    sv_square(ui, &mut h.s, &mut h.v, |s, v| HsvaGamma { s, v, ..opaque }.into());
    bar(ui, &mut h.h, false, |x| HsvaGamma { h: x, s: 1.0, v: 1.0, a: 1.0 }.into()).on_hover_text("Hue");
    bar(ui, &mut h.a, true, |a| HsvaGamma { a, ..opaque }.into()).on_hover_text("Opacity");
    let mut c = to_rgba(h);

    let mut typed = false;
    ui.horizontal(|ui| {
        for (i, name) in ["R ", "G ", "B ", "A "].into_iter().enumerate() {
            typed |= ui.add(DragValue::new(&mut c[i]).speed(0.5).prefix(name)).changed();
        }
    });
    if typed {
        h = HsvaGamma::from(Hsva::from_srgba_unmultiplied(c));
    }
    ui.data_mut(|d| d.insert_temp(key, h));
    *rgba = c;
    *rgba != before
}

/// The saturation (x) / value (y) square.
fn sv_square(ui: &mut Ui, s: &mut f32, v: &mut f32, color_at: impl Fn(f32, f32) -> Color32) {
    let (rect, response) = ui.allocate_exact_size(vec2(WIDTH, WIDTH * 0.75), Sense::click_and_drag());
    if let Some(p) = response.interact_pointer_pos() {
        *s = remap_clamp(p.x, rect.left()..=rect.right(), 0.0..=1.0);
        *v = remap_clamp(p.y, rect.bottom()..=rect.top(), 0.0..=1.0);
    }
    let mut mesh = Mesh::default();
    for yi in 0..=STEPS {
        for xi in 0..=STEPS {
            let (x, y) = (xi as f32 / STEPS as f32, yi as f32 / STEPS as f32);
            let pos = pos2(lerp(rect.left()..=rect.right(), x), lerp(rect.bottom()..=rect.top(), y));
            mesh.colored_vertex(pos, color_at(x, y));
        }
    }
    let row = STEPS as u32 + 1;
    for yi in 0..STEPS as u32 {
        for xi in 0..STEPS as u32 {
            let i = yi * row + xi;
            mesh.add_triangle(i, i + 1, i + row);
            mesh.add_triangle(i + 1, i + row + 1, i + row);
        }
    }
    let painter = ui.painter();
    painter.add(mesh);
    painter.rect_stroke(rect, 0.0, ui.visuals().widgets.noninteractive.bg_stroke, StrokeKind::Outside);
    let picked = color_at(*s, *v);
    painter.add(CircleShape {
        center: pos2(lerp(rect.left()..=rect.right(), *s), lerp(rect.bottom()..=rect.top(), *v)),
        radius: 7.0,
        fill: picked,
        stroke: Stroke::new(2.0, contrast(picked)),
    });
}

/// A horizontal 0…1 bar (hue or alpha; alpha on checkers).
fn bar(ui: &mut Ui, value: &mut f32, checkers: bool, color_at: impl Fn(f32) -> Color32) -> Response {
    let (rect, response) = ui.allocate_exact_size(vec2(WIDTH, BAR), Sense::click_and_drag());
    if let Some(p) = response.interact_pointer_pos() {
        *value = remap_clamp(p.x, rect.left()..=rect.right(), 0.0..=1.0);
    }
    let painter = ui.painter();
    if checkers {
        paint_checkers(painter, rect);
    }
    let mut mesh = Mesh::default();
    for i in 0..=STEPS {
        let t = i as f32 / STEPS as f32;
        let x = lerp(rect.left()..=rect.right(), t);
        let c = color_at(t);
        mesh.colored_vertex(pos2(x, rect.top()), c);
        mesh.colored_vertex(pos2(x, rect.bottom()), c);
        if i > 0 {
            let j = 2 * i as u32;
            mesh.add_triangle(j - 2, j - 1, j);
            mesh.add_triangle(j - 1, j + 1, j);
        }
    }
    painter.add(mesh);
    painter.rect_stroke(rect, 0.0, ui.visuals().widgets.noninteractive.bg_stroke, StrokeKind::Outside);
    let x = lerp(rect.left()..=rect.right(), *value);
    let marker = Rect::from_center_size(pos2(x, rect.center().y), vec2(6.0, BAR + 4.0));
    let fill = color_at(*value).to_opaque();
    painter.rect(marker, 2.0, fill, Stroke::new(2.0, contrast(fill)), StrokeKind::Middle);
    response
}

fn paint_checkers(painter: &egui::Painter, rect: Rect) {
    let size = rect.height() / 2.0;
    painter.rect_filled(rect, 0.0, Color32::from_gray(128));
    let mut x = rect.left();
    let mut col = 0;
    while x < rect.right() {
        let w = size.min(rect.right() - x);
        for row in 0..2 {
            if (col + row) % 2 == 0 {
                let r = Rect::from_min_size(pos2(x, rect.top() + row as f32 * size), vec2(w, size));
                painter.rect_filled(r, 0.0, Color32::from_gray(32));
            }
        }
        x += size;
        col += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_parses_pasted_text() {
        assert_eq!(parse_hex("#c83434"), Some([0xc8, 0x34, 0x34, 0xff]));
        assert_eq!(parse_hex("  c83434 \n"), Some([0xc8, 0x34, 0x34, 0xff]));
        assert_eq!(parse_hex("#C8343480"), Some([0xc8, 0x34, 0x34, 0x80]));
        assert_eq!(parse_hex("#c834"), None);
        assert_eq!(parse_hex("zz3434"), None);
        assert_eq!(hex([0xc8, 0x34, 0x34, 0xff]), "#c83434");
    }

    #[test]
    fn hsv_round_trips_bytes() {
        for c in [[0xc8, 0x34, 0x34, 0xff], [0, 0, 0, 0x80], [12, 200, 99, 255], [255, 255, 255, 1]] {
            assert_eq!(to_rgba(HsvaGamma::from(Hsva::from_srgba_unmultiplied(c))), c);
        }
    }
}
