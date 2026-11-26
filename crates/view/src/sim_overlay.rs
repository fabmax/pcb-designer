//! Simulation on the schematic: what the Probe tool would probe under the pointer, and
//! the overlay of a simulation: probed nets' wires in their trace colour, arrows on
//! probed pins, and operating point values (net voltages, the currents of two-pin
//! parts), and the controls of switches and potentiometers (a toggle, a slider) that
//! change them. The app decides what to show; this draws it.

use editor_core::connectivity::Connectivity;
use editor_core::query::symbol_of_instance;
use model::*;
use render::font::{self, HAlign as FH, VAlign as FV};
use render::{Color, Instance};

use crate::draw::{Painter, mmf};
use crate::index::SpatialIndex;
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
    /// Switches and potentiometers: a control on each, at its setting.
    pub controls: Vec<Control>,
}

/// A switch's toggle or a potentiometer's slider on the schematic.
#[derive(Clone, Debug, PartialEq)]
pub struct Control {
    pub component: Id,
    pub kind: SimControl,
    /// 0 or 1 (a switch), 0…1 (a wiper).
    pub value: f64,
}

/// Where a control went on a sheet, to click and drag it.
#[derive(Clone, Debug, PartialEq)]
pub struct ControlArea {
    pub component: Id,
    pub kind: SimControl,
    pub rect: Rect,
    /// A slider's track, left and right end (x).
    pub track: Option<(Coord, Coord)>,
}

impl ControlArea {
    /// The setting at `p`: a slider's under the pointer (clamped to its track).
    pub fn value_at(&self, p: Point) -> f64 {
        match self.track {
            Some((a, b)) if b > a => ((p.x - a) as f64 / (b - a) as f64).clamp(0.0, 1.0),
            _ => 0.0,
        }
    }
}

/// What a sheet's overlay drew: where the value tags and the controls went.
#[derive(Clone, Debug, Default)]
pub struct Drawn {
    pub tags: Vec<Rect>,
    pub controls: Vec<ControlArea>,
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
            && self.controls.is_empty()
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

/// The overlay on `sheet`; `index`: the sheet's objects (frames around parts, and
/// what value tags keep clear of). Returns where the value tags and controls went.
pub(crate) fn draw(
    project: &Project,
    conn: &Connectivity,
    sheet: Id,
    o: &SimOverlay,
    layers: Layers,
    p: &mut Painter,
    index: &SpatialIndex,
) -> Drawn {
    let bounds = |id| index.bounds(id);
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
    let (volts, amps, bg) = (alpha(pal().sim_voltage), alpha(pal().sim_current), alpha(pal().sim_value_bg));
    let mut placer = Placer { index, placed: Vec::new() };
    // Controls first, next to their part: they are what's clicked.
    let mut controls = Vec::new();
    for c in &o.controls {
        let Some(s) =
            sch.symbols.iter().filter(|s| s.component == c.component && s.sheet == sheet).min_by_key(|s| s.unit)
        else {
            continue;
        };
        let Some(b) = bounds(ObjectId::Symbol(s.id)) else { continue };
        let size = control_size(c.kind);
        let r = placer.place_rect(&control_spots(b, size), Some(s.id));
        let part = sch.components.get(c.component).and_then(|c| project.library.parts.get(c.part));
        let pins = |p: &Part| {
            p.units.iter().filter_map(|u| project.library.symbols.get(u.symbol)).map(|s| s.pins.len()).sum::<usize>()
        };
        let three_pins = part.is_some_and(|p| pins(p) == 3usize);
        let track = draw_control(p, layer, r, c, three_pins);
        controls.push(ControlArea { component: c.component, kind: c.kind, rect: r, track });
    }
    // Currents of two-pin parts first (they have the fewest places to go): an arrow on
    // the pin the current flows into, the value beside it.
    for (symbol, pin, text) in o.currents.iter().filter(|(s, ..)| on_sheet(*s)) {
        let Some((a, b)) = pin_ends(project, *symbol, *pin) else { continue };
        arrow(p, layer, a, b, mm(0.9), amps);
        let spot = placer.place(&current_spots(a, b), text, Some(*symbol));
        tag(p, layer, spot.0, text, spot.1, spot.2, amps, bg);
    }
    // Voltages: by the net's labels on this sheet, else along its wires, longest first.
    if !o.voltages.is_empty() {
        let mut labels: std::collections::HashMap<&str, Vec<Point>> = Default::default();
        for l in sch.labels.iter().filter(|l| l.sheet == sheet) {
            if let Some(net) = conn.net_of(ObjectId::Label(l.id)) {
                labels.entry(net.name.as_str()).or_default().push(l.pos);
            }
        }
        let mut wires: std::collections::HashMap<&str, Vec<(Point, Point)>> = Default::default();
        for w in sch.wires.iter().filter(|w| w.sheet == sheet) {
            if let Some(net) = conn.net_of(ObjectId::Wire(w.id)) {
                wires.entry(net.name.as_str()).or_default().push((w.a, w.b));
            }
        }
        for (net, text) in &o.voltages {
            let mut ws = wires.get(net.as_str()).cloned().unwrap_or_default();
            ws.sort_by_key(|(a, b)| std::cmp::Reverse((*b - *a).length() as Coord));
            let spots = voltage_spots(labels.get(net.as_str()).map_or(&[][..], |l| &l[..]), &ws);
            if spots.is_empty() {
                continue;
            }
            let spot = placer.place(&spots, text, None);
            tag(p, layer, spot.0, text, spot.1, spot.2, volts, bg);
        }
    }
    Drawn { tags: placer.placed, controls }
}

const CONTROL_HEIGHT: f64 = 1.8;
const SWITCH_WIDTH: f64 = 4.6;
const TRACK: f64 = 7.0;
const CONTROL_TEXT: f64 = 0.8;

/// A control's width and height in mm: a slider leaves room for "100 %".
fn control_size(kind: SimControl) -> (f64, f64) {
    match kind {
        SimControl::Switch => (SWITCH_WIDTH, CONTROL_HEIGHT),
        SimControl::Position => {
            let [x0, _, x1, _] = font::text_box("100 %", CONTROL_TEXT as f32, FH::Left, FV::Center);
            (TRACK + 2.0 * 0.9 + (x1 - x0) as f64 + 0.4, CONTROL_HEIGHT)
        }
    }
}

/// Where a control may go around a symbol's bounds `b`: below it, above, right, left.
fn control_spots(b: Rect, (w, h): (f64, f64)) -> Vec<Rect> {
    let (w, h, gap) = (mm(w), mm(h), mm(0.6));
    let cx = (b.min.x + b.max.x) / 2;
    let cy = (b.min.y + b.max.y) / 2;
    let r = |x0: Coord, y0: Coord| Rect::from_points(Point::new(x0, y0), Point::new(x0 + w, y0 + h));
    vec![
        r(cx - w / 2, b.min.y - gap - h),
        r(cx - w / 2, b.max.y + gap),
        r(b.max.x + gap, cy - h / 2),
        r(b.min.x - gap - w, cy - h / 2),
    ]
}

/// Draws a control into `r`; returns a slider's track.
fn draw_control(
    p: &mut Painter,
    layer: render::LayerId,
    r: Rect,
    c: &Control,
    three_pins: bool,
) -> Option<(Coord, Coord)> {
    let accent = pal().sim_voltage;
    let bg = pal().sim_value_bg.with_alpha(0xff);
    let (x0, y0) = (r.min.x as f64 * 1e-6, r.min.y as f64 * 1e-6);
    let (w, h) = ((r.max.x - r.min.x) as f64 * 1e-6, (r.max.y - r.min.y) as f64 * 1e-6);
    let cy = y0 + h / 2.0;
    let at = |x: f64, y: f64| Point::new(mm(x), mm(y));
    let f = |x: f64| x as f32;
    let text = |p: &mut Painter, s: &str, x: f64, h: HAlign, color: Color| {
        let mut t = Text::new(s, at(x, cy), mm(CONTROL_TEXT));
        t.halign = h;
        t.valign = VAlign::Center;
        p.text(layer, &t, color, &|s| s.to_string(), false);
    };
    match c.kind {
        SimControl::Switch => {
            let on = c.value > 0.5;
            // A changeover switch is always "on" (to A or to B).
            let filled = on || three_pins;
            let center = [f(x0 + w / 2.0), f(cy)];
            let corner = f(h / 2.0);
            p.push(layer, Instance::rect(center, [f(w), f(h)], corner, 0.0, accent));
            if !filled {
                p.push(layer, Instance::rect(center, [f(w - 0.4), f(h - 0.4)], corner - 0.2, 0.0, bg));
            }
            let knob = if on { x0 + w - h / 2.0 } else { x0 + h / 2.0 };
            p.circle(layer, at(knob, cy), mm(h / 2.0 - 0.3), if filled { bg } else { accent });
            let label = match (three_pins, on) {
                (true, false) => "A",
                (true, true) => "B",
                (false, false) => "OFF",
                (false, true) => "ON",
            };
            // The label in the part the knob leaves free.
            let free = if on { x0 + (w - h) / 2.0 } else { x0 + h + (w - h) / 2.0 };
            text(p, label, free, HAlign::Center, if filled { bg } else { accent });
            p.grow(r);
            None
        }
        SimControl::Position => {
            p.push(layer, Instance::rect([f(x0 + w / 2.0), f(cy)], [f(w), f(h)], f(h / 2.0), 0.0, pal().sim_value_bg));
            let (a, b) = (x0 + 0.9, x0 + 0.9 + TRACK);
            let k = a + c.value.clamp(0.0, 1.0) * TRACK;
            p.line(layer, at(a, cy), at(b, cy), mm(0.4), accent.with_alpha(0x50));
            p.line(layer, at(a, cy), at(k, cy), mm(0.4), accent);
            p.circle(layer, at(k, cy), mm(0.75), bg);
            p.circle(layer, at(k, cy), mm(0.55), accent);
            let pct = format!("{:.0} %", c.value * 100.0);
            text(p, &pct, b + 0.9, HAlign::Left, accent);
            p.grow(r);
            Some((mm(a), mm(b)))
        }
    }
}

/// Where a value tag may go: its anchor and alignment.
type Spot = (Point, FH, FV);

/// A voltage tag's spots: above, then below each of the net's labels (clear of the
/// label's text above it), then along its wires (`wires` longest first): just above
/// or below a horizontal one, beside a vertical one, at its middle, then its quarters.
fn voltage_spots(labels: &[Point], wires: &[(Point, Point)]) -> Vec<Spot> {
    let mut out = Vec::new();
    for &l in labels.iter().take(4) {
        out.push((l + Point::new(0, mm(1.6)), FH::Center, FV::Bottom));
        out.push((l - Point::new(0, mm(1.6)), FH::Center, FV::Top));
    }
    for &(a, b) in wires.iter().take(8) {
        let d = b - a;
        let at = |t: f64| Point::new(a.x + (d.x as f64 * t) as Coord, a.y + (d.y as f64 * t) as Coord);
        for t in [0.5, 0.25, 0.75] {
            let q = at(t);
            if d.x.abs() >= d.y.abs() {
                out.push((q + Point::new(0, mm(0.5)), FH::Center, FV::Bottom));
                out.push((q - Point::new(0, mm(0.5)), FH::Center, FV::Top));
            } else {
                out.push((q + Point::new(mm(0.6), 0), FH::Left, FV::Center));
                out.push((q - Point::new(mm(0.6), 0), FH::Right, FV::Center));
            }
        }
    }
    out
}

/// A current tag's spots beside the pin from `a` (its outer end) to `b`: reaching away
/// from the body (clear of the reference and value, which sit at the body), on one side
/// of the pin, then the other, then out past its end.
fn current_spots(a: Point, b: Point) -> Vec<Spot> {
    let mid = Point::new((a.x + b.x) / 2, (a.y + b.y) / 2);
    let d = b - a;
    if d.x.abs() >= d.y.abs() {
        let h = if d.x > 0 { FH::Right } else { FH::Left };
        let (up, down) = (Point::new(0, mm(0.5)), Point::new(0, -mm(0.5)));
        vec![(mid + up, h, FV::Bottom), (mid + down, h, FV::Top), (a + up, h, FV::Bottom), (a + down, h, FV::Top)]
    } else {
        let v = if d.y > 0 { FV::Top } else { FV::Bottom };
        let (left, right) = (Point::new(-mm(0.6), 0), Point::new(mm(0.6), 0));
        vec![(mid + left, FH::Right, v), (mid + right, FH::Left, v), (a + left, FH::Right, v), (a + right, FH::Left, v)]
    }
}

/// Places value tags clear of what the sheet shows: a tag takes the first of its spots
/// that covers nothing, else the one covering least (texts and other tags weigh most,
/// symbols less, wires least).
struct Placer<'a> {
    index: &'a SpatialIndex,
    placed: Vec<Rect>,
}

impl Placer<'_> {
    /// How much `r` covers, in weighted mm². `own`: the symbol a current tag belongs
    /// to (its pin is under the tag on purpose).
    fn cost(&self, r: Rect, own: Option<Id>) -> f64 {
        let mut cost = 0.0;
        for id in self.index.in_rect(r, true) {
            let weight = match id {
                ObjectId::SymbolField(_) | ObjectId::Label(_) | ObjectId::SchText(_) => 4.0,
                ObjectId::Symbol(s) if Some(s) == own => 0.0,
                ObjectId::Symbol(_) | ObjectId::Sheet(_) => 2.0,
                ObjectId::Wire(_) | ObjectId::BusWire(_) => 1.0,
                _ => 0.5,
            };
            if let Some(b) = self.index.bounds(id) {
                cost += weight * overlap(r, b);
            }
        }
        cost + self.placed.iter().map(|p| 4.0 * overlap(r, *p)).sum::<f64>()
    }

    /// The best of `rects` (the first when nothing is better); it counts as taken from
    /// now on.
    fn place_rect(&mut self, rects: &[Rect], own: Option<Id>) -> Rect {
        let mut best = (f64::INFINITY, rects[0]);
        for &r in rects {
            let cost = self.cost(r, own);
            if cost < best.0 {
                best = (cost, r);
            }
            if cost == 0.0 {
                break;
            }
        }
        self.placed.push(best.1);
        best.1
    }

    /// The best of `spots` for a tag of `text` (the first when nothing is better); it
    /// counts as taken from now on.
    fn place(&mut self, spots: &[Spot], text: &str, own: Option<Id>) -> Spot {
        let mut best = (f64::INFINITY, spots[0]);
        for &spot in spots {
            let cost = self.cost(tag_rect(spot, text), own);
            if cost < best.0 {
                best = (cost, spot);
            }
            if cost == 0.0 {
                break;
            }
        }
        self.placed.push(tag_rect(best.1, text));
        best.1
    }
}

/// The area two rectangles share, in mm².
fn overlap(a: Rect, b: Rect) -> f64 {
    let w = (a.max.x.min(b.max.x) - a.min.x.max(b.min.x)).max(0) as f64 * 1e-6;
    let h = (a.max.y.min(b.max.y) - a.min.y.max(b.min.y)).max(0) as f64 * 1e-6;
    w * h
}

/// The rectangle a tag covers, with its background.
fn tag_rect((pos, h, v): Spot, text: &str) -> Rect {
    let height = (TAG_HEIGHT as f64 * 1e-6) as f32;
    let [x0, y0, x1, y1] = font::text_box(text, height, h, v);
    let pad = height * 0.3;
    let nm = |f: f32| (f as f64 * 1e6) as Coord;
    Rect::from_points(pos + Point::new(nm(x0 - pad), nm(y0 - pad)), pos + Point::new(nm(x1 + pad), nm(y1 + pad)))
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
