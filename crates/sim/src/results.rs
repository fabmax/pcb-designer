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

    /// The voltage of `net` to `reference`.
    pub fn voltage_between(&self, net: &str, reference: &str) -> Option<Vector> {
        let (a, b) = (self.voltage(net)?, self.voltage(reference)?);
        let sub = |x: &[f64], y: &[f64]| x.iter().zip(y).map(|(x, y)| x - y).collect::<Vec<_>>();
        let im = match (&a.im, &b.im) {
            (Some(x), Some(y)) => Some(sub(x, y)),
            _ => None,
        };
        Some(Vector { name: format!("{}-{}", a.name, b.name), kind: "voltage".into(), re: sub(&a.re, &b.re), im })
    }

    /// The power a component takes, Σ V·I over its pins, when every pin's current was
    /// measured (not for AC: complex power isn't shown).
    pub fn power(&self, component: Id) -> Option<Vector> {
        if self.plot.complex {
            return None;
        }
        let pins: Vec<_> = self.netlist.pins.iter().filter(|p| p.component == component).collect();
        if pins.is_empty() {
            return None;
        }
        let mut re = vec![0.0; self.plot.len()];
        for p in pins {
            // An unplaced unit's pins are open: no current.
            let Some(symbol) = p.symbol else { continue };
            let i = self.current(symbol, p.pin)?;
            if p.node == "0" {
                continue;
            }
            let v = self.plot.vector(&format!("v({})", p.node))?;
            for (k, out) in re.iter_mut().enumerate() {
                *out += v.re.get(k).copied().unwrap_or(0.0) * i.re.get(k).copied().unwrap_or(0.0);
            }
        }
        Some(Vector { name: "power".into(), kind: "power".into(), re, im: None })
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
