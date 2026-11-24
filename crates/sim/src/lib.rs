//! Circuit simulation of the schematic.
//!
//! * [`netlist`]: the schematic as a SPICE netlist, with maps from nets and pins to SPICE
//!   names, and the problems found on the way (no ground, values that aren't numbers,
//!   parts without a model).
//! * [`ngspice`]: runs ngspice as a separate process (a [`Backend`]); [`shared`]: as a
//!   shared library (libngspice, loaded at run time), streaming results while it runs.
//!   [`Engine`] picks one.
//! * [`raw`]: reads ngspice's raw result files.
//! * [`results`]: a run's vectors by net and pin; [`report`]: them as text and CSV.
//!
//! Values with SI prefixes are parsed and formatted by `model::si`.

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

use editor_core::connectivity::Connectivity;
use std::sync::{Arc, Mutex};

use model::{Analysis, Project};

pub use netlist::{Level, Netlist, SimProblem};
pub use ngspice::{Cancel, Ngspice};
pub use raw::{Plot, Vector};
pub use results::SimResult;
pub use session::{Attempt, Finished, Outcome, Session, SharedBackend};
pub use shared::{Libngspice, bundled_library};

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

/// Runs a netlist: [`Ngspice`] (a process per run) or [`Libngspice`] (in-process).
pub trait Backend {
    /// Runs `netlist`. Backends that stream put the points into `live` as they come.
    /// Cancelled, a backend returns what it has so far (`stopped`) or
    /// [`SimError::Cancelled`].
    fn run(&self, netlist: &str, cancel: &Cancel, live: &Live) -> Result<Output, SimError>;

    /// Whether several runs can go at once (a sweep's steps on several workers).
    fn parallel(&self) -> bool {
        true
    }

    /// "ngspice-47, process".
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
    /// The executable (`None`: `PATH` and the usual places).
    pub executable: Option<std::path::PathBuf>,
    /// The shared library (`None`: the usual places).
    pub library: Option<std::path::PathBuf>,
    /// Never load the library: every run in a process (a crashing model can't take the
    /// app down).
    pub process_only: bool,
}

/// Which backend runs simulations: the shared library when found (single runs and the
/// tuner), processes for parallel sweeps and as the fallback.
#[derive(Clone, Debug, Default)]
pub struct Engine {
    pub process: Option<Ngspice>,
    pub library: Option<Libngspice>,
    /// Why neither was found.
    pub error: Option<String>,
}

impl Engine {
    /// Finds the executable and, unless `process_only`, the library. A configured path
    /// that doesn't work is an error even when the other is found.
    pub fn find(settings: &EngineSettings) -> Engine {
        let process = Ngspice::find(settings.executable.as_deref());
        let library = if settings.process_only {
            Err(SimError::NotFound("process only".into()))
        } else {
            Libngspice::find(settings.library.as_deref())
        };
        let error = match (&process, &library) {
            (Err(e), _) if settings.executable.is_some() => Some(e.to_string()),
            (_, Err(e)) if settings.library.is_some() && !settings.process_only => Some(e.to_string()),
            (Err(e), Err(_)) => Some(e.to_string()),
            _ => None,
        };
        Engine { process: process.ok(), library: library.ok(), error }
    }

    /// The backend for `analysis`: the library, but for sweeps when the executable is
    /// there too, and once ngspice exited in this process ([`Libngspice::exited`]).
    pub fn backend(&self, analysis: &Analysis) -> Result<session::SharedBackend, String> {
        let process_first = analysis.step.is_some() || Libngspice::exited();
        let library = self.library.clone().map(|l| Arc::new(l) as session::SharedBackend);
        let process = self.process.clone().map(|p| Arc::new(p) as session::SharedBackend);
        if let Some(e) = &self.error {
            return Err(e.clone());
        }
        let pick = if process_first { process.or(library) } else { library.or(process) };
        pick.ok_or_else(|| "ngspice is not installed".into())
    }

    /// What runs single analyses: "libngspice-47, shared library".
    pub fn describe(&self) -> Option<String> {
        match (&self.library, &self.process) {
            (Some(_), Some(p)) if Libngspice::exited() => Some(p.describe()),
            (Some(l), _) => Some(l.describe()),
            (None, Some(p)) => Some(p.describe()),
            _ => None,
        }
    }
}

/// Builds the netlist of `analysis` and runs it. Netlist errors stop it before the
/// backend starts; warnings stay in the result's netlist.
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
