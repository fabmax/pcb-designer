//! "Create array…": the Tool settings form laying out copies of a selected pad or pin.

use model::*;
use view::array::{Dir, Layout, Slot};

use crate::fields::{Fields, hint};
use crate::theme;

#[derive(Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
enum Kind {
    Linear,
    DualRow,
    Grid,
    Quad,
}

impl Kind {
    fn label(self) -> &'static str {
        match self {
            Kind::Linear => "Linear",
            Kind::DualRow => "Dual row",
            Kind::Grid => "Grid",
            Kind::Quad => "Quad",
        }
    }
}

/// Remembered across sessions (all but `pins` and the fields' typing state).
#[derive(serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct ArrayForm {
    /// Making pins: linear arrays only.
    #[serde(skip)]
    pins: bool,
    kind: Kind,
    count: usize,
    per_row: usize,
    rows: usize,
    cols: usize,
    side_count: usize,
    end_count: usize,
    pitch: Coord,
    pitch_y: Coord,
    span: Coord,
    end_span: Coord,
    dir: Dir,
    #[serde(skip)]
    fields: Fields,
}

impl Default for ArrayForm {
    fn default() -> Self {
        ArrayForm::new(false)
    }
}

impl ArrayForm {
    pub fn new(pins: bool) -> ArrayForm {
        ArrayForm {
            pins,
            kind: Kind::Linear,
            count: 4,
            per_row: 4,
            rows: 4,
            cols: 4,
            side_count: 8,
            end_count: 8,
            pitch: if pins { mm(2.54) } else { mm(1.27) },
            pitch_y: mm(1.27),
            span: mm(5.0),
            end_span: mm(5.0),
            dir: Dir::Down,
            fields: Fields::default(),
        }
    }

    /// The remembered form of `value`, for pins or pads.
    pub fn recall(value: Option<&serde_json::Value>, pins: bool) -> ArrayForm {
        match value {
            Some(v) => ArrayForm { pins, ..view::tool_memory::recall(Some(v)) },
            None => ArrayForm::new(pins),
        }
    }

    pub fn layout(&self) -> Layout {
        match self.kind {
            Kind::Linear => Layout::Linear { count: self.count, pitch: self.pitch, dir: self.dir },
            Kind::DualRow => {
                Layout::DualRow { per_row: self.per_row, pitch: self.pitch, span: self.span, dir: self.dir }
            }
            Kind::Grid => Layout::Grid { rows: self.rows, cols: self.cols, pitch_x: self.pitch, pitch_y: self.pitch_y },
            Kind::Quad => Layout::Quad {
                side_count: self.side_count,
                end_count: self.end_count,
                pitch: self.pitch,
                span: self.span,
                end_span: self.end_span,
                dir: self.dir,
            },
        }
    }

    /// The form (Tool settings panel) for an array of `what` ("pad 1"). `slots` is the
    /// current layout (or why it can't be made) and `clashes` the names taken already.
    /// Returns `Some(true)` to create, `Some(false)` to cancel.
    pub fn ui(
        &mut self,
        ui: &mut egui::Ui,
        what: &str,
        slots: &Result<Vec<Slot>, String>,
        clashes: &[String],
    ) -> Option<bool> {
        let mut outcome = None;
        let noun = if self.pins { "pin" } else { "pad" };
        crate::fields::subject(
            ui,
            &format!("{} array", if self.pins { "Pin" } else { "Pad" }),
            &format!("of {what}"),
            None,
        );
        self.fields.begin_frame();
        if !self.pins {
            ui.horizontal_wrapped(|ui| {
                for k in [Kind::Linear, Kind::DualRow, Kind::Grid, Kind::Quad] {
                    ui.selectable_value(&mut self.kind, k, k.label());
                }
            });
            ui.add_space(6.0);
        }
        crate::fields::grid(ui, "array_grid", |ui| self.rows_ui(ui));
        ui.add_space(6.0);
        match slots {
            Ok(s) => hint(ui, &format!("{} {noun}s, numbered on from the selected one.", s.len())),
            Err(e) => {
                ui.colored_label(theme::warn(), e);
            }
        }
        if !clashes.is_empty() {
            let mut list = clashes.iter().take(8).cloned().collect::<Vec<_>>().join(", ");
            if clashes.len() > 8 {
                list.push_str(", …");
            }
            ui.colored_label(theme::warn(), format!("Names in use already: {list}"));
        }
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            if ui.add_enabled(slots.is_ok(), egui::Button::new("Create")).clicked() {
                outcome = Some(true);
            }
            if ui.button("Cancel").clicked() {
                outcome = Some(false);
            }
        });
        ui.add_space(4.0);
        hint(ui, "Enter creates · Esc cancels");
        self.fields.end_frame();
        outcome
    }

    fn rows_ui(&mut self, ui: &mut egui::Ui) {
        let count = |ui: &mut egui::Ui, label: &str, v: &mut usize| {
            crate::fields::label(ui, label);
            crate::fields::fill(ui, egui::DragValue::new(v).range(1..=1000).speed(0.1));
            ui.end_row();
        };
        let length = |f: &mut Fields, ui: &mut egui::Ui, label: &str, v: &mut Coord| {
            crate::fields::label(ui, label);
            if let Some(n) = f.length(ui, *v, 0.001) {
                *v = n;
            }
            ui.end_row();
        };
        match self.kind {
            Kind::Linear => count(ui, "Count", &mut self.count),
            Kind::DualRow => count(ui, "Per row", &mut self.per_row),
            Kind::Grid => {
                count(ui, "Rows", &mut self.rows);
                count(ui, "Columns", &mut self.cols);
            }
            Kind::Quad => {
                count(ui, "Per side", &mut self.side_count);
                count(ui, "Per end", &mut self.end_count);
            }
        }
        if self.kind == Kind::Grid {
            length(&mut self.fields, ui, "Pitch X", &mut self.pitch);
            length(&mut self.fields, ui, "Pitch Y", &mut self.pitch_y);
        } else {
            length(&mut self.fields, ui, "Pitch", &mut self.pitch);
        }
        match self.kind {
            Kind::DualRow => length(&mut self.fields, ui, "Row spacing", &mut self.span),
            Kind::Quad => {
                length(&mut self.fields, ui, "Side spacing", &mut self.span);
                length(&mut self.fields, ui, "End spacing", &mut self.end_span);
            }
            _ => {}
        }
        if self.kind != Kind::Grid {
            crate::fields::label(ui, "Direction");
            ui.horizontal(|ui| {
                for d in Dir::ALL {
                    ui.selectable_value(&mut self.dir, d, d.label());
                }
            });
            ui.end_row();
        }
    }
}
