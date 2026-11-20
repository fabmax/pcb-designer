//! The SPICE netlist of a schematic.
//!
//! * Nodes: one per net with pins. Ground nets (the nets of ground power ports, or the
//!   net named in the setup) are node `0`; the others get readable, sanitised lower-case
//!   names (`/OUT` → `out`, `+5V` → `p5v`, `Net-(R1-1)` → `net_r1_1`), unique.
//! * Devices: one per component (units merged), named by the SPICE letter and the
//!   reference (`R1`, `D1`, `Q1`, `XU1`; `RFB1` for a resistor named `FB1`). Parts
//!   without a model are left out with one warning; `Ignore` ones silently.
//! * Pins that are open (unwired, or of an unplaced unit) get a node of their own;
//!   `.options rshunt` ties every node to ground through 1 TΩ, so ngspice doesn't stop on
//!   a singular matrix.
//! * Numbers are written as plain exponents (`si::spice`), never with SPICE suffixes:
//!   SPICE reads `M` as milli and ignores unknown letters (`1F` is one femto).
//! * Current probes put a 0 V source in series with the pin (`VSENSE_R1_1`), whose
//!   branch current is the current into the pin. An operating point measures every pin
//!   of every device this way (the device table and power).

use std::collections::{BTreeMap, HashMap, HashSet};

use editor_core::connectivity::Connectivity;
use model::*;

use crate::tune::Overrides;

/// How serious a netlist finding is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    Error,
    Warning,
    Note,
}

/// Something about the netlist the user should know, with the objects involved.
#[derive(Clone, Debug, PartialEq)]
pub struct SimProblem {
    pub level: Level,
    pub message: String,
    pub objects: Vec<ObjectId>,
}

/// A netlist and how its names map back to the schematic.
#[derive(Clone, Debug, Default)]
pub struct Netlist {
    pub text: String,
    /// Net name → SPICE node (`0` for ground).
    pub nodes: BTreeMap<String, String>,
    /// Component → SPICE device name, as written (upper case letter first).
    pub devices: HashMap<Id, String>,
    /// (symbol instance, pin) → the sense source measuring the current into the pin.
    pub sense: HashMap<(Id, Id), String>,
    /// The pins of the simulated devices, devices in reference order, pins in node order.
    pub pins: Vec<DevicePin>,
    pub problems: Vec<SimProblem>,
}

/// A pin of a simulated device.
#[derive(Clone, Debug, PartialEq)]
pub struct DevicePin {
    pub component: Id,
    pub refdes: String,
    /// The symbol of the pin's unit (`None`: the unit isn't placed).
    pub symbol: Option<Id>,
    pub pin: Id,
    /// The pin's number, else its name.
    pub key: String,
    /// The SPICE node outside the sense source (the net's node).
    pub node: String,
}

impl Netlist {
    /// The label of the current into a pin: `I(R1.1)`.
    pub fn current_label(&self, symbol: Id, pin: Id) -> Option<String> {
        self.pins
            .iter()
            .find(|p| p.symbol == Some(symbol) && p.pin == pin)
            .map(|p| format!("I({}.{})", p.refdes, p.key))
    }

    pub fn has_errors(&self) -> bool {
        self.problems.iter().any(|p| p.level == Level::Error)
    }

    /// The SPICE node of a net.
    pub fn node(&self, net: &str) -> Option<&str> {
        self.nodes.get(net).map(String::as_str)
    }
}

/// The netlist of `project` for `analysis` (its analysis line and its current probes).
pub fn build(project: &Project, conn: &Connectivity, analysis: &Analysis) -> Netlist {
    build_with(project, conn, analysis, &Overrides::default())
}

/// [`build`] with components overridden (tuned values, a sweep's step).
pub fn build_with(project: &Project, conn: &Connectivity, analysis: &Analysis, overrides: &Overrides) -> Netlist {
    let mut b = Builder {
        project,
        overrides,
        out: Netlist::default(),
        used_nodes: HashSet::new(),
        used_devices: HashSet::new(),
    };
    b.used_nodes.insert("0".into());
    b.nodes(conn);
    let mut lines = Vec::new();
    let mut models = Models::default();
    b.devices(conn, analysis, &mut lines, &mut models);
    b.supplies(&mut lines);
    let analysis_line = b.analysis_line(analysis);

    let sim = &project.schematic.simulation;
    let mut text = format!("{}: {}\n", one_line(&project.name), one_line(&analysis.name));
    text.push_str(".options rshunt=1e12\n");
    if sim.temperature != 27.0 {
        text.push_str(&format!(".temp {}\n", si::spice(sim.temperature)));
    }
    text.push('\n');
    for l in lines {
        text.push_str(&l);
        text.push('\n');
    }
    for card in models.cards() {
        text.push('\n');
        text.push_str(&card);
        text.push('\n');
    }
    if !sim.extra.trim().is_empty() {
        text.push('\n');
        text.push_str(sim.extra.trim_end());
        text.push('\n');
    }
    text.push('\n');
    if let Some(l) = analysis_line {
        text.push_str(&l);
        text.push('\n');
    }
    text.push_str(".end\n");
    b.out.text = text;
    b.out.problems.sort_by_key(|p| p.level);
    b.out
}

/// A transient length that shows a few periods of the slowest periodic source: five
/// periods, else 10 ms.
pub fn suggested_stop(project: &Project) -> f64 {
    project
        .schematic
        .components
        .iter()
        .filter_map(|c| c.sim.source.as_ref()?.waveform.period())
        .fold(None, |m: Option<f64>, p| Some(m.map_or(p, |m| m.max(p))))
        .map_or(10e-3, |p| 5.0 * p)
}

fn one_line(s: &str) -> String {
    s.lines().next().unwrap_or("").trim().to_string()
}

struct Builder<'a> {
    project: &'a Project,
    overrides: &'a Overrides,
    out: Netlist,
    used_nodes: HashSet<String>,
    used_devices: HashSet<String>,
}

impl Builder<'_> {
    fn problem(&mut self, level: Level, message: impl Into<String>, objects: Vec<ObjectId>) {
        self.out.problems.push(SimProblem { level, message: message.into(), objects });
    }

    /// Symbols of a component (to point at it).
    fn symbols_of(&self, component: Id) -> Vec<ObjectId> {
        self.project
            .schematic
            .symbols
            .iter()
            .filter(|s| s.component == component)
            .map(|s| ObjectId::Symbol(s.id))
            .collect()
    }

    fn unique_node(&mut self, base: &str) -> String {
        let base = if base.is_empty() { "n".to_string() } else { base.to_string() };
        let mut name = base.clone();
        let mut k = 2;
        while !self.used_nodes.insert(name.clone()) {
            name = format!("{base}_{k}");
            k += 1;
        }
        name
    }

    fn nodes(&mut self, conn: &Connectivity) {
        let sch = &self.project.schematic;
        let sim = &sch.simulation;
        let mut ground: Vec<String> = Vec::new();
        match &sim.ground {
            Some(name) => {
                if conn.net(name).is_some() {
                    ground.push(name.clone());
                } else {
                    self.problem(Level::Error, format!("The ground net {name} doesn't exist"), Vec::new());
                }
            }
            None => {
                for l in sch.labels.iter().filter(|l| l.kind == LabelKind::Power) {
                    if matches!(l.power_style(), PowerStyle::Ground | PowerStyle::SignalGround)
                        && let Some(net) = conn.net_of(ObjectId::Label(l.id))
                        && !ground.contains(&net.name)
                    {
                        ground.push(net.name.clone());
                    }
                }
                for net in conn.netlist() {
                    if (net.name == "0" || net.name.eq_ignore_ascii_case("GND")) && !ground.contains(&net.name) {
                        ground.push(net.name.clone());
                    }
                }
            }
        }
        if ground.len() > 1 {
            ground.sort_by(|a, b| natural_cmp(a, b));
            self.problem(Level::Note, format!("Ground nets joined as node 0: {}", ground.join(", ")), Vec::new());
        }
        for net in conn.netlist() {
            let node =
                if ground.contains(&net.name) { "0".to_string() } else { self.unique_node(&node_name(&net.name)) };
            self.out.nodes.insert(net.name.clone(), node);
        }
        if ground.is_empty() && sim.ground.is_none() && conn.netlist().next().is_some() {
            self.problem(Level::Error, "No ground: place a GND power port (or choose the ground net)", Vec::new());
        }
    }

    fn devices(&mut self, conn: &Connectivity, analysis: &Analysis, lines: &mut Vec<String>, models: &mut Models) {
        let project = self.project;
        let sch = &project.schematic;
        let op = matches!(analysis.kind, AnalysisKind::Op);
        let probed: HashSet<(Id, Id)> = analysis
            .probes
            .iter()
            .filter_map(|p| match p {
                Probe::Current { symbol, pin } => Some((*symbol, *pin)),
                _ => None,
            })
            .collect();
        let powered: HashSet<Id> = analysis
            .probes
            .iter()
            .filter_map(|p| if let Probe::Power { component } = p { Some(*component) } else { None })
            .collect();
        let mut unit_symbol: HashMap<(Id, u32), Id> = HashMap::new();
        for s in sch.symbols.iter() {
            unit_symbol.entry((s.component, s.unit)).or_insert(s.id);
        }
        let mut components: Vec<&Component> = sch.components.iter().collect();
        components.sort_by(|a, b| natural_cmp(&a.refdes, &b.refdes));
        let mut no_model = Vec::new();
        let mut simulated = HashSet::new();
        for c in components {
            let c = self.overrides.component(c);
            let Some(part) = project.library.parts.get(c.part) else { continue };
            if part.is_board_part() || c.sim.exclude {
                continue;
            }
            let Some(model) = &part.sim else {
                no_model.push(c);
                continue;
            };
            if model.device == SimDevice::Ignore {
                continue;
            }
            simulated.insert(c.id);
            let objects = self.symbols_of(c.id);
            // The pins, in node order.
            let model_pins = project.library.sim_pins(part);
            let name = self.device_name(model, c);
            let mut nodes = Vec::with_capacity(model_pins.len());
            let mut sense = Vec::new();
            let mut pins = Vec::new();
            for (k, sp) in model_pins.iter().enumerate() {
                let symbol = unit_symbol.get(&(c.id, sp.unit)).copied();
                let net = symbol.and_then(|s| conn.net_of_pin(s, sp.pin));
                let mut node = match net.and_then(|n| self.out.nodes.get(&n.name)) {
                    Some(n) => n.clone(),
                    None => self.unique_node(&format!("nc_{}_{}", node_name(&c.refdes), k + 1)),
                };
                let key = pin_key(project, part, *sp);
                pins.push(DevicePin {
                    component: c.id,
                    refdes: c.refdes.clone(),
                    symbol,
                    pin: sp.pin,
                    key: key.clone(),
                    node: node.clone(),
                });
                if let Some(s) = symbol
                    && (op || powered.contains(&c.id) || probed.contains(&(s, sp.pin)))
                {
                    let source = self.unique_device(&format!("VSENSE_{}_{}", sanitize(&c.refdes), sanitize(&key)));
                    let inner = self.unique_node(&format!("{}_{}", node_name(&source), "in"));
                    sense.push(format!("{source} {node} {inner} 0"));
                    self.out.sense.insert((s, sp.pin), source.to_ascii_lowercase());
                    node = inner;
                }
                nodes.push(node);
            }
            match self.device_line(c, model, &name, &nodes, models) {
                Ok(line) => {
                    lines.extend(sense);
                    lines.push(line);
                    self.out.devices.insert(c.id, name);
                    self.out.pins.extend(pins);
                }
                Err(message) => {
                    for p in pins.iter().filter_map(|p| p.symbol.map(|s| (s, p.pin))) {
                        self.out.sense.remove(&p);
                    }
                    self.problem(Level::Error, format!("{}: {message}", c.refdes), objects)
                }
            }
        }
        if !no_model.is_empty() {
            let refs: Vec<&str> = no_model.iter().map(|c| c.refdes.as_str()).collect();
            let objects = no_model.iter().flat_map(|c| self.symbols_of(c.id)).collect();
            self.problem(Level::Warning, format!("Not simulated, no model: {}", refs.join(", ")), objects);
        }
        for p in &analysis.probes {
            if let Probe::Power { component } = p
                && !simulated.contains(component)
            {
                let what = sch.components.get(*component).map_or("a deleted part", |c| c.refdes.as_str());
                let objects = self.symbols_of(*component);
                self.problem(Level::Warning, format!("Power probe on {what}: it is not simulated"), objects);
            }
            if let Probe::Current { symbol, pin } = p
                && !self.out.sense.contains_key(&(*symbol, *pin))
            {
                let what = sch
                    .symbols
                    .get(*symbol)
                    .and_then(|s| sch.components.get(s.component))
                    .map_or("a deleted symbol".to_string(), |c| c.refdes.clone());
                let why = if sch.symbols.get(*symbol).is_some_and(|s| simulated.contains(&s.component)) {
                    "the pin is not part of its model"
                } else {
                    "it is not simulated"
                };
                self.problem(
                    Level::Warning,
                    format!("Current probe on {what}: {why}"),
                    vec![ObjectId::Symbol(*symbol)],
                );
            }
        }
    }

    fn unique_device(&mut self, base: &str) -> String {
        let mut name = base.to_string();
        let mut k = 2;
        while !self.used_devices.insert(name.to_ascii_lowercase()) {
            name = format!("{base}_{k}");
            k += 1;
        }
        name
    }

    /// The SPICE letter, then the reference unless it starts with that letter.
    fn device_name(&mut self, model: &SimModel, c: &Component) -> String {
        let letter = match &model.device {
            SimDevice::Resistor => Some('R'),
            SimDevice::Capacitor => Some('C'),
            SimDevice::Inductor => Some('L'),
            SimDevice::Primitive { kind, .. } => Some(kind.letter()),
            SimDevice::Subckt { .. } => Some('X'),
            SimDevice::VoltageSource => Some('V'),
            SimDevice::CurrentSource => Some('I'),
            SimDevice::Spice { .. } | SimDevice::Ignore => None,
        };
        let r = sanitize(&c.refdes);
        let base = match letter {
            Some(l) if !r.starts_with(l) && !r.starts_with(l.to_ascii_lowercase()) => format!("{l}{r}"),
            _ => r,
        };
        self.unique_device(&base)
    }

    fn device_line(
        &mut self,
        c: &Component,
        model: &SimModel,
        name: &str,
        nodes: &[String],
        models: &mut Models,
    ) -> Result<String, String> {
        let want = |n: usize| {
            if nodes.len() == n {
                Ok(())
            } else {
                Err(format!("the model needs {n} pins, the part maps {}", nodes.len()))
            }
        };
        let mut params = model.params.clone();
        params.extend(c.sim.params.iter().map(|(k, v)| (k.clone(), v.clone())));
        let params = params
            .iter()
            .map(|(k, v)| Ok((k.clone(), param_value(v, &c.value)?)))
            .collect::<Result<Vec<(String, String)>, String>>()?;
        let n = nodes.join(" ");
        let extra = params.iter().map(|(k, v)| format!(" {k}={v}")).collect::<String>();
        Ok(match &model.device {
            SimDevice::Resistor | SimDevice::Capacitor | SimDevice::Inductor => {
                want(2)?;
                let v = si::parse(&c.value)
                    .ok_or_else(|| format!("the value {:?} is not a number (like 10k or 100n)", c.value))?;
                format!("{name} {n} {}{extra}", si::spice(v))
            }
            SimDevice::Primitive { kind, model: card } => {
                want(kind.ports().len())?;
                let card = ModelCard::parse(card).ok_or("the model text has no .model line")?;
                let model_name = models.primitive(card, &params, name);
                // MOSFETs: VDMOS has three nodes, the others a bulk tied to the source.
                let bulk = if kind.letter() == 'M' && !models.is_vdmos(&model_name) {
                    format!(" {}", nodes[2])
                } else {
                    String::new()
                };
                format!("{name} {n}{bulk} {model_name}")
            }
            SimDevice::Subckt { text } => {
                let sub = Subckt::parse(text).ok_or("the model text has no .subckt line")?;
                want(sub.ports)?;
                let sub_name = models.subckt(sub);
                format!("{name} {n} {sub_name}{extra}")
            }
            SimDevice::VoltageSource | SimDevice::CurrentSource => {
                want(2)?;
                format!("{name} {n} {}", source_spec(c.sim.source.as_ref())?)
            }
            SimDevice::Spice { template } => {
                let mut line = template.replace("${REF}", &sanitize(&c.refdes)).replace("${VALUE}", c.value.trim());
                for (k, node) in nodes.iter().enumerate().rev() {
                    line = line.replace(&format!("${{N{}}}", k + 1), node);
                }
                line
            }
            SimDevice::Ignore => unreachable!(),
        })
    }

    fn supplies(&mut self, lines: &mut Vec<String>) {
        let supplies = self.project.schematic.simulation.supplies.clone();
        for s in supplies {
            match self.out.nodes.get(&s.net).cloned() {
                Some(node) if node == "0" => {
                    self.problem(Level::Error, format!("Supply {}: the net is ground", s.net), Vec::new())
                }
                Some(node) => {
                    let name = self.unique_device(&format!("VSUPPLY_{}", sanitize(&node).to_ascii_uppercase()));
                    lines.push(format!("{name} {node} 0 DC {}", si::spice(s.voltage)));
                }
                None => self.problem(Level::Warning, format!("Supply {}: no such net", s.net), Vec::new()),
            }
        }
    }

    fn analysis_line(&mut self, analysis: &Analysis) -> Option<String> {
        let s = si::spice;
        Some(match &analysis.kind {
            AnalysisKind::Op => ".op".into(),
            AnalysisKind::Dc { source, start, stop, step } => {
                let sch = &self.project.schematic;
                let is_source = sch
                    .components
                    .get(*source)
                    .and_then(|c| self.project.library.parts.get(c.part))
                    .and_then(|p| p.sim.as_ref())
                    .is_some_and(|m| matches!(m.device, SimDevice::VoltageSource | SimDevice::CurrentSource));
                let Some(name) = self.out.devices.get(source).filter(|_| is_source).cloned() else {
                    let objects = self.symbols_of(*source);
                    self.problem(Level::Error, "DC sweep: the swept component is not a simulated source", objects);
                    return None;
                };
                format!(".dc {name} {} {} {}", s(*start), s(*stop), s(*step))
            }
            AnalysisKind::Ac { start, stop, points_per_decade } => {
                format!(".ac dec {points_per_decade} {} {}", s(*start), s(*stop))
            }
            AnalysisKind::Tran { stop, step, start, max_step } => {
                let step = step.unwrap_or(stop / 1000.0);
                let mut l = format!(".tran {} {}", s(step), s(*stop));
                if *start != 0.0 || max_step.is_some() {
                    l.push_str(&format!(" {}", s(*start)));
                }
                if let Some(m) = max_step {
                    l.push_str(&format!(" {}", s(*m)));
                }
                l
            }
        })
    }
}

/// The pin's number, else its name.
fn pin_key(project: &Project, part: &Part, sp: SimPin) -> String {
    part.units
        .get(sp.unit as usize)
        .and_then(|u| project.library.symbols.get(u.symbol))
        .and_then(|s| s.pins.get(sp.pin))
        .map_or_else(|| "pin".into(), |p| p.key().to_string())
}

/// Letters, digits and `_` only.
fn sanitize(s: &str) -> String {
    s.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }).collect()
}

/// A readable node name for a net: lower case, `+` and `-` in front as `p` and `n`,
/// other symbols as `_` (runs collapsed, none at the ends); a leading digit gets `n`.
pub fn node_name(net: &str) -> String {
    let t = net.trim().trim_start_matches('/');
    let mut out = String::new();
    for (i, c) in t.chars().enumerate() {
        match c {
            c if c.is_ascii_alphanumeric() => out.push(c.to_ascii_lowercase()),
            '+' if i == 0 => out.push('p'),
            '-' if i == 0 => out.push('n'),
            _ => {
                if !out.ends_with('_') {
                    out.push('_')
                }
            }
        }
    }
    let out = out.trim_matches('_').to_string();
    if out.is_empty() {
        "n".into()
    } else if out.starts_with(|c: char| c.is_ascii_digit()) {
        format!("n{out}")
    } else {
        out
    }
}

/// A parameter value for the netlist: `${VALUE}` replaced by the component's value; a
/// number with an SI prefix written as a plain number, other text as typed.
fn param_value(v: &str, value: &str) -> Result<String, String> {
    if v.contains("${VALUE}") {
        let number =
            value_number(value).ok_or_else(|| format!("no number in the value {value:?} for the model parameter"))?;
        return Ok(v.replace("${VALUE}", &si::spice(number)));
    }
    Ok(si::parse(v).map_or_else(|| v.trim().to_string(), si::spice))
}

/// The number in a component value: the value itself (`5V1`, `4.7`), else a voltage in a
/// part number (`BZT52C3V3` → 3.3).
pub fn value_number(value: &str) -> Option<f64> {
    if let Some(v) = si::parse(value) {
        return Some(v);
    }
    let b = value.as_bytes();
    for (i, &c) in b.iter().enumerate() {
        if c == b'V' && i > 0 && b[i - 1].is_ascii_digit() {
            let start = (0..i).rev().take_while(|&k| b[k].is_ascii_digit() || b[k] == b'.').last().unwrap_or(i);
            let end = (i + 1..b.len()).take_while(|&k| b[k].is_ascii_digit()).last().map_or(i + 1, |k| k + 1);
            if let Some(v) = si::parse(&value[start..end]) {
                return Some(v);
            }
        }
    }
    None
}

/// The source part of a V/I line: `DC 0 AC 1 0 SIN(…)`.
fn source_spec(source: Option<&Source>) -> Result<String, String> {
    let default = Source::default();
    let source = source.unwrap_or(&default);
    let s = si::spice;
    let mut spec = format!("DC {}", s(source.waveform.initial()));
    if source.ac != 0.0 {
        spec.push_str(&format!(" AC {} {}", s(source.ac), s(source.ac_phase)));
    }
    let list = |v: &[f64]| v.iter().map(|&x| s(x)).collect::<Vec<_>>().join(" ");
    match &source.waveform {
        Waveform::Dc { .. } => {}
        Waveform::Sine { offset, amplitude, frequency, delay, damping, phase } => {
            spec.push_str(&format!(" SIN({})", list(&[*offset, *amplitude, *frequency, *delay, *damping, *phase])));
        }
        Waveform::Pulse { low, high, delay, rise, fall, width, period } => {
            spec.push_str(&format!(" PULSE({})", list(&[*low, *high, *delay, *rise, *fall, *width, *period])));
        }
        Waveform::Pwl { points, repeat } => {
            if points.is_empty() {
                return Err("the piecewise linear source has no points".into());
            }
            if points.windows(2).any(|w| w[1].0 <= w[0].0) {
                return Err("the piecewise linear points must have increasing times".into());
            }
            let flat: Vec<f64> = points.iter().flat_map(|&(t, v)| [t, v]).collect();
            spec.push_str(&format!(" PWL({})", list(&flat)));
            if *repeat {
                spec.push_str(" r=0");
            }
        }
        Waveform::Exp { v1, v2, delay1, tau1, delay2, tau2 } => {
            spec.push_str(&format!(" EXP({})", list(&[*v1, *v2, *delay1, *tau1, *delay2, *tau2])));
        }
    }
    Ok(spec)
}

/// A `.subckt NAME ports…` definition.
#[derive(Clone, Debug, PartialEq)]
struct Subckt {
    name: String,
    ports: usize,
    text: String,
}

impl Subckt {
    fn parse(text: &str) -> Option<Subckt> {
        let h = SubcktHeader::parse(text)?;
        Some(Subckt { name: h.name, ports: h.ports.len(), text: text.trim().to_string() })
    }

    /// The text with the subcircuit renamed (its `.subckt` and `.ends` lines).
    fn renamed(&self, name: &str) -> String {
        self.text
            .lines()
            .map(|l| {
                let lower = l.trim_start().to_ascii_lowercase();
                if lower.starts_with(".subckt") || lower.starts_with(".ends") {
                    let mut t: Vec<String> = l.split_whitespace().map(str::to_string).collect();
                    if t.len() > 1 && t[1].eq_ignore_ascii_case(&self.name) {
                        t[1] = name.to_string();
                    }
                    t.join(" ")
                } else {
                    l.to_string()
                }
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
}

/// The `.model` cards and `.subckt`s of a netlist, each written once; same names with
/// different text are renamed (`NAME_2`).
#[derive(Default)]
struct Models {
    cards: Vec<ModelCard>,
    subckts: Vec<(String, String)>,
}

impl Models {
    fn unique(&self, base: &str) -> String {
        let taken = |n: &str| {
            self.cards.iter().any(|c| c.name.eq_ignore_ascii_case(n))
                || self.subckts.iter().any(|(s, _)| s.eq_ignore_ascii_case(n))
        };
        let mut name = base.to_string();
        let mut k = 2;
        while taken(&name) {
            name = format!("{base}_{k}");
            k += 1;
        }
        name
    }

    /// The model name to use: the card's, or a copy with the overrides named after the
    /// device.
    fn primitive(&mut self, card: ModelCard, overrides: &[(String, String)], device: &str) -> String {
        let card = if overrides.is_empty() {
            card
        } else {
            let name = format!("{}_{device}", card.name);
            ModelCard { name, ..card.with(overrides) }
        };
        if let Some(c) = self.cards.iter().find(|c| c.name.eq_ignore_ascii_case(&card.name)) {
            if c.params == card.params && c.kind.eq_ignore_ascii_case(&card.kind) {
                return c.name.clone();
            }
            let name = self.unique(&card.name);
            self.cards.push(ModelCard { name: name.clone(), ..card });
            return name;
        }
        let name = card.name.clone();
        self.cards.push(card);
        name
    }

    fn is_vdmos(&self, name: &str) -> bool {
        self.cards.iter().any(|c| c.name == name && c.kind.eq_ignore_ascii_case("vdmos"))
    }

    fn subckt(&mut self, sub: Subckt) -> String {
        if let Some((n, _)) = self.subckts.iter().find(|(_, t)| *t == sub.text) {
            return n.clone();
        }
        let name = self.unique(&sub.name);
        let text = if name == sub.name { sub.text.clone() } else { sub.renamed(&name) };
        self.subckts.push((name.clone(), text));
        name
    }

    fn cards(&self) -> Vec<String> {
        self.cards.iter().map(ModelCard::text).chain(self.subckts.iter().map(|(_, t)| t.clone())).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn node_names() {
        assert_eq!(node_name("/OUT"), "out");
        assert_eq!(node_name("+5V"), "p5v");
        assert_eq!(node_name("-12V"), "n12v");
        assert_eq!(node_name("Net-(R1-1)"), "net_r1_1");
        assert_eq!(node_name("/Power/A"), "power_a");
        assert_eq!(node_name("3V3"), "n3v3");
        assert_eq!(node_name("///"), "n");
    }

    #[test]
    fn subckt_ports() {
        let s = Subckt::parse(
            "* op amp\n.subckt OPAMP inp inn out vp vn params: gain=1e5\nE1 out 0 inp inn 1\n.ends OPAMP",
        )
        .unwrap();
        assert_eq!((s.name.as_str(), s.ports), ("OPAMP", 5));
        assert!(s.renamed("OPAMP_2").contains(".subckt OPAMP_2 inp"));
        assert!(s.renamed("OPAMP_2").contains(".ends OPAMP_2"));
    }

    #[test]
    fn value_numbers() {
        assert_eq!(value_number("5V1"), Some(5.1));
        assert_eq!(value_number("BZT52C3V3"), Some(3.3));
        assert_eq!(value_number("BZX84C12V"), Some(12.0));
        assert_eq!(value_number("red"), None);
    }
}
