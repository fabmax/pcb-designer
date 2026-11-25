//! Simulation on the schematic: what the Probe tool would probe under the pointer, and
//! the overlay of a simulation: probed nets' wires in their trace colour, arrows on
//! probed pins, and operating point values (net voltages, the currents of two-pin
//! parts). The app decides what to show; this draws it.

use editor_core::connectivity::Connectivity;
use editor_core::query::symbol_of_instance;
use model::*;
use render::font::{self, HAlign as FH, VAlign as FV};
use render::{Color, Instance};

use crate::draw::{Painter, mmf};
use crate::style::WIRE_WIDTH;
use crate::style::pal;
use crate::view::View;

/// What a click with the Probe tool measures.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProbeTarget {
    /// A net's voltage, found by the wire, label or junction clicked.
    Net { net: String, at: ObjectId },
    /// The voltage of a net to another (a drag from one to the other).
    Diff { net: String, at: ObjectId, reference: String, ref_at: ObjectId },
    /// The current into a pin (its body was clicked, not its end).
    Pin { symbol: Id, pin: Id },
    /// The power a part takes (Alt-click on its body).
    Part { component: Id },
}

impl ProbeTarget {
    pub fn probe(&self) -> Probe {
        match self {
            ProbeTarget::Net { net, at } => Probe::voltage(net.clone(), Some(*at)),
            ProbeTarget::Diff { net, at, reference, ref_at } => Probe::Voltage {
                net: net.clone(),
                at: Some(*at),
                reference: Some(NetRef { net: reference.clone(), at: Some(*ref_at) }),
                logic: false,
            },
            ProbeTarget::Pin { symbol, pin } => Probe::Current { symbol: *symbol, pin: *pin },
            ProbeTarget::Part { component } => Probe::Power { component: *component },
        }
    }

    /// "Voltage of /OUT", "Voltage of OUT to IN", "Current into R1 pin 1", "Power of R1".
    pub fn describe(&self, project: &Project) -> String {
        match self {
            ProbeTarget::Net { net, .. } => format!("Voltage of {net}"),
            ProbeTarget::Diff { net, reference, .. } => format!("Voltage of {net} to {reference}"),
            ProbeTarget::Pin { symbol, pin } => {
                let (refdes, key) = pin_name(project, *symbol, *pin).unwrap_or_else(|| ("?".into(), "?".into()));
                format!("Current into {refdes} pin {key}")
            }
            ProbeTarget::Part { component } => {
                let refdes = project.schematic.components.get(*component).map_or("?", |c| c.refdes.as_str());
                format!("Power of {refdes}")
            }
        }
    }
}

/// The reference of a pin's part and the pin's number (else name).
pub fn pin_name(project: &Project, symbol: Id, pin: Id) -> Option<(String, String)> {
    let inst = project.schematic.symbols.get(symbol)?;
    let c = project.schematic.components.get(inst.component)?;
    let p = symbol_of_instance(project, inst)?.pins.get(pin)?;
    Some((c.refdes.clone(), p.key().to_string()))
}

/// The outer and inner end of a placed pin.
fn pin_ends(project: &Project, symbol: Id, pin: Id) -> Option<(Point, Point)> {
    let inst = project.schematic.symbols.get(symbol)?;
    let p = symbol_of_instance(project, inst)?.pins.get(pin)?;
    let t = inst.transform();
    Some((t.point(p.pos), t.point(p.inner_end())))
}

/// What the Probe tool measures at `p` on `view`'s sheet: a wire, label or junction's
/// net, else a pin whose body (not its end) is within `tol`. With `power`, the part
/// under `p` (its power).
pub fn probe_target(
    project: &Project,
    conn: &Connectivity,
    view: &View,
    p: Point,
    tol: Coord,
    power: bool,
) -> Option<ProbeTarget> {
    if power {
        let hit = view.hit(project, p, tol).filter(|id| matches!(id, ObjectId::Symbol(_)));
        let hit = hit.or_else(|| view.index.near(p, 0).into_iter().find(|id| matches!(id, ObjectId::Symbol(_))));
        return match hit {
            Some(ObjectId::Symbol(s)) => {
                project.schematic.symbols.get(s).map(|s| ProbeTarget::Part { component: s.component })
            }
            _ => None,
        };
    }
    if let Some(at @ (ObjectId::Wire(_) | ObjectId::Label(_) | ObjectId::Junction(_))) = view.hit(project, p, tol)
        && let Some(net) = conn.net_of(at)
    {
        return Some(ProbeTarget::Net { net: net.name.clone(), at });
    }
    let mut best: Option<(f64, ProbeTarget)> = None;
    for id in view.index.near(p, tol) {
        let ObjectId::Symbol(symbol) = id else { continue };
        let Some(inst) = project.schematic.symbols.get(symbol) else { continue };
        let Some(sym) = symbol_of_instance(project, inst) else { continue };
        let t = inst.transform();
        for pin in sym.pins.iter() {
            let (a, b) = (t.point(pin.pos), t.point(pin.inner_end()));
            let d = seg_dist(p, a, b);
            if d <= tol as f64 && (p - a).length() > tol as f64 && best.as_ref().is_none_or(|(bd, _)| d < *bd) {
                best = Some((d, ProbeTarget::Pin { symbol, pin: pin.id }));
            }
        }
    }
    best.map(|(_, t)| t)
}

fn seg_dist(p: Point, a: Point, b: Point) -> f64 {
    let (ax, ay, bx, by, px, py) = (a.x as f64, a.y as f64, b.x as f64, b.y as f64, p.x as f64, p.y as f64);
    let (dx, dy) = (bx - ax, by - ay);
    let len2 = dx * dx + dy * dy;
    let t = if len2 == 0.0 { 0.0 } else { (((px - ax) * dx + (py - ay) * dy) / len2).clamp(0.0, 1.0) };
    ((ax + t * dx - px).powi(2) + (ay + t * dy - py).powi(2)).sqrt()
}

/// What the schematic shows of a simulation.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SimOverlay {
    /// Probed nets and their trace colours.
    pub nets: Vec<(String, Color)>,
    /// Probed pins (symbol, pin) and their trace colours.
    pub pins: Vec<(Id, Id, Color)>,
    /// Operating point voltages: net → text ("4.98 V"). One tag per net and sheet.
    pub voltages: Vec<(String, String)>,
    /// Currents of two-pin parts: the pin the current flows into and the magnitude.
    pub currents: Vec<(Id, Id, String)>,
    /// The values are out of date: drawn faint.
    pub dim: bool,
    /// Parts whose power is probed: their outline in the trace colour.
    pub parts: Vec<(Id, Color)>,
    /// Nets, pins and parts to mark (the Probe tool or a table row hovers them).
    pub hover_nets: Vec<String>,
    pub hover_pins: Vec<(Id, Id)>,
    pub hover_parts: Vec<Id>,
}

impl SimOverlay {
    pub fn is_empty(&self) -> bool {
        self.nets.is_empty()
            && self.pins.is_empty()
            && self.voltages.is_empty()
            && self.currents.is_empty()
            && self.parts.is_empty()
            && self.hover_nets.is_empty()
            && self.hover_pins.is_empty()
            && self.hover_parts.is_empty()
    }
}

const TAG_HEIGHT: Coord = 800_000;

/// Where the overlay draws: probes and values on the schematic's Simulation layer (which
/// the user can hide), the marks of hovered nets, pins and parts on the hover layer.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Layers {
    pub sim: render::LayerId,
    pub hover: render::LayerId,
}

/// The overlay on `sheet`. `here` tells whether a symbol is drawn on this sheet.
pub(crate) fn draw(
    project: &Project,
    conn: &Connectivity,
    sheet: Id,
    o: &SimOverlay,
    layers: Layers,
    p: &mut Painter,
    bounds: &dyn Fn(ObjectId) -> Option<Rect>,
) {
    let sch = &project.schematic;
    let on_sheet = |symbol: Id| sch.symbols.get(symbol).is_some_and(|s| s.sheet == sheet);
    // Probed nets (and the hovered one): their wires, thick.
    let probed = o.nets.iter().map(|(n, c)| (n, *c, layers.sim));
    let hover = o.hover_nets.iter().map(|n| (n, pal().hover, layers.hover));
    let nets: Vec<(&String, Color, render::LayerId)> = probed.chain(hover).collect();
    if !nets.is_empty() {
        let style_of = |id: ObjectId| {
            let net = conn.net_of(id)?;
            nets.iter().rev().find(|(n, ..)| **n == net.name).map(|&(_, c, l)| (c, l))
        };
        for w in sch.wires.iter().filter(|w| w.sheet == sheet) {
            if let Some((color, layer)) = style_of(ObjectId::Wire(w.id)) {
                p.line(layer, w.a, w.b, WIRE_WIDTH * 3, color.with_alpha(0xc0));
            }
        }
        for l in sch.labels.iter().filter(|l| l.sheet == sheet) {
            if let Some((color, layer)) = style_of(ObjectId::Label(l.id)) {
                p.circle(layer, l.pos, WIRE_WIDTH * 2, color);
            }
        }
    }
    let layer = layers.sim;
    // Probed pins: the pin in the colour, an arrow into the part.
    for &(symbol, pin, color) in o.pins.iter().filter(|(s, ..)| on_sheet(*s)) {
        if let Some((a, b)) = pin_ends(project, symbol, pin) {
            p.line(layer, a, b, WIRE_WIDTH * 2, color.with_alpha(0xc0));
            arrow(p, layer, a, b, mm(1.0), color);
        }
    }
    for &(symbol, pin) in o.hover_pins.iter().filter(|(s, _)| on_sheet(*s)) {
        if let Some((a, b)) = pin_ends(project, symbol, pin) {
            p.line(layers.hover, a, b, WIRE_WIDTH * 3, pal().hover);
            arrow(p, layers.hover, a, b, mm(1.0), pal().hover);
        }
    }
    // Parts whose power is probed (or hovered): a rounded frame around each unit.
    let probed = o.parts.iter().map(|&(c, color)| (c, color, layers.sim));
    let hover = o.hover_parts.iter().map(|&c| (c, pal().hover, layers.hover));
    for (component, color, layer) in probed.chain(hover) {
        for s in sch.symbols.iter().filter(|s| s.component == component && s.sheet == sheet) {
            if let Some(r) = bounds(ObjectId::Symbol(s.id)) {
                let r = r.grow(mm(0.5));
                let c = [r.min, Point::new(r.max.x, r.min.y), r.max, Point::new(r.min.x, r.max.y)];
                for k in 0..4 {
                    p.line(layer, c[k], c[(k + 1) % 4], WIRE_WIDTH * 2, color.with_alpha(0xc0));
                }
            }
        }
    }
    let alpha = |c: Color| if o.dim { c.with_alpha(0x70) } else { c };
    let (fg, bg) = (alpha(pal().sim_value), alpha(pal().sim_value_bg));
    // Voltages: at the first label of the net on this sheet, else the middle of its
    // longest wire here.
    if !o.voltages.is_empty() {
        let mut at: std::collections::HashMap<&str, (Point, bool)> = Default::default();
        for l in sch.labels.iter().filter(|l| l.sheet == sheet) {
            if let Some(net) = conn.net_of(ObjectId::Label(l.id)) {
                at.entry(net.name.as_str()).or_insert((l.pos, true));
            }
        }
        let mut longest: std::collections::HashMap<&str, (Coord, Point)> = Default::default();
        for w in sch.wires.iter().filter(|w| w.sheet == sheet) {
            if let Some(net) = conn.net_of(ObjectId::Wire(w.id)) {
                let len = (w.b - w.a).length() as Coord;
                let e = longest.entry(net.name.as_str()).or_insert((-1, w.a));
                if len > e.0 {
                    *e = (len, Point::new((w.a.x + w.b.x) / 2, (w.a.y + w.b.y) / 2));
                }
            }
        }
        for (net, (_, mid)) in longest {
            at.entry(net).or_insert((mid, false));
        }
        for (net, text) in &o.voltages {
            if let Some(&(pos, label)) = at.get(net.as_str()) {
                // Above a label (clear of its text), just above a wire.
                let pos = pos + Point::new(0, if label { mm(1.6) } else { mm(0.5) });
                tag(p, layer, pos, text, FH::Center, FV::Bottom, fg, bg);
            }
        }
    }
    // Currents of two-pin parts: an arrow on the pin the current flows into, the value
    // beside it.
    for (symbol, pin, text) in o.currents.iter().filter(|(s, ..)| on_sheet(*s)) {
        let Some((a, b)) = pin_ends(project, *symbol, *pin) else { continue };
        arrow(p, layer, a, b, mm(0.9), fg);
        // Beside the pin, reaching away from the body (clear of the reference and
        // value, which sit at the body).
        let mid = Point::new((a.x + b.x) / 2, (a.y + b.y) / 2);
        let d = b - a;
        let (pos, h, v) = if d.x.abs() >= d.y.abs() {
            let h = if d.x > 0 { FH::Right } else { FH::Left };
            (mid + Point::new(0, mm(0.5)), h, FV::Bottom)
        } else {
            let v = if d.y > 0 { FV::Top } else { FV::Bottom };
            (mid - Point::new(mm(0.6), 0), FH::Right, v)
        };
        tag(p, layer, pos, text, h, v, fg, bg);
    }
}

fn mm(v: f64) -> Coord {
    (v * 1e6) as Coord
}

/// A filled arrow head in the middle of `a`→`b`, pointing to `b`.
fn arrow(p: &mut Painter, layer: render::LayerId, a: Point, b: Point, size: Coord, color: Color) {
    let (dx, dy) = ((b.x - a.x) as f64, (b.y - a.y) as f64);
    let len = dx.hypot(dy);
    if len == 0.0 {
        return;
    }
    let (ux, uy) = (dx / len, dy / len);
    let s = size as f64;
    let mid = (a.x as f64 + dx / 2.0, a.y as f64 + dy / 2.0);
    let tip = (mid.0 + ux * s / 2.0, mid.1 + uy * s / 2.0);
    let base = (mid.0 - ux * s / 2.0, mid.1 - uy * s / 2.0);
    let w = s * 0.45;
    let pt = |x: f64, y: f64| Point::new(x as Coord, y as Coord);
    p.fill(
        layer,
        [pt(tip.0, tip.1), pt(base.0 - uy * w, base.1 + ux * w), pt(base.0 + uy * w, base.1 - ux * w)],
        color,
    );
}

/// A value on a background.
#[allow(clippy::too_many_arguments)]
fn tag(p: &mut Painter, layer: render::LayerId, pos: Point, text: &str, h: FH, v: FV, fg: Color, bg: Color) {
    let height = (TAG_HEIGHT as f64 * 1e-6) as f32;
    let [x0, y0, x1, y1] = font::text_box(text, height, h, v);
    let pad = height * 0.3;
    let o = mmf(pos);
    let center = [o[0] + (x0 + x1) / 2.0, o[1] + (y0 + y1) / 2.0];
    let size = [x1 - x0 + 2.0 * pad, y1 - y0 + 2.0 * pad];
    p.push(layer, Instance::rect(center, size, pad, 0.0, bg));
    let mut t = Text::new(text, pos, TAG_HEIGHT);
    t.halign = match h {
        FH::Left => HAlign::Left,
        FH::Center => HAlign::Center,
        FH::Right => HAlign::Right,
    };
    t.valign = match v {
        FV::Bottom => VAlign::Bottom,
        FV::Center => VAlign::Center,
        FV::Top => VAlign::Top,
    };
    p.text(layer, &t, fg, &|s| s.to_string(), false);
}
