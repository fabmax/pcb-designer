//! Simulation through MCP in the headless host: the SPICE netlist, `simulate` (with a
//! fake simulator, and with ngspice when it's installed) and the plot screenshot.

use std::sync::Arc;

use mcp::headless::HeadlessHost;
use mcp::simulation::SimulateRequest;
use mcp::{Call, Reply, Request, handle};
use serde_json::Value;
use sim::{Backend, Cancel, Output, Plot, SimError, Vector};
use view::Document;

fn call(host: &mut HeadlessHost, request: Request) -> Result<Reply, String> {
    let (tx, mut rx) = tokio::sync::oneshot::channel();
    handle(Call { request, reply: tx }, host);
    rx.try_recv().expect("answered")
}

fn ask(host: &mut HeadlessHost, request: Request) -> Result<Value, String> {
    match call(host, request)? {
        Reply::Json(v) => Ok(v),
        Reply::Png(_) => panic!("expected JSON"),
    }
}

fn simulate(
    host: &mut HeadlessHost,
    analysis: &str,
    signals: Option<&[&str]>,
    points: &[f64],
) -> Result<Value, String> {
    let req = SimulateRequest {
        analysis: Some(analysis.into()),
        signals: signals.map(|s| s.iter().map(|s| s.to_string()).collect()),
        points: points.to_vec(),
        ..Default::default()
    };
    ask(host, Request::Simulate(req))
}

fn demo() -> HeadlessHost {
    HeadlessHost::new(Document::new(view::demo::demo_project()))
}

/// A transient run of 2001 points over 2 ms: every node at 5 V plus a 1 kHz sine of
/// 0.5 V.
struct Fake {
    nodes: Vec<String>,
}

impl Backend for Fake {
    fn run(&self, _netlist: &str, _cancel: &Cancel, _live: &sim::Live) -> Result<Output, SimError> {
        let n = 2001;
        let t: Vec<f64> = (0..n).map(|i| i as f64 * 1e-6).collect();
        let v: Vec<f64> = t.iter().map(|t| 5.0 + 0.5 * (2.0 * std::f64::consts::PI * 1e3 * t).sin()).collect();
        let mut vectors = vec![Vector { name: "time".into(), kind: "time".into(), re: t, im: None }];
        for node in &self.nodes {
            vectors.push(Vector { name: format!("v({node})"), kind: "voltage".into(), re: v.clone(), im: None });
        }
        let plot = Plot { name: "Transient Analysis".into(), complex: false, vectors };
        Ok(Output { plots: vec![plot], log: String::new(), warnings: Vec::new(), stopped: false })
    }

    fn describe(&self) -> String {
        "fake".into()
    }
}

#[test]
fn simulate_takes_values_for_one_run() {
    let mut host = demo();
    let nl = ask(&mut host, Request::SpiceNetlist { analysis: None }).unwrap();
    let nodes = nl["nodes"].as_object().unwrap().values().map(|n| n.as_str().unwrap().to_string()).collect();
    host.sim_backend = Some(Arc::new(Fake { nodes }));
    let r1 = |host: &HeadlessHost| {
        host.doc.project().schematic.components.iter().find(|c| c.refdes == "R1").unwrap().value.clone()
    };
    let before = r1(&host);
    let values = vec![("R1".to_string(), "2k2".to_string()), ("V1.ac".to_string(), "0.25".to_string())];
    let req = SimulateRequest { analysis: Some("Transient".into()), values, ..Default::default() };
    ask(&mut host, Request::Simulate(req)).unwrap();
    let netlist = host.sim.attempt().unwrap().netlist.clone();
    assert!(netlist.lines().any(|l| l.starts_with("R1 ") && l.ends_with(" 2200")), "{netlist}");
    assert!(netlist.contains("AC 0.25"), "{netlist}");
    // The schematic keeps its value.
    assert_eq!(r1(&host), before);

    let bad = |key: &str, value: &str| {
        let mut host = demo();
        let values = vec![(key.to_string(), value.to_string())];
        let req = SimulateRequest { analysis: Some("Transient".into()), values, ..Default::default() };
        ask(&mut host, Request::Simulate(req)).unwrap_err()
    };
    assert!(bad("R99", "1k").contains("no component"));
    assert!(bad("R1", "lots").contains("isn't a value"));
    let e = bad("V1.nope", "1");
    assert!(e.contains("V1.ac"), "{e}");
}

#[test]
fn simulate_answers_a_sweep_per_step() {
    let mut host = demo();
    let nl = ask(&mut host, Request::SpiceNetlist { analysis: None }).unwrap();
    let nodes = nl["nodes"].as_object().unwrap().values().map(|n| n.as_str().unwrap().to_string()).collect();
    host.sim_backend = Some(Arc::new(Fake { nodes }));
    let project = host.doc.project();
    let r1 = project.schematic.components.iter().find(|c| c.refdes == "R1").unwrap().id;
    let mut setup = project.schematic.simulation.clone();
    let tran = setup.analyses.iter_mut().find(|a| a.name == "Transient").unwrap();
    tran.step = Some(model::Step {
        param: model::TuneParam::Value { component: r1 },
        values: model::StepValues::List { values: vec![100.0, 1000.0] },
    });
    host.doc.execute(editor_core::Command::SetSimulation { simulation: setup }).unwrap();

    let v = simulate(&mut host, "Transient", Some(&["+5V"]), &[]).unwrap();
    assert_eq!(v["step"]["parameter"], "R1", "{v}");
    let signals = v["signals"].as_array().unwrap();
    let labels: Vec<&str> = signals.iter().map(|s| s["signal"].as_str().unwrap()).collect();
    assert_eq!(labels, ["V(+5V) · R1 = 100 Ω", "V(+5V) · R1 = 1 kΩ"]);
    assert_eq!(signals[1]["step_value"], 1000.0);
    assert!(signals[0]["max"].is_number(), "{v}");
    // The plot draws every step.
    assert!(matches!(
        call(
            &mut host,
            Request::SimulationScreenshot {
                analysis: Some("Transient".into()),
                spectrum: false,
                width: 300,
                height: 200
            }
        ),
        Ok(Reply::Png(_))
    ));
}

#[test]
fn spice_netlist_names_nodes_and_devices() {
    let mut host = demo();
    let v = ask(&mut host, Request::SpiceNetlist { analysis: Some("ac".into()) }).unwrap();
    assert_eq!(v["analysis"], "AC");
    let text = v["netlist"].as_str().unwrap();
    assert!(text.contains(".ac dec 50"), "{text}");
    assert_eq!(v["nodes"]["GND"], "0");
    assert!(v["nodes"]["+5V"].is_string(), "{v}");
    let devices = v["devices"].as_array().unwrap();
    assert!(devices.iter().any(|d| d["refdes"] == "V1" && d["device"] == "V1"), "{v}");
    assert!(devices.iter().any(|d| d["refdes"] == "R1"), "{v}");
    // The default: the first analysis.
    let v = ask(&mut host, Request::SpiceNetlist { analysis: None }).unwrap();
    assert_eq!(v["analysis"], "Operating point");
    let err = ask(&mut host, Request::SpiceNetlist { analysis: Some("nope".into()) }).unwrap_err();
    assert!(err.contains("Transient"), "{err}");
}

#[test]
fn simulate_answers_a_sweep_by_signal() {
    let mut host = demo();
    let nl = ask(&mut host, Request::SpiceNetlist { analysis: None }).unwrap();
    let nodes = nl["nodes"].as_object().unwrap().values().map(|n| n.as_str().unwrap().to_string()).collect();
    host.sim_backend = Some(Arc::new(Fake { nodes }));

    // The analysis' probes by default.
    let v = simulate(&mut host, "Transient", None, &[0.25e-3, 1.0]).unwrap();
    assert_eq!(v["kind"], "Transient");
    assert_eq!(v["x"]["samples"], 2001);
    let signals = v["signals"].as_array().unwrap();
    let labels: Vec<&str> = signals.iter().map(|s| s["signal"].as_str().unwrap()).collect();
    assert_eq!(labels, ["V(+5V)", "V(Net-(D1-2))"]);
    let s = &signals[0];
    assert_eq!(s["unit"], "V");
    assert!((s["max"].as_f64().unwrap() - 5.5).abs() < 1e-3, "{s}");
    assert!((s["min"].as_f64().unwrap() - 4.5).abs() < 1e-3, "{s}");
    assert!((s["average"].as_f64().unwrap() - 5.0).abs() < 1e-3, "{s}");
    assert!((s["end"].as_f64().unwrap() - 5.0).abs() < 1e-3, "{s}");
    assert!((s["frequency"].as_f64().unwrap() - 1e3).abs() < 1.0, "{s}");
    assert!((s["rms"].as_f64().unwrap() - (25.0f64 + 0.125).sqrt()).abs() < 1e-3, "{s}");
    assert!((s["peak_to_peak"].as_f64().unwrap() - 1.0).abs() < 1e-3, "{s}");
    // A pure sine: 2 periods, no distortion.
    let d = &s["distortion"];
    assert!((d["fundamental"].as_f64().unwrap() - 1e3).abs() < 1.0, "{d}");
    assert_eq!(d["periods"], 2);
    assert!((d["level_rms"].as_f64().unwrap() - 0.5 / 2f64.sqrt()).abs() < 1e-3, "{d}");
    assert!(d["thd_percent"].as_f64().unwrap() < 1e-3, "{d}");
    assert_eq!(d["harmonics_dbc"].as_array().unwrap().len(), 9, "{d}");
    let points = s["points"].as_array().unwrap();
    assert!(points.len() <= 200 && points.len() > 50, "{}", points.len());
    // At the sine's peak; past the end, the last value.
    assert!((s["at"][0][1].as_f64().unwrap() - 5.5).abs() < 1e-3, "{s}");
    assert!((s["at"][1][1].as_f64().unwrap() - 5.0).abs() < 1e-3, "{s}");

    // Named signals, in both spellings; a missing current.
    let v = simulate(&mut host, "transient", Some(&["V(+5V)", "GND", "R1.1"]), &[]).unwrap();
    let labels: Vec<&str> = v["signals"].as_array().unwrap().iter().map(|s| s["signal"].as_str().unwrap()).collect();
    assert_eq!(labels, ["V(+5V)", "V(GND)"]);
    assert_eq!(v["missing"][0]["signal"], "I(R1.1)", "the fake doesn't measure it: {v}");

    // The plot of the last run.
    match call(
        &mut host,
        Request::SimulationScreenshot { analysis: Some("Transient".into()), spectrum: false, width: 400, height: 250 },
    ) {
        Ok(Reply::Png(png)) => assert!(png.starts_with(b"\x89PNG")),
        Ok(Reply::Json(v)) => panic!("expected a PNG: {v}"),
        Err(e) => panic!("{e}"),
    }
    let spectrum =
        Request::SimulationScreenshot { analysis: Some("Transient".into()), spectrum: true, width: 400, height: 250 };
    assert!(matches!(call(&mut host, spectrum), Ok(Reply::Png(_))));
    let err = call(
        &mut host,
        Request::SimulationScreenshot { analysis: Some("AC".into()), spectrum: false, width: 400, height: 250 },
    );
    assert!(err.unwrap_err().contains("no results yet"));
    let err = call(
        &mut host,
        Request::SimulationScreenshot { analysis: Some("AC".into()), spectrum: true, width: 400, height: 250 },
    );
    assert!(err.unwrap_err().contains("only transients"));
}

#[test]
fn simulate_refuses_unknown_signals() {
    let mut host = demo();
    host.sim_backend = Some(Arc::new(Fake { nodes: Vec::new() }));
    let err = simulate(&mut host, "Transient", Some(&["NOPE"]), &[]).unwrap_err();
    assert!(err.contains("no net \"NOPE\""), "{err}");
    let err = simulate(&mut host, "Transient", Some(&["R1.9"]), &[]).unwrap_err();
    assert!(err.contains("R1 has no simulated pin \"9\" (pins: 1, 2)"), "{err}");
    let err = simulate(&mut host, "Transient", Some(&["I(X9.1)"]), &[]).unwrap_err();
    assert!(err.contains("REFDES.PIN"), "{err}");
    let err = simulate(&mut host, "Transient", Some(&["P(X9)"]), &[]).unwrap_err();
    assert!(err.contains("no simulated part"), "{err}");
    let err = simulate(&mut host, "Transient", Some(&["V(+5V,NOPE)"]), &[]).unwrap_err();
    assert!(err.contains("no net \"NOPE\""), "{err}");
    let err = simulate(&mut host, "Gone", None, &[]).unwrap_err();
    assert!(err.contains("no analysis"), "{err}");
}

/// Whether ngspice is found (the executable or the library).
fn ngspice() -> bool {
    let engine = sim::Engine::find(&sim::EngineSettings::default());
    let found = engine.process.is_some() || engine.library.is_some();
    if !found {
        eprintln!("skipped: ngspice not found");
    }
    found
}

/// Through the headless host's engine, as the app's: the library when found.
#[test]
fn simulate_with_ngspice() {
    if !ngspice() {
        return;
    }
    let mut host = demo();
    // An operating point: every net, pin current and power.
    let v = simulate(&mut host, "Operating point", None, &[]).unwrap();
    let values = v["values"].as_array().unwrap();
    let value = |label: &str| values.iter().find(|r| r["signal"] == label).map(|r| r["value"].as_f64().unwrap());
    assert!((value("V(+5V)").unwrap() - 5.0).abs() < 1e-6, "{v}");
    let i = value("I(R1.1)").expect("pin currents");
    assert!(i.abs() > 1e-4 && i.abs() < 0.02, "an LED current: {i}");
    assert!(value("P(R1)").unwrap() > 0.0, "{v}");
    // Only some values.
    let v = simulate(&mut host, "Operating point", Some(&["+5V"]), &[]).unwrap();
    assert_eq!(v["values"].as_array().unwrap().len(), 1, "{v}");

    // A current the transient doesn't probe: measured for this run, the probes unchanged.
    let probes = host.doc.project().schematic.simulation.analyses[1].probes.clone();
    let v = simulate(&mut host, "Transient", Some(&["I(R1.1)", "+5V"]), &[1e-3]).unwrap();
    let s = &v["signals"][0];
    assert_eq!(s["signal"], "I(R1.1)", "{v}");
    assert_eq!(s["unit"], "A");
    assert!(s["max"].as_f64().unwrap().abs() > 1e-4, "{s}");
    assert_eq!(host.doc.project().schematic.simulation.analyses[1].probes, probes);
    // The result isn't stale for it.
    let doc = &mut host.doc;
    doc.update_connectivity(false);
    let a = doc.project().schematic.simulation.analyses[1].clone();
    let none = sim::tune::Overrides::default();
    assert!(!host.sim.is_stale(host.doc.project(), host.doc.connectivity(), &a, &none, host.doc.revision));
    // AC: magnitude and phase.
    let v = simulate(&mut host, "AC", None, &[1e3]).unwrap();
    let labels: Vec<&str> = v["signals"].as_array().unwrap().iter().map(|s| s["signal"].as_str().unwrap()).collect();
    assert_eq!(labels, ["V(+5V)", "V(+5V) phase", "V(Net-(D1-2))", "V(Net-(D1-2)) phase"]);
    assert_eq!(v["signals"][0]["unit"], "dB");
    assert!(v["signals"][0]["peak"]["db"].is_number(), "{v}");
    assert!(v["signals"][1]["phase_margin"].is_null() || v["signals"][1]["phase_margin"].is_number());
    // A net to another, and a part's power, in a transient.
    let v = simulate(&mut host, "Transient", Some(&["V(+5V,GND)", "P(R1)"]), &[]).unwrap();
    let labels: Vec<&str> = v["signals"].as_array().unwrap().iter().map(|s| s["signal"].as_str().unwrap()).collect();
    assert_eq!(labels, ["V(+5V,GND)", "P(R1)"], "{v}");
    assert_eq!(v["signals"][1]["unit"], "W");
    assert!(v["signals"][1]["average"].as_f64().unwrap() > 1e-3, "{v}");
}
