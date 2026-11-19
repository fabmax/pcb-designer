//! SPICE raw files, as ngspice writes them with `-r` (binary, or ASCII with
//! `SPICE_ASCIIRAWFILE=1`): one or more plots, each a text header followed by the data.
//!
//! ```text
//! Title: …
//! Plotname: Transient Analysis
//! Flags: real            (or complex)
//! No. Variables: 3
//! No. Points: 101
//! Variables:
//!     0  time    time        (tab separated)
//!     1  v(out)  voltage
//!     2  i(v1)   current
//! Binary:                (or Values:)
//! ```
//!
//! Binary data is point-major little-endian `f64`s, two per value for complex plots.
//! Gotcha: `No. Points` may be larger than the data written (a run stopped early); the
//! reader takes the whole points present.

use crate::SimError;

/// One analysis result: its vectors, the first being the scale (time, frequency, the
/// swept value), except for an operating point, which has none.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Plot {
    /// "Transient Analysis", "AC Analysis", "Operating Point", "DC transfer characteristic".
    pub name: String,
    pub complex: bool,
    pub vectors: Vec<Vector>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Vector {
    /// As ngspice names it, lower case: `time`, `v(out)`, `i(v1)`.
    pub name: String,
    /// `time`, `frequency`, `voltage`, `current`, …
    pub kind: String,
    pub re: Vec<f64>,
    /// Imaginary parts (complex plots only).
    pub im: Option<Vec<f64>>,
}

impl Plot {
    pub fn vector(&self, name: &str) -> Option<&Vector> {
        self.vectors.iter().find(|v| v.name.eq_ignore_ascii_case(name))
    }

    /// The number of points.
    pub fn len(&self) -> usize {
        self.vectors.first().map_or(0, |v| v.re.len())
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// All plots of a raw file, in file order.
pub fn parse(data: &[u8]) -> Result<Vec<Plot>, SimError> {
    let mut plots = Vec::new();
    let mut at = 0;
    while at < data.len() {
        // Skip blank space between plots.
        if data[at].is_ascii_whitespace() {
            at += 1;
            continue;
        }
        // Bytes after the last whole point of a cut-off run: no further plot.
        if !data[at..].starts_with(b"Title:") && !plots.is_empty() {
            break;
        }
        let (plot, next) = parse_plot(data, at)?;
        plots.push(plot);
        at = next;
    }
    Ok(plots)
}

fn bad(message: impl Into<String>) -> SimError {
    SimError::Raw(message.into())
}

/// The line starting at `at` (without its end) and where the next one starts.
fn line(data: &[u8], at: usize) -> Option<(&str, usize)> {
    if at >= data.len() {
        return None;
    }
    let end = data[at..].iter().position(|&b| b == b'\n').map_or(data.len(), |p| at + p);
    let text = std::str::from_utf8(&data[at..end]).ok()?;
    Some((text.trim_end_matches('\r'), end + 1))
}

fn parse_plot(data: &[u8], mut at: usize) -> Result<(Plot, usize), SimError> {
    let mut plot = Plot::default();
    let (mut vars, mut points) = (None::<usize>, None::<usize>);
    loop {
        let (text, next) = line(data, at).ok_or_else(|| bad("header ends before the data"))?;
        at = next;
        let (key, value) = text.split_once(':').map_or((text, ""), |(k, v)| (k.trim(), v.trim()));
        match key.to_ascii_lowercase().as_str() {
            "plotname" => plot.name = value.to_string(),
            "flags" => plot.complex = value.split_whitespace().any(|f| f.eq_ignore_ascii_case("complex")),
            "no. variables" => vars = value.parse().ok(),
            "no. points" => points = value.parse().ok(),
            "variables" => {
                let n = vars.ok_or_else(|| bad("variables before their count"))?;
                // The first may follow on the same line in some writers.
                let mut lines = Vec::new();
                if !value.is_empty() {
                    lines.push(value.to_string());
                }
                while lines.len() < n {
                    let (text, next) = line(data, at).ok_or_else(|| bad("missing variables"))?;
                    at = next;
                    lines.push(text.to_string());
                }
                for l in lines {
                    let mut f = l.split_whitespace();
                    let (_index, name, kind) = (f.next(), f.next(), f.next());
                    let name = name.ok_or_else(|| bad(format!("bad variable line {l:?}")))?;
                    plot.vectors.push(Vector {
                        name: name.to_ascii_lowercase(),
                        kind: kind.unwrap_or("").to_string(),
                        re: Vec::new(),
                        im: plot.complex.then(Vec::new),
                    });
                }
            }
            "binary" => {
                let points = points.ok_or_else(|| bad("no point count"))?;
                at = read_binary(&mut plot, data, at, points)?;
                return Ok((plot, at));
            }
            "values" => {
                let points = points.ok_or_else(|| bad("no point count"))?;
                at = read_ascii(&mut plot, data, at, points)?;
                return Ok((plot, at));
            }
            _ => {} // Title, Date, Command, Option, …
        }
    }
}

fn read_binary(plot: &mut Plot, data: &[u8], at: usize, points: usize) -> Result<usize, SimError> {
    let per_value = if plot.complex { 16 } else { 8 };
    let per_point = per_value * plot.vectors.len();
    if per_point == 0 {
        return Ok(at);
    }
    let available = (data.len() - at) / per_point;
    let points = points.min(available);
    let f = |i: usize| f64::from_le_bytes(data[i..i + 8].try_into().unwrap());
    for p in 0..points {
        let base = at + p * per_point;
        for (k, v) in plot.vectors.iter_mut().enumerate() {
            let i = base + k * per_value;
            v.re.push(f(i));
            if let Some(im) = &mut v.im {
                im.push(f(i + 8));
            }
        }
    }
    Ok(at + points * per_point)
}

fn read_ascii(plot: &mut Plot, data: &[u8], mut at: usize, points: usize) -> Result<usize, SimError> {
    let n = plot.vectors.len();
    for _ in 0..points {
        for k in 0..n {
            let Some((text, next)) = line(data, at) else {
                return Ok(at);
            };
            if text.trim().is_empty() || text.contains(':') {
                return Ok(at);
            }
            at = next;
            // The first value of a point follows its index.
            let mut fields = text.split_whitespace();
            let value = if k == 0 { fields.nth(1) } else { fields.next() }
                .ok_or_else(|| bad(format!("bad value line {text:?}")))?;
            let (re, im) = value.split_once(',').unwrap_or((value, "0"));
            let num = |s: &str| s.parse::<f64>().map_err(|_| bad(format!("bad number {s:?}")));
            plot.vectors[k].re.push(num(re)?);
            if let Some(v) = &mut plot.vectors[k].im {
                v.push(num(im)?);
            }
        }
    }
    Ok(at)
}
