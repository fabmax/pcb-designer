//! Board tool bar, tool settings and board setup (layer stack, design rules).

use editor_core::Command;
use editor_core::copper::{CopperItem, Flag};
use editor_core::drc::ViolationKind;
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
    egui::Frame::NONE.fill(theme::panel_dark()).inner_margin(egui::Margin::symmetric(8, 4)).show(ui, |ui| {
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
                0 => RichText::new("Fully routed").color(theme::success()),
                n => RichText::new(format!("{n} unrouted")).color(theme::text_dim()),
            };
            ui.label(text);
            let flagged = copper.flagged().count();
            if flagged > 0 {
                ui.label(RichText::new(format!("{flagged} flagged")).color(theme::warn()))
                    .on_hover_text("Tracks and vias in a short, or touching no pad (see Board problems)");
            }
            if !copper.shorts.is_empty() {
                let tip: Vec<String> = copper.shorts.iter().map(|(nets, _)| nets.join(" ↔ ")).collect();
                ui.label(RichText::new(format!("{} short(s)", copper.shorts.len())).color(theme::warn()))
                    .on_hover_text(tip.join("\n"));
            }
            ui.separator();
            ui.add(egui::Label::new(RichText::new(pcb.tool().hint()).color(theme::text_dim())).truncate());
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
                row(ui, "Clearance", format!("{} mm", to_mm(rules.clearance)));
                row(ui, "Thermal gap", format!("{} mm", to_mm(rules.thermal_gap)));
                row(ui, "Spoke width", format!("{} mm", to_mm(rules.thermal_spoke)));
            });
            ui.add_space(6.0);
            hint(
                ui,
                "Zones fill their outline (inside the board) with copper, keeping the clearance to other nets. \
                 Pads of the zone's net connect through thermal spokes. Islands without the net are removed.",
            );
        }
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

/// Shorts and flagged copper (board properties with nothing selected). Returns objects
/// to select.
pub fn problems(ui: &mut egui::Ui, doc: &Document) -> Option<Vec<ObjectId>> {
    let copper = doc.copper();
    let flagged: Vec<ObjectId> = copper
        .flagged()
        .filter(|c| matches!(c.item, CopperItem::Track(_) | CopperItem::Via(_)))
        .map(|c| c.item.object())
        .collect();
    if flagged.is_empty() && copper.shorts.is_empty() {
        return None;
    }
    let mut select = None;
    ui.add_space(8.0);
    egui::CollapsingHeader::new(RichText::new("Board problems").strong().color(theme::warn())).default_open(true).show(
        ui,
        |ui| {
            for (nets, at) in &copper.shorts {
                ui.label(format!("Short: {} at {:.2}, {:.2} mm", nets.join(" ↔ "), to_mm(at.x), to_mm(at.y)));
            }
            let unconnected = copper.flagged().filter(|c| c.flag == Some(Flag::Unconnected)).count();
            if unconnected > 0 {
                ui.label(format!("{unconnected} track(s)/via(s) touch no pad"));
            }
            if !flagged.is_empty() && ui.button(format!("Select {} flagged", flagged.len())).clicked() {
                select = Some(flagged.clone());
            }
            hint(ui, "Schematic changes never delete copper; it is flagged here instead.");
        },
    );
    select
}

/// Design rule check: run it, list the violations. Returns a violation's objects and
/// position when one is clicked.
pub fn drc(ui: &mut egui::Ui, doc: &mut Document) -> Option<(Vec<ObjectId>, Point)> {
    let mut chosen = None;
    ui.add_space(8.0);
    let title = match doc.drc() {
        Some(r) if r.violations.is_empty() => {
            RichText::new("Design rules: no violations").strong().color(theme::success())
        }
        Some(r) => {
            RichText::new(format!("Design rules: {} violation(s)", r.violations.len())).strong().color(theme::warn())
        }
        None => RichText::new("Design rule check").strong(),
    };
    egui::CollapsingHeader::new(title).id_salt("drc").default_open(true).show(ui, |ui| {
        ui.horizontal(|ui| {
            if ui.button("Run DRC").clicked() {
                doc.run_drc();
            }
            if doc.drc().is_some() && ui.small_button("Clear").clicked() {
                doc.clear_drc();
            }
        });
        let Some(report) = doc.drc() else {
            hint(ui, "Checks clearances, shorts, unrouted nets, the board edge, courtyards, track widths and drills.");
            return;
        };
        let outdated = report.revision != doc.revision;
        let status = format!(
            "{} in {:.1} ms{}",
            if report.violations.is_empty() { "Clean" } else { "Found problems" },
            report.time.as_secs_f64() * 1e3,
            if outdated { " · board changed since" } else { "" }
        );
        ui.label(RichText::new(status).color(if outdated { theme::warn() } else { theme::text_dim() }));
        const SHOWN: usize = 300;
        let mut kind: Option<ViolationKind> = None;
        for v in report.violations.iter().take(SHOWN) {
            if kind != Some(v.kind) {
                kind = Some(v.kind);
                let n = report.violations.iter().filter(|w| w.kind == v.kind).count();
                ui.add_space(4.0);
                ui.label(RichText::new(format!("{} ({n})", v.kind.label())).strong());
            }
            let r = ui.add(egui::Button::selectable(false, &v.message).truncate());
            if r.on_hover_text(&v.message).clicked() {
                chosen = Some((v.objects.clone(), v.pos));
            }
        }
        if report.violations.len() > SHOWN {
            hint(ui, &format!("… and {} more", report.violations.len() - SHOWN));
        }
    });
    chosen
}
