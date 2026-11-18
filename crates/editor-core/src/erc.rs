//! Electrical rules on top of connectivity: pin kinds that conflict or go undriven, and
//! reused reference designators. The findings join the schematic's [`Problem`]s; their
//! severities and exclusions are applied afterwards (`Connectivity::apply_settings`).
//!
//! - Pin conflicts: every pair of pin kinds on a wired net is looked up in the pin
//!   conflict matrix ([`ErcSettings::pin_pair`], KiCad's defaults); one finding per
//!   conflicting pair of kinds, listing the pins.
//! - An input needs some other pin that can drive it (anything but inputs, power inputs
//!   and "not connected"), a power port or a power flag.
//! - A power input needs a power output pin, a power port or a power flag. Power ports
//!   and power flags are no pins, so they take no part in the matrix.

use rustc_hash::FxHashMap as HashMap;

use model::*;

use crate::connectivity::{Net, NetPin, Problem, pin_part};

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

/// A finding about `pins`, keyed by them.
fn problem(rule: ErcRule, message: String, pins: &[&NetPin]) -> Problem {
    let mut objects: Vec<ObjectId> = Vec::new();
    for p in pins {
        let o = ObjectId::Symbol(p.symbol);
        if !objects.contains(&o) {
            objects.push(o);
        }
    }
    let parts: Vec<String> = pins.iter().map(|p| pin_part(p.symbol, &p.pin_number)).collect();
    Problem::new(rule, message, objects, pins.first().map(|p| p.pos)).keyed(&parts)
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
    let erc = &project.schematic.erc;
    let of = |kinds: &[PinKind]| -> Vec<&NetPin> { net.pins.iter().filter(|p| kinds.contains(&p.kind)).collect() };

    for (i, &a) in PIN_KINDS.iter().enumerate() {
        let pa = of(&[a]);
        if pa.is_empty() {
            continue;
        }
        for &b in &PIN_KINDS[i..] {
            let severity = erc.pin_pair(a, b);
            if severity == Severity::Ignore {
                continue;
            }
            let pins: Vec<&NetPin> = if a == b {
                if pa.len() < 2 {
                    continue;
                }
                pa.clone()
            } else {
                let pb = of(&[b]);
                if pb.is_empty() {
                    continue;
                }
                pa.iter().chain(&pb).copied().collect()
            };
            let what = format!("{} – {}", a.label().to_lowercase(), b.label().to_lowercase());
            let mut p = problem(ErcRule::PinConflict, format!("Pin conflict ({what}): {}", list(&pins)), &pins);
            p.severity = severity;
            problems.push(p);
        }
    }

    let powered = !net.power_flags.is_empty()
        || net.labels.iter().filter_map(|&l| project.schematic.labels.get(l)).any(|l| l.kind == LabelKind::Power);
    let inputs = of(&[PinKind::Input]);
    let drivers = of(&[
        PinKind::Output,
        PinKind::PowerOut,
        PinKind::Bidirectional,
        PinKind::TriState,
        PinKind::OpenCollector,
        PinKind::Passive,
    ]);
    if !inputs.is_empty() && drivers.is_empty() && !powered {
        let message = if inputs.len() == 1 {
            format!("Input {} is not driven", list(&inputs))
        } else {
            format!("Inputs {} are not driven", list(&inputs))
        };
        problems.push(problem(ErcRule::UndrivenInput, message, &inputs));
    }

    let power_in = of(&[PinKind::PowerIn]);
    if !power_in.is_empty() && !powered && of(&[PinKind::PowerOut]).is_empty() {
        let pins = list(&power_in);
        let message = if power_in.len() == 1 {
            format!("Power input {pins} has no power source (power port, power flag or power output)")
        } else {
            format!("Power inputs {pins} have no power source (power port, power flag or power output)")
        };
        problems.push(problem(ErcRule::PowerInputUnpowered, message, &power_in));
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
        problems.push(Problem::new(ErcRule::DuplicateReference, message, objects, symbols.first().map(|i| i.pos)));
    }
}
