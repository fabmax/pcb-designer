//! Circuit simulation of the schematic.
//!
//! * [`netlist`]: the schematic as a SPICE netlist, with maps from nets and pins to SPICE
//!   names, and the problems found on the way (no ground, values that aren't numbers,
//!   parts without a model).
//! * [`shared`]: runs ngspice as a shared library (libngspice, loaded at run time; the
//!   app bundles one), streaming results while it runs; [`worker`]: the same library in
//!   a process of our own; [`browser`]: ngspice compiled to WebAssembly, in Web Workers.
//!   [`Engine`] picks one.
//! * [`raw`]: reads ngspice's raw result files.
//! * [`results`]: a run's vectors by net and pin; [`report`]: them as text and CSV.
//!
//! Values with SI prefixes are parsed and formatted by `model::si`.

// Its message decoding is used by the wasm build (and tests) only.
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub mod browser;
pub mod measure;
pub mod netlist;
pub mod ngspice;
pub mod plot;
pub mod raw;
pub mod report;
pub mod results;
pub mod session;
pub mod shared;
pub mod spectrum;
pub mod tune;
pub mod worker;

use editor_core::connectivity::Connectivity;
use std::sync::{Arc, Mutex};

use model::{Analysis, Project};

pub use netlist::{Level, Netlist, SimProblem};
pub use ngspice::Cancel;
pub use raw::{Plot, Vector};
pub use results::SimResult;
pub use session::{Attempt, Finished, Outcome, Session, SharedBackend};
pub use shared::{Libngspice, bundled_library};
pub use worker::{Worker, WorkerCommand};

#[derive(Debug, thiserror::Error)]
pub enum SimError {
    #[error("{0}")]
    NotFound(String),
    #[error("{0}")]
    Io(String),
    /// The netlist has errors; ngspice wasn't started.
    #[error("{}", .0.iter().filter(|p| p.level == Level::Error).map(|p| p.message.as_str()).collect::<Vec<_>>().join("\n"))]
    Netlist(Vec<SimProblem>),
    /// ngspice ran and failed.
    #[error("{message}")]
    Failed { message: String, log: String },
    #[error("cancelled")]
    Cancelled,
    #[error("unreadable simulation results: {0}")]
    Raw(String),
}

impl SimError {
    pub(crate) fn io(e: std::io::Error) -> SimError {
        SimError::Io(e.to_string())
    }
}

/// What a backend returns: the plots in analysis order and the simulator's log.
#[derive(Clone, Debug, Default)]
pub struct Output {
    pub plots: Vec<Plot>,
    pub log: String,
    pub warnings: Vec<String>,
    /// Cancelled part way: the plot has the points up to there.
    pub stopped: bool,
}

/// A run going on elsewhere, polled rather than waited for ([`Backend::start`]).
pub trait Pending {
    /// The output once the run has ended. Cancelled, it ends at the next poll: with the
    /// points so far (`stopped`) or [`SimError::Cancelled`].
    fn poll(&mut self) -> Option<Result<Output, SimError>>;
}

/// Runs a netlist: [`Libngspice`] (in-process), a [`Worker`] (a process per run) or, in
/// the browser, [`browser::Ngspice`].
pub trait Backend {
    /// Runs `netlist`. Backends that stream put the points into `live` as they come.
    /// Cancelled, a backend returns what it has so far (`stopped`) or
    /// [`SimError::Cancelled`].
    fn run(&self, netlist: &str, cancel: &Cancel, live: &Live) -> Result<Output, SimError>;

    /// Starts a run without blocking, for backends that can't block (the browser has no
    /// threads to wait on). `None`: [`run`](Self::run) it on a thread.
    fn start(&self, _netlist: &str, _cancel: &Cancel, _live: &Live) -> Option<Box<dyn Pending + Send>> {
        None
    }

    /// Whether several runs can go at once (a sweep's steps on several workers).
    fn parallel(&self) -> bool {
        true
    }

    /// "libngspice-47, shared library".
    fn describe(&self) -> String;
}

/// The plot of a running analysis so far, and its progress (filled by backends that
/// stream).
#[derive(Clone, Debug, Default)]
pub struct Live(Arc<Mutex<LiveData>>);

#[derive(Debug, Default)]
struct LiveData {
    plot: Plot,
    progress: Option<f64>,
    /// Counts changes (points added).
    revision: u64,
}

impl Live {
    fn data(&self) -> std::sync::MutexGuard<'_, LiveData> {
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// A new plot begins: its (empty) vectors.
    pub fn begin(&self, plot: Plot) {
        let mut d = self.data();
        d.plot = plot;
        d.revision += 1;
    }

    /// One point: a value (real, imaginary) per vector, in the plot's order.
    pub fn push(&self, point: &[(f64, f64)]) {
        let mut d = self.data();
        if point.len() != d.plot.vectors.len() {
            return;
        }
        for (v, (re, im)) in d.plot.vectors.iter_mut().zip(point) {
            v.re.push(*re);
            if let Some(i) = &mut v.im {
                i.push(*im);
            }
        }
        d.revision += 1;
    }

    /// How far the analysis is, 0…1.
    pub fn set_progress(&self, p: f64) {
        self.data().progress = Some(p);
    }

    pub fn progress(&self) -> Option<f64> {
        self.data().progress
    }

    pub fn revision(&self) -> u64 {
        self.data().revision
    }

    /// The plot so far (`None` before any point).
    pub fn plot(&self) -> Option<Plot> {
        let d = self.data();
        (!d.plot.is_empty()).then(|| d.plot.clone())
    }

    /// Brings `plot` (an earlier copy) up to date: appends the points it lacks (copies
    /// the whole plot when it's another one).
    pub fn update(&self, plot: &mut Plot) {
        let d = self.data();
        let same = plot.name == d.plot.name
            && plot.vectors.len() == d.plot.vectors.len()
            && plot.vectors.iter().zip(&d.plot.vectors).all(|(a, b)| a.name == b.name)
            && plot.len() <= d.plot.len();
        if !same {
            *plot = d.plot.clone();
            return;
        }
        let from = plot.len();
        for (to, v) in plot.vectors.iter_mut().zip(&d.plot.vectors) {
            to.re.extend_from_slice(&v.re[from..]);
            if let (Some(to), Some(im)) = (&mut to.im, &v.im) {
                to.extend_from_slice(&im[from..]);
            }
        }
    }
}

/// Where ngspice is and how to run it (the app's settings).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct EngineSettings {
    /// The shared library (`None`: `NGSPICE_LIBRARY`, else the bundled one).
    pub library: Option<std::path::PathBuf>,
    /// Every run in a worker process (a crashing model can't take the app down).
    pub process_only: bool,
    /// How to start a worker process (`None`: none; sweeps then run one step at a time).
    pub worker: Option<WorkerCommand>,
}

/// Which backend runs simulations: the library in this process (single runs and the
/// tuner, streaming), worker processes for parallel sweeps, once ngspice exited in this
/// process, and for everything with `process_only`.
#[derive(Clone, Debug, Default)]
pub struct Engine {
    pub library: Option<Libngspice>,
    pub worker: Option<Worker>,
    /// Why there is no ngspice to run.
    pub error: Option<String>,
    /// ngspice in Web Workers (the browser build, when it comes with it).
    pub browser: bool,
}

impl Engine {
    /// Finds the library ([`Libngspice::locate`]) and loads it here, unless
    /// `process_only` with a worker.
    pub fn find(settings: &EngineSettings) -> Engine {
        if cfg!(target_arch = "wasm32") {
            if browser::available() {
                return Engine { browser: true, ..Engine::default() };
            }
            let error = "This build of the browser version doesn't include ngspice".to_string();
            return Engine { error: Some(error), ..Engine::default() };
        }
        let path = match Libngspice::locate(settings.library.as_deref()) {
            Ok(p) => p,
            Err(e) => return Engine { error: Some(e.to_string()), ..Engine::default() },
        };
        let worker = settings.worker.clone().map(|c| Worker::new(c, path.clone()));
        if settings.process_only && worker.is_some() {
            return Engine { worker, ..Engine::default() };
        }
        match Libngspice::load(&path) {
            Ok(l) => Engine { library: Some(l), worker, ..Engine::default() },
            Err(e) => Engine { error: Some(e.to_string()), ..Engine::default() },
        }
    }

    fn use_worker(&self, sweep: bool) -> bool {
        self.worker.is_some() && (sweep || self.library.is_none() || Libngspice::exited())
    }

    /// The backend for `analysis` (see [`Engine`]).
    pub fn backend(&self, analysis: &Analysis) -> Result<session::SharedBackend, String> {
        if let Some(e) = &self.error {
            return Err(e.clone());
        }
        if self.browser {
            return Ok(Arc::new(browser::Ngspice));
        }
        if self.use_worker(analysis.step.is_some()) {
            return Ok(Arc::new(self.worker.clone().expect("worker")));
        }
        let library = self.library.clone().ok_or_else(|| "ngspice is not available".to_string())?;
        Ok(Arc::new(library))
    }

    /// What runs single analyses: "libngspice-47, shared library (bundled)".
    pub fn describe(&self) -> Option<String> {
        if self.browser {
            return Some(browser::Ngspice.describe());
        }
        if self.use_worker(false) {
            return self.worker.as_ref().map(Worker::describe);
        }
        self.library.as_ref().map(Libngspice::describe)
    }
}

/// Builds the netlist of `analysis` and runs it. Netlist errors stop it before the
/// backend starts; warnings stay in the result's netlist.
/// For tests that need ngspice: without it they skip with a note, so a fresh checkout's
/// tests pass. CI, which builds the bundled one, sets `REQUIRE_NGSPICE`: there a skip
/// would hide a broken build, so it fails instead.
#[doc(hidden)]
pub fn skip_without_ngspice(why: impl std::fmt::Display) {
    if std::env::var_os("REQUIRE_NGSPICE").is_some() {
        panic!("ngspice is required (REQUIRE_NGSPICE): {why}");
    }
    eprintln!("skipped: {why}");
}

pub fn simulate(
    project: &Project,
    conn: &Connectivity,
    analysis: &Analysis,
    backend: &dyn Backend,
    cancel: &Cancel,
) -> Result<SimResult, SimError> {
    let netlist = netlist::build(project, conn, analysis);
    if netlist.has_errors() {
        return Err(SimError::Netlist(netlist.problems));
    }
    let output = backend.run(&netlist.text, cancel, &Live::default())?;
    let plot = output.plots.into_iter().next().ok_or_else(|| SimError::Raw("no results".into()))?;
    Ok(SimResult { plot, netlist, log: output.log, warnings: output.warnings })
}
