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
    fn run(&self, netlist: &str, cancel: &Cancel) -> Result<Output, SimError> {
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
        Ok(Output { plots: vec![plot], log: "the log".into(), warnings: vec!["warning: a hint".into()] })
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
    s.stop();
    assert!(s.running().is_none());
    assert_eq!(s.attempt().unwrap().outcome, Outcome::Cancelled);
    // Starting again while one runs replaces it.
    assert!(s.start(b.project(), &b.conn(), &op, Arc::new(Fake { gate: gate.clone(), ..Default::default() })));
    assert!(s.start(b.project(), &b.conn(), &op, Arc::new(Fake { gate: gate.clone(), ..Default::default() })));
    *gate.lock().unwrap() = true;
    assert_eq!(wait(&mut s), Outcome::Done);
    assert!(s.result(op.id).is_some());
}
