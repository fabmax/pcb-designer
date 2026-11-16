//! Rounding the corner between two straight legs with a tangent arc.

use model::*;

/// The arc rounding the corner at `c` of a path `a` → `c` → `b`: its start (on the leg
/// from `a`), a point in its middle, and its end (on the leg to `b`). With `radius`
/// `None`, the largest arc the shorter leg allows. `None` if the legs are in line, turn
/// back on themselves, or are too short for the radius.
pub fn fillet(a: Point, c: Point, b: Point, radius: Option<f64>) -> Option<(Point, Point, Point)> {
    let f = |p: Point| (p.x as f64, p.y as f64);
    let (a, c, b) = (f(a), f(c), f(b));
    let (l1, l2) = (((c.0 - a.0).hypot(c.1 - a.1)), ((b.0 - c.0).hypot(b.1 - c.1)));
    if l1 == 0.0 || l2 == 0.0 {
        return None;
    }
    let u1 = ((c.0 - a.0) / l1, (c.1 - a.1) / l1);
    let u2 = ((b.0 - c.0) / l2, (b.1 - c.1) / l2);
    let cross = u1.0 * u2.1 - u1.1 * u2.0;
    let dot = u1.0 * u2.0 + u1.1 * u2.1;
    // The angle the path turns by.
    let turn = cross.atan2(dot);
    if turn.abs() < 1e-6 || turn.abs() > std::f64::consts::PI - 1e-3 {
        return None;
    }
    let half_tan = (turn.abs() / 2.0).tan();
    let (r, t) = match radius {
        Some(r) => (r, r * half_tan),
        None => {
            let t = l1.min(l2);
            (t / half_tan, t)
        }
    };
    // Some slack for rounding to nanometres.
    if r <= 0.0 || t > l1 + 0.5 || t > l2 + 0.5 {
        return None;
    }
    let p1 = (c.0 - u1.0 * t, c.1 - u1.1 * t);
    let p2 = (c.0 + u2.0 * t, c.1 + u2.1 * t);
    // The centre lies to the side the path turns to.
    let s = turn.signum();
    let o = (p1.0 - u1.1 * r * s, p1.1 + u1.0 * r * s);
    let (dx, dy) = (c.0 - o.0, c.1 - o.1);
    let d = dx.hypot(dy);
    let mid = (o.0 + dx / d * r, o.1 + dy / d * r);
    let p = |q: (f64, f64)| Point::new(q.0.round() as Coord, q.1.round() as Coord);
    Some((p(p1), p(mid), p(p2)))
}

/// A corner between two straight tracks: they end at `at` on the same layer, and nothing
/// else is there (no other track end, pad or via).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Corner {
    pub at: Point,
    pub layer: LayerId,
    pub tracks: [Id; 2],
}

/// The corners between `tracks` (straight ones; arcs are left out).
pub fn corners(project: &Project, tracks: &[Id]) -> Vec<Corner> {
    use std::collections::{HashMap, HashSet};
    let b = &project.board;
    let chosen: HashSet<Id> = tracks.iter().copied().collect();
    let mut ends: HashMap<(LayerId, Point), Vec<(Id, bool)>> = HashMap::new();
    for t in b.tracks.iter() {
        for q in [t.a, t.b] {
            ends.entry((t.layer, q)).or_default().push((t.id, chosen.contains(&t.id) && t.mid.is_none()));
        }
    }
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for &id in tracks {
        let Some(t) = b.tracks.get(id).filter(|t| t.mid.is_none()) else { continue };
        for q in [t.a, t.b] {
            let key = (t.layer, q);
            let Some(at) = ends.get(&key) else { continue };
            let [(x, true), (y, true)] = at[..] else { continue };
            if !seen.insert(key) || on_pad_or_via(project, t.layer, q) {
                continue;
            }
            out.push(Corner { at: q, layer: t.layer, tracks: [x, y] });
        }
    }
    out
}

fn on_pad_or_via(project: &Project, layer: LayerId, q: Point) -> bool {
    use crate::copper::{footprint_copper, layer_bit, via_geo, via_layers};
    let b = &project.board;
    let bit = layer_bit(layer);
    b.vias.iter().any(|v| via_layers(b, v) & bit != 0 && via_geo(v).contains(q))
        || b.footprints.iter().any(|f| {
            footprint_copper(project, f).iter().any(|(_, l, g)| l & bit != 0 && g.bounds().contains(q) && g.contains(q))
        })
}

/// Round `corner` with an arc of `radius`: both tracks get shorter and an arc track joins
/// them. Returns false (changing nothing) if the legs are too short.
pub(crate) fn round(
    tx: &mut crate::transaction::Transaction,
    corner: &Corner,
    radius: Coord,
) -> Result<bool, crate::CommandError> {
    let b = &tx.project().board;
    let (Some(t1), Some(t2)) = (b.tracks.get(corner.tracks[0]).cloned(), b.tracks.get(corner.tracks[1]).cloned())
    else {
        return Ok(false);
    };
    let far = |t: &Track| {
        if t.a == corner.at {
            Some(t.b)
        } else if t.b == corner.at {
            Some(t.a)
        } else {
            None
        }
    };
    let (Some(f1), Some(f2)) = (far(&t1), far(&t2)) else { return Ok(false) };
    let Some((p1, mid, p2)) = fillet(f1, corner.at, f2, Some(radius as f64)) else { return Ok(false) };
    for (t, p) in [(&t1, p1), (&t2, p2)] {
        let mut n = t.clone();
        if n.a == corner.at {
            n.a = p
        } else {
            n.b = p
        }
        if n.a == n.b {
            tx.remove(ObjectId::Track(t.id))?;
        } else {
            tx.replace(Object::Track(n))?;
        }
    }
    let mut arc = Track::straight(corner.layer, p1, p2, t1.width.min(t2.width));
    arc.mid = Some(mid);
    tx.insert(Object::Track(arc))?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quarter_arc() {
        let (a, c, b) = (Point::mm(0.0, 0.0), Point::mm(10.0, 0.0), Point::mm(10.0, 4.0));
        let (p1, mid, p2) = fillet(a, c, b, None).unwrap();
        // Largest: the shorter leg (4 mm) is used up.
        assert_eq!((p1, p2), (Point::mm(6.0, 0.0), Point::mm(10.0, 4.0)));
        let r = 4.0 * (1.0 - std::f64::consts::FRAC_1_SQRT_2);
        let want = Point::mm(6.0 + 4.0 * std::f64::consts::FRAC_1_SQRT_2, r);
        assert!((mid - want).length() < 2.0, "{mid:?} vs {want:?}");
        // A given radius, turning the other way.
        let (p1, _, p2) = fillet(a, c, Point::mm(10.0, -5.0), Some(1e6)).unwrap();
        assert_eq!((p1, p2), (Point::mm(9.0, 0.0), Point::mm(10.0, -1.0)));
        assert!(fillet(a, c, b, Some(5e6)).is_none(), "legs too short");
        assert!(fillet(a, c, Point::mm(20.0, 0.0), None).is_none(), "no corner");
    }

    #[test]
    fn rounded_45_degree_corner() {
        let (a, c, b) = (Point::mm(0.0, 0.0), Point::mm(10.0, 0.0), Point::mm(13.0, 3.0));
        let (p1, _, p2) = fillet(a, c, b, None).unwrap();
        // Tangent length = the diagonal leg (3√2).
        let t = 3.0 * std::f64::consts::SQRT_2;
        assert!((p1 - Point::mm(10.0 - t, 0.0)).length() < 2.0);
        assert!((p2 - b).length() < 2.0);
    }
}
