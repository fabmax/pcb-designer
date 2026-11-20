//! The plot's logic: scales, ticks, decimation, traces.

use std::collections::HashSet;

use model::*;
use sim::plot::{self, Axis, Scale, Signal};

#[test]
fn linear_ticks_are_round() {
    assert_eq!(plot::linear_ticks(0.0, 1.0, 5), vec![0.0, 0.2, 0.4, 0.6, 0.8, 1.0]);
    assert_eq!(plot::linear_ticks(-0.013, 0.031, 5), vec![-0.01, 0.0, 0.01, 0.02, 0.03]);
    let t = plot::linear_ticks(0.1, 0.7, 6);
    assert_eq!(t, vec![0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7]);
    assert!(plot::linear_ticks(1.0, 1.0, 5).is_empty());
}

#[test]
fn log_ticks_are_decades() {
    let s = Scale::new(1.0, 1e7, true);
    assert_eq!(s.ticks(10), vec![1.0, 10.0, 100.0, 1e3, 1e4, 1e5, 1e6, 1e7]);
    let s = Scale::new(10.0, 1000.0, true);
    assert_eq!(s.ticks(8), vec![10.0, 20.0, 50.0, 100.0, 200.0, 500.0, 1000.0]);
    let s = Scale::new(1e-3, 1e9, true);
    assert!(s.ticks(5).len() <= 7, "{:?}", s.ticks(5));
}

#[test]
fn scales_zoom_around_a_point_and_pan() {
    let mut s = Scale::new(0.0, 10.0, false);
    s.zoom(0.5, 2.0);
    assert_eq!((s.lo, s.hi), (2.5, 7.5));
    s.pan(0.2);
    assert!((s.lo - 3.5).abs() < 1e-12 && (s.hi - 8.5).abs() < 1e-12);
    let mut l = Scale::new(1.0, 1e6, true);
    assert!((l.frac(1e3) - 0.5).abs() < 1e-12);
    l.zoom(0.5, 2.0);
    assert!((l.lo - 10f64.powf(1.5)).abs() < 1e-9 && (l.hi - 10f64.powf(4.5)).abs() < 1e-6);
    // A flat range still gets a width; log ranges stay positive.
    let f = Scale::new(5.0, 5.0, false);
    assert!(f.hi > f.lo);
    let z = Scale::new(0.0, 100.0, true);
    assert!(z.lo > 0.0);
}

#[test]
fn labels() {
    assert_eq!(plot::label(0.001, "s"), "1 ms");
    assert_eq!(plot::label(-20.0, "dB"), "-20 dB");
    assert_eq!(plot::label(-90.0, "°"), "-90°");
    assert_eq!(plot::label(2.5, "dB"), "2.5 dB");
    assert_eq!(plot::label(-0.001, "°"), "0°");
    assert_eq!(plot::label(-6000.0, "dB"), "−∞ dB");
}

#[test]
fn phase_unwraps() {
    let p = plot::unwrap_phase(&[-170.0, 175.0, 160.0, -179.0]);
    assert_eq!(p, vec![-170.0, -185.0, -200.0, -179.0]);
    let wraps = plot::unwrap_phase(&[170.0, -170.0, -150.0, 170.0]);
    assert_eq!(wraps, vec![170.0, 190.0, 210.0, 170.0]);
}

#[test]
fn nearest_and_interpolate() {
    let xs = [0.0, 1.0, 2.0, 4.0];
    let ys = [0.0, 10.0, 20.0, 0.0];
    assert_eq!(plot::nearest(&xs, 2.9), Some(2));
    assert_eq!(plot::nearest(&xs, 3.1), Some(3));
    assert_eq!(plot::nearest(&xs, -5.0), Some(0));
    assert_eq!(plot::nearest(&xs, 9.0), Some(3));
    assert_eq!(plot::interpolate(&xs, &ys, 3.0), Some(10.0));
    assert_eq!(plot::interpolate(&xs, &ys, 9.0), Some(0.0));
}

#[test]
fn decimation_keeps_the_envelope() {
    let n = 1_000_000;
    let xs: Vec<f64> = (0..n).map(|i| i as f64).collect();
    // A fast square wave between -1 and 1, with one spike.
    let mut ys: Vec<f64> = (0..n).map(|i| if i % 2 == 0 { 1.0 } else { -1.0 }).collect();
    ys[500_123] = 7.0;
    let cols = 800;
    let col = |x: f64| (x / n as f64 * cols as f64) as i64;
    let pts = plot::decimate(&xs, &ys, 0.0, n as f64, cols, col);
    assert!(pts.len() <= 4 * cols + 4, "{}", pts.len());
    assert!(pts.iter().any(|p| p[1] == 7.0), "the spike survives");
    assert!(pts.iter().any(|p| p[1] == -1.0));
    assert!(pts.windows(2).all(|w| w[1][0] >= w[0][0]), "in x order");
    // Few samples: all of them, plus one beyond each edge.
    let few = plot::decimate(&xs, &ys, 10.5, 20.5, cols, col);
    assert_eq!(few.first().unwrap()[0], 10.0);
    assert_eq!(few.last().unwrap()[0], 21.0);
}

fn rc() -> (crate::bench::Bench, sim::SimResult, Analysis) {
    use crate::bench::Bench;
    let mut b = Bench::new();
    b.source("VSource", Waveform::Dc { value: 1.0 }, 1.0, &["IN", "GND"]);
    b.place("Resistor", "1k", &["IN", "OUT"]);
    b.place("Capacitor", "1u", &["OUT", "GND"]);
    let a = Analysis::new("AC", AnalysisKind::Ac { start: 1.0, stop: 1e5, points_per_decade: 10 });
    // A fake result: the netlist of the circuit and a made-up AC plot.
    let netlist = sim::netlist::build(b.project(), &b.conn(), &a);
    let n = 51;
    let f: Vec<f64> = (0..n).map(|i| 10f64.powf(i as f64 / 10.0)).collect();
    let vec = |name: &str, re: Vec<f64>, im: Vec<f64>| sim::Vector {
        name: name.into(),
        kind: "voltage".into(),
        re,
        im: Some(im),
    };
    let out: Vec<(f64, f64)> = f
        .iter()
        .map(|f| {
            let w = 2.0 * std::f64::consts::PI * f * 1e-3;
            (1.0 / (1.0 + w * w), -w / (1.0 + w * w))
        })
        .collect();
    let plot = sim::Plot {
        name: "AC Analysis".into(),
        complex: true,
        vectors: vec![
            sim::Vector { name: "frequency".into(), kind: "frequency".into(), re: f.clone(), im: Some(vec![0.0; n]) },
            vec("v(in)", vec![1.0; n], vec![0.0; n]),
            vec("v(out)", out.iter().map(|p| p.0).collect(), out.iter().map(|p| p.1).collect()),
        ],
    };
    (b, sim::SimResult { plot, netlist, log: String::new(), warnings: Vec::new() }, a)
}

#[test]
fn signals_default_to_named_nets() {
    let (b, r, mut a) = rc();
    let (s, total) = plot::signals(&a, &r, &b.conn());
    assert_eq!(s, vec![Signal::Voltage("IN".into()), Signal::Voltage("OUT".into())]);
    assert_eq!(total, 2);
    a.probes.push(Probe::Voltage { net: "OUT".into(), at: None });
    assert_eq!(plot::signals(&a, &r, &b.conn()).0, vec![Signal::Voltage("OUT".into())]);
}

#[test]
fn ac_traces_are_db_and_phase() {
    let (b, r, _a) = rc();
    let conn = b.conn();
    let signals = [Signal::Voltage("OUT".into()), Signal::Voltage("GONE".into())];
    let d = plot::data(&r, &signals, true, &conn).unwrap();
    assert!(d.log_x);
    assert_eq!(d.x_unit, "Hz");
    let labels: Vec<&str> = d.traces.iter().map(|t| t.label.as_str()).collect();
    assert_eq!(labels, ["V(OUT)", "V(OUT) phase"]);
    let missing = plot::Missing { label: "V(GONE)".into(), reason: "the net is gone".into(), signal: 1 };
    assert_eq!(d.missing, vec![missing]);
    assert_eq!((d.axis_unit(Axis::Left), d.axis_unit(Axis::Right)), ("dB", "°"));
    assert!(d.traces[0].y[0].abs() < 0.01, "0 dB at 1 Hz");
    // At the corner (159 Hz), -3 dB and -45°.
    let corner = 1.0 / (2.0 * std::f64::consts::PI * 1e-3);
    let db = plot::interpolate(&d.x, &d.traces[0].y, corner).unwrap();
    assert!((db + 3.0).abs() < 0.2, "{db}");
    let ph = plot::interpolate(&d.x, &d.traces[1].y, corner).unwrap();
    assert!((ph + 45.0).abs() < 3.0, "{ph}");
    // Without phase; the y fit of the visible part.
    let d = plot::data(&r, &signals[..1], false, &conn).unwrap();
    assert_eq!(d.traces.len(), 1);
    let (lo, hi) = plot::fit_y(&d, Axis::Left, 1.0, 10.0, &HashSet::new()).unwrap();
    assert!(lo > -1.0 && hi < 1.0, "{lo} {hi}");
    let csv = plot::csv(&d, &HashSet::new());
    assert!(csv.starts_with("Frequency (Hz),V(OUT) (dB)\n1,"), "{csv}");
    assert_eq!(csv.lines().count(), 52);
}

#[test]
fn voltage_probes_follow_their_object() {
    let (b, r, _a) = rc();
    let conn = b.conn();
    let label = b.project().schematic.labels.iter().find(|l| l.name == "OUT").unwrap().id;
    // The probe's name is out of date (the net was renamed): its label finds the net.
    let p = Probe::Voltage { net: "OLD".into(), at: Some(ObjectId::Label(label)) };
    assert_eq!(Signal::of_probe(&p, &conn), Signal::Voltage("OUT".into()));
    // The object is gone: the name.
    let p = Probe::Voltage { net: "IN".into(), at: Some(ObjectId::Label(Id::new())) };
    assert_eq!(Signal::of_probe(&p, &conn), Signal::Voltage("IN".into()));
    // A current that wasn't measured.
    let r1 = b
        .project()
        .schematic
        .symbols
        .iter()
        .find(|s| r.netlist.pins.iter().any(|p| p.symbol == Some(s.id) && p.refdes == "R1"))
        .unwrap()
        .id;
    let pin = r.netlist.pins.iter().find(|p| p.symbol == Some(r1)).unwrap().pin;
    let d = plot::data(&r, &[Signal::Current(r1, pin)], false, &conn).unwrap();
    assert_eq!(d.missing[0].label, "I(R1.1)");
    assert_eq!(d.missing[0].reason, "not measured in this result: run again");
}
