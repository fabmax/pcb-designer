//! Runs simulations in the background for the app: one run at a time on a worker
//! thread, the last result of each analysis, and what went wrong in the last attempt.
//!
//! The netlist is built on the caller's thread (it needs the project), only the backend
//! runs on the worker. A result is stale when the netlist its analysis would get now
//! differs from the one it ran (checked once per document revision).
//!
//! A parameter sweep (`Analysis::step`) runs one netlist per value, on up to as many
//! workers as there are cores (one when the backend can't run in parallel); it's stale
//! by its base netlist (without the step).
//!
//! A single run streams into a [`Live`] plot when the backend does (libngspice):
//! [`live`](Session::live) is it as a result, refreshed at most ten times a second.
//! [`stop`](Session::stop) keeps what a single run has so far (`Finished::stopped`).
//!
//! Backends that can't block (the browser's, [`Backend::start`]) aren't run on threads:
//! [`poll`](Session::poll) polls their runs and starts a sweep's next steps.

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use web_time::{Duration, Instant};

use editor_core::connectivity::Connectivity;
use model::{Analysis, Id, Probe, Project, TuneParam};

use crate::netlist::{self, Level, Netlist, SimProblem};
use crate::tune::Overrides;
use crate::{Backend, Cancel, Live, Output, Pending, SimError, SimResult, report};

/// A backend the worker thread can own.
pub type SharedBackend = Arc<dyn Backend + Send + Sync>;

/// A finished run of an analysis.
#[derive(Clone, Debug)]
pub struct Finished {
    /// The result (of a parameter sweep: the first step's).
    pub result: SimResult,
    /// A parameter sweep's parameter and its values (one per step, in order).
    pub step: Option<(TuneParam, Vec<f64>)>,
    /// The results of the steps after the first.
    pub rest: Vec<SimResult>,
    /// The netlist the stale check compares (a sweep's without the step).
    pub base: String,
    pub duration: Duration,
    /// The summary table ([`report::rows`]).
    pub rows: Vec<report::Row>,
    /// Counts runs of the session: a new result has a new number (for caches).
    pub run: u64,
    /// Current probes measured in this run only, beyond the analysis' (an agent's).
    pub extra: Vec<Probe>,
    /// The tuned values it ran with.
    pub overrides: Overrides,
    /// Stopped before the end: the points up to there.
    pub stopped: bool,
    /// The x range to show of a live or stopped run: the whole analysis (the plot doesn't
    /// jump as it grows, and shows where it stopped).
    pub range: Option<(f64, f64)>,
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

/// A step's output, once its worker ran it.
type Outputs = Arc<Mutex<Vec<Option<Result<Output, SimError>>>>>;

struct Run {
    analysis: Id,
    started: Instant,
    cancel: Cancel,
    /// The netlist of each step (one without a sweep).
    netlists: Vec<Netlist>,
    step: Option<(TuneParam, Vec<f64>)>,
    base: String,
    extra: Vec<Probe>,
    overrides: Overrides,
    outputs: Outputs,
    steps: Steps,
    /// The single run's points so far, and them as a result (with the revision and
    /// time it was made).
    live: Live,
    shown: Option<(u64, Instant, Finished)>,
    range: Option<(f64, f64)>,
}

/// How a run's steps run.
enum Steps {
    /// Threads running [`Backend::run`], taking the next step until none is left.
    Threads(Vec<JoinHandle<()>>),
    /// Runs [`Backend::start`]ed, up to `slots` at once; [`Session::poll`] polls them and
    /// starts the next steps.
    Polled {
        backend: SharedBackend,
        texts: Arc<Vec<String>>,
        /// The next step to start.
        next: usize,
        going: Vec<(usize, Box<dyn Pending + Send>)>,
        slots: usize,
    },
}

impl Steps {
    /// Polls started runs, starts the next steps while there are free slots. Returns
    /// whether all ended.
    fn advance(&mut self, outputs: &Outputs, cancel: &Cancel, live: &Live, single: bool) -> bool {
        let Steps::Polled { backend, texts, next, going, slots } = self else {
            let Steps::Threads(workers) = self else { unreachable!() };
            return workers.iter().all(|w| w.is_finished());
        };
        going.retain_mut(|(k, run)| {
            let Some(out) = run.poll() else { return true };
            // A failed step ends the sweep.
            if out.is_err() {
                cancel.cancel();
            }
            outputs.lock().expect("outputs")[*k] = Some(out);
            false
        });
        while going.len() < *slots && *next < texts.len() && !cancel.is_cancelled() {
            let live = if single { live.clone() } else { Live::default() };
            match backend.start(&texts[*next], cancel, &live) {
                Some(run) => going.push((*next, run)),
                None => {
                    let message = "the backend stopped starting runs".to_string();
                    outputs.lock().expect("outputs")[*next] = Some(Err(SimError::NotFound(message)));
                    cancel.cancel();
                }
            }
            *next += 1;
        }
        going.is_empty() && (*next >= texts.len() || cancel.is_cancelled())
    }
}

/// How often a live result is refreshed.
const LIVE_EVERY: Duration = Duration::from_millis(100);

/// The x range of an analysis' plot (`None` for an operating point).
pub fn x_range(analysis: &Analysis) -> Option<(f64, f64)> {
    match &analysis.kind {
        model::AnalysisKind::Op => None,
        model::AnalysisKind::Tran { stop, start, .. } => Some((*start, *stop)),
        model::AnalysisKind::Ac { start, stop, .. } => Some((*start, *stop)),
        model::AnalysisKind::Dc { start, stop, .. } => Some((start.min(*stop), start.max(*stop))),
    }
}

impl Finished {
    /// Each step's value (`None` without a sweep) and result.
    pub fn results(&self) -> Vec<(Option<f64>, &SimResult)> {
        match &self.step {
            None => vec![(None, &self.result)],
            Some((_, values)) => {
                values.iter().map(|v| Some(*v)).zip(std::iter::once(&self.result).chain(&self.rest)).collect()
            }
        }
    }
}

#[derive(Default)]
pub struct Session {
    run: Option<Run>,
    results: HashMap<Id, Finished>,
    attempt: Option<Attempt>,
    /// Analysis → (revision and overrides checked, stale).
    stale: HashMap<Id, (u64, Overrides, bool)>,
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
        self.start_with(project, conn, analysis, &[], &Overrides::default(), backend)
    }

    /// [`start`](Self::start), measuring the currents of `extra` probes as well (for this
    /// run only: the result isn't stale for them), with components `overrides`.
    pub fn start_with(
        &mut self,
        project: &Project,
        conn: &Connectivity,
        analysis: &Analysis,
        extra: &[Probe],
        overrides: &Overrides,
        backend: SharedBackend,
    ) -> bool {
        self.abandon();
        let analysis = &with_probes(analysis, extra);
        let base = netlist::build_with(project, conn, analysis, overrides);
        let mut attempt = Attempt {
            analysis: analysis.id,
            name: analysis.name.clone(),
            netlist: base.text.clone(),
            problems: base.problems.clone(),
            log: String::new(),
            outcome: Outcome::Running,
        };
        // A sweep: a netlist per value.
        let mut step = None;
        let netlists = match &analysis.step {
            None => vec![base.clone()],
            Some(s) => {
                let values = s.values.points();
                let mut out = Vec::new();
                for v in &values {
                    let Some(o) = overrides.try_with(project, &s.param, *v) else {
                        attempt.problems.push(SimProblem {
                            level: Level::Error,
                            message: "the swept parameter no longer exists".into(),
                            objects: Vec::new(),
                        });
                        break;
                    };
                    out.push(netlist::build_with(project, conn, analysis, &o));
                }
                if let Some(first) = out.first() {
                    attempt.netlist = first.text.clone();
                }
                step = Some((s.param.clone(), values));
                out
            }
        };
        if base.has_errors() || attempt.problems.iter().any(|p| p.level == Level::Error) || netlists.is_empty() {
            attempt.outcome = Outcome::NetlistErrors;
            self.attempt = Some(attempt);
            return false;
        }
        self.attempt = Some(attempt);
        let cancel = Cancel::default();
        let texts: Arc<Vec<String>> = Arc::new(netlists.iter().map(|n| n.text.clone()).collect());
        let outputs: Outputs = Arc::new(Mutex::new((0..texts.len()).map(|_| None).collect()));
        let cores = if backend.parallel() { std::thread::available_parallelism().map_or(4, |n| n.get()) } else { 1 };
        // Only a single run streams (a sweep's steps would mix).
        let live = Live::default();
        let single = step.is_none();
        let first_live = if single { live.clone() } else { Live::default() };
        let steps = match backend.start(&texts[0], &cancel, &first_live) {
            // A backend that can't block: polled.
            Some(first) => {
                let mut steps = Steps::Polled {
                    backend,
                    texts: texts.clone(),
                    next: 1,
                    going: vec![(0, first)],
                    slots: texts.len().min(cores),
                };
                steps.advance(&outputs, &cancel, &live, single);
                steps
            }
            None => Steps::Threads(threads(&texts, &outputs, &backend, &cancel, &live, single, cores)),
        };
        let (extra, overrides) = (extra.to_vec(), overrides.clone());
        let started = Instant::now();
        let base = base.text;
        let range = x_range(analysis);
        self.run = Some(Run {
            analysis: analysis.id,
            started,
            cancel,
            netlists,
            step,
            base,
            extra,
            overrides,
            outputs,
            steps,
            live,
            shown: None,
            range,
        });
        true
    }

    /// Stops the run in progress. A single run keeps what it has so far (once
    /// [`poll`](Self::poll) collects it), a sweep is dropped.
    pub fn stop(&mut self) {
        if let Some(run) = &self.run {
            run.cancel.cancel();
        }
    }

    /// Stops the run in progress and drops it (its worker ends on its own).
    fn abandon(&mut self) {
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

    /// How far the run is, 0…1 (backends that report it).
    pub fn progress(&self) -> Option<f64> {
        self.run.as_ref()?.live.progress()
    }

    /// The running `analysis`' points so far as a result (backends that stream).
    pub fn live(&self, analysis: Id) -> Option<&Finished> {
        let run = self.run.as_ref().filter(|r| r.analysis == analysis)?;
        run.shown.as_ref().map(|(_, _, f)| f)
    }

    /// Brings the live result up to date when points came and it's time (every 100 ms,
    /// less often for long runs: the plot redraws them all).
    fn refresh_live(&mut self) {
        let Some(run) = self.run.as_mut() else { return };
        let revision = run.live.revision();
        if let Some((r, at, f)) = &mut run.shown {
            let every = LIVE_EVERY.mul_f64((f.result.plot.len() as f64 / 200_000.0).max(1.0));
            if *r == revision || at.elapsed() < every {
                return;
            }
            run.live.update(&mut f.result.plot);
            f.rows = report::rows(&f.result);
            f.duration = run.started.elapsed();
            self.runs += 1;
            f.run = self.runs;
            *r = revision;
            *at = Instant::now();
            return;
        }
        let Some(plot) = run.live.plot() else { return };
        self.runs += 1;
        let Some(netlist) = run.netlists.first().cloned() else { return };
        let result = SimResult { plot, netlist, log: String::new(), warnings: Vec::new() };
        let finished = Finished {
            rows: report::rows(&result),
            result,
            step: None,
            rest: Vec::new(),
            base: run.base.clone(),
            duration: run.started.elapsed(),
            run: self.runs,
            extra: run.extra.clone(),
            overrides: run.overrides.clone(),
            stopped: false,
            range: run.range,
        };
        run.shown = Some((revision, Instant::now(), finished));
    }

    /// Collects a finished run. Returns its outcome when one finished since the last call.
    pub fn poll(&mut self) -> Option<Outcome> {
        let run = self.run.as_mut()?;
        let single = run.step.is_none();
        if !run.steps.advance(&run.outputs, &run.cancel, &run.live, single) {
            self.refresh_live();
            return None;
        }
        let run = self.run.take()?;
        let duration = run.started.elapsed();
        let panicked = match run.steps {
            Steps::Threads(workers) => workers.into_iter().any(|w| w.join().is_err()),
            Steps::Polled { .. } => false,
        };
        let outputs = std::mem::take(&mut *run.outputs.lock().expect("outputs"));
        let stepped = run.step.is_some();
        let labels: Vec<String> = match &run.step {
            Some((_, values)) => values.iter().map(|v| format!("step {}: ", crate::plot::label(*v, ""))).collect(),
            None => vec![String::new()],
        };
        // The first failure (by step), else every step's output.
        let mut done = Vec::new();
        let mut failure = None;
        let mut stopped = false;
        for (k, out) in outputs.into_iter().enumerate() {
            match out {
                // A stopped sweep step: the sweep is incomplete.
                Some(Ok(o)) if o.stopped && stepped => {
                    failure.get_or_insert((k, SimError::Cancelled));
                }
                Some(Ok(o)) => {
                    stopped |= o.stopped;
                    done.push(o);
                }
                Some(Err(SimError::Cancelled)) | None => {
                    failure.get_or_insert((k, SimError::Cancelled));
                }
                Some(Err(e)) => {
                    // A real error wins over the cancels it caused.
                    if failure.as_ref().is_none_or(|(_, f)| matches!(f, SimError::Cancelled)) {
                        failure = Some((k, e));
                    }
                }
            }
        }
        if panicked {
            failure =
                Some((0, SimError::Failed { message: "the simulation thread panicked".into(), log: String::new() }));
        }
        let attempt = self.attempt.as_mut()?;
        let line = |level, message: &str| SimProblem { level, message: message.to_string(), objects: Vec::new() };
        if let Some((k, e)) = failure {
            let prefix = if stepped { labels[k].as_str() } else { "" };
            attempt.outcome = match e {
                SimError::Cancelled => Outcome::Cancelled,
                SimError::Failed { message, log } => {
                    attempt.log = log;
                    attempt.problems.extend(message.lines().map(|l| line(Level::Error, &format!("{prefix}{l}"))));
                    Outcome::Failed(format!("{prefix}{message}"))
                }
                e => {
                    let message = format!("{prefix}{e}");
                    attempt.problems.push(line(Level::Error, &message));
                    Outcome::Failed(message)
                }
            };
            return Some(attempt.outcome.clone());
        }
        let mut results = Vec::new();
        for (k, (out, netlist)) in done.into_iter().zip(run.netlists).enumerate() {
            if k == 0 {
                attempt.log = out.log.clone();
            }
            let prefix = if stepped { labels[k].as_str() } else { "" };
            attempt.problems.extend(out.warnings.iter().map(|w| line(Level::Warning, &format!("{prefix}{w}"))));
            let Some(plot) = out.plots.into_iter().next() else {
                let message = format!("{prefix}ngspice wrote no results");
                attempt.problems.push(line(Level::Error, &message));
                attempt.outcome = Outcome::Failed(message);
                return Some(attempt.outcome.clone());
            };
            results.push(SimResult { plot, netlist, log: out.log, warnings: out.warnings });
        }
        let mut results = results.into_iter();
        let result = results.next()?;
        let rows = report::rows(&result);
        self.runs += 1;
        let finished = Finished {
            result,
            step: run.step,
            rest: results.collect(),
            base: run.base,
            duration,
            rows,
            run: self.runs,
            extra: run.extra,
            overrides: run.overrides,
            stopped,
            range: if stopped { run.range } else { None },
        };
        self.results.insert(run.analysis, finished);
        self.stale.remove(&run.analysis);
        attempt.outcome = Outcome::Done;
        Some(Outcome::Done)
    }

    /// The last finished run of an analysis.
    pub fn result(&self, analysis: Id) -> Option<&Finished> {
        self.results.get(&analysis)
    }

    /// The last run started (or refused for netlist errors).
    pub fn attempt(&self) -> Option<&Attempt> {
        self.attempt.as_ref()
    }

    /// Whether `analysis`' result no longer matches the schematic with `overrides` (the
    /// tuned values now): its netlist would differ. False without a result. Rebuilds the
    /// netlist once per `revision` and overrides.
    pub fn is_stale(
        &mut self,
        project: &Project,
        conn: &Connectivity,
        analysis: &Analysis,
        overrides: &Overrides,
        revision: u64,
    ) -> bool {
        let Some(done) = self.results.get(&analysis.id) else { return false };
        if let Some((rev, o, stale)) = self.stale.get(&analysis.id)
            && *rev == revision
            && o == overrides
        {
            return *stale;
        }
        let a = with_probes(analysis, &done.extra);
        let stale = netlist::build_with(project, conn, &a, overrides).text != done.base;
        self.stale.insert(analysis.id, (revision, overrides.clone(), stale));
        stale
    }

    /// Drops the results of analyses no longer in the setup.
    pub fn retain(&mut self, keep: impl Fn(Id) -> bool) {
        self.results.retain(|id, _| keep(*id));
        self.stale.retain(|id, _| keep(*id));
    }

    /// Forgets everything (another project was opened).
    pub fn clear(&mut self) {
        self.abandon();
        *self = Session::default();
    }
}

impl Drop for Session {
    /// A run going ends with the session (the library would run on otherwise).
    fn drop(&mut self) {
        self.abandon();
    }
}

/// `analysis` with `extra` probes added.
fn with_probes(analysis: &Analysis, extra: &[Probe]) -> Analysis {
    let mut a = analysis.clone();
    a.probes.extend(extra.iter().cloned());
    a
}

/// Threads running `texts` on `backend` (up to `cores`), each taking the next step until
/// none is left or the run is cancelled.
fn threads(
    texts: &Arc<Vec<String>>,
    outputs: &Outputs,
    backend: &SharedBackend,
    cancel: &Cancel,
    live: &Live,
    single: bool,
    cores: usize,
) -> Vec<JoinHandle<()>> {
    let next = Arc::new(AtomicUsize::new(0));
    (0..texts.len().min(cores))
        .map(|_| {
            let (texts, outputs, next, backend, c) =
                (texts.clone(), outputs.clone(), next.clone(), backend.clone(), cancel.clone());
            let live = if single { live.clone() } else { Live::default() };
            std::thread::Builder::new()
                .name("simulation".into())
                .spawn(move || {
                    loop {
                        let k = next.fetch_add(1, Ordering::SeqCst);
                        if k >= texts.len() || c.is_cancelled() {
                            break;
                        }
                        let out = backend.run(&texts[k], &c, &live);
                        // A failed step ends the sweep.
                        if out.is_err() {
                            c.cancel();
                        }
                        outputs.lock().expect("outputs")[k] = Some(out);
                    }
                })
                .expect("spawn simulation thread")
        })
        .collect()
}
