//! Board tool bar, tool settings and board setup (layer stack, design rules).

use editor_core::Command;

use egui::RichText;
use model::*;
use view::Document;
use view::interaction::new_group;
use view::pcb_tools::{Corners, PcbPlacer, PcbTool};

use crate::fields::{self, grid, hint, label, row};
use crate::icons::Icon;
use crate::keymap::{Action, Keymap};
use crate::theme;
use crate::toolbar::{self, ToolButton};
use crate::units;

/// Floating tool bar, header (active layer, routing status) and usage hint over the
/// board canvas. Returns a newly chosen tool.
pub fn toolbar(
    ui: &mut egui::Ui,
    canvas: egui::Rect,
    pcb: &mut PcbPlacer,
    doc: &Document,
    keymap: &Keymap,
) -> Option<PcbTool> {
    let b =
        |tool, icon, about, action| ToolButton { tool, icon, name: PcbTool::label(tool), about, action: Some(action) };
    let select = [b(
        PcbTool::Select,
        Icon::Select,
        "Select, move and edit footprints, tracks and vias. Drag a box to select several.",
        Action::ToolSelect,
    )];
    let copper = [
        b(PcbTool::Route, Icon::Route, "Route tracks from pad to pad on the active layer.", Action::PcbRoute),
        b(PcbTool::Via, Icon::Via, "Place a via between the copper layers.", Action::PcbVia),
    ];
    let board = [
        b(PcbTool::Outline, Icon::Outline, "Draw the board outline.", Action::PcbOutline),
        b(PcbTool::Zone, Icon::Zone, "Draw a copper zone, filled around other nets.", Action::PcbZone),
        b(
            PcbTool::Keepout,
            Icon::Keepout,
            "Draw a keepout: an area zone fills leave out and where DRC forbids tracks, vias or pads.",
            Action::PcbKeepout,
        ),
        b(
            PcbTool::Cutout,
            Icon::Cutout,
            "Cut an area out of a zone or keepout: start inside it, click the corners, close the polygon.",
            Action::PcbCutout,
        ),
    ];
    let measure = [
        b(PcbTool::Text, Icon::Text, "Write a text on a silkscreen, copper or other layer.", Action::PcbText),
        b(
            PcbTool::Measure,
            Icon::Measure,
            "Measure distances between two points, or clearances between copper, holes and the board edge (E).",
            Action::ToolMeasure,
        ),
    ];
    let place = [ToolButton {
        tool: PcbTool::Place,
        icon: Icon::Place,
        name: "Place",
        about: "Placing a board part from the library. Esc stops.",
        action: None,
    }];
    let placing: &[ToolButton<PcbTool>] = if pcb.tool() == PcbTool::Place { &place } else { &[] };
    let id = egui::Id::unique("pcb_toolbar");
    let groups: [&[ToolButton<PcbTool>]; 5] = [&select, &copper, &board, &measure, placing];
    let (chosen, right) = toolbar::strip(ui, canvas, id, &groups, pcb.tool(), keymap);
    toolbar::header(ui, canvas, right, id.with("header"), |ui| {
        layer_combo(ui, "pcb_layer", &mut pcb.settings.layer, doc)
            .on_hover_text(format!("Active copper layer ({})", keymap.menu_text(ui.ctx(), Action::PcbNextLayer)));
        ui.separator();
        let copper = doc.copper();
        let text = match copper.unrouted() {
            0 => RichText::new("Fully routed").color(theme::success()),
            n => RichText::new(format!("{n} unrouted")).color(theme::text_dim()),
        };
        ui.label(text).on_hover_text("Connections still to route (ratsnest lines)");
        let flagged = copper.flagged().count();
        if flagged > 0 {
            ui.label(RichText::new(format!("{flagged} flagged")).color(theme::warn()))
                .on_hover_text("Tracks and vias in a short, or touching no pad (see Board problems)");
        }
        if !copper.shorts.is_empty() {
            let tip: Vec<String> = copper.shorts.iter().map(|(nets, _)| nets.join(", ")).collect();
            ui.label(RichText::new(format!("{} short(s)", copper.shorts.len())).color(theme::warn()))
                .on_hover_text(tip.join("\n"));
        }
    });
    match pcb.clash() {
        Some(c) if !pcb.allows_violations() => toolbar::hint_in(
            ui,
            canvas,
            &format!("Can't add here: {} · Shift+click adds it anyway", c.message),
            crate::theme::warn(),
        ),
        Some(c) => toolbar::hint_in(ui, canvas, &format!("Violation: {}", c.message), crate::theme::warn()),
        None if pcb.tool() == PcbTool::Measure && pcb.settings.clearance => {
            toolbar::hint(ui, canvas, view::measure::CLEARANCE_HINT)
        }
        None => toolbar::hint(ui, canvas, pcb.tool().hint()),
    }
    chosen.filter(|&t| t != PcbTool::Place)
}

fn layer_combo(ui: &mut egui::Ui, salt: &str, layer: &mut LayerId, doc: &Document) -> egui::Response {
    let board = &doc.project().board;
    let name = |id: LayerId| board.layers.iter().find(|l| l.id == id).map_or(format!("{id:?}"), |l| l.name.clone());
    egui::ComboBox::from_id_salt(salt)
        .selected_text(name(*layer))
        .show_ui(ui, |ui| {
            for l in board.copper_layers() {
                ui.selectable_value(layer, l.id, &l.name);
            }
        })
        .response
}

/// Settings of the active tool.
pub fn settings(ui: &mut egui::Ui, pcb: &mut PcbPlacer, doc: &Document) {
    let tool = pcb.tool();
    fields::heading(ui, &format!("{} tool", tool.label()), "");
    ui.add_space(4.0);
    let rules = doc.project().board.rules.clone();
    let s = &mut pcb.settings;
    match tool {
        PcbTool::Select => {}
        PcbTool::Route => {
            grid(ui, "pcb_tool", |ui| {
                label(ui, "Layer");
                layer_combo(ui, "pcb_tool_layer", &mut s.layer, doc);
                ui.end_row();
                label(ui, "Width");
                let w = s.width.unwrap_or(rules.track_width);
                if let (_, Some(w)) = units::drag(ui, w, rules.min_track_width, mm(10.0)) {
                    s.width = Some(w);
                }
                ui.end_row();
                fields::corner_rows(ui, &mut s.corners, &mut s.bend, Corners::TRACK);
                label(ui, "Corner radius");
                if let (_, Some(r)) = units::drag(ui, s.corner_radius, mm(0.01), mm(100.0)) {
                    s.corner_radius = r;
                }
                ui.end_row();
                if doc.project().board.copper_layers().count() > 2 {
                    label(ui, "Via span");
                    ui.horizontal(|ui| {
                        ui.selectable_value(&mut s.via_pair, false, "Through")
                            .on_hover_text("L adds vias through all layers");
                        ui.selectable_value(&mut s.via_pair, true, "Two layers")
                            .on_hover_text("L adds blind or buried vias joining just the two layers");
                    });
                    ui.end_row();
                }
            });
            ui.checkbox(&mut s.allow_violations, "Allow violations").on_hover_text(
                "Add tracks and vias that break the clearance, keepout or board edge rules \
                     (Shift+click does it once); DRC flags them",
            );
            let classes = !rules.net_classes.is_empty();
            if s.width.is_some() {
                let text = if classes { "Use the net class width" } else { "Use the design rule width" };
                if ui.small_button(text).clicked() {
                    s.width = None;
                }
            } else if classes {
                hint(ui, "Tracks take the width of their net's class.");
            }
            ui.add_space(6.0);
            hint(
                ui,
                "Tracks start and end on pad centers, vias and track ends. The net being routed is highlighted. \
                 The Edit menu's Round track corners rounds the selected tracks with the corner radius.",
            );
        }
        PcbTool::Via => {
            let layers = doc.project().board.copper_layers().count();
            grid(ui, "pcb_tool", |ui| {
                if rules.net_classes.is_empty() {
                    row(ui, "Diameter", units::fmt(rules.via_diameter));
                    row(ui, "Drill", units::fmt(rules.via_drill));
                } else {
                    row(ui, "Size", "by net class".into());
                }
                if layers > 2 {
                    label(ui, "From layer");
                    layer_combo(ui, "via_from", &mut s.via_span[0], doc);
                    ui.end_row();
                    label(ui, "To layer");
                    layer_combo(ui, "via_to", &mut s.via_span[1], doc);
                    ui.end_row();
                }
            });
            ui.checkbox(&mut s.allow_violations, "Allow violations").on_hover_text(
                "Place vias that break the clearance, keepout or board edge rules \
                     (Shift+click does it once); DRC flags them",
            );
            ui.add_space(6.0);
            hint(
                ui,
                if layers > 2 {
                    "Vias join the layers from–to (blind or buried), or all layers (through). Sizes come from the design rules."
                } else {
                    "Through vias connect all copper layers. Sizes come from the design rules."
                },
            );
        }
        PcbTool::Text => {
            grid(ui, "pcb_tool", |ui| {
                label(ui, "Layer");
                let board = &doc.project().board;
                let name = |id: LayerId| {
                    board.layers.iter().find(|l| l.id == id).map_or(format!("{id:?}"), |l| l.name.clone())
                };
                egui::ComboBox::from_id_salt("pcb_text_layer").selected_text(name(s.text_layer)).show_ui(ui, |ui| {
                    for l in board.layers.iter().filter(|l| l.id != LayerId::OUTLINE) {
                        ui.selectable_value(&mut s.text_layer, l.id, &l.name);
                    }
                });
                ui.end_row();
                label(ui, "Size");
                if let (_, Some(v)) = units::drag(ui, s.text_size, mm(0.1), mm(100.0)) {
                    s.text_size = v;
                }
                ui.end_row();
                label(ui, "Bold");
                ui.checkbox(&mut s.text_bold, "").on_hover_text("Bold strokes fabricate better on silkscreen");
                ui.end_row();
                label(ui, "Alignment");
                fields::halign_row(ui, &mut s.text_align);
                ui.end_row();
                label(ui, "Rotation");
                if let Some(a) = fields::rotation_combo(ui, "pcb_text_rot", s.rotation, ["0°", "90°", "180°", "270°"])
                {
                    s.rotation = a;
                }
                ui.end_row();
            });
            ui.add_space(6.0);
            hint(
                ui,
                "Click where the text goes and type. Shift+Enter starts a new line, Enter finishes. R rotates, \
                 F moves it to the other side. Texts on copper have no net.",
            );
        }
        PcbTool::Outline => hint(ui, "Click two corners. The outline is a graphic on the Board Outline layer."),
        PcbTool::Place => {
            let Some(part) = pcb.placing.and_then(|p| doc.project().library.parts.get(p)) else { return };
            grid(ui, "pcb_tool", |ui| {
                row(ui, "Part", part.name.clone());
                label(ui, "Rotation");
                if let Some(a) = fields::rotation_combo(ui, "pcb_place_rot", s.rotation, ["0°", "90°", "180°", "270°"])
                {
                    s.rotation = a;
                }
                ui.end_row();
                label(ui, "Side");
                egui::ComboBox::from_id_salt("pcb_place_side").selected_text(format!("{:?}", s.side)).show_ui(
                    ui,
                    |ui| {
                        ui.selectable_value(&mut s.side, Side::Top, "Top");
                        ui.selectable_value(&mut s.side, Side::Bottom, "Bottom");
                    },
                );
                ui.end_row();
            });
            ui.add_space(6.0);
            hint(ui, "A board part has no symbol: it lives on the board only, and its pads have no net.");
        }
        PcbTool::Measure => {
            let mut toggle = false;
            grid(ui, "pcb_tool", |ui| {
                label(ui, "Measure");
                ui.horizontal(|ui| {
                    let mut c = s.clearance;
                    ui.selectable_value(&mut c, false, "Points").on_hover_text("The distance between two points");
                    ui.selectable_value(&mut c, true, "Clearance")
                        .on_hover_text("The gap between two objects' edges: copper, drill holes, the board edge");
                    toggle = c != s.clearance;
                });
                ui.end_row();
                if s.clearance {
                    label(ui, "Layer");
                    layer_combo(ui, "pcb_tool_layer", &mut s.layer, doc);
                    ui.end_row();
                }
            });
            if toggle {
                pcb.toggle_clearance();
            }
            ui.add_space(6.0);
            hint(
                ui,
                if pcb.settings.clearance {
                    "Click two objects: copper on the active layer, a drill hole or the board edge. The gap \
                     between their edges shows with the clearance the design rules ask for (red when it's \
                     less). E switches to points, Esc clears."
                } else {
                    "Click a start point, then an end point. Ends snap to pad and via centres (from anywhere \
                     inside them), track ends and outline corners, else to the grid; hold Alt for a free \
                     point. The next click starts a new measurement, E measures clearances, Esc clears."
                },
            );
        }
        PcbTool::Cutout => {
            grid(ui, "pcb_tool", |ui| {
                label(ui, "Layer");
                layer_combo(ui, "pcb_tool_layer", &mut s.layer, doc);
                ui.end_row();
            });
            ui.add_space(6.0);
            hint(
                ui,
                "Start inside a zone or keepout on the active layer and click the cutout's corners. Inside, \
                 it becomes a hole (merged with holes it overlaps); across the edge, it trims the outline. \
                 A cut that would split the area in two is refused.",
            );
        }
        PcbTool::Zone => {
            grid(ui, "pcb_tool", |ui| {
                label(ui, "Layer");
                layer_combo(ui, "pcb_tool_layer", &mut s.layer, doc);
                ui.end_row();
                label(ui, "Net");
                let text = if s.zone_net.is_empty() { "From the first corner" } else { s.zone_net.as_str() };
                egui::ComboBox::from_id_salt("zone_net").selected_text(text).show_ui(ui, |ui| {
                    ui.selectable_value(&mut s.zone_net, String::new(), "From the first corner");
                    for n in &doc.copper().nets {
                        ui.selectable_value(&mut s.zone_net, n.clone(), n);
                    }
                });
                ui.end_row();
                row(ui, "Clearance", units::fmt(rules.clearance));
                row(ui, "Thermal gap", units::fmt(rules.thermal_gap));
                row(ui, "Spoke width", units::fmt(rules.thermal_spoke));
            });
            ui.add_space(6.0);
            hint(
                ui,
                "Zones fill their outline (inside the board) with copper, keeping the clearance to other nets. \
                 Pads of the zone's net connect through thermal spokes. Islands without the net are removed. \
                 Properties change how pads connect, the thermal sizes and which islands stay.",
            );
        }
        PcbTool::Keepout => {
            grid(ui, "pcb_tool", |ui| {
                label(ui, "Layers");
                ui.checkbox(&mut s.keepout_all_layers, "All copper layers");
                ui.end_row();
                if !s.keepout_all_layers {
                    label(ui, "Layer");
                    layer_combo(ui, "pcb_tool_layer", &mut s.layer, doc);
                    ui.end_row();
                }
            });
            ui.add_space(6.0);
            hint(
                ui,
                "A keepout forbids tracks, vias, pads and zone fills inside it; change what it forbids in \
                 Properties. Zone fills leave it out, DRC reports the rest.",
            );
        }
    }
    ui.add_space(6.0);
    hint(ui, "Esc ends the tool.");
}

const LAYER_COUNTS: &[u16] = &[2, 4, 6, 8];

/// Layer stack and design rules (Board setup panel).
pub fn board_setup(ui: &mut egui::Ui, doc: &mut Document, message: &mut Option<String>) {
    {
        let board = &doc.project().board;
        let count = board.copper_layers().count() as u16;
        let mut rules = board.rules.clone();
        let mut command = None;
        grid(ui, "board_setup", |ui| {
            label(ui, "Copper layers");
            egui::ComboBox::from_id_salt("copper_layers").selected_text(count.to_string()).show_ui(ui, |ui| {
                for &n in LAYER_COUNTS {
                    if ui.selectable_label(n == count, n.to_string()).clicked() && n != count {
                        command = Some(Command::SetCopperLayers { count: n });
                    }
                }
            });
            ui.end_row();
            let mut changed = false;
            let mut active = false;
            for (name, value, min, max) in [
                ("Track width", &mut rules.track_width, 0.05, 20.0),
                ("Clearance", &mut rules.clearance, 0.05, 20.0),
                ("Via diameter", &mut rules.via_diameter, 0.2, 20.0),
                ("Via drill", &mut rules.via_drill, 0.1, 20.0),
                ("Min. track width", &mut rules.min_track_width, 0.05, 20.0),
                ("Min. drill", &mut rules.min_drill, 0.1, 20.0),
                ("Mask expansion", &mut rules.mask_expansion, 0.0, 1.0),
                ("Paste margin", &mut rules.paste_margin, -1.0, 1.0),
            ] {
                label(ui, name);
                let (r, v) = units::drag(ui, *value, mm(min), mm(max));
                if let Some(v) = v {
                    *value = v;
                    changed = true;
                }
                active |= r.dragged() || r.has_focus();
                ui.end_row();
            }
            // Live while dragging or typing; one undo step per edit.
            let key = ui.make_persistent_id("design_rules_group");
            if changed && command.is_none() {
                let group = ui.data_mut(|d| *d.get_temp_mut_or_insert_with(key, new_group));
                if let Err(e) = doc.execute_grouped(Command::SetDesignRules { rules: rules.clone() }, Some(group)) {
                    *message = Some(e.to_string());
                }
            }
            if !active && ui.data_mut(|d| d.remove_temp::<u64>(key)).is_some() {
                doc.close_group();
            }
        });
        if let Some(cmd) = command
            && let Err(e) = doc.execute(cmd)
        {
            *message = Some(e.to_string());
        }
    }
    ui.add_space(6.0);
    hint(
        ui,
        "Applies to the whole board, from any tab. Removing an inner layer needs it to be empty. \
         Pads open the solder mask, grown by the mask expansion (vias stay covered); SMD pads get paste, \
         grown by the paste margin (negative shrinks).",
    );
    ui.add_space(12.0);
    net_classes(ui, doc, message);
    ui.add_space(12.0);
    crate::checks_ui::drc_severities(ui, doc, message);
}

/// The net classes: a card per class with its sizes (empty: the board's) and nets.
fn net_classes(ui: &mut egui::Ui, doc: &mut Document, message: &mut Option<String>) {
    let mut rules = doc.project().board.rules.clone();
    let nets: Vec<String> = doc.connectivity().netlist().map(|n| n.name.clone()).collect();
    ui.label(RichText::new("Net classes").strong());
    ui.add_space(2.0);
    let mut changed = false;
    let mut active = false;
    let mut remove = None;
    let board = [rules.clearance, rules.track_width, rules.via_diameter, rules.via_drill];
    for (i, class) in rules.net_classes.iter_mut().enumerate() {
        egui::Frame::group(ui.style()).corner_radius(6.0).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                let id = ui.make_persistent_id(("net_class_name", i));
                if let Some(name) = committed_text(ui, id, &class.name, 140.0) {
                    class.name = name.trim().to_string();
                    changed = true;
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.small_button("Remove").clicked() {
                        remove = Some(i);
                    }
                });
            });
            grid(ui, &format!("net_class_{i}"), |ui| {
                for (k, (name, value)) in [
                    ("Clearance", &mut class.clearance),
                    ("Track width", &mut class.track_width),
                    ("Via diameter", &mut class.via_diameter),
                    ("Via drill", &mut class.via_drill),
                ]
                .into_iter()
                .enumerate()
                {
                    label(ui, name);
                    ui.horizontal(|ui| {
                        let (r, v) = units::drag(ui, value.unwrap_or(board[k]), mm(0.05), mm(20.0));
                        if let Some(v) = v {
                            *value = Some(v);
                            changed = true;
                        }
                        active |= r.dragged() || r.has_focus();
                        if value.is_some() {
                            let tip = format!("Back to the board's value ({})", units::fmt(board[k]));
                            if crate::project_panel::header_button(ui, Icon::Reset, &tip, false).clicked() {
                                *value = None;
                                changed = true;
                            }
                        } else {
                            ui.label(RichText::new("default").color(theme::text_dim()))
                                .on_hover_text("The Default class's value (above)");
                        }
                    });
                    ui.end_row();
                }
                label(ui, "Nets");
                let id = ui.make_persistent_id(("net_class_nets", i));
                if let Some(text) = committed_text(ui, id, &class.nets.join(", "), ui.available_width()) {
                    class.nets = text.split([',', ' ']).filter(|p| !p.is_empty()).map(str::to_string).collect();
                    changed = true;
                }
                ui.end_row();
            });
            let count = nets.iter().filter(|n| class.matches(n)).count();
            let members = match count {
                0 => "no nets yet".to_string(),
                1 => "1 net".to_string(),
                n => format!("{n} nets"),
            };
            hint(ui, &members);
        });
        ui.add_space(4.0);
    }
    if let Some(i) = remove {
        rules.net_classes.remove(i);
        changed = true;
    }
    if ui.button("Add net class").clicked() {
        let name = (1..).map(|n| format!("Class {n}")).find(|n| !rules.net_classes.iter().any(|c| &c.name == n));
        rules.net_classes.push(NetClass { name: name.expect("a free name"), ..Default::default() });
        changed = true;
    }
    // Live while dragging or typing; one undo step per edit.
    let key = ui.make_persistent_id("net_classes_group");
    if changed {
        let group = ui.data_mut(|d| *d.get_temp_mut_or_insert_with(key, new_group));
        if let Err(e) = doc.execute_grouped(Command::SetDesignRules { rules }, Some(group)) {
            *message = Some(e.to_string());
        }
    }
    if !active && ui.data_mut(|d| d.remove_temp::<u64>(key)).is_some() {
        doc.close_group();
    }
    ui.add_space(2.0);
    hint(
        ui,
        "Nets by name or by a pattern (* any characters, ? one), separated by commas; a name beats a \
         pattern, else the first class matching wins. Other nets are in the Default class (the values \
         above). Between two classes the larger clearance applies. The Nets panel's context menu \
         puts a net into a class.",
    );
}

/// A text field that hands back its text when editing ends (Enter or focus lost) with a
/// change; while it has focus it keeps its own text.
fn committed_text(ui: &mut egui::Ui, id: egui::Id, current: &str, width: f32) -> Option<String> {
    let mut text = ui.data_mut(|d| d.get_temp::<String>(id)).unwrap_or_else(|| current.to_string());
    let r = ui.add(egui::TextEdit::singleline(&mut text).id(id).desired_width(width));
    if r.has_focus() {
        ui.data_mut(|d| d.insert_temp(id, text.clone()));
        None
    } else {
        ui.data_mut(|d| d.remove_temp::<String>(id));
        (r.lost_focus() && text != current).then_some(text)
    }
}
