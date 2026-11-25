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

    /// Problems of the model itself (its text and pin count), `pins` being the mapped
    /// pins (explicit or automatic).
    pub fn problems(&self, pins: usize) -> Vec<String> {
        let mut out = Vec::new();
        let want = |n: usize, out: &mut Vec<String>| {
            if pins != n {
                out.push(format!("the model has {n} ports, {pins} pins are mapped"));
            }
        };
        match &self.device {
            SimDevice::Ignore => {}
            SimDevice::Resistor
            | SimDevice::Capacitor
            | SimDevice::Inductor
            | SimDevice::VoltageSource
            | SimDevice::CurrentSource => want(2, &mut out),
            SimDevice::Primitive { kind, model } => match ModelCard::parse(model) {
                None => out.push("the model text has no .model line".into()),
                Some(card) => {
                    if PrimitiveKind::of_card(&card) != Some(*kind) {
                        out.push(format!("a {} card doesn't fit a {}", card.kind, kind.label()));
                    }
                    want(kind.ports().len(), &mut out);
                }
            },
            SimDevice::Subckt { text } => match SubcktHeader::parse(text) {
                None => out.push("the model text has no .subckt line".into()),
                Some(h) => {
                    if !logical_lines(text).iter().any(|l| l.to_ascii_lowercase().starts_with(".ends")) {
                        out.push("the subcircuit has no .ends line".into());
                    }
                    want(h.ports.len(), &mut out);
                }
            },
            SimDevice::Spice { template } => {
                let used = spice_nodes(template);
                if used > pins {
                    out.push(format!("the line uses ${{N{used}}}, {pins} pins are mapped"));
                }
            }
        }
        out
    }
}

/// The highest `${Nk}` a SPICE line template uses.
pub fn spice_nodes(template: &str) -> usize {
    let mut max = 0;
    let mut rest = template;
    while let Some(i) = rest.find("${N") {
        rest = &rest[i + 3..];
        let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
        if rest[digits.len()..].starts_with('}')
            && let Ok(k) = digits.parse::<usize>()
        {
            max = max.max(k);
        }
    }
    max
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
    /// One SPICE line, as written, with `${REF}` (the reference), `${VALUE}` and
    /// `${N1}`, `${N2}`, … for the nodes in `pins` order. A device letter right before
    /// `${REF}` is left out when the reference starts with it (`V${REF}`: `V1` for V1,
    /// `VIN1` for IN1).
    Spice {
        template: String,
    },
}

impl SimDevice {
    /// The SPICE ports in node order: fixed for primitives, R, C, L and sources, the
    /// `.subckt` line's for subcircuits, `N1`… for a SPICE line (as many as it uses).
    pub fn ports(&self) -> Vec<String> {
        let fixed = |p: &[&str]| p.iter().map(|s| s.to_string()).collect();
        match self {
            SimDevice::Ignore => Vec::new(),
            SimDevice::Resistor | SimDevice::Capacitor | SimDevice::Inductor => fixed(&["1", "2"]),
            SimDevice::VoltageSource | SimDevice::CurrentSource => fixed(&["+", "−"]),
            SimDevice::Primitive { kind, .. } => fixed(kind.ports()),
            SimDevice::Subckt { text } => SubcktHeader::parse(text).map(|h| h.ports).unwrap_or_default(),
            SimDevice::Spice { template } => (1..=spice_nodes(template)).map(|k| format!("N{k}")).collect(),
        }
    }

    /// The text of a primitive, subcircuit or SPICE line.
    pub fn text(&self) -> Option<&str> {
        match self {
            SimDevice::Primitive { model, .. } => Some(model),
            SimDevice::Subckt { text } => Some(text),
            SimDevice::Spice { template } => Some(template),
            _ => None,
        }
    }

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

    pub const ALL: [PrimitiveKind; 7] = [
        PrimitiveKind::Diode,
        PrimitiveKind::Npn,
        PrimitiveKind::Pnp,
        PrimitiveKind::Njf,
        PrimitiveKind::Pjf,
        PrimitiveKind::Nmos,
        PrimitiveKind::Pmos,
    ];

    /// The kind a `.model` card is for (`VDMOS` by its `pchan` flag).
    pub fn of_card(card: &ModelCard) -> Option<PrimitiveKind> {
        Some(match card.kind.to_ascii_uppercase().as_str() {
            "D" => PrimitiveKind::Diode,
            "NPN" => PrimitiveKind::Npn,
            "PNP" => PrimitiveKind::Pnp,
            "NJF" => PrimitiveKind::Njf,
            "PJF" => PrimitiveKind::Pjf,
            "NMOS" => PrimitiveKind::Nmos,
            "PMOS" => PrimitiveKind::Pmos,
            "VDMOS" if card.params.iter().any(|(k, _)| k.eq_ignore_ascii_case("pchan")) => PrimitiveKind::Pmos,
            "VDMOS" => PrimitiveKind::Nmos,
            _ => return None,
        })
    }

    /// A card with SPICE's default parameters, named after `name`.
    pub fn generic_card(self, name: &str) -> String {
        let kind = match self {
            PrimitiveKind::Diode => "D",
            PrimitiveKind::Npn => "NPN",
            PrimitiveKind::Pnp => "PNP",
            PrimitiveKind::Njf => "NJF",
            PrimitiveKind::Pjf => "PJF",
            PrimitiveKind::Nmos => "VDMOS",
            PrimitiveKind::Pmos => "VDMOS",
        };
        let flags = if self == PrimitiveKind::Pmos { "pchan" } else { "" };
        let name: String = name.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }).collect();
        format!(".model {}_{name} {kind}({flags})", self.letter())
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
// SPICE text

/// SPICE text joined across `+` continuation lines, without comment lines.
pub fn logical_lines(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for l in text.lines() {
        let t = l.trim();
        if t.is_empty() || t.starts_with('*') {
            continue;
        }
        if let Some(rest) = t.strip_prefix('+')
            && let Some(last) = out.last_mut()
        {
            last.push(' ');
            last.push_str(rest.trim());
            continue;
        }
        out.push(t.to_string());
    }
    out
}

/// A `.model NAME TYPE(params)` card, taken apart.
#[derive(Clone, Debug, PartialEq)]
pub struct ModelCard {
    pub name: String,
    /// `D`, `NPN`, `VDMOS`, …
    pub kind: String,
    /// In order; flags (VDMOS `pchan`) have no value.
    pub params: Vec<(String, Option<String>)>,
}

impl ModelCard {
    pub fn parse(text: &str) -> Option<ModelCard> {
        let line = logical_lines(text).into_iter().find(|l| l.to_ascii_lowercase().starts_with(".model"))?;
        let rest = line[6..].trim_start();
        let (name, rest) = rest.split_once(char::is_whitespace)?;
        let rest = rest.trim_start();
        let end = rest.find(|c: char| c == '(' || c.is_whitespace()).unwrap_or(rest.len());
        let kind = rest[..end].to_string();
        let body = rest[end..].replace(['(', ')', ','], " ").replace('=', " = ");
        let tokens: Vec<&str> = body.split_whitespace().collect();
        let mut params = Vec::new();
        let mut i = 0;
        while i < tokens.len() {
            if tokens.get(i + 1) == Some(&"=") && i + 2 < tokens.len() {
                params.push((tokens[i].to_string(), Some(tokens[i + 2].to_string())));
                i += 3;
            } else {
                params.push((tokens[i].to_string(), None));
                i += 1;
            }
        }
        (!kind.is_empty()).then(|| ModelCard { name: name.to_string(), kind, params })
    }

    pub fn with(mut self, overrides: &[(String, String)]) -> ModelCard {
        for (k, v) in overrides {
            match self.params.iter_mut().find(|(n, _)| n.eq_ignore_ascii_case(k)) {
                Some(p) => p.1 = Some(v.clone()),
                None => self.params.push((k.clone(), Some(v.clone()))),
            }
        }
        self
    }

    pub fn text(&self) -> String {
        let params: Vec<String> =
            self.params.iter().map(|(k, v)| v.as_ref().map_or_else(|| k.clone(), |v| format!("{k}={v}"))).collect();
        format!(".model {} {}({})", self.name, self.kind, params.join(" "))
    }
}

/// The `.subckt NAME ports… [params: …]` line of a subcircuit.
#[derive(Clone, Debug, PartialEq)]
pub struct SubcktHeader {
    pub name: String,
    pub ports: Vec<String>,
}

impl SubcktHeader {
    pub fn parse(text: &str) -> Option<SubcktHeader> {
        let lines = logical_lines(text);
        let line = lines.iter().find(|l| l.to_ascii_lowercase().starts_with(".subckt"))?;
        let mut tokens = line.split_whitespace().skip(1);
        let name = tokens.next()?.to_string();
        let ports =
            tokens.take_while(|t| !t.contains('=') && !t.eq_ignore_ascii_case("params:")).map(str::to_string).collect();
        Some(SubcktHeader { name, ports })
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
    /// The numeric fields a tuner or sweep can change: the waveform's (see
    /// [`Waveform::fields_mut`]) plus `ac` (the AC magnitude).
    pub fn fields_mut(&mut self) -> Vec<(&'static str, &mut f64)> {
        let mut f = self.waveform.fields_mut();
        f.push(("ac", &mut self.ac));
        f
    }

    /// A field's value by key.
    pub fn field(&self, key: &str) -> Option<f64> {
        self.clone().fields_mut().into_iter().find(|(k, _)| *k == key).map(|(_, v)| *v)
    }

    /// Sets a field by key; false when there is no such field.
    pub fn set_field(&mut self, key: &str, value: f64) -> bool {
        match self.fields_mut().into_iter().find(|(k, _)| *k == key) {
            Some((_, v)) => {
                *v = value;
                true
            }
            None => false,
        }
    }

    /// The unit of a field: the source's own (`unit`, V or A) for levels, s, Hz, ° or
    /// 1/s for the others.
    pub fn field_unit<'a>(key: &str, unit: &'a str) -> &'a str {
        match key {
            "frequency" => "Hz",
            "phase" => "°",
            "damping" => "1/s",
            "delay" | "rise" | "fall" | "width" | "period" | "delay1" | "tau1" | "delay2" | "tau2" => "s",
            _ => unit,
        }
    }

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
    /// Its numeric fields by key (none for PWL).
    pub fn fields_mut(&mut self) -> Vec<(&'static str, &mut f64)> {
        match self {
            Waveform::Dc { value } => vec![("value", value)],
            Waveform::Sine { offset, amplitude, frequency, delay, damping, phase } => vec![
                ("offset", offset),
                ("amplitude", amplitude),
                ("frequency", frequency),
                ("delay", delay),
                ("damping", damping),
                ("phase", phase),
            ],
            Waveform::Pulse { low, high, delay, rise, fall, width, period } => vec![
                ("low", low),
                ("high", high),
                ("delay", delay),
                ("rise", rise),
                ("fall", fall),
                ("width", width),
                ("period", period),
            ],
            Waveform::Pwl { .. } => Vec::new(),
            Waveform::Exp { v1, v2, delay1, tau1, delay2, tau2 } => {
                vec![("v1", v1), ("v2", v2), ("delay1", delay1), ("tau1", tau1), ("delay2", delay2), ("tau2", tau2)]
            }
        }
    }

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

    /// The labels of the kinds, in menu order ([`with_kind`](Self::with_kind) takes them).
    pub const KINDS: [&'static str; 5] = ["DC", "Sine", "Pulse", "Piecewise linear", "Exponential"];

    /// The base value: DC value, sine offset, pulse low, first PWL value, exponential v1.
    pub fn base(&self) -> f64 {
        match self {
            Waveform::Dc { value } => *value,
            Waveform::Sine { offset, .. } => *offset,
            Waveform::Pulse { low, .. } => *low,
            Waveform::Pwl { points, .. } => points.first().map_or(0.0, |p| p.1),
            Waveform::Exp { v1, .. } => *v1,
        }
    }

    /// This waveform as another kind (by its [`label`](Self::label)), keeping the base
    /// value; the rest are defaults (1 above the base, 1 kHz).
    pub fn with_kind(&self, kind: &str) -> Waveform {
        if kind == self.label() {
            return self.clone();
        }
        let b = self.base();
        match kind {
            "Sine" => {
                Waveform::Sine { offset: b, amplitude: 1.0, frequency: 1e3, delay: 0.0, damping: 0.0, phase: 0.0 }
            }
            "Pulse" => Waveform::Pulse {
                low: b,
                high: b + 1.0,
                delay: 0.0,
                rise: 1e-6,
                fall: 1e-6,
                width: 0.5e-3,
                period: 1e-3,
            },
            "Piecewise linear" => Waveform::Pwl { points: vec![(0.0, b), (1e-3, b + 1.0)], repeat: false },
            "Exponential" => Waveform::Exp { v1: b, v2: b + 1.0, delay1: 0.0, tau1: 1e-4, delay2: 5e-4, tau2: 1e-4 },
            _ => Waveform::Dc { value: b },
        }
    }

    /// The value at time `t` (as ngspice computes it in a transient).
    pub fn at(&self, t: f64) -> f64 {
        match self {
            Waveform::Dc { value } => *value,
            Waveform::Sine { offset, amplitude, frequency, delay, damping, phase } => {
                if t < *delay {
                    offset + amplitude * phase.to_radians().sin()
                } else {
                    let u = t - delay;
                    offset
                        + amplitude
                            * (std::f64::consts::TAU * frequency * u + phase.to_radians()).sin()
                            * (-damping * u).exp()
                }
            }
            Waveform::Pulse { low, high, delay, rise, fall, width, period } => {
                if t < *delay {
                    return *low;
                }
                let mut u = t - delay;
                if *period > 0.0 {
                    u %= period;
                } else if u > rise + width + fall {
                    return *low;
                }
                if u < *rise {
                    low + (high - low) * u / rise.max(1e-30)
                } else if u < rise + width {
                    *high
                } else if u < rise + width + fall {
                    high - (high - low) * (u - rise - width) / fall.max(1e-30)
                } else {
                    *low
                }
            }
            Waveform::Pwl { points, repeat } => {
                let Some(&(t_end, _)) = points.last() else { return 0.0 };
                let t = if *repeat && t_end > 0.0 { t % t_end } else { t };
                match points.iter().position(|p| p.0 > t) {
                    None => points.last().map_or(0.0, |p| p.1),
                    Some(0) => points[0].1,
                    Some(i) => {
                        let ((t0, v0), (t1, v1)) = (points[i - 1], points[i]);
                        if t1 > t0 { v0 + (v1 - v0) * (t - t0) / (t1 - t0) } else { v1 }
                    }
                }
            }
            Waveform::Exp { v1, v2, delay1, tau1, delay2, tau2 } => {
                let mut v = *v1;
                if t >= *delay1 {
                    v += (v2 - v1) * (1.0 - (-(t - delay1) / tau1.max(1e-30)).exp());
                }
                if t >= *delay2 {
                    v += (v1 - v2) * (1.0 - (-(t - delay2) / tau2.max(1e-30)).exp());
                }
                v
            }
        }
    }

    /// A time span that shows the waveform: three periods, a pulse or PWL with some time
    /// after it, an exponential's two edges settled.
    pub fn preview_span(&self) -> f64 {
        let span = match self {
            Waveform::Dc { .. } => 1e-3,
            Waveform::Sine { frequency, delay, .. } if *frequency > 0.0 => delay + 3.0 / frequency,
            Waveform::Sine { .. } => 1e-3,
            Waveform::Pulse { delay, period, .. } if *period > 0.0 => delay + 3.0 * period,
            Waveform::Pulse { delay, rise, width, fall, .. } => (delay + rise + width + fall) * 1.5,
            Waveform::Pwl { points, .. } => points.last().map_or(1e-3, |p| p.0 * 1.2),
            Waveform::Exp { delay2, tau2, .. } => delay2 + 5.0 * tau2,
        };
        if span > 0.0 && span.is_finite() { span } else { 1e-3 }
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
    /// The tuner's rows (the tuned values themselves aren't saved).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tune: Vec<TuneRow>,
}

/// A value the tuner (or a parameter sweep) changes.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TuneParam {
    /// A component's value (parts whose model takes `${VALUE}`: R, C, L, …).
    Value { component: Id },
    /// A source's field: a waveform field (`amplitude`, `frequency`, `value`, …) or `ac`.
    Source { component: Id, field: String },
}

impl TuneParam {
    pub fn component(&self) -> Id {
        match self {
            TuneParam::Value { component } | TuneParam::Source { component, .. } => *component,
        }
    }

    /// Component values move on a log scale, source fields on a linear one.
    pub fn is_log(&self) -> bool {
        matches!(self, TuneParam::Value { .. })
    }
}

/// Snapping of tuned values to a preferred value series.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Snap {
    #[default]
    None,
    E6,
    E12,
    E24,
    E96,
}

impl Snap {
    pub const ALL: [Snap; 5] = [Snap::None, Snap::E6, Snap::E12, Snap::E24, Snap::E96];

    pub fn label(self) -> &'static str {
        match self {
            Snap::None => "None",
            Snap::E6 => "E6",
            Snap::E12 => "E12",
            Snap::E24 => "E24",
            Snap::E96 => "E96",
        }
    }

    pub fn apply(self, v: f64) -> f64 {
        match self {
            Snap::None => v,
            Snap::E6 => crate::si::snap_e(v, 6),
            Snap::E12 => crate::si::snap_e(v, 12),
            Snap::E24 => crate::si::snap_e(v, 24),
            Snap::E96 => crate::si::snap_e(v, 96),
        }
    }
}

/// A tuner row: the parameter, its slider's range and snapping.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct TuneRow {
    pub param: TuneParam,
    pub min: f64,
    pub max: f64,
    #[serde(default, skip_serializing_if = "is_no_snap")]
    pub snap: Snap,
}

fn is_no_snap(s: &Snap) -> bool {
    *s == Snap::None
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
            tune: Vec::new(),
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
    /// Runs it once per value of a parameter (a parameter sweep).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub step: Option<Step>,
}

impl Analysis {
    pub fn new(name: impl Into<String>, kind: AnalysisKind) -> Analysis {
        Analysis { id: Id::new(), name: name.into(), kind, probes: Vec::new(), step: None }
    }
}

/// Most steps of a parameter sweep.
pub const MAX_STEPS: usize = 64;

/// A parameter sweep: the parameter (a part's value or a source's field, like the
/// tuner's) and its values.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Step {
    pub param: TuneParam,
    pub values: StepValues,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum StepValues {
    /// These values, in this order.
    List { values: Vec<f64> },
    /// `start`, `start + step`, … up to `stop` (included when it falls on a step).
    Linear { start: f64, stop: f64, step: f64 },
    /// `points_per_decade` values per decade from `start` to `stop` (both > 0).
    Log { start: f64, stop: f64, points_per_decade: u32 },
}

impl StepValues {
    /// The values, at most [`MAX_STEPS`] + 1 (more means too many: see
    /// [`problem`](Self::problem)).
    pub fn points(&self) -> Vec<f64> {
        let cap = MAX_STEPS + 1;
        match self {
            StepValues::List { values } => values.iter().copied().take(cap).collect(),
            StepValues::Linear { start, stop, step } => {
                if !(step.is_finite() && *step != 0.0 && start.is_finite() && stop.is_finite()) {
                    return Vec::new();
                }
                let n = ((stop - start) / step + 1e-9).floor();
                if n < 0.0 {
                    return Vec::new();
                }
                (0..=(n as usize).min(cap)).map(|k| start + k as f64 * step).take(cap).collect()
            }
            StepValues::Log { start, stop, points_per_decade } => {
                if !(*start > 0.0 && *stop > 0.0 && *points_per_decade > 0 && start.is_finite() && stop.is_finite()) {
                    return Vec::new();
                }
                let decades = (stop / start).log10();
                let n = (decades * *points_per_decade as f64 + 1e-9).floor();
                if n < 0.0 {
                    return Vec::new();
                }
                (0..=(n as usize).min(cap))
                    .map(|k| start * 10f64.powf(k as f64 / *points_per_decade as f64))
                    .take(cap)
                    .collect()
            }
        }
    }

    /// What is wrong with the values, if anything.
    pub fn problem(&self) -> Option<String> {
        let p = self.points();
        if p.is_empty() {
            return Some("a parameter sweep needs values".into());
        }
        if p.len() > MAX_STEPS {
            return Some(format!("a parameter sweep has at most {MAX_STEPS} steps"));
        }
        if p.iter().any(|v| !v.is_finite()) {
            return Some("a parameter sweep's values must be numbers".into());
        }
        None
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
    /// The voltage of a net to ground, or to `reference` (differential). Found by `at`
    /// (a wire, label or junction on it), else by the net's name when `at` is gone.
    Voltage {
        net: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        at: Option<ObjectId>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reference: Option<NetRef>,
        /// Shown as a logic level (high or low) in a lane under the plot, not as a
        /// voltage on its axes (transient and DC sweep; not differential ones).
        #[serde(default, skip_serializing_if = "is_false")]
        logic: bool,
    },
    /// The current into a pin of a placed symbol.
    Current { symbol: Id, pin: Id },
    /// The power a component takes: Σ V·I over its pins (negative: it delivers power).
    Power { component: Id },
}

impl Probe {
    /// A voltage probe of a net to ground.
    pub fn voltage(net: impl Into<String>, at: Option<ObjectId>) -> Probe {
        Probe::Voltage { net: net.into(), at, reference: None, logic: false }
    }
}

/// A net as a probe names it: an object on it, else its name.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct NetRef {
    pub net: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at: Option<ObjectId>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn waveforms() {
        let sine = Waveform::Dc { value: 5.0 }.with_kind("Sine");
        assert_eq!(sine.base(), 5.0);
        assert!((sine.at(0.25e-3) - 6.0).abs() < 1e-9, "the peak a quarter period in");
        assert!((sine.preview_span() - 3e-3).abs() < 1e-12);
        let pulse = sine.with_kind("Pulse");
        assert_eq!((pulse.at(0.0), pulse.at(0.2e-3), pulse.at(0.8e-3), pulse.at(1.2e-3)), (5.0, 6.0, 5.0, 6.0));
        let pwl = Waveform::Pwl { points: vec![(0.0, 0.0), (1.0, 2.0)], repeat: false };
        assert_eq!((pwl.at(0.5), pwl.at(5.0)), (1.0, 2.0));
        let exp = pwl.with_kind("Exponential");
        assert!((exp.at(0.49e-3) - 1.0).abs() < 0.01, "risen to v2");
        assert!(exp.at(1.5e-3).abs() < 0.01, "back to v1");
    }

    #[test]
    fn model_problems() {
        let diode = |text: &str| SimModel::new(SimDevice::Primitive { kind: PrimitiveKind::Diode, model: text.into() });
        assert!(diode(".model X D(IS=1n)").problems(2).is_empty());
        assert_eq!(diode(".model X NPN()").problems(2), ["a NPN card doesn't fit a Diode"]);
        assert_eq!(diode("IS=1n").problems(2), ["the model text has no .model line"]);
        assert_eq!(diode(".model X D()").problems(3), ["the model has 2 ports, 3 pins are mapped"]);
        let sub = SimModel::new(SimDevice::Subckt {
            text: ".subckt AMP inp inn out params: g=1\nE1 out 0 inp inn {g}".into(),
        });
        assert_eq!(sub.device.ports(), ["inp", "inn", "out"]);
        assert_eq!(sub.problems(3), ["the subcircuit has no .ends line"]);
        let line = SimModel::new(SimDevice::Spice { template: "B${REF} ${N1} ${N3} V=1".into() });
        assert_eq!(line.device.ports(), ["N1", "N2", "N3"]);
        assert_eq!(line.problems(2), ["the line uses ${N3}, 2 pins are mapped"]);
        for k in PrimitiveKind::ALL {
            let card = ModelCard::parse(&k.generic_card("2N7002")).unwrap();
            assert_eq!(PrimitiveKind::of_card(&card), Some(k), "{}", k.generic_card("x"));
        }
    }

    #[test]
    fn builtin_models_have_no_issues() {
        let lib = crate::builtin::builtin_library();
        let issues: Vec<_> =
            lib.parts.iter().flat_map(|p| lib.sim_issues(p).into_iter().map(|m| (p.name.clone(), m))).collect();
        assert!(issues.is_empty(), "{issues:?}");
    }

    #[test]
    fn model_cards() {
        let c = ModelCard::parse(".model 1N4148 D(IS=2.52n N = 1.752\n+ RS=0.568)").unwrap();
        assert_eq!(c.name, "1N4148");
        assert_eq!(c.kind, "D");
        assert_eq!(c.params.len(), 3);
        assert_eq!(c.params[1], ("N".into(), Some("1.752".into())));
        let c = c.with(&[("rs".into(), "1".into()), ("BV".into(), "100".into())]);
        assert_eq!(c.text(), ".model 1N4148 D(IS=2.52n N=1.752 RS=1 BV=100)");
        let v = ModelCard::parse(".model BSS84 VDMOS(pchan Vto=-1.6 Kp=0.1)").unwrap();
        assert_eq!(v.params[0], ("pchan".into(), None));
    }
}
