//! Hulls: the area a track's centre line must keep out of to clear an obstacle. A convex
//! polygon (counterclockwise), the obstacle's outline grown by an octagon of the clearance
//! plus the track's half width, so its edges run at multiples of 45° around round and
//! axis-aligned copper and a path along them keeps the routing directions.

use model::{Coord, Point};

use crate::copper::Geo;

/// How far inside a hull's boundary a point must be to count as inside (nm): a path
/// running along a hull's edge (rounded to whole nm) is outside.
pub const EPS: f64 = 5.0;

/// A convex polygon, counterclockwise, without repeated or collinear points.
#[derive(Clone, Debug, PartialEq)]
pub struct Hull {
    pub points: Vec<Point>,
}

/// A segment's way through a hull: where it goes in and out (parameters along the segment,
/// 0 at its start) and the hull edges it crosses there (edge `i` runs from point `i` to
/// point `i + 1`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Crossing {
    pub t_in: f64,
    pub edge_in: Option<usize>,
    pub t_out: f64,
    pub edge_out: Option<usize>,
}

/// tan(22.5°): an octagon's half side over its inner radius.
const TAN_22_5: f64 = std::f64::consts::SQRT_2 - 1.0;

/// The corners of an octagon around `c` whose sides are `r` from it (rounded outwards).
fn octagon(c: Point, r: Coord) -> [Point; 8] {
    let s = (r as f64 * TAN_22_5).ceil() as Coord;
    [(r, -s), (r, s), (s, r), (-s, r), (-r, s), (-r, -s), (-s, -r), (s, -r)].map(|(x, y)| Point::new(c.x + x, c.y + y))
}

impl Hull {
    /// The convex hull of `points` grown by an octagon whose sides are `grow` out.
    pub fn grown(points: &[Point], grow: Coord) -> Hull {
        let all: Vec<Point> = points.iter().flat_map(|&p| octagon(p, grow)).collect();
        Hull { points: convex_hull(all) }
    }

    /// The hulls around `geo` for a clearance of `grow` (a path gives one per chord;
    /// regions none: zone fills aren't obstacles).
    pub fn around(geo: &Geo, grow: Coord) -> Vec<Hull> {
        match geo {
            Geo::Capsule { a, b, r } => vec![Hull::grown(&[*a, *b], r + grow)],
            Geo::Polygon(points) => vec![Hull::grown(points, grow)],
            Geo::Path { points, r } => points.windows(2).map(|w| Hull::grown(w, r + grow)).collect(),
            Geo::Region { .. } => Vec::new(),
        }
    }

    fn edge(&self, i: usize) -> (Point, Point) {
        (self.points[i], self.points[(i + 1) % self.points.len()])
    }

    /// `p` lies more than `margin` inside.
    pub fn contains(&self, p: Point, margin: f64) -> bool {
        self.points.len() >= 3
            && (0..self.points.len()).all(|i| {
                let (a, b) = self.edge(i);
                signed_distance(a, b, p) > margin
            })
    }

    /// Where the segment `a`–`b` runs inside the hull shrunk by `margin` (Cyrus–Beck
    /// clipping), if it does over more than a point.
    pub fn crossing(&self, a: Point, b: Point, margin: f64) -> Option<Crossing> {
        if self.points.len() < 3 {
            return None;
        }
        let (mut t_in, mut t_out) = (0.0_f64, 1.0_f64);
        let (mut edge_in, mut edge_out) = (None, None);
        let d = (b.x as f64 - a.x as f64, b.y as f64 - a.y as f64);
        for i in 0..self.points.len() {
            let (p, q) = self.edge(i);
            let e = (q.x as f64 - p.x as f64, q.y as f64 - p.y as f64);
            let len = e.0.hypot(e.1);
            if len == 0.0 {
                continue;
            }
            // Inside: num + t * den > 0 (left of the edge, by more than the margin).
            let num = signed_distance(p, q, a) - margin;
            let den = (e.0 * d.1 - e.1 * d.0) / len;
            if den == 0.0 {
                if num <= 0.0 {
                    return None;
                }
            } else {
                let t = -num / den;
                if den > 0.0 {
                    if t > t_in {
                        (t_in, edge_in) = (t, Some(i));
                    }
                } else if t < t_out {
                    (t_out, edge_out) = (t, Some(i));
                }
            }
            if t_in >= t_out {
                return None;
            }
        }
        // Over more than a nanometre.
        let len = d.0.hypot(d.1);
        ((t_out - t_in) * len > 1.0).then_some(Crossing { t_in, edge_in, t_out, edge_out })
    }

    /// The point a parameter `t` along `a`–`b` lands on, rounded to whole nm.
    pub fn at(a: Point, b: Point, t: f64) -> Point {
        Point::new(
            (a.x as f64 + (b.x - a.x) as f64 * t).round() as Coord,
            (a.y as f64 + (b.y - a.y) as f64 * t).round() as Coord,
        )
    }
}

/// Distance of `p` left of the line `a`→`b` (negative: right of it).
fn signed_distance(a: Point, b: Point, p: Point) -> f64 {
    let e = ((b.x - a.x) as f64, (b.y - a.y) as f64);
    let len = e.0.hypot(e.1);
    if len == 0.0 {
        return 0.0;
    }
    (e.0 * (p.y - a.y) as f64 - e.1 * (p.x - a.x) as f64) / len
}

fn cross(o: Point, a: Point, b: Point) -> i128 {
    (a.x - o.x) as i128 * (b.y - o.y) as i128 - (a.y - o.y) as i128 * (b.x - o.x) as i128
}

/// Andrew's monotone chain: counterclockwise, collinear points dropped.
fn convex_hull(mut points: Vec<Point>) -> Vec<Point> {
    points.sort_by_key(|p| (p.x, p.y));
    points.dedup();
    if points.len() < 3 {
        return points;
    }
    let mut lower: Vec<Point> = Vec::new();
    for &p in &points {
        while lower.len() >= 2 && cross(lower[lower.len() - 2], lower[lower.len() - 1], p) <= 0 {
            lower.pop();
        }
        lower.push(p);
    }
    let mut upper: Vec<Point> = Vec::new();
    for &p in points.iter().rev() {
        while upper.len() >= 2 && cross(upper[upper.len() - 2], upper[upper.len() - 1], p) <= 0 {
            upper.pop();
        }
        upper.push(p);
    }
    lower.pop();
    upper.pop();
    lower.extend(upper);
    lower
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn octagon_around_a_point() {
        let h = Hull::grown(&[Point::ZERO], 1000);
        assert_eq!(h.points.len(), 8);
        // Sides 1000 from the centre, corners on 45° lines.
        assert!(h.contains(Point::new(999, 0), 0.0));
        assert!(!h.contains(Point::new(1001, 0), 0.0));
        assert!(!h.contains(Point::new(710, 710), 0.0), "the diagonal side is 1000 out");
        assert!(h.contains(Point::new(700, 700), 0.0));
    }

    #[test]
    fn segment_crossing() {
        let h = Hull::grown(&[Point::new(-1000, 0), Point::new(1000, 0)], 500);
        let c = h.crossing(Point::new(0, -5000), Point::new(0, 5000), 0.0).unwrap();
        assert!((c.t_in - 0.45).abs() < 1e-9 && (c.t_out - 0.55).abs() < 1e-9, "{c:?}");
        // Along the top edge: not inside.
        assert!(h.crossing(Point::new(-1000, 500), Point::new(1000, 500), EPS).is_none());
        // Missing it.
        assert!(h.crossing(Point::new(-5000, 600), Point::new(5000, 600), 0.0).is_none());
    }
}
