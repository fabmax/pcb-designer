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
    fn run(&self, _netlist: &str, _cancel: &Cancel) -> Result<Output, SimError> {
        let n = 2001;
        let t: Vec<f64> = (0..n).map(|i| i as f64 * 1e-6).collect();
        let v: Vec<f64> = t.iter().map(|t| 5.0 + 0.5 * (2.0 * std::f64::consts::PI * 1e3 * t).sin()).collect();
        let mut vectors = vec![Vector { name: "time".into(), kind: "time".into(), re: t, im: None }];
        for node in &self.nodes {
            vectors.push(Vector { name: format!("v({node})"), kind: "voltage".into(), re: v.clone(), im: None });
        }
        let plot = Plot { name: "Transient Analysis".into(), complex: false, vectors };
        Ok(Output { plots: vec![plot], log: String::new(), warnings: Vec::new() })
    }
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
    match call(&mut host, Request::SimulationScreenshot { analysis: Some("Transient".into()), width: 400, height: 250 })
    {
        Ok(Reply::Png(png)) => assert!(png.starts_with(b"\x89PNG")),
        Ok(Reply::Json(v)) => panic!("expected a PNG: {v}"),
        Err(e) => panic!("{e}"),
    }
    let err = call(&mut host, Request::SimulationScreenshot { analysis: Some("AC".into()), width: 400, height: 250 });
    assert!(err.unwrap_err().contains("no results yet"));
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

fn ngspice() -> Option<sim::SharedBackend> {
    match sim::Ngspice::find(None) {
        Ok(n) => Some(Arc::new(n)),
        Err(e) => {
            eprintln!("skipped: {e}");
            None
        }
    }
}

#[test]
fn simulate_with_ngspice() {
    let Some(backend) = ngspice() else { return };
    let mut host = demo();
    host.sim_backend = Some(backend);
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
    assert!(!host.sim.is_stale(host.doc.project(), host.doc.connectivity(), &a, host.doc.revision));
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
