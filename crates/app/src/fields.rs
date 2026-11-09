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

    /// Length in mm, edited by dragging or typing. Returns the new value in nm.
    pub fn length(&mut self, ui: &mut egui::Ui, value: Coord, min: f64) -> Option<Coord> {
        let mut v = to_mm(value);
        let r = ui.add(egui::DragValue::new(&mut v).speed(0.05).range(min..=f64::MAX).suffix(" mm").max_decimals(4));
        self.active |= r.dragged() || r.has_focus();
        r.changed().then(|| mm(v))
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
