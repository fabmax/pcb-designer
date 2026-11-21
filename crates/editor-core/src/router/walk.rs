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
        if hull.contains(path[0], EPS) {
            return Walk { path: vec![path[0]], reached: false };
        }
        // Where it goes in (on the hull's outline, else just inside it).
        let crossing = |a: Point, b: Point| hull.crossing(a, b, 0.0).or_else(|| hull.crossing(a, b, EPS));
        let Some(c_in) = crossing(path[k], path[k + 1]) else { return stop(path, k, hull) };
        let entry = Hull::at(path[k], path[k + 1], c_in.t_in);
        let Some(edge_in) = c_in.edge_in else { return stop(path, k, hull) };
        // Where it last comes out.
        let last = (k..path.len() - 1).rev().find_map(|m| crossing(path[m], path[m + 1]).map(|c| (m, c)));
        let Some((m, c_out)) = last else { return stop(path, k, hull) };
        let Some(edge_out) = c_out.edge_out else {
            // Ends inside: stop where it goes in.
            path.truncate(k + 1);
            path.push(entry);
            return Walk { path: simplify(path), reached: false };
        };
        let exit = Hull::at(path[m], path[m + 1], c_out.t_out);
        let mut next: Vec<Point> = path[..=k].to_vec();
        next.push(entry);
        next.extend(round(hull, entry, edge_in, exit, edge_out, ccw));
        next.push(exit);
        next.extend_from_slice(&path[m + 1..]);
        path = simplify(next);
    }
    // Too many: stop at the first hull still crossed.
    match first_crossing(&path, hulls) {
        Some((k, h)) => stop(path, k, &hulls[h]),
        None => Walk { path, reached: true },
    }
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
