//! Walkaround: a path that goes round obstacle hulls. The path's first crossing of a hull
//! is replaced by the hull's outline between where it goes in and where it last comes
//! out, always turning the same way round (counterclockwise or clockwise); repeated
//! until nothing is crossed. A path ending inside a hull stops where it goes in.

use model::Point;

use super::hull::{EPS, Hull};

/// Most hulls walked round for one path.
const MAX_STEPS: usize = 64;

/// A walked path.
#[derive(Clone, Debug, PartialEq)]
pub struct Walk {
    pub path: Vec<Point>,
    /// It gets to the end of the path asked for (else it stops short of a hull).
    pub reached: bool,
}

/// `path` walked round `hulls`, turning counterclockwise round each (`ccw`) or
/// clockwise.
pub fn walk(path: &[Point], hulls: &[Hull], ccw: bool) -> Walk {
    let mut path = simplify(path.to_vec());
    for _ in 0..MAX_STEPS {
        let Some((k, h)) = first_crossing(&path, hulls) else { return Walk { path, reached: true } };
        let hull = &hulls[h];
        // Where it goes in: on segment k, or earlier if k starts inside (a corner of
        // another hull's outline can lie inside this one).
        let Some(j) = (0..=k).rev().find(|&j| !inside(hull, path[j])) else {
            return Walk { path: vec![path[0]], reached: false };
        };
        let (entry, edge_in) = match hull.crossing(path[j], path[j + 1], 0.0) {
            Some(c) if c.edge_in.is_some() => (Hull::at(path[j], path[j + 1], c.t_in), c.edge_in.expect("some")),
            // It starts on the outline.
            _ => (path[j], nearest_edge(hull, path[j])),
        };
        if inside(hull, *path.last().expect("a path")) {
            // Ends inside: stop where it goes in.
            path.truncate(j + 1);
            path.push(entry);
            return Walk { path: simplify(path), reached: false };
        }
        // Where it last comes out.
        let m = (j..path.len() - 1)
            .rev()
            .find(|&m| hull.crossing(path[m], path[m + 1], 0.0).is_some() || inside(hull, path[m]))
            .unwrap_or(j);
        let (exit, edge_out) = match hull.crossing(path[m], path[m + 1], 0.0) {
            Some(c) if c.edge_out.is_some() => (Hull::at(path[m], path[m + 1], c.t_out), c.edge_out.expect("some")),
            // It leaves at the segment's end, on the outline.
            _ => (path[m + 1], nearest_edge(hull, path[m + 1])),
        };
        let mut next: Vec<Point> = path[..=j].to_vec();
        next.push(entry);
        next.extend(round(hull, entry, edge_in, exit, edge_out, ccw));
        next.push(exit);
        next.extend_from_slice(&path[m + 1..]);
        let next = simplify(next);
        if next == path {
            break; // no progress
        }
        path = next;
    }
    // Too many: stop at the first hull still crossed.
    match first_crossing(&path, hulls) {
        Some((k, h)) => stop(path, k, &hulls[h]),
        None => Walk { path, reached: true },
    }
}

/// Inside the hull (more than half a nanometre), not on its outline.
fn inside(hull: &Hull, p: Point) -> bool {
    hull.contains(p, 0.5)
}

/// The edge of `hull` that `p` (on or near its outline) is closest to.
fn nearest_edge(hull: &Hull, p: Point) -> usize {
    let n = hull.points.len();
    (0..n)
        .min_by(|&i, &j| {
            let d = |i: usize| crate::copper::point_seg_dist(p, hull.points[i], hull.points[(i + 1) % n]);
            d(i).total_cmp(&d(j))
        })
        .unwrap_or(0)
}

/// The first segment crossing a hull, and the hull it crosses first along it.
fn first_crossing(path: &[Point], hulls: &[Hull]) -> Option<(usize, usize)> {
    (0..path.len().saturating_sub(1)).find_map(|k| {
        hulls
            .iter()
            .enumerate()
            .filter_map(|(h, hull)| hull.crossing(path[k], path[k + 1], EPS).map(|c| (c.t_in, h)))
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .map(|(_, h)| (k, h))
    })
}

/// The path up to where segment `k` goes into `hull`.
fn stop(mut path: Vec<Point>, k: usize, hull: &Hull) -> Walk {
    let (a, b) = (path[k], path[k + 1]);
    let t = hull.crossing(a, b, 0.0).map_or(0.0, |c| c.t_in);
    path.truncate(k + 1);
    path.push(Hull::at(a, b, t));
    Walk { path: simplify(path), reached: false }
}

/// The hull's corners passed going round from `entry` on edge `i` to `exit` on edge `j`.
fn round(hull: &Hull, entry: Point, i: usize, exit: Point, j: usize, ccw: bool) -> Vec<Point> {
    let n = hull.points.len();
    let (a, b) = (hull.points[i], hull.points[(i + 1) % n]);
    // Along edge i (counterclockwise direction): is the exit ahead of the entry?
    let along = |p: Point| (p.x - a.x) as i128 * (b.x - a.x) as i128 + (p.y - a.y) as i128 * (b.y - a.y) as i128;
    if ccw {
        let count = match (j + n - i) % n {
            0 if along(exit) >= along(entry) => 0,
            0 => n,
            c => c,
        };
        (1..=count).map(|s| hull.points[(i + s) % n]).collect()
    } else {
        let count = match (i + n - j) % n {
            0 if along(exit) <= along(entry) => 0,
            0 => n,
            c => c,
        };
        (0..count).map(|s| hull.points[(i + n - s) % n]).collect()
    }
}

/// Without repeated points and corners that don't turn (or turn back on themselves).
pub fn simplify(path: Vec<Point>) -> Vec<Point> {
    let mut out: Vec<Point> = Vec::with_capacity(path.len());
    for p in path {
        if out.last() == Some(&p) {
            continue;
        }
        while out.len() >= 2 {
            let (a, b) = (out[out.len() - 2], out[out.len() - 1]);
            let cross = (b.x - a.x) as i128 * (p.y - b.y) as i128 - (b.y - a.y) as i128 * (p.x - b.x) as i128;
            if cross == 0 {
                out.pop();
            } else {
                break;
            }
        }
        if out.last() != Some(&p) {
            out.push(p);
        }
    }
    out
}

/// Length of a path (nm).
pub fn length(path: &[Point]) -> f64 {
    path.windows(2).map(|w| (w[1] - w[0]).length()).sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(x: i64, y: i64) -> Point {
        Point::new(x, y)
    }

    #[test]
    fn round_one_obstacle_both_ways() {
        let hull = Hull::grown(&[p(5000, 0)], 1000);
        let path = [p(0, 0), p(10_000, 0)];
        for ccw in [true, false] {
            let w = walk(&path, std::slice::from_ref(&hull), ccw);
            assert!(w.reached, "{w:?}");
            assert_eq!((w.path[0], *w.path.last().unwrap()), (path[0], path[1]));
            // Counterclockwise round the hull means passing below it (y up).
            let side = w.path.iter().map(|q| q.y).find(|&y| y != 0).unwrap();
            assert_eq!(side < 0, ccw, "{w:?}");
            for s in w.path.windows(2) {
                assert!(hull.crossing(s[0], s[1], EPS).is_none(), "{w:?}");
                let (dx, dy) = ((s[1].x - s[0].x).abs(), (s[1].y - s[0].y).abs());
                assert!(dx == 0 || dy == 0 || dx.abs_diff(dy) <= 1, "45° directions: {w:?}");
            }
        }
    }

    #[test]
    fn stops_short_of_an_obstacle_at_the_end() {
        let hull = Hull::grown(&[p(10_000, 0)], 1000);
        let w = walk(&[p(0, 0), p(10_000, 0)], &[hull], true);
        assert!(!w.reached);
        assert_eq!(w.path, vec![p(0, 0), p(9000, 0)]);
    }

    #[test]
    fn two_overlapping_obstacles() {
        let hulls = [Hull::grown(&[p(5000, 0)], 1000), Hull::grown(&[p(6500, 800)], 1000)];
        for ccw in [true, false] {
            let w = walk(&[p(0, 0), p(12_000, 0)], &hulls, ccw);
            assert!(w.reached, "{w:?}");
            for s in w.path.windows(2) {
                assert!(hulls.iter().all(|h| h.crossing(s[0], s[1], EPS).is_none()), "{w:?}");
            }
        }
    }
}
