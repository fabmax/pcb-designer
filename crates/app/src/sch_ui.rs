//! Schematic tool bar and tool settings (right panel).

use egui::RichText;
use view::Document;
use view::sch_tools::{POWER_NAMES, SchPlacer, SchTool};

use crate::fields::{self, grid, hint, label, row};
use crate::theme;

const LABEL_SIDES: [&str; 4] = ["0°", "90°", "180°", "270°"];

/// Tool buttons and usage hint above the schematic. Returns a newly chosen tool.
pub fn toolbar(ui: &mut egui::Ui, sch: &SchPlacer) -> Option<SchTool> {
    let mut chosen = None;
    let full = ui.max_rect();
    egui::Frame::NONE.fill(theme::panel()).inner_margin(egui::Margin::symmetric(8, 4)).show(ui, |ui| {
        ui.set_width(full.width() - 16.0);
        ui.horizontal(|ui| {
            for &tool in SchTool::BAR {
                if ui.selectable_label(sch.tool() == tool, tool.label()).clicked() {
                    chosen = Some(tool);
                }
            }
            if sch.tool() == SchTool::Place {
                let _ = ui.selectable_label(true, SchTool::Place.label());
            }
            ui.separator();
            ui.add(egui::Label::new(RichText::new(sch.tool().hint()).color(theme::text_dim())).truncate());
        });
    });
    chosen
}

/// Settings of the active tool.
pub fn settings(ui: &mut egui::Ui, sch: &mut SchPlacer, doc: &Document) {
    let tool = sch.tool();
    fields::heading(ui, &format!("{} tool", tool.label()), "");
    ui.add_space(4.0);
    let s = &mut sch.settings;
    match tool {
        SchTool::Select => {}
        SchTool::Wire => {
            grid(ui, "sch_tool", |ui| {
                label(ui, "Corner");
                ui.horizontal(|ui| {
                    ui.selectable_value(&mut s.vertical_first, false, "Horizontal first");
                    ui.selectable_value(&mut s.vertical_first, true, "Vertical first");
                });
                ui.end_row();
            });
            ui.add_space(6.0);
            hint(ui, "Wires end automatically on pins, wires, labels and junctions. / switches the corner.");
        }
        SchTool::Label => {
            grid(ui, "sch_tool", |ui| {
                label(ui, "Name");
                ui.add(egui::TextEdit::singleline(&mut s.label).desired_width(f32::INFINITY));
                ui.end_row();
                label(ui, "Rotation");
                if let Some(a) = fields::rotation_combo(ui, "sch_rot", s.rotation, LABEL_SIDES) {
                    s.rotation = a;
                }
                ui.end_row();
            });
            ui.add_space(6.0);
            hint(ui, "Labels with the same name are connected. A trailing number counts up.");
        }
        SchTool::Power => {
            grid(ui, "sch_tool", |ui| {
                label(ui, "Net");
                ui.add(egui::TextEdit::singleline(&mut s.power).desired_width(f32::INFINITY));
                ui.end_row();
                label(ui, "");
                ui.horizontal_wrapped(|ui| {
                    for &name in POWER_NAMES {
                        if ui.selectable_label(s.power == name, name).clicked() {
                            s.power = name.to_string();
                        }
                    }
                });
                ui.end_row();
                label(ui, "Rotation");
                if let Some(a) = fields::rotation_combo(ui, "sch_rot", s.rotation, LABEL_SIDES) {
                    s.rotation = a;
                }
                ui.end_row();
            });
        }
        SchTool::Junction => hint(ui, "Connects crossing wires. T-connections and pins don't need one."),
        SchTool::Place => {
            let Some(pl) = sch.placing else { return };
            let project = doc.project();
            let Some(part) = project.library.parts.get(pl.part) else { return };
            grid(ui, "sch_tool", |ui| {
                row(ui, "Part", part.name.clone());
                if part.units.len() > 1 {
                    let unit = part.units.get(pl.unit as usize).map_or(String::new(), |u| u.name.clone());
                    let refdes = pl
                        .component
                        .and_then(|c| project.schematic.components.get(c))
                        .map_or(format!("new {}?", part.prefix), |c| c.refdes.clone());
                    row(ui, "Unit", format!("{unit} ({} of {}) · {refdes}", pl.unit + 1, part.units.len()));
                }
                label(ui, "Rotation");
                if let Some(a) = fields::rotation_combo(ui, "sch_rot", s.rotation, LABEL_SIDES) {
                    s.rotation = a;
                }
                ui.end_row();
                label(ui, "Mirror");
                ui.checkbox(&mut s.mirror, "");
                ui.end_row();
            });
            ui.add_space(6.0);
            hint(ui, "Each click places a part; multi-unit parts place their units one after another.");
        }
    }
    ui.add_space(6.0);
    hint(ui, "Esc ends the tool.");
}
