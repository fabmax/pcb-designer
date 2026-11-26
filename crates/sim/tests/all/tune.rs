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

#[test]
fn step_values() {
    let lin = StepValues::Linear { start: 0.0, stop: 1.0, step: 0.25 };
    assert_eq!(lin.points(), vec![0.0, 0.25, 0.5, 0.75, 1.0]);
    let down = StepValues::Linear { start: 1.0, stop: 0.0, step: -0.5 };
    assert_eq!(down.points(), vec![1.0, 0.5, 0.0]);
    let log = StepValues::Log { start: 1e3, stop: 1e5, points_per_decade: 2 };
    let p = log.points();
    assert_eq!(p.len(), 5);
    assert!((p[1] - 1e3 * 10f64.sqrt()).abs() < 1e-6 && (p[4] - 1e5).abs() < 1e-6, "{p:?}");
    assert!(StepValues::List { values: vec![] }.problem().is_some());
    assert!(StepValues::Linear { start: 0.0, stop: 1.0, step: 0.001 }.problem().unwrap().contains("at most"));
    assert!(StepValues::Linear { start: 0.0, stop: 1.0, step: 0.0 }.problem().is_some());
    assert!(StepValues::Log { start: 0.0, stop: 1.0, points_per_decade: 3 }.problem().is_some());
    assert!(lin.problem().is_none());
}

#[test]
fn default_steps() {
    let (b, v1, r1, _) = circuit();
    let s = tune::default_step(b.project(), &TuneParam::Value { component: r1 }).unwrap();
    assert_eq!(s.values, StepValues::Log { start: 1e3, stop: 1e5, points_per_decade: 3 });
    assert_eq!(s.values.points().len(), 7);
    let a = tune::default_step(b.project(), &TuneParam::Source { component: v1, field: "amplitude".into() }).unwrap();
    assert_eq!(a.values.points(), vec![0.0, 0.5, 1.0, 1.5, 2.0]);
}

#[test]
fn overrides_stack_a_step_on_the_tuned_values() {
    let (b, v1, r1, _) = circuit();
    let tuned = Overrides::new(b.project(), &[(TuneParam::Value { component: r1 }, 4700.0)]);
    let amp = TuneParam::Source { component: v1, field: "amplitude".into() };
    let o = tuned.try_with(b.project(), &amp, 0.3).unwrap();
    let a = Analysis::new("T", AnalysisKind::Tran { stop: 1e-3, step: None, start: 0.0, max_step: None });
    let text = sim::netlist::build_with(b.project(), &b.conn(), &a, &o).text;
    assert!(text.contains("R1 in out 4700") && text.contains("SIN(0 0.3 1000"), "{text}");
    // A field the source doesn't have, a part that's gone.
    assert!(tuned.try_with(b.project(), &TuneParam::Source { component: v1, field: "width".into() }, 1.0).is_none());
    assert!(tuned.try_with(b.project(), &TuneParam::Value { component: Id::new() }, 1.0).is_none());
}

/// A switch's state and a potentiometer's position: tunable, read from the model's
/// default until set, written into the component's parameters.
#[test]
fn switch_states_and_wiper_positions_are_tunable() {
    let mut b = Bench::new();
    let (sw, _) = b.place("SW_SPDT", "", &["IN", "A", "B"]);
    let (rv, _) = b.place("Potentiometer", "10k", &["IN", "GND", "W"]);
    let state = TuneParam::Param { component: sw, name: "state".into() };
    let pos = TuneParam::Param { component: rv, name: "pos".into() };
    let params = tune::tunable(b.project());
    assert!(params.contains(&state) && params.contains(&pos));
    assert!(params.contains(&TuneParam::Value { component: rv }));
    assert_eq!(tune::current(b.project(), &state), Some(0.0));
    assert_eq!(tune::current(b.project(), &pos), Some(0.5));
    assert_eq!(tune::label(b.project(), &pos), "RV1 position");
    assert_eq!(tune::short_label(b.project(), &state), "SW1.state");
    let row = tune::default_row(b.project(), &pos).unwrap();
    assert_eq!((row.min, row.max, row.param.is_log()), (0.0, 1.0, false));
    // A switch steps through its two states.
    let step = tune::default_step(b.project(), &state).unwrap();
    assert_eq!(step.values, StepValues::Linear { start: 0.0, stop: 1.0, step: 1.0 });

    let a = Analysis::new("OP", AnalysisKind::Op);
    let o = Overrides::new(b.project(), &[(state.clone(), 0.8), (pos.clone(), 0.25)]);
    let text = sim::netlist::build_with(b.project(), &b.conn(), &a, &o).text;
    assert!(text.contains("state=1") && text.contains("pos=0.25"), "{text}");
    let mut c = b.component(rv);
    assert!(tune::apply(&mut c, &pos, 0.3333));
    assert_eq!(c.sim.params["pos"], "0.333");
    // The tuner keeps what the component would read back.
    let mut t = Tuner::default();
    t.set(&state, 0.7);
    assert_eq!(t.value(b.project(), &state), Some(1.0));
}
