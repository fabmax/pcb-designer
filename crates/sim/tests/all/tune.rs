//! The tuner's parameters, ranges and overrides.

use model::*;
use sim::tune::{self, Overrides, Tuner};

use crate::bench::Bench;

fn circuit() -> (Bench, Id, Id, Id) {
    let mut b = Bench::new();
    let v1 = b.source(
        "VSource",
        Waveform::Sine { offset: 0.0, amplitude: 1.0, frequency: 1e3, delay: 0.0, damping: 0.0, phase: 0.0 },
        1.0,
        &["IN", "GND"],
    );
    let (r1, _) = b.place("Resistor", "10k", &["IN", "OUT"]);
    let (c1, _) = b.place("Capacitor", "100n", &["OUT", "GND"]);
    (b, v1, r1, c1)
}

#[test]
fn values_and_source_fields_are_tunable() {
    let (b, v1, r1, c1) = circuit();
    let params = tune::tunable(b.project());
    assert!(params.contains(&TuneParam::Value { component: r1 }));
    assert!(params.contains(&TuneParam::Value { component: c1 }));
    let amp = TuneParam::Source { component: v1, field: "amplitude".into() };
    assert!(params.contains(&amp));
    assert!(params.contains(&TuneParam::Source { component: v1, field: "ac".into() }));
    // A source's value text isn't a part value.
    assert!(!params.contains(&TuneParam::Value { component: v1 }));
    assert_eq!(tune::label(b.project(), &amp), "V1 amplitude");
    assert_eq!(tune::unit(b.project(), &amp), "V");
    assert_eq!(tune::unit(b.project(), &TuneParam::Source { component: v1, field: "frequency".into() }), "Hz");
    assert_eq!(tune::unit(b.project(), &TuneParam::Value { component: c1 }), "F");
}

#[test]
fn default_rows_span_a_decade_or_twice_the_level() {
    let (b, v1, r1, c1) = circuit();
    let r = tune::default_row(b.project(), &TuneParam::Value { component: r1 }).unwrap();
    assert_eq!((r.min, r.max, r.snap), (1e3, 1e5, Snap::E24));
    let c = tune::default_row(b.project(), &TuneParam::Value { component: c1 }).unwrap();
    assert_eq!(c.snap, Snap::E12);
    let a = tune::default_row(b.project(), &TuneParam::Source { component: v1, field: "amplitude".into() }).unwrap();
    assert_eq!((a.min, a.max, a.snap), (0.0, 2.0, Snap::None));
    let o = tune::default_row(b.project(), &TuneParam::Source { component: v1, field: "offset".into() }).unwrap();
    assert_eq!((o.min, o.max), (-1.0, 1.0));
}

#[test]
fn sliders_are_log_for_values_and_snap() {
    let (b, _, r1, _) = circuit();
    let row = tune::default_row(b.project(), &TuneParam::Value { component: r1 }).unwrap();
    assert_eq!(tune::at(&row, 0.0), 1e3);
    assert_eq!(tune::at(&row, 1.0), 1e5);
    // Halfway on a log slider is the geometric middle: 10k.
    assert!((tune::at(&row, 0.5) - 1e4).abs() < 1e-6);
    // Snapped to E24: 10^0.25 · 10k ≈ 17.8k → 18k.
    assert!((tune::at(&row, 0.625) - 18e3).abs() < 1e-6, "{}", tune::at(&row, 0.625));
    assert!((tune::position(&row, 1e4) - 0.5).abs() < 1e-9);
    assert_eq!(tune::position(&row, 1e9), 1.0);
}

#[test]
fn overrides_change_the_netlist_not_the_document() {
    let (b, v1, r1, _) = circuit();
    let a = Analysis::new("AC", AnalysisKind::Ac { start: 1.0, stop: 1e6, points_per_decade: 10 });
    let values =
        [(TuneParam::Value { component: r1 }, 4700.0), (TuneParam::Source { component: v1, field: "ac".into() }, 0.5)];
    let o = Overrides::new(b.project(), &values);
    let plain = sim::netlist::build(b.project(), &b.conn(), &a).text;
    let tuned = sim::netlist::build_with(b.project(), &b.conn(), &a, &o).text;
    assert!(plain.contains("R1 in out 10000"), "{plain}");
    assert!(tuned.contains("R1 in out 4700"), "{tuned}");
    assert!(tuned.contains("AC 0.5"), "{tuned}");
    assert_eq!(b.component(r1).value, "10k");
}

#[test]
fn the_tuner_reports_what_differs_from_the_document() {
    let (b, v1, r1, _) = circuit();
    let p = b.project();
    let rows: Vec<TuneRow> =
        [TuneParam::Value { component: r1 }, TuneParam::Source { component: v1, field: "amplitude".into() }]
            .iter()
            .map(|q| tune::default_row(p, q).unwrap())
            .collect();
    let mut t = Tuner::default();
    assert!(t.changed(p, &rows, None).is_empty());
    assert_eq!(t.value(p, &rows[0].param), Some(1e4));
    // Values round to what the value text keeps.
    t.set(&rows[0].param, 4712.34);
    t.set(&rows[1].param, 1.0);
    assert_eq!(t.changed(p, &rows, None), vec![(rows[0].param.clone(), 4712.0)]);
    assert!(t.changed(p, &rows, Some(&rows[0].param)).is_empty());
    assert!(!t.overrides(p, &rows, None).is_empty());
    // Rows removed: their values go.
    t.retain(&rows[1..]);
    assert!(t.changed(p, &rows, None).is_empty());
}
