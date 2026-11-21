//! Schematic tool bar and tool settings (right panel).

use model::{EntryDir, LabelKind, LabelShape};
use view::Document;
use view::bend::Corners;
use view::sch_tools::{POWER_NAMES, SchPlacer, SchTool};

use crate::fields::{self, grid, label, row};
use crate::icons::Icon;
use crate::keymap::{Action, Keymap};
use crate::toolbar::{self, ToolButton};

const LABEL_SIDES: [&str; 4] = ["0°", "90°", "180°", "270°"];

/// A tool bar button: a tool, or Run (simulation).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pick {
    Tool(SchTool),
    Run,
}

/// Floating tool bar and usage hint over the schematic canvas. With `sim` (the Simulation
/// panel or tab is open) a group with the Probe tool and Run. Returns what was clicked.
pub fn toolbar(ui: &mut egui::Ui, canvas: egui::Rect, sch: &SchPlacer, keymap: &Keymap, sim: bool) -> Option<Pick> {
    let b = |tool, icon, about, action| ToolButton {
        tool: Pick::Tool(tool),
        icon,
        name: SchTool::label(tool),
        about,
        action: Some(action),
    };
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
        b(SchTool::Text, Icon::Text, "Write a free text: a note on the sheet.", Action::SchText),
        b(SchTool::Power, Icon::Power, "Place a power port (GND, +3V3, ...): connects to its net.", Action::SchPower),
        b(SchTool::Junction, Icon::Junction, "Connect crossing wires with a junction dot.", Action::SchJunction),
        b(
            SchTool::NoConnect,
            Icon::NoConnect,
            "Mark a pin as unconnected on purpose with a no-connect flag.",
            Action::SchNoConnect,
        ),
        b(
            SchTool::PowerFlag,
            Icon::PowerFlag,
            "Mark a net as powered from elsewhere (a connector, a supply) with a power flag.",
            Action::SchPowerFlag,
        ),
    ];
    let simulation = [
        b(
            SchTool::Probe,
            Icon::Probe,
            "Click wires and pins to plot their voltages and currents in the simulation.",
            Action::SchProbe,
        ),
        ToolButton {
            tool: Pick::Run,
            icon: Icon::Play,
            name: "Run simulation",
            about: "Run the analysis the Simulation panel shows.",
            action: Some(Action::RunSimulation),
        },
    ];
    let place = [ToolButton {
        tool: Pick::Tool(SchTool::Place),
        icon: Icon::Place,
        name: "Place",
        about: "Placing a part from the library. Esc stops.",
        action: None,
    }];
    let placing: &[ToolButton<Pick>] = if sch.tool() == SchTool::Place { &place } else { &[] };
    let sim: &[ToolButton<Pick>] = if sim || sch.tool() == SchTool::Probe { &simulation } else { &[] };
    let id = egui::Id::unique("sch_toolbar");
    let current = Pick::Tool(sch.tool());
    let (chosen, _) = toolbar::strip(ui, canvas, id, &[&select, &draw, sim, placing], current, keymap);
    toolbar::hint(ui, canvas, sch.tool().hint());
    chosen.filter(|&t| t != Pick::Tool(SchTool::Place))
}

/// Settings of the active tool.
pub fn settings(ui: &mut egui::Ui, sch: &mut SchPlacer, doc: &Document) {
    let tool = sch.tool();
    // The subject line goes on top, drawn last: it shows the help the tool sets below.
    let header = fields::reserve_subject(ui);
    let mut help: Option<String> = None;
    let s = &mut sch.settings;
    match tool {
        SchTool::Select => {}
        SchTool::Probe => {
            help = Some(
                ("Click a wire, label or junction to plot its net's voltage, drag from one net to another for \
             the voltage between them, click the body of a pin to plot the current into it, Alt-click a \
             part for its power. Clicking a probed one removes it. Probes go to the analysis the \
             Simulation panel shows if it's a sweep, else to the first transient, AC or DC sweep.")
                    .to_string(),
            )
        }
        SchTool::Sheet => {
            let name = sch.new_sheet.as_deref().unwrap_or_default();
            help = Some(format!(
                "Click to place the symbol of sheet \"{name}\" (its top left corner). Its size can be changed in Properties."
            ));
        }
        SchTool::Wire => {
            grid(ui, "sch_tool", |ui| fields::corner_rows(ui, &mut s.corners, &mut s.bend, Corners::WIRE));
            help = Some(
                ("Wires end automatically on pins, wires, labels and junctions. / switches the corner, \
                 Shift+Space the corner style.")
                    .to_string(),
            );
        }
        SchTool::Bus => {
            grid(ui, "sch_tool", |ui| fields::corner_rows(ui, &mut s.corners, &mut s.bend, Corners::WIRE));
            help = Some(
                ("A new bus without a name asks for one when you finish it; the name becomes a label on \
                 the bus: D[0..7], {SDA SCL} or I2C{SDA SCL} (signals I2C.SDA, I2C.SCL).")
                    .to_string(),
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
            help = Some(
                "Click on a bus. Wiring an unnamed net to the entry asks which of the bus's signals it is. R rotates."
                    .into(),
            );
        }
        SchTool::Text => {
            grid(ui, "sch_tool", |ui| {
                label(ui, "Size");
                if let (_, Some(v)) = crate::units::drag(ui, s.text_size, model::mm(0.1), model::mm(100.0)) {
                    s.text_size = v;
                }
                ui.end_row();
                label(ui, "Alignment");
                crate::fields::halign_row(ui, &mut s.text_align);
                ui.end_row();
                label(ui, "Rotation");
                if let Some(a) = fields::rotation_combo(ui, "sch_text_rot", s.rotation, LABEL_SIDES) {
                    s.rotation = a;
                }
                ui.end_row();
            });
            help = Some(
                ("Click where the text goes and type. Shift+Enter starts a new line, Enter finishes. R rotates.")
                    .to_string(),
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
                    fields::combo_box(ui, "label_shape").selected_text(s.shape.label()).show_ui(ui, |ui| {
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
            help = Some(
                (match s.label_kind {
                    LabelKind::Local | LabelKind::Power => {
                        "Same-named labels on this sheet are connected; the net is named /NAME. A trailing \
                         number counts up. On a bus, use a bus name like D[0..7]."
                    }
                    LabelKind::Global => "Same-named global labels are connected on every sheet.",
                    LabelKind::Hierarchical => {
                        "Connects to the same-named pin of this sheet's symbol on the parent sheet."
                    }
                })
                .to_string(),
            );
        }
        SchTool::Power => {
            grid(ui, "sch_tool", |ui| {
                label(ui, "Net");
                ui.add(egui::TextEdit::singleline(&mut s.power).desired_width(f32::INFINITY));
                ui.end_row();
                label(ui, "");
                // Explicit rows: a wrapped layout in a grid cell gets the height of one.
                ui.vertical(|ui| {
                    for names in POWER_NAMES.chunks(4) {
                        ui.horizontal(|ui| {
                            for &name in names {
                                if ui.selectable_label(s.power == name, name).clicked() {
                                    s.power = name.to_string();
                                }
                            }
                        });
                    }
                });
                ui.end_row();
                label(ui, "Style");
                if let Some(style) = fields::power_style_combo(ui, "power_style", s.power_style, &s.power) {
                    s.power_style = style;
                }
                ui.end_row();
                label(ui, "Rotation");
                if let Some(a) = fields::rotation_combo(ui, "sch_rot", s.rotation, LABEL_SIDES) {
                    s.rotation = a;
                }
                ui.end_row();
            });
        }
        SchTool::Junction => help = Some("Connects crossing wires. T-connections and pins don't need one.".into()),
        SchTool::NoConnect => {
            help = Some(
                ("Click a pin end that stays unconnected on purpose: it is no longer reported as unconnected. \
             The flag moves and is deleted with its symbol.")
                    .to_string(),
            )
        }
        SchTool::PowerFlag => {
            help = Some(
                ("Click a wire or pin whose net is powered from outside the schematic (a connector, a supply): \
                 its power inputs and inputs count as driven. On a pin end, the flag moves and is deleted with \
                 its symbol.")
                    .to_string(),
            );
            grid(ui, "sch_tool", |ui| {
                label(ui, "Rotation");
                if let Some(a) = fields::rotation_combo(ui, "sch_rot", s.rotation, ["Up", "Left", "Down", "Right"]) {
                    s.rotation = a;
                }
                ui.end_row();
            });
        }
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
            help = Some("Each click places a part; multi-unit parts place their units one after another.".into());
        }
    }
    fields::draw_subject(ui, header, &format!("{} tool", tool.label()), help.as_deref());
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
