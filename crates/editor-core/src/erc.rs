//! Electrical rules on top of connectivity: pin kinds that fight or go undriven, wired
//! pins of kind "not connected" and reused reference designators. The findings join the
//! schematic's [`Problem`]s.
//!
//! Pin kinds, as in KiCad's default matrix (simplified):
//! - Drivers: output and power output drive hard; tri-state and open collector share a
//!   net but not with a hard driver. Two hard drivers (or one with a tri-state or open
//!   collector pin) conflict.
//! - An input needs some other pin that can drive it (anything but inputs, power inputs
//!   and "not connected"), or a power port.
//! - A power input needs a power output pin or a power port (power ports are KiCad's
//!   power symbols, whose pin is a power output).

use rustc_hash::FxHashMap as HashMap;

use model::*;

use crate::connectivity::{Net, NetPin, Problem};

/// Problems of `nets` (pins, wires and labels already grouped).
pub fn check(project: &Project, nets: &[Net]) -> Vec<Problem> {
    let mut problems = Vec::new();
    for net in nets {
        check_net(project, net, &mut problems);
    }
    duplicate_references(project, &mut problems);
    problems
}

/// "R1-3", with the pin name when it says more: "U1-14 (VCC)".
fn pin_text(p: &NetPin) -> String {
    if p.pin_name.is_empty() || p.pin_name == p.pin_number {
        format!("{}-{}", p.refdes, p.pin_number)
    } else {
        format!("{}-{} ({})", p.refdes, p.pin_number, p.pin_name)
    }
}

fn list(pins: &[&NetPin]) -> String {
    pins.iter().map(|p| pin_text(p)).collect::<Vec<_>>().join(", ")
}

fn problem(message: String, pins: &[&NetPin]) -> Problem {
    let mut objects: Vec<ObjectId> = Vec::new();
    for p in pins {
        let o = ObjectId::Symbol(p.symbol);
        if !objects.contains(&o) {
            objects.push(o);
        }
    }
    Problem { message, objects, pos: pins.first().map(|p| p.pos) }
}

fn check_net(project: &Project, net: &Net, problems: &mut Vec<Problem>) {
    if net.pins.is_empty() {
        return;
    }
    let wired = net.pins.len() > 1
        || !net.wires.is_empty()
        || !net.labels.is_empty()
        || !net.junctions.is_empty()
        || !net.entries.is_empty()
        || !net.sheet_pins.is_empty();
    // A lone pin is unconnected or flagged; connectivity reports it.
    if !wired {
        return;
    }
    let of = |kinds: &[PinKind]| -> Vec<&NetPin> { net.pins.iter().filter(|p| kinds.contains(&p.kind)).collect() };

    let nc = of(&[PinKind::NotConnected]);
    if !nc.is_empty() {
        let message = if nc.len() == 1 {
            format!("Pin {} is of kind \"not connected\" but is wired", list(&nc))
        } else {
            format!("Pins {} are of kind \"not connected\" but are wired", list(&nc))
        };
        problems.push(problem(message, &nc));
    }

    let hard = of(&[PinKind::Output, PinKind::PowerOut]);
    let shared = of(&[PinKind::TriState, PinKind::OpenCollector]);
    if hard.len() > 1 || (hard.len() == 1 && !shared.is_empty()) {
        let pins: Vec<&NetPin> = hard.iter().chain(&shared).copied().collect();
        problems.push(problem(format!("Outputs drive against each other: {}", list(&pins)), &pins));
    }

    let power_port =
        net.labels.iter().filter_map(|&l| project.schematic.labels.get(l)).any(|l| l.kind == LabelKind::Power);
    let inputs = of(&[PinKind::Input]);
    let drivers = of(&[
        PinKind::Output,
        PinKind::PowerOut,
        PinKind::Bidirectional,
        PinKind::TriState,
        PinKind::OpenCollector,
        PinKind::Passive,
    ]);
    if !inputs.is_empty() && drivers.is_empty() && !power_port {
        let message = if inputs.len() == 1 {
            format!("Input {} is not driven", list(&inputs))
        } else {
            format!("Inputs {} are not driven", list(&inputs))
        };
        problems.push(problem(message, &inputs));
    }

    let power_in = of(&[PinKind::PowerIn]);
    if !power_in.is_empty() && !power_port && of(&[PinKind::PowerOut]).is_empty() {
        let pins = list(&power_in);
        let message = if power_in.len() == 1 {
            format!("Power input {pins} has no power source (power port or power output)")
        } else {
            format!("Power inputs {pins} have no power source (power port or power output)")
        };
        problems.push(problem(message, &power_in));
    }
}

/// Components sharing a reference designator (unannotated ones, "R?", aside).
fn duplicate_references(project: &Project, problems: &mut Vec<Problem>) {
    let s = &project.schematic;
    let mut by_refdes: HashMap<&str, Vec<Id>> = HashMap::default();
    let mut order: Vec<&str> = Vec::new();
    for c in s.components.iter() {
        if c.refdes.is_empty() || c.refdes.ends_with('?') {
            continue;
        }
        let ids = by_refdes.entry(c.refdes.as_str()).or_default();
        if ids.is_empty() {
            order.push(c.refdes.as_str());
        }
        ids.push(c.id);
    }
    order.sort_by(|a, b| natural_cmp(a, b));
    for refdes in order {
        let ids = &by_refdes[refdes];
        if ids.len() < 2 {
            continue;
        }
        let symbols: Vec<&SymbolInstance> = s.symbols.iter().filter(|i| ids.contains(&i.component)).collect();
        let objects = symbols.iter().map(|i| ObjectId::Symbol(i.id)).collect();
        let message = format!("Reference {refdes} is used by {} parts", ids.len());
        problems.push(Problem { message, objects, pos: symbols.first().map(|i| i.pos) });
    }
}
