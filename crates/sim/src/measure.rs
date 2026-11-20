//! Measurements of a trace over an x range: statistics, frequency and edges of time
//! signals, the peak and −3 dB points of an AC magnitude, the phase margin of a loop
//! gain. Used by the plot's Measure table and by agents.

/// The samples between `lo` and `hi`, with values interpolated at both ends.
fn window(xs: &[f64], ys: &[f64], lo: f64, hi: f64) -> (Vec<f64>, Vec<f64>) {
    let (mut wx, mut wy) = (Vec::new(), Vec::new());
    if xs.len() != ys.len() || xs.is_empty() || hi < lo {
        return (wx, wy);
    }
    let lo = lo.max(xs[0]);
    let hi = hi.min(xs[xs.len() - 1]);
    if hi < lo {
        return (wx, wy);
    }
    let at = |x: f64| crate::plot::interpolate(xs, ys, x);
    if let Some(y) = at(lo) {
        wx.push(lo);
        wy.push(y);
    }
    for (x, y) in xs.iter().zip(ys) {
        if *x > lo && *x < hi {
            wx.push(*x);
            wy.push(*y);
        }
    }
    if hi > lo
        && let Some(y) = at(hi)
    {
        wx.push(hi);
        wy.push(y);
    }
    (wx, wy)
}

/// Measurements of a time-domain (or DC sweep) trace.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Measures {
    pub min: f64,
    pub max: f64,
    /// Over x (trapezoids, so uneven time steps weigh right).
    pub average: f64,
    pub rms: f64,
    pub peak_to_peak: f64,
    /// From the crossings of the mean upwards (with hysteresis); needs two.
    pub frequency: Option<f64>,
    /// 10 % to 90 % of the range, the first rising edge in the window.
    pub rise: Option<f64>,
    /// 90 % to 10 %, the first falling edge.
    pub fall: Option<f64>,
}

/// Measures `ys` over `xs` between `lo` and `hi`. `None` without samples there.
pub fn measure(xs: &[f64], ys: &[f64], lo: f64, hi: f64) -> Option<Measures> {
    let (x, y) = window(xs, ys, lo, hi);
    if y.is_empty() {
        return None;
    }
    let min = y.iter().copied().fold(f64::INFINITY, f64::min);
    let max = y.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let span = x[x.len() - 1] - x[0];
    let (average, rms) = if span > 0.0 {
        let mut sum = 0.0;
        let mut sq = 0.0;
        for k in 1..x.len() {
            let dx = x[k] - x[k - 1];
            let (a, b) = (y[k - 1], y[k]);
            sum += dx * (a + b) / 2.0;
            // The exact integral of a linear piece's square.
            sq += dx * (a * a + a * b + b * b) / 3.0;
        }
        (sum / span, (sq / span).max(0.0).sqrt())
    } else {
        (y[0], y[0].abs())
    };
    let pp = max - min;
    let mut m = Measures { min, max, average, rms, peak_to_peak: pp, ..Default::default() };
    if pp > 0.0 && pp.is_finite() {
        m.frequency = frequency(&x, &y, average, pp * 0.1);
        let (l, h) = (min + 0.1 * pp, min + 0.9 * pp);
        m.rise = edge(&x, &y, l, h);
        let neg: Vec<f64> = y.iter().map(|v| -v).collect();
        m.fall = edge(&x, &neg, -h, -l);
    }
    Some(m)
}

/// Where the line from (x0, y0) to (x1, y1) reaches `level`.
fn cross(x0: f64, y0: f64, x1: f64, y1: f64, level: f64) -> f64 {
    if y1 == y0 { x0 } else { x0 + (level - y0) / (y1 - y0) * (x1 - x0) }
}

/// The frequency from crossings of `mid`: a crossing counts once the signal went
/// from below `mid - hyst` to above `mid + hyst` (or back). Upward crossings when there
/// are two (exact for any duty cycle), else every crossing as half a period.
fn frequency(x: &[f64], y: &[f64], mid: f64, hyst: f64) -> Option<f64> {
    let mut state = 0i8; // -1 below, 1 above, 0 not yet either
    let mut last_side: Option<usize> = None;
    let mut up = Vec::new();
    let mut all = Vec::new();
    for k in 0..y.len() {
        let side = if y[k] < mid - hyst {
            -1
        } else if y[k] > mid + hyst {
            1
        } else {
            0
        };
        if side != 0 && side != state {
            if state != 0
                && let Some(j) = last_side
            {
                // The crossing of `mid` between the last sample on the old side and here.
                let i = (j..k).find(|&i| (y[i] - mid).signum() != (y[i + 1] - mid).signum()).unwrap_or(j);
                let t = cross(x[i], y[i], x[i + 1], y[i + 1], mid);
                all.push(t);
                if side == 1 {
                    up.push(t);
                }
            }
            state = side;
        }
        if side != 0 {
            last_side = Some(k);
        }
    }
    let period = if up.len() >= 2 {
        (up[up.len() - 1] - up[0]) / (up.len() - 1) as f64
    } else if all.len() >= 3 {
        2.0 * (all[all.len() - 1] - all[0]) / (all.len() - 1) as f64
    } else {
        return None;
    };
    (period > 0.0).then(|| 1.0 / period)
}

/// The time from `low` to `high` of the first rise that starts below `low`.
fn edge(x: &[f64], y: &[f64], low: f64, high: f64) -> Option<f64> {
    let start = y.iter().position(|v| *v <= low)?;
    let mut t_low = None;
    for k in start + 1..y.len() {
        if t_low.is_none() && y[k] > low && y[k - 1] <= low {
            t_low = Some(cross(x[k - 1], y[k - 1], x[k], y[k], low));
        }
        if let Some(t0) = t_low {
            if y[k] < low {
                // Fell back before reaching the top: start over.
                t_low = None;
                continue;
            }
            if y[k] >= high {
                return Some(cross(x[k - 1], y[k - 1], x[k], y[k], high) - t0);
            }
        }
    }
    None
}

/// Measurements of an AC magnitude in dB.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AcMeasures {
    pub peak_db: f64,
    pub peak_frequency: f64,
    /// Where the magnitude is 3 dB under the peak: below it and above it.
    pub minus_3db: (Option<f64>, Option<f64>),
}

/// Peak and −3 dB points of `db` over frequencies `f` between `lo` and `hi`
/// (interpolated in log frequency).
pub fn measure_ac(f: &[f64], db: &[f64], lo: f64, hi: f64) -> Option<AcMeasures> {
    let (x, y) = window(f, db, lo, hi);
    let (k, peak) = y.iter().copied().enumerate().fold(None, |best: Option<(usize, f64)>, (k, v)| match best {
        Some((_, b)) if b >= v => best,
        _ => Some((k, v)),
    })?;
    let level = peak - 3.0;
    let at = |j: usize| {
        let (a, b) = (x[j].max(1e-300).ln(), x[j + 1].max(1e-300).ln());
        cross(a, y[j], b, y[j + 1], level).exp()
    };
    let above = (k..y.len().saturating_sub(1)).find(|&j| y[j] >= level && y[j + 1] < level).map(at);
    let below = (0..k).rev().find(|&j| y[j] < level && y[j + 1] >= level).map(at);
    Some(AcMeasures { peak_db: peak, peak_frequency: x[k], minus_3db: (below, above) })
}

/// The phase margin of a loop gain: 180° plus its phase where its magnitude crosses
/// 0 dB downwards (the first crossing in the range), wrapped to (−180°, 180°].
pub fn phase_margin(f: &[f64], db: &[f64], phase: &[f64], lo: f64, hi: f64) -> Option<f64> {
    let (x, y) = window(f, db, lo, hi);
    let j = (0..y.len().saturating_sub(1)).find(|&j| y[j] >= 0.0 && y[j + 1] < 0.0)?;
    let (a, b) = (x[j].max(1e-300).ln(), x[j + 1].max(1e-300).ln());
    let fc = cross(a, y[j], b, y[j + 1], 0.0).exp();
    let p = crate::plot::interpolate(f, phase, fc)?;
    let mut m = (180.0 + p) % 360.0;
    if m <= -180.0 {
        m += 360.0;
    } else if m > 180.0 {
        m -= 360.0;
    }
    Some(m)
}

/// The Measure table of a plot over `lo..hi`: its column names and, per trace, its
/// cells ("—" where a measurement doesn't apply or wasn't found). Time and DC sweeps:
/// min, max, average, RMS, peak-to-peak, frequency, rise and fall; AC: the peak, its
/// frequency, the −3 dB points and (phase traces) the phase margin.
pub fn table(data: &crate::plot::PlotData, lo: f64, hi: f64) -> (Vec<&'static str>, Vec<Vec<String>>) {
    use crate::plot::label;
    let none = || "—".to_string();
    let opt = |v: Option<f64>, unit: &str| v.map_or_else(none, |v| label(v, unit));
    if data.log_x && data.traces.iter().any(|t| t.unit == "dB") {
        let cols = vec!["Peak", "At", "−3 dB", "Phase margin"];
        let rows = data
            .traces
            .iter()
            .map(|t| {
                if t.unit == "°" {
                    let mag = data.traces.iter().find(|m| m.signal == t.signal && m.unit == "dB");
                    let pm = mag.and_then(|m| phase_margin(&data.x, &m.y, &t.y, lo, hi));
                    return vec![none(), none(), none(), pm.map_or_else(none, |v| format!("{v:.1}°"))];
                }
                match measure_ac(&data.x, &t.y, lo, hi) {
                    Some(m) => {
                        let corners = match m.minus_3db {
                            (Some(a), Some(b)) => format!("{} … {}", label(a, "Hz"), label(b, "Hz")),
                            (a, b) => opt(a.or(b), "Hz"),
                        };
                        vec![label(m.peak_db, "dB"), label(m.peak_frequency, "Hz"), corners, none()]
                    }
                    None => vec![none(); 4],
                }
            })
            .collect();
        return (cols, rows);
    }
    let cols = vec!["Min", "Max", "Average", "RMS", "Peak-peak", "Frequency", "Rise", "Fall"];
    let rows = data
        .traces
        .iter()
        .map(|t| match measure(&data.x, &t.y, lo, hi) {
            Some(m) => vec![
                label(m.min, t.unit),
                label(m.max, t.unit),
                label(m.average, t.unit),
                label(m.rms, t.unit),
                label(m.peak_to_peak, t.unit),
                opt(m.frequency, "Hz"),
                opt(m.rise, data.x_unit),
                opt(m.fall, data.x_unit),
            ],
            None => vec![none(); 8],
        })
        .collect();
    (cols, rows)
}
