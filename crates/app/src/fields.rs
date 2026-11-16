//! Form widgets that commit edits the way the properties panels do: text fields on Enter
//! or focus loss, dragged values continuously but as one undo group.

use egui::RichText;
use model::*;
use rustc_hash::FxHashMap;
use view::interaction::new_group;

use crate::theme;

#[derive(Default)]
pub struct Fields {
    group: Option<u64>,
    /// A grouped widget was dragged or edited this frame.
    active: bool,
    texts: FxHashMap<egui::Id, String>,
}

impl Fields {
    pub fn begin_frame(&mut self) {
        self.active = false;
    }

    /// Returns true when an edit group just ended (the caller closes it in its document).
    pub fn end_frame(&mut self) -> bool {
        !self.active && self.group.take().is_some()
    }

    /// Undo group for continuous edits.
    pub fn group(&mut self) -> u64 {
        *self.group.get_or_insert_with(new_group)
    }

    pub fn forget(&mut self) {
        self.texts.clear();
    }

    /// Text field committed on Enter / focus loss. Returns the new text if it changed.
    pub fn text(
        &mut self,
        ui: &mut egui::Ui,
        salt: impl std::hash::Hash + std::fmt::Debug,
        current: &str,
    ) -> Option<String> {
        self.text_sized(ui, salt, current, f32::INFINITY)
    }

    pub fn text_sized(
        &mut self,
        ui: &mut egui::Ui,
        salt: impl std::hash::Hash + std::fmt::Debug,
        current: &str,
        width: f32,
    ) -> Option<String> {
        let id = ui.make_persistent_id(salt);
        let focused = ui.memory(|m| m.has_focus(id));
        let buf = self.texts.entry(id).or_insert_with(|| current.to_string());
        if !focused {
            buf.clear();
            buf.push_str(current);
        }
        let width = width.min(ui.available_width());
        let r = ui.add(egui::TextEdit::singleline(buf).id(id).desired_width(width));
        (r.lost_focus() && buf != current).then(|| buf.clone())
    }

    /// Length in the display unit, edited by dragging or typing; at least `min` mm.
    /// Returns the new value in nm.
    pub fn length(&mut self, ui: &mut egui::Ui, value: Coord, min: f64) -> Option<Coord> {
        // Lengths are below a kilometre.
        let limit = mm(1e6);
        let min = if min > -1e6 { mm(min) } else { -limit };
        let (r, v) = crate::units::drag(ui, value, min, limit);
        self.active |= r.dragged() || r.has_focus();
        v
    }

    pub fn number(
        &mut self,
        ui: &mut egui::Ui,
        value: f64,
        range: std::ops::RangeInclusive<f64>,
        speed: f64,
    ) -> Option<f64> {
        let mut v = value;
        let r = ui.add(egui::DragValue::new(&mut v).speed(speed).range(range).max_decimals(3));
        self.active |= r.dragged() || r.has_focus();
        r.changed().then_some(v)
    }

    pub fn point(&mut self, ui: &mut egui::Ui, p: Point) -> Option<Point> {
        let mut out = None;
        ui.horizontal(|ui| {
            let x = self.length(ui, p.x, f64::MIN);
            let y = self.length(ui, p.y, f64::MIN);
            if x.is_some() || y.is_some() {
                out = Some(Point::new(x.unwrap_or(p.x), y.unwrap_or(p.y)));
            }
        });
        out
    }

    pub fn size(&mut self, ui: &mut egui::Ui, s: Size) -> Option<Size> {
        let mut out = None;
        ui.horizontal(|ui| {
            let w = self.length(ui, s.w, 0.01);
            let h = self.length(ui, s.h, 0.01);
            if w.is_some() || h.is_some() {
                out = Some(Size { w: w.unwrap_or(s.w), h: h.unwrap_or(s.h) });
            }
        });
        out
    }
}

pub fn heading(ui: &mut egui::Ui, title: &str, subtitle: &str) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(title).strong().size(16.0));
        if !subtitle.is_empty() {
            ui.label(RichText::new(subtitle).color(theme::text_dim()));
        }
    });
}

pub fn grid(ui: &mut egui::Ui, salt: &str, add: impl FnOnce(&mut egui::Ui)) {
    egui::Grid::new(salt).num_columns(2).spacing([12.0, 6.0]).show(ui, add);
}

pub fn label(ui: &mut egui::Ui, text: &str) {
    ui.label(RichText::new(text).color(theme::text_dim()));
}

pub fn row(ui: &mut egui::Ui, k: &str, v: String) {
    label(ui, k);
    ui.label(v);
    ui.end_row();
}

pub fn hint(ui: &mut egui::Ui, text: &str) {
    ui.label(RichText::new(text).color(theme::text_dim()));
}

/// Quarter-turn picker. Returns the new angle if changed.
pub fn rotation_combo(ui: &mut egui::Ui, salt: &str, current: Angle, names: [&str; 4]) -> Option<Angle> {
    let deg = current.degrees();
    let name = |d: f64| match current.quarter_turns() {
        Some(q) if (d - deg).abs() < 1e-9 => names[q as usize].to_string(),
        _ => format!("{d:.1}°"),
    };
    let mut out = None;
    egui::ComboBox::from_id_salt(salt).selected_text(name(deg)).show_ui(ui, |ui| {
        for (q, n) in names.iter().enumerate() {
            let d = q as f64 * 90.0;
            if ui.selectable_label((d - deg).abs() < 1e-9, *n).clicked() && (d - deg).abs() > 1e-9 {
                out = Some(Angle::deg(d));
            }
        }
    });
    out
}

/// The glyph of a power port named `name`: "Auto" (what the name suggests) or a fixed
/// style. Returns the newly chosen value.
pub fn power_style_combo(
    ui: &mut egui::Ui,
    salt: &str,
    current: Option<model::PowerStyle>,
    name: &str,
) -> Option<Option<model::PowerStyle>> {
    let auto = format!("Auto ({})", model::PowerStyle::for_name(name).label().to_lowercase());
    let options: Vec<_> = std::iter::once((None, auto))
        .chain(model::PowerStyle::ALL.into_iter().map(|s| (Some(s), s.label().to_string())))
        .collect();
    combo(ui, salt, &current, &options)
}

/// Generic combo over `options`; returns the newly chosen value.
pub fn combo<T: PartialEq + Clone>(
    ui: &mut egui::Ui,
    salt: impl std::hash::Hash + std::fmt::Debug,
    current: &T,
    options: &[(T, String)],
) -> Option<T> {
    let text = options.iter().find(|(v, _)| v == current).map_or("—".to_string(), |(_, n)| n.clone());
    let mut out = None;
    egui::ComboBox::from_id_salt(salt).selected_text(text).show_ui(ui, |ui| {
        for (v, n) in options {
            if ui.selectable_label(v == current, n).clicked() && v != current {
                out = Some(v.clone());
            }
        }
    });
    out
}

/// Rows of a drawing tool's corner style (icon buttons) and posture (which leg first:
/// automatic or fixed), for the styles `styles`.
pub fn corner_rows(
    ui: &mut egui::Ui,
    corners: &mut view::bend::Corners,
    bend: &mut view::bend::Bend,
    styles: &[view::bend::Corners],
) {
    use crate::icons::{self, Icon};
    use view::bend::{Bend, Corners};
    label(ui, "Corners");
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        for &style in styles {
            let icon = match style {
                Corners::Deg90 => Icon::Corner90,
                Corners::Deg45 => Icon::Corner45,
                Corners::Free => Icon::CornerFree,
            };
            let active = *corners == style;
            let (rect, r) = ui.allocate_exact_size(egui::vec2(26.0, 22.0), egui::Sense::click());
            let v = ui.visuals();
            let (fill, color) = if active {
                (theme::accent(), egui::Color32::WHITE)
            } else if r.hovered() {
                (v.widgets.hovered.weak_bg_fill, v.strong_text_color())
            } else {
                (v.widgets.inactive.weak_bg_fill, v.text_color())
            };
            ui.painter().rect_filled(rect, 4.0, fill);
            icons::paint(ui.painter(), rect.shrink(4.0), icon, color);
            if r.on_hover_text(style.label()).clicked() {
                *corners = style;
            }
        }
    });
    ui.end_row();
    if *corners != Corners::Free {
        label(ui, "First leg");
        ui.horizontal(|ui| {
            let (first, second) = corners.postures();
            ui.selectable_value(bend, Bend::Auto, "Auto")
                .on_hover_text("Follows the way the cursor leaves the start point; / flips it");
            ui.selectable_value(bend, Bend::First, first).on_hover_text(format!("{first} leg first"));
            ui.selectable_value(bend, Bend::Second, second).on_hover_text(format!("{second} leg first"));
        });
        ui.end_row();
    }
}
