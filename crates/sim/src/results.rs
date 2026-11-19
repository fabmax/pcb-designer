//! The result of a simulation run, read by net and pin.

use std::borrow::Cow;

use model::Id;

use crate::netlist::Netlist;
use crate::raw::{Plot, Vector};

/// One run: its plot, the netlist it ran (with the name maps), ngspice's log.
#[derive(Clone, Debug)]
pub struct SimResult {
    pub plot: Plot,
    pub netlist: Netlist,
    pub log: String,
    /// ngspice's warnings of a run that succeeded.
    pub warnings: Vec<String>,
}

impl SimResult {
    /// The scale (time, frequency, the swept value); `None` for an operating point.
    pub fn scale(&self) -> Option<&Vector> {
        let first = self.plot.vectors.first()?;
        matches!(first.kind.as_str(), "time" | "frequency" | "voltage" | "current")
            .then_some(first)
            .filter(|_| !self.plot.name.eq_ignore_ascii_case("Operating Point"))
    }

    /// The voltage of a net to ground (zeros for ground itself).
    pub fn voltage(&self, net: &str) -> Option<Cow<'_, Vector>> {
        let node = self.netlist.node(net)?;
        if node == "0" {
            let n = self.plot.len();
            return Some(Cow::Owned(Vector {
                name: "0".into(),
                kind: "voltage".into(),
                re: vec![0.0; n],
                im: self.plot.complex.then(|| vec![0.0; n]),
            }));
        }
        self.plot.vector(&format!("v({node})")).map(Cow::Borrowed)
    }

    /// The current into a pin that had a current probe.
    pub fn current(&self, symbol: Id, pin: Id) -> Option<&Vector> {
        let source = self.netlist.sense.get(&(symbol, pin))?;
        self.plot.vector(&format!("i({source})"))
    }

    /// An operating point value (the first point) of a net's voltage.
    pub fn op_voltage(&self, net: &str) -> Option<f64> {
        self.voltage(net).and_then(|v| v.re.first().copied())
    }
}

impl Vector {
    /// Magnitude of each point (the value itself for real vectors).
    pub fn magnitude(&self) -> Vec<f64> {
        match &self.im {
            Some(im) => self.re.iter().zip(im).map(|(r, i)| r.hypot(*i)).collect(),
            None => self.re.iter().map(|r| r.abs()).collect(),
        }
    }

    /// Magnitude in dB (20 log10).
    pub fn db(&self) -> Vec<f64> {
        self.magnitude().into_iter().map(|m| 20.0 * m.max(1e-300).log10()).collect()
    }

    /// Phase in degrees (0 for real vectors).
    pub fn phase(&self) -> Vec<f64> {
        match &self.im {
            Some(im) => self.re.iter().zip(im).map(|(r, i)| i.atan2(*r).to_degrees()).collect(),
            None => vec![0.0; self.re.len()],
        }
    }

    /// Linear interpolation of the real part at `x` of `scale` (sorted ascending);
    /// clamped to the ends.
    pub fn at(&self, scale: &Vector, x: f64) -> Option<f64> {
        let s = &scale.re;
        if s.is_empty() || s.len() != self.re.len() {
            return None;
        }
        let i = s.partition_point(|&v| v < x);
        if i == 0 {
            return Some(self.re[0]);
        }
        if i >= s.len() {
            return self.re.last().copied();
        }
        let (x0, x1) = (s[i - 1], s[i]);
        let t = if x1 > x0 { (x - x0) / (x1 - x0) } else { 0.0 };
        Some(self.re[i - 1] + t * (self.re[i] - self.re[i - 1]))
    }
}
