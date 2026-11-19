//! Circuit simulation of the schematic.
//!
//! * [`netlist`]: the schematic as a SPICE netlist, with maps from nets and pins to SPICE
//!   names, and the problems found on the way (no ground, values that aren't numbers,
//!   parts without a model).
//! * [`ngspice`]: runs ngspice as a separate process (a [`Backend`]).
//! * [`raw`]: reads ngspice's raw result files.
//! * [`results`]: a run's vectors by net and pin; [`report`]: them as text and CSV.
//!
//! Values with SI prefixes are parsed and formatted by `model::si`.

pub mod netlist;
pub mod ngspice;
pub mod raw;
pub mod report;
pub mod results;
pub mod session;

use editor_core::connectivity::Connectivity;
use model::{Analysis, Project};

pub use netlist::{Level, Netlist, SimProblem};
pub use ngspice::{Cancel, Ngspice};
pub use raw::{Plot, Vector};
pub use results::SimResult;
pub use session::{Attempt, Finished, Outcome, Session, SharedBackend};

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
}

/// Runs a netlist. Implemented by [`Ngspice`] (a process per run); the shared library
/// comes later behind the same trait.
pub trait Backend {
    fn run(&self, netlist: &str, cancel: &Cancel) -> Result<Output, SimError>;
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
    let output = backend.run(&netlist.text, cancel)?;
    let plot = output.plots.into_iter().next().ok_or_else(|| SimError::Raw("no results".into()))?;
    Ok(SimResult { plot, netlist, log: output.log, warnings: output.warnings })
}
