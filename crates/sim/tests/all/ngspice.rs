//! Circuits run by ngspice, checked against what they should do. Skipped (with a note)
//! when ngspice isn't installed, so a fresh checkout's tests pass.

use model::*;
use sim::{Cancel, Ngspice, SimError, SimResult};

use crate::bench::Bench;

fn ngspice() -> Option<Ngspice> {
    match Ngspice::find(None) {
        Ok(n) => Some(n),
        Err(e) => {
            eprintln!("skipped: {e}");
            None
        }
    }
}

fn run(b: &Bench, analysis: &Analysis) -> Option<Result<SimResult, SimError>> {
    let ng = ngspice()?;
    Some(sim::simulate(b.project(), &b.conn(), analysis, &ng, &Cancel::default()))
}

fn ok(b: &Bench, analysis: &Analysis) -> Option<SimResult> {
    run(b, analysis).map(|r| r.unwrap_or_else(|e| panic!("{e}: {e:?}")))
}

fn near(a: f64, b: f64, tol: f64) -> bool {
    (a - b).abs() <= tol
}

fn op(b: &mut Bench) -> Analysis {
    b.analysis(AnalysisKind::Op)
}

#[test]
fn rc_lowpass_ac_and_transient() {
    let mut b = Bench::new();
    let step = Waveform::Pulse { low: 0.0, high: 1.0, delay: 0.0, rise: 1e-9, fall: 1e-9, width: 1.0, period: 0.0 };
    b.source("VSource", step, 1.0, &["IN", "GND"]);
    b.place("Resistor", "10k", &["IN", "OUT"]);
    b.place("Capacitor", "100n", &["OUT", "GND"]);
    let fc = 1.0 / (2.0 * std::f64::consts::PI * 10e3 * 100e-9);

    let ac = b.analysis(AnalysisKind::Ac { start: 1.0, stop: 1e5, points_per_decade: 100 });
    let Some(r) = ok(&b, &ac) else { return };
    assert!(r.plot.complex);
    let f = r.scale().unwrap();
    assert_eq!(f.name, "frequency");
    let out = r.voltage("OUT").unwrap();
    let db = sim::Vector { re: out.db(), ..out.clone().into_owned() };
    let at_fc = db.at(f, fc).unwrap();
    assert!(near(at_fc, -3.01, 0.05), "{at_fc} dB at {fc} Hz");
    let phase = sim::Vector { re: out.phase(), ..out.into_owned() };
    assert!(near(phase.at(f, fc).unwrap(), -45.0, 0.5));

    let tran = b.analysis(AnalysisKind::Tran { stop: 5e-3, step: Some(1e-6), start: 0.0, max_step: Some(1e-5) });
    let r = ok(&b, &tran).unwrap();
    let t = r.scale().unwrap();
    assert_eq!(t.name, "time");
    let out = r.voltage("OUT").unwrap();
    // One time constant (1 ms): 63 %.
    let v = out.at(t, 1e-3).unwrap();
    assert!(near(v, 1.0 - (-1.0f64).exp(), 0.01), "{v}");
    // Ground reads as zeros.
    assert!(r.voltage("GND").unwrap().re.iter().all(|&v| v == 0.0));
}

#[test]
fn diode_forward_drop_and_current_probe() {
    let mut b = Bench::new();
    b.dc(5.0, "VCC", "GND");
    let (_, r1) = b.place("Resistor", "1k", &["VCC", "A"]);
    b.place("Diode", "", &["A", "GND"]);
    let mut a = op(&mut b);
    a.probes.push(Probe::Current { symbol: r1, pin: b.pin(r1, "1") });
    let Some(r) = ok(&b, &a) else { return };
    let vd = r.op_voltage("A").unwrap();
    assert!((0.6..0.8).contains(&vd), "{vd}");
    let i = r.current(r1, b.pin(r1, "1")).unwrap().re[0];
    assert!(near(i, (5.0 - vd) / 1e3, 1e-6), "{i}");
    assert!(r.scale().is_none());
}

#[test]
fn op_amp_non_inverting_gain() {
    for part in ["OpAmp", "LM358"] {
        let mut b = Bench::new();
        b.dc(12.0, "VP", "GND");
        b.dc(-12.0, "VN", "GND");
        b.dc(0.1, "IN", "GND");
        // +IN, −IN, OUT (V+ and V− on the OpAmp's one unit, on the LM358's unit C).
        let nets = ["IN", "FB", "OUT", "VP", "VN"];
        let (u1, _) = b.place(part, "", if part == "LM358" { &nets[..3] } else { &nets });
        if part == "LM358" {
            b.place_unit(u1, 2, &["VP", "VN"]);
        }
        b.place("Resistor", "100k", &["OUT", "FB"]);
        b.place("Resistor", "10k", &["FB", "GND"]);
        let a = op(&mut b);
        let Some(r) = ok(&b, &a) else { return };
        let out = r.op_voltage("OUT").unwrap();
        assert!(near(out, 1.1, 0.005), "{part}: {out}");
    }
}

#[test]
fn lm358_follower_from_a_single_supply() {
    let mut b = Bench::new();
    b.dc(5.0, "VP", "GND");
    b.dc(2.0, "IN", "GND");
    let (u1, _) = b.place("LM358", "", &["IN", "OUT", "OUT"]);
    b.place_unit(u1, 2, &["VP", "GND"]);
    b.place("Resistor", "10k", &["OUT", "GND"]);
    let a = op(&mut b);
    let Some(r) = ok(&b, &a) else { return };
    assert!(near(r.op_voltage("OUT").unwrap(), 2.0, 0.01));
    let tran = b.analysis(AnalysisKind::Tran { stop: 1e-4, step: None, start: 0.0, max_step: None });
    assert!(ok(&b, &tran).is_some());
}

#[test]
fn regulators() {
    for (part, vout) in [("LM7805", 5.0), ("AMS1117-3.3", 3.3), ("AP2112K-3.3", 3.3)] {
        let mut b = Bench::new();
        // Label by pin name: the symbols list their pins in different orders.
        let p = b.project().library.parts.iter().find(|p| p.name == part).unwrap();
        let s = b.project().library.symbols.get(p.units[0].symbol).unwrap();
        let nets: Vec<&str> = s
            .pins
            .iter()
            .map(|p| match p.name.as_str() {
                "IN" | "VI" | "VIN" | "EN" => "VIN",
                "OUT" | "VO" | "VOUT" => "OUT",
                "GND" => "GND",
                _ => "",
            })
            .collect();
        b.dc(12.0, "VIN", "GND");
        b.place(part, "", &nets);
        b.place("Resistor", "100", &["OUT", "GND"]);
        let a = op(&mut b);
        let Some(r) = ok(&b, &a) else { return };
        let out = r.op_voltage("OUT").unwrap();
        assert!(near(out, vout, 0.01), "{part}: {out}");
    }
}

#[test]
fn transistor_switches_and_logic() {
    let mut b = Bench::new();
    b.dc(5.0, "VCC", "GND");
    // NMOS low-side switch, gate high: the drain is pulled low.
    b.place("Resistor", "1k", &["VCC", "D"]);
    b.place("NMOS", "", &["VCC", "D", "GND"]);
    // NPN with base current: saturated.
    b.place("Resistor", "10k", &["VCC", "B"]);
    b.place("Resistor", "1k", &["VCC", "C"]);
    b.place("NPN", "", &["B", "C", "GND"]);
    // A NAND with one input low: high.
    let (u1, _) = b.place("74HC00", "", &["VCC", "GND", "Y"]);
    b.place_unit(u1, 4, &["VCC", "GND"]);
    // Zener at 5.1 V from 12 V.
    b.dc(12.0, "V12", "GND");
    b.place("Resistor", "1k", &["V12", "Z"]);
    b.place("Diode_Zener", "5V1", &["GND", "Z"]);
    let a = op(&mut b);
    let Some(r) = ok(&b, &a) else { return };
    let v = |n: &str| r.op_voltage(n).unwrap();
    assert!(v("D") < 0.1, "NMOS drain {}", v("D"));
    assert!(v("C") < 0.3, "NPN collector {}", v("C"));
    assert!((0.6..0.85).contains(&v("B")), "NPN base {}", v("B"));
    assert!(v("Y") > 4.9, "NAND output {}", v("Y"));
    assert!(near(v("Z"), 5.1, 0.15), "zener {}", v("Z"));
}

#[test]
fn dc_sweep_of_a_divider() {
    let mut b = Bench::new();
    let v1 = b.dc(0.0, "IN", "GND");
    b.place("Resistor", "1k", &["IN", "OUT"]);
    b.place("Resistor", "3k", &["OUT", "GND"]);
    let a = b.analysis(AnalysisKind::Dc { source: v1, start: 0.0, stop: 4.0, step: 0.5 });
    let Some(r) = ok(&b, &a) else { return };
    let s = r.scale().unwrap();
    assert_eq!(s.re.len(), 9);
    let out = r.voltage("OUT").unwrap();
    for (x, y) in s.re.iter().zip(&out.re) {
        assert!(near(*y, x * 0.75, 1e-6), "{x} → {y}");
    }
}

#[test]
fn crystal_and_sources_run() {
    let mut b = Bench::new();
    let sine = Waveform::Sine { offset: 0.0, amplitude: 1.0, frequency: 16e6, delay: 0.0, damping: 0.0, phase: 0.0 };
    b.source("VSource", sine, 1.0, &["IN", "GND"]);
    b.place("Crystal", "16MHz", &["IN", "X"]);
    b.place("Resistor", "50", &["X", "GND"]);
    let pwl = Waveform::Pwl { points: vec![(0.0, 0.0), (1e-6, 1e-3), (2e-6, 0.0)], repeat: true };
    b.source("ISource", pwl, 0.0, &["GND", "P"]);
    b.place("Resistor", "1k", &["P", "GND"]);
    let exp = Waveform::Exp { v1: 0.0, v2: 1.0, delay1: 0.0, tau1: 1e-7, delay2: 1e-6, tau2: 1e-7 };
    b.source("VSource", exp, 0.0, &["E", "GND"]);
    b.place("Resistor", "1k", &["E", "GND"]);
    // Q ≈ 20 000: a bandwidth of ~800 Hz, so a fine sweep around 16 MHz.
    let ac = b.analysis(AnalysisKind::Ac { start: 15.99e6, stop: 16.01e6, points_per_decade: 1_000_000 });
    let Some(r) = ok(&b, &ac) else { return };
    // Series resonance: the crystal is nearly a short, the 50 Ω gets almost all.
    let x = r.voltage("X").unwrap().magnitude();
    let peak = x.iter().cloned().fold(0.0, f64::max);
    assert!(peak > 0.5, "{peak}");
    let tran = b.analysis(AnalysisKind::Tran { stop: 3e-6, step: Some(1e-9), start: 0.0, max_step: None });
    let r = ok(&b, &tran).unwrap();
    let t = r.scale().unwrap();
    // 1 mA into 1 kΩ at the PWL's peak (current flows from + through the source to −).
    assert!(near(r.voltage("P").unwrap().at(t, 1e-6).unwrap(), 1.0, 0.01));
    assert!(near(r.voltage("E").unwrap().at(t, 0.9e-6).unwrap(), 1.0, 0.01));
}

#[test]
fn ngspice_errors_are_reported() {
    let mut b = Bench::new();
    b.dc(1.0, "A", "GND");
    let (r1, _) = b.place("Resistor", "1k", &["A", "GND"]);
    let part = b.project().library.parts.get(b.component(r1).part).unwrap().clone();
    let mut broken = part.clone();
    broken.sim = Some(SimModel::new(SimDevice::Spice { template: "R${REF} ${N1} ${N2} nomodel_here".into() }));
    b.ed.execute(editor_core::Command::Update { objects: vec![Object::LibPart(broken)] }).unwrap();
    let a = op(&mut b);
    let Some(r) = run(&b, &a) else { return };
    match r {
        Err(SimError::Failed { message, log }) => {
            assert!(!message.is_empty());
            assert!(log.contains("nomodel_here") || message.contains("nomodel_here"), "{message}\n{log}");
        }
        other => panic!("expected a failure, got {other:?}"),
    }
}

#[test]
fn netlist_errors_stop_before_ngspice() {
    let mut b = Bench::new();
    b.place("Resistor", "1k", &["A", "B"]);
    let a = op(&mut b);
    // No backend needed: the netlist has no ground.
    struct Never;
    impl sim::Backend for Never {
        fn run(&self, _: &str, _: &Cancel) -> Result<sim::Output, SimError> {
            panic!("must not run")
        }
    }
    let r = sim::simulate(b.project(), &b.conn(), &a, &Never, &Cancel::default());
    assert!(matches!(r, Err(SimError::Netlist(_))));
    assert!(r.unwrap_err().to_string().starts_with("No ground"));
}

#[test]
fn cancel_stops_a_run() {
    let mut b = Bench::new();
    let sine = Waveform::Sine { offset: 0.0, amplitude: 1.0, frequency: 1e6, delay: 0.0, damping: 0.0, phase: 0.0 };
    b.source("VSource", sine, 0.0, &["A", "GND"]);
    b.place("Resistor", "1k", &["A", "GND"]);
    // Long enough to be still running when cancelled.
    let a = b.analysis(AnalysisKind::Tran { stop: 10.0, step: Some(1e-9), start: 0.0, max_step: Some(1e-9) });
    let Some(ng) = ngspice() else { return };
    let cancel = Cancel::default();
    let c = cancel.clone();
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(300));
        c.cancel();
    });
    let started = std::time::Instant::now();
    let r = sim::simulate(b.project(), &b.conn(), &a, &ng, &cancel);
    assert!(matches!(r, Err(SimError::Cancelled)), "{r:?}");
    assert!(started.elapsed().as_secs() < 5);
}

#[test]
fn reports() {
    let mut b = Bench::new();
    b.dc(5.0, "IN", "GND");
    let (_, r1) = b.place("Resistor", "1k", &["IN", "OUT"]);
    b.place("Resistor", "4k", &["OUT", "GND"]);
    let mut a = op(&mut b);
    a.probes.push(Probe::Current { symbol: r1, pin: b.pin(r1, "1") });
    let Some(r) = ok(&b, &a) else { return };
    let text = sim::report::text(&r);
    assert!(text.starts_with("Operating Point\n"), "{text}");
    assert!(text.contains("V(IN)    5 V\n"), "{text}");
    assert!(text.contains("V(OUT)   4 V\n"), "{text}");
    assert!(text.contains("I(R1.1)  1 mA\n"), "{text}");
    assert!(text.contains("I(R1.2)  -1 mA\n"), "{text}");
    assert!(text.contains("P(R1)    1 mW\n"), "{text}");
    assert!(text.contains("P(R2)    4 mW\n"), "{text}");
    assert!(text.contains("P(V1)    -5 mW\n"), "{text}");
    let devices = sim::report::devices(&r);
    let r1 = devices.iter().find(|d| d.refdes == "R1").unwrap();
    assert_eq!(
        r1.pins.iter().map(|p| p.voltage.map(|v| (v * 1e3).round())).collect::<Vec<_>>(),
        [Some(5000.0), Some(4000.0)]
    );

    let tran = b.analysis(AnalysisKind::Tran { stop: 1e-3, step: Some(1e-4), start: 0.0, max_step: None });
    let r = ok(&b, &tran).unwrap();
    let csv = sim::report::csv(&r);
    let mut lines = csv.lines();
    assert_eq!(lines.next(), Some("time (s),V(IN) (V),V(OUT) (V)"));
    let first: Vec<f64> = lines.next().unwrap().split(',').map(|v| v.parse().unwrap()).collect();
    // rshunt (1 TΩ on every node) shows in the ninth digit.
    assert!(first.len() == 3 && first[0] == 0.0 && near(first[1], 5.0, 1e-6) && near(first[2], 4.0, 1e-6), "{first:?}");
    assert!(sim::report::text(&r).contains("V(OUT)         4 V         4 V         4 V"), "{}", sim::report::text(&r));
}

/// The demo's LED: (5 V − ~1.8 V) / 330 Ω.
#[test]
fn demo_project_simulates() {
    let project = view::demo::demo_project();
    let analysis = project.schematic.simulation.analyses[0].clone();
    let conn = editor_core::connectivity::Connectivity::build(&project);
    let n = sim::netlist::build(&project, &conn, &analysis);
    assert!(!n.has_errors(), "{:?}", n.problems);
    let Some(ng) = ngspice() else { return };
    let r = sim::simulate(&project, &conn, &analysis, &ng, &Cancel::default()).unwrap();
    let led = r.op_voltage("Net-(D1-2)").unwrap();
    assert!((1.6..2.0).contains(&led), "LED at {led} V");
    assert_eq!(r.op_voltage("+5V"), Some(5.0));
}

#[test]
fn version() {
    let Some(ng) = ngspice() else { return };
    let v = ng.version().unwrap();
    assert!(v.starts_with("ngspice-"), "{v}");
}

#[test]
fn differential_and_power_signals_in_a_sweep() {
    use sim::plot::{self, Signal};
    let mut b = Bench::new();
    let v1 = b.dc(4.0, "IN", "GND");
    let (r1, _) = b.place("Resistor", "1k", &["IN", "MID"]);
    b.place("Resistor", "3k", &["MID", "GND"]);
    let mut a = b.analysis(AnalysisKind::Dc { source: v1, start: 0.0, stop: 4.0, step: 1.0 });
    a.probes =
        vec![Signal::Diff("IN".into(), "MID".into()).probe(), Signal::Power(r1).probe(), Signal::Power(v1).probe()];
    let Some(r) = ok(&b, &a) else { return };
    let conn = b.conn();
    let signals: Vec<Signal> = a.probes.iter().map(|p| Signal::of_probe(p, &conn)).collect();
    let d = plot::data(&r, &signals, false, &conn).unwrap();
    let labels: Vec<&str> = d.traces.iter().map(|t| t.label.as_str()).collect();
    assert_eq!(labels, ["V(IN,MID)", "P(R1)", "P(V1)"], "{:?}", d.missing);
    // At 4 V: 1 mA, 1 V across R1, 1 mW in R1, the source delivers 4 mW.
    let end = |k: usize| *d.traces[k].y.last().unwrap();
    assert!(near(end(0), 1.0, 1e-6), "{}", end(0));
    assert!(near(end(1), 1e-3, 1e-8), "{}", end(1));
    assert!(near(end(2), -4e-3, 1e-8), "{}", end(2));
    // Volts left, watts right.
    assert_eq!((d.traces[0].axis, d.traces[1].axis), (plot::Axis::Left, plot::Axis::Right));
}

#[test]
fn a_parameter_sweep_in_parallel_processes() {
    let Some(ng) = ngspice() else { return };
    let mut b = Bench::new();
    b.dc(10.0, "IN", "GND");
    b.place("Resistor", "10k", &["IN", "OUT"]);
    let (r2, _) = b.place("Resistor", "10k", &["OUT", "GND"]);
    let mut a = op(&mut b);
    let values = vec![10e3, 30e3, 90e3];
    a.step =
        Some(Step { param: TuneParam::Value { component: r2 }, values: StepValues::List { values: values.clone() } });
    let mut s = sim::Session::default();
    let none = sim::tune::Overrides::default();
    assert!(s.start_with(b.project(), &b.conn(), &a, &[], &none, std::sync::Arc::new(ng)));
    let t = std::time::Instant::now();
    let outcome = loop {
        if let Some(o) = s.poll() {
            break o;
        }
        assert!(t.elapsed().as_secs() < 30, "the sweep didn't finish");
        std::thread::sleep(std::time::Duration::from_millis(5));
    };
    assert_eq!(outcome, sim::Outcome::Done);
    let done = s.result(a.id).unwrap();
    let out: Vec<f64> = done.results().iter().map(|(_, r)| r.op_voltage("OUT").unwrap()).collect();
    for (v, r) in out.iter().zip(&values) {
        assert!(near(*v, 10.0 * r / (10e3 + r), 1e-3), "{out:?}");
    }
}
