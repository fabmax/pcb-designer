//! Simulation data: how parts simulate ([`SimModel`]), per-component settings
//! ([`ComponentSim`], source waveforms) and the schematic's simulation setup
//! ([`Simulation`]: analyses, ground, supplies). The `sim` crate turns these into a SPICE
//! netlist and runs it.
//!
//! Numbers of waveforms and analyses are plain SI `f64` (volts, amperes, seconds,
//! hertz). Model parameters and component values stay strings, as typed.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::id::Id;
use crate::object::ObjectId;

fn is_false(b: &bool) -> bool {
    !*b
}

// ---------------------------------------------------------------------------------------
// Models (part content)

/// How a part simulates. Part content: it travels with the part into projects and is
/// covered by its content hash.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct SimModel {
    pub device: SimDevice,
    /// The part's pins in SPICE node order. Empty: every pin of every unit, in the order
    /// the symbols list them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pins: Vec<SimPin>,
    /// Parameters by name, as typed ("BV" = "5.1"). For primitives they override the
    /// `.model` card's; for subcircuits they are passed as instance parameters. A value
    /// `${VALUE}` takes the component's value. Component parameters override these.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub params: BTreeMap<String, String>,
}

impl SimModel {
    pub fn new(device: SimDevice) -> SimModel {
        SimModel { device, pins: Vec::new(), params: BTreeMap::new() }
    }
}

/// A pin of a part: its unit and the pin of that unit's symbol.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, schemars::JsonSchema)]
pub struct SimPin {
    pub unit: u32,
    pub pin: Id,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SimDevice {
    /// Not simulated, on purpose (connectors, test points, mounting holes): left out
    /// without a warning.
    Ignore,
    /// Resistor, capacitor, inductor: the value is the component's value ("10k", "100n").
    Resistor,
    Capacitor,
    Inductor,
    /// A SPICE primitive with its `.model` card (`.model 1N4148 D(IS=2.52n N=1.752)`).
    /// Nodes: diode A K; BJT C B E; JFET and MOSFET D G S (bulk tied to the source).
    Primitive {
        kind: PrimitiveKind,
        model: String,
    },
    /// A subcircuit (`.subckt NAME ports… … .ends`); its ports in order are `pins`.
    Subckt {
        text: String,
    },
    /// Independent sources; the waveform is the component's ([`ComponentSim::source`]).
    /// Nodes: + then −. Current flows from + through the source to −.
    VoltageSource,
    CurrentSource,
    /// One SPICE line, as written, with `${REF}` (the SPICE name), `${VALUE}` and
    /// `${N1}`, `${N2}`, … for the nodes in `pins` order.
    Spice {
        template: String,
    },
}

impl SimDevice {
    pub fn label(&self) -> &'static str {
        match self {
            SimDevice::Ignore => "Not simulated",
            SimDevice::Resistor => "Resistor",
            SimDevice::Capacitor => "Capacitor",
            SimDevice::Inductor => "Inductor",
            SimDevice::Primitive { kind, .. } => kind.label(),
            SimDevice::Subckt { .. } => "Subcircuit",
            SimDevice::VoltageSource => "Voltage source",
            SimDevice::CurrentSource => "Current source",
            SimDevice::Spice { .. } => "SPICE line",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PrimitiveKind {
    Diode,
    Npn,
    Pnp,
    Njf,
    Pjf,
    Nmos,
    Pmos,
}

impl PrimitiveKind {
    pub fn label(self) -> &'static str {
        match self {
            PrimitiveKind::Diode => "Diode",
            PrimitiveKind::Npn => "NPN transistor",
            PrimitiveKind::Pnp => "PNP transistor",
            PrimitiveKind::Njf => "N-channel JFET",
            PrimitiveKind::Pjf => "P-channel JFET",
            PrimitiveKind::Nmos => "N-channel MOSFET",
            PrimitiveKind::Pmos => "P-channel MOSFET",
        }
    }

    /// The SPICE device letter.
    pub fn letter(self) -> char {
        match self {
            PrimitiveKind::Diode => 'D',
            PrimitiveKind::Npn | PrimitiveKind::Pnp => 'Q',
            PrimitiveKind::Njf | PrimitiveKind::Pjf => 'J',
            PrimitiveKind::Nmos | PrimitiveKind::Pmos => 'M',
        }
    }

    /// Names of the nodes, in order.
    pub fn ports(self) -> &'static [&'static str] {
        match self {
            PrimitiveKind::Diode => &["A", "K"],
            PrimitiveKind::Npn | PrimitiveKind::Pnp => &["C", "B", "E"],
            _ => &["D", "G", "S"],
        }
    }
}

// ---------------------------------------------------------------------------------------
// Per component

/// A component's simulation settings.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ComponentSim {
    /// Overrides of the part model's parameters.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub params: BTreeMap<String, String>,
    /// Left out of the simulation.
    #[serde(default, skip_serializing_if = "is_false")]
    pub exclude: bool,
    /// The waveform of a source part; `None`: 0 V / 0 A DC.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<Source>,
}

impl ComponentSim {
    pub fn is_default(&self) -> bool {
        *self == ComponentSim::default()
    }
}

/// An independent source's signal.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Source {
    pub waveform: Waveform,
    /// Small-signal magnitude for AC analysis; 0: not an AC input.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub ac: f64,
    /// Small-signal phase in degrees.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub ac_phase: f64,
}

fn is_zero(v: &f64) -> bool {
    *v == 0.0
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Waveform {
    Dc {
        value: f64,
    },
    /// `offset + amplitude · sin(2π f (t − delay) + phase) · e^(−damping (t − delay))`.
    Sine {
        offset: f64,
        amplitude: f64,
        frequency: f64,
        #[serde(default)]
        delay: f64,
        #[serde(default)]
        damping: f64,
        /// Degrees.
        #[serde(default)]
        phase: f64,
    },
    /// `low` until `delay`, then a ramp to `high` over `rise`, `width` at `high`, a ramp
    /// back over `fall`; repeating every `period` (0: once).
    Pulse {
        low: f64,
        high: f64,
        #[serde(default)]
        delay: f64,
        rise: f64,
        fall: f64,
        width: f64,
        #[serde(default)]
        period: f64,
    },
    /// Straight lines between (time, value) points; held after the last.
    Pwl {
        points: Vec<(f64, f64)>,
        /// Start again from the first point after the last.
        #[serde(default, skip_serializing_if = "is_false")]
        repeat: bool,
    },
    /// `v1` until `delay1`, towards `v2` with `tau1`, back towards `v1` from `delay2`
    /// with `tau2`.
    Exp {
        v1: f64,
        v2: f64,
        delay1: f64,
        tau1: f64,
        delay2: f64,
        tau2: f64,
    },
}

impl Source {
    /// The value text of a source component: "DC 5 V", "SIN 1 V 1 kHz", "PULSE 0–3.3 V
    /// 1 kHz", plus "AC 1 V" for an AC input. `unit`: "V" or "A".
    pub fn describe(&self, unit: &str) -> String {
        let f = |v: f64| crate::si::format(v, unit);
        let range = |a: f64, b: f64| {
            let (a, b) = (crate::si::format(a, ""), crate::si::format(b, unit));
            format!("{a}–{b}")
        };
        let mut text = match &self.waveform {
            Waveform::Dc { value } => format!("DC {}", f(*value)),
            Waveform::Sine { offset, amplitude, frequency, .. } => {
                let offset = if *offset == 0.0 { String::new() } else { format!("{} ± ", f(*offset)) };
                format!("SIN {offset}{} {}", f(*amplitude), crate::si::format(*frequency, "Hz"))
            }
            Waveform::Pulse { low, high, period, .. } => {
                let freq =
                    if *period > 0.0 { format!(" {}", crate::si::format(1.0 / period, "Hz")) } else { String::new() };
                format!("PULSE {}{freq}", range(*low, *high))
            }
            Waveform::Pwl { points, .. } => format!("PWL {} points", points.len()),
            Waveform::Exp { v1, v2, .. } => format!("EXP {}", range(*v1, *v2)),
        };
        if self.ac != 0.0 {
            text.push_str(&format!(" AC {}", f(self.ac)));
        }
        text
    }
}

impl Default for Waveform {
    fn default() -> Self {
        Waveform::Dc { value: 0.0 }
    }
}

impl Waveform {
    /// The value at time 0 (the DC value for operating point and DC sweeps).
    pub fn initial(&self) -> f64 {
        match self {
            Waveform::Dc { value } => *value,
            Waveform::Sine { offset, amplitude, delay, phase, .. } => {
                if *delay > 0.0 {
                    *offset
                } else {
                    offset + amplitude * phase.to_radians().sin()
                }
            }
            Waveform::Pulse { low, .. } => *low,
            Waveform::Pwl { points, .. } => points.first().map_or(0.0, |p| p.1),
            Waveform::Exp { v1, .. } => *v1,
        }
    }

    /// The longest period of a periodic waveform (for default transient lengths).
    pub fn period(&self) -> Option<f64> {
        match self {
            Waveform::Sine { frequency, .. } if *frequency > 0.0 => Some(1.0 / frequency),
            Waveform::Pulse { period, .. } if *period > 0.0 => Some(*period),
            _ => None,
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            Waveform::Dc { .. } => "DC",
            Waveform::Sine { .. } => "Sine",
            Waveform::Pulse { .. } => "Pulse",
            Waveform::Pwl { .. } => "Piecewise linear",
            Waveform::Exp { .. } => "Exponential",
        }
    }
}

// ---------------------------------------------------------------------------------------
// Setup

/// The schematic's simulation setup. Changed as a whole (`SetSimulation`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Simulation {
    /// In tab order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub analyses: Vec<Analysis>,
    /// The net that is SPICE node 0; `None`: the nets of ground power ports.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ground: Option<String>,
    /// DC supplies for power nets that have no source drawn.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub supplies: Vec<Supply>,
    /// Circuit temperature in °C.
    #[serde(default = "room", skip_serializing_if = "is_room")]
    pub temperature: f64,
    /// Extra SPICE lines (`.param`, `.options`), as typed.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub extra: String,
}

fn room() -> f64 {
    27.0
}

fn is_room(t: &f64) -> bool {
    *t == 27.0
}

impl Default for Simulation {
    fn default() -> Self {
        Simulation {
            analyses: Vec::new(),
            ground: None,
            supplies: Vec::new(),
            temperature: room(),
            extra: String::new(),
        }
    }
}

impl Simulation {
    pub fn is_default(&self) -> bool {
        *self == Simulation::default()
    }

    /// An analysis by id, else by name (case-insensitive).
    pub fn find(&self, key: &str) -> Option<&Analysis> {
        self.analyses
            .iter()
            .find(|a| a.id.to_string() == key)
            .or_else(|| self.analyses.iter().find(|a| a.name.eq_ignore_ascii_case(key)))
    }
}

/// A DC supply on a net, to ground.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Supply {
    /// Net name ("+5V").
    pub net: String,
    pub voltage: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Analysis {
    pub id: Id,
    pub name: String,
    pub kind: AnalysisKind,
    /// The signals its plot shows.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub probes: Vec<Probe>,
}

impl Analysis {
    pub fn new(name: impl Into<String>, kind: AnalysisKind) -> Analysis {
        Analysis { id: Id::new(), name: name.into(), kind, probes: Vec::new() }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AnalysisKind {
    /// DC operating point.
    Op,
    /// Sweeps the DC value of a source component.
    Dc { source: Id, start: f64, stop: f64, step: f64 },
    /// Small-signal frequency response, logarithmic.
    Ac { start: f64, stop: f64, points_per_decade: u32 },
    /// Time domain. `step`: the output step (`None`: `stop` / 1000); `max_step` limits
    /// the solver's internal step.
    Tran {
        stop: f64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        step: Option<f64>,
        #[serde(default, skip_serializing_if = "is_zero")]
        start: f64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max_step: Option<f64>,
    },
}

impl AnalysisKind {
    pub fn label(&self) -> &'static str {
        match self {
            AnalysisKind::Op => "Operating point",
            AnalysisKind::Dc { .. } => "DC sweep",
            AnalysisKind::Ac { .. } => "AC",
            AnalysisKind::Tran { .. } => "Transient",
        }
    }
}

/// A signal to show.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Probe {
    /// The voltage of a net to ground. Found by `at` (a wire, label or junction on it),
    /// else by the net's name when `at` is gone.
    Voltage {
        net: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        at: Option<ObjectId>,
    },
    /// The current into a pin of a placed symbol.
    Current { symbol: Id, pin: Id },
}
