//! Measurements of traces: statistics, frequency, edges, AC peak and −3 dB, phase margin.

use std::f64::consts::PI;

use sim::measure::{measure, measure_ac, phase_margin};

fn near(a: f64, b: f64, tol: f64) -> bool {
    (a - b).abs() <= tol
}

#[test]
fn a_sine_measures_its_statistics_and_frequency() {
    let n = 10_001;
    let t: Vec<f64> = (0..n).map(|i| i as f64 * 1e-6).collect();
    let y: Vec<f64> = t.iter().map(|t| 1.0 + 2.0 * (2.0 * PI * 1e3 * t).sin()).collect();
    let m = measure(&t, &y, 0.0, 0.01).unwrap();
    assert!(near(m.min, -1.0, 1e-3) && near(m.max, 3.0, 1e-3), "{m:?}");
    assert!(near(m.peak_to_peak, 4.0, 1e-3));
    assert!(near(m.average, 1.0, 1e-4), "{m:?}");
    // RMS of 1 + 2 sin: sqrt(1 + 2) .
    assert!(near(m.rms, 3f64.sqrt(), 1e-4), "{m:?}");
    assert!(near(m.frequency.unwrap(), 1e3, 0.5), "{m:?}");
    // A window: the first half period only, from the interpolated ends.
    let half = measure(&t, &y, 0.0, 0.5e-3).unwrap();
    assert!(near(half.min, 1.0, 1e-3) && near(half.max, 3.0, 1e-3), "{half:?}");
    assert_eq!(half.frequency, None, "one crossing");
    assert!(measure(&t, &y, 1.0, 2.0).is_none(), "outside the run");
}

#[test]
fn edges_and_uneven_steps() {
    // A ramp from 0 to 1 between 1 and 2 s, back down between 4 and 6 s.
    let t = [0.0, 1.0, 2.0, 4.0, 6.0, 7.0];
    let y = [0.0, 0.0, 1.0, 1.0, 0.0, 0.0];
    let m = measure(&t, &y, 0.0, 7.0).unwrap();
    assert!(near(m.rise.unwrap(), 0.8, 1e-9), "{m:?}");
    assert!(near(m.fall.unwrap(), 1.6, 1e-9), "{m:?}");
    // Average over time, not over samples: area 0.5 + 2 + 1 = 3.5 over 7 s.
    assert!(near(m.average, 0.5, 1e-9), "{m:?}");
    let flat = measure(&t, &[2.0; 6], 0.0, 7.0).unwrap();
    assert_eq!((flat.rise, flat.fall, flat.frequency), (None, None, None));
}

#[test]
fn ac_peak_and_corners() {
    // A first-order low-pass at 1 kHz.
    let f: Vec<f64> = (0..=60).map(|i| 10f64.powf(i as f64 / 10.0)).collect();
    let db: Vec<f64> = f.iter().map(|f| -10.0 * (1.0 + (f / 1e3).powi(2)).log10()).collect();
    let m = measure_ac(&f, &db, 1.0, 1e6).unwrap();
    assert!(near(m.peak_db, 0.0, 1e-3) && m.peak_frequency == 1.0, "{m:?}");
    assert_eq!(m.minus_3db.0, None);
    assert!(near(m.minus_3db.1.unwrap(), 1e3, 20.0), "{m:?}");
    // An integrator-like loop gain: 0 dB at 1 kHz with −120° there → 60° margin.
    let db: Vec<f64> = f.iter().map(|f| -20.0 * (f / 1e3).log10()).collect();
    let phase: Vec<f64> = f.iter().map(|f| -90.0 - 30.0 * (f / 1e3).log10().clamp(-1.0, 1.0) - 30.0).collect();
    let pm = phase_margin(&f, &db, &phase, 1.0, 1e6).unwrap();
    assert!(near(pm, 60.0, 0.5), "{pm}");
}
