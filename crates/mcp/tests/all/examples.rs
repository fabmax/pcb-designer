//! The audio amplifier example: its schematic and board pass the checks, and it
//! simulates (with ngspice, when installed) to the bias point and gain it was designed for.

use mcp::headless::HeadlessHost;
use mcp::simulation::SimulateRequest;
use mcp::{Call, Reply, Request, handle};
use serde_json::Value;
use view::Document;
use view::demo::Example;

fn ask(host: &mut HeadlessHost, request: Request) -> Result<Value, String> {
    let (tx, mut rx) = tokio::sync::oneshot::channel();
    handle(Call { request, reply: tx }, host);
    match rx.try_recv().expect("answered at once")? {
        Reply::Json(v) => Ok(v),
        Reply::Png(_) => panic!("expected JSON"),
    }
}

fn amp() -> HeadlessHost {
    HeadlessHost::new(Document::new(Example::Amp.project()))
}

#[test]
fn examples_are_found_by_name() {
    for e in Example::ALL {
        assert_eq!(Example::parse(e.name()), Some(e));
    }
    assert_eq!(Example::parse("AMP"), Some(Example::Amp));
    assert_eq!(Example::parse("nope"), None);
}

#[test]
fn amp_passes_erc_and_drc() {
    let mut host = amp();
    let n = ask(&mut host, Request::Netlist { include_excluded: false }).unwrap();
    assert_eq!(n["problems"].as_array().unwrap().len(), 0, "{}", n["problems"]);
    let drc = ask(&mut host, Request::Drc { limit: 50, include_excluded: false }).unwrap();
    assert_eq!(drc["violations"].as_array().unwrap().len(), 0, "{drc}");
}

#[test]
fn amp_simulates_with_ngspice() {
    let engine = sim::Engine::find(&sim::EngineSettings::default());
    if engine.library.is_none() {
        sim::skip_without_ngspice("ngspice not found");
        return;
    }
    let mut host = amp();
    let simulate = |host: &mut HeadlessHost, analysis: &str, signals: &[&str], points: &[f64]| {
        let req = SimulateRequest {
            analysis: Some(analysis.into()),
            signals: Some(signals.iter().map(|s| s.to_string()).collect()),
            points: points.to_vec(),
            ..Default::default()
        };
        ask(host, Request::Simulate(req)).unwrap()
    };
    // The output sits near half the supply.
    let v = simulate(&mut host, "Operating point", &["/OUT"], &[]);
    let out = v["values"][0]["value"].as_f64().unwrap();
    assert!((5.5..7.5).contains(&out), "{v}");
    // About 12 dB of gain into the 8 Ω load at 1 kHz.
    let v = simulate(&mut host, "Frequency response", &["SPK"], &[1e3]);
    let db = v["signals"][0]["at"][0][1].as_f64().unwrap_or(f64::NAN);
    assert!((10.0..14.0).contains(&db), "{v}");
    // The 1 kHz sine into the speaker: about 1.5 % distortion, mostly the 2nd harmonic
    // (the output pair's halves differ); the input is clean.
    let v = simulate(&mut host, "Transient", &["SPK", "IN"], &[]);
    let d = &v["signals"][0]["distortion"];
    assert!((d["fundamental"].as_f64().unwrap() - 1e3).abs() < 1.0, "{d}");
    assert!((0.5..3.0).contains(&d["thd_percent"].as_f64().unwrap()), "{d}");
    let h = d["harmonics_dbc"].as_array().unwrap();
    assert!(h[0].as_f64().unwrap() > h[1].as_f64().unwrap(), "{d}");
    let input = &v["signals"][1]["distortion"];
    assert!(input["thd_percent"].as_f64().unwrap() < 0.01, "{input}");
}
