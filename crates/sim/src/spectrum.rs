//! Distortion and spectra of time signals: the harmonics of a periodic trace (THD,
//! THD+N) and the magnitude spectrum the plot shows instead of the waveform.
//!
//! Harmonics are integrated exactly over the piecewise-linear trace (like RMS in
//! [`crate::measure`]), over whole periods of the fundamental, so uneven transient steps
//! weigh right and need no resampling. The spectrum resamples evenly and takes an FFT
//! with a Hann window, which suits any signal, periodic or not.

use crate::measure;
use crate::plot::{self, PlotData, Trace};

/// The highest harmonic measured.
pub const HARMONICS: usize = 10;

/// The harmonic content of a periodic signal.
#[derive(Clone, Debug, PartialEq)]
pub struct Distortion {
    /// The fundamental's frequency, from the signal's crossings.
    pub fundamental: f64,
    /// The fundamental's RMS value.
    pub level: f64,
    /// Whole periods measured: the last ones of the window.
    pub periods: usize,
    /// The window measured (whole periods, ending where the window ends).
    pub from: f64,
    pub to: f64,
    /// Harmonics 2, 3, … (as far as [`HARMONICS`] and half the sample rate allow), each
    /// as a fraction of the fundamental.
    pub harmonics: Vec<f64>,
    /// Total harmonic distortion: the RMS of the harmonics over the fundamental's.
    pub thd: f64,
    /// THD plus noise: everything but DC and the fundamental, over the fundamental. With
    /// a clean signal what's above THD is the simulation's numerical noise and the
    /// coarseness of its time steps.
    pub thd_n: f64,
}

/// The distortion of `ys` over time `xs`, measured over the whole periods of its
/// fundamental that fit between `lo` and `hi` (the last ones, as the circuit is most
/// settled there). `None` without a periodic signal or a whole period.
pub fn distortion(xs: &[f64], ys: &[f64], lo: f64, hi: f64) -> Option<Distortion> {
    let m = measure::measure(xs, ys, lo, hi)?;
    let f0 = m.frequency?;
    let lo = lo.max(*xs.first()?);
    let hi = hi.min(*xs.last()?);
    // The frequency from crossings is a few ppm off: a window a hair short of whole
    // periods still counts them (the missing sliver leaks less than −100 dB).
    let periods = ((hi - lo) * f0 + 1e-3).floor() as usize;
    if periods == 0 {
        return None;
    }
    let from = (hi - periods as f64 / f0).max(lo);
    let (x, y) = measure::window(xs, ys, from, hi);
    if x.len() < 3 {
        return None;
    }
    let span = x[x.len() - 1] - x[0];
    let (dc, rms) = mean_rms(&x, &y);
    let ac_sq = (rms * rms - dc * dc).max(0.0);
    let rate = (x.len() - 1) as f64 / span;
    let amplitude = |h: usize| harmonic(&x, &y, 2.0 * std::f64::consts::PI * h as f64 * f0) / span;
    let a1 = amplitude(1);
    if a1 <= 0.0 || !a1.is_finite() {
        return None;
    }
    let top = (2..=HARMONICS).take_while(|&h| h as f64 * f0 < rate / 2.0).last().unwrap_or(1);
    let harmonics: Vec<f64> = (2..=top).map(|h| amplitude(h) / a1).collect();
    let thd = harmonics.iter().map(|h| h * h).sum::<f64>().sqrt();
    let fund_sq = a1 * a1 / 2.0;
    let thd_n = ((ac_sq - fund_sq).max(0.0) / fund_sq).sqrt().max(thd);
    Some(Distortion { fundamental: f0, level: fund_sq.sqrt(), periods, from, to: hi, harmonics, thd, thd_n })
}

/// The mean and RMS of a piecewise-linear signal.
fn mean_rms(x: &[f64], y: &[f64]) -> (f64, f64) {
    let (mut sum, mut sq) = (0.0, 0.0);
    for k in 1..x.len() {
        let dx = x[k] - x[k - 1];
        let (a, b) = (y[k - 1], y[k]);
        sum += dx * (a + b) / 2.0;
        sq += dx * (a * a + a * b + b * b) / 3.0;
    }
    let span = x[x.len() - 1] - x[0];
    (sum / span, (sq / span).max(0.0).sqrt())
}

/// `2·|∫ y(t)·e^(−iωt) dt|` over the samples, `y` linear between them: the amplitude
/// times the span when the span is whole periods of ω. Integrated by parts, the value
/// terms cancel between pieces (the signal is continuous), leaving the ends and each
/// piece's slope term, written with sin(ωd/2)/(ωd/2) so short steps don't cancel out.
fn harmonic(x: &[f64], y: &[f64], w: f64) -> f64 {
    let t0 = x[0];
    let (mut re, mut im) = (0.0, 0.0);
    // The ends: i/ω · (y_b e^(−iωt_b) − y_a e^(−iωt_a)).
    let n = x.len() - 1;
    let end = |t: f64, v: f64| {
        let p = w * (t - t0);
        // i · v · (cos p − i sin p) = v sin p + i v cos p
        (v * p.sin() / w, v * p.cos() / w)
    };
    let (br, bi) = end(x[n], y[n]);
    let (ar, ai) = end(x[0], y[0]);
    re += br - ar;
    im += bi - ai;
    for k in 1..x.len() {
        let d = x[k] - x[k - 1];
        if d <= 0.0 {
            continue;
        }
        let half = w * d / 2.0;
        let s = if half.abs() < 1e-8 { 1.0 } else { half.sin() / half };
        let p = w * ((x[k] + x[k - 1]) / 2.0 - t0);
        let c = -(y[k] - y[k - 1]) * s / w;
        // c · i · (cos p − i sin p) = c sin p + i c cos p
        re += c * p.sin();
        im += c * p.cos();
    }
    2.0 * re.hypot(im)
}

/// The magnitude spectrum of `ys` over time `xs` between `lo` and `hi`: frequencies
/// (from 1/span up to half the sample rate) and RMS levels in dB (of 1 V, 1 A, …).
/// Resampled evenly to a power of two at least as dense as the samples, Hann windowed.
pub fn spectrum(xs: &[f64], ys: &[f64], lo: f64, hi: f64) -> Option<(Vec<f64>, Vec<f64>)> {
    let (x, y) = measure::window(xs, ys, lo, hi);
    let n = points(x.len())?;
    let span = x[x.len() - 1] - x[0];
    if span <= 0.0 {
        return None;
    }
    let mut buf = resample(&x, &y, n);
    let mean = buf.iter().map(|c| c.0).sum::<f64>() / n as f64;
    for (k, c) in buf.iter_mut().enumerate() {
        let hann = 0.5 - 0.5 * (2.0 * std::f64::consts::PI * k as f64 / n as f64).cos();
        c.0 = (c.0 - mean) * hann;
    }
    fft(&mut buf);
    let df = 1.0 / span;
    // A sine of amplitude A gives |X| = A·N/4 at its bin (the window's gain is ½); its RMS
    // is A/√2.
    let scale = 4.0 / n as f64 / std::f64::consts::SQRT_2;
    let (f, db) = (1..n / 2)
        .map(|k| {
            let mag = buf[k].0.hypot(buf[k].1) * scale;
            (k as f64 * df, (20.0 * mag.max(1e-15).log10()).max(-300.0))
        })
        .unzip();
    Some((f, db))
}

/// Points of the resampled window: a power of two from 1024 up to 2^18.
fn points(samples: usize) -> Option<usize> {
    (samples >= 2).then(|| samples.next_power_of_two().clamp(1 << 10, 1 << 18))
}

/// `n` even samples from `x[0]` to just before `x[last]`, as complex numbers.
fn resample(x: &[f64], y: &[f64], n: usize) -> Vec<(f64, f64)> {
    let (t0, span) = (x[0], x[x.len() - 1] - x[0]);
    let mut j = 0;
    (0..n)
        .map(|k| {
            let t = t0 + span * k as f64 / n as f64;
            while j + 2 < x.len() && x[j + 1] <= t {
                j += 1;
            }
            let (a, b) = (x[j], x[j + 1]);
            let f = if b > a { ((t - a) / (b - a)).clamp(0.0, 1.0) } else { 0.0 };
            (y[j] + f * (y[j + 1] - y[j]), 0.0)
        })
        .collect()
}

/// An in-place radix-2 FFT (`buf.len()` a power of two).
fn fft(buf: &mut [(f64, f64)]) {
    let n = buf.len();
    let mut j = 0;
    for i in 1..n {
        let mut bit = n >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j |= bit;
        if i < j {
            buf.swap(i, j);
        }
    }
    let mut len = 2;
    while len <= n {
        let ang = -2.0 * std::f64::consts::PI / len as f64;
        for start in (0..n).step_by(len) {
            for k in 0..len / 2 {
                let (s, c) = (ang * k as f64).sin_cos();
                let (ar, ai) = buf[start + k];
                let (br, bi) = buf[start + k + len / 2];
                let (tr, ti) = (br * c - bi * s, br * s + bi * c);
                buf[start + k] = (ar + tr, ai + ti);
                buf[start + k + len / 2] = (ar - tr, ai - ti);
            }
        }
        len <<= 1;
    }
}

/// The spectrum plot of a time plot's traces between `lo` and `hi` (each trace in dB on
/// log frequency; the steps and missing signals carried over). `None` for plots that
/// aren't over time or without two samples in the window.
pub fn data(time: &PlotData, lo: f64, hi: f64) -> Option<PlotData> {
    if time.x_unit != "s" {
        return None;
    }
    let mut x = None;
    let mut traces = Vec::new();
    for t in &time.traces {
        let (f, db) = spectrum(&time.x, &t.y, lo, hi)?;
        x.get_or_insert(f);
        traces.push(Trace { unit: "dB", y: db, logic: None, ..t.clone() });
    }
    // Every trace on the left axis: they share the unit now.
    for t in &mut traces {
        t.axis = plot::Axis::Left;
    }
    Some(PlotData {
        x_label: "Frequency".into(),
        x_unit: "Hz",
        log_x: true,
        x: x?,
        traces,
        missing: time.missing.clone(),
        steps: time.steps.clone(),
    })
}

/// Harmonics shown as columns of the Measure table.
const TABLE_HARMONICS: usize = 5;

/// The distortion table of a time plot's traces between `lo` and `hi`: columns
/// fundamental, its level, THD, THD+N and harmonics 2 to 5 (in dB of the fundamental).
pub fn table(time: &PlotData, lo: f64, hi: f64) -> (Vec<&'static str>, Vec<Vec<String>>) {
    let cols = vec!["Fundamental", "Level", "THD", "THD+N", "H2", "H3", "H4", "H5"];
    let rows = time
        .traces
        .iter()
        .map(|t| match distortion(&time.x, &t.y, lo, hi) {
            Some(d) => {
                let mut row = vec![
                    plot::label(d.fundamental, "Hz"),
                    format!("{} rms", plot::label(d.level, t.unit)),
                    percent(d.thd),
                    percent(d.thd_n),
                ];
                for h in 0..TABLE_HARMONICS - 1 {
                    row.push(d.harmonics.get(h).map_or("—".into(), |r| dbc(*r)));
                }
                row
            }
            None => vec!["—".to_string(); cols.len()],
        })
        .collect();
    (cols, rows)
}

/// A ratio as a percentage with three significant digits ("1.52 %", "0.00341 %"), under
/// 0.0001 % (−120 dB, below what a simulation resolves) as "< 0.0001 %".
pub fn percent(ratio: f64) -> String {
    let p = ratio * 100.0;
    if !p.is_finite() {
        return "—".into();
    }
    if p < 1e-4 {
        return "< 0.0001 %".into();
    }
    let digits = (2 - p.abs().log10().floor() as i32).max(0) as usize;
    format!("{p:.digits$} %")
}

/// A ratio in dB ("−42.1 dBc").
pub fn dbc(ratio: f64) -> String {
    if ratio <= 0.0 {
        return "−∞ dBc".into();
    }
    format!("{:.1} dBc", 20.0 * ratio.log10()).replace('-', "−")
}
