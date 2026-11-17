//! Corner handles of a selected zone, keepout or board drawing (polygon, rectangle,
//! line): drag a corner to move it, an edge's midpoint to insert a corner (polygons) or
//! move the edge (rectangles), and delete corners. GUI independent: the app draws the
//! handles, hit-tests them and runs the edits as `Update`s.

use model::*;

/// What a handle changes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Grip {
    /// Corner `i` of ring `ring` (0: the outline, 1…: holes).
    Corner { ring: usize, i: usize },
    /// The midpoint of the edge from corner `i` to the next: dragging inserts a corner.
    Mid { ring: usize, i: usize },
    /// A rectangle's corner (0: `a`, 1: (b.x, a.y), 2: `b`, 3: (a.x, b.y)).
    RectCorner(usize),
    /// A rectangle's edge (0: bottom, 1: right, 2: top, 3: left in `a`/`b` order).
    RectEdge(usize),
    /// A line's end (0: `a`, 1: `b`).
    LineEnd(usize),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Handle {
    pub grip: Grip,
    pub pos: Point,
}

impl Handle {
    /// A corner that can be deleted (polygon corners).
    pub fn is_corner(&self) -> bool {
        matches!(self.grip, Grip::Corner { .. })
    }
}

/// The rings (outline, then holes) of an object with polygon corners.
fn rings(o: &Object) -> Option<Vec<&Vec<Point>>> {
    match o {
        Object::Zone(z) => Some(std::iter::once(&z.outline).chain(&z.holes).collect()),
        Object::Keepout(k) => Some(std::iter::once(&k.outline).chain(&k.holes).collect()),
        Object::BoardGraphic(BoardGraphic { shape: Shape::Polygon { points, .. }, .. }) => Some(vec![points]),
        _ => None,
    }
}

fn rings_mut(o: &mut Object) -> Option<Vec<&mut Vec<Point>>> {
    match o {
        Object::Zone(z) => Some(std::iter::once(&mut z.outline).chain(z.holes.iter_mut()).collect()),
        Object::Keepout(k) => Some(std::iter::once(&mut k.outline).chain(k.holes.iter_mut()).collect()),
        Object::BoardGraphic(BoardGraphic { shape: Shape::Polygon { points, .. }, .. }) => Some(vec![points]),
        _ => None,
    }
}

fn mid(a: Point, b: Point) -> Point {
    Point::new((a.x + b.x) / 2, (a.y + b.y) / 2)
}

fn rect_corners(a: Point, b: Point) -> [Point; 4] {
    [a, Point::new(b.x, a.y), b, Point::new(a.x, b.y)]
}

/// The handles of `o` (none for objects without corners).
pub fn handles(o: &Object) -> Vec<Handle> {
    if let Some(rings) = rings(o) {
        let mut out = Vec::new();
        for (ring, pts) in rings.iter().enumerate() {
            let n = pts.len();
            for i in 0..n {
                out.push(Handle { grip: Grip::Corner { ring, i }, pos: pts[i] });
                out.push(Handle { grip: Grip::Mid { ring, i }, pos: mid(pts[i], pts[(i + 1) % n]) });
            }
        }
        return out;
    }
    match o {
        Object::BoardGraphic(BoardGraphic { shape: Shape::Rect { a, b, .. }, .. }) => {
            let c = rect_corners(*a, *b);
            let corners = (0..4).map(|i| Handle { grip: Grip::RectCorner(i), pos: c[i] });
            let edges = (0..4).map(|i| Handle { grip: Grip::RectEdge(i), pos: mid(c[i], c[(i + 1) % 4]) });
            corners.chain(edges).collect()
        }
        Object::BoardGraphic(BoardGraphic { shape: Shape::Line { a, b, .. }, .. }) => {
            vec![Handle { grip: Grip::LineEnd(0), pos: *a }, Handle { grip: Grip::LineEnd(1), pos: *b }]
        }
        _ => Vec::new(),
    }
}

/// Does the object have handles?
pub fn has_handles(o: &Object) -> bool {
    !handles(o).is_empty()
}

/// The handle at `p` (within `tol`): corners before midpoints, the nearest first.
pub fn hit(handles: &[Handle], p: Point, tol: Coord) -> Option<Handle> {
    let near = |h: &&Handle| (h.pos - p).length() <= tol as f64;
    let d = |h: &Handle| (h.pos - p).length();
    let corner = |h: &&Handle| !matches!(h.grip, Grip::Mid { .. } | Grip::RectEdge(_));
    let best = handles.iter().filter(near).filter(corner).min_by(|a, b| d(a).total_cmp(&d(b)));
    best.or_else(|| handles.iter().filter(near).min_by(|a, b| d(a).total_cmp(&d(b)))).copied()
}

/// The corners next to what `grip` moves, for 0°/45°/90° constraining.
pub fn neighbours(o: &Object, grip: Grip) -> Vec<Point> {
    match grip {
        Grip::Corner { ring, i } => rings(o)
            .and_then(|r| {
                let pts = r.get(ring)?;
                let n = pts.len();
                Some(vec![pts[(i + n - 1) % n], pts[(i + 1) % n]])
            })
            .unwrap_or_default(),
        Grip::Mid { ring, i } => rings(o)
            .and_then(|r| {
                let pts = r.get(ring)?;
                Some(vec![pts[i], pts[(i + 1) % pts.len()]])
            })
            .unwrap_or_default(),
        Grip::LineEnd(e) => match o {
            Object::BoardGraphic(BoardGraphic { shape: Shape::Line { a, b, .. }, .. }) => {
                vec![if e == 0 { *b } else { *a }]
            }
            _ => Vec::new(),
        },
        Grip::RectCorner(_) | Grip::RectEdge(_) => Vec::new(),
    }
}

/// `o` with the handle `grip` dragged to `to`. A midpoint inserts a new corner there.
/// `None` if the handle doesn't fit the object.
pub fn moved(o: &Object, grip: Grip, to: Point) -> Option<Object> {
    let mut o = o.clone();
    match grip {
        Grip::Corner { ring, i } => *rings_mut(&mut o)?.into_iter().nth(ring)?.get_mut(i)? = to,
        Grip::Mid { ring, i } => {
            let r = rings_mut(&mut o)?.into_iter().nth(ring)?;
            if i >= r.len() {
                return None;
            }
            r.insert(i + 1, to);
        }
        Grip::RectCorner(c) => {
            let Object::BoardGraphic(BoardGraphic { shape: Shape::Rect { a, b, .. }, .. }) = &mut o else {
                return None;
            };
            // The opposite corner stays.
            match c {
                0 => *a = to,
                1 => (b.x, a.y) = (to.x, to.y),
                2 => *b = to,
                3 => (a.x, b.y) = (to.x, to.y),
                _ => return None,
            }
        }
        Grip::RectEdge(e) => {
            let Object::BoardGraphic(BoardGraphic { shape: Shape::Rect { a, b, .. }, .. }) = &mut o else {
                return None;
            };
            match e {
                0 => a.y = to.y,
                1 => b.x = to.x,
                2 => b.y = to.y,
                3 => a.x = to.x,
                _ => return None,
            }
        }
        Grip::LineEnd(e) => {
            let Object::BoardGraphic(BoardGraphic { shape: Shape::Line { a, b, .. }, .. }) = &mut o else {
                return None;
            };
            if e == 0 { *a = to } else { *b = to }
        }
    }
    Some(o)
}

/// After [`moved`] with a midpoint, the inserted corner's handle (for the rest of the
/// drag).
pub fn after_insert(grip: Grip) -> Grip {
    match grip {
        Grip::Mid { ring, i } => Grip::Corner { ring, i: i + 1 },
        g => g,
    }
}

/// `o` without corner `grip`. Fails below three corners.
pub fn without_corner(o: &Object, grip: Grip) -> Result<Object, String> {
    let Grip::Corner { ring, i } = grip else { return Err("only polygon corners can be deleted".into()) };
    let mut o = o.clone();
    let r = rings_mut(&mut o).and_then(|r| r.into_iter().nth(ring)).ok_or("no such corner")?;
    if r.len() <= 3 {
        return Err("a polygon keeps at least three corners".into());
    }
    if i >= r.len() {
        return Err("no such corner".into());
    }
    r.remove(i);
    Ok(o)
}

/// A rectangle drawing as a polygon of its four corners (other objects: `None`).
pub fn rect_to_polygon(o: &Object) -> Option<Object> {
    let Object::BoardGraphic(g) = o else { return None };
    let Shape::Rect { a, b, width, fill } = g.shape else { return None };
    let shape = Shape::Polygon { points: rect_corners(a, b).to_vec(), width, fill };
    Some(Object::BoardGraphic(BoardGraphic { shape, ..g.clone() }))
}

/// `p` moved onto the nearest line at 0°, 45° or 90° through the neighbours (with two
/// neighbours: the nearest crossing of such lines).
pub fn constrain(p: Point, neighbours: &[Point]) -> Point {
    let dirs = [(1.0, 0.0), (0.0, 1.0), (1.0, 1.0), (1.0, -1.0)];
    let fp = (p.x as f64, p.y as f64);
    let lines: Vec<((f64, f64), (f64, f64))> =
        neighbours.iter().flat_map(|n| dirs.iter().map(move |d| ((n.x as f64, n.y as f64), *d))).collect();
    let dist = |q: (f64, f64)| (q.0 - fp.0).hypot(q.1 - fp.1);
    let mut candidates: Vec<(f64, f64)> = Vec::new();
    if neighbours.len() >= 2 {
        let per = dirs.len();
        for (i, (o1, d1)) in lines.iter().enumerate() {
            for (o2, d2) in lines.iter().skip((i / per + 1) * per) {
                let den = d1.0 * d2.1 - d1.1 * d2.0;
                if den.abs() < 1e-9 {
                    continue;
                }
                let t = ((o2.0 - o1.0) * d2.1 - (o2.1 - o1.1) * d2.0) / den;
                candidates.push((o1.0 + t * d1.0, o1.1 + t * d1.1));
            }
        }
    }
    if candidates.is_empty() {
        for (o, d) in &lines {
            let len2 = d.0 * d.0 + d.1 * d.1;
            let t = ((fp.0 - o.0) * d.0 + (fp.1 - o.1) * d.1) / len2;
            candidates.push((o.0 + t * d.0, o.1 + t * d.1));
        }
    }
    candidates
        .into_iter()
        .min_by(|a, b| dist(*a).total_cmp(&dist(*b)))
        .map_or(p, |q| Point::new(q.0.round() as Coord, q.1.round() as Coord))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn zone() -> Object {
        let sq = vec![Point::mm(0.0, 0.0), Point::mm(10.0, 0.0), Point::mm(10.0, 10.0), Point::mm(0.0, 10.0)];
        let mut z = Zone::new(LayerId::TOP_COPPER, "", sq);
        z.holes.push(vec![Point::mm(2.0, 2.0), Point::mm(4.0, 2.0), Point::mm(3.0, 4.0)]);
        Object::Zone(z)
    }

    fn outline(o: &Object) -> &Vec<Point> {
        let Object::Zone(z) = o else { panic!() };
        &z.outline
    }

    #[test]
    fn corners_and_midpoints_of_outline_and_holes() {
        let h = handles(&zone());
        assert_eq!(h.len(), 2 * 4 + 2 * 3);
        let at = hit(&h, Point::mm(5.0, 0.1), mm(0.5)).unwrap();
        assert_eq!(at.grip, Grip::Mid { ring: 0, i: 0 });
        let at = hit(&h, Point::mm(3.9, 2.0), mm(1.0)).unwrap();
        assert_eq!(at.grip, Grip::Corner { ring: 1, i: 1 }, "corners before midpoints");
    }

    #[test]
    fn moving_inserting_and_deleting_corners() {
        let z = zone();
        let m = moved(&z, Grip::Corner { ring: 0, i: 2 }, Point::mm(12.0, 12.0)).unwrap();
        assert_eq!(outline(&m)[2], Point::mm(12.0, 12.0));
        let m = moved(&z, Grip::Mid { ring: 0, i: 0 }, Point::mm(5.0, -2.0)).unwrap();
        assert_eq!(outline(&m).len(), 5);
        assert_eq!(outline(&m)[1], Point::mm(5.0, -2.0));
        assert_eq!(after_insert(Grip::Mid { ring: 0, i: 0 }), Grip::Corner { ring: 0, i: 1 });
        let d = without_corner(&m, Grip::Corner { ring: 0, i: 1 }).unwrap();
        assert_eq!(outline(&d), outline(&z));
        let err = without_corner(&z, Grip::Corner { ring: 1, i: 0 }).unwrap_err();
        assert!(err.contains("three"), "{err}");
    }

    #[test]
    fn rectangles_resize_and_convert() {
        let g = Object::BoardGraphic(BoardGraphic {
            id: Id::new(),
            layer: LayerId::OUTLINE,
            shape: Shape::rect(Point::ZERO, Point::mm(10.0, 5.0), 100_000),
        });
        assert_eq!(handles(&g).len(), 8);
        let r = moved(&g, Grip::RectCorner(1), Point::mm(12.0, -1.0)).unwrap();
        let Object::BoardGraphic(BoardGraphic { shape: Shape::Rect { a, b, .. }, .. }) = r else { panic!() };
        assert_eq!((a, b), (Point::mm(0.0, -1.0), Point::mm(12.0, 5.0)), "the opposite corner stays");
        let r = moved(&g, Grip::RectEdge(2), Point::mm(3.0, 7.0)).unwrap();
        let Object::BoardGraphic(BoardGraphic { shape: Shape::Rect { b, .. }, .. }) = r else { panic!() };
        assert_eq!(b, Point::mm(10.0, 7.0));
        let p = rect_to_polygon(&g).unwrap();
        let Object::BoardGraphic(BoardGraphic { shape: Shape::Polygon { points, .. }, .. }) = &p else { panic!() };
        assert_eq!(points.len(), 4);
        assert_eq!(handles(&p).len(), 8, "now with corner and midpoint handles");
    }

    #[test]
    fn constraining_to_45_degrees() {
        // One neighbour: onto the nearest line through it.
        assert_eq!(constrain(Point::mm(5.0, 0.3), &[Point::ZERO]), Point::mm(5.0, 0.0));
        assert_eq!(constrain(Point::mm(4.8, 5.1), &[Point::ZERO]), Point::mm(4.95, 4.95));
        // Two: the nearest crossing (here a right angle corner).
        let c = constrain(Point::mm(9.5, 0.4), &[Point::ZERO, Point::mm(10.0, 10.0)]);
        assert_eq!(c, Point::mm(10.0, 0.0));
    }
}
