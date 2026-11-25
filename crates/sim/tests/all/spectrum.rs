//! Distortion (harmonics, THD, THD+N) and spectra of known waveforms.

use std::f64::consts::PI;

use sim::plot::{Axis, PlotData, Trace};
use sim::spectrum::{dbc, distortion, percent, spectrum};

/// `n` samples over `t1` seconds of `f`, with uneven steps when `uneven`.
fn sampled(n: usize, t1: f64, uneven: bool, f: impl Fn(f64) -> f64) -> (Vec<f64>, Vec<f64>) {
    let t: Vec<f64> = (0..n)
        .map(|i| {
            let u = i as f64 / (n - 1) as f64;
            // Steps between 0.6 and 1.4 times the mean, the ends fixed.
            t1 * if uneven { u + 0.4 * (2.0 * PI * 7.3 * u).sin() / (2.0 * PI * 7.3) } else { u }
        })
        .collect();
    let y = t.iter().map(|&t| f(t)).collect();
    (t, y)
}

#[test]
fn a_pure_sine_has_next_to_no_distortion() {
    let (t, y) = sampled(20_001, 0.01, false, |t| 0.5 + 2.0 * (2.0 * PI * 1e3 * t).sin());
    let d = distortion(&t, &y, 0.0, 0.01).unwrap();
    assert!((d.fundamental - 1e3).abs() < 1e-3, "{d:?}");
    assert_eq!(d.periods, 10);
    // Linear pieces read a sine a little low: about (ωΔt)²/12, 1e-6 here.
    assert!((d.level - 2.0 / 2f64.sqrt()).abs() < 1e-5, "{d:?}");
    assert_eq!(d.harmonics.len(), 9, "harmonics 2 to 10");
    assert!(d.thd < 1e-6, "{d:?}");
    // What linear interpolation of 2000 samples a period leaves: about 1e-6.
    assert!(d.thd_n < 1e-5, "{d:?}");
}

#[test]
fn known_harmonics_are_found_on_uneven_steps() {
    // 1 % second and 0.3 % third harmonic, phase shifted.
    let f = |t: f64| {
        let w = 2.0 * PI * 1e3 * t;
        w.sin() + 0.01 * (2.0 * w + 0.7).sin() + 0.003 * (3.0 * w - 1.1).cos()
    };
    let (t, y) = sampled(30_001, 0.0105, true, f);
    // A window that isn't whole periods: the last 10 periods of it are measured.
    let d = distortion(&t, &y, 0.0003, 0.0105).unwrap();
    assert_eq!(d.periods, 10);
    assert!((d.to - 0.0105).abs() < 1e-12 && (d.from - 0.0005).abs() < 1e-8, "{d:?}");
    assert!((d.harmonics[0] - 0.01).abs() < 2e-5, "{d:?}");
    assert!((d.harmonics[1] - 0.003).abs() < 2e-5, "{d:?}");
    assert!(d.harmonics[2..].iter().all(|h| *h < 2e-5), "{d:?}");
    let thd = (0.01f64.powi(2) + 0.003f64.powi(2)).sqrt();
    assert!((d.thd - thd).abs() < 2e-5, "{d:?}");
    assert!(d.thd_n >= d.thd && d.thd_n < thd * 1.01, "{d:?}");
}

#[test]
fn a_clipped_sine_has_odd_harmonics() {
    let (t, y) = sampled(40_001, 0.01, false, |t| (1.5 * (2.0 * PI * 1e3 * t).sin()).clamp(-1.0, 1.0));
    let d = distortion(&t, &y, 0.0, 0.01).unwrap();
    // Symmetric clipping: odd harmonics only.
    assert!(d.harmonics[0] < 1e-4 && d.harmonics[2] < 1e-4, "{d:?}");
    assert!(d.harmonics[1] > 0.02, "a strong third: {d:?}");
    assert!(d.thd > 0.03 && d.thd < 0.2, "{d:?}");
}

#[test]
fn no_distortion_without_a_period() {
    let (t, y) = sampled(1001, 1.0, false, |t| t);
    assert!(distortion(&t, &y, 0.0, 1.0).is_none(), "a ramp");
    let (t, y) = sampled(1001, 0.0005, false, |t| (2.0 * PI * 1e3 * t).sin());
    assert!(distortion(&t, &y, 0.0, 0.0005).is_none(), "half a period");
}

#[test]
fn harmonics_stop_at_half_the_sample_rate() {
    // 16 samples a period: harmonics up to the 7th.
    let (t, y) = sampled(161, 0.01, false, |t| (2.0 * PI * 1e3 * t).sin());
    let d = distortion(&t, &y, 0.0, 0.01).unwrap();
    assert_eq!(d.harmonics.len(), 6, "{d:?}");
}

#[test]
fn the_spectrum_shows_the_sine_and_its_harmonic() {
    // 1 V rms at 1 kHz and 10 mV rms at 3 kHz, over 20 periods.
    let a = 2f64.sqrt();
    let (t, y) = sampled(10_001, 0.02, false, |t| {
        let w = 2.0 * PI * 1e3 * t;
        a * w.sin() + 0.01 * a * (3.0 * w).sin()
    });
    let (f, db) = spectrum(&t, &y, 0.0, 0.02).unwrap();
    assert!((f[0] - 50.0).abs() < 1e-9, "resolution 1/span: {}", f[0]);
    let at = |hz: f64| db[f.iter().position(|v| (v - hz).abs() < 1e-6).unwrap()];
    assert!(at(1e3).abs() < 0.01, "0 dB: {}", at(1e3));
    assert!((at(3e3) + 40.0).abs() < 0.05, "−40 dB: {}", at(3e3));
    assert!(at(2e3) < -100.0, "nothing at 2 kHz: {}", at(2e3));
}

#[test]
fn spectrum_plots_follow_the_time_plot() {
    let (t, y) = sampled(2001, 0.01, false, |t| (2.0 * PI * 1e3 * t).sin());
    let trace = |label: &str, unit, axis| Trace {
        label: label.into(),
        unit,
        axis,
        dashed: false,
        signal: 0,
        step: 0,
        y: y.clone(),
        logic: None,
    };
    let time = PlotData {
        x_label: "Time".into(),
        x_unit: "s",
        log_x: false,
        x: t,
        traces: vec![trace("V(OUT)", "V", Axis::Left), trace("I(R1.1)", "A", Axis::Right)],
        missing: Vec::new(),
        steps: Vec::new(),
    };
    let s = sim::spectrum::data(&time, 0.0, 0.01).unwrap();
    assert!(s.log_x && s.x_unit == "Hz");
    assert!(s.traces.iter().all(|t| t.unit == "dB" && t.axis == Axis::Left));
    assert_eq!(s.traces[1].label, "I(R1.1)");
    let (cols, rows) = sim::spectrum::table(&time, 0.0, 0.01);
    assert_eq!(cols[2], "THD");
    assert_eq!(rows[0][0], "1 kHz");
    let (cols, rows) = sim::measure::table(&time, 0.0, 0.01);
    assert_eq!(cols.last(), Some(&"THD"));
    assert!(rows[0].last().unwrap().ends_with(" %"), "{rows:?}");
}

#[test]
fn ratios_read_well() {
    assert_eq!(percent(0.01523), "1.52 %");
    assert_eq!(percent(0.0000341), "0.00341 %");
    assert_eq!(percent(0.25), "25.0 %");
    assert_eq!(percent(4.45e-8), "< 0.0001 %");
    assert_eq!(dbc(0.01), "−40.0 dBc");
}
