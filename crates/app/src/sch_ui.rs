//! Schematic tool bar and tool settings (right panel).

use model::{EntryDir, LabelKind, LabelShape};
use view::Document;
use view::sch_tools::{POWER_NAMES, SchPlacer, SchTool};

use crate::fields::{self, grid, hint, label, row};
use crate::icons::Icon;
use crate::keymap::{Action, Keymap};
use crate::toolbar::{self, ToolButton};

const LABEL_SIDES: [&str; 4] = ["0°", "90°", "180°", "270°"];

/// Floating tool bar and usage hint over the schematic canvas. Returns a newly chosen tool.
pub fn toolbar(ui: &mut egui::Ui, canvas: egui::Rect, sch: &SchPlacer, keymap: &Keymap) -> Option<SchTool> {
    let b =
        |tool, icon, about, action| ToolButton { tool, icon, name: SchTool::label(tool), about, action: Some(action) };
    let select = [b(
        SchTool::Select,
        Icon::Select,
        "Select, move and edit symbols, wires and labels. Drag a box to select several.",
        Action::ToolSelect,
    )];
    let draw = [
        b(SchTool::Wire, Icon::Wire, "Draw wires between pins.", Action::SchWire),
        b(
            SchTool::Bus,
            Icon::Bus,
            "Draw a bus: groups signals, named by a label like D[0..7] on it. Never connects by itself.",
            Action::SchBus,
        ),
        b(
            SchTool::BusEntry,
            Icon::BusEntry,
            "Place a bus entry: a signal's wire leaves the bus here and connects by its label.",
            Action::SchBusEntry,
        ),
        b(SchTool::Label, Icon::Label, "Name a net: local, global or hierarchical labels.", Action::SchLabel),
        b(SchTool::Power, Icon::Power, "Place a power port (GND, +3V3, ...): connects to its net.", Action::SchPower),
        b(SchTool::Junction, Icon::Junction, "Connect crossing wires with a junction dot.", Action::SchJunction),
    ];
    let place = [ToolButton {
        tool: SchTool::Place,
        icon: Icon::Place,
        name: "Place",
        about: "Placing a part from the library. Esc stops.",
        action: None,
    }];
    let placing: &[ToolButton<SchTool>] = if sch.tool() == SchTool::Place { &place } else { &[] };
    let id = egui::Id::unique("sch_toolbar");
    let (chosen, _) = toolbar::strip(ui, canvas, id, &[&select, &draw, placing], sch.tool(), keymap);
    toolbar::hint(ui, canvas, sch.tool().hint());
    chosen.filter(|&t| t != SchTool::Place)
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
        SchTool::Bus => {
            grid(ui, "sch_tool", |ui| {
                label(ui, "Corner");
                ui.horizontal(|ui| {
                    ui.selectable_value(&mut s.vertical_first, false, "Horizontal first");
                    ui.selectable_value(&mut s.vertical_first, true, "Vertical first");
                });
                ui.end_row();
            });
            ui.add_space(6.0);
            hint(
                ui,
                "A new bus without a name asks for one when you finish it; the name becomes a label on \
                 the bus: D[0..7], {SDA SCL} or I2C{SDA SCL} (signals I2C.SDA, I2C.SCL).",
            );
        }
        SchTool::BusEntry => {
            grid(ui, "sch_tool", |ui| {
                label(ui, "Direction");
                if let Some(dir) = entry_dir_picker(ui, s.entry) {
                    s.entry = dir;
                }
                ui.end_row();
            });
            ui.add_space(6.0);
            hint(
                ui,
                "Click on a bus. Wiring an unnamed net to the entry asks which of the bus's signals it is. R rotates.",
            );
        }
        SchTool::Label => {
            grid(ui, "sch_tool", |ui| {
                label(ui, "Name");
                ui.add(egui::TextEdit::singleline(&mut s.label).desired_width(f32::INFINITY));
                ui.end_row();
                label(ui, "Kind");
                ui.horizontal(|ui| {
                    for kind in [LabelKind::Local, LabelKind::Global, LabelKind::Hierarchical] {
                        ui.selectable_value(&mut s.label_kind, kind, kind.label());
                    }
                });
                ui.end_row();
                if s.label_kind.has_shape() {
                    label(ui, "Shape");
                    egui::ComboBox::from_id_salt("label_shape").selected_text(s.shape.label()).show_ui(ui, |ui| {
                        for shape in LabelShape::ALL {
                            ui.selectable_value(&mut s.shape, shape, shape.label());
                        }
                    });
                    ui.end_row();
                }
                label(ui, "Rotation");
                if let Some(a) = fields::rotation_combo(ui, "sch_rot", s.rotation, LABEL_SIDES) {
                    s.rotation = a;
                }
                ui.end_row();
            });
            ui.add_space(6.0);
            hint(
                ui,
                match s.label_kind {
                    LabelKind::Local | LabelKind::Power => {
                        "Same-named labels on this sheet are connected; the net is named /NAME. A trailing \
                         number counts up. On a bus, use a bus name like D[0..7]."
                    }
                    LabelKind::Global => "Same-named global labels are connected on every sheet.",
                    LabelKind::Hierarchical => {
                        "Connects to the same-named pin of this sheet's symbol on the parent sheet."
                    }
                },
            );
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

/// Four buttons showing the bus entry directions. Returns a newly picked one.
pub fn entry_dir_picker(ui: &mut egui::Ui, current: EntryDir) -> Option<EntryDir> {
    let mut picked = None;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        for dir in EntryDir::ALL {
            let (rect, r) = ui.allocate_exact_size(egui::vec2(26.0, 22.0), egui::Sense::click());
            let selected = dir == current;
            let v = ui.style().interact_selectable(&r, selected);
            ui.painter().rect_filled(rect, 4.0, v.weak_bg_fill);
            // The diagonal, from the bus end (dot) towards the wire end; y up like the canvas.
            let o = dir.offset();
            let d = egui::vec2(o.x.signum() as f32, -(o.y.signum() as f32)) * 5.0;
            let c = rect.center();
            let stroke = egui::Stroke::new(2.0, v.fg_stroke.color);
            ui.painter().line_segment([c - d, c + d], stroke);
            ui.painter().circle_filled(c - d, 2.0, v.fg_stroke.color);
            let name = match dir {
                EntryDir::UpRight => "Up right",
                EntryDir::DownRight => "Down right",
                EntryDir::DownLeft => "Down left",
                EntryDir::UpLeft => "Up left",
            };
            if r.on_hover_text(name).clicked() && !selected {
                picked = Some(dir);
            }
        }
    });
    picked
}
