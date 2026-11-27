//! The logic of the waveform plot, independent of the GUI: which signals a plot shows
//! and their traces (dB and unwrapped phase for AC), axis scales (linear or log) with
//! zoom and pan, nice tick values, min/max decimation per pixel column, snapping to
//! samples and fitting y to the visible part.

use std::collections::{HashMap, HashSet};

use editor_core::connectivity::Connectivity;
use model::{Analysis, Id, Probe, natural_cmp, si};

use crate::SimResult;

/// Traces shown without probes: the named nets, at most this many.
pub const AUTO_TRACES: usize = 8;

/// A signal a plot can show.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Signal {
    Voltage(String),
    /// The voltage of the first net to the second.
    Diff(String, String),
    /// Into a pin: (symbol, pin).
    Current(Id, Id),
    /// The power a component takes.
    Power(Id),
}

impl Signal {
    /// A probe's signal: a voltage probe's net is the one of its object (`at`) if that
    /// is still on a net, else the net of its name (the reference's likewise).
    pub fn of_probe(p: &Probe, conn: &Connectivity) -> Signal {
        let net = |name: &String, at: &Option<model::ObjectId>| {
            at.and_then(|o| conn.net_of(o)).map_or_else(|| name.clone(), |n| n.name.clone())
        };
        match p {
            Probe::Voltage { net: name, at, reference: None, .. } => Signal::Voltage(net(name, at)),
            Probe::Voltage { net: name, at, reference: Some(r), .. } => Signal::Diff(net(name, at), net(&r.net, &r.at)),
            Probe::Current { symbol, pin } => Signal::Current(*symbol, *pin),
            Probe::Power { component } => Signal::Power(*component),
        }
    }

    pub fn probe(&self) -> Probe {
        match self {
            Signal::Voltage(net) => Probe::voltage(net.clone(), None),
            Signal::Diff(net, reference) => Probe::Voltage {
                net: net.clone(),
                at: None,
                reference: Some(model::NetRef { net: reference.clone(), at: None }),
                logic: false,
            },
            Signal::Current(symbol, pin) => Probe::Current { symbol: *symbol, pin: *pin },
            Signal::Power(component) => Probe::Power { component: *component },
        }
    }

    /// Needs currents measured in a sweep (sense sources in the netlist).
    pub fn needs_sensing(&self) -> bool {
        matches!(self, Signal::Current(..) | Signal::Power(_))
    }

    /// The unit of its values (not AC).
    pub fn unit(&self) -> &'static str {
        match self {
            Signal::Voltage(_) | Signal::Diff(..) => "V",
            Signal::Current(..) => "A",
            Signal::Power(_) => "W",
        }
    }
}

/// A signal's label in a result: `V(OUT)`, `V(OUT,IN)`, `I(R1.1)`, `P(R1)`.
pub fn signal_label(result: &SimResult, s: &Signal) -> String {
    match s {
        Signal::Voltage(net) => format!("V({net})"),
        Signal::Diff(a, b) => format!("V({a},{b})"),
        Signal::Current(symbol, pin) => result.netlist.current_label(*symbol, *pin).unwrap_or_else(|| "I(?)".into()),
        Signal::Power(c) => match result.netlist.pins.iter().find(|p| p.component == *c) {
            Some(p) => format!("P({})", p.refdes),
            None => "P(?)".into(),
        },
    }
}

/// A signal's values in a result (complex for AC).
pub fn vector<'a>(result: &'a SimResult, s: &Signal) -> Option<std::borrow::Cow<'a, crate::Vector>> {
    use std::borrow::Cow;
    match s {
        Signal::Voltage(net) => result.voltage(net),
        Signal::Diff(a, b) => result.voltage_between(a, b).map(Cow::Owned),
        Signal::Current(symbol, pin) => result.current(*symbol, *pin).map(Cow::Borrowed),
        Signal::Power(c) => result.power(*c).map(Cow::Owned),
    }
}

/// What a plot shows: the analysis' probes, or without any the named nets of the result
/// (at most [`AUTO_TRACES`], natural order). The second value: how many named nets
/// there are (0 when probes were given).
pub fn signals(analysis: &Analysis, result: &SimResult, conn: &Connectivity) -> (Vec<Signal>, usize) {
    if !analysis.probes.is_empty() {
        return (analysis.probes.iter().map(|p| Signal::of_probe(p, conn)).collect(), 0);
    }
    let mut named: Vec<&String> = result
        .netlist
        .nodes
        .iter()
        .filter(|(net, node)| *node != "0" && conn.net(net).is_some_and(|n| n.named))
        .map(|(net, _)| net)
        .collect();
    named.sort_by(|a, b| natural_cmp(a, b));
    let total = named.len();
    (named.into_iter().take(AUTO_TRACES).map(|n| Signal::Voltage(n.clone())).collect(), total)
}

/// Which y axis a trace goes on: left, right, or a second right axis further out
/// (a third unit).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Axis {
    Left,
    Right,
    Right2,
}

impl Axis {
    pub const ALL: [Axis; 3] = [Axis::Left, Axis::Right, Axis::Right2];

    pub fn index(self) -> usize {
        self as usize
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Trace {
    /// `V(OUT)`, `I(R1.1)`; AC phase traces `V(OUT) phase`.
    pub label: String,
    pub unit: &'static str,
    pub axis: Axis,
    /// Phase traces are dashed.
    pub dashed: bool,
    /// Index of the signal (its colour).
    pub signal: usize,
    /// Index of a parameter sweep's step (0 without one).
    pub step: usize,
    pub y: Vec<f64>,
    /// Shown as a logic level in a lane under the plot, off the axes: its threshold
    /// ([`set_logic`], [`logic_threshold`]).
    pub logic: Option<f64>,
}

/// A plot's data: the scale and the traces.
#[derive(Clone, Debug, PartialEq)]
pub struct PlotData {
    /// "Time", "Frequency", or the swept source's quantity.
    pub x_label: String,
    pub x_unit: &'static str,
    /// Log x by default (AC).
    pub log_x: bool,
    pub x: Vec<f64>,
    pub traces: Vec<Trace>,
    /// Signals that have no data (net gone, pin not probed in this run).
    pub missing: Vec<Missing>,
    /// A parameter sweep's steps ("R1 = 4.7 kΩ"); empty without one.
    pub steps: Vec<String>,
}

/// A signal without data, and why.
#[derive(Clone, Debug, PartialEq)]
pub struct Missing {
    pub label: String,
    pub reason: String,
    /// Index of the signal (the probe).
    pub signal: usize,
}

impl PlotData {
    /// Orders the traces by their signal's name (natural: `Q2` before `Q10`), whatever
    /// order they were probed in: a signal's phase after its magnitude, its steps in
    /// order. Each keeps `Trace::signal`, so its colour.
    pub fn sort_by_name(&mut self) {
        let mut names: Vec<(usize, &str)> =
            self.traces.iter().filter(|t| !t.dashed && t.step == 0).map(|t| (t.signal, t.label.as_str())).collect();
        names.sort_by(|a, b| natural_cmp(a.1, b.1));
        let rank: HashMap<usize, usize> = names.iter().enumerate().map(|(k, (signal, _))| (*signal, k)).collect();
        self.traces.sort_by_key(|t| (rank.get(&t.signal).copied().unwrap_or(usize::MAX), t.dashed, t.step));
        self.missing.sort_by(|a, b| natural_cmp(&a.label, &b.label));
    }

    /// The units on each axis (empty: the axis is unused; logic traces use none).
    pub fn axis_unit(&self, axis: Axis) -> &'static str {
        self.traces.iter().find(|t| t.axis == axis && t.logic.is_none()).map_or("", |t| t.unit)
    }

    /// The palette index of a trace and how far to fade it (0: not at all): with one
    /// signal each step gets its own colour, with several each signal keeps its colour
    /// and its steps fade from full to lighter.
    pub fn color(&self, t: &Trace) -> (usize, f32) {
        let n = self.steps.len();
        if n <= 1 {
            return (t.signal, 0.0);
        }
        let signals = self.traces.iter().map(|t| t.signal).max().unwrap_or(0) + 1;
        if signals == 1 { (t.step, 0.0) } else { (t.signal, 0.6 * t.step as f32 / (n - 1) as f32) }
    }
}

/// Most x samples of a sweep's merged plot (more: the first step's x).
const MAX_MERGED: usize = 400_000;

/// The plot of a parameter sweep: [`data`] of each step's result, put onto one x axis
/// (the union of the steps' samples; transients step differently), a trace per signal
/// and step labelled "V(OUT) · R1 = 4.7 kΩ". `steps`: each step's label and result.
pub fn data_steps(
    steps: &[(String, &SimResult)],
    signals: &[Signal],
    phase: bool,
    conn: &Connectivity,
) -> Option<PlotData> {
    let [(_, first), ..] = steps else { return None };
    if steps.len() == 1 {
        return data(first, signals, phase, conn);
    }
    let all: Vec<PlotData> = steps.iter().map(|(_, r)| data(r, signals, phase, conn)).collect::<Option<_>>()?;
    let mut x: Vec<f64> = all.iter().flat_map(|d| d.x.iter().copied()).collect();
    x.sort_by(f64::total_cmp);
    x.dedup();
    if x.len() > MAX_MERGED {
        x = all[0].x.clone();
    }
    let mut traces = Vec::new();
    // By signal (and its phase), then step.
    let firsts = &all[0].traces;
    for (i, proto) in firsts.iter().enumerate() {
        for (k, d) in all.iter().enumerate() {
            let Some(t) = d.traces.get(i) else { continue };
            let y = if d.x == x {
                t.y.clone()
            } else {
                x.iter().map(|v| interpolate(&d.x, &t.y, *v).unwrap_or(f64::NAN)).collect()
            };
            traces.push(Trace { label: format!("{} · {}", proto.label, steps[k].0), step: k, y, ..proto.clone() });
        }
    }
    let first = &all[0];
    Some(PlotData {
        x_label: first.x_label.clone(),
        x_unit: first.x_unit,
        log_x: first.log_x,
        x,
        traces,
        missing: first.missing.clone(),
        steps: steps.iter().map(|(l, _)| l.clone()).collect(),
    })
}

/// The plot of `signals` in `result` (`None` for an operating point), its traces in their
/// order ([`PlotData::sort_by_name`] for a plot's). AC: magnitude in
/// dB on the left, with `phase` the unwrapped phase in degrees on the right (dashed).
/// Else one axis per unit in the order volts, amperes, watts: the first present on the
/// left, the next on the right, a third on a second right axis.
pub fn data(result: &SimResult, signals: &[Signal], phase: bool, conn: &Connectivity) -> Option<PlotData> {
    let scale = result.scale()?;
    let (x_label, x_unit) = match scale.kind.as_str() {
        "time" => ("Time".to_string(), "s"),
        "frequency" => ("Frequency".to_string(), "Hz"),
        "current" => (format!("Source {}", scale.name.to_uppercase()), "A"),
        _ => (format!("Source {}", scale.name.to_uppercase()), "V"),
    };
    let ac = result.plot.complex;
    let units: Vec<&str> = ["V", "A", "W"].into_iter().filter(|u| signals.iter().any(|s| s.unit() == *u)).collect();
    let mut traces = Vec::new();
    let mut missing = Vec::new();
    for (i, s) in signals.iter().enumerate() {
        let label = signal_label(result, s);
        let Some(v) = vector(result, s) else {
            missing.push(Missing { label, reason: missing_reason(result, s, conn), signal: i });
            continue;
        };
        if ac {
            traces.push(Trace {
                label: label.clone(),
                unit: "dB",
                axis: Axis::Left,
                dashed: false,
                signal: i,
                step: 0,
                y: v.db(),
                logic: None,
            });
            if phase {
                let y = unwrap_phase(&v.phase());
                traces.push(Trace {
                    label: format!("{label} phase"),
                    unit: "°",
                    axis: Axis::Right,
                    dashed: true,
                    signal: i,
                    step: 0,
                    y,
                    logic: None,
                });
            }
        } else {
            let unit = s.unit();
            let axis = Axis::ALL[units.iter().position(|u| *u == unit).unwrap_or(0)];
            traces.push(Trace { label, unit, axis, dashed: false, signal: i, step: 0, y: v.re.clone(), logic: None });
        }
    }
    Some(PlotData { x_label, x_unit, log_x: ac, x: scale.re.clone(), traces, missing, steps: Vec::new() })
}

/// Marks the traces of the signals `logic` says (by signal index) as logic traces:
/// voltages to ground only, not in AC.
pub fn set_logic(data: &mut PlotData, signals: &[Signal], logic: &[bool]) {
    // AC and spectra: levels make no sense in frequency.
    if data.x_unit == "Hz" {
        return;
    }
    for t in &mut data.traces {
        let on =
            logic.get(t.signal).copied().unwrap_or(false) && matches!(signals.get(t.signal), Some(Signal::Voltage(_)));
        t.logic = on.then(|| logic_threshold(&t.y));
    }
}

/// Which signals of `analysis`' probes show as logic (by signal index, like
/// [`signals`]; none without probes).
pub fn logic_flags(analysis: &Analysis) -> Vec<bool> {
    analysis.probes.iter().map(|p| matches!(p, Probe::Voltage { logic: true, reference: None, .. })).collect()
}

/// The voltage a logic trace switches at: halfway between its lowest and highest
/// value over the run (it works for any supply). A trace that barely moves is high
/// above half of its value, at least 0.5 V (a steady 0 V is low, a steady 3.3 V high).
pub fn logic_threshold(y: &[f64]) -> f64 {
    let (lo, hi) =
        y.iter().filter(|v| v.is_finite()).fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), &v| (a.min(v), b.max(v)));
    if !lo.is_finite() {
        return 0.5;
    }
    if hi - lo >= 0.5 { (lo + hi) / 2.0 } else { (hi / 2.0).max(0.5) }
}

/// A logic trace between `lo` and `hi` as runs of one level: (from, to, high), the
/// switching points interpolated where the trace crosses `threshold`.
pub fn logic_runs(x: &[f64], y: &[f64], lo: f64, hi: f64, threshold: f64) -> Vec<(f64, f64, bool)> {
    let r = visible(x, lo, hi);
    let mut runs: Vec<(f64, f64, bool)> = Vec::new();
    // Include the samples just outside, so runs reach the edges.
    let (a, b) = (r.start.saturating_sub(1), (r.end + 1).min(x.len()));
    for i in a..b {
        let high = y[i] > threshold;
        match runs.last_mut() {
            Some(last) if last.2 == high => last.1 = x[i],
            Some(last) => {
                // Where it crossed, between the previous sample and this one.
                let (x0, y0) = (x[i - 1], y[i - 1]);
                let t = if y[i] != y0 { ((threshold - y0) / (y[i] - y0)).clamp(0.0, 1.0) } else { 1.0 };
                let at = x0 + t * (x[i] - x0);
                last.1 = at;
                runs.push((at, x[i], high));
            }
            None => runs.push((x[i], x[i], high)),
        }
    }
    for run in &mut runs {
        run.0 = run.0.max(lo);
        run.1 = run.1.min(hi);
    }
    runs.retain(|r| r.1 >= r.0);
    runs
}

/// Why a signal has no data in `result`.
pub fn missing_reason(result: &SimResult, s: &Signal, conn: &Connectivity) -> String {
    let net_reason = |net: &str| {
        if conn.net(net).is_none() { "the net is gone" } else { "not in this result: run again" }
    };
    let nl = &result.netlist;
    match s {
        Signal::Voltage(net) => net_reason(net).into(),
        Signal::Diff(a, b) => {
            if conn.net(a).is_none() || conn.net(b).is_none() {
                net_reason(if conn.net(a).is_none() { a } else { b }).into()
            } else {
                "not in this result: run again".into()
            }
        }
        Signal::Current(symbol, pin) => {
            let simulated = nl.pins.iter().any(|p| p.symbol == Some(*symbol));
            if !simulated {
                "the part was not simulated".into()
            } else if !nl.pins.iter().any(|p| p.symbol == Some(*symbol) && p.pin == *pin) {
                "the pin is not part of the model".into()
            } else {
                "not measured in this result: run again".into()
            }
        }
        Signal::Power(_) if result.plot.complex => "power isn't shown for AC".into(),
        Signal::Power(c) if !nl.pins.iter().any(|p| p.component == *c) => "the part was not simulated".into(),
        Signal::Power(_) => "not measured in this result: run again".into(),
    }
}

/// Phase in degrees without the jumps at ±180°.
pub fn unwrap_phase(deg: &[f64]) -> Vec<f64> {
    let mut out = Vec::with_capacity(deg.len());
    let mut offset = 0.0;
    let mut last: Option<f64> = None;
    for &d in deg {
        if let Some(prev) = last {
            let mut step = d + offset - prev;
            while step > 180.0 {
                offset -= 360.0;
                step -= 360.0;
            }
            while step < -180.0 {
                offset += 360.0;
                step += 360.0;
            }
        }
        let v = d + offset;
        out.push(v);
        last = Some(v);
    }
    out
}

/// An axis' visible range, linear or logarithmic.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Scale {
    pub lo: f64,
    pub hi: f64,
    pub log: bool,
}

impl Scale {
    pub fn new(lo: f64, hi: f64, log: bool) -> Scale {
        let mut s = Scale { lo, hi, log };
        s.sanitize();
        s
    }

    fn t(&self, v: f64) -> f64 {
        if self.log { v.max(f64::MIN_POSITIVE).log10() } else { v }
    }

    fn inv(&self, t: f64) -> f64 {
        if self.log { 10f64.powf(t) } else { t }
    }

    /// 0 at `lo`, 1 at `hi`.
    pub fn frac(&self, v: f64) -> f64 {
        let (a, b) = (self.t(self.lo), self.t(self.hi));
        (self.t(v) - a) / (b - a)
    }

    pub fn value(&self, frac: f64) -> f64 {
        let (a, b) = (self.t(self.lo), self.t(self.hi));
        self.inv(a + frac * (b - a))
    }

    /// Zooms by `factor` (> 1 zooms in) keeping the value at `frac` in place.
    pub fn zoom(&mut self, frac: f64, factor: f64) {
        let (a, b) = (self.t(self.lo), self.t(self.hi));
        let at = a + frac * (b - a);
        let (a, b) = (at - (at - a) / factor, at + (b - at) / factor);
        (self.lo, self.hi) = (self.inv(a), self.inv(b));
        self.sanitize();
    }

    /// Moves the range by `dfrac` of its width.
    pub fn pan(&mut self, dfrac: f64) {
        let (a, b) = (self.t(self.lo), self.t(self.hi));
        let d = dfrac * (b - a);
        (self.lo, self.hi) = (self.inv(a + d), self.inv(b + d));
    }

    /// A range of at least some width (a flat trace still gets an axis); positive for log.
    fn sanitize(&mut self) {
        if self.log {
            if self.lo <= 0.0 {
                self.lo = if self.hi > 0.0 { self.hi * 1e-6 } else { 1.0 };
            }
            if self.hi <= self.lo * 1.000_001 {
                self.hi = self.lo * 10.0;
            }
        } else {
            let span = self.hi - self.lo;
            let size = self.lo.abs().max(self.hi.abs());
            if !(span > size * 1e-9 && span > 1e-30) {
                let pad = if size > 0.0 { size * 0.1 } else { 1.0 };
                (self.lo, self.hi) = (self.lo - pad, self.hi + pad);
            }
        }
    }

    /// Tick values inside the range, about `count` of them (1/2/5 steps; decades for
    /// log, with 2 and 5 when there are few).
    pub fn ticks(&self, count: usize) -> Vec<f64> {
        if self.log { log_ticks(self.lo, self.hi, count) } else { linear_ticks(self.lo, self.hi, count) }
    }
}

/// Values of 1, 2 or 5 × 10ⁿ steps between `lo` and `hi`, about `count` of them.
pub fn linear_ticks(lo: f64, hi: f64, count: usize) -> Vec<f64> {
    let span = hi - lo;
    if span.is_nan() || span <= 0.0 || !span.is_finite() {
        return Vec::new();
    }
    let raw = span / count.max(1) as f64;
    let exp = raw.log10().floor() as i32;
    let mag = 10f64.powi(exp);
    let m = [1.0, 2.0, 5.0, 10.0].into_iter().find(|m| m * mag >= raw).unwrap_or(10.0);
    let step = m * mag;
    // i · m · 10^exp, divided rather than multiplied for negative exponents, so 3 · 0.2
    // is 0.6 and not 0.6000000000000001.
    let value = |i: i64| {
        let n = i as f64 * m;
        if exp < 0 { n / 10f64.powi(-exp) } else { n * mag }
    };
    // Ends that are ticks themselves (0.7 / 0.1 = 6.999…) count.
    let first = (lo / step - 1e-9).ceil() as i64;
    let last = (hi / step + 1e-9).floor() as i64;
    (first..=last).map(value).collect()
}

fn log_ticks(lo: f64, hi: f64, count: usize) -> Vec<f64> {
    let (a, b) = (lo.log10().floor() as i32, hi.log10().ceil() as i32);
    let decades = (b - a).max(1) as usize;
    let mantissas: &[f64] = if decades * 3 <= count {
        &[1.0, 2.0, 5.0]
    } else if decades <= count {
        &[1.0]
    } else {
        // Every n-th decade.
        let n = decades.div_ceil(count.max(1)) as i32;
        return (a..=b).filter(|e| e % n == 0).map(|e| 10f64.powi(e)).filter(|v| *v >= lo && *v <= hi).collect();
    };
    (a..=b)
        .flat_map(|e| mantissas.iter().map(move |m| m * 10f64.powi(e)))
        .filter(|v| *v >= lo * (1.0 - 1e-9) && *v <= hi * (1.0 + 1e-9))
        .collect()
}

/// A tick label: SI prefixed for V, A, s, Hz; plain for dB and degrees.
/// A value as read out: volts under 1 µV are 0 V (the solver's noise around a level is
/// femtovolts), and −0 is 0.
pub fn reading(value: f64, unit: &str) -> f64 {
    if value == 0.0 || (unit == "V" && value.abs() < 0.5e-6) { 0.0 } else { value }
}

/// [`label`] of a value read out ([`reading`]): hover, readout, result tables.
pub fn value_label(value: f64, unit: &str) -> String {
    label(reading(value, unit), unit)
}

pub fn label(value: f64, unit: &str) -> String {
    match unit {
        "dB" | "°" => {
            // Rounded first, so -0.001 isn't "-0".
            let v = (value * 100.0).round() / 100.0;
            let v = if v == 0.0 { 0.0 } else { v };
            if v <= NO_SIGNAL_DB {
                return "−∞ dB".into();
            }
            let s = format!("{v:.2}");
            let s = s.trim_end_matches('0').trim_end_matches('.');
            format!("{s}{}", if unit == "°" { "°" } else { " dB" })
        }
        _ => si::format(value, unit),
    }
}

/// The index of the sample nearest to `x` (`xs` ascending).
pub fn nearest(xs: &[f64], x: f64) -> Option<usize> {
    if xs.is_empty() {
        return None;
    }
    let i = xs.partition_point(|&v| v < x);
    if i == 0 {
        return Some(0);
    }
    if i >= xs.len() {
        return Some(xs.len() - 1);
    }
    Some(if (x - xs[i - 1]) <= (xs[i] - x) { i - 1 } else { i })
}

/// `ys` at `x`, linearly interpolated (clamped to the ends).
pub fn interpolate(xs: &[f64], ys: &[f64], x: f64) -> Option<f64> {
    if xs.is_empty() || xs.len() != ys.len() {
        return None;
    }
    let i = xs.partition_point(|&v| v < x);
    if i == 0 {
        return Some(ys[0]);
    }
    if i >= xs.len() {
        return ys.last().copied();
    }
    let (x0, x1) = (xs[i - 1], xs[i]);
    let t = if x1 > x0 { (x - x0) / (x1 - x0) } else { 0.0 };
    Some(ys[i - 1] + t * (ys[i] - ys[i - 1]))
}

/// The samples visible between `lo` and `hi`, with one more on each side (so lines
/// reach the edges): a range of indices.
pub fn visible(xs: &[f64], lo: f64, hi: f64) -> std::ops::Range<usize> {
    let a = xs.partition_point(|&v| v < lo).saturating_sub(1);
    let b = (xs.partition_point(|&v| v <= hi) + 1).min(xs.len());
    a..b.max(a)
}

/// The points to draw of a trace, as (x, y) values: every visible sample, or, when
/// there are more than two per pixel column, each column's first, minimum, maximum and
/// last sample in their order (so a dense signal draws as its envelope and a long
/// transient as a few thousand vertices). `column` maps x to a pixel column.
pub fn decimate(
    xs: &[f64],
    ys: &[f64],
    lo: f64,
    hi: f64,
    columns: usize,
    column: impl Fn(f64) -> i64,
) -> Vec<[f64; 2]> {
    let r = visible(xs, lo, hi);
    if r.len() <= 2 * columns.max(1) {
        return r.map(|i| [xs[i], ys[i]]).collect();
    }
    let mut out = Vec::with_capacity(4 * columns + 4);
    let mut i = r.start;
    while i < r.end {
        let col = column(xs[i]);
        let start = i;
        let (mut min, mut max) = (i, i);
        while i < r.end && column(xs[i]) == col {
            if ys[i] < ys[min] {
                min = i;
            }
            if ys[i] > ys[max] {
                max = i;
            }
            i += 1;
        }
        let last = i - 1;
        let mut picks = [start, min, max, last];
        picks.sort_unstable();
        let mut prev = usize::MAX;
        for p in picks {
            if p != prev {
                out.push([xs[p], ys[p]]);
                prev = p;
            }
        }
    }
    out
}

/// A magnitude at or below this is no signal (a DC net in an AC analysis: 0 is −6000
/// dB, from `Vector::db`'s floor): "−∞ dB", left out of the y fit.
pub const NO_SIGNAL_DB: f64 = -300.0;

/// The y range of the traces on `axis` between `lo` and `hi` in x, with 5 % margin.
/// `hidden` traces (by label) don't count, nor dB values of no signal.
pub fn fit_y(data: &PlotData, axis: Axis, lo: f64, hi: f64, hidden: &HashSet<String>) -> Option<(f64, f64)> {
    let r = visible(&data.x, lo, hi);
    let mut range: Option<(f64, f64)> = None;
    for t in data.traces.iter().filter(|t| t.axis == axis && t.logic.is_none() && !hidden.contains(&t.label)) {
        let db = t.unit == "dB";
        for &y in &t.y[r.clone()] {
            if y.is_finite() && !(db && y <= NO_SIGNAL_DB) {
                range = Some(match range {
                    None => (y, y),
                    Some((a, b)) => (a.min(y), b.max(y)),
                });
            }
        }
    }
    let (a, b) = range?;
    let pad = (b - a) * 0.05;
    Some((a - pad, b + pad))
}

/// The CSV of the traces not hidden, over the whole run: the scale, then one column
/// per trace.
pub fn csv(data: &PlotData, hidden: &HashSet<String>) -> String {
    let traces: Vec<&Trace> = data.traces.iter().filter(|t| !hidden.contains(&t.label)).collect();
    let quote = |s: &str| if s.contains(',') { format!("\"{s}\"") } else { s.to_string() };
    let mut out = quote(&format!("{} ({})", data.x_label, data.x_unit));
    for t in &traces {
        out.push(',');
        out.push_str(&quote(&format!("{} ({})", t.label, t.unit)));
    }
    out.push('\n');
    for (i, x) in data.x.iter().enumerate() {
        out.push_str(&x.to_string());
        for t in &traces {
            out.push(',');
            out.push_str(&t.y[i].to_string());
        }
        out.push('\n');
    }
    out
}
