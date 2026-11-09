//! Keep schematic wires attached to objects that move ("drag" rather than "move").
//!
//! For each wire with exactly one end on a moved connection point:
//! * Horizontal and vertical wires keep their direction when the move is a translation:
//!   the wire shifts sideways with the moved end. Its other end is then
//!   - carried by stretching the single wire continuing perpendicular from there, if
//!     nothing else is attached; or
//!   - joined back to where it was by a new short wire (a dogleg), if something else
//!     (a pin, label, junction or other wire) is attached there; or
//!   - just moved, if it was dangling.
//!
//!   This is stable when applied repeatedly during a drag: after the first step the
//!   dogleg is the perpendicular wire that gets stretched.
//! * Other wires, and all wires on rotation or mirroring, stretch.
//!
//! Wires with both ends on moved points move entirely. Wires that become zero-length
//! are deleted.
//!
//! Bus wires follow the same rules, separately: they attach to bus wires, bus entries'
//! bus ends, labels and junctions; wires attach to pins, wires, entries' wire ends,
//! labels and junctions.

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
struct Seg {
    id: Id,
    sheet: Id,
    a: Point,
    b: Point,
}

impl Kind {
    fn segments(self, p: &Project) -> Vec<Seg> {
        let s = &p.schematic;
        match self {
            Kind::Wire => s.wires.iter().map(|w| Seg { id: w.id, sheet: w.sheet, a: w.a, b: w.b }).collect(),
            Kind::Bus => s.bus_wires.iter().map(|w| Seg { id: w.id, sheet: w.sheet, a: w.a, b: w.b }).collect(),
        }
    }

    fn get(self, p: &Project, id: Id) -> Option<Seg> {
        let s = &p.schematic;
        match self {
            Kind::Wire => s.wires.get(id).map(|w| Seg { id, sheet: w.sheet, a: w.a, b: w.b }),
            Kind::Bus => s.bus_wires.get(id).map(|w| Seg { id, sheet: w.sheet, a: w.a, b: w.b }),
        }
    }

    fn object_id(self, id: Id) -> ObjectId {
        match self {
            Kind::Wire => ObjectId::Wire(id),
            Kind::Bus => ObjectId::BusWire(id),
        }
    }

    fn object(self, w: Seg) -> Object {
        let Seg { id, sheet, a, b } = w;
        match self {
            Kind::Wire => Object::Wire(Wire { id, sheet, a, b }),
            Kind::Bus => Object::BusWire(BusWire { id, sheet, a, b }),
        }
    }

    /// Other things (not segments of this kind) attached at `f`.
    fn attached_at(self, p: &Project, sheet: Id, f: Point) -> usize {
        let s = &p.schematic;
        let common = s.labels.iter().filter(|l| l.sheet == sheet && l.pos == f).count()
            + s.junctions.iter().filter(|j| j.sheet == sheet && j.pos == f).count();
        common
            + match self {
                Kind::Wire => {
                    s.symbols
                        .iter()
                        .filter(|i| i.sheet == sheet)
                        .flat_map(|i| crate::query::pin_positions(p, i.id))
                        .filter(|(_, _, pt)| *pt == f)
                        .count()
                        + s.bus_entries.iter().filter(|e| e.sheet == sheet && e.wire_end() == f).count()
                }
                Kind::Bus => s.bus_entries.iter().filter(|e| e.sheet == sheet && e.pos == f).count(),
            }
    }
}

pub(crate) fn follow(
    tx: &mut Transaction,
    kind: Kind,
    ids: &[ObjectId],
    moved: &[(Id, Point)],
    t: &Transform,
) -> Result<(), CommandError> {
    if moved.is_empty() {
        return Ok(());
    }
    let moved_wires: HashSet<Id> =
        ids.iter().filter(|id| kind.object_id(id.raw()) == **id).map(|id| id.raw()).collect();
    let map: HashMap<(Id, Point), Point> = moved.iter().map(|&(s, p)| ((s, p), t.point(p))).collect();
    let translation = (!t.mirror && t.rotation.degrees() == 0.0).then_some(t.offset);
    let attached: Vec<Id> = kind
        .segments(tx.project())
        .into_iter()
        .filter(|w| {
            !moved_wires.contains(&w.id) && (map.contains_key(&(w.sheet, w.a)) || map.contains_key(&(w.sheet, w.b)))
        })
        .map(|w| w.id)
        .collect();
    let mut handled = moved_wires;
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
    let horizontal = m.y == f.y && m.x != f.x;
    let vertical = m.x == f.x && m.y != f.y;
    let perp = match translation {
        Some(d) if horizontal && d.y != 0 => Point::new(0, d.y),
        Some(d) if vertical && d.x != 0 => Point::new(d.x, 0),
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
