//! Keep schematic wires, bus wires and board tracks in a tidy shape after every command,
//! around what the command changed:
//!
//! 1. Segments lying on top of each other on the same line become one, covering both
//!    (overlapping segments always connect, so they're always of one net).
//! 2. A segment is split where another segment's end, a junction or a via lies on its
//!    inside (a T connection), so each piece ends where something connects.
//! 3. Collinear segments meeting end to end with nothing else at the shared point become
//!    one: no other segment and no T onto it; for wires no pin, label, junction, entry or
//!    sheet pin; for tracks no via or pad covering it.
//!
//! Tracks take part on their layer, and only straight ones: arcs are never split or
//! joined (their ends still count as connections). Tracks become one only with the same
//! width and lock; tracks of different widths stay stacked.
//!
//! The segments `keep` (those a move moves) keep their ids and are never split: a drag
//! replays its edits with the ids it started with. When segments become one, the one kept
//! is from `keep`, else one the command didn't touch, else one it changed, else a new one.
//!
//! Only segments reaching into the box around the changed points take part (a new tee,
//! overlap or join always involves one of them), indexed by line and by end, so a small
//! edit on a big sheet or board stays cheap.

use std::collections::{HashMap, HashSet};

use model::*;

use crate::CommandError;
use crate::connectivity::on_segment_inside;
use crate::copper::{Geo, footprint_copper, layer_bit, via_geo, via_layers};
use crate::rubber_band::Kind;
use crate::transaction::Transaction;

/// Tidy the wires, bus wires and tracks around what the transaction changed so far.
pub(crate) fn tidy(tx: &mut Transaction, keep: &HashSet<Id>) -> Result<(), CommandError> {
    for family in [Family::Wire, Family::Bus, Family::Track] {
        tidy_family(tx, family, keep)?;
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Family {
    Wire,
    Bus,
    Track,
}

/// Where segments meet: a sheet (wires, buses) or a copper layer (tracks).
type Space = Id;

fn layer_space(l: LayerId) -> Space {
    Id::from_u128(l.0 as u128 + 1)
}

/// A segment as the tidy sees it.
#[derive(Clone, Debug)]
struct Seg {
    id: Id,
    space: Space,
    a: Point,
    b: Point,
    /// What must match for segments to become one (tracks: width and lock).
    class: (Coord, bool),
    /// Arcs only count as ends.
    straight: bool,
}

impl Family {
    fn object_id(self, id: Id) -> ObjectId {
        match self {
            Family::Wire => ObjectId::Wire(id),
            Family::Bus => ObjectId::BusWire(id),
            Family::Track => ObjectId::Track(id),
        }
    }

    fn of(self, id: ObjectId) -> Option<Id> {
        match (self, id) {
            (Family::Wire, ObjectId::Wire(i))
            | (Family::Bus, ObjectId::BusWire(i))
            | (Family::Track, ObjectId::Track(i)) => Some(i),
            _ => None,
        }
    }

    fn rubber(self) -> Option<Kind> {
        match self {
            Family::Wire => Some(Kind::Wire),
            Family::Bus => Some(Kind::Bus),
            Family::Track => None,
        }
    }

    /// The segments in `spaces` reaching into `region`.
    fn segments(self, p: &Project, spaces: &HashSet<Space>, region: Rect) -> Vec<Seg> {
        let s = &p.schematic;
        let wire = |id, sheet, a, b| Seg { id, space: sheet, a, b, class: (0, false), straight: true };
        let all: Vec<Seg> = match self {
            Family::Wire => s.wires.iter().map(|w| wire(w.id, w.sheet, w.a, w.b)).collect(),
            Family::Bus => s.bus_wires.iter().map(|w| wire(w.id, w.sheet, w.a, w.b)).collect(),
            Family::Track => p
                .board
                .tracks
                .iter()
                .filter(|t| {
                    let r = Rect::from_points(t.a, t.b);
                    region.intersects(&t.mid.map_or(r, |m| r.union(Rect::from_points(m, m))))
                })
                .map(|t| Seg {
                    id: t.id,
                    space: layer_space(t.layer),
                    a: t.a,
                    b: t.b,
                    class: (t.width, t.locked),
                    straight: t.mid.is_none(),
                })
                .collect(),
        };
        all.into_iter()
            .filter(|g| {
                spaces.contains(&g.space)
                    && g.a != g.b
                    && (!g.straight || region.intersects(&Rect::from_points(g.a, g.b)))
            })
            .collect()
    }

    /// Points where something may attach to a segment, from one object (before or after
    /// a change).
    fn points_of(self, p: &Project, o: &Object) -> Vec<(Space, Point)> {
        match (self, o) {
            (Family::Wire, Object::Wire(w)) => vec![(w.sheet, w.a), (w.sheet, w.b)],
            (Family::Bus, Object::BusWire(w)) => vec![(w.sheet, w.a), (w.sheet, w.b)],
            (Family::Wire, Object::BusEntry(e)) => vec![(e.sheet, e.wire_end())],
            (Family::Bus, Object::BusEntry(e)) => vec![(e.sheet, e.pos)],
            (Family::Wire | Family::Bus, Object::Junction(j)) => vec![(j.sheet, j.pos)],
            (Family::Wire | Family::Bus, Object::Label(l)) => vec![(l.sheet, l.pos)],
            (Family::Track, Object::Track(t)) => vec![(layer_space(t.layer), t.a), (layer_space(t.layer), t.b)],
            (Family::Track, Object::Via(v)) => via_spaces(p, v).into_iter().map(|s| (s, v.pos)).collect(),
            _ => Vec::new(),
        }
    }
}

fn via_spaces(p: &Project, v: &Via) -> Vec<Space> {
    let mask = via_layers(&p.board, v);
    p.board.copper_layers().filter(|l| mask & layer_bit(l.id) != 0).map(|l| layer_space(l.id)).collect()
}

/// The line a segment lies on: space, direction reduced to lowest terms (pointing right,
/// or up when vertical), and offset (the same for every point of the line).
type LineKey = (Space, i64, i64, i128);

fn gcd(a: i64, b: i64) -> i64 {
    if b == 0 { a } else { gcd(b, a % b) }
}

fn line_key(space: Space, a: Point, b: Point) -> LineKey {
    let d = b - a;
    let g = gcd(d.x.abs(), d.y.abs()).max(1);
    let (mut dx, mut dy) = (d.x / g, d.y / g);
    if dx < 0 || (dx == 0 && dy < 0) {
        (dx, dy) = (-dx, -dy);
    }
    (space, dx, dy, offset(dx, dy, a))
}

fn offset(dx: i64, dy: i64, p: Point) -> i128 {
    dx as i128 * p.y as i128 - dy as i128 * p.x as i128
}

/// Position of `p` along a line of direction (`dx`, `dy`).
fn along(key: &LineKey, p: Point) -> i128 {
    key.1 as i128 * p.x as i128 + key.2 as i128 * p.y as i128
}

/// The segments of one family in the spaces involved, indexed by line (straight ones)
/// and by end.
struct Segments {
    family: Family,
    segs: HashMap<Id, Seg>,
    lines: HashMap<LineKey, Vec<Id>>,
    /// Lines neither horizontal nor vertical, per space (searched by scanning).
    slanted: HashMap<Space, HashSet<LineKey>>,
    ends: HashMap<(Space, Point), Vec<Id>>,
}

impl Segments {
    fn index(&mut self, s: Seg) {
        if s.straight {
            let key = line_key(s.space, s.a, s.b);
            self.lines.entry(key).or_default().push(s.id);
            if key.1 != 0 && key.2 != 0 {
                self.slanted.entry(s.space).or_default().insert(key);
            }
        }
        for p in [s.a, s.b] {
            self.ends.entry((s.space, p)).or_default().push(s.id);
        }
        self.segs.insert(s.id, s);
    }

    fn unindex(&mut self, id: Id) -> Option<Seg> {
        let s = self.segs.remove(&id)?;
        if let Some(v) = self.lines.get_mut(&line_key(s.space, s.a, s.b)) {
            v.retain(|&i| i != id);
        }
        for p in [s.a, s.b] {
            if let Some(v) = self.ends.get_mut(&(s.space, p)) {
                v.retain(|&i| i != id);
            }
        }
        Some(s)
    }

    fn remove(&mut self, tx: &mut Transaction, id: Id) -> Result<(), CommandError> {
        self.unindex(id);
        tx.remove(self.family.object_id(id)).map(drop)
    }

    fn set(&mut self, tx: &mut Transaction, id: Id, a: Point, b: Point) -> Result<(), CommandError> {
        let Some(s) = self.unindex(id) else { return Ok(()) };
        tx.update(self.family.object_id(id), |o| {
            match o {
                Object::Wire(w) => (w.a, w.b) = (a, b),
                Object::BusWire(w) => (w.a, w.b) = (a, b),
                Object::Track(t) => (t.a, t.b) = (a, b),
                _ => {}
            }
            Ok(())
        })?;
        self.index(Seg { a, b, ..s });
        Ok(())
    }

    /// A new segment like `like` (its space and class) from `a` to `b`.
    fn insert(&mut self, tx: &mut Transaction, like: &Seg, a: Point, b: Point) -> Result<Id, CommandError> {
        let id = Id::new();
        let object = match self.family {
            Family::Wire => Object::Wire(Wire { id, sheet: like.space, a, b }),
            Family::Bus => Object::BusWire(BusWire { id, sheet: like.space, a, b }),
            Family::Track => {
                let t =
                    tx.project().board.tracks.get(like.id).ok_or(CommandError::NotFound(ObjectId::Track(like.id)))?;
                Object::Track(Track { id, a, b, mid: None, ..t.clone() })
            }
        };
        tx.insert(object)?;
        self.index(Seg { id, a, b, ..like.clone() });
        Ok(id)
    }

    /// The lines through `p` that straight segments lie on.
    fn lines_through(&self, space: Space, p: Point) -> Vec<LineKey> {
        let mut out = vec![line_key(space, p, p + Point::new(1, 0)), line_key(space, p, p + Point::new(0, 1))];
        if let Some(keys) = self.slanted.get(&space) {
            out.extend(keys.iter().filter(|k| offset(k.1, k.2, p) == k.3).copied());
        }
        out
    }

    /// Straight segments with `p` on their inside.
    fn through(&self, space: Space, p: Point) -> Vec<Id> {
        let mut out = Vec::new();
        for key in self.lines_through(space, p) {
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

/// Pads (on the board) near `region`: their layers and copper.
fn pads_near(p: &Project, region: Rect) -> Vec<(u64, Geo)> {
    let reach = p
        .library
        .footprints
        .iter()
        .flat_map(|f| f.pads.iter())
        .map(|pad| pad.pos.x.abs() + pad.pos.y.abs() + pad.size.w.max(pad.size.h))
        .max()
        .unwrap_or(0);
    let near = region.grow(reach);
    p.board
        .footprints
        .iter()
        .filter(|f| near.contains(f.pos))
        .flat_map(|f| footprint_copper(p, f))
        .map(|(_, layers, geo)| (layers, geo))
        .collect()
}

fn tidy_family(tx: &mut Transaction, family: Family, keep: &HashSet<Id>) -> Result<(), CommandError> {
    // What the command changed: segments, and points where something came or went.
    let mut seeds: Vec<Id> = Vec::new();
    let mut points: HashSet<(Space, Point)> = HashSet::new();
    let mut fresh: HashSet<Id> = HashSet::new();
    let mut changed: HashSet<Id> = HashSet::new();
    let p = tx.project();
    for c in tx.changes() {
        for o in c.before.iter().map(|(_, o)| o).chain(&c.after) {
            points.extend(family.points_of(p, o));
        }
        if let Some(id) = family.of(c.id)
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
    let spaces: HashSet<Space> = points.iter().map(|p| p.0).collect();
    let region = Rect::union_all(points.iter().map(|p| Rect::from_points(p.1, p.1))).expect("points");
    let mut s =
        Segments { family, segs: HashMap::new(), lines: HashMap::new(), slanted: HashMap::new(), ends: HashMap::new() };
    for seg in family.segments(p, &spaces, region) {
        s.index(seg);
    }
    // Other things segments are cut at: junctions (wires, buses), vias (tracks).
    let mut nodes: HashSet<(Space, Point)> = HashSet::new();
    match family {
        Family::Wire | Family::Bus => nodes.extend(
            p.schematic
                .junctions
                .iter()
                .filter(|j| spaces.contains(&j.sheet) && region.contains(j.pos))
                .map(|j| (j.sheet, j.pos)),
        ),
        Family::Track => {
            for v in p.board.vias.iter().filter(|v| region.contains(v.pos)) {
                nodes.extend(via_spaces(p, v).into_iter().filter(|s| spaces.contains(s)).map(|s| (s, v.pos)));
            }
        }
    }

    // 1. Overlaps: one segment covering all on a line that overlap.
    let mut touched: Vec<Id> = Vec::new();
    let mut work = seeds.clone();
    while let Some(id) = work.pop() {
        let Some(seg) = s.segs.get(&id).cloned().filter(|g| g.straight) else { continue };
        let key = line_key(seg.space, seg.a, seg.b);
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
                o.id == id || (o.class == seg.class && olo.max(lo) < ohi.min(hi))
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

    // 2. Splits where a segment end, junction or via lies inside a straight segment.
    let mut affected: Vec<Id> = seeds.iter().chain(&touched).copied().filter(|id| s.segs.contains_key(id)).collect();
    let mut at: HashSet<(Space, Point)> = points.clone();
    for id in &affected {
        let g = &s.segs[id];
        at.extend([(g.space, g.a), (g.space, g.b)]);
    }
    for &(space, p) in &at {
        if nodes.contains(&(space, p)) || s.ends.get(&(space, p)).is_some_and(|v| !v.is_empty()) {
            affected.extend(s.through(space, p));
        }
    }
    affected.sort();
    affected.dedup();
    // Where segments may be cut, by row and column, and per space for slanted segments.
    // Cutting only adds ends where these already are.
    let mut cut_points: HashSet<(Space, Point)> = nodes.clone();
    cut_points.extend(s.ends.iter().filter(|(_, v)| !v.is_empty()).map(|(k, _)| *k));
    let mut rows: HashMap<(Space, Coord), Vec<Point>> = HashMap::new();
    let mut columns: HashMap<(Space, Coord), Vec<Point>> = HashMap::new();
    let mut all: HashMap<Space, Vec<Point>> = HashMap::new();
    for &(space, p) in &cut_points {
        rows.entry((space, p.y)).or_default().push(p);
        columns.entry((space, p.x)).or_default().push(p);
        all.entry(space).or_default().push(p);
    }
    for id in affected {
        if keep.contains(&id) {
            continue;
        }
        let Some(seg) = s.segs.get(&id).cloned().filter(|g| g.straight) else { continue };
        let key = line_key(seg.space, seg.a, seg.b);
        let candidates = match (key.1, key.2) {
            (_, 0) => rows.get(&(seg.space, seg.a.y)),
            (0, _) => columns.get(&(seg.space, seg.a.x)),
            _ => all.get(&seg.space),
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
            touched.push(s.insert(tx, &seg, from, p)?);
            from = p;
        }
    }

    // 3. Joins of collinear segments meeting where nothing else is.
    for id in seeds.iter().chain(&touched) {
        if let Some(g) = s.segs.get(id) {
            points.extend([(g.space, g.a), (g.space, g.b)]);
        }
    }
    let p = tx.project();
    let blocked: HashSet<(Space, Point)> = match family.rubber() {
        Some(kind) => kind.attach_counts(p, &points).into_iter().filter(|(_, n)| *n > 0).map(|(k, _)| k).collect(),
        None => {
            // Vias and pads covering the point on its layer.
            let mut copper: Vec<(u64, Geo)> = pads_near(p, region);
            copper.extend(p.board.vias.iter().map(|v| (via_layers(&p.board, v), via_geo(v))));
            let bit = |space: Space| {
                p.board.copper_layers().find(|l| layer_space(l.id) == space).map_or(0, |l| layer_bit(l.id))
            };
            points
                .iter()
                .filter(|&&(space, q)| {
                    let bit = bit(space);
                    copper.iter().any(|(layers, geo)| layers & bit != 0 && geo.bounds().contains(q) && geo.contains(q))
                })
                .copied()
                .collect()
        }
    };
    let mut work: Vec<(Space, Point)> = points.into_iter().filter(|k| !blocked.contains(k)).collect();
    work.sort_by_key(|&(sp, p)| (sp, p.x, p.y));
    for (space, q) in work {
        let [x, y] = s.ends.get(&(space, q)).map_or(&[][..], Vec::as_slice)[..] else { continue };
        let (u, v) = (s.segs[&x].clone(), s.segs[&y].clone());
        if !u.straight || !v.straight || u.class != v.class {
            continue;
        }
        let (u_far, v_far) = (if u.a == q { u.b } else { u.a }, if v.a == q { v.b } else { v.a });
        let (du, dv) = (u_far - q, v_far - q);
        let cross = du.x as i128 * dv.y as i128 - du.y as i128 * dv.x as i128;
        let dot = du.x as i128 * dv.x as i128 + du.y as i128 * dv.y as i128;
        if cross != 0 || dot >= 0 || !s.through(space, q).is_empty() {
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
