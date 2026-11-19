//! The Simulation panel: the analyses of the setup as tabs with their settings, Run and
//! Stop, the last result of the analysis shown as a table, and the circuit settings
//! (ground, supplies, temperature, extra SPICE lines). Also the SPICE netlist and log
//! window.
//!
//! Runs go through [`sim::Session`] (one at a time, on a worker thread). Every setup edit
//! is one `SetSimulation` command.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use editor_core::Command;
use egui::{RichText, Ui};
use model::*;
use rustc_hash::FxHashMap;
use sim::{Attempt, Level, Outcome, Session};
use view::Document;

use crate::fields::{grid, hint, label};
use crate::icons::{self, Icon};
use crate::theme;

/// What the panel asks the app to do.
pub enum SimAct {
    Run(Id),
    Stop,
    ShowNetlist,
    ShowLog,
    OpenSettings,
    /// Open the Simulation tab.
    ShowPlot,
    /// Messages, on the Simulation tab.
    ShowMessages,
    SelectNet(String),
}

/// ngspice as found: its version, or why not.
type Detected = Result<(sim::Ngspice, Option<String>), String>;

#[derive(Default)]
pub struct SimPanel {
    pub session: Session,
    /// The analysis tab shown.
    tab: Option<Id>,
    /// Text being typed into value fields, and those that didn't parse.
    texts: FxHashMap<egui::Id, String>,
    invalid: HashSet<egui::Id>,
    filter: String,
    show_all: bool,
    ngspice: Option<Detected>,
    /// The netlist window: shown, on the log tab.
    pub window: bool,
    window_log: bool,
    /// The current netlist of the shown analysis: (revision, analysis, text).
    netlist: Option<(u64, Id, String)>,
    /// The plots' views, per analysis.
    pub plots: std::collections::HashMap<Id, crate::plot_ui::PlotState>,
    /// AC plots leave out the phase.
    pub no_phase: bool,
    /// The user closed the plot tab: runs don't open it again (until opened by hand).
    pub plot_closed: bool,
}

/// The kinds of analyses, in the "+" menu's order.
const KINDS: [&str; 4] = ["Operating point", "Transient", "AC", "DC sweep"];

impl SimPanel {
    /// A new project: no results, nothing selected.
    pub fn clear(&mut self) {
        self.session.clear();
        self.tab = None;
        self.texts.clear();
        self.invalid.clear();
        self.netlist = None;
        self.plots.clear();
    }

    /// Looks for ngspice again (the setting changed).
    pub fn forget_ngspice(&mut self) {
        self.ngspice = None;
    }

    /// ngspice, found once (at `configured`, or the usual places).
    pub fn ngspice(&mut self, configured: Option<PathBuf>) -> &Detected {
        self.ngspice.get_or_insert_with(|| match sim::Ngspice::find(configured.as_deref()) {
            Ok(n) => {
                let v = n.version();
                Ok((n, v))
            }
            Err(e) => Err(e.to_string()),
        })
    }

    /// The analysis shown (the tab picked, else the first).
    pub fn current<'a>(&self, setup: &'a Simulation) -> Option<&'a Analysis> {
        self.tab.and_then(|id| setup.analyses.iter().find(|a| a.id == id)).or(setup.analyses.first())
    }

    pub fn show_tab(&mut self, id: Id) {
        self.tab = Some(id);
    }

    /// Starts `analysis`. Returns an error to show when ngspice is missing or the netlist
    /// has errors.
    pub fn run(&mut self, doc: &mut Document, analysis: Id, configured: Option<PathBuf>) -> Result<(), String> {
        let ngspice = match self.ngspice(configured) {
            Ok((n, _)) => n.clone(),
            Err(e) => return Err(e.clone()),
        };
        self.tab = Some(analysis);
        doc.update_connectivity(false);
        let project = doc.project();
        let Some(a) = project.schematic.simulation.analyses.iter().find(|a| a.id == analysis) else {
            return Err("No such analysis".into());
        };
        if self.session.start(project, doc.connectivity(), a, Arc::new(ngspice)) {
            Ok(())
        } else {
            Err(format!("{} not started: {}", a.name, errors_text(self.session.attempt())))
        }
    }

    /// Per frame: collects a finished run: `Ok(sweep)` when it worked (a sweep: it has a
    /// plot), else a status line.
    pub fn poll(&mut self, doc: &Document) -> Option<Result<bool, String>> {
        let setup = &doc.project().schematic.simulation;
        self.session.retain(|id| setup.analyses.iter().any(|a| a.id == id));
        self.plots.retain(|id, _| setup.analyses.iter().any(|a| a.id == *id));
        let outcome = self.session.poll()?;
        let attempt = self.session.attempt()?;
        match outcome {
            Outcome::Failed(_) => {
                Some(Err(format!("Simulation of {} failed: {}", attempt.name, errors_text(self.session.attempt()))))
            }
            Outcome::Done => {
                let sweep = self.session.result(attempt.analysis).is_some_and(|f| f.result.scale().is_some());
                Some(Ok(sweep))
            }
            _ => None,
        }
    }

    /// The status bar's text while running: "Simulating Transient… 3.2 s".
    pub fn running_text(&self) -> Option<String> {
        let (_, elapsed) = self.session.running()?;
        let name = self.session.attempt().map_or("", |a| a.name.as_str());
        Some(format!("Simulating {name}… {:.1} s", elapsed.as_secs_f64()))
    }

    pub fn ui(&mut self, ui: &mut Ui, doc: &mut Document, message: &mut Option<String>) -> Option<SimAct> {
        let mut act = None;
        if let Some(Err(e)) = &self.ngspice {
            not_found(ui, e, &mut act);
            ui.add_space(6.0);
        }
        let mut setup = doc.project().schematic.simulation.clone();
        let before = setup.clone();
        if setup.analyses.is_empty() {
            hint(ui, "Add an analysis to simulate the schematic with ngspice:");
            ui.horizontal_wrapped(|ui| {
                for kind in KINDS {
                    if ui.button(format!("Add {kind}")).clicked() {
                        self.add(&mut setup, doc.project(), kind, message);
                    }
                }
            });
        } else {
            self.tabs_ui(ui, &mut setup, doc.project(), message);
            ui.add_space(4.0);
            if let Some(i) = self.current(&setup).and_then(|a| setup.analyses.iter().position(|b| b.id == a.id)) {
                self.run_row(ui, &setup.analyses[i], &mut act);
                ui.add_space(4.0);
                self.analysis_ui(ui, &mut setup.analyses[i], doc.project());
                ui.add_space(8.0);
                let a = setup.analyses[i].clone();
                if let Some(probe) = self.results_ui(ui, doc, &a, &mut act) {
                    let probes = &mut setup.analyses[i].probes;
                    let sig = sim::plot::Signal::of_probe(&probe);
                    match probes.iter().position(|p| sim::plot::Signal::of_probe(p) == sig) {
                        Some(k) => _ = probes.remove(k),
                        None => probes.push(probe),
                    }
                }
            }
        }
        ui.add_space(8.0);
        self.circuit_ui(ui, &mut setup, doc, &mut act);
        if setup != before
            && let Err(e) = doc.execute(Command::SetSimulation { simulation: setup })
        {
            *message = Some(e.to_string());
        }
        act
    }

    fn add(&mut self, setup: &mut Simulation, project: &Project, kind: &str, message: &mut Option<String>) {
        let analysis = match kind {
            "Transient" => AnalysisKind::Tran {
                stop: sim::netlist::suggested_stop(project),
                step: None,
                start: 0.0,
                max_step: None,
            },
            "AC" => AnalysisKind::Ac { start: 1.0, stop: 1e7, points_per_decade: 100 },
            "DC sweep" => match sources(project).first() {
                Some(&(id, _, _)) => AnalysisKind::Dc { source: id, start: 0.0, stop: 5.0, step: 0.05 },
                None => {
                    *message = Some("A DC sweep needs a source (VSource or ISource) to sweep".into());
                    return;
                }
            },
            _ => AnalysisKind::Op,
        };
        let a = Analysis::new(unique_name(setup, kind), analysis);
        self.tab = Some(a.id);
        setup.analyses.push(a);
    }

    /// The analysis tabs, with "+" to add one; right-click a tab to rename, duplicate or
    /// delete it.
    fn tabs_ui(&mut self, ui: &mut Ui, setup: &mut Simulation, project: &Project, message: &mut Option<String>) {
        let current = self.current(setup).map(|a| a.id);
        let mut remove = None;
        let mut duplicate = None;
        let mut rename: Option<(usize, String)> = None;
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            for (i, a) in setup.analyses.iter().enumerate() {
                let r = ui.selectable_label(Some(a.id) == current, &a.name).on_hover_text(a.kind.label());
                if r.clicked() {
                    self.tab = Some(a.id);
                }
                r.context_menu(|ui| {
                    let buf = egui::Id::unique(("sim_rename", a.id));
                    let mut name: String =
                        ui.data_mut(|d| d.get_temp_mut_or_insert_with(buf, || a.name.clone()).clone());
                    let edit = ui.add(egui::TextEdit::singleline(&mut name).desired_width(160.0));
                    let enter = edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                    ui.data_mut(|d| d.insert_temp(buf, name.clone()));
                    let done = |ui: &mut Ui| {
                        ui.data_mut(|d| d.remove_temp::<String>(buf));
                        ui.close();
                    };
                    if ui.button("Rename").clicked() || enter {
                        rename = Some((i, name.trim().to_string()));
                        done(ui);
                    }
                    if ui.button("Duplicate").clicked() {
                        duplicate = Some(i);
                        done(ui);
                    }
                    if ui.button("Delete").clicked() {
                        remove = Some(i);
                        done(ui);
                    }
                });
            }
            ui.menu_button("+", |ui| {
                for kind in KINDS {
                    if ui.button(kind).clicked() {
                        self.add(setup, project, kind, message);
                    }
                }
            })
            .response
            .on_hover_text("Add an analysis");
        });
        if let Some((i, name)) = rename
            && name != setup.analyses[i].name
        {
            setup.analyses[i].name = name;
        }
        if let Some(i) = duplicate {
            let mut a = setup.analyses[i].clone();
            a.id = Id::new();
            a.name = unique_name(setup, &a.name);
            self.tab = Some(a.id);
            setup.analyses.insert(i + 1, a);
        }
        if let Some(i) = remove {
            setup.analyses.remove(i);
        }
    }

    /// Run or Stop, and how the last run went.
    fn run_row(&mut self, ui: &mut Ui, a: &Analysis, act: &mut Option<SimAct>) {
        ui.horizontal(|ui| {
            let running = self.session.running();
            if let Some((_, elapsed)) = running {
                if icon_button(ui, Icon::Stop, "Stop", theme::error())
                    .on_hover_text("Stop the simulation (⌘.)")
                    .clicked()
                {
                    *act = Some(SimAct::Stop);
                }
                ui.spinner();
                let name = self.session.attempt().map_or("", |a| a.name.as_str());
                ui.label(format!("{name}… {:.1} s", elapsed.as_secs_f64()));
                ui.ctx().request_repaint_after(Duration::from_millis(100));
                return;
            }
            if icon_button(ui, Icon::Play, "Run", theme::success()).on_hover_text("Run this analysis (⌘R)").clicked()
            {
                *act = Some(SimAct::Run(a.id));
            }
            if a.kind != AnalysisKind::Op
                && self.session.result(a.id).is_some()
                && ui.button("Plot").on_hover_text("Show the Simulation tab").clicked()
            {
                *act = Some(SimAct::ShowPlot);
            }
            let Some(attempt) = self.session.attempt().filter(|t| t.analysis == a.id) else { return };
            let errors = attempt.problems.iter().filter(|p| p.level == Level::Error).count();
            let warnings = attempt.problems.iter().filter(|p| p.level == Level::Warning).count();
            let (text, color) = match &attempt.outcome {
                Outcome::Cancelled => ("Stopped".to_string(), theme::text_dim()),
                Outcome::Done if warnings > 0 => (plural(warnings, "warning"), theme::warn()),
                Outcome::Done => return,
                _ => (plural(errors.max(1), "error"), theme::error()),
            };
            ui.label(RichText::new(text).color(color));
            if attempt.outcome != Outcome::Cancelled && ui.link("show").clicked() {
                *act = Some(SimAct::ShowMessages);
            }
        });
    }

    /// The settings of an analysis' kind.
    fn analysis_ui(&mut self, ui: &mut Ui, a: &mut Analysis, project: &Project) {
        let id = a.id;
        match &mut a.kind {
            AnalysisKind::Op => hint(ui, "The DC voltages and currents with capacitors open and inductors shorted."),
            AnalysisKind::Tran { stop, step, start, max_step } => grid(ui, "sim_tran", |ui| {
                self.value_row(ui, (id, "stop"), "Stop", stop, "s");
                self.optional_row(ui, (id, "step"), "Step", step, "s", "auto (stop / 1000)");
                self.value_row(ui, (id, "start"), "Start saving at", start, "s");
                self.optional_row(ui, (id, "max"), "Max. step", max_step, "s", "auto");
            }),
            AnalysisKind::Ac { start, stop, points_per_decade } => grid(ui, "sim_ac", |ui| {
                self.value_row(ui, (id, "start"), "Start", start, "Hz");
                self.value_row(ui, (id, "stop"), "Stop", stop, "Hz");
                label(ui, "Points per decade");
                ui.add(egui::DragValue::new(points_per_decade).range(1..=10_000).speed(1.0));
                ui.end_row();
            }),
            AnalysisKind::Dc { source, start, stop, step } => {
                let sources = sources(project);
                let unit = sources.iter().find(|s| s.0 == *source).map_or("V", |s| s.2);
                grid(ui, "sim_dc", |ui| {
                    label(ui, "Source");
                    let options: Vec<(Id, String)> = sources.iter().map(|(id, name, _)| (*id, name.clone())).collect();
                    if let Some(s) = crate::fields::combo(ui, ("sim_dc_source", id), source, &options) {
                        *source = s;
                    }
                    ui.end_row();
                    self.value_row(ui, (id, "start"), "Start", start, unit);
                    self.value_row(ui, (id, "stop"), "Stop", stop, unit);
                    self.value_row(ui, (id, "step"), "Step", step, unit);
                });
            }
        }
    }

    /// A labelled value with an SI prefix.
    fn value_row(
        &mut self,
        ui: &mut Ui,
        salt: impl std::hash::Hash + std::fmt::Debug,
        name: &str,
        value: &mut f64,
        unit: &str,
    ) {
        label(ui, name);
        if let Some(Some(v)) = self.si_field(ui, salt, Some(*value), unit, "") {
            *value = v;
        }
        ui.end_row();
    }

    /// A labelled value that may be left empty (`hint` says what then).
    fn optional_row(
        &mut self,
        ui: &mut Ui,
        salt: impl std::hash::Hash + std::fmt::Debug,
        name: &str,
        value: &mut Option<f64>,
        unit: &str,
        empty: &str,
    ) {
        label(ui, name);
        if let Some(v) = self.si_field(ui, salt, *value, unit, empty) {
            *value = v;
        }
        ui.end_row();
    }

    /// A value typed with an SI prefix (`10m`, `1.5 kHz`), committed on Enter or focus
    /// loss. Text that isn't a value shows red and isn't committed. `empty`: the hint of
    /// an optional value's empty field (empty: the value is required). Returns the new
    /// value when it changed.
    fn si_field(
        &mut self,
        ui: &mut Ui,
        salt: impl std::hash::Hash + std::fmt::Debug,
        value: Option<f64>,
        unit: &str,
        empty: &str,
    ) -> Option<Option<f64>> {
        let id = egui::Id::unique(("sim_value", salt));
        let shown = value.map_or(String::new(), |v| si::format(v, unit));
        let focused = ui.memory(|m| m.has_focus(id));
        let bad = self.invalid.contains(&id);
        let buf = self.texts.entry(id).or_insert_with(|| shown.clone());
        if !focused && !bad {
            buf.clone_from(&shown);
        }
        let mut edit = egui::TextEdit::singleline(buf).id(id).desired_width(110.0).hint_text(empty);
        if bad {
            edit = edit.text_color(theme::error());
        }
        let r = ui.add(edit);
        if bad {
            r.clone()
                .on_hover_text(format!("Not a value: type a number with an optional prefix, like 10m or 1.5k{unit}"));
        }
        if !r.lost_focus() {
            return None;
        }
        let text = buf.trim().to_string();
        let new = if text.is_empty() && !empty.is_empty() {
            None
        } else if let Some(v) = si::parse(&text) {
            Some(v)
        } else {
            self.invalid.insert(id);
            return None;
        };
        self.invalid.remove(&id);
        (new != value).then_some(new)
    }

    /// The last result of `a`: a table of the probed and named nets (all with "Show all
    /// nets") and the probed currents. Dimmed when the schematic changed since.
    /// The result table. Returns a probe to add or remove (a row's check box: the
    /// signals the plot shows).
    fn results_ui(&mut self, ui: &mut Ui, doc: &Document, a: &Analysis, act: &mut Option<SimAct>) -> Option<Probe> {
        let stale = self.session.is_stale(doc.project(), doc.connectivity(), a, doc.revision);
        let Some(done) = self.session.result(a.id) else {
            hint(ui, "Not run yet.");
            return None;
        };
        let sweep = done.result.scale().is_some();
        let mut toggle = None;
        ui.horizontal(|ui| {
            ui.label(RichText::new("Results").strong());
            let took = format!("{} · {:.2} s", sim::report::title(&done.result), done.duration.as_secs_f64());
            ui.label(RichText::new(took).color(theme::text_dim()).small());
        });
        if stale {
            ui.label(RichText::new("Changed since the run · run again to update").color(theme::warn()).small());
        }
        ui.horizontal(|ui| {
            ui.add(egui::TextEdit::singleline(&mut self.filter).hint_text("Filter").desired_width(140.0));
            ui.checkbox(&mut self.show_all, "Show all nets");
        });
        let conn = doc.connectivity();
        let probed: HashSet<&str> = a
            .probes
            .iter()
            .filter_map(|p| if let Probe::Voltage { net, .. } = p { Some(net.as_str()) } else { None })
            .collect();
        let filter = self.filter.to_lowercase();
        let rows: Vec<&sim::report::Row> = done
            .rows
            .iter()
            .filter(|r| match &r.net {
                Some(net) => self.show_all || probed.contains(net.as_str()) || conn.net(net).is_some_and(|n| n.named),
                None => true,
            })
            .filter(|r| filter.is_empty() || r.label.to_lowercase().contains(&filter))
            .collect();
        if rows.is_empty() {
            hint(ui, if self.show_all { "No signals match." } else { "No named nets: tick Show all nets." });
            return None;
        }
        let plotted: HashSet<sim::plot::Signal> = a.probes.iter().map(sim::plot::Signal::of_probe).collect();
        let columns = sim::report::columns(&done.result);
        let dim = |t: String| if stale { RichText::new(t).color(theme::text_dim()) } else { RichText::new(t) };
        egui::Grid::new(("sim_results", a.id)).striped(true).spacing([14.0, 3.0]).show(ui, |ui| {
            if sweep {
                label(ui, "Plot");
            }
            label(ui, "Signal");
            for c in columns {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| label(ui, c));
            }
            ui.end_row();
            for row in rows.iter().take(500) {
                let sig = match (&row.net, row.current) {
                    (Some(net), _) => Some(sim::plot::Signal::Voltage(net.clone())),
                    (None, Some((s, p))) => Some(sim::plot::Signal::Current(s, p)),
                    _ => None,
                };
                if sweep {
                    let mut on = sig.as_ref().is_some_and(|s| plotted.contains(s));
                    let tip = if a.probes.is_empty() { "Plot only the ticked signals" } else { "Plot this signal" };
                    if ui.checkbox(&mut on, "").on_hover_text(tip).changed() {
                        toggle = sig.map(|s| s.probe());
                    }
                }
                let r = ui.add(egui::Button::selectable(false, dim(row.label.clone())));
                if let Some(net) = &row.net
                    && r.on_hover_text("Select the net").clicked()
                {
                    *act = Some(SimAct::SelectNet(net.clone()));
                }
                for v in &row.values {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(dim(v.clone()).monospace())
                    });
                }
                ui.end_row();
            }
        });
        if rows.len() > 500 {
            hint(ui, &format!("… and {} more", rows.len() - 500));
        }
        toggle
    }

    /// Ground, supplies, temperature and extra lines (for every analysis), and the netlist
    /// and log.
    fn circuit_ui(&mut self, ui: &mut Ui, setup: &mut Simulation, doc: &Document, act: &mut Option<SimAct>) {
        egui::CollapsingHeader::new(RichText::new("Circuit").strong()).id_salt("sim_circuit").default_open(false).show(
            ui,
            |ui| {
                let mut nets: Vec<String> = doc.connectivity().netlist().map(|n| n.name.clone()).collect();
                nets.sort_by(|a, b| natural_cmp(a, b));
                grid(ui, "sim_circuit_grid", |ui| {
                    label(ui, "Ground");
                    let options: Vec<(Option<String>, String)> =
                        std::iter::once((None, "Automatic (GND ports)".into()))
                            .chain(nets.iter().map(|n| (Some(n.clone()), n.clone())))
                            .collect();
                    if let Some(g) = crate::fields::combo(ui, "sim_ground", &setup.ground, &options) {
                        setup.ground = g;
                    }
                    ui.end_row();
                    label(ui, "Temperature");
                    if let Some(Some(t)) = self.si_field(ui, "temperature", Some(setup.temperature), "°C", "") {
                        setup.temperature = t;
                    }
                    ui.end_row();
                });
                ui.add_space(4.0);
                label(ui, "Supplies (DC sources for power nets without one drawn)");
                let mut remove = None;
                grid(ui, "sim_supplies", |ui| {
                    for (i, s) in setup.supplies.iter_mut().enumerate() {
                        let options: Vec<(String, String)> = nets.iter().map(|n| (n.clone(), n.clone())).collect();
                        if let Some(n) = crate::fields::combo(ui, ("sim_supply", i), &s.net, &options) {
                            s.net = n;
                        }
                        ui.horizontal(|ui| {
                            if let Some(Some(v)) = self.si_field(ui, ("supply", i), Some(s.voltage), "V", "") {
                                s.voltage = v;
                            }
                            if ui.small_button("✕").on_hover_text("Remove").clicked() {
                                remove = Some(i);
                            }
                        });
                        ui.end_row();
                    }
                });
                if let Some(i) = remove {
                    setup.supplies.remove(i);
                    self.texts.clear();
                }
                let unused = power_nets(doc).into_iter().find(|n| !setup.supplies.iter().any(|s| &s.net == n));
                if ui.small_button("Add supply").clicked() {
                    let net = unused.or(nets.first().cloned()).unwrap_or_default();
                    let voltage = supply_voltage(&net);
                    setup.supplies.push(Supply { net, voltage });
                }
                ui.add_space(4.0);
                label(ui, "Extra SPICE lines (.param, .options, models)");
                let id = egui::Id::unique("sim_extra");
                let focused = ui.memory(|m| m.has_focus(id));
                let buf = self.texts.entry(id).or_insert_with(|| setup.extra.clone());
                if !focused {
                    buf.clone_from(&setup.extra);
                }
                let r = ui.add(
                    egui::TextEdit::multiline(buf).id(id).code_editor().desired_rows(3).desired_width(f32::INFINITY),
                );
                if r.lost_focus() && *buf != setup.extra {
                    setup.extra = buf.clone();
                }
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    if ui.button("SPICE netlist").clicked() {
                        *act = Some(SimAct::ShowNetlist);
                    }
                    if ui.button("Log").clicked() {
                        *act = Some(SimAct::ShowLog);
                    }
                });
            },
        );
    }

    /// Opens the netlist window on its netlist or log tab.
    pub fn show_window(&mut self, log: bool) {
        self.window = true;
        self.window_log = log;
    }

    /// The netlist and log window.
    pub fn window_ui(&mut self, ctx: &egui::Context, doc: &Document) {
        if !self.window {
            return;
        }
        let setup = &doc.project().schematic.simulation;
        let analysis = self.current(setup).cloned();
        if let Some(a) = &analysis
            && self.netlist.as_ref().is_none_or(|(rev, id, _)| *rev != doc.revision || *id != a.id)
        {
            let text = sim::netlist::build(doc.project(), doc.connectivity(), a).text;
            self.netlist = Some((doc.revision, a.id, text));
        }
        let mut open = self.window;
        let log = &mut self.window_log;
        let netlist = analysis.as_ref().and(self.netlist.as_ref()).map(|n| n.2.as_str());
        let attempt = self.session.attempt();
        theme::window(ctx, "SPICE netlist and log", &mut open, [640.0, 560.0], |ui| {
            ui.horizontal(|ui| {
                ui.selectable_value(log, false, "Netlist");
                ui.selectable_value(log, true, "Log");
                ui.add_space(8.0);
                let text = if *log { attempt.map(|a| a.log.as_str()) } else { netlist };
                let what = if *log {
                    attempt.map_or(String::new(), |a| format!("ngspice's output of the last run ({})", a.name))
                } else {
                    analysis.as_ref().map_or(String::new(), |a| format!("{} as of now", a.name))
                };
                hint(ui, &what);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if let Some(t) = text
                        && ui.button("Copy").clicked()
                    {
                        ui.ctx().copy_text(t.to_string());
                    }
                });
            });
            ui.separator();
            let text = if *log {
                attempt.map_or("No run yet.", |a| if a.log.is_empty() { "(empty)" } else { a.log.as_str() })
            } else {
                netlist.unwrap_or("Add an analysis first.")
            };
            egui::ScrollArea::both().auto_shrink([false, false]).show(ui, |ui| {
                ui.add(egui::Label::new(RichText::new(text).monospace()).extend());
            });
        });
        self.window = open;
    }
}

/// Where ngspice wasn't found: how to get it.
fn not_found(ui: &mut Ui, error: &str, act: &mut Option<SimAct>) {
    egui::Frame::new().fill(theme::warn().gamma_multiply(0.12)).corner_radius(6).inner_margin(8).show(ui, |ui| {
        ui.label(RichText::new(error).color(theme::warn()));
        hint(
            ui,
            "Install ngspice (macOS: brew install ngspice; Linux: your package manager; Windows: \
                 ngspice.sourceforge.io), or set where it is in Settings.",
        );
        if ui.button("Settings…").clicked() {
            *act = Some(SimAct::OpenSettings);
        }
    });
}

/// A button with an icon tinted `color` and a label.
fn icon_button(ui: &mut Ui, icon: Icon, text: &str, color: egui::Color32) -> egui::Response {
    let r = ui.add(egui::Button::new(format!("     {text}")).min_size(egui::vec2(64.0, 22.0)));
    let rect =
        egui::Rect::from_center_size(egui::pos2(r.rect.left() + 14.0, r.rect.center().y), egui::vec2(14.0, 14.0));
    icons::paint(ui.painter(), rect, icon, color);
    r
}

fn plural(n: usize, what: &str) -> String {
    format!("{n} {what}{}", if n == 1 { "" } else { "s" })
}

/// The errors of an attempt, one per line.
fn errors_text(attempt: Option<&Attempt>) -> String {
    let errors: Vec<&str> = attempt
        .iter()
        .flat_map(|a| &a.problems)
        .filter(|p| p.level == Level::Error)
        .map(|p| p.message.as_str())
        .collect();
    match errors.as_slice() {
        [] => "unknown error".into(),
        [one] => one.to_string(),
        [first, rest @ ..] => format!("{first} (and {} more)", rest.len()),
    }
}

/// `kind`, or `kind 2`, … : a name no analysis has.
pub fn unique_name(setup: &Simulation, kind: &str) -> String {
    let taken = |n: &str| setup.analyses.iter().any(|a| a.name.eq_ignore_ascii_case(n));
    if !taken(kind) {
        return kind.to_string();
    }
    let base = kind.trim_end_matches(|c: char| c.is_ascii_digit()).trim_end();
    (2..).map(|i| format!("{base} {i}")).find(|n| !taken(n)).expect("a free name")
}

/// The source components (a voltage or current source model), by reference, with their
/// unit.
pub fn sources(project: &Project) -> Vec<(Id, String, &'static str)> {
    let mut out: Vec<(Id, String, &'static str)> = project
        .schematic
        .components
        .iter()
        .filter_map(|c| {
            let part = project.library.parts.get(c.part)?;
            match part.sim.as_ref()?.device {
                SimDevice::VoltageSource => Some((c.id, c.refdes.clone(), "V")),
                SimDevice::CurrentSource => Some((c.id, c.refdes.clone(), "A")),
                _ => None,
            }
        })
        .collect();
    out.sort_by(|a, b| natural_cmp(&a.1, &b.1));
    out
}

/// The nets of power ports, but not ground.
fn power_nets(doc: &Document) -> Vec<String> {
    let mut nets: Vec<String> = doc
        .project()
        .schematic
        .labels
        .iter()
        .filter(|l| l.kind == LabelKind::Power)
        .filter(|l| !matches!(l.power_style(), PowerStyle::Ground | PowerStyle::SignalGround))
        .filter_map(|l| Some(doc.connectivity().net_of(ObjectId::Label(l.id))?.name.clone()))
        .collect();
    nets.sort_by(|a, b| natural_cmp(a, b));
    nets.dedup();
    nets
}

/// The voltage a supply net's name suggests (`+5V` → 5, `3V3` → 3.3, `-12V` → −12).
pub fn supply_voltage(net: &str) -> f64 {
    let t = net.trim_start_matches('+');
    si::parse(t).unwrap_or(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_count_up() {
        let mut s = Simulation::default();
        assert_eq!(unique_name(&s, "AC"), "AC");
        s.analyses.push(Analysis::new("AC", AnalysisKind::Op));
        assert_eq!(unique_name(&s, "AC"), "AC 2");
        s.analyses.push(Analysis::new("ac 2", AnalysisKind::Op));
        assert_eq!(unique_name(&s, "AC"), "AC 3");
        assert_eq!(unique_name(&s, "AC 2"), "AC 3");
    }

    #[test]
    fn supply_voltages_from_names() {
        assert_eq!(supply_voltage("+5V"), 5.0);
        assert_eq!(supply_voltage("3V3"), 3.3);
        assert_eq!(supply_voltage("-12V"), -12.0);
        assert_eq!(supply_voltage("VCC"), 0.0);
    }

    #[test]
    fn source_components() {
        let p = view::demo::demo_project();
        let s = sources(&p);
        assert_eq!(s.len(), 1, "the test bench's V1");
        assert_eq!((s[0].1.as_str(), s[0].2), ("V1", "V"));
    }
}
