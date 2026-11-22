//! Keep schematic wires (and bus wires) in a tidy shape after every command, around what
//! the command changed:
//!
//! 1. Segments lying on top of each other on the same line become one, covering both
//!    (overlapping segments always connect, so they're always of one net).
//! 2. A segment is split where another segment's end or a junction lies on its inside
//!    (a T connection), so each piece ends where something connects.
//! 3. Collinear segments meeting end to end with nothing else at the shared point (no
//!    other segment, pin, label, junction, entry or sheet pin, and no T onto it) become one.
//!
//! The segments `keep` (those a move moves) keep their ids and are never split: a drag
//! replays its edits with the ids it started with. When segments become one, the one kept
//! is from `keep`, else one the command didn't touch, else one it changed, else a new one.
//!
//! Only segments reaching into the box around the changed points take part (a new tee,
//! overlap or join always involves one of them), indexed by line and by end, so a small
//! edit on a big sheet stays cheap.

use std::collections::{HashMap, HashSet};

use model::*;

use crate::CommandError;
use crate::connectivity::on_segment_inside;
use crate::rubber_band::{Kind, Seg};
use crate::transaction::Transaction;

/// Tidy the wires and bus wires around what the transaction changed so far.
pub(crate) fn tidy(tx: &mut Transaction, keep: &HashSet<Id>) -> Result<(), CommandError> {
    for kind in [Kind::Wire, Kind::Bus] {
        tidy_kind(tx, kind, keep)?;
    }
    Ok(())
}

/// The line a segment lies on: sheet, direction reduced to lowest terms (pointing right,
/// or up when vertical), and offset (the same for every point of the line).
type LineKey = (Id, i64, i64, i128);

fn gcd(a: i64, b: i64) -> i64 {
    if b == 0 { a } else { gcd(b, a % b) }
}

fn line_key(sheet: Id, a: Point, b: Point) -> LineKey {
    let d = b - a;
    let g = gcd(d.x.abs(), d.y.abs()).max(1);
    let (mut dx, mut dy) = (d.x / g, d.y / g);
    if dx < 0 || (dx == 0 && dy < 0) {
        (dx, dy) = (-dx, -dy);
    }
    (sheet, dx, dy, offset(dx, dy, a))
}

fn offset(dx: i64, dy: i64, p: Point) -> i128 {
    dx as i128 * p.y as i128 - dy as i128 * p.x as i128
}

/// Position of `p` along a line of direction (`dx`, `dy`).
fn along(key: &LineKey, p: Point) -> i128 {
    key.1 as i128 * p.x as i128 + key.2 as i128 * p.y as i128
}

/// Points where something may attach to a segment of `kind`, from one object (before or
/// after a change).
fn points_of(kind: Kind, o: &Object) -> Vec<(Id, Point)> {
    match (kind, o) {
        (Kind::Wire, Object::Wire(w)) => vec![(w.sheet, w.a), (w.sheet, w.b)],
        (Kind::Bus, Object::BusWire(w)) => vec![(w.sheet, w.a), (w.sheet, w.b)],
        (Kind::Wire, Object::BusEntry(e)) => vec![(e.sheet, e.wire_end())],
        (Kind::Bus, Object::BusEntry(e)) => vec![(e.sheet, e.pos)],
        (_, Object::Junction(j)) => vec![(j.sheet, j.pos)],
        (_, Object::Label(l)) => vec![(l.sheet, l.pos)],
        _ => Vec::new(),
    }
}

/// The segments of one kind on the sheets involved, indexed by line and by end.
struct Segments {
    kind: Kind,
    segs: HashMap<Id, Seg>,
    lines: HashMap<LineKey, Vec<Id>>,
    /// Lines neither horizontal nor vertical, per sheet (searched by scanning).
    slanted: HashMap<Id, HashSet<LineKey>>,
    ends: HashMap<(Id, Point), Vec<Id>>,
}

impl Segments {
    fn index(&mut self, s: Seg) {
        let key = line_key(s.sheet, s.a, s.b);
        self.lines.entry(key).or_default().push(s.id);
        if key.1 != 0 && key.2 != 0 {
            self.slanted.entry(s.sheet).or_default().insert(key);
        }
        for p in [s.a, s.b] {
            self.ends.entry((s.sheet, p)).or_default().push(s.id);
        }
        self.segs.insert(s.id, s);
    }

    fn unindex(&mut self, id: Id) -> Option<Seg> {
        let s = self.segs.remove(&id)?;
        if let Some(v) = self.lines.get_mut(&line_key(s.sheet, s.a, s.b)) {
            v.retain(|&i| i != id);
        }
        for p in [s.a, s.b] {
            if let Some(v) = self.ends.get_mut(&(s.sheet, p)) {
                v.retain(|&i| i != id);
            }
        }
        Some(s)
    }

    fn remove(&mut self, tx: &mut Transaction, id: Id) -> Result<(), CommandError> {
        self.unindex(id);
        tx.remove(self.kind.object_id(id)).map(drop)
    }

    fn set(&mut self, tx: &mut Transaction, id: Id, a: Point, b: Point) -> Result<(), CommandError> {
        let Some(s) = self.unindex(id) else { return Ok(()) };
        tx.update(self.kind.object_id(id), |o| {
            match o {
                Object::Wire(w) => (w.a, w.b) = (a, b),
                Object::BusWire(w) => (w.a, w.b) = (a, b),
                _ => {}
            }
            Ok(())
        })?;
        self.index(Seg { a, b, ..s });
        Ok(())
    }

    fn insert(&mut self, tx: &mut Transaction, sheet: Id, a: Point, b: Point) -> Result<Id, CommandError> {
        let s = Seg { id: Id::new(), sheet, a, b };
        tx.insert(self.kind.object(s.clone()))?;
        let id = s.id;
        self.index(s);
        Ok(id)
    }

    /// The lines through `p` that segments lie on.
    fn lines_through(&self, sheet: Id, p: Point) -> Vec<LineKey> {
        let mut out = vec![line_key(sheet, p, p + Point::new(1, 0)), line_key(sheet, p, p + Point::new(0, 1))];
        if let Some(keys) = self.slanted.get(&sheet) {
            out.extend(keys.iter().filter(|k| offset(k.1, k.2, p) == k.3).copied());
        }
        out
    }

    /// Segments with `p` on their inside.
    fn through(&self, sheet: Id, p: Point) -> Vec<Id> {
        let mut out = Vec::new();
        for key in self.lines_through(sheet, p) {
            for id in self.lines.get(&key).map_or(&[][..], Vec::as_slice) {
                let s = &self.segs[id];
                if on_segment_inside(p, s.a, s.b) {
                    out.push(*id);
                }
            }
        }
        out
    }
}

fn tidy_kind(tx: &mut Transaction, kind: Kind, keep: &HashSet<Id>) -> Result<(), CommandError> {
    // What the command changed: segments, and points where something came or went.
    let mut seeds: Vec<Id> = Vec::new();
    let mut points: HashSet<(Id, Point)> = HashSet::new();
    let mut fresh: HashSet<Id> = HashSet::new();
    let mut changed: HashSet<Id> = HashSet::new();
    for c in tx.changes() {
        let objects = c.before.iter().map(|(_, o)| o).chain(&c.after);
        for o in objects {
            points.extend(points_of(kind, o));
        }
        if let Some((_, id)) = Kind::of(c.id).filter(|(k, _)| *k == kind)
            && c.after.is_some()
        {
            seeds.push(id);
            if c.before.is_none() {
                fresh.insert(id);
            } else {
                changed.insert(id);
            }
        }
    }
    if points.is_empty() {
        return Ok(());
    }
    let rank = |id: Id| {
        if keep.contains(&id) {
            0
        } else if fresh.contains(&id) {
            3
        } else if changed.contains(&id) {
            2
        } else {
            1
        }
    };
    let sheets: HashSet<Id> = points.iter().map(|p| p.0).collect();
    let mut s =
        Segments { kind, segs: HashMap::new(), lines: HashMap::new(), slanted: HashMap::new(), ends: HashMap::new() };
    let region = Rect::union_all(points.iter().map(|p| Rect::from_points(p.1, p.1))).expect("points");
    for seg in kind.segments(tx.project()) {
        if sheets.contains(&seg.sheet) && seg.a != seg.b && region.intersects(&Rect::from_points(seg.a, seg.b)) {
            s.index(seg);
        }
    }
    let junctions: HashSet<(Id, Point)> = tx
        .project()
        .schematic
        .junctions
        .iter()
        .filter(|j| sheets.contains(&j.sheet) && region.contains(j.pos))
        .map(|j| (j.sheet, j.pos))
        .collect();

    // 1. Overlaps: one segment covering all on a line that overlap.
    let mut touched: Vec<Id> = Vec::new();
    let mut work = seeds.clone();
    while let Some(id) = work.pop() {
        let Some(seg) = s.segs.get(&id).cloned() else { continue };
        let key = line_key(seg.sheet, seg.a, seg.b);
        let span = |x: &Seg| {
            let (u, v) = (along(&key, x.a), along(&key, x.b));
            (u.min(v), u.max(v))
        };
        let (lo, hi) = span(&seg);
        let group: Vec<Seg> = s.lines[&key]
            .iter()
            .map(|o| s.segs[o].clone())
            .filter(|o| {
                let (olo, ohi) = span(o);
                o.id == id || olo.max(lo) < ohi.min(hi)
            })
            .collect();
        if group.len() < 2 {
            continue;
        }
        let ends = group.iter().flat_map(|g| [g.a, g.b]);
        let first = ends.clone().min_by_key(|&p| along(&key, p)).unwrap();
        let last = ends.max_by_key(|&p| along(&key, p)).unwrap();
        let winner = group.iter().min_by_key(|g| rank(g.id)).unwrap().clone();
        for g in group.iter().filter(|g| g.id != winner.id) {
            s.remove(tx, g.id)?;
        }
        let (a, b) = if along(&key, winner.a) <= along(&key, winner.b) { (first, last) } else { (last, first) };
        s.set(tx, winner.id, a, b)?;
        touched.push(winner.id);
        work.push(winner.id);
    }

    // 2. Splits where a segment end or a junction lies inside a segment.
    let mut affected: Vec<Id> = seeds.iter().chain(&touched).copied().filter(|id| s.segs.contains_key(id)).collect();
    let mut at: HashSet<(Id, Point)> = points.clone();
    for id in &affected {
        let g = &s.segs[id];
        at.extend([(g.sheet, g.a), (g.sheet, g.b)]);
    }
    for &(sheet, p) in &at {
        if junctions.contains(&(sheet, p)) || s.ends.get(&(sheet, p)).is_some_and(|v| !v.is_empty()) {
            affected.extend(s.through(sheet, p));
        }
    }
    affected.sort();
    affected.dedup();
    // Where segments may be cut (segment ends and junctions), by row and column, and per
    // sheet for slanted segments. Cutting only adds ends where these already are.
    let mut cut_points: HashSet<(Id, Point)> = junctions.clone();
    cut_points.extend(s.ends.iter().filter(|(_, v)| !v.is_empty()).map(|(k, _)| *k));
    let mut rows: HashMap<(Id, Coord), Vec<Point>> = HashMap::new();
    let mut columns: HashMap<(Id, Coord), Vec<Point>> = HashMap::new();
    let mut all: HashMap<Id, Vec<Point>> = HashMap::new();
    for &(sheet, p) in &cut_points {
        rows.entry((sheet, p.y)).or_default().push(p);
        columns.entry((sheet, p.x)).or_default().push(p);
        all.entry(sheet).or_default().push(p);
    }
    for id in affected {
        if keep.contains(&id) {
            continue;
        }
        let Some(seg) = s.segs.get(&id).cloned() else { continue };
        let key = line_key(seg.sheet, seg.a, seg.b);
        // Ends of other segments and junctions inside this one.
        let candidates = match (key.1, key.2) {
            (_, 0) => rows.get(&(seg.sheet, seg.a.y)),
            (0, _) => columns.get(&(seg.sheet, seg.a.x)),
            _ => all.get(&seg.sheet),
        };
        let mut cuts: Vec<Point> = candidates
            .map_or(&[][..], Vec::as_slice)
            .iter()
            .copied()
            .filter(|&p| on_segment_inside(p, seg.a, seg.b))
            .collect();
        if cuts.is_empty() {
            continue;
        }
        let dir = if along(&key, seg.a) <= along(&key, seg.b) { 1 } else { -1 };
        cuts.sort_by_key(|&p| dir * along(&key, p));
        cuts.dedup();
        s.set(tx, id, seg.a, cuts[0])?;
        touched.push(id);
        let mut from = cuts[0];
        for &p in cuts[1..].iter().chain([&seg.b]) {
            touched.push(s.insert(tx, seg.sheet, from, p)?);
            from = p;
        }
    }

    // 3. Joins of collinear segments meeting where nothing else is.
    for id in seeds.iter().chain(&touched) {
        if let Some(g) = s.segs.get(id) {
            points.extend([(g.sheet, g.a), (g.sheet, g.b)]);
        }
    }
    let attached = kind.attach_counts(tx.project(), &points);
    let mut work: Vec<(Id, Point)> = points.into_iter().collect();
    work.sort_by_key(|&(sh, p)| (sh, p.x, p.y));
    for (sheet, q) in work {
        if attached.get(&(sheet, q)).is_some_and(|&n| n > 0) {
            continue;
        }
        let [x, y] = s.ends.get(&(sheet, q)).map_or(&[][..], Vec::as_slice)[..] else { continue };
        let (u, v) = (s.segs[&x].clone(), s.segs[&y].clone());
        let (u_far, v_far) = (if u.a == q { u.b } else { u.a }, if v.a == q { v.b } else { v.a });
        let (du, dv) = (u_far - q, v_far - q);
        let cross = du.x as i128 * dv.y as i128 - du.y as i128 * dv.x as i128;
        let dot = du.x as i128 * dv.x as i128 + du.y as i128 * dv.y as i128;
        if cross != 0 || dot >= 0 || !s.through(sheet, q).is_empty() {
            continue;
        }
        let (win, lose) = if rank(v.id) < rank(u.id) { (v, u) } else { (u, v) };
        let lose_far = if lose.a == q { lose.b } else { lose.a };
        let (a, b) = if win.a == q { (lose_far, win.b) } else { (win.a, lose_far) };
        s.remove(tx, lose.id)?;
        s.set(tx, win.id, a, b)?;
    }
    Ok(())
}
