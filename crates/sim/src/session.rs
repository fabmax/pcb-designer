//! Runs simulations in the background for the app: one run at a time on a worker
//! thread, the last result of each analysis, and what went wrong in the last attempt.
//!
//! The netlist is built on the caller's thread (it needs the project), only the backend
//! runs on the worker. A result is stale when the netlist its analysis would get now
//! differs from the one it ran (checked once per document revision).

use std::collections::HashMap;
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use editor_core::connectivity::Connectivity;
use model::{Analysis, Id, Project};

use crate::netlist::{self, Level, Netlist, SimProblem};
use crate::{Backend, Cancel, Output, SimError, SimResult, report};

/// A backend the worker thread can own.
pub type SharedBackend = Arc<dyn Backend + Send + Sync>;

/// A finished run of an analysis.
#[derive(Clone, Debug)]
pub struct Finished {
    pub result: SimResult,
    pub duration: Duration,
    /// The summary table ([`report::rows`]).
    pub rows: Vec<report::Row>,
    /// Counts runs of the session: a new result has a new number (for caches).
    pub run: u64,
}

/// The last run that was started (or refused): its problems, netlist and log.
#[derive(Clone, Debug)]
pub struct Attempt {
    pub analysis: Id,
    pub name: String,
    pub netlist: String,
    /// Netlist problems, then ngspice's errors and warnings (without objects).
    pub problems: Vec<SimProblem>,
    pub log: String,
    pub outcome: Outcome,
}

/// How an attempt ended.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum Outcome {
    #[default]
    Running,
    Done,
    /// The netlist had errors; ngspice wasn't started.
    NetlistErrors,
    Failed(String),
    Cancelled,
}

struct Run {
    analysis: Id,
    started: Instant,
    cancel: Cancel,
    netlist: Netlist,
    worker: JoinHandle<Result<Output, SimError>>,
}

#[derive(Default)]
pub struct Session {
    run: Option<Run>,
    results: HashMap<Id, Finished>,
    attempt: Option<Attempt>,
    /// Analysis → (revision checked, stale).
    stale: HashMap<Id, (u64, bool)>,
    runs: u64,
}

impl Session {
    /// Starts `analysis` (stopping a run in progress). Returns false if the netlist has
    /// errors (they're in [`attempt`](Self::attempt)).
    pub fn start(
        &mut self,
        project: &Project,
        conn: &Connectivity,
        analysis: &Analysis,
        backend: SharedBackend,
    ) -> bool {
        self.stop();
        let netlist = netlist::build(project, conn, analysis);
        let mut attempt = Attempt {
            analysis: analysis.id,
            name: analysis.name.clone(),
            netlist: netlist.text.clone(),
            problems: netlist.problems.clone(),
            log: String::new(),
            outcome: Outcome::Running,
        };
        if netlist.has_errors() {
            attempt.outcome = Outcome::NetlistErrors;
            self.attempt = Some(attempt);
            return false;
        }
        self.attempt = Some(attempt);
        let cancel = Cancel::default();
        let text = netlist.text.clone();
        let c = cancel.clone();
        let worker = std::thread::Builder::new()
            .name("simulation".into())
            .spawn(move || backend.run(&text, &c))
            .expect("spawn simulation thread");
        self.run = Some(Run { analysis: analysis.id, started: Instant::now(), cancel, netlist, worker });
        true
    }

    /// Stops the run in progress (its worker ends on its own; its result is dropped).
    pub fn stop(&mut self) {
        if let Some(run) = self.run.take() {
            run.cancel.cancel();
            if let Some(a) = &mut self.attempt {
                a.outcome = Outcome::Cancelled;
            }
        }
    }

    /// The analysis running and for how long.
    pub fn running(&self) -> Option<(Id, Duration)> {
        self.run.as_ref().map(|r| (r.analysis, r.started.elapsed()))
    }

    /// Collects a finished run. Returns its outcome when one finished since the last call.
    pub fn poll(&mut self) -> Option<Outcome> {
        if !self.run.as_ref()?.worker.is_finished() {
            return None;
        }
        let run = self.run.take()?;
        let duration = run.started.elapsed();
        let output = run.worker.join().unwrap_or_else(|_| {
            Err(SimError::Failed { message: "the simulation thread panicked".into(), log: String::new() })
        });
        let attempt = self.attempt.as_mut()?;
        let line = |level, message: &str| SimProblem { level, message: message.to_string(), objects: Vec::new() };
        attempt.outcome = match output {
            Ok(out) => {
                attempt.log = out.log.clone();
                attempt.problems.extend(out.warnings.iter().map(|w| line(Level::Warning, w)));
                match out.plots.into_iter().next() {
                    Some(plot) => {
                        let result = SimResult { plot, netlist: run.netlist, log: out.log, warnings: out.warnings };
                        let rows = report::rows(&result);
                        self.runs += 1;
                        let finished = Finished { result, duration, rows, run: self.runs };
                        self.results.insert(run.analysis, finished);
                        self.stale.remove(&run.analysis);
                        Outcome::Done
                    }
                    None => {
                        attempt.problems.push(line(Level::Error, "ngspice wrote no results"));
                        Outcome::Failed("ngspice wrote no results".into())
                    }
                }
            }
            Err(SimError::Cancelled) => Outcome::Cancelled,
            Err(SimError::Failed { message, log }) => {
                attempt.log = log;
                attempt.problems.extend(message.lines().map(|l| line(Level::Error, l)));
                Outcome::Failed(message)
            }
            Err(e) => {
                let message = e.to_string();
                attempt.problems.push(line(Level::Error, &message));
                Outcome::Failed(message)
            }
        };
        Some(attempt.outcome.clone())
    }

    /// The last finished run of an analysis.
    pub fn result(&self, analysis: Id) -> Option<&Finished> {
        self.results.get(&analysis)
    }

    /// The last run started (or refused for netlist errors).
    pub fn attempt(&self) -> Option<&Attempt> {
        self.attempt.as_ref()
    }

    /// Whether `analysis`' result no longer matches the schematic (its netlist would
    /// differ now). False without a result. Rebuilds the netlist once per `revision`.
    pub fn is_stale(&mut self, project: &Project, conn: &Connectivity, analysis: &Analysis, revision: u64) -> bool {
        let Some(done) = self.results.get(&analysis.id) else { return false };
        if let Some(&(rev, stale)) = self.stale.get(&analysis.id)
            && rev == revision
        {
            return stale;
        }
        let stale = netlist::build(project, conn, analysis).text != done.result.netlist.text;
        self.stale.insert(analysis.id, (revision, stale));
        stale
    }

    /// Drops the results of analyses no longer in the setup.
    pub fn retain(&mut self, keep: impl Fn(Id) -> bool) {
        self.results.retain(|id, _| keep(*id));
        self.stale.retain(|id, _| keep(*id));
    }

    /// Forgets everything (another project was opened).
    pub fn clear(&mut self) {
        self.stop();
        *self = Session::default();
    }
}
