//! Results as text: a summary table and CSV (the `simulate` command line, agents).

use model::{natural_cmp, si};

use crate::SimResult;
use crate::raw::Vector;

/// A signal of a result: a net's voltage (`net` set) or a probed current.
#[derive(Clone, Debug)]
pub struct Signal {
    /// `V(/OUT)`, `I(R1_1)`.
    pub label: String,
    pub net: Option<String>,
    pub unit: &'static str,
    pub vector: Vector,
}

/// The signals of a result: every net's voltage (in natural order) and the probed
/// currents.
pub fn signals(r: &SimResult) -> Vec<Signal> {
    let mut nets: Vec<(&String, &String)> = r.netlist.nodes.iter().filter(|(_, node)| *node != "0").collect();
    nets.sort_by(|a, b| natural_cmp(a.0, b.0));
    let mut out: Vec<Signal> = nets
        .into_iter()
        .filter_map(|(net, _)| {
            let vector = r.voltage(net)?.into_owned();
            Some(Signal { label: format!("V({net})"), net: Some(net.clone()), unit: "V", vector })
        })
        .collect();
    let mut currents: Vec<Signal> = r
        .netlist
        .sense
        .values()
        .filter_map(|source| {
            let vector = r.plot.vector(&format!("i({source})"))?.clone();
            let name = source.trim_start_matches("vsense_").to_ascii_uppercase();
            Some(Signal { label: format!("I({name})"), net: None, unit: "A", vector })
        })
        .collect();
    currents.sort_by(|a, b| natural_cmp(&a.label, &b.label));
    out.extend(currents);
    out
}

/// A line of the summary table.
#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    pub label: String,
    pub net: Option<String>,
    /// Formatted, one per [`columns`] entry.
    pub values: Vec<String>,
}

/// The summary's value columns: the value of an operating point; min, max and end of a
/// sweep; for AC the maximum and final magnitude in dB and the final phase.
pub fn columns(r: &SimResult) -> &'static [&'static str] {
    match (r.scale(), r.plot.complex) {
        (None, _) => &["Value"],
        (Some(_), true) => &["Max dB", "End dB", "End phase"],
        (Some(_), false) => &["Min", "Max", "End"],
    }
}

/// The summary table, one row per signal (see [`columns`]).
pub fn rows(r: &SimResult) -> Vec<Row> {
    let op = r.scale().is_none();
    signals(r)
        .into_iter()
        .map(|s| {
            let v = &s.vector;
            let f = |x: f64| si::format(x, s.unit);
            let values = if op {
                vec![f(v.re.first().copied().unwrap_or(f64::NAN))]
            } else if r.plot.complex {
                let db = v.db();
                let max = db.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
                let end = db.last().copied().unwrap_or(f64::NAN);
                let phase = v.phase().last().copied().unwrap_or(f64::NAN);
                vec![format!("{max:.2}"), format!("{end:.2}"), format!("{phase:.1}°")]
            } else {
                let min = v.re.iter().cloned().fold(f64::INFINITY, f64::min);
                let max = v.re.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
                vec![f(min), f(max), f(v.re.last().copied().unwrap_or(f64::NAN))]
            };
            Row { label: s.label, net: s.net, values }
        })
        .collect()
}

/// The plot's name and, for sweeps, its points and range: "Transient Analysis: 126
/// points, 0 s … 1 ms".
pub fn title(r: &SimResult) -> String {
    match r.scale() {
        None => r.plot.name.clone(),
        Some(scale) => {
            let su = scale_unit(scale);
            let (first, last) = (scale.re.first().copied().unwrap_or(0.0), scale.re.last().copied().unwrap_or(0.0));
            format!("{}: {} points, {} … {}", r.plot.name, scale.re.len(), si::format(first, su), si::format(last, su))
        }
    }
}

/// A readable summary: [`title`] and the [`rows`] as a table.
pub fn text(r: &SimResult) -> String {
    let rows = rows(r);
    let width = rows.iter().map(|row| row.label.chars().count()).max().unwrap_or(0).max(6);
    let mut out = format!("{}\n", title(r));
    let cols = columns(r);
    if cols.len() > 1 {
        out.push_str(&format!("{:width$}", ""));
        for c in cols {
            out.push_str(&format!("  {c:>10}"));
        }
        out.push('\n');
    }
    for row in &rows {
        out.push_str(&format!("{:width$}", row.label));
        for v in &row.values {
            if cols.len() > 1 {
                out.push_str(&format!("  {v:>10}"));
            } else {
                out.push_str(&format!("  {v}"));
            }
        }
        out.push('\n');
    }
    out
}

fn scale_unit(scale: &Vector) -> &'static str {
    match scale.kind.as_str() {
        "time" => "s",
        "frequency" => "Hz",
        "current" => "A",
        _ => "V",
    }
}

/// Every signal at every point, comma separated, the scale first. AC: magnitude in dB
/// and phase in degrees per signal.
pub fn csv(r: &SimResult) -> String {
    let signals = signals(r);
    let mut header = Vec::new();
    let mut columns: Vec<Vec<f64>> = Vec::new();
    if let Some(s) = r.scale() {
        header.push(format!("{} ({})", s.name, scale_unit(s)));
        columns.push(s.re.clone());
    }
    for s in &signals {
        let (name, v) = (&s.label, &s.vector);
        if r.plot.complex {
            header.push(format!("{name} (dB)"));
            columns.push(v.db());
            header.push(format!("{name} (°)"));
            columns.push(v.phase());
        } else {
            header.push(format!("{name} ({})", s.unit));
            columns.push(v.re.clone());
        }
    }
    let quote = |s: &str| if s.contains(',') { format!("\"{s}\"") } else { s.to_string() };
    let mut out = header.iter().map(|h| quote(h)).collect::<Vec<_>>().join(",");
    out.push('\n');
    for i in 0..r.plot.len() {
        let row: Vec<String> = columns.iter().map(|c| format!("{}", c[i])).collect();
        out.push_str(&row.join(","));
        out.push('\n');
    }
    out
}
