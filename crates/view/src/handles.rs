//! Handles of a selected zone, keepout or drawing (board graphics and the graphics of
//! symbols and footprints): drag a corner to move it, an edge's midpoint to insert a
//! corner (polygons) or move the edge (rectangles), delete corners; line and arc ends, a
//! circle's radius, an arc's bulge. GUI independent: the app draws the handles,
//! hit-tests them and runs the edits as `Update`s.

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
    /// A circle's radius (the handle sits on its right).
    Radius,
    /// An arc's end (0: start, 1: end): turns around the centre, the radius stays.
    ArcEnd(usize),
    /// An arc's middle: the arc through both ends and this point.
    ArcMid,
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

    /// Drawn small and round (handles that don't sit on a corner).
    pub fn is_minor(&self) -> bool {
        matches!(self.grip, Grip::Mid { .. } | Grip::RectEdge(_) | Grip::ArcMid)
    }
}

fn mid(a: Point, b: Point) -> Point {
    Point::new((a.x + b.x) / 2, (a.y + b.y) / 2)
}

fn rect_corners(a: Point, b: Point) -> [Point; 4] {
    [a, Point::new(b.x, a.y), b, Point::new(a.x, b.y)]
}

fn polar(c: Point, r: f64, deg: f64) -> Point {
    let t = deg.to_radians();
    c + Point::new((r * t.cos()).round() as Coord, (r * t.sin()).round() as Coord)
}

fn angle_of(c: Point, p: Point) -> f64 {
    ((p.y - c.y) as f64).atan2((p.x - c.x) as f64).to_degrees()
}

/// Start, middle and end of an arc.
fn arc_points(center: Point, radius: Coord, start: Angle, sweep: f64) -> [Point; 3] {
    let (r, s) = (radius as f64, start.degrees());
    [polar(center, r, s), polar(center, r, s + sweep / 2.0), polar(center, r, s + sweep)]
}

fn ring_handles(rings: &[&Vec<Point>]) -> Vec<Handle> {
    let mut out = Vec::new();
    for (ring, pts) in rings.iter().enumerate() {
        let n = pts.len();
        for i in 0..n {
            out.push(Handle { grip: Grip::Corner { ring, i }, pos: pts[i] });
            out.push(Handle { grip: Grip::Mid { ring, i }, pos: mid(pts[i], pts[(i + 1) % n]) });
        }
    }
    out
}

fn ring_moved(rings: Vec<&mut Vec<Point>>, grip: Grip, to: Point) -> Option<()> {
    match grip {
        Grip::Corner { ring, i } => *rings.into_iter().nth(ring)?.get_mut(i)? = to,
        Grip::Mid { ring, i } => {
            let r = rings.into_iter().nth(ring)?;
            if i >= r.len() {
                return None;
            }
            r.insert(i + 1, to);
        }
        _ => return None,
    }
    Some(())
}

fn ring_neighbours(rings: &[&Vec<Point>], grip: Grip) -> Vec<Point> {
    let get = |ring: usize| rings.get(ring).filter(|r| !r.is_empty());
    match grip {
        Grip::Corner { ring, i } => get(ring).map_or(Vec::new(), |pts| {
            let n = pts.len();
            vec![pts[(i + n - 1) % n], pts[(i + 1) % n]]
        }),
        Grip::Mid { ring, i } => get(ring).map_or(Vec::new(), |pts| vec![pts[i % pts.len()], pts[(i + 1) % pts.len()]]),
        _ => Vec::new(),
    }
}

fn ring_without(rings: Vec<&mut Vec<Point>>, grip: Grip) -> Result<(), String> {
    let Grip::Corner { ring, i } = grip else { return Err("only polygon corners can be deleted".into()) };
    let r = rings.into_iter().nth(ring).ok_or("no such corner")?;
    if r.len() <= 3 {
        return Err("a polygon keeps at least three corners".into());
    }
    if i >= r.len() {
        return Err("no such corner".into());
    }
    r.remove(i);
    Ok(())
}

// ---------------------------------------------------------------------------------------
// Shapes

/// The handles of a drawing.
pub fn shape_handles(s: &Shape) -> Vec<Handle> {
    match s {
        Shape::Polygon { points, .. } => ring_handles(&[points]),
        Shape::Rect { a, b, .. } => {
            let c = rect_corners(*a, *b);
            let corners = (0..4).map(|i| Handle { grip: Grip::RectCorner(i), pos: c[i] });
            let edges = (0..4).map(|i| Handle { grip: Grip::RectEdge(i), pos: mid(c[i], c[(i + 1) % 4]) });
            corners.chain(edges).collect()
        }
        Shape::Line { a, b, .. } => {
            vec![Handle { grip: Grip::LineEnd(0), pos: *a }, Handle { grip: Grip::LineEnd(1), pos: *b }]
        }
        Shape::Circle { center, radius, .. } => {
            vec![Handle { grip: Grip::Radius, pos: *center + Point::new(*radius, 0) }]
        }
        Shape::Arc { center, radius, start, sweep, .. } => {
            let [a, m, b] = arc_points(*center, *radius, *start, *sweep);
            vec![
                Handle { grip: Grip::ArcEnd(0), pos: a },
                Handle { grip: Grip::ArcEnd(1), pos: b },
                Handle { grip: Grip::ArcMid, pos: m },
            ]
        }
    }
}

/// The drawing with handle `grip` dragged to `to` (`None`: the handle doesn't fit, or
/// the result would be degenerate).
pub fn shape_moved(s: &Shape, grip: Grip, to: Point) -> Option<Shape> {
    let mut s = s.clone();
    match (&mut s, grip) {
        (Shape::Polygon { points, .. }, Grip::Corner { .. } | Grip::Mid { .. }) => ring_moved(vec![points], grip, to)?,
        (Shape::Rect { a, b, .. }, Grip::RectCorner(c)) => match c {
            // The opposite corner stays.
            0 => *a = to,
            1 => (b.x, a.y) = (to.x, to.y),
            2 => *b = to,
            3 => (a.x, b.y) = (to.x, to.y),
            _ => return None,
        },
        (Shape::Rect { a, b, .. }, Grip::RectEdge(e)) => match e {
            0 => a.y = to.y,
            1 => b.x = to.x,
            2 => b.y = to.y,
            3 => a.x = to.x,
            _ => return None,
        },
        (Shape::Line { a, b, .. }, Grip::LineEnd(e)) => {
            if e == 0 {
                *a = to
            } else {
                *b = to
            }
        }
        (Shape::Circle { center, radius, .. }, Grip::Radius) => {
            let r = (to - *center).length().round() as Coord;
            if r <= 0 {
                return None;
            }
            *radius = r;
        }
        (Shape::Arc { center, start, sweep, .. }, Grip::ArcEnd(e)) => {
            if to == *center {
                return None;
            }
            let end = start.degrees() + *sweep;
            let t = angle_of(*center, to);
            // Keep the arc going the same way round; the other end stays.
            let ccw = *sweep >= 0.0;
            let span = |from: f64, to: f64| {
                let d = (to - from).rem_euclid(360.0);
                if ccw { d } else { d - 360.0 }
            };
            let new = if e == 0 { span(t, end) } else { span(start.degrees(), t) };
            if new.abs() < 1e-3 {
                return None;
            }
            if e == 0 {
                *start = Angle::deg(t);
            }
            *sweep = new;
        }
        (Shape::Arc { center, radius, start, sweep, .. }, Grip::ArcMid) => {
            let [a, _, b] = arc_points(*center, *radius, *start, *sweep);
            let g = arc_through(a, to, b)?;
            *center = Point::new(g.center.0.round() as Coord, g.center.1.round() as Coord);
            *radius = g.radius.round() as Coord;
            *start = Angle::deg(g.start.to_degrees());
            *sweep = g.sweep.to_degrees();
        }
        _ => return None,
    }
    Some(s)
}

/// The points next to what `grip` moves, for 0°/45°/90° constraining.
pub fn shape_neighbours(s: &Shape, grip: Grip) -> Vec<Point> {
    match (s, grip) {
        (Shape::Polygon { points, .. }, _) => ring_neighbours(&[points], grip),
        (Shape::Line { a, b, .. }, Grip::LineEnd(e)) => vec![if e == 0 { *b } else { *a }],
        (Shape::Circle { center, .. } | Shape::Arc { center, .. }, Grip::Radius | Grip::ArcEnd(_)) => vec![*center],
        _ => Vec::new(),
    }
}

/// The drawing without corner `grip` (polygons; at least three stay).
pub fn shape_without_corner(s: &Shape, grip: Grip) -> Result<Shape, String> {
    let mut s = s.clone();
    let Shape::Polygon { points, .. } = &mut s else { return Err("only polygon corners can be deleted".into()) };
    ring_without(vec![points], grip)?;
    Ok(s)
}

/// A rectangle as a polygon of its four corners (other shapes: `None`).
pub fn rect_shape_to_polygon(s: &Shape) -> Option<Shape> {
    let Shape::Rect { a, b, width, fill } = *s else { return None };
    Some(Shape::Polygon { points: rect_corners(a, b).to_vec(), width, fill })
}

// ---------------------------------------------------------------------------------------
// Board objects

/// The rings (outline, then holes) of a zone or keepout.
fn rings(o: &Object) -> Option<Vec<&Vec<Point>>> {
    match o {
        Object::Zone(z) => Some(std::iter::once(&z.outline).chain(&z.holes).collect()),
        Object::Keepout(k) => Some(std::iter::once(&k.outline).chain(&k.holes).collect()),
        _ => None,
    }
}

fn rings_mut(o: &mut Object) -> Option<Vec<&mut Vec<Point>>> {
    match o {
        Object::Zone(z) => Some(std::iter::once(&mut z.outline).chain(z.holes.iter_mut()).collect()),
        Object::Keepout(k) => Some(std::iter::once(&mut k.outline).chain(k.holes.iter_mut()).collect()),
        _ => None,
    }
}

/// The handles of `o` (none for objects without corners).
pub fn handles(o: &Object) -> Vec<Handle> {
    match o {
        Object::BoardGraphic(g) => shape_handles(&g.shape),
        _ => rings(o).map(|r| ring_handles(&r)).unwrap_or_default(),
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
    let major = |h: &&Handle| !h.is_minor();
    let best = handles.iter().filter(near).filter(major).min_by(|a, b| d(a).total_cmp(&d(b)));
    best.or_else(|| handles.iter().filter(near).min_by(|a, b| d(a).total_cmp(&d(b)))).copied()
}

/// The points next to what `grip` moves, for 0°/45°/90° constraining.
pub fn neighbours(o: &Object, grip: Grip) -> Vec<Point> {
    match o {
        Object::BoardGraphic(g) => shape_neighbours(&g.shape, grip),
        _ => rings(o).map(|r| ring_neighbours(&r, grip)).unwrap_or_default(),
    }
}

/// `o` with the handle `grip` dragged to `to`. A midpoint inserts a new corner there.
/// `None` if the handle doesn't fit the object.
pub fn moved(o: &Object, grip: Grip, to: Point) -> Option<Object> {
    let mut o = o.clone();
    match &mut o {
        Object::BoardGraphic(g) => g.shape = shape_moved(&g.shape, grip, to)?,
        _ => ring_moved(rings_mut(&mut o)?, grip, to)?,
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
    let mut o = o.clone();
    match &mut o {
        Object::BoardGraphic(g) => g.shape = shape_without_corner(&g.shape, grip)?,
        _ => ring_without(rings_mut(&mut o).ok_or("no corners")?, grip)?,
    }
    Ok(o)
}

/// A rectangle drawing as a polygon of its four corners (other objects: `None`).
pub fn rect_to_polygon(o: &Object) -> Option<Object> {
    let Object::BoardGraphic(g) = o else { return None };
    let shape = rect_shape_to_polygon(&g.shape)?;
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
    fn circles_and_arcs() {
        let c = Shape::Circle { center: Point::ZERO, radius: mm(2.0), width: 0, fill: false };
        let h = shape_handles(&c);
        assert_eq!(h[0].pos, Point::mm(2.0, 0.0));
        let Some(Shape::Circle { radius, .. }) = shape_moved(&c, Grip::Radius, Point::mm(0.0, 3.0)) else { panic!() };
        assert_eq!(radius, mm(3.0));

        // A quarter arc from (2, 0) to (0, 2).
        let arc = Shape::Arc { center: Point::ZERO, radius: mm(2.0), start: Angle::ZERO, sweep: 90.0, width: 0 };
        let h = shape_handles(&arc);
        assert_eq!((h[0].pos, h[1].pos), (Point::mm(2.0, 0.0), Point::mm(0.0, 2.0)));
        // The end turns on to (-2, 0): a half circle; the start stays.
        let Some(Shape::Arc { start, sweep, radius, .. }) = shape_moved(&arc, Grip::ArcEnd(1), Point::mm(-5.0, 0.0))
        else {
            panic!()
        };
        assert_eq!((start.degrees(), radius), (0.0, mm(2.0)));
        assert!((sweep - 180.0).abs() < 1e-6, "{sweep}");
        // The middle pulled out flattens it: same ends, larger radius.
        let Some(m @ Shape::Arc { radius, .. }) = shape_moved(&arc, Grip::ArcMid, Point::mm(1.2, 1.2)) else {
            panic!()
        };
        assert!(radius > mm(2.0));
        let h = shape_handles(&m);
        assert!((h[0].pos - Point::mm(2.0, 0.0)).length() < 2.0 && (h[1].pos - Point::mm(0.0, 2.0)).length() < 2.0);
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
