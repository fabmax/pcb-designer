//! Schematic connectivity: which pins, wires and labels form a net.
//!
//! Rules (per sheet, labels across sheets):
//! * Items touching at a point connect: wire ends, pin ends, labels, junctions.
//! * A wire end, pin end, label or junction on the *inside* of a wire connects to it
//!   (T connection). Two wires that merely cross do not connect unless a junction sits
//!   on the crossing.
//! * Labels with the same name connect: local and hierarchical ones on their sheet,
//!   global ones and power ports on all sheets.
//! * A sub-sheet's symbol has a pin for each name of its hierarchical labels (see
//!   [`Sheet::layout`]); on the parent sheet it connects like a component pin, and it
//!   joins the labels of its name on the sub-sheet.
//! * Buses never connect anything. Touching bus wires form a [`Bus`] whose members come
//!   from the bus labels on it (labels with bus names, see [`crate::bus`]); those labels
//!   create no nets. A bus entry's wire end joins the net there, so it knows its signal.
//!
//! Nets are named after their labels (power ports first). Unnamed nets with pins are
//! called `Net-(R1-1)` after their first pin (reference and pad), like KiCad does.

use rustc_hash::FxHashMap as HashMap;

use model::*;

use crate::query::symbol_of_instance;

/// A pin in a net.
#[derive(Clone, Debug, PartialEq)]
pub struct NetPin {
    /// Symbol instance and pin.
    pub symbol: Id,
    pub pin: Id,
    pub component: Id,
    pub unit: u32,
    pub refdes: String,
    pub pin_name: String,
    /// Pin number (pin name if the pin has no number).
    pub pin_number: String,
    /// Footprint pads of this pin in the component's variant.
    pub pads: Vec<String>,
    pub pos: Point,
}

impl NetPin {
    /// "R1.1": reference and first pad (or pin number when unmapped).
    pub fn label(&self) -> String {
        format!("{}.{}", self.refdes, self.pads.first().unwrap_or(&self.pin_number))
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Net {
    pub name: String,
    /// The name comes from a label (not generated from a pin).
    pub named: bool,
    /// Sorted by reference designator and pad.
    pub pins: Vec<NetPin>,
    pub wires: Vec<Id>,
    pub labels: Vec<Id>,
    pub junctions: Vec<Id>,
    /// Bus entries whose wire end is on this net.
    pub entries: Vec<Id>,
    /// Sheet pins on this net: (sub-sheet, pin name).
    pub sheet_pins: Vec<(Id, String)>,
}

/// Touching bus wires with their labels and entries.
#[derive(Clone, Debug, PartialEq)]
pub struct Bus {
    /// Name of its first bus label; empty if it has none.
    pub name: String,
    pub sheet: Id,
    /// Signals of all its bus labels, in order.
    pub members: Vec<String>,
    pub wires: Vec<Id>,
    pub labels: Vec<Id>,
    pub entries: Vec<Id>,
}

/// Something wrong in the schematic, with the objects involved.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct Problem {
    pub message: String,
    pub objects: Vec<ObjectId>,
    pub pos: Option<Point>,
}

impl Problem {
    fn new(message: String, objects: Vec<ObjectId>, pos: Option<Point>) -> Self {
        Problem { message, objects, pos }
    }
}

/// Connectivity of a whole schematic.
#[derive(Clone, Debug, Default)]
pub struct Connectivity {
    /// Nets with at least one pin or label, sorted by name; then wire-only groups
    /// (dangling wires) with an empty name.
    pub nets: Vec<Net>,
    net_of: HashMap<ObjectId, usize>,
    pin_net: HashMap<(Id, Id), usize>,
    /// Buses sorted by name, unnamed ones last.
    pub buses: Vec<Bus>,
    bus_of: HashMap<ObjectId, usize>,
    /// Automatic junction dots: (sheet, point) where three or more wire ends / pins meet,
    /// or where something connects to the inside of a wire.
    pub dots: Vec<(Id, Point)>,
    /// Pins connected to nothing: (symbol instance, pin, position).
    pub unconnected: Vec<(Id, Id, Point)>,
    /// Nets with conflicting names, misused buses and similar problems.
    pub problems: Vec<Problem>,
}

struct UnionFind(Vec<u32>);

impl UnionFind {
    fn find(&mut self, mut x: usize) -> usize {
        while self.0[x] as usize != x {
            let p = self.0[x] as usize;
            self.0[x] = self.0[p];
            x = p;
        }
        x
    }

    fn union(&mut self, a: usize, b: usize) {
        let (a, b) = (self.find(a), self.find(b));
        if a != b {
            self.0[a.max(b)] = a.min(b) as u32;
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum Node {
    Pin(usize),
    Wire(Id),
    Label(Id),
    Junction(Id),
    Entry(Id),
    /// Index into the sheet pins.
    SheetPin(usize),
}

/// Grid cell size for finding wires through a point.
const CELL: Coord = 10 * MM;

fn cell(p: Point) -> (i64, i64) {
    (p.x.div_euclid(CELL), p.y.div_euclid(CELL))
}

/// Segments (of one kind) by grid cell, to find those through a point.
struct SegmentGrid {
    segments: Vec<(Id, Point, Point)>,
    cells: HashMap<(Id, (i64, i64)), Vec<usize>>,
}

impl SegmentGrid {
    fn new(segments: impl Iterator<Item = (Id, Point, Point)>) -> Self {
        let segments: Vec<_> = segments.collect();
        let mut cells: HashMap<(Id, (i64, i64)), Vec<usize>> = HashMap::default();
        for (i, &(sheet, a, b)) in segments.iter().enumerate() {
            let (c0, c1) = (cell(a), cell(b));
            for x in c0.0.min(c1.0)..=c0.0.max(c1.0) {
                for y in c0.1.min(c1.1)..=c0.1.max(c1.1) {
                    cells.entry((sheet, (x, y))).or_default().push(i);
                }
            }
        }
        SegmentGrid { segments, cells }
    }

    /// Segments touching `p` (at an end or inside), with whether `p` is inside.
    fn at(&self, sheet: Id, p: Point) -> impl Iterator<Item = (usize, bool)> + '_ {
        self.cells.get(&(sheet, cell(p))).into_iter().flatten().filter_map(move |&i| {
            let (_, a, b) = self.segments[i];
            if p == a || p == b { Some((i, false)) } else { on_segment_inside(p, a, b).then_some((i, true)) }
        })
    }
}

/// `p` lies on segment a-b, excluding the end points.
pub fn on_segment_inside(p: Point, a: Point, b: Point) -> bool {
    if p == a || p == b {
        return false;
    }
    let (ab, ap) = (b - a, p - a);
    let cross = ab.x as i128 * ap.y as i128 - ab.y as i128 * ap.x as i128;
    if cross != 0 {
        return false;
    }
    let dot = ab.x as i128 * ap.x as i128 + ab.y as i128 * ap.y as i128;
    let len2 = ab.x as i128 * ab.x as i128 + ab.y as i128 * ab.y as i128;
    dot > 0 && dot < len2
}

impl Connectivity {
    pub fn build(project: &Project) -> Connectivity {
        let s = &project.schematic;
        let lib = &project.library;

        // ---- Nodes
        let mut pins: Vec<NetPin> = Vec::new();
        let mut pin_sheet: Vec<Id> = Vec::new();
        for inst in s.symbols.iter() {
            let Some(sym) = symbol_of_instance(project, inst) else { continue };
            let Some(c) = s.components.get(inst.component) else { continue };
            let part = lib.parts.get(c.part);
            let variant = part.and_then(|p| p.variant(c.variant));
            let fp = variant.and_then(|v| lib.footprints.get(v.footprint));
            let t = inst.transform();
            for pin in sym.pins.iter() {
                let mut pads: Vec<String> = match (variant, fp) {
                    (Some(v), Some(fp)) => v
                        .connections
                        .iter()
                        .filter(|x| x.unit == inst.unit && x.pin == pin.id)
                        .filter_map(|x| fp.pads.get(x.pad).map(|p| p.name.clone()))
                        .collect(),
                    _ => Vec::new(),
                };
                pads.sort_by(|a, b| natural_cmp(a, b));
                pads.dedup();
                pins.push(NetPin {
                    symbol: inst.id,
                    pin: pin.id,
                    component: c.id,
                    unit: inst.unit,
                    refdes: c.refdes.clone(),
                    pin_name: pin.name.clone(),
                    pin_number: pin.key().to_string(),
                    pads,
                    pos: t.point(pin.pos),
                });
                pin_sheet.push(inst.sheet);
            }
        }
        let bus_pins: Vec<(Id, Point, Id, String)> = if s.bus_wires.is_empty() {
            Vec::new()
        } else {
            pins.iter().zip(&pin_sheet).map(|(p, &sheet)| (p.symbol, p.pos, sheet, p.label())).collect()
        };
        let mut nodes: Vec<Node> = (0..pins.len()).map(Node::Pin).collect();
        nodes.extend(s.wires.ids().map(Node::Wire));
        // Bus labels name buses, not nets.
        let bus_label = |l: &NetLabel| crate::bus::is_bus_name(&l.name);
        nodes.extend(s.labels.iter().filter(|l| !bus_label(l)).map(|l| Node::Label(l.id)));
        nodes.extend(s.junctions.ids().map(Node::Junction));
        nodes.extend(s.bus_entries.ids().map(Node::Entry));
        // Sheet pins: (sub-sheet, parent sheet, pin).
        let mut ports: HashMap<Id, Vec<(&str, LabelShape)>> = HashMap::default();
        for l in s.labels.iter().filter(|l| l.kind == LabelKind::Hierarchical) {
            let p = ports.entry(l.sheet).or_default();
            if !p.iter().any(|x| x.0 == l.name) {
                p.push((l.name.as_str(), l.shape));
            }
        }
        let mut sheet_pins: Vec<(Id, Id, SheetPin)> = Vec::new();
        for sheet in s.sheets.iter() {
            let Some(parent) = sheet.parent else { continue };
            let (_, pins) = sheet.layout(ports.get(&sheet.id).map_or(&[][..], Vec::as_slice));
            sheet_pins.extend(pins.into_iter().map(|p| (sheet.id, parent, p)));
        }
        nodes.extend((0..sheet_pins.len()).map(Node::SheetPin));
        let index: HashMap<Node, usize> = nodes.iter().enumerate().map(|(i, &n)| (n, i)).collect();
        let mut uf = UnionFind((0..nodes.len() as u32).collect());

        // ---- Points: what touches where
        #[derive(Default)]
        struct At {
            nodes: Vec<usize>,
            wire_ends: u32,
            pins: u32,
            junction: bool,
        }
        let mut points: HashMap<(Id, Point), At> = HashMap::default();
        for (i, p) in pins.iter().enumerate() {
            let at = points.entry((pin_sheet[i], p.pos)).or_default();
            at.nodes.push(i);
            at.pins += 1;
        }
        for w in s.wires.iter() {
            let n = index[&Node::Wire(w.id)];
            for e in [w.a, w.b] {
                let at = points.entry((w.sheet, e)).or_default();
                at.nodes.push(n);
                at.wire_ends += 1;
            }
        }
        for l in s.labels.iter().filter(|l| !bus_label(l)) {
            points.entry((l.sheet, l.pos)).or_default().nodes.push(index[&Node::Label(l.id)]);
        }
        for e in s.bus_entries.iter() {
            points.entry((e.sheet, e.wire_end())).or_default().nodes.push(index[&Node::Entry(e.id)]);
        }
        for j in s.junctions.iter() {
            let at = points.entry((j.sheet, j.pos)).or_default();
            at.nodes.push(index[&Node::Junction(j.id)]);
            at.junction = true;
        }
        for (i, (_, parent, pin)) in sheet_pins.iter().enumerate() {
            let at = points.entry((*parent, pin.pos)).or_default();
            at.nodes.push(index[&Node::SheetPin(i)]);
            at.pins += 1;
        }
        for at in points.values() {
            for w in at.nodes.windows(2) {
                uf.union(w[0], w[1]);
            }
        }

        // ---- T connections: points inside wires
        let mut grid: HashMap<(Id, (i64, i64)), Vec<&Wire>> = HashMap::default();
        for w in s.wires.iter() {
            let (c0, c1) = (cell(w.a), cell(w.b));
            for x in c0.0.min(c1.0)..=c0.0.max(c1.0) {
                for y in c0.1.min(c1.1)..=c0.1.max(c1.1) {
                    grid.entry((w.sheet, (x, y))).or_default().push(w);
                }
            }
        }
        let mut dots = Vec::new();
        // Points something reaches from the inside of a wire.
        let mut on_wire: std::collections::HashSet<(Id, Point)> = Default::default();
        for (&(sheet, p), at) in &points {
            let mut inside = 0;
            if let Some(ws) = grid.get(&(sheet, cell(p))) {
                for w in ws {
                    if on_segment_inside(p, w.a, w.b) {
                        inside += 1;
                        uf.union(at.nodes[0], index[&Node::Wire(w.id)]);
                    }
                }
            }
            if inside > 0 {
                on_wire.insert((sheet, p));
            }
            let count = at.wire_ends + 2 * inside + at.pins;
            if count >= 3 && (at.wire_ends > 0 || inside > 0) && !at.junction {
                dots.push((sheet, p));
            }
        }
        dots.sort_by_key(|&(s, p)| (s, p.x, p.y));

        // ---- Labels with the same name: global and power labels across all sheets, local
        // and hierarchical labels on their own sheet.
        let mut by_name: HashMap<(Option<Id>, &str), usize> = HashMap::default();
        for l in s.labels.iter().filter(|l| !bus_label(l)) {
            let n = index[&Node::Label(l.id)];
            let key = (if l.kind.is_global() { None } else { Some(l.sheet) }, l.name.as_str());
            match by_name.get(&key) {
                Some(&first) => uf.union(first, n),
                None => {
                    by_name.insert(key, n);
                }
            }
        }

        // ---- Sheet pins join their sub-sheet's labels.
        let mut problems = Vec::new();
        for (i, (sheet, parent, pin)) in sheet_pins.iter().enumerate() {
            let n = index[&Node::SheetPin(i)];
            match by_name.get(&(Some(*sheet), pin.name.as_str())) {
                Some(&label) => uf.union(label, n),
                None => problems.push(Problem::new(
                    format!(
                        "Sheet pin {} of {}: hierarchical bus pins don't connect yet",
                        pin.name,
                        s.display_path(*sheet)
                    ),
                    vec![ObjectId::Sheet(*sheet)],
                    Some(pin.pos),
                )),
            }
            let wired = points.get(&(*parent, pin.pos)).is_some_and(|at| at.nodes.len() > 1)
                || on_wire.contains(&(*parent, pin.pos));
            if !wired {
                problems.push(Problem::new(
                    format!("Sheet pin {} of {} is not connected", pin.name, s.display_path(*sheet)),
                    vec![ObjectId::Sheet(*sheet)],
                    Some(pin.pos),
                ));
            }
        }
        for l in s.labels.iter().filter(|l| l.kind == LabelKind::Hierarchical) {
            if s.sheets.get(l.sheet).is_some_and(Sheet::is_root) {
                problems.push(Problem::new(
                    format!(
                        "Hierarchical label {} is on the root sheet: there is no sheet pin to connect it to",
                        l.name
                    ),
                    vec![ObjectId::Label(l.id)],
                    Some(l.pos),
                ));
            }
        }

        // ---- Group into nets
        let mut groups: HashMap<usize, Net> = HashMap::default();
        let mut pins: Vec<Option<NetPin>> = pins.into_iter().map(Some).collect();
        for (i, &node) in nodes.iter().enumerate() {
            let net = groups.entry(uf.find(i)).or_default();
            match node {
                Node::Pin(p) => net.pins.extend(pins[p].take()),
                Node::Wire(id) => net.wires.push(id),
                Node::Label(id) => net.labels.push(id),
                Node::Junction(id) => net.junctions.push(id),
                Node::Entry(id) => net.entries.push(id),
                Node::SheetPin(i) => net.sheet_pins.push((sheet_pins[i].0, sheet_pins[i].2.name.clone())),
            }
        }
        let mut unconnected = Vec::new();
        let mut nets: Vec<Net> = Vec::new();
        for (_, mut net) in groups {
            if net.pins.len() > 1 {
                net.pins.sort_by_cached_key(|p| (natural_key(&p.refdes), natural_key(&pin_key(p))));
            }
            if net.pins.len() == 1
                && net.wires.is_empty()
                && net.labels.is_empty()
                && net.junctions.is_empty()
                && net.entries.is_empty()
                && net.sheet_pins.is_empty()
            {
                let p = &net.pins[0];
                unconnected.push((p.symbol, p.pin, p.pos));
            }
            // Name, as KiCad does: power ports first, then global labels, then local and
            // then hierarchical labels by their sheet path (`/NAME` on the root sheet), the
            // shortest path first.
            let full_name = |l: &NetLabel| match l.kind {
                LabelKind::Power => (0, 0, l.name.clone()),
                LabelKind::Global => (1, 0, l.name.clone()),
                LabelKind::Local | LabelKind::Hierarchical => {
                    let path = s.sheet_path(l.sheet);
                    let tier = if l.kind == LabelKind::Local { 2 } else { 3 };
                    (tier, path.matches('/').count(), format!("{path}/{}", l.name))
                }
            };
            let labels: Vec<&NetLabel> = net.labels.iter().filter_map(|&id| s.labels.get(id)).collect();
            let mut names: Vec<(u8, usize, String)> = labels.iter().map(|l| full_name(l)).collect();
            names.sort();
            names.dedup();
            if let Some((_, _, name)) = names.first() {
                // Different names on one sheet conflict; hierarchical labels only carry a
                // net between sheets, so a net named differently on each sheet is fine.
                let mut sheets: Vec<Id> = labels.iter().map(|l| l.sheet).collect();
                sheets.sort();
                sheets.dedup();
                for sheet in sheets {
                    let here: Vec<&&NetLabel> =
                        labels.iter().filter(|l| l.sheet == sheet && l.kind != LabelKind::Hierarchical).collect();
                    let mut all: Vec<(u8, usize, String)> = here.iter().map(|l| full_name(l)).collect();
                    all.sort();
                    all.dedup();
                    if all.len() > 1 {
                        let all: Vec<&str> = all.iter().map(|n| n.2.as_str()).collect();
                        let objects: Vec<ObjectId> = here.iter().map(|l| ObjectId::Label(l.id)).collect();
                        let pos = here.first().map(|l| l.pos);
                        let msg = format!("Net has several names: {}", all.join(", "));
                        problems.push(Problem::new(msg, objects, pos));
                    }
                }
                net.name = name.clone();
                net.named = true;
            } else if let Some(p) = net.pins.first() {
                net.name = format!("Net-({}-{})", p.refdes, pin_key(p));
            }
            nets.push(net);
        }
        // Named nets and nets with pins first (by name), dangling wires last.
        nets.sort_by_cached_key(|n| (n.name.is_empty(), natural_key(&n.name), n.wires.first().copied()));
        unconnected.sort_by_key(|&(s, p, _)| (s, p));

        let mut net_of = HashMap::default();
        let mut pin_net = HashMap::default();
        for (i, net) in nets.iter().enumerate() {
            for &w in &net.wires {
                net_of.insert(ObjectId::Wire(w), i);
            }
            for &l in &net.labels {
                net_of.insert(ObjectId::Label(l), i);
            }
            for &j in &net.junctions {
                net_of.insert(ObjectId::Junction(j), i);
            }
            for &e in &net.entries {
                net_of.insert(ObjectId::BusEntry(e), i);
            }
            for p in &net.pins {
                pin_net.insert((p.symbol, p.pin), i);
            }
        }
        let mut c = Connectivity {
            nets,
            net_of,
            pin_net,
            buses: Vec::new(),
            bus_of: HashMap::default(),
            dots,
            unconnected,
            problems,
        };
        c.build_buses(project, &bus_pins);
        c
    }

    /// Group bus wires into buses, attach their labels and entries, and check how they
    /// are used.
    fn build_buses(&mut self, project: &Project, pins: &[(Id, Point, Id, String)]) {
        let s = &project.schematic;
        if s.bus_wires.is_empty()
            && s.bus_entries.is_empty()
            && !s.labels.iter().any(|l| crate::bus::is_bus_name(&l.name))
        {
            return;
        }
        let wires: Vec<&BusWire> = s.bus_wires.iter().collect();
        let grid = SegmentGrid::new(wires.iter().map(|w| (w.sheet, w.a, w.b)));
        let mut uf = UnionFind((0..wires.len() as u32).collect());

        // Touching ends, T-joints and junctions join bus wires.
        let mut ends: HashMap<(Id, Point), u32> = HashMap::default();
        for w in &wires {
            *ends.entry((w.sheet, w.a)).or_default() += 1;
            *ends.entry((w.sheet, w.b)).or_default() += 1;
        }
        let junctions: Vec<(Id, Point)> = s.junctions.iter().map(|j| (j.sheet, j.pos)).collect();
        for (&(sheet, p), &n) in &ends {
            let on: Vec<(usize, bool)> = grid.at(sheet, p).collect();
            for w in on.windows(2) {
                uf.union(w[0].0, w[1].0);
            }
            let inside = on.iter().filter(|o| o.1).count() as u32;
            if n + 2 * inside >= 3 && !junctions.contains(&(sheet, p)) {
                self.dots.push((sheet, p));
            }
        }
        for &(sheet, p) in &junctions {
            let on: Vec<usize> = grid.at(sheet, p).map(|o| o.0).collect();
            for w in on.windows(2) {
                uf.union(w[0], w[1]);
            }
        }
        self.dots.sort_by_key(|&(s, p)| (s, p.x, p.y));
        self.dots.dedup();

        let mut groups: HashMap<usize, Bus> = HashMap::default();
        for (i, w) in wires.iter().enumerate() {
            let bus = groups.entry(uf.find(i)).or_insert_with(|| Bus {
                name: String::new(),
                sheet: w.sheet,
                members: Vec::new(),
                wires: Vec::new(),
                labels: Vec::new(),
                entries: Vec::new(),
            });
            bus.wires.push(w.id);
        }
        let mut problems = Vec::new();
        let first_wire = |sheet: Id, p: Point| grid.at(sheet, p).next().map(|o| o.0);
        for l in s.labels.iter() {
            let Some(members) = crate::bus::members(&l.name) else { continue };
            match first_wire(l.sheet, l.pos) {
                Some(w) => {
                    let bus = groups.get_mut(&uf.find(w)).unwrap();
                    bus.labels.push(l.id);
                    for m in members {
                        if !bus.members.contains(&m) {
                            bus.members.push(m);
                        }
                    }
                }
                None => problems.push(Problem::new(
                    format!("Bus label {} is not on a bus", l.name),
                    vec![ObjectId::Label(l.id)],
                    Some(l.pos),
                )),
            }
        }
        for e in s.bus_entries.iter() {
            match first_wire(e.sheet, e.pos) {
                Some(w) => groups.get_mut(&uf.find(w)).unwrap().entries.push(e.id),
                None => problems.push(Problem::new(
                    "Bus entry is not on a bus".into(),
                    vec![ObjectId::BusEntry(e.id)],
                    Some(e.pos),
                )),
            }
        }
        let mut buses: Vec<Bus> = groups.into_values().collect();
        for bus in &mut buses {
            bus.name = bus.labels.first().and_then(|&l| s.labels.get(l)).map_or(String::new(), |l| l.name.clone());
        }
        buses.sort_by_cached_key(|b| (b.name.is_empty(), natural_key(&b.name), b.wires.first().copied()));
        for (i, bus) in buses.iter().enumerate() {
            let ids = bus
                .wires
                .iter()
                .map(|&w| ObjectId::BusWire(w))
                .chain(bus.labels.iter().map(|&l| ObjectId::Label(l)))
                .chain(bus.entries.iter().map(|&e| ObjectId::BusEntry(e)));
            for id in ids {
                self.bus_of.insert(id, i);
            }
        }
        let bus_at = |sheet: Id, p: Point| grid.at(sheet, p).next().map(|o| wires[o.0].id);
        let name_of = |w: Id| {
            let bus = &buses[self.bus_of[&ObjectId::BusWire(w)]];
            if bus.name.is_empty() { "an unnamed bus".to_string() } else { format!("bus {}", bus.name) }
        };

        // Wires and pins must not touch a bus: signals leave it through entries.
        for w in s.wires.iter() {
            for p in [w.a, w.b] {
                if let Some(b) = bus_at(w.sheet, p) {
                    problems.push(Problem::new(
                        format!("Wire touches {} directly; connect it through a bus entry", name_of(b)),
                        vec![ObjectId::Wire(w.id), ObjectId::BusWire(b)],
                        Some(p),
                    ));
                }
            }
        }
        for (symbol, p, sheet, pin) in pins {
            if let Some(b) = bus_at(*sheet, *p) {
                problems.push(Problem::new(
                    format!("Pin {pin} touches {} directly; connect it through a bus entry", name_of(b)),
                    vec![ObjectId::Symbol(*symbol), ObjectId::BusWire(b)],
                    Some(*p),
                ));
            }
        }
        for bus in &buses {
            if bus.name.is_empty() {
                let pos = s.bus_wires.get(bus.wires[0]).map(|w| w.a);
                problems.push(Problem::new(
                    "Bus has no bus label (like D[0..7] or {SDA SCL})".into(),
                    bus.wires.iter().map(|&w| ObjectId::BusWire(w)).collect(),
                    pos,
                ));
            }
            // Signals leaving through entries need a label naming a member.
            for &e in &bus.entries {
                let Some(net) = self.net_of(ObjectId::BusEntry(e)) else { continue };
                if net.wires.is_empty() && net.pins.is_empty() {
                    continue;
                }
                let pos = s.bus_entries.get(e).map(|e| e.wire_end());
                let mut objects = vec![ObjectId::BusEntry(e)];
                objects.extend(net.wires.iter().map(|&w| ObjectId::Wire(w)));
                if !net.named {
                    problems.push(Problem::new(
                        format!("Wire from a bus entry of {} has no label", name_of(bus.wires[0])),
                        objects,
                        pos,
                    ));
                } else if !bus.members.is_empty()
                    && !net.labels.iter().filter_map(|&l| s.labels.get(l)).any(|l| bus.members.contains(&l.name))
                {
                    // Members are label names (`D0`), not net names (`/D0` on the root sheet).
                    let name = net.labels.first().and_then(|&l| s.labels.get(l)).map_or(net.name.as_str(), |l| &l.name);
                    objects.extend(net.labels.iter().map(|&l| ObjectId::Label(l)));
                    problems.push(Problem::new(
                        format!("{name} is not a member of {}", name_of(bus.wires[0])),
                        objects,
                        pos,
                    ));
                }
            }
        }
        self.buses = buses;
        self.problems.extend(problems);
    }

    /// Bus of a bus wire, bus entry or bus label.
    pub fn bus_of(&self, id: ObjectId) -> Option<&Bus> {
        self.bus_of.get(&id).map(|&i| &self.buses[i])
    }

    /// Net of a wire, label, junction or bus entry (its wire end).
    pub fn net_of(&self, id: ObjectId) -> Option<&Net> {
        self.net_of.get(&id).map(|&i| &self.nets[i])
    }

    /// The net of a bus member: the net of a label with the member's name that the bus's
    /// sheet sees (local there, or global). Members are label names (`D0`); net names carry
    /// the sheet path (`/D0`).
    pub fn member_net(&self, project: &Project, bus: &Bus, member: &str) -> Option<&Net> {
        project
            .schematic
            .labels
            .iter()
            .filter(|l| l.name == member && (l.kind.is_global() || l.sheet == bus.sheet))
            .find_map(|l| self.net_of(ObjectId::Label(l.id)))
    }

    /// Net of a pin of a placed symbol.
    pub fn net_of_pin(&self, symbol: Id, pin: Id) -> Option<&Net> {
        self.pin_net.get(&(symbol, pin)).map(|&i| &self.nets[i])
    }

    pub fn net(&self, name: &str) -> Option<&Net> {
        self.nets.iter().find(|n| n.name == name)
    }

    /// Nets with at least one pin: the netlist.
    pub fn netlist(&self) -> impl Iterator<Item = &Net> {
        self.nets.iter().filter(|n| !n.pins.is_empty())
    }

    /// Compact text form: one net per line, `NAME: R1.1 R2.2`. Single-pin nets without
    /// a label are left out. Used by tests and agents.
    pub fn to_text(&self) -> String {
        let mut out = String::new();
        for n in self.netlist() {
            if n.pins.len() < 2 && !n.named {
                continue;
            }
            let pins: Vec<String> = n.pins.iter().map(NetPin::label).collect();
            out.push_str(&format!("{}: {}\n", n.name, pins.join(" ")));
        }
        out
    }
}

/// Sort key ordering embedded numbers by value ("R2" < "R10"), case-insensitive.
fn natural_key(s: &str) -> Vec<(u8, u64, String)> {
    let mut out = Vec::new();
    let mut rest = s;
    while let Some(c) = rest.chars().next() {
        let digit = c.is_ascii_digit();
        let end = rest.find(|ch: char| ch.is_ascii_digit() != digit).unwrap_or(rest.len());
        let (chunk, tail) = rest.split_at(end);
        out.push(match chunk.parse::<u64>() {
            Ok(n) if digit => (0, n, String::new()),
            _ => (1, 0, chunk.to_lowercase()),
        });
        rest = tail;
    }
    out
}

fn pin_key(p: &NetPin) -> String {
    p.pads.first().cloned().unwrap_or_else(|| p.pin_number.clone())
}
