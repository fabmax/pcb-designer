//! The app's simulation session, with a fake backend (no ngspice needed).

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use model::*;
use sim::{Backend, Cancel, Level, Outcome, Output, Plot, Session, SimError, Vector};

use crate::bench::Bench;

/// Answers with an operating point of `v(out) = 1`, or what `answer` says; waits for
/// `gate` to open (or a cancel).
#[derive(Default)]
struct Fake {
    gate: Arc<Mutex<bool>>,
    fail: Option<String>,
    netlists: Arc<Mutex<Vec<String>>>,
}

impl Backend for Fake {
    fn run(&self, netlist: &str, cancel: &Cancel, _: &sim::Live) -> Result<Output, SimError> {
        self.netlists.lock().unwrap().push(netlist.to_string());
        while !*self.gate.lock().unwrap() {
            if cancel.is_cancelled() {
                return Err(SimError::Cancelled);
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        if let Some(message) = &self.fail {
            return Err(SimError::Failed { message: message.clone(), log: "the log".into() });
        }
        let out = Vector { name: "v(out)".into(), kind: "voltage".into(), re: vec![1.0], im: None };
        let plot = Plot { name: "Operating Point".into(), complex: false, vectors: vec![out] };
        Ok(Output {
            plots: vec![plot],
            log: "the log".into(),
            warnings: vec!["warning: a hint".into()],
            stopped: false,
        })
    }

    fn describe(&self) -> String {
        "fake".into()
    }
}

fn divider() -> Bench {
    let mut b = Bench::new();
    b.source("VSource", Waveform::Dc { value: 5.0 }, 0.0, &["IN", "GND"]);
    b.place("Resistor", "10k", &["IN", "OUT"]);
    b.place("Resistor", "10k", &["OUT", "GND"]);
    b
}

fn wait(s: &mut Session) -> Outcome {
    let t = Instant::now();
    loop {
        if let Some(o) = s.poll() {
            return o;
        }
        assert!(t.elapsed() < Duration::from_secs(5), "the run didn't finish");
        std::thread::sleep(Duration::from_millis(1));
    }
}

fn open() -> Arc<Mutex<bool>> {
    Arc::new(Mutex::new(true))
}

#[test]
fn a_run_keeps_its_result_until_the_schematic_changes() {
    let mut b = divider();
    let op = Analysis::new("OP", AnalysisKind::Op);
    let mut s = Session::default();
    assert!(s.start(b.project(), &b.conn(), &op, Arc::new(Fake { gate: open(), ..Default::default() })));
    assert_eq!(s.running().map(|r| r.0), Some(op.id));
    assert_eq!(wait(&mut s), Outcome::Done);
    assert!(s.running().is_none());
    let r = &s.result(op.id).unwrap().result;
    assert_eq!(r.op_voltage("OUT"), Some(1.0));
    let a = s.attempt().unwrap();
    assert_eq!(a.log, "the log");
    assert!(a.problems.iter().any(|p| p.level == Level::Warning && p.message == "warning: a hint"));
    assert!(a.netlist.contains("R1 vsense_r1_1_in vsense_r1_2_in 10000"), "{}", a.netlist);

    let none = sim::tune::Overrides::default();
    assert!(!s.is_stale(b.project(), &b.conn(), &op, &none, 1));
    // Tuned values make it stale too, until a run with them.
    let r1 = b.project().schematic.components.iter().find(|c| c.refdes == "R1").unwrap().id;
    let tuned = sim::tune::Overrides::new(b.project(), &[(TuneParam::Value { component: r1 }, 2200.0)]);
    assert!(s.is_stale(b.project(), &b.conn(), &op, &tuned, 1));
    let fake = Fake { gate: open(), ..Default::default() };
    let netlists = fake.netlists.clone();
    assert!(s.start_with(b.project(), &b.conn(), &op, &[], &tuned, Arc::new(fake)));
    assert_eq!(wait(&mut s), Outcome::Done);
    assert!(netlists.lock().unwrap()[0].contains("vsense_r1_2_in 2200"), "{:?}", netlists.lock().unwrap());
    assert!(!s.is_stale(b.project(), &b.conn(), &op, &tuned, 1));
    assert!(s.is_stale(b.project(), &b.conn(), &op, &none, 1));
    // A value changes: the result stays, marked stale.
    let r2 = b.project().schematic.components.iter().find(|c| c.refdes == "R2").unwrap().id;
    b.update_component(r2, |c| c.value = "4k7".into());
    assert!(s.is_stale(b.project(), &b.conn(), &op, &tuned, 2));
    assert!(s.result(op.id).is_some());
    // Results of deleted analyses go.
    s.retain(|id| id != op.id);
    assert!(s.result(op.id).is_none());
}

#[test]
fn netlist_errors_keep_ngspice_from_starting() {
    let mut b = Bench::new();
    b.place("Resistor", "10k", &["IN", "OUT"]);
    let op = Analysis::new("OP", AnalysisKind::Op);
    let fake = Fake { gate: open(), ..Default::default() };
    let netlists = fake.netlists.clone();
    let mut s = Session::default();
    assert!(!s.start(b.project(), &b.conn(), &op, Arc::new(fake)));
    assert!(s.running().is_none());
    let a = s.attempt().unwrap();
    assert_eq!(a.outcome, Outcome::NetlistErrors);
    assert!(a.problems.iter().any(|p| p.level == Level::Error));
    assert!(netlists.lock().unwrap().is_empty());
}

#[test]
fn failures_become_problems() {
    let b = divider();
    let op = Analysis::new("OP", AnalysisKind::Op);
    let fake = Fake {
        gate: open(),
        fail: Some("Error: no such model\nError: circuit not parsed".into()),
        ..Default::default()
    };
    let mut s = Session::default();
    assert!(s.start(b.project(), &b.conn(), &op, Arc::new(fake)));
    assert!(matches!(wait(&mut s), Outcome::Failed(_)));
    let a = s.attempt().unwrap();
    let errors: Vec<_> = a.problems.iter().filter(|p| p.level == Level::Error).map(|p| p.message.as_str()).collect();
    assert_eq!(errors, ["Error: no such model", "Error: circuit not parsed"]);
    assert_eq!(a.log, "the log");
    assert!(s.result(op.id).is_none());
}

#[test]
fn stop_and_restart() {
    let b = divider();
    let op = Analysis::new("OP", AnalysisKind::Op);
    let gate = Arc::new(Mutex::new(false));
    let mut s = Session::default();
    assert!(s.start(b.project(), &b.conn(), &op, Arc::new(Fake { gate: gate.clone(), ..Default::default() })));
    std::thread::sleep(Duration::from_millis(5));
    assert!(s.poll().is_none());
    // Stopped, it ends once the backend gives up (with nothing so far: cancelled).
    s.stop();
    assert_eq!(wait(&mut s), Outcome::Cancelled);
    assert!(s.running().is_none());
    assert_eq!(s.attempt().unwrap().outcome, Outcome::Cancelled);
    // Starting again while one runs replaces it.
    assert!(s.start(b.project(), &b.conn(), &op, Arc::new(Fake { gate: gate.clone(), ..Default::default() })));
    assert!(s.start(b.project(), &b.conn(), &op, Arc::new(Fake { gate: gate.clone(), ..Default::default() })));
    *gate.lock().unwrap() = true;
    assert_eq!(wait(&mut s), Outcome::Done);
    assert!(s.result(op.id).is_some());
}

#[test]
fn a_parameter_sweep_runs_a_netlist_per_step() {
    let mut b = divider();
    let r2 = b.project().schematic.components.iter().find(|c| c.refdes == "R2").unwrap().id;
    let mut op = Analysis::new("OP", AnalysisKind::Op);
    let param = TuneParam::Value { component: r2 };
    op.step = Some(Step { param: param.clone(), values: StepValues::List { values: vec![1e3, 2.2e3, 4.7e3] } });
    let fake = Fake { gate: open(), ..Default::default() };
    let netlists = fake.netlists.clone();
    let mut s = Session::default();
    let none = sim::tune::Overrides::default();
    assert!(s.start_with(b.project(), &b.conn(), &op, &[], &none, Arc::new(fake)));
    assert_eq!(wait(&mut s), Outcome::Done);
    let mut ran: Vec<String> = netlists.lock().unwrap().clone();
    ran.sort();
    assert_eq!(ran.len(), 3);
    for v in ["1000", "2200", "4700"] {
        assert!(ran.iter().any(|n| n.contains(&format!("vsense_r2_2_in {v}"))), "{v}: {ran:?}");
    }
    let done = s.result(op.id).unwrap();
    assert_eq!(done.step, Some((param.clone(), vec![1e3, 2.2e3, 4.7e3])));
    // In step order, whichever worker finished first.
    let results = done.results();
    assert_eq!(results.len(), 3);
    for ((v, r), want) in results.iter().zip(["1000", "2200", "4700"]) {
        assert!(v.is_some());
        assert!(r.netlist.text.contains(&format!("vsense_r2_2_in {want}")), "{}", r.netlist.text);
    }
    let labels: Vec<String> = sim::tune::steps(b.project(), done).into_iter().map(|(l, _)| l).collect();
    assert_eq!(labels, ["R2 = 1 kΩ", "R2 = 2.2 kΩ", "R2 = 4.7 kΩ"]);
    // Stale by the netlist without the step: the swept value itself doesn't count.
    assert!(!s.is_stale(b.project(), &b.conn(), &op, &none, 1));
    b.update_component(r2, |c| c.value = "3k3".into());
    assert!(s.is_stale(b.project(), &b.conn(), &op, &none, 2));

    // A failing step fails the sweep, named by its step.
    let fake = Fake { gate: open(), fail: Some("singular matrix".into()), ..Default::default() };
    assert!(s.start_with(b.project(), &b.conn(), &op, &[], &none, Arc::new(fake)));
    assert!(matches!(wait(&mut s), Outcome::Failed(m) if m.starts_with("step ")));
}

#[test]
fn a_sweep_of_a_deleted_part_does_not_start() {
    let b = divider();
    let mut op = Analysis::new("OP", AnalysisKind::Op);
    op.step = Some(Step {
        param: TuneParam::Value { component: Id::new() },
        values: StepValues::List { values: vec![1.0, 2.0] },
    });
    let mut s = Session::default();
    let none = sim::tune::Overrides::default();
    assert!(!s.start_with(
        b.project(),
        &b.conn(),
        &op,
        &[],
        &none,
        Arc::new(Fake { gate: open(), ..Default::default() })
    ));
    let a = s.attempt().unwrap();
    assert_eq!(a.outcome, Outcome::NetlistErrors);
    assert!(a.problems.iter().any(|p| p.message.contains("no longer exists")));
}

/// A backend that can't block (as the browser's): its runs end on their `polls`-th poll,
/// stream a point when started, and stop with it when cancelled.
#[derive(Default)]
struct Polled {
    polls: usize,
    /// Runs going now, and the most at once.
    going: Arc<Mutex<(usize, usize)>>,
    netlists: Arc<Mutex<Vec<String>>>,
    serial: bool,
}

struct PolledRun {
    left: usize,
    cancel: Cancel,
    live: sim::Live,
    going: Arc<Mutex<(usize, usize)>>,
}

fn op_plot(v: f64) -> Plot {
    let out = Vector { name: "v(out)".into(), kind: "voltage".into(), re: vec![v], im: None };
    Plot { name: "Operating Point".into(), complex: false, vectors: vec![out] }
}

impl sim::Pending for PolledRun {
    fn poll(&mut self) -> Option<Result<Output, SimError>> {
        let stopped = self.cancel.is_cancelled();
        if !stopped && self.left > 1 {
            self.left -= 1;
            return None;
        }
        self.going.lock().unwrap().0 -= 1;
        let plot = if stopped { self.live.plot().unwrap() } else { op_plot(1.0) };
        Some(Ok(Output { plots: vec![plot], log: "the log".into(), warnings: Vec::new(), stopped }))
    }
}

impl Backend for Polled {
    fn run(&self, _: &str, _: &Cancel, _: &sim::Live) -> Result<Output, SimError> {
        panic!("a polled backend isn't run on a thread")
    }

    fn start(&self, netlist: &str, cancel: &Cancel, live: &sim::Live) -> Option<Box<dyn sim::Pending + Send>> {
        self.netlists.lock().unwrap().push(netlist.to_string());
        let mut g = self.going.lock().unwrap();
        g.0 += 1;
        g.1 = g.1.max(g.0);
        let mut empty = op_plot(0.0);
        empty.vectors[0].re.clear();
        live.begin(empty);
        live.push(&[(0.5, 0.0)]);
        Some(Box::new(PolledRun {
            left: self.polls,
            cancel: cancel.clone(),
            live: live.clone(),
            going: self.going.clone(),
        }))
    }

    fn parallel(&self) -> bool {
        !self.serial
    }

    fn describe(&self) -> String {
        "polled".into()
    }
}

#[test]
fn backends_that_cannot_block_are_polled() {
    let b = divider();
    let op = Analysis::new("OP", AnalysisKind::Op);
    let mut s = Session::default();
    // Polled at the start already, then by each `poll`.
    assert!(s.start(b.project(), &b.conn(), &op, Arc::new(Polled { polls: 3, ..Default::default() })));
    assert!(s.poll().is_none());
    assert_eq!(s.poll(), Some(Outcome::Done));
    assert_eq!(s.result(op.id).unwrap().result.op_voltage("OUT"), Some(1.0));

    // Stopped, it keeps the points streamed so far.
    assert!(s.start(b.project(), &b.conn(), &op, Arc::new(Polled { polls: 100, ..Default::default() })));
    assert!(s.poll().is_none());
    s.stop();
    assert_eq!(s.poll(), Some(Outcome::Done));
    let done = s.result(op.id).unwrap();
    assert!(done.stopped);
    assert_eq!(done.result.op_voltage("OUT"), Some(0.5));

    // A sweep: every step, no more at once than the backend allows (one when serial).
    let r2 = b.project().schematic.components.iter().find(|c| c.refdes == "R2").unwrap().id;
    let mut sweep = Analysis::new("OP", AnalysisKind::Op);
    let values = vec![1e3, 2.2e3, 4.7e3, 10e3];
    sweep.step = Some(Step { param: TuneParam::Value { component: r2 }, values: StepValues::List { values } });
    for serial in [true, false] {
        let backend = Polled { polls: 2, serial, ..Default::default() };
        let (going, netlists) = (backend.going.clone(), backend.netlists.clone());
        assert!(s.start(b.project(), &b.conn(), &sweep, Arc::new(backend)));
        assert_eq!(wait(&mut s), Outcome::Done);
        assert_eq!(netlists.lock().unwrap().len(), 4);
        assert_eq!(s.result(sweep.id).unwrap().results().len(), 4);
        let most = going.lock().unwrap().1;
        assert!(if serial { most == 1 } else { most > 1 }, "serial {serial}: {most} at once");
    }
}
