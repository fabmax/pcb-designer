//! Results as text: a summary table and CSV (the `simulate` command line, agents).

use model::{natural_cmp, si};

use crate::SimResult;
use crate::raw::Vector;

/// The signals of a result: every net's voltage (named nets first) and the probed
/// currents, by display name (`V(/OUT)`, `I(R1.1)`).
fn signals(r: &SimResult) -> Vec<(String, Vector)> {
    let mut nets: Vec<(&String, &String)> = r.netlist.nodes.iter().filter(|(_, node)| *node != "0").collect();
    nets.sort_by(|a, b| natural_cmp(a.0, b.0));
    let mut out: Vec<(String, Vector)> =
        nets.into_iter().filter_map(|(net, _)| Some((format!("V({net})"), r.voltage(net)?.into_owned()))).collect();
    let mut currents: Vec<(String, Vector)> = r
        .netlist
        .sense
        .values()
        .filter_map(|source| {
            let v = r.plot.vector(&format!("i({source})"))?;
            let name = source.trim_start_matches("vsense_").to_ascii_uppercase();
            Some((format!("I({name})"), v.clone()))
        })
        .collect();
    currents.sort_by(|a, b| natural_cmp(&a.0, &b.0));
    out.extend(currents);
    out
}

fn unit(name: &str) -> &'static str {
    if name.starts_with('I') { "A" } else { "V" }
}

/// A readable summary: the value of each signal for an operating point; else its range
/// and final value (magnitude in dB and phase at the end for AC).
pub fn text(r: &SimResult) -> String {
    let mut out = String::new();
    let signals = signals(r);
    let width = signals.iter().map(|(n, _)| n.chars().count()).max().unwrap_or(0).max(6);
    match r.scale() {
        None => {
            out.push_str(&format!("{}\n", r.plot.name));
            for (name, v) in &signals {
                let value = v.re.first().copied().unwrap_or(f64::NAN);
                out.push_str(&format!("{name:width$}  {}\n", si::format(value, unit(name))));
            }
        }
        Some(scale) => {
            let su = scale_unit(scale);
            let (first, last) = (scale.re.first().copied().unwrap_or(0.0), scale.re.last().copied().unwrap_or(0.0));
            out.push_str(&format!(
                "{}: {} points, {} … {}\n",
                r.plot.name,
                scale.re.len(),
                si::format(first, su),
                si::format(last, su)
            ));
            if r.plot.complex {
                out.push_str(&format!("{:width$}  {:>10}  {:>10}  {:>10}\n", "", "max dB", "end dB", "end phase"));
                for (name, v) in &signals {
                    let db = v.db();
                    let max = db.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
                    let end = db.last().copied().unwrap_or(f64::NAN);
                    let phase = v.phase().last().copied().unwrap_or(f64::NAN);
                    out.push_str(&format!("{name:width$}  {max:>10.2}  {end:>10.2}  {phase:>9.1}°\n"));
                }
            } else {
                out.push_str(&format!("{:width$}  {:>10}  {:>10}  {:>10}\n", "", "min", "max", "end"));
                for (name, v) in &signals {
                    let min = v.re.iter().cloned().fold(f64::INFINITY, f64::min);
                    let max = v.re.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
                    let end = v.re.last().copied().unwrap_or(f64::NAN);
                    let f = |x| si::format(x, unit(name));
                    out.push_str(&format!("{name:width$}  {:>10}  {:>10}  {:>10}\n", f(min), f(max), f(end)));
                }
            }
        }
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
    for (name, v) in &signals {
        if r.plot.complex {
            header.push(format!("{name} (dB)"));
            columns.push(v.db());
            header.push(format!("{name} (°)"));
            columns.push(v.phase());
        } else {
            header.push(format!("{name} ({})", unit(name)));
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
