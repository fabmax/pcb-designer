//! The Simulation panel: the settings of the simulation (analysis) selected last in the
//! project tree or by its tab, Run and Stop, its last result as a table, and the circuit
//! settings (ground, supplies, temperature, extra SPICE lines). Also the SPICE netlist
//! and log window, and Run all's queue.
//!
//! Runs go through [`sim::Session`] (one at a time, on a worker thread). Every setup edit
//! is one `SetSimulation` command.

use std::collections::{HashSet, VecDeque};
use std::time::{Duration, Instant};

use editor_core::Command;
use egui::{RichText, Ui};
use model::*;
use rustc_hash::FxHashMap;
use sim::{Attempt, Level, Outcome, Session};
use view::Document;
use view::sim_overlay::{ProbeTarget, SimOverlay};

use crate::fields::{fill, grid, hint, label};
use crate::icons::{self, Icon};
use crate::theme;

/// What the panel asks the app to do.
pub enum SimAct {
    Run(Id),
    RunAll,
    Stop,
    ShowNetlist,
    ShowLog,
    OpenSettings,
    /// Open the analysis' output tab.
    ShowPlot(Id),
    /// Messages, on the Simulation tab.
    ShowMessages,
    SelectNet(String),
}

#[derive(Default)]
pub struct SimPanel {
    pub session: Session,
    /// The simulation the panel shows: selected last in the project tree, or by its tab.
    pub current: Option<Id>,
    /// The simulation whose output is in the preview tab.
    pub preview: Option<Id>,
    /// Run all: the simulations still to run, and how the batch went so far.
    queue: VecDeque<Id>,
    batch: Option<Batch>,
    /// The error count of each simulation's last attempt that failed (cleared when it
    /// runs through).
    failed: FxHashMap<Id, usize>,
    /// The stale simulations as last checked, and when (and at which revision) that was.
    stale: HashSet<Id>,
    stale_checked: Option<(Instant, u64)>,
    /// Text being typed into value fields, and those that didn't parse.
    texts: FxHashMap<egui::Id, String>,
    pub si: crate::fields::SiFields,
    filter: String,
    show_all: bool,
    /// ngspice as found (once; again when the settings change).
    engine: Option<sim::Engine>,
    /// The netlist window: shown, on the log tab.
    pub window: bool,
    window_log: bool,
    /// The current netlist of the shown analysis: (revision, analysis, text).
    netlist: Option<(u64, Id, String)>,
    /// Export CSV: the file dialog, with the CSV text to write.
    pub csv_dialog: crate::file_dialog::Dialog<String>,
    /// The plots' views, per analysis.
    pub plots: std::collections::HashMap<Id, crate::plot_ui::PlotState>,
    /// Transients that show their spectrum, and the spectrum plots' views.
    pub spectrum: std::collections::HashSet<Id>,
    pub spectra: std::collections::HashMap<Id, crate::plot_ui::PlotState>,
    /// AC plots leave out the phase.
    pub no_phase: bool,
    /// The table under the plot as a share of the plot tab's height, once the user
    /// dragged the handle between them (`None`: sized to its rows).
    pub table_share: Option<f32>,
    /// What the Probe tool points at, or a signal hovered in a table: highlighted.
    pub hover: Option<ProbeTarget>,
    /// The overlay as last built, and what it was built from.
    overlay: Option<(OverlayKey, SimOverlay, Vec<sim::report::Device>)>,
    /// Voltages typed for suggested supplies whose name says none.
    suggest: FxHashMap<String, f64>,
    /// The Probe tool's press: what it pressed on and where (a drag to another net makes
    /// a differential probe).
    pub probe_drag: Option<(ProbeTarget, egui::Pos2)>,
    /// An agent's run (MCP `simulate`), answered when it ends.
    agent: Option<(mcp::simulation::SimJob, mcp::host::Responder)>,
    /// The tuner's slider values (its rows are the setup's).
    pub tuner: sim::tune::Tuner,
    /// The tuner changed a value while a run was going: run the tab's analysis again
    /// when it ends.
    pub tune_pending: bool,
}

/// How a Run all went so far.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Batch {
    ran: usize,
    failed: usize,
    /// The simulation it started last (a run going on that it didn't start is replaced).
    started: Option<Id>,
}

/// At most this often the tree's outdated marks are checked (a netlist per simulation).
const STALE_EVERY: Duration = Duration::from_secs(1);

/// What the schematic overlay depends on.
#[derive(Clone, Debug, PartialEq)]
struct OverlayKey {
    revision: u64,
    probes: Option<Id>,
    /// The operating point shown: analysis, run and step.
    values: Option<(Id, u64, usize)>,
    hover: Option<ProbeTarget>,
    dark: bool,
    tuned: Vec<(TuneParam, f64)>,
}

/// `v` rounded to three significant digits (the schematic's tags).
fn three_digits(v: f64) -> f64 {
    if v == 0.0 || !v.is_finite() {
        return v;
    }
    let f = 10f64.powf(v.abs().log10().floor() - 2.0);
    (v / f).round() * f
}

/// Adds `probe`, or removes it when its signal is probed already.
fn toggle(probes: &mut Vec<Probe>, probe: Probe, conn: &editor_core::connectivity::Connectivity) {
    let sig = sim::plot::Signal::of_probe(&probe, conn);
    match probes.iter().position(|p| sim::plot::Signal::of_probe(p, conn) == sig) {
        Some(k) => _ = probes.remove(k),
        None => probes.push(probe),
    }
}

/// The kinds of analyses, in the Add simulation menu's order: the kind (also a new
/// analysis' name) and its label, which says what the SPICE names mean.
pub const KINDS: [(&str, &str); 4] = [
    ("Operating point", "Operating point"),
    ("Transient", "Transient"),
    ("AC", "AC (frequency response)"),
    ("DC sweep", "DC sweep (transfer curve)"),
];

impl SimPanel {
    /// A new project: no results, nothing selected.
    /// The view of an analysis' plot as shown: its spectrum's, when it shows one.
    pub fn plot_state(&mut self, analysis: Id) -> &mut crate::plot_ui::PlotState {
        if self.spectrum.contains(&analysis) {
            self.spectra.entry(analysis).or_default()
        } else {
            self.plots.entry(analysis).or_default()
        }
    }

    pub fn clear(&mut self) {
        self.session.clear();
        self.current = None;
        self.preview = None;
        self.queue.clear();
        self.batch = None;
        self.failed.clear();
        self.stale.clear();
        self.stale_checked = None;
        self.texts.clear();
        self.si.clear();
        self.netlist = None;
        self.plots.clear();
        self.spectra.clear();
        self.overlay = None;
        self.hover = None;
        self.tuner.reset();
        self.tune_pending = false;
    }

    /// Looks for ngspice again (the settings changed).
    pub fn forget_ngspice(&mut self) {
        self.engine = None;
    }

    /// ngspice, found once (the library and the executable).
    pub fn engine(&mut self, settings: &sim::EngineSettings) -> &sim::Engine {
        self.engine.get_or_insert_with(|| sim::Engine::find(settings))
    }

    /// Why ngspice can't run (nothing found, a configured path that doesn't work).
    fn engine_error(&self) -> Option<String> {
        let e = self.engine.as_ref()?;
        e.error
            .clone()
            .or_else(|| (e.library.is_none() && e.process.is_none()).then(|| "ngspice is not installed".to_string()))
    }

    /// The simulation shown, if it still exists.
    pub fn current<'a>(&self, setup: &'a Simulation) -> Option<&'a Analysis> {
        self.current.and_then(|id| setup.analyses.iter().find(|a| a.id == id))
    }

    /// The analysis probes go to: the one shown if it's a sweep, else the first sweep.
    pub fn probe_analysis<'a>(&self, setup: &'a Simulation) -> Option<&'a Analysis> {
        let sweep = |a: &&Analysis| !matches!(a.kind, AnalysisKind::Op);
        self.current(setup).filter(sweep).or_else(|| setup.analyses.iter().find(sweep))
    }

    /// The operating point whose values the schematic shows: the one shown, else the
    /// last one run.
    fn values_analysis<'a>(&self, setup: &'a Simulation) -> Option<&'a Analysis> {
        let op = |a: &&Analysis| matches!(a.kind, AnalysisKind::Op) && self.session.result(a.id).is_some();
        self.current(setup)
            .filter(op)
            .or_else(|| setup.analyses.iter().filter(op).max_by_key(|a| self.session.result(a.id).map(|r| r.run)))
    }

    /// Adds a probe to or removes it from the probe analysis (one `SetSimulation`).
    pub fn toggle_probe(&self, doc: &mut Document, probe: Probe) -> Result<(), String> {
        let mut setup = doc.project().schematic.simulation.clone();
        let Some(id) = self.probe_analysis(&setup).map(|a| a.id) else {
            return Err("Add a transient, AC or DC sweep analysis to probe".into());
        };
        let a = setup.analyses.iter_mut().find(|a| a.id == id).expect("probe analysis");
        toggle(&mut a.probes, probe, doc.connectivity());
        doc.execute(Command::SetSimulation { simulation: setup }).map(|_| ()).map_err(|e| e.to_string())
    }

    /// Removes the probe at `index` of `analysis`.
    pub fn remove_probe(doc: &mut Document, analysis: Id, index: usize) -> Result<(), String> {
        let mut setup = doc.project().schematic.simulation.clone();
        let Some(a) = setup.analyses.iter_mut().find(|a| a.id == analysis) else { return Ok(()) };
        if index >= a.probes.len() {
            return Ok(());
        }
        a.probes.remove(index);
        doc.execute(Command::SetSimulation { simulation: setup }).map(|_| ()).map_err(|e| e.to_string())
    }

    /// Whether `target` is probed in the probe analysis.
    pub fn is_probed(&self, doc: &Document, target: &ProbeTarget) -> bool {
        let conn = doc.connectivity();
        let sig = sim::plot::Signal::of_probe(&target.probe(), conn);
        self.probe_analysis(&doc.project().schematic.simulation)
            .is_some_and(|a| a.probes.iter().any(|p| sim::plot::Signal::of_probe(p, conn) == sig))
    }

    /// What the schematic shows (`shown`: the Simulation panel or tab is open): the probe
    /// analysis' probes in their trace colours, the operating point's voltages and the
    /// currents of two-pin parts, and the hovered target.
    pub fn overlay(&mut self, doc: &Document, shown: bool) -> SimOverlay {
        // Its colours are the schematic's.
        let _theme = view::style::scope(view::style::Editor::Schematic);
        let setup = &doc.project().schematic.simulation;
        let probes = shown.then(|| self.probe_analysis(setup)).flatten().map(|a| a.id);
        let values = if shown { self.values_analysis(setup) } else { None };
        let key = OverlayKey {
            revision: doc.revision,
            probes,
            values: values.and_then(|a| {
                let step = self.plots.get(&a.id).map_or(0, |p| p.sort.0.saturating_sub(1));
                Some((a.id, self.session.result(a.id)?.run, step))
            }),
            hover: self.hover.clone().filter(|_| shown),
            dark: view::style::theme() == view::style::Theme::Dark,
            tuned: self.tuned(doc.project()),
        };
        if let Some((k, o, _)) = &self.overlay
            && *k == key
        {
            return o.clone();
        }
        let conn = doc.connectivity();
        let mut o = SimOverlay::default();
        if let Some(a) = probes.and_then(|id| setup.analyses.iter().find(|a| a.id == id)) {
            for (i, p) in a.probes.iter().enumerate() {
                let color = view::style::trace_color(i);
                match sim::plot::Signal::of_probe(p, conn) {
                    sim::plot::Signal::Voltage(net) => o.nets.push((net, color)),
                    sim::plot::Signal::Diff(net, reference) => {
                        o.nets.push((net, color));
                        o.nets.push((reference, color));
                    }
                    sim::plot::Signal::Current(s, pin) => o.pins.push((s, pin, color)),
                    sim::plot::Signal::Power(c) => o.parts.push((c, color)),
                }
            }
        }
        let mut devices = Vec::new();
        let stale = values.is_some_and(|a| {
            let overrides = self.overrides(doc.project(), a);
            self.session.is_stale(doc.project(), conn, a, &overrides, doc.revision)
        });
        if let Some(a) = values
            && let Some(done) = self.session.result(a.id)
        {
            o.dim = stale;
            // A sweep: the step its table is sorted by, else the first.
            let results = done.results();
            let r = results.get(key.values.map_or(0, |v| v.2)).unwrap_or(&results[0]).1;
            for (net, node) in &r.netlist.nodes {
                if node != "0"
                    && let Some(v) = r.op_voltage(net)
                {
                    o.voltages.push((net.clone(), si::format(three_digits(v), "V")));
                }
            }
            devices = sim::report::devices(r);
            for d in devices.iter().filter(|d| d.pins.len() == 2) {
                let (a, b) = (&d.pins[0], &d.pins[1]);
                let (Some(ia), Some(sa), Some(sb)) = (a.current, a.symbol, b.symbol) else { continue };
                if ia.abs() < 1e-12 {
                    continue;
                }
                let (symbol, pin) = if ia > 0.0 { (sa, a.pin) } else { (sb, b.pin) };
                o.currents.push((symbol, pin, si::format(three_digits(ia.abs()), "A")));
            }
        }
        match key.hover.clone() {
            Some(ProbeTarget::Net { net, .. }) => o.hover_nets.push(net),
            Some(ProbeTarget::Diff { net, reference, .. }) => o.hover_nets.extend([net, reference]),
            Some(ProbeTarget::Pin { symbol, pin }) => o.hover_pins.push((symbol, pin)),
            Some(ProbeTarget::Part { component }) => o.hover_parts.push(component),
            None => {}
        }
        self.overlay = Some((key, o.clone(), devices));
        o
    }

    /// The operating point of a device whose values the schematic shows (as of the last
    /// [`overlay`](Self::overlay)).
    pub fn device(&self, component: Id) -> Option<&sim::report::Device> {
        self.overlay.as_ref()?.2.iter().find(|d| d.component == component)
    }

    /// The tuned values that differ from the document's.
    pub fn tuned(&self, project: &Project) -> Vec<(TuneParam, f64)> {
        self.tuner.changed(project, &project.schematic.simulation.tune, None)
    }

    /// The components runs of `a` use: the tuned values put in (but the parameter it
    /// sweeps).
    pub fn overrides(&self, project: &Project, a: &Analysis) -> sim::tune::Overrides {
        let swept = a.step.as_ref().map(|s| &s.param);
        self.tuner.overrides(project, &project.schematic.simulation.tune, swept)
    }

    /// The tuner changed a value: runs the tab's analysis with it, or once the run going
    /// ends. Quiet when ngspice is missing (the panel says so) or the netlist has errors
    /// (the panel's problems show them).
    pub fn tune_run(&mut self, doc: &mut Document, engine: &sim::EngineSettings) {
        let Some(id) = self.current(&doc.project().schematic.simulation).map(|a| a.id) else { return };
        if self.session.running().is_some() {
            self.tune_pending = true;
            return;
        }
        self.tune_pending = false;
        // Nothing to do when the last run had these values already.
        doc.update_connectivity(false);
        let project = doc.project();
        if let Some(a) = project.schematic.simulation.analyses.iter().find(|a| a.id == id)
            && self.session.result(id).is_some()
        {
            let overrides = self.overrides(project, a);
            if !self.session.is_stale(project, doc.connectivity(), a, &overrides, doc.revision) {
                return;
            }
        }
        let _ = self.run(doc, id, engine);
    }

    /// Whether the tab's last run was quick enough to re-run while a slider moves.
    pub fn tune_live(&self, project: &Project) -> bool {
        self.current(&project.schematic.simulation)
            .and_then(|a| self.session.result(a.id))
            .is_some_and(|f| f.duration < sim::tune::LIVE)
    }

    /// Starts `analysis` (a single run: Run all's queue is dropped). Returns an error to
    /// show when ngspice is missing or the netlist has errors.
    pub fn run(&mut self, doc: &mut Document, analysis: Id, engine: &sim::EngineSettings) -> Result<(), String> {
        self.queue.clear();
        self.batch = None;
        self.start(doc, analysis, engine)
    }

    fn start(&mut self, doc: &mut Document, analysis: Id, engine: &sim::EngineSettings) -> Result<(), String> {
        self.engine(engine);
        doc.update_connectivity(false);
        let project = doc.project();
        let Some(a) = project.schematic.simulation.analyses.iter().find(|a| a.id == analysis) else {
            return Err("No such analysis".into());
        };
        let overrides = self.overrides(project, a);
        let backend = self.engine.as_ref().expect("found above").backend(a)?;
        if self.session.start_with(project, doc.connectivity(), a, &[], &overrides, backend) {
            Ok(())
        } else {
            self.failed.insert(a.id, error_count(self.session.attempt()));
            Err(format!("{} not started: {}", a.name, errors_text(self.session.attempt())))
        }
    }

    /// Run all: every simulation in order, one after the other (replacing a Run all
    /// going on). [`next_in_queue`](Self::next_in_queue) starts them.
    pub fn run_all(&mut self, setup: &Simulation) {
        self.queue = setup.analyses.iter().map(|a| a.id).collect();
        self.batch = Some(Batch::default());
    }

    /// Whether a Run all is going on.
    pub fn running_all(&self) -> bool {
        self.batch.is_some()
    }

    /// Stops the run going on, and Run all's queue.
    pub fn stop(&mut self) {
        self.queue.clear();
        self.batch = None;
        self.session.stop();
    }

    /// Per frame, while nothing runs: starts Run all's next simulation (skipping those
    /// that can't start). At the end, the status line: "Ran 4 simulations, 1 failed".
    pub fn next_in_queue(&mut self, doc: &mut Document, engine: &sim::EngineSettings) -> Option<String> {
        let batch = self.batch?;
        if self.session.running().is_some() && batch.started.is_some() {
            return None;
        }
        while let Some(id) = self.queue.pop_front() {
            if !doc.project().schematic.simulation.analyses.iter().any(|a| a.id == id) {
                continue;
            }
            let started = self.start(doc, id, engine);
            let batch = self.batch.as_mut().expect("going on");
            batch.started = Some(id);
            match started {
                Ok(()) => return None,
                Err(_) => {
                    batch.ran += 1;
                    batch.failed += 1;
                }
            }
        }
        let b = self.batch.take()?;
        Some(batch_text(b.ran, b.failed))
    }

    /// Starts an agent's run; [`poll`](Self::poll) answers it when it ends.
    pub fn start_agent(
        &mut self,
        doc: &Document,
        job: mcp::simulation::SimJob,
        reply: mcp::host::Responder,
        engine: &sim::EngineSettings,
    ) {
        if let Some((_, earlier)) = self.agent.take() {
            let _ = earlier.send(Err("stopped: another simulate call came".into()));
        }
        let analysis = doc.project().schematic.simulation.analyses.iter().find(|a| a.id == job.analysis);
        let backend =
            match analysis.ok_or_else(|| "no such analysis".to_string()).and_then(|a| self.engine(engine).backend(a)) {
                Ok(b) => b,
                Err(e) => {
                    let _ = reply.send(Err(e));
                    return;
                }
            };
        self.queue.clear();
        self.batch = None;
        match mcp::simulation::start(&mut self.session, doc, &job, backend) {
            Ok(()) => self.agent = Some((job, reply)),
            Err(e) => _ = reply.send(Err(e)),
        }
    }

    /// Per frame: collects a finished run: `Ok(sweep)` when it worked (a sweep: it has a
    /// plot), else a status line. Answers an agent's run that ended.
    pub fn poll(&mut self, doc: &Document) -> Option<Result<bool, String>> {
        let setup = &doc.project().schematic.simulation;
        self.session.retain(|id| setup.analyses.iter().any(|a| a.id == id));
        self.plots.retain(|id, _| setup.analyses.iter().any(|a| a.id == *id));
        self.spectra.retain(|id, _| setup.analyses.iter().any(|a| a.id == *id));
        self.tuner.retain(&setup.tune);
        let outcome = self.session.poll();
        if let Some((job, _)) = &self.agent
            && let Some(answer) = mcp::simulation::check(job, &mut self.session, doc)
            && let Some((_, reply)) = self.agent.take()
        {
            let _ = reply.send(answer.map(mcp::Reply::Json));
        }
        let outcome = outcome?;
        let attempt = self.session.attempt()?;
        let id = attempt.analysis;
        match &outcome {
            Outcome::Failed(_) => _ = self.failed.insert(id, error_count(Some(attempt))),
            Outcome::Done => {
                self.failed.remove(&id);
                self.stale.remove(&id);
            }
            _ => {}
        }
        if let Some(b) = &mut self.batch
            && b.started == Some(id)
        {
            b.ran += 1;
            b.failed += usize::from(matches!(outcome, Outcome::Failed(_)));
            // Run all says how it went at its end (`next_in_queue`).
            return None;
        }
        match outcome {
            Outcome::Failed(_) => {
                Some(Err(format!("Simulation of {} failed: {}", attempt.name, errors_text(self.session.attempt()))))
            }
            Outcome::Done => {
                let sweep = self.session.result(id).is_some_and(|f| f.result.scale().is_some());
                Some(Ok(sweep))
            }
            _ => None,
        }
    }

    /// Checks which simulations are stale, at most once a second and only after an edit
    /// or a run (a netlist per simulation).
    pub fn check_stale(&mut self, doc: &Document) {
        let now = Instant::now();
        let runs: u64 = doc
            .project()
            .schematic
            .simulation
            .analyses
            .iter()
            .filter_map(|a| self.session.result(a.id).map(|f| f.run))
            .sum();
        let key = doc.revision.wrapping_add(runs.wrapping_mul(0x9e37_79b9));
        if self.stale_checked.is_some_and(|(at, k)| k == key || now - at < STALE_EVERY) {
            return;
        }
        self.stale_checked = Some((now, key));
        let project = doc.project();
        self.stale.clear();
        for a in &project.schematic.simulation.analyses {
            if self.session.result(a.id).is_none() {
                continue;
            }
            let overrides = self.overrides(project, a);
            if self.session.is_stale(project, doc.connectivity(), a, &overrides, doc.revision) {
                self.stale.insert(a.id);
            }
        }
    }

    /// A simulation's state in the project tree (outdated as of [`check_stale`](Self::check_stale)).
    pub fn tree_detail(&self, a: &Analysis) -> crate::project_panel::AnalysisDetail {
        use crate::project_panel::{AnalysisDetail, OUTDATED};
        if self.session.running().is_some_and(|(id, _)| id == a.id) {
            let text = self.session.progress().map_or("running".into(), |p| format!("{:.0} %", p * 100.0));
            return AnalysisDetail { text, warn: false };
        }
        if self.queue.contains(&a.id) {
            return AnalysisDetail { text: "queued".into(), warn: false };
        }
        if let Some(&n) = self.failed.get(&a.id) {
            return AnalysisDetail { text: plural(n.max(1), "error"), warn: true };
        }
        if self.stale.contains(&a.id) {
            return AnalysisDetail { text: OUTDATED.into(), warn: true };
        }
        AnalysisDetail::default()
    }

    /// The status bar's text while running: "Simulating Transient… 3.2 s".
    pub fn running_text(&self) -> Option<String> {
        let (_, elapsed) = self.session.running()?;
        let name = self.session.attempt().map_or("", |a| a.name.as_str());
        let percent = self.session.progress().map(|p| format!(" {:.0} %", p * 100.0)).unwrap_or_default();
        Some(format!("Simulating {name}…{percent} {:.1} s", elapsed.as_secs_f64()))
    }

    /// The panel: the current simulation's settings and results with the circuit
    /// settings collapsed below, or (`circuit`: the Simulation node is selected) the
    /// circuit settings only.
    pub fn ui(
        &mut self,
        ui: &mut Ui,
        doc: &mut Document,
        circuit: bool,
        message: &mut Option<String>,
    ) -> Option<SimAct> {
        let mut act = None;
        if let Some(e) = self.engine_error() {
            not_found(ui, &e, &mut act);
            ui.add_space(6.0);
        }
        let mut setup = doc.project().schematic.simulation.clone();
        let before = setup.clone();
        let current = self.current(&setup).and_then(|a| setup.analyses.iter().position(|b| b.id == a.id));
        match current {
            _ if circuit => {
                crate::fields::section(ui, "Circuit", Some("The settings every simulation shares"));
                self.circuit_grid(ui, &mut setup, doc, &mut act);
                if !setup.analyses.is_empty() {
                    ui.add_space(8.0);
                    self.run_all_button(ui, &mut act);
                }
            }
            Some(i) => {
                let a = &setup.analyses[i];
                let kind = if a.name.eq_ignore_ascii_case(a.kind.label()) { "" } else { a.kind.label() };
                crate::fields::section_note(ui, &a.name, kind);
                self.run_row(ui, &setup.analyses[i], &mut act);
                ui.add_space(4.0);
                self.analysis_ui(ui, &mut setup.analyses[i], doc.project());
                ui.add_space(8.0);
                let a = setup.analyses[i].clone();
                if let Some(probe) = self.results_ui(ui, doc, &a, &mut act) {
                    toggle(&mut setup.analyses[i].probes, probe, doc.connectivity());
                }
                ui.add_space(8.0);
                crate::fields::collapsing(ui, "Circuit", "sim_circuit", |ui| {
                    self.circuit_grid(ui, &mut setup, doc, &mut act)
                });
            }
            None => {
                hint(ui, "Create or select a simulation in the project tree.");
                ui.add_space(4.0);
                if setup.analyses.is_empty() {
                    hint(ui, "Right-click its Simulation node, or use the + above the tree, to add one.");
                }
            }
        }
        if setup != before
            && let Err(e) = doc.execute(Command::SetSimulation { simulation: setup })
        {
            *message = Some(e.to_string());
        }
        act
    }

    /// Adds a simulation of `kind` ([`KINDS`]) to `setup` and makes it the current one.
    /// Refuses a DC sweep without a source to sweep.
    pub fn add(&mut self, setup: &mut Simulation, project: &Project, kind: &str) -> Result<Id, String> {
        let a = new_analysis(setup, project, kind)?;
        let id = a.id;
        setup.analyses.push(a);
        self.current = Some(id);
        Ok(id)
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
                let name = if self.running_all() { format!("Run all: {name}") } else { name.to_string() };
                let percent = self.session.progress().map(|p| format!(" {:.0} %", p * 100.0)).unwrap_or_default();
                ui.label(format!("{name}…{percent} {:.1} s", elapsed.as_secs_f64()));
                ui.ctx().request_repaint_after(Duration::from_millis(100));
                return;
            }
            if icon_button(ui, Icon::Play, "Run", theme::success()).on_hover_text("Run this analysis (⌘R)").clicked()
            {
                *act = Some(SimAct::Run(a.id));
            }
            if ui.button("Run all").on_hover_text("Run every simulation (⇧⌘R)").clicked() {
                *act = Some(SimAct::RunAll);
            }
            if self.session.result(a.id).is_some()
                && ui.button("Show").on_hover_text("Show its output in a tab").clicked()
            {
                *act = Some(SimAct::ShowPlot(a.id));
            }
            let Some(attempt) = self.session.attempt().filter(|t| t.analysis == a.id) else { return };
            let errors = attempt.problems.iter().filter(|p| p.level == Level::Error).count();
            let warnings = attempt.problems.iter().filter(|p| p.level == Level::Warning).count();
            let stopped = self.session.result(a.id).and_then(crate::plot_ui::stopped_at);
            let (text, color) = match &attempt.outcome {
                Outcome::Cancelled => ("Stopped".to_string(), theme::text_dim()),
                Outcome::Done if let Some(at) = stopped => (format!("Stopped at {at}"), theme::text_dim()),
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
                fill(ui, egui::DragValue::new(points_per_decade).range(1..=10_000).speed(1.0));
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
        ui.add_space(4.0);
        self.step_ui(ui, a, project);
    }

    /// The parameter sweep: off, or a parameter and its values (a list, linear or log).
    fn step_ui(&mut self, ui: &mut Ui, a: &mut Analysis, project: &Project) {
        let id = a.id;
        let mut on = a.step.is_some();
        let tip = "Run it once per value of a part's value or a source setting";
        let mut toggled = false;
        grid(ui, ("sim_sweep_on", id), |ui| {
            toggled = crate::fields::check_row(ui, "Parameter sweep", &mut on).on_hover_text(tip).changed();
        });
        if toggled {
            a.step = if on {
                // The tuner's first row, else the first tunable parameter.
                let tuned = project.schematic.simulation.tune.first().map(|r| r.param.clone());
                tuned
                    .or_else(|| sim::tune::tunable(project).into_iter().next())
                    .and_then(|p| sim::tune::default_step(project, &p))
            } else {
                None
            };
        }
        let Some(step) = &mut a.step else { return };
        let unit = sim::tune::unit(project, &step.param);
        grid(ui, "sim_step", |ui| {
            label(ui, "Parameter");
            let options: Vec<(TuneParam, String)> =
                sim::tune::tunable(project).into_iter().map(|p| (p.clone(), sim::tune::label(project, &p))).collect();
            if let Some(p) = crate::fields::combo(ui, ("sim_step_param", id), &step.param, &options)
                && let Some(s) = sim::tune::default_step(project, &p)
            {
                *step = s;
            }
            ui.end_row();
            label(ui, "Values");
            let kinds = [(0, "List".to_string()), (1, "Linear".to_string()), (2, "Log".to_string())];
            let kind = match step.values {
                StepValues::List { .. } => 0,
                StepValues::Linear { .. } => 1,
                StepValues::Log { .. } => 2,
            };
            if let Some(k) = crate::fields::combo(ui, ("sim_step_kind", id), &kind, &kinds) {
                let points = step.values.points();
                let (lo, hi) = (points.first().copied().unwrap_or(1.0), points.last().copied().unwrap_or(10.0));
                step.values = match k {
                    0 => StepValues::List { values: points },
                    1 => StepValues::Linear { start: lo, stop: hi, step: (hi - lo) / 4.0 },
                    _ => StepValues::Log { start: lo.max(1e-12), stop: hi.max(lo * 10.0), points_per_decade: 3 },
                };
            }
            ui.end_row();
            match &mut step.values {
                StepValues::List { values } => {
                    label(ui, "List");
                    let key = egui::Id::unique(("sim_step_list", id));
                    let shown = values.iter().map(|v| si::format(*v, unit)).collect::<Vec<_>>().join(", ");
                    let focused = ui.memory(|m| m.has_focus(key));
                    let buf = self.texts.entry(key).or_insert_with(|| shown.clone());
                    if !focused {
                        buf.clone_from(&shown);
                    }
                    let r = ui
                        .add(egui::TextEdit::singleline(buf).id(key).desired_width(160.0).hint_text("1k, 2.2k, 4.7k"));
                    if r.lost_focus() {
                        let parsed: Option<Vec<f64>> =
                            buf.split([',', ';']).map(str::trim).filter(|t| !t.is_empty()).map(si::parse).collect();
                        if let Some(v) = parsed {
                            *values = v;
                        }
                    }
                    ui.end_row();
                }
                StepValues::Linear { start, stop, step } => {
                    self.value_row(ui, (id, "step_start"), "From", start, unit);
                    self.value_row(ui, (id, "step_stop"), "To", stop, unit);
                    self.value_row(ui, (id, "step_step"), "Step", step, unit);
                }
                StepValues::Log { start, stop, points_per_decade } => {
                    self.value_row(ui, (id, "step_start"), "From", start, unit);
                    self.value_row(ui, (id, "step_stop"), "To", stop, unit);
                    label(ui, "Per decade");
                    fill(ui, egui::DragValue::new(points_per_decade).range(1..=100).speed(0.1));
                    ui.end_row();
                }
            }
        });
        let points = step.values.points();
        let text = match step.values.problem() {
            Some(p) => RichText::new(p).color(theme::warn()),
            None => {
                let list: Vec<String> = points.iter().take(8).map(|v| si::format(*v, unit)).collect();
                let more = if points.len() > 8 { ", …" } else { "" };
                RichText::new(format!("{} steps: {}{more}", points.len(), list.join(", "))).color(theme::text_dim())
            }
        };
        ui.label(text.small());
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
        self.si.field(ui, ("sim_value", salt), value, unit, empty)
    }

    /// The last result of `a` (or the points so far of a run going): a table of the
    /// probed and named nets (all with "Show all nets") and the probed currents. Dimmed
    /// when the schematic changed since. The result table. Returns a probe to add or remove (a row's check box: the
    /// signals the plot shows).
    fn results_ui(&mut self, ui: &mut Ui, doc: &Document, a: &Analysis, act: &mut Option<SimAct>) -> Option<Probe> {
        let overrides = self.overrides(doc.project(), a);
        let stale = self.session.is_stale(doc.project(), doc.connectivity(), a, &overrides, doc.revision);
        // A run going shows its points so far (backends that stream).
        let Some(done) = self.session.live(a.id).or_else(|| self.session.result(a.id)) else {
            hint(
                ui,
                if self.session.running().is_some_and(|(id, _)| id == a.id) { "Running…" } else { "Not run yet." },
            );
            return None;
        };
        let sweep = done.result.scale().is_some();
        let mut toggle = None;
        let took = format!("{} · {:.2} s", sim::report::title(&done.result), done.duration.as_secs_f64());
        crate::fields::section_note(ui, "Results", &took);
        if stale {
            ui.label(RichText::new("Changed since the run · run again to update").color(theme::warn()).small());
        }
        crate::fields::search_field(ui, &mut self.filter, |e| e.hint_text("Filter"));
        ui.checkbox(&mut self.show_all, "Show all nets");
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
        let plotted: HashSet<sim::plot::Signal> =
            a.probes.iter().map(|p| sim::plot::Signal::of_probe(p, conn)).collect();
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
                let sig = row_signal(row);
                if sweep {
                    let mut on = sig.as_ref().is_some_and(|s| plotted.contains(s));
                    let tip = if a.probes.is_empty() { "Plot only the ticked signals" } else { "Plot this signal" };
                    if ui.checkbox(&mut on, "").on_hover_text(tip).changed() {
                        toggle = sig.as_ref().map(|s| s.probe());
                    }
                }
                let r = ui.add(egui::Button::selectable(false, dim(row.label.clone())));
                if sig.as_ref().is_some_and(|s| plotted.contains(s)) {
                    r.context_menu(|ui| {
                        if ui.button("Remove probe").clicked() {
                            toggle = sig.as_ref().map(|s| s.probe());
                            ui.close();
                        }
                    });
                }
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
            // Probes without data in this result, and why.
            if sweep {
                let have: HashSet<sim::plot::Signal> = done.rows.iter().filter_map(row_signal).collect();
                for sig in a.probes.iter().map(|p| sim::plot::Signal::of_probe(p, conn)) {
                    if have.contains(&sig) {
                        continue;
                    }
                    let mut on = true;
                    if ui.checkbox(&mut on, "").on_hover_text("Remove the probe").changed() {
                        toggle = Some(sig.probe());
                    }
                    let label = probe_label(doc.project(), &sig);
                    let reason = sim::plot::missing_reason(&done.result, &sig, conn);
                    let r = ui.label(RichText::new(format!("{label} · {reason}")).color(theme::text_dim()));
                    r.context_menu(|ui| {
                        if ui.button("Remove probe").clicked() {
                            toggle = Some(sig.probe());
                            ui.close();
                        }
                    });
                    ui.end_row();
                }
            }
        });
        if rows.len() > 500 {
            hint(ui, &format!("… and {} more", rows.len() - 500));
        }
        toggle
    }

    /// Ground, supplies, temperature and extra lines (for every analysis), and the netlist
    /// and log.
    fn circuit_grid(&mut self, ui: &mut Ui, setup: &mut Simulation, doc: &Document, act: &mut Option<SimAct>) {
        {
            let mut nets: Vec<String> = doc.connectivity().netlist().map(|n| n.name.clone()).collect();
            nets.sort_by(|a, b| natural_cmp(a, b));
            grid(ui, "sim_circuit_grid", |ui| {
                label(ui, "Ground");
                let options: Vec<(Option<String>, String)> = std::iter::once((None, "Automatic (GND ports)".into()))
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
                self.si.clear();
            }
            let suggested = suggested_supplies(doc);
            if ui.small_button("Add supply").clicked() {
                let net = suggested.first().map(|s| s.0.clone()).or(nets.first().cloned()).unwrap_or_default();
                let voltage = supply_voltage(&net).unwrap_or(0.0);
                setup.supplies.push(Supply { net, voltage });
            }
            if !suggested.is_empty() {
                ui.add_space(4.0);
                label(ui, "Power nets without a source");
                let mut add = Vec::new();
                grid(ui, "sim_suggested", |ui| {
                    for (net, guess) in &suggested {
                        ui.label(net);
                        let typed = self.suggest.get(net).copied();
                        let v = typed.or(*guess);
                        ui.horizontal(|ui| {
                            if let Some(new) = self.si_field(ui, ("suggest", net.as_str()), v, "V", "volts") {
                                match new {
                                    Some(x) => _ = self.suggest.insert(net.clone(), x),
                                    None => _ = self.suggest.remove(net),
                                }
                            }
                            let r = ui.add_enabled(v.is_some(), egui::Button::new("Add").small());
                            if r.on_disabled_hover_text("Enter its voltage first").clicked()
                                && let Some(v) = v
                            {
                                add.push(Supply { net: net.clone(), voltage: v });
                            }
                        });
                        ui.end_row();
                    }
                });
                let known: Vec<Supply> = suggested
                    .iter()
                    .filter_map(|(n, g)| Some(Supply { net: n.clone(), voltage: self.suggest.get(n).copied().or(*g)? }))
                    .collect();
                if known.len() > 1 && ui.small_button("Add all").on_hover_text("Those with a voltage").clicked() {
                    add = known;
                }
                if !add.is_empty() {
                    setup.supplies.extend(add);
                    self.texts.clear();
                    self.si.clear();
                }
            }
            ui.add_space(4.0);
            label(ui, "Extra SPICE lines (.param, .options, models)");
            let id = egui::Id::unique("sim_extra");
            let focused = ui.memory(|m| m.has_focus(id));
            let buf = self.texts.entry(id).or_insert_with(|| setup.extra.clone());
            if !focused {
                buf.clone_from(&setup.extra);
            }
            let r = ui
                .add(egui::TextEdit::multiline(buf).id(id).code_editor().desired_rows(3).desired_width(f32::INFINITY));
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
        }
    }

    /// Run all, or Stop while it goes.
    fn run_all_button(&mut self, ui: &mut Ui, act: &mut Option<SimAct>) {
        if self.running_all() {
            if icon_button(ui, Icon::Stop, "Stop", theme::error()).on_hover_text("Stop Run all (⌘.)").clicked() {
                *act = Some(SimAct::Stop);
            }
        } else if icon_button(ui, Icon::Play, "Run all", theme::success())
            .on_hover_text("Run every simulation (⇧⌘R)")
            .clicked()
        {
            *act = Some(SimAct::RunAll);
        }
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

/// The signal of a result table row.
pub fn row_signal(row: &sim::report::Row) -> Option<sim::plot::Signal> {
    match (&row.net, row.current, row.power) {
        (Some(net), ..) => Some(sim::plot::Signal::Voltage(net.clone())),
        (None, Some((s, p)), _) => Some(sim::plot::Signal::Current(s, p)),
        (None, None, Some(c)) => Some(sim::plot::Signal::Power(c)),
        _ => None,
    }
}

/// A probe's label from the project (it may not be in a result): `V(OUT)`, `V(OUT,IN)`,
/// `I(R1.1)`, `P(R1)`.
pub fn probe_label(project: &Project, sig: &sim::plot::Signal) -> String {
    use sim::plot::Signal;
    match sig {
        Signal::Voltage(net) => format!("V({net})"),
        Signal::Diff(a, b) => format!("V({a},{b})"),
        Signal::Current(s, p) => {
            view::sim_overlay::pin_name(project, *s, *p).map_or("I(?)".into(), |(r, k)| format!("I({r}.{k})"))
        }
        Signal::Power(c) => {
            format!("P({})", project.schematic.components.get(*c).map_or("?", |c| c.refdes.as_str()))
        }
    }
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

/// The number of errors of an attempt (at least one: it failed).
fn error_count(attempt: Option<&Attempt>) -> usize {
    attempt.map_or(0, |a| a.problems.iter().filter(|p| p.level == Level::Error).count()).max(1)
}

/// Run all's status line: "Ran 4 simulations, 1 failed".
fn batch_text(ran: usize, failed: usize) -> String {
    let ran = if ran == 1 { "Ran 1 simulation".to_string() } else { format!("Ran {ran} simulations") };
    if failed == 0 { ran } else { format!("{ran}, {failed} failed") }
}

/// A new simulation of `kind` ([`KINDS`]) with its defaults and a name no other has. A DC
/// sweep needs a source to sweep.
pub fn new_analysis(setup: &Simulation, project: &Project, kind: &str) -> Result<Analysis, String> {
    let analysis = match kind {
        "Transient" => {
            AnalysisKind::Tran { stop: sim::netlist::suggested_stop(project), step: None, start: 0.0, max_step: None }
        }
        "AC" => AnalysisKind::Ac { start: 1.0, stop: 1e7, points_per_decade: 100 },
        "DC sweep" => match sources(project).first() {
            Some(&(id, _, _)) => AnalysisKind::Dc { source: id, start: 0.0, stop: 5.0, step: 0.05 },
            None => return Err("A DC sweep needs a source (VSource or ISource) to sweep".into()),
        },
        _ => AnalysisKind::Op,
    };
    Ok(Analysis::new(unique_name(setup, kind), analysis))
}

/// A copy of simulation `id` right after it, under a free name. Returns the copy's id.
pub fn duplicate(setup: &mut Simulation, id: Id) -> Option<Id> {
    let i = setup.analyses.iter().position(|a| a.id == id)?;
    let mut a = setup.analyses[i].clone();
    a.id = Id::new();
    a.name = unique_name(setup, &a.name);
    let copy = a.id;
    setup.analyses.insert(i + 1, a);
    Some(copy)
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

/// The voltage a supply net's name suggests (`+5V` → 5, `3V3` → 3.3, `-12V` → −12),
/// none for names without one (`VCC`).
pub fn supply_voltage(net: &str) -> Option<f64> {
    si::parse(net.trim_start_matches('+'))
}

/// Power port nets (not ground) that neither a supply nor a voltage source's pin
/// powers, with the voltage their name suggests.
pub fn suggested_supplies(doc: &Document) -> Vec<(String, Option<f64>)> {
    let p = doc.project();
    let conn = doc.connectivity();
    let setup = &p.schematic.simulation;
    let mut sourced: HashSet<String> = HashSet::new();
    for (c, _, kind) in sources(p) {
        if kind != "V" {
            continue;
        }
        for s in p.schematic.symbols.iter().filter(|s| s.component == c) {
            let Some(sym) = editor_core::query::symbol_of_instance(p, s) else { continue };
            for pin in sym.pins.iter() {
                if let Some(n) = conn.net_of_pin(s.id, pin.id) {
                    sourced.insert(n.name.clone());
                }
            }
        }
    }
    power_nets(doc)
        .into_iter()
        .filter(|n| !sourced.contains(n) && !setup.supplies.iter().any(|s| &s.net == n))
        .map(|n| {
            let v = supply_voltage(&n);
            (n, v)
        })
        .collect()
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
    fn duplicates_go_right_after_their_original() {
        let mut s = Simulation::default();
        for name in ["Transient", "AC"] {
            s.analyses.push(Analysis::new(name, AnalysisKind::Op));
        }
        let first = s.analyses[0].id;
        let copy = duplicate(&mut s, first).unwrap();
        let names: Vec<&str> = s.analyses.iter().map(|a| a.name.as_str()).collect();
        assert_eq!(names, ["Transient", "Transient 2", "AC"]);
        assert_eq!(s.analyses[1].id, copy);
        assert_eq!(duplicate(&mut s, Id::new()), None);
    }

    #[test]
    fn new_analyses_need_a_source_for_dc_sweeps() {
        let project = view::demo::demo_project();
        let setup = Simulation::default();
        for (kind, _) in KINDS {
            let a = new_analysis(&setup, &project, kind).unwrap();
            assert_eq!(a.name, kind);
            assert_eq!(a.kind.label(), kind);
        }
        let empty = Project::new("empty");
        assert!(new_analysis(&setup, &empty, "DC sweep").is_err());
    }

    #[test]
    fn tree_details_say_queued_failed_and_outdated() {
        let mut panel = SimPanel::default();
        let mut setup = Simulation::default();
        let a = Analysis::new("Transient", AnalysisKind::Op);
        let b = Analysis::new("AC", AnalysisKind::Op);
        setup.analyses = vec![a.clone(), b.clone()];
        assert_eq!(panel.tree_detail(&a).text, "", "never run");
        panel.failed.insert(a.id, 2);
        panel.stale.insert(b.id);
        assert_eq!(panel.tree_detail(&a), crate::project_panel::AnalysisDetail { text: "2 errors".into(), warn: true });
        assert_eq!(panel.tree_detail(&b).text, crate::project_panel::OUTDATED);
        panel.run_all(&setup);
        assert!(panel.running_all());
        assert_eq!(panel.tree_detail(&a).text, "queued");
        panel.stop();
        assert!(!panel.running_all());
        assert_eq!(panel.tree_detail(&b).text, crate::project_panel::OUTDATED);
    }

    #[test]
    fn run_all_says_how_it_went() {
        assert_eq!(batch_text(1, 0), "Ran 1 simulation");
        assert_eq!(batch_text(4, 1), "Ran 4 simulations, 1 failed");
    }

    #[test]
    fn supply_voltages_from_names() {
        assert_eq!(supply_voltage("+5V"), Some(5.0));
        assert_eq!(supply_voltage("3V3"), Some(3.3));
        assert_eq!(supply_voltage("-12V"), Some(-12.0));
        assert_eq!(supply_voltage("VCC"), None);
    }

    #[test]
    fn supplies_suggested_for_power_nets_without_a_source() {
        let mut doc = Document::new(view::demo::demo_project());
        doc.update_connectivity(false);
        assert!(suggested_supplies(&doc).is_empty(), "V1 powers +5V");
        let v1 = doc.project().schematic.components.iter().find(|c| c.refdes == "V1").unwrap().id;
        let symbols: Vec<ObjectId> = doc
            .project()
            .schematic
            .symbols
            .iter()
            .filter(|s| s.component == v1)
            .map(|s| ObjectId::Symbol(s.id))
            .collect();
        doc.execute(editor_core::Command::Delete { ids: symbols }).unwrap();
        doc.update_connectivity(false);
        assert_eq!(suggested_supplies(&doc), vec![("+5V".to_string(), Some(5.0))]);
    }

    #[test]
    fn source_components() {
        let p = view::demo::demo_project();
        let s = sources(&p);
        assert_eq!(s.len(), 1, "the test bench's V1");
        assert_eq!((s[0].1.as_str(), s[0].2), ("V1", "V"));
    }
}
