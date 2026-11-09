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

use std::collections::{HashMap, HashSet};

use model::*;

use crate::CommandError;
use crate::connectivity::on_segment_inside;
use crate::transaction::Transaction;

pub(crate) fn follow(
    tx: &mut Transaction,
    ids: &[ObjectId],
    moved: &[(Id, Point)],
    t: &Transform,
) -> Result<(), CommandError> {
    let moved_wires: HashSet<Id> =
        ids.iter().filter_map(|id| if let ObjectId::Wire(w) = id { Some(*w) } else { None }).collect();
    let map: HashMap<(Id, Point), Point> = moved.iter().map(|&(s, p)| ((s, p), t.point(p))).collect();
    let translation = (!t.mirror && t.rotation.degrees() == 0.0).then_some(t.offset);
    let attached: Vec<Id> = tx
        .project()
        .schematic
        .wires
        .iter()
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
        let Some(w) = tx.project().schematic.wires.get(id).cloned() else { continue };
        match (map.get(&(w.sheet, w.a)), map.get(&(w.sheet, w.b))) {
            (Some(&a), Some(&b)) => set_wire(tx, &w, a, b)?,
            (Some(&a), None) => one_end(tx, &w, true, a, translation, &mut handled)?,
            (None, Some(&b)) => one_end(tx, &w, false, b, translation, &mut handled)?,
            (None, None) => {}
        }
    }
    Ok(())
}

/// Set a wire's end points; delete it if it collapsed.
fn set_wire(tx: &mut Transaction, w: &Wire, a: Point, b: Point) -> Result<(), CommandError> {
    if a == b {
        tx.remove(ObjectId::Wire(w.id))?;
        return Ok(());
    }
    tx.update(ObjectId::Wire(w.id), |o| {
        if let Object::Wire(w) = o {
            w.a = a;
            w.b = b;
        }
        Ok(())
    })
}

/// Wire `w` whose end `a` (or `b` if `!moving_a`) moved to `new`.
fn one_end(
    tx: &mut Transaction,
    w: &Wire,
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
            return set_wire(tx, w, a, b);
        }
    };
    let f_new = f + perp;
    let p = tx.project();
    let s = &p.schematic;
    // What else is attached at the far end?
    let others: Vec<&Wire> =
        s.wires.iter().filter(|o| o.id != w.id && o.sheet == w.sheet && (o.a == f || o.b == f)).collect();
    let aligned = |o: &Wire| {
        let other_end = if o.a == f { o.b } else { o.a };
        if perp.x == 0 { other_end.x == f.x } else { other_end.y == f.y }
    };
    let pins_at_f = s
        .symbols
        .iter()
        .filter(|i| i.sheet == w.sheet)
        .flat_map(|i| crate::query::pin_positions(p, i.id))
        .filter(|(_, _, pt)| *pt == f)
        .count();
    let labels_at_f = s.labels.iter().filter(|l| l.sheet == w.sheet && l.pos == f).count()
        + s.junctions.iter().filter(|j| j.sheet == w.sheet && j.pos == f).count();
    let inside = s.wires.iter().any(|o| o.id != w.id && o.sheet == w.sheet && on_segment_inside(f, o.a, o.b));
    let stretch_neighbor = match others.as_slice() {
        [o] if aligned(o) && pins_at_f + labels_at_f == 0 && !inside && !handled.contains(&o.id) => Some((*o).clone()),
        _ => None,
    };
    let something_else = !others.is_empty() || pins_at_f + labels_at_f > 0 || inside;
    let sheet = w.sheet;

    if let Some(o) = stretch_neighbor {
        handled.insert(o.id);
        let (a, b) = if o.a == f { (f_new, o.b) } else { (o.a, f_new) };
        set_wire(tx, &o, a, b)?;
    } else if something_else {
        tx.insert(Object::Wire(Wire { id: Id::new(), sheet, a: f, b: f_new }))?;
    }
    let (a, b) = if moving_a { (new, f_new) } else { (f_new, new) };
    set_wire(tx, w, a, b)
}
