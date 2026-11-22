//! Keep schematic wires attached while objects move ("drag" rather than "move").
//!
//! A [`plan`] is made from the geometry before the move, so what is attached is decided
//! once; [`apply`] then fixes up the wires after the objects moved. A GUI drag replays
//! its whole move from the start at every step, so pointing over other wires on the way
//! never attaches them.
//!
//! Points that move with the selection (pins of moved symbols, moved labels, junctions,
//! entries, sheet pins, and points where two moved wires meet) drag the unmoved wires
//! ending there along ([`follow`]):
//! * Horizontal and vertical wires keep their direction when the move is a translation:
//!   the wire shifts sideways with the moved end. Its other end is then
//!   - carried by stretching the single wire continuing perpendicular from there, if
//!     nothing else is attached; or
//!   - joined back to where it was by a new short wire (a dogleg), if something else
//!     (a pin, label, junction or other wire) is attached there; or
//!   - just moved, if it was dangling.
//! * Other wires, and all wires on rotation or mirroring, stretch.
//!
//! An end of a moved wire never comes loose from what stays put:
//! * If a single unmoved wire continues from it (perpendicular to a horizontal or
//!   vertical moved wire, or any for a diagonal one), that wire follows as above.
//! * If something stationary is there (a pin, label, junction, entry, sheet pin, a
//!   branch, a collinear or diagonal neighbour, or the inside of another wire), the end
//!   is *held*: a horizontal or vertical wire keeps its direction and a dogleg joins it
//!   back to the point; other wires (and rotations) stretch to the point.
//!
//! Unmoved wire ends, labels and junctions on the inside of a moved wire (T connections)
//! slide along with it. Wires that become zero-length are deleted; joining collinear
//! wires afterwards is `wire_tidy`'s, which runs after every command.
//!
//! Bus wires follow the same rules, separately: they attach to bus wires, bus entries'
//! bus ends, labels and junctions; wires attach to pins, wires, entries' wire ends,
//! labels, junctions and sheet pins.

use std::collections::{HashMap, HashSet};

use model::*;

use crate::CommandError;
use crate::connectivity::on_segment_inside;
use crate::transaction::Transaction;

/// Wires or bus wires.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    Wire,
    Bus,
}

/// A wire or bus wire.
#[derive(Clone, Debug)]
pub(crate) struct Seg {
    pub id: Id,
    pub sheet: Id,
    pub a: Point,
    pub b: Point,
}

impl Kind {
    pub fn of(id: ObjectId) -> Option<(Kind, Id)> {
        match id {
            ObjectId::Wire(i) => Some((Kind::Wire, i)),
            ObjectId::BusWire(i) => Some((Kind::Bus, i)),
            _ => None,
        }
    }

    pub fn segments(self, p: &Project) -> Vec<Seg> {
        let s = &p.schematic;
        match self {
            Kind::Wire => s.wires.iter().map(|w| Seg { id: w.id, sheet: w.sheet, a: w.a, b: w.b }).collect(),
            Kind::Bus => s.bus_wires.iter().map(|w| Seg { id: w.id, sheet: w.sheet, a: w.a, b: w.b }).collect(),
        }
    }

    pub fn get(self, p: &Project, id: Id) -> Option<Seg> {
        let s = &p.schematic;
        match self {
            Kind::Wire => s.wires.get(id).map(|w| Seg { id, sheet: w.sheet, a: w.a, b: w.b }),
            Kind::Bus => s.bus_wires.get(id).map(|w| Seg { id, sheet: w.sheet, a: w.a, b: w.b }),
        }
    }

    pub fn object_id(self, id: Id) -> ObjectId {
        match self {
            Kind::Wire => ObjectId::Wire(id),
            Kind::Bus => ObjectId::BusWire(id),
        }
    }

    pub(crate) fn object(self, w: Seg) -> Object {
        let Seg { id, sheet, a, b } = w;
        match self {
            Kind::Wire => Object::Wire(Wire { id, sheet, a, b }),
            Kind::Bus => Object::BusWire(BusWire { id, sheet, a, b }),
        }
    }

    /// How many other things (not segments of this kind) attach at each of the `wanted`
    /// points: labels and junctions, plus pins, entries' wire ends and sheet pins (wires)
    /// or entries' bus ends (buses).
    pub fn attach_counts(self, p: &Project, wanted: &HashSet<(Id, Point)>) -> HashMap<(Id, Point), u32> {
        let mut out: HashMap<(Id, Point), u32> = HashMap::new();
        if wanted.is_empty() {
            return out;
        }
        let sheets: HashSet<Id> = wanted.iter().map(|w| w.0).collect();
        // The box around the wanted points: what's outside is skipped before hashing, and
        // symbols whose pins can't reach it (by the longest pin reach in the library).
        let region = Rect::union_all(wanted.iter().map(|w| Rect::from_points(w.1, w.1))).unwrap();
        let reach = p
            .library
            .symbols
            .iter()
            .flat_map(|s| s.pins.iter())
            .map(|pin| pin.pos.x.abs() + pin.pos.y.abs())
            .max()
            .unwrap_or(0);
        let near = region.grow(reach);
        let s = &p.schematic;
        let mut add = |sheet: Id, pt: Point| {
            if region.contains(pt) && wanted.contains(&(sheet, pt)) {
                *out.entry((sheet, pt)).or_default() += 1;
            }
        };
        s.labels.iter().for_each(|l| add(l.sheet, l.pos));
        s.junctions.iter().for_each(|j| add(j.sheet, j.pos));
        match self {
            Kind::Wire => {
                for i in s.symbols.iter().filter(|i| near.contains(i.pos) && sheets.contains(&i.sheet)) {
                    crate::query::pin_points(p, i.id).into_iter().for_each(|pt| add(i.sheet, pt));
                }
                s.bus_entries.iter().for_each(|e| add(e.sheet, e.wire_end()));
                for sheet in s.sheets.iter() {
                    if let Some(parent) = sheet.parent.filter(|parent| sheets.contains(parent)) {
                        crate::query::sheet_pin_points(p, sheet.id).into_iter().for_each(|pt| add(parent, pt));
                    }
                }
            }
            Kind::Bus => s.bus_entries.iter().for_each(|e| add(e.sheet, e.pos)),
        }
        out
    }

    /// Other things (not segments of this kind) attached at `f`.
    fn attached_at(self, p: &Project, sheet: Id, f: Point) -> usize {
        let wanted = HashSet::from([(sheet, f)]);
        self.attach_counts(p, &wanted).get(&(sheet, f)).copied().unwrap_or(0) as usize
    }
}

/// `Some(true)` for a horizontal segment, `Some(false)` for a vertical one.
pub(crate) fn orientation(a: Point, b: Point) -> Option<bool> {
    if a.y == b.y && a.x != b.x {
        Some(true)
    } else if a.x == b.x && a.y != b.y {
        Some(false)
    } else {
        None
    }
}

/// The part of `d` perpendicular to a horizontal (`true`) or vertical segment.
fn across(horizontal: bool, d: Point) -> Point {
    if horizontal { Point::new(0, d.y) } else { Point::new(d.x, 0) }
}

/// What to do with the wires of one kind when objects move; made before the move.
pub(crate) struct Plan {
    kind: Kind,
    /// Moved segments of this kind.
    moved: HashSet<Id>,
    /// Points moving with the selection; unmoved segments ending there follow.
    follow: Vec<(Id, Point)>,
    /// Ends of moved segments held at stationary points: (segment, end `a`?, point).
    held: Vec<(Id, bool, Point)>,
    /// Points moving rigidly with stationary things on them: a new segment joins the old
    /// and the new position.
    joins: Vec<(Id, Point)>,
    /// Ends of unmoved segments on the inside of a moved one: (segment, end, new end).
    tees: Vec<(Id, Point, Point)>,
    /// Labels and junctions on the inside of a moved segment: (object, old, new).
    riders: Vec<(ObjectId, Point, Point)>,
}

/// Something that may lie on the inside of a moved segment: an unmoved segment's end
/// (index), or a label or junction, at a point.
type Rider = (Option<usize>, Option<ObjectId>, Point);

/// Plan how wires of `kind` stay attached when `ids` move by `t`.
pub(crate) fn plan(p: &Project, kind: Kind, ids: &[ObjectId], t: &Transform) -> Plan {
    let selected: HashSet<ObjectId> = ids.iter().copied().collect();
    let moved: HashSet<Id> =
        ids.iter().filter_map(|&id| Kind::of(id)).filter(|(k, _)| *k == kind).map(|(_, i)| i).collect();
    let fixed = connection_points(p, ids, kind);
    let fixed_set: HashSet<(Id, Point)> = fixed.iter().copied().collect();
    let mut plan = Plan {
        kind,
        moved: moved.clone(),
        follow: fixed,
        held: Vec::new(),
        joins: Vec::new(),
        tees: Vec::new(),
        riders: Vec::new(),
    };
    if moved.is_empty() {
        return plan;
    }
    let segs = kind.segments(p);
    // What is at the ends of moved segments.
    let wanted: HashSet<(Id, Point)> =
        segs.iter().filter(|w| moved.contains(&w.id)).flat_map(|w| [(w.sheet, w.a), (w.sheet, w.b)]).collect();
    let mut ends: HashMap<(Id, Point), Vec<usize>> = HashMap::new();
    for (i, s) in segs.iter().enumerate() {
        for q in [s.a, s.b] {
            if wanted.contains(&(s.sheet, q)) {
                ends.entry((s.sheet, q)).or_default().push(i);
            }
        }
    }
    let stuff = kind.attach_counts(p, &wanted);
    let translation = (!t.mirror && t.rotation.degrees() == 0.0).then_some(t.offset);
    // Unmoved segments by row (horizontal), column (vertical), or other.
    let mut rows: HashMap<(Id, Coord), Vec<usize>> = HashMap::new();
    let mut cols: HashMap<(Id, Coord), Vec<usize>> = HashMap::new();
    let mut slanted: Vec<usize> = Vec::new();
    for (i, o) in segs.iter().enumerate().filter(|(_, o)| !moved.contains(&o.id)) {
        match orientation(o.a, o.b) {
            Some(true) => rows.entry((o.sheet, o.a.y)).or_default().push(i),
            Some(false) => cols.entry((o.sheet, o.a.x)).or_default().push(i),
            None => slanted.push(i),
        }
    }
    let unmoved_inside = |sheet: Id, q: Point| {
        let row = rows.get(&(sheet, q.y)).into_iter().flatten();
        let col = cols.get(&(sheet, q.x)).into_iter().flatten();
        row.chain(col).chain(slanted.iter()).any(|&i| {
            let o = &segs[i];
            o.sheet == sheet && on_segment_inside(q, o.a, o.b)
        })
    };
    let mut seen: HashSet<(Id, Point)> = HashSet::new();
    for w in segs.iter().filter(|w| moved.contains(&w.id)) {
        for (is_a, q) in [(true, w.a), (false, w.b)] {
            let key = (w.sheet, q);
            if fixed_set.contains(&key) {
                continue;
            }
            let at = &ends[&key];
            let moved_here = at.iter().filter(|&&i| moved.contains(&segs[i].id)).count();
            let unmoved: Vec<&Seg> = at.iter().map(|&i| &segs[i]).filter(|o| !moved.contains(&o.id)).collect();
            let stationary = stuff.get(&key).is_some_and(|&n| n > 0) || unmoved_inside(w.sheet, q);
            if moved_here >= 2 {
                // Moves rigidly with the selection.
                if seen.insert(key) {
                    plan.follow.push(key);
                    if stationary {
                        plan.joins.push(key);
                    }
                }
                continue;
            }
            let follows = |o: &Seg| match (orientation(w.a, w.b), orientation(o.a, o.b)) {
                (Some(h), Some(oh)) => h != oh,
                (Some(_), None) => false,
                (None, _) => true,
            };
            match unmoved[..] {
                [] if !stationary => {}
                [o] if !stationary && follows(o) => plan.follow.push(key),
                _ => plan.held.push((w.id, is_a, q)),
            }
        }
    }

    // Tees and riders: unmoved ends, labels and junctions on the inside of moved segments.
    // Indexed by row and column for horizontal and vertical segments.
    let mut by_row: HashMap<(Id, Coord), Vec<Rider>> = HashMap::new();
    let mut by_col: HashMap<(Id, Coord), Vec<Rider>> = HashMap::new();
    let mut all: Vec<(Id, Option<usize>, Option<ObjectId>, Point)> = Vec::new();
    for (i, s) in segs.iter().enumerate().filter(|(_, s)| !moved.contains(&s.id)) {
        all.push((s.sheet, Some(i), None, s.a));
        all.push((s.sheet, Some(i), None, s.b));
    }
    let sch = &p.schematic;
    for l in sch.labels.iter().filter(|l| !selected.contains(&ObjectId::Label(l.id))) {
        all.push((l.sheet, None, Some(ObjectId::Label(l.id)), l.pos));
    }
    for j in sch.junctions.iter().filter(|j| !selected.contains(&ObjectId::Junction(j.id))) {
        all.push((j.sheet, None, Some(ObjectId::Junction(j.id)), j.pos));
    }
    for &(sheet, seg, obj, q) in &all {
        by_row.entry((sheet, q.y)).or_default().push((seg, obj, q));
        by_col.entry((sheet, q.x)).or_default().push((seg, obj, q));
    }
    for w in segs.iter().filter(|w| moved.contains(&w.id)) {
        let horizontal = orientation(w.a, w.b);
        let candidates: Box<dyn Iterator<Item = Rider>> = match horizontal {
            Some(true) => Box::new(by_row.get(&(w.sheet, w.a.y)).into_iter().flatten().copied()),
            Some(false) => Box::new(by_col.get(&(w.sheet, w.a.x)).into_iter().flatten().copied()),
            None => Box::new(all.iter().filter(|e| e.0 == w.sheet).map(|e| (e.1, e.2, e.3))),
        };
        for (seg, obj, q) in candidates {
            let key = (w.sheet, q);
            if !on_segment_inside(q, w.a, w.b) || fixed_set.contains(&key) {
                continue;
            }
            // A moved segment ending here decides (see above).
            if ends.get(&key).is_some_and(|at| at.iter().any(|&i| moved.contains(&segs[i].id))) {
                continue;
            }
            let to = match (translation, horizontal) {
                (Some(d), Some(h)) => q + across(h, d),
                _ => t.point(q),
            };
            match (seg, obj) {
                (Some(i), _) => plan.tees.push((segs[i].id, q, to)),
                (_, Some(o)) => plan.riders.push((o, q, to)),
                _ => {}
            }
        }
    }
    plan
}

/// Points of moved schematic objects (other than segments of `kind`) where segments of
/// `kind` attach: (sheet, point).
fn connection_points(p: &Project, ids: &[ObjectId], kind: Kind) -> Vec<(Id, Point)> {
    let s = &p.schematic;
    let mut out = Vec::new();
    for &id in ids {
        match (kind, id) {
            (Kind::Wire, ObjectId::Symbol(i)) => {
                if let Some(inst) = s.symbols.get(i) {
                    out.extend(crate::query::pin_positions(p, i).into_iter().map(|(_, _, pt)| (inst.sheet, pt)));
                }
            }
            (Kind::Wire, ObjectId::Sheet(i)) => {
                if let Some(parent) = s.sheets.get(i).and_then(|sh| sh.parent) {
                    out.extend(crate::query::sheet_pin_points(p, i).into_iter().map(|pt| (parent, pt)));
                }
            }
            (Kind::Wire, ObjectId::BusEntry(i)) => out.extend(s.bus_entries.get(i).map(|e| (e.sheet, e.wire_end()))),
            (Kind::Bus, ObjectId::BusEntry(i)) => out.extend(s.bus_entries.get(i).map(|e| (e.sheet, e.pos))),
            (_, ObjectId::Label(i)) => out.extend(s.labels.get(i).map(|l| (l.sheet, l.pos))),
            (_, ObjectId::Junction(i)) => out.extend(s.junctions.get(i).map(|j| (j.sheet, j.pos))),
            _ => {}
        }
    }
    out.sort_by_key(|&(s, p)| (s, p.x, p.y));
    out.dedup();
    out
}

/// Carry out `plan` after the objects moved by `t`.
pub(crate) fn apply(tx: &mut Transaction, plan: Plan, t: &Transform) -> Result<(), CommandError> {
    let kind = plan.kind;
    let translation = (!t.mirror && t.rotation.degrees() == 0.0).then_some(t.offset);
    let mut created: HashSet<Id> = HashSet::new();
    let mut held: HashSet<Id> = HashSet::new();
    for &(id, is_a, q) in &plan.held {
        let Some(w) = kind.get(tx.project(), id) else { continue };
        held.insert(id);
        let to = match (translation, orientation(w.a, w.b)) {
            (Some(d), Some(h)) => q + across(h, d),
            _ => q,
        };
        let (a, b) = if is_a { (to, w.b) } else { (w.a, to) };
        set_wire(tx, kind, &w, a, b)?;
        if to != q {
            created.insert(insert(tx, kind, w.sheet, q, to)?);
        }
    }
    for &(sheet, q) in &plan.joins {
        let to = t.point(q);
        if to != q {
            created.insert(insert(tx, kind, sheet, q, to)?);
        }
    }
    follow(tx, kind, &plan.moved, &plan.follow, t, &created)?;
    for &(id, q, to) in &plan.tees {
        let Some(o) = kind.get(tx.project(), id) else { continue };
        if o.a == q {
            set_wire(tx, kind, &o, to, o.b)?;
        } else if o.b == q {
            set_wire(tx, kind, &o, o.a, to)?;
        }
    }
    for &(id, q, to) in &plan.riders {
        if !tx.project().contains(id) {
            continue;
        }
        tx.update(id, |o| {
            match o {
                Object::Label(l) if l.pos == q => l.pos = to,
                Object::Junction(j) if j.pos == q => j.pos = to,
                _ => {}
            }
            Ok(())
        })?;
    }
    // `wire_tidy` merges what the move made collinear afterwards.
    Ok(())
}

fn insert(tx: &mut Transaction, kind: Kind, sheet: Id, a: Point, b: Point) -> Result<Id, CommandError> {
    let id = Id::new();
    tx.insert(kind.object(Seg { id, sheet, a, b }))?;
    Ok(id)
}

/// Unmoved segments ending at the `moved` points (sheet, old position) follow them to
/// their new positions (see the module docs). `skip`: segments to leave alone.
fn follow(
    tx: &mut Transaction,
    kind: Kind,
    moved_segs: &HashSet<Id>,
    moved: &[(Id, Point)],
    t: &Transform,
    skip: &HashSet<Id>,
) -> Result<(), CommandError> {
    if moved.is_empty() {
        return Ok(());
    }
    let map: HashMap<(Id, Point), Point> = moved.iter().map(|&(s, p)| ((s, p), t.point(p))).collect();
    let translation = (!t.mirror && t.rotation.degrees() == 0.0).then_some(t.offset);
    let attached: Vec<Id> = kind
        .segments(tx.project())
        .into_iter()
        .filter(|w| {
            !moved_segs.contains(&w.id)
                && !skip.contains(&w.id)
                && (map.contains_key(&(w.sheet, w.a)) || map.contains_key(&(w.sheet, w.b)))
        })
        .map(|w| w.id)
        .collect();
    let mut handled: HashSet<Id> = moved_segs.iter().chain(skip).copied().collect();
    for id in attached {
        if !handled.insert(id) {
            continue;
        }
        let Some(w) = kind.get(tx.project(), id) else { continue };
        match (map.get(&(w.sheet, w.a)), map.get(&(w.sheet, w.b))) {
            (Some(&a), Some(&b)) => set_wire(tx, kind, &w, a, b)?,
            (Some(&a), None) => one_end(tx, kind, &w, true, a, translation, &mut handled)?,
            (None, Some(&b)) => one_end(tx, kind, &w, false, b, translation, &mut handled)?,
            (None, None) => {}
        }
    }
    Ok(())
}

/// Set a wire's end points; delete it if it collapsed.
fn set_wire(tx: &mut Transaction, kind: Kind, w: &Seg, a: Point, b: Point) -> Result<(), CommandError> {
    if a == b {
        tx.remove(kind.object_id(w.id))?;
        return Ok(());
    }
    tx.update(kind.object_id(w.id), |o| {
        match o {
            Object::Wire(w) => (w.a, w.b) = (a, b),
            Object::BusWire(w) => (w.a, w.b) = (a, b),
            _ => {}
        }
        Ok(())
    })
}

/// Wire `w` whose end `a` (or `b` if `!moving_a`) moved to `new`.
#[allow(clippy::too_many_arguments)]
fn one_end(
    tx: &mut Transaction,
    kind: Kind,
    w: &Seg,
    moving_a: bool,
    new: Point,
    translation: Option<Point>,
    handled: &mut HashSet<Id>,
) -> Result<(), CommandError> {
    let (m, f) = if moving_a { (w.a, w.b) } else { (w.b, w.a) };
    let perp = match (translation, orientation(m, f)) {
        (Some(d), Some(true)) if d.y != 0 => Point::new(0, d.y),
        (Some(d), Some(false)) if d.x != 0 => Point::new(d.x, 0),
        _ => {
            // Stretch.
            let (a, b) = if moving_a { (new, f) } else { (f, new) };
            return set_wire(tx, kind, w, a, b);
        }
    };
    let f_new = f + perp;
    let p = tx.project();
    let segments = kind.segments(p);
    // What else is attached at the far end?
    let others: Vec<&Seg> =
        segments.iter().filter(|o| o.id != w.id && o.sheet == w.sheet && (o.a == f || o.b == f)).collect();
    let aligned = |o: &Seg| {
        let other_end = if o.a == f { o.b } else { o.a };
        if perp.x == 0 { other_end.x == f.x } else { other_end.y == f.y }
    };
    let attached_at_f = kind.attached_at(p, w.sheet, f);
    let inside = segments.iter().any(|o| o.id != w.id && o.sheet == w.sheet && on_segment_inside(f, o.a, o.b));
    let stretch_neighbor = match others.as_slice() {
        [o] if aligned(o) && attached_at_f == 0 && !inside && !handled.contains(&o.id) => Some((*o).clone()),
        _ => None,
    };
    let something_else = !others.is_empty() || attached_at_f > 0 || inside;
    let sheet = w.sheet;

    if let Some(o) = stretch_neighbor {
        handled.insert(o.id);
        let (a, b) = if o.a == f { (f_new, o.b) } else { (o.a, f_new) };
        set_wire(tx, kind, &o, a, b)?;
    } else if something_else {
        tx.insert(kind.object(Seg { id: Id::new(), sheet, a: f, b: f_new }))?;
    }
    let (a, b) = if moving_a { (new, f_new) } else { (f_new, new) };
    set_wire(tx, kind, w, a, b)
}
