//! Keep tracks attached while board objects move (the board side of
//! [`rubber_band`](crate::rubber_band)). Planned from the geometry before the move.
//!
//! * A track end inside the copper of a moved pad or via (on the track's layer) moves by
//!   the same transform. On a translation of a straight track, the whole track moves
//!   along, keeping its direction, and its far end slides along the line of the next
//!   track, if exactly one straight, non-parallel track continues there and nothing else
//!   is there. Otherwise the track stretches.
//! * An end of a moved track that sat on copper staying put slides the same way along a
//!   single straight, non-parallel neighbour track (keeping 45° and 90° corners), or a
//!   new track joins it back to the old point (on a pad, a via, a branch, or next to a
//!   parallel or arc neighbour).
//!
//! Arc tracks keep their bulge when their ends move (see [`Track::set_ends`]).

use std::collections::{HashMap, HashSet};

use model::*;

use crate::CommandError;
use crate::copper::{Geo, LayerMask, footprint_copper, layer_bit, via_geo, via_layers};
use crate::transaction::Transaction;

/// Grid cell size for indexing track ends (1 mm).
const CELL: Coord = 1_000_000;

fn cell(p: Point) -> (Coord, Coord) {
    (p.x.div_euclid(CELL), p.y.div_euclid(CELL))
}

pub(crate) struct BoardPlan {
    /// Unmoved tracks with ends on moved copper: (track, end `a` moves, end `b` moves,
    /// the neighbour the far end slides along).
    followers: Vec<(Id, bool, bool, Option<Id>)>,
    /// Ends of moved tracks on copper staying put: (track, end `a`?, point, the neighbour
    /// to slide along).
    held: Vec<(Id, bool, Point, Option<Id>)>,
}

/// Plan how tracks stay attached when `ids` move by `t`.
pub(crate) fn plan(p: &Project, ids: &[ObjectId], t: &Transform) -> BoardPlan {
    let b = &p.board;
    let mut plan = BoardPlan { followers: Vec::new(), held: Vec::new() };
    let moved_tracks: HashSet<Id> =
        ids.iter().filter_map(|id| if let ObjectId::Track(i) = id { Some(*i) } else { None }).collect();
    let moved_vias: HashSet<Id> =
        ids.iter().filter_map(|id| if let ObjectId::Via(i) = id { Some(*i) } else { None }).collect();
    let moved_fps: HashSet<Id> =
        ids.iter().filter_map(|id| if let ObjectId::Footprint(i) = id { Some(*i) } else { None }).collect();
    if moved_tracks.is_empty() && moved_vias.is_empty() && moved_fps.is_empty() {
        return plan;
    }
    let translation = (!t.mirror && t.rotation.degrees() == 0.0).then_some(t.offset);

    // Copper: moving and staying.
    let mut moving: Vec<(LayerMask, Geo)> = Vec::new();
    let mut staying: Vec<(LayerMask, Geo)> = Vec::new();
    for f in b.footprints.iter() {
        let into = if moved_fps.contains(&f.id) && !f.locked { &mut moving } else { &mut staying };
        // Staying copper is only needed around moved tracks.
        if !moved_fps.contains(&f.id) && moved_tracks.is_empty() {
            continue;
        }
        into.extend(footprint_copper(p, f).into_iter().map(|(_, l, g)| (l, g)));
    }
    for v in b.vias.iter() {
        let entry = (via_layers(b, v), via_geo(v));
        if moved_vias.contains(&v.id) {
            moving.push(entry);
        } else if !moved_tracks.is_empty() {
            staying.push(entry);
        }
    }
    let on = |copper: &[(LayerMask, Geo)], layer: LayerId, q: Point| {
        let bit = layer_bit(layer);
        copper.iter().any(|(l, g)| l & bit != 0 && g.bounds().contains(q) && g.contains(q))
    };

    // Track ends: exact (layer, point) and by grid cell.
    let mut ends: HashMap<(LayerId, Point), Vec<Id>> = HashMap::new();
    let mut cells: HashMap<(Coord, Coord), Vec<(Id, bool)>> = HashMap::new();
    for tr in b.tracks.iter() {
        ends.entry((tr.layer, tr.a)).or_default().push(tr.id);
        ends.entry((tr.layer, tr.b)).or_default().push(tr.id);
        if !moved_tracks.contains(&tr.id) {
            cells.entry(cell(tr.a)).or_default().push((tr.id, true));
            cells.entry(cell(tr.b)).or_default().push((tr.id, false));
        }
    }

    // The neighbour track `me`'s end at `q` slides along: the single other track there,
    // straight, not parallel to `dir` and staying put (`stays`), with no copper there.
    let slide_along = |me: Id, layer: LayerId, q: Point, dir: Point, stays: &dyn Fn(Id) -> bool| -> Option<Id> {
        let others: Vec<Id> = ends.get(&(layer, q))?.iter().copied().filter(|&i| i != me).collect();
        let [n] = others[..] else { return None };
        let nt = b.tracks.get(n).filter(|_| stays(n))?;
        if nt.mid.is_some() || on(&staying, layer, q) || on(&moving, layer, q) {
            return None;
        }
        let nd = nt.b - nt.a;
        let cross = dir.x as i128 * nd.y as i128 - dir.y as i128 * nd.x as i128;
        (cross != 0).then_some(n)
    };

    // Followers: unmoved track ends on moving copper.
    let mut follow: HashMap<Id, (bool, bool)> = HashMap::new();
    for (_, geo) in &moving {
        let r = geo.bounds();
        let (lo, hi) = (cell(r.min), cell(r.max));
        for cx in lo.0..=hi.0 {
            for cy in lo.1..=hi.1 {
                for &(id, is_a) in cells.get(&(cx, cy)).into_iter().flatten() {
                    let Some(tr) = b.tracks.get(id) else { continue };
                    let q = if is_a { tr.a } else { tr.b };
                    if on(&moving, tr.layer, q) {
                        let e = follow.entry(id).or_default();
                        if is_a { e.0 = true } else { e.1 = true }
                    }
                }
            }
        }
    }
    let mut followers: Vec<(Id, bool, bool, Option<Id>)> = Vec::new();
    for (&id, &(a, bb)) in &follow {
        let Some(tr) = b.tracks.get(id) else { continue };
        let slide = match (translation, a, bb, tr.mid) {
            (Some(_), true, false, None) => slide_along(id, tr.layer, tr.b, tr.b - tr.a, &|i| {
                !follow.contains_key(&i) && !moved_tracks.contains(&i)
            }),
            (Some(_), false, true, None) => slide_along(id, tr.layer, tr.a, tr.b - tr.a, &|i| {
                !follow.contains_key(&i) && !moved_tracks.contains(&i)
            }),
            _ => None,
        };
        followers.push((id, a, bb, slide));
    }
    followers.sort_by_key(|f| f.0);
    plan.followers = followers;

    // Held ends of moved tracks.
    for tr in b.tracks.iter().filter(|tr| moved_tracks.contains(&tr.id)) {
        for (is_a, q) in [(true, tr.a), (false, tr.b)] {
            let at = ends.get(&(tr.layer, q)).map_or(&[][..], Vec::as_slice);
            if at.iter().any(|&i| i != tr.id && moved_tracks.contains(&i)) || on(&moving, tr.layer, q) {
                continue; // moves rigidly
            }
            let neighbours = at.iter().filter(|&&i| i != tr.id).count();
            if neighbours == 0 && !on(&staying, tr.layer, q) {
                continue; // dangling
            }
            let slide = match (translation, tr.mid) {
                (Some(_), None) => slide_along(tr.id, tr.layer, q, tr.b - tr.a, &|i| !moved_tracks.contains(&i)),
                _ => None,
            };
            plan.held.push((tr.id, is_a, q, slide));
        }
    }
    plan
}

/// Carry out `plan` after the objects moved by `t`.
pub(crate) fn apply(tx: &mut Transaction, plan: BoardPlan, t: &Transform) -> Result<(), CommandError> {
    for (id, is_a, q, slide) in plan.held {
        let Some(tr) = tx.project().board.tracks.get(id).cloned() else { continue };
        let moved_end = if is_a { tr.a } else { tr.b };
        let other = if is_a { tr.b } else { tr.a };
        if let Some(n) = slide
            && let Some(nt) = tx.project().board.tracks.get(n).cloned()
            && let Some(x) = slide_point(moved_end, other, q, &nt)
        {
            set_end(tx, &tr, is_a, x)?;
            let n_a = nt.a == q;
            set_end(tx, &nt, n_a, x)?;
            continue;
        }
        if moved_end != q {
            tx.insert(Object::Track(Track::straight(tr.layer, q, moved_end, tr.width)))?;
        }
    }
    for (id, a_moves, b_moves, slide) in plan.followers {
        let Some(tr) = tx.project().board.tracks.get(id).cloned() else { continue };
        let mut new = tr.clone();
        if a_moves && b_moves {
            new.a = t.point(tr.a);
            new.b = t.point(tr.b);
            new.mid = tr.mid.map(|m| t.point(m));
        } else {
            let (q, far) = if a_moves { (tr.a, tr.b) } else { (tr.b, tr.a) };
            let to = t.point(q);
            let far_to = slide.and_then(|n| {
                let nt = tx.project().board.tracks.get(n)?.clone();
                // The track keeps its direction: its far end moves to where the shifted
                // line meets the neighbour.
                let x = slide_point(far + (to - q), to, far, &nt)?;
                Some((nt, x))
            });
            match far_to {
                Some((nt, x)) => {
                    let n_a = nt.a == far;
                    set_end(tx, &nt, n_a, x)?;
                    if a_moves { new.set_ends(to, x) } else { new.set_ends(x, to) }
                }
                None if a_moves => new.set_ends(to, far),
                None => new.set_ends(far, to),
            }
        }
        if new.a == new.b {
            tx.remove(ObjectId::Track(id))?;
        } else if new != tr {
            tx.replace(Object::Track(new))?;
        }
    }
    Ok(())
}

/// Where a straight track, whose end moved to `end` (its other end at `other`), meets
/// the line of neighbour `n` that ended at `q`: `None` if the lines are parallel or the
/// track or the neighbour would turn around.
fn slide_point(end: Point, other: Point, q: Point, n: &Track) -> Option<Point> {
    let n_far = if n.a == q { n.b } else { n.a };
    let (p1, u) = (other, end - other);
    let (p2, v) = (n_far, q - n_far);
    let f = |p: Point| (p.x as f64, p.y as f64);
    let (u, v, p1, p2) = (f(u), f(v), f(p1), f(p2));
    let cross = u.0 * v.1 - u.1 * v.0;
    if cross == 0.0 {
        return None;
    }
    let d = (p2.0 - p1.0, p2.1 - p1.1);
    let s = (d.0 * v.1 - d.1 * v.0) / cross;
    let r = (d.0 * u.1 - d.1 * u.0) / cross;
    // Both keep their direction and some length.
    if s <= 0.0 || r <= 0.0 {
        return None;
    }
    Some(Point::new((p1.0 + s * u.0).round() as Coord, (p1.1 + s * u.1).round() as Coord))
}

fn set_end(tx: &mut Transaction, tr: &Track, is_a: bool, to: Point) -> Result<(), CommandError> {
    let Some(cur) = tx.project().board.tracks.get(tr.id).cloned() else { return Ok(()) };
    let mut new = cur.clone();
    if is_a {
        new.set_ends(to, cur.b)
    } else {
        new.set_ends(cur.a, to)
    }
    if new.a == new.b {
        tx.remove(ObjectId::Track(tr.id))?;
        return Ok(());
    }
    tx.replace(Object::Track(new))
}
