//! Board tool bar, tool settings and board setup (layer stack, design rules).

use editor_core::Command;
use egui::RichText;
use model::*;
use view::Document;
use view::interaction::new_group;
use view::pcb_tools::{Corners, PcbPlacer, PcbTool};

use crate::fields::{self, grid, hint, label, row};
use crate::theme;

/// Tool buttons, active layer and usage hint above the board. Returns a newly chosen tool.
pub fn toolbar(ui: &mut egui::Ui, pcb: &mut PcbPlacer, doc: &Document) -> Option<PcbTool> {
    let mut chosen = None;
    let full = ui.max_rect();
    egui::Frame::NONE.fill(theme::PANEL_DARK).inner_margin(egui::Margin::symmetric(8, 4)).show(ui, |ui| {
        ui.set_width(full.width() - 16.0);
        ui.horizontal(|ui| {
            for &tool in PcbTool::ALL {
                if ui.selectable_label(pcb.tool() == tool, tool.label()).clicked() {
                    chosen = Some(tool);
                }
            }
            ui.separator();
            layer_combo(ui, "pcb_layer", &mut pcb.settings.layer, doc);
            ui.separator();
            let copper = doc.copper();
            let text = match copper.unrouted() {
                0 => RichText::new("Fully routed").color(theme::SUCCESS),
                n => RichText::new(format!("{n} unrouted")).color(theme::TEXT_DIM),
            };
            ui.label(text);
            if !copper.shorts.is_empty() {
                let tip: Vec<String> = copper.shorts.iter().map(|(nets, _)| nets.join(" ↔ ")).collect();
                ui.label(RichText::new(format!("{} short(s)", copper.shorts.len())).color(theme::WARN))
                    .on_hover_text(tip.join("\n"));
            }
            ui.separator();
            ui.add(egui::Label::new(RichText::new(pcb.tool().hint()).color(theme::TEXT_DIM)).truncate());
        });
    });
    chosen
}

fn layer_combo(ui: &mut egui::Ui, salt: &str, layer: &mut LayerId, doc: &Document) {
    let board = &doc.project().board;
    let name = |id: LayerId| board.layers.iter().find(|l| l.id == id).map_or(format!("{id:?}"), |l| l.name.clone());
    egui::ComboBox::from_id_salt(salt).selected_text(name(*layer)).show_ui(ui, |ui| {
        for l in board.copper_layers() {
            ui.selectable_value(layer, l.id, &l.name);
        }
    });
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
                let mut w = to_mm(s.width.unwrap_or(rules.track_width));
                let min = to_mm(rules.min_track_width);
                let r =
                    ui.add(egui::DragValue::new(&mut w).speed(0.01).range(min..=10.0).suffix(" mm").max_decimals(3));
                if r.changed() {
                    s.width = Some(mm(w));
                }
                ui.end_row();
                label(ui, "Corners");
                ui.horizontal(|ui| {
                    ui.selectable_value(&mut s.corners, Corners::Deg45, "45°");
                    ui.selectable_value(&mut s.corners, Corners::Deg90, "90°");
                });
                ui.end_row();
                label(ui, "Bend");
                ui.horizontal(|ui| {
                    let (first, second) = match s.corners {
                        Corners::Deg45 => ("Straight first", "Diagonal first"),
                        Corners::Deg90 => ("Horizontal first", "Vertical first"),
                    };
                    ui.selectable_value(&mut s.bend_first, false, first);
                    ui.selectable_value(&mut s.bend_first, true, second);
                });
                ui.end_row();
            });
            if s.width.is_some() && ui.small_button("Use the design rule width").clicked() {
                s.width = None;
            }
            ui.add_space(6.0);
            hint(ui, "Tracks start and end on pad centers, vias and track ends. The net being routed is highlighted.");
        }
        PcbTool::Via => {
            grid(ui, "pcb_tool", |ui| {
                row(ui, "Diameter", format!("{} mm", to_mm(rules.via_diameter)));
                row(ui, "Drill", format!("{} mm", to_mm(rules.via_drill)));
            });
            ui.add_space(6.0);
            hint(ui, "Through vias connect all copper layers. Sizes come from the design rules.");
        }
        PcbTool::Outline => hint(ui, "Click two corners. The outline is a graphic on the Board Outline layer."),
    }
    ui.add_space(6.0);
    hint(ui, "Esc ends the tool.");
}

const LAYER_COUNTS: &[u16] = &[2, 4, 6, 8];

/// Layer stack and design rules (board properties with nothing selected).
pub fn board_setup(ui: &mut egui::Ui, doc: &mut Document, message: &mut Option<String>) {
    ui.add_space(8.0);
    egui::CollapsingHeader::new(RichText::new("Board setup").strong()).default_open(true).show(ui, |ui| {
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
            for (name, value, min) in [
                ("Track width", &mut rules.track_width, 0.05),
                ("Clearance", &mut rules.clearance, 0.05),
                ("Via diameter", &mut rules.via_diameter, 0.2),
                ("Via drill", &mut rules.via_drill, 0.1),
                ("Min. track width", &mut rules.min_track_width, 0.05),
                ("Min. drill", &mut rules.min_drill, 0.1),
            ] {
                label(ui, name);
                let mut v = to_mm(*value);
                let r =
                    ui.add(egui::DragValue::new(&mut v).speed(0.01).range(min..=20.0).suffix(" mm").max_decimals(3));
                if r.changed() {
                    *value = mm(v);
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
    });
}
