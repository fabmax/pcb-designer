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
    ];
    let measure = [
        b(PcbTool::Text, Icon::Text, "Write a text on a silkscreen, copper or other layer.", Action::PcbText),
        b(PcbTool::Measure, Icon::Measure, "Measure distances between two points.", Action::ToolMeasure),
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
        Some(c) if !pcb.settings.allow_violations => {
            toolbar::hint_in(ui, canvas, &format!("Can't add here: {}", c.message), crate::theme::warn())
        }
        Some(c) => toolbar::hint_in(ui, canvas, &format!("Violation: {}", c.message), crate::theme::warn()),
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
            ui.checkbox(&mut s.allow_violations, "Allow violations")
                .on_hover_text("Add tracks and vias that break the clearance, keepout or board edge rules");
            if s.width.is_some() && ui.small_button("Use the design rule width").clicked() {
                s.width = None;
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
                row(ui, "Diameter", units::fmt(rules.via_diameter));
                row(ui, "Drill", units::fmt(rules.via_drill));
                if layers > 2 {
                    label(ui, "From layer");
                    layer_combo(ui, "via_from", &mut s.via_span[0], doc);
                    ui.end_row();
                    label(ui, "To layer");
                    layer_combo(ui, "via_to", &mut s.via_span[1], doc);
                    ui.end_row();
                }
            });
            ui.checkbox(&mut s.allow_violations, "Allow violations")
                .on_hover_text("Place vias that break the clearance, keepout or board edge rules");
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
                label(ui, "Stroke width");
                if let (_, Some(v)) = units::drag(ui, s.text_width, mm(0.01), mm(10.0)) {
                    s.text_width = v;
                }
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
        PcbTool::Measure => hint(
            ui,
            "Click a start point, then an end point. Ends snap to pad and via centres, track ends and \
             outline corners, else to the grid; hold Alt for a free point. The next click starts a new \
             measurement, Esc clears it.",
        ),
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
}
