//! The tuner under the Simulation tab's plot: a row per tuned parameter (the setup's
//! `tune`) with a slider, its value, the slider's range and snapping, Apply and remove;
//! "Tune…" adds one. Moving a slider re-runs the tab's analysis with the value as a
//! temporary override (on release, or live when runs are quick); the bar above the rows
//! says what is tuned and applies or resets it all.
//!
//! The values live in [`sim::tune::Tuner`] (in memory); the logic (ranges, snapping,
//! overrides) is `sim::tune`.

use egui::{RichText, Ui};
use model::*;
use view::Document;

use crate::plot_ui::PlotAct;
use crate::sim_ui::SimPanel;
use crate::theme;

/// Height of a row.
const ROW: f32 = 26.0;

/// The height the tuner wants: its header and rows. (It doesn't change with what is
/// tuned: a layout change under a dragged slider would end the drag.)
pub fn height(project: &Project) -> f32 {
    let rows = project.schematic.simulation.tune.len() as f32;
    12.0 + ROW + rows * ROW
}

/// Draws the tuner. Edits of the rows and runs come back as [`PlotAct`]s; slider values
/// change the panel's tuner directly.
pub fn ui(ui: &mut Ui, panel: &mut SimPanel, doc: &Document) -> Option<PlotAct> {
    let project = doc.project();
    let rows = project.schematic.simulation.tune.clone();
    let mut act = None;
    let mut new_rows = rows.clone();
    let live = panel.tune_live(project);
    egui::Frame::new().fill(theme::panel()).inner_margin(egui::Margin { left: 12, right: 12, top: 4, bottom: 8 }).show(
        ui,
        |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new("Tuner").strong());
                if let Some(param) = add_menu(ui, project, &rows)
                    && let Some(row) = sim::tune::default_row(project, &param)
                {
                    new_rows.push(row);
                }
                if rows.is_empty() {
                    ui.label(
                        RichText::new("Try values without changing the schematic: right-click a part → Tune")
                            .color(theme::text_dim())
                            .small(),
                    );
                }
                // What is tuned, applied or reset at once.
                let tuned = panel.tuned(project);
                if !tuned.is_empty() {
                    ui.separator();
                    let text: Vec<String> = tuned
                        .iter()
                        .map(|(p, v)| {
                            format!("{} = {}", short_label(project, p), si::format(*v, sim::tune::unit(project, p)))
                        })
                        .collect();
                    ui.label(RichText::new(format!("Tuned: {}", text.join(" · "))).color(theme::warn()));
                    if ui.small_button("Apply").on_hover_text("Write the tuned values into the schematic").clicked() {
                        act = Some(PlotAct::ApplyTune(None));
                    }
                    if ui.small_button("Reset").on_hover_text("Back to the schematic's values").clicked() {
                        panel.tuner.reset();
                        act = Some(PlotAct::TuneRun);
                    }
                }
            });
            egui::ScrollArea::vertical().id_salt("tuner_rows").auto_shrink([false, true]).show(ui, |ui| {
                for (i, row) in rows.iter().enumerate() {
                    // A stable id per parameter.
                    ui.push_id(("tune_row", &row.param), |ui| {
                        ui.horizontal(|ui| {
                            ui.set_height(ROW - 4.0);
                            match row_ui(ui, panel, project, row, live) {
                                RowAct::None => {}
                                RowAct::Run => act = Some(PlotAct::TuneRun),
                                RowAct::Edit(r) => new_rows[i] = r,
                                RowAct::Apply => act = Some(PlotAct::ApplyTune(Some(row.param.clone()))),
                                RowAct::Remove => {
                                    new_rows.retain(|r| r.param != row.param);
                                }
                            }
                        })
                    });
                }
            });
        },
    );
    if new_rows != rows {
        act = Some(PlotAct::SetTune(new_rows));
    }
    act
}

enum RowAct {
    None,
    /// A value changed: run.
    Run,
    /// The range or snapping changed.
    Edit(TuneRow),
    Apply,
    Remove,
}

/// One row: name, slider, value, min and max, snapping, Apply, ✕.
fn row_ui(ui: &mut Ui, panel: &mut SimPanel, project: &Project, row: &TuneRow, live: bool) -> RowAct {
    let mut out = RowAct::None;
    let param = &row.param;
    let unit = sim::tune::unit(project, param);
    let value = panel.tuner.value(project, param);
    let changed = value.is_some() && value != sim::tune::current(project, param);
    let name = RichText::new(sim::tune::label(project, param));
    let name = if changed { name.color(theme::warn()) } else { name };
    ui.allocate_ui_with_layout(egui::vec2(110.0, ROW - 6.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
        ui.set_min_width(110.0);
        ui.add(egui::Label::new(name).truncate());
    });

    // The fixed parts to the right of the slider: value, min, max, snap, Apply, ✕.
    let right = 92.0 + 2.0 * 72.0 + 70.0 + 50.0 + 24.0 + 6.0 * ui.spacing().item_spacing.x;
    let width = (ui.available_width() - right).max(60.0);
    let mut t = value.map_or(0.0, |v| sim::tune::position(row, v));
    ui.spacing_mut().slider_width = width;
    let r = ui.add(egui::Slider::new(&mut t, 0.0..=1.0).show_value(false));
    if r.changed() {
        let v = sim::tune::at(row, t);
        if Some(v) != value {
            panel.tuner.set(param, v);
            if live || !r.dragged() {
                out = RowAct::Run;
            }
        }
    }
    if r.drag_stopped() {
        out = RowAct::Run;
    }
    let salt = |what: &str| ("tune", what.to_string(), param.clone());
    if let Some(Some(v)) = panel.si.field_sized(ui, salt("value"), value, unit, "", 92.0) {
        panel.tuner.set(param, v);
        out = RowAct::Run;
    }
    let mut edit = row.clone();
    if let Some(Some(v)) = panel.si.field_sized(ui, salt("min"), Some(row.min), unit, "", 72.0) {
        edit.min = v;
    }
    if let Some(Some(v)) = panel.si.field_sized(ui, salt("max"), Some(row.max), unit, "", 72.0) {
        edit.max = v;
    }
    let snaps: Vec<(Snap, String)> = Snap::ALL.iter().map(|s| (*s, s.label().to_string())).collect();
    egui::ComboBox::from_id_salt(salt("snap")).width(56.0).selected_text(row.snap.label()).show_ui(ui, |ui| {
        for (s, label) in &snaps {
            if ui.selectable_label(*s == row.snap, label).clicked() {
                edit.snap = *s;
            }
        }
    });
    // A range the setup refuses (min ≥ max, ≤ 0 on a log slider) isn't taken.
    let valid =
        edit.min.is_finite() && edit.max.is_finite() && edit.min < edit.max && (!param.is_log() || edit.min > 0.0);
    if edit != *row && valid {
        out = RowAct::Edit(edit);
    }
    if ui
        .add_enabled(changed, egui::Button::new("Apply").small())
        .on_hover_text("Write it into the schematic")
        .clicked()
    {
        out = RowAct::Apply;
    }
    if ui.small_button("✕").on_hover_text("Remove from the tuner").clicked() {
        out = RowAct::Remove;
    }
    out
}

/// "R1", "V1.amplitude": the bar's names.
fn short_label(project: &Project, param: &TuneParam) -> String {
    let refdes = project.schematic.components.get(param.component()).map_or("?", |c| c.refdes.as_str());
    match param {
        TuneParam::Value { .. } => refdes.to_string(),
        TuneParam::Source { field, .. } => format!("{refdes}.{field}"),
    }
}

/// "Tune…": the parameters not in the tuner yet, with a filter.
fn add_menu(ui: &mut Ui, project: &Project, rows: &[TuneRow]) -> Option<TuneParam> {
    let mut picked = None;
    ui.menu_button("Tune…", |ui| {
        let id = egui::Id::unique("tune_filter");
        let mut filter = ui.data_mut(|d| d.get_temp::<String>(id).unwrap_or_default());
        let r = ui.add(egui::TextEdit::singleline(&mut filter).hint_text("Filter").desired_width(180.0));
        if !r.has_focus() && filter.is_empty() {
            r.request_focus();
        }
        let f = filter.to_lowercase();
        let options: Vec<TuneParam> = sim::tune::tunable(project)
            .into_iter()
            .filter(|p| !rows.iter().any(|r| &r.param == p))
            .filter(|p| f.is_empty() || sim::tune::label(project, p).to_lowercase().contains(&f))
            .collect();
        egui::ScrollArea::vertical().max_height(300.0).show(ui, |ui| {
            if options.is_empty() {
                ui.label(RichText::new("Nothing to tune").color(theme::text_dim()));
            }
            for p in options {
                let current = sim::tune::current(project, &p)
                    .map_or(String::new(), |v| si::format(v, sim::tune::unit(project, &p)));
                let text = format!("{}  {current}", sim::tune::label(project, &p));
                if ui.button(text).clicked() {
                    picked = Some(p);
                    ui.close();
                }
            }
        });
        ui.data_mut(|d| d.insert_temp(id, filter));
    });
    picked
}
