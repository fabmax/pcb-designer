//! Circuits run by ngspice, checked against what they should do: by every backend found
//! (the executable and the shared library), which must agree. Skipped (with a note)
//! when ngspice isn't installed, so a fresh checkout's tests pass.

use model::*;
use sim::{Backend, Cancel, Libngspice, Ngspice, SimError, SimResult};

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

fn library() -> Option<Libngspice> {
    match Libngspice::find(None) {
        Ok(n) => Some(n),
        Err(e) => {
            eprintln!("skipped: {e}");
            None
        }
    }
}

/// Every backend found.
fn backends() -> Vec<Box<dyn Backend>> {
    let mut out: Vec<Box<dyn Backend>> = Vec::new();
    if let Some(n) = ngspice() {
        out.push(Box::new(n));
    }
    if let Some(l) = library() {
        out.push(Box::new(l));
    }
    out
}

/// The run by every backend (`None` without any); they must agree: the same vectors,
/// points and values, or both fail alike.
fn run(b: &Bench, analysis: &Analysis) -> Option<Result<SimResult, SimError>> {
    let mut results = backends()
        .into_iter()
        .map(|ng| (ng.describe(), sim::simulate(b.project(), &b.conn(), analysis, &*ng, &Cancel::default())));
    let (first_name, first) = results.next()?;
    for (name, other) in results {
        match (&first, &other) {
            (Ok(a), Ok(b)) => {
                // The same vectors (in any order: `write` sorts them), the scale first.
                let names = |r: &SimResult| {
                    let mut n: Vec<String> = r.plot.vectors.iter().map(|v| v.name.clone()).collect();
                    n.sort();
                    n
                };
                assert_eq!(names(a), names(b), "{first_name} vs. {name}");
                assert_eq!(a.scale().map(|s| &s.name), b.scale().map(|s| &s.name));
                assert_eq!(a.plot.len(), b.plot.len(), "{first_name} vs. {name}");
                for x in &a.plot.vectors {
                    let y = b.plot.vector(&x.name).unwrap();
                    for (p, q) in x.re.iter().zip(&y.re) {
                        assert!(
                            near(*p, *q, 1e-9 * p.abs().max(1e-3)),
                            "{}: {p} vs. {q} ({first_name} vs. {name})",
                            x.name
                        );
                    }
                }
            }
            (Err(a), Err(b)) => assert_eq!(
                std::mem::discriminant(a),
                std::mem::discriminant(b),
                "{a} vs. {b} ({first_name} vs. {name})"
            ),
            (a, b) => panic!("{first_name}: {a:?}\n{name}: {b:?}"),
        }
    }
    Some(first)
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
    // Repeating, from 0.2 mA (a current source's repeat is split for ngspice < 46).
    let pwl = Waveform::Pwl { points: vec![(0.0, 0.2e-3), (1e-6, 1e-3), (2e-6, 0.2e-3)], repeat: true };
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
    let p = r.voltage("P").unwrap();
    assert!(near(p.at(t, 0.0).unwrap(), 0.2, 0.01));
    assert!(near(p.at(t, 1e-6).unwrap(), 1.0, 0.01));
    // Repeated: 2.5 µs is 0.5 µs into the second period.
    assert!(near(p.at(t, 2.5e-6).unwrap(), 0.6, 0.01), "{}", p.at(t, 2.5e-6).unwrap());
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
        fn run(&self, _: &str, _: &Cancel, _: &sim::Live) -> Result<sim::Output, SimError> {
            panic!("must not run")
        }

        fn describe(&self) -> String {
            "never".into()
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
    let a = b.analysis(AnalysisKind::Tran { stop: 0.1, step: Some(1e-7), start: 0.0, max_step: Some(1e-7) });
    for ng in backends() {
        let cancel = Cancel::default();
        let c = cancel.clone();
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(300));
            c.cancel();
        });
        let started = std::time::Instant::now();
        let r = sim::simulate(b.project(), &b.conn(), &a, &*ng, &cancel);
        // What it had so far (the library always, a process as far as its raw file got).
        match r {
            Err(SimError::Cancelled) => {}
            Ok(r) => assert!(r.plot.len() > 1, "{}", ng.describe()),
            Err(e) => panic!("{}: {e}", ng.describe()),
        }
        assert!(started.elapsed().as_secs() < 5);
    }
}

/// The library streams the points while it runs, its progress too, and Stop keeps
/// what it has.
#[test]
fn the_library_streams_and_stops() {
    let Some(lib) = library() else { return };
    let mut b = Bench::new();
    let sine = Waveform::Sine { offset: 0.0, amplitude: 1.0, frequency: 1e3, delay: 0.0, damping: 0.0, phase: 0.0 };
    b.source("VSource", sine, 0.0, &["A", "GND"]);
    b.place("Resistor", "1k", &["A", "OUT"]);
    b.place("Capacitor", "100n", &["OUT", "GND"]);
    let a = b.analysis(AnalysisKind::Tran { stop: 100.0, step: Some(1e-6), start: 0.0, max_step: Some(1e-6) });
    let mut s = sim::Session::default();
    let none = sim::tune::Overrides::default();
    assert!(s.start_with(b.project(), &b.conn(), &a, &[], &none, std::sync::Arc::new(lib)));
    let t = std::time::Instant::now();
    // Points come while it runs.
    let live = loop {
        assert!(s.poll().is_none(), "{:?}", s.attempt());
        if let Some(f) = s.live(a.id)
            && f.result.plot.len() > 10
        {
            break f.clone();
        }
        // Other tests' library runs go first (one at a time per process).
        assert!(t.elapsed().as_secs() < 60, "no live points");
        std::thread::sleep(std::time::Duration::from_millis(20));
    };
    assert_eq!(live.range, Some((0.0, 100.0)));
    assert!(live.result.voltage("OUT").is_some());
    assert_eq!(live.result.scale().unwrap().name, "time");
    // ngspice reports progress in 0.1% steps, at most every 100 ms: the first can come
    // a while after the first points (about 250 ms here, longer on a slow CI runner).
    let t = std::time::Instant::now();
    let p = loop {
        if let Some(p) = s.progress() {
            break p;
        }
        assert!(s.poll().is_none(), "{:?}", s.attempt());
        assert!(t.elapsed().as_secs() < 30, "no progress");
        std::thread::sleep(std::time::Duration::from_millis(20));
    };
    assert!((0.0..0.5).contains(&p), "{p}");
    s.stop();
    let t = std::time::Instant::now();
    let outcome = loop {
        if let Some(o) = s.poll() {
            break o;
        }
        assert!(t.elapsed().as_secs() < 10, "didn't stop");
        std::thread::sleep(std::time::Duration::from_millis(5));
    };
    assert_eq!(outcome, sim::Outcome::Done);
    let done = s.result(a.id).unwrap();
    assert!(done.stopped && done.range == Some((0.0, 100.0)));
    let end = *done.result.scale().unwrap().re.last().unwrap();
    assert!(end > 0.0 && end < 100.0, "{end}");
    assert!(done.result.plot.len() >= live.result.plot.len());
    assert!(s.live(a.id).is_none());
}

/// ngspice asking to exit (a `quit`: what it does when it gives up, e.g. when it can't
/// allocate a run's vectors) doesn't take the app down. The instance is spent: later
/// library runs fail, and the engine picks processes. It runs in a child process of
/// this test binary, so the other tests' library stays usable.
#[test]
fn an_exited_library_runs_no_more() {
    const CHILD: &str = "PCB_DESIGNER_EXIT_CHILD";
    if std::env::var_os(CHILD).is_none() {
        if library().is_none() {
            return;
        }
        let out = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "ngspice::an_exited_library_runs_no_more", "--nocapture", "--test-threads=1"])
            .env(CHILD, "1")
            .output()
            .unwrap();
        let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
        assert!(out.status.success() && text.contains("1 passed"), "{text}");
        return;
    }
    let lib = library().unwrap();
    assert!(!Libngspice::exited());
    let quit = "* quit\nV1 a 0 1\nR1 a 0 1k\n.op\n.control\nquit\n.endc\n.end\n";
    let r = lib.run(quit, &Cancel::default(), &sim::Live::default());
    assert!(matches!(&r, Err(SimError::Failed { message, .. }) if message.contains("exited")), "{r:?}");
    assert!(Libngspice::exited());

    let mut b = Bench::new();
    b.dc(5.0, "IN", "GND");
    b.place("Resistor", "1k", &["IN", "OUT"]);
    b.place("Resistor", "4k", &["OUT", "GND"]);
    let a = op(&mut b);
    let r = sim::simulate(b.project(), &b.conn(), &a, &lib, &Cancel::default());
    assert!(matches!(&r, Err(SimError::Failed { message, .. }) if message.contains("restart")), "{r:?}");
    // The engine runs it in a process instead (when the executable is there).
    let engine = sim::Engine { process: ngspice(), library: Some(lib), error: None };
    if engine.process.is_some() {
        let backend = engine.backend(&a).unwrap();
        let r = sim::simulate(b.project(), &b.conn(), &a, backend.as_ref(), &Cancel::default()).unwrap();
        assert!(near(r.op_voltage("OUT").unwrap(), 4.0, 1e-6));
    }
}

/// Runs one after the other through the one instance, from several threads.
#[test]
fn library_runs_are_serialised() {
    let Some(lib) = library() else { return };
    let mut b = Bench::new();
    b.dc(5.0, "IN", "GND");
    b.place("Resistor", "1k", &["IN", "OUT"]);
    b.place("Resistor", "4k", &["OUT", "GND"]);
    let a = op(&mut b);
    let text = sim::netlist::build(b.project(), &b.conn(), &a).text;
    let threads: Vec<_> = (0..4)
        .map(|_| {
            let (lib, text) = (lib.clone(), text.clone());
            std::thread::spawn(move || lib.run(&text, &Cancel::default(), &sim::Live::default()).unwrap())
        })
        .collect();
    for t in threads {
        let out = t.join().unwrap();
        let v = out.plots[0].vector("v(out)").unwrap().re[0];
        assert!(near(v, 4.0, 1e-6), "{v}");
    }
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
    let Some(r) = run_project(&project, &conn, &analysis) else { return };
    let led = r.op_voltage("Net-(D1-2)").unwrap();
    assert!((1.6..2.0).contains(&led), "LED at {led} V");
    assert_eq!(r.op_voltage("+5V"), Some(5.0));
}

fn run_project(project: &Project, conn: &editor_core::connectivity::Connectivity, a: &Analysis) -> Option<SimResult> {
    let mut out = None;
    for ng in backends() {
        let r = sim::simulate(project, conn, a, &*ng, &Cancel::default())
            .unwrap_or_else(|e| panic!("{}: {e}", ng.describe()));
        out.get_or_insert(r);
    }
    out
}

#[test]
fn version() {
    if let Some(ng) = ngspice() {
        let v = ng.version().unwrap();
        assert!(v.starts_with("ngspice-"), "{v}");
        assert_eq!(ng.describe(), format!("{v}, process"));
    }
    if let Some(lib) = library() {
        let v = lib.version().unwrap();
        assert!(v.starts_with("ngspice-"), "{v}");
        assert!(lib.describe().starts_with(&format!("lib{v}, shared library")), "{}", lib.describe());
    }
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

#[test]
fn the_bundled_library_comes_first() {
    // scripts/build-ngspice.sh's build counts as bundled in development.
    let Some(bundled) = sim::bundled_library() else {
        eprintln!("skipped: no bundled libngspice (scripts/build-ngspice.sh)");
        return;
    };
    if std::env::var_os("NGSPICE_LIBRARY").is_some() {
        eprintln!("skipped: NGSPICE_LIBRARY overrides it");
        return;
    }
    let lib = Libngspice::find(None).unwrap();
    assert_eq!(lib.path, bundled);
    assert!(lib.describe().ends_with("shared library (bundled)"), "{}", lib.describe());
}
