//! Smoothing: a path made shorter and with fewer corners where it stays clear. Walking
//! round hulls leaves the outline's corners in a path (walls along a hull's sides, steps
//! round each obstacle); smoothing replaces runs of corners by the shortest octilinear
//! connection (one segment, or a straight and a diagonal one either way round), then
//! cuts the square corners left at 45°. The path's ends stay; it never turns back on
//! itself more sharply than a square corner.

use model::Point;

use super::walk::{length, simplify};

/// Most corners a shortcut replaces at once (longer runs are taken in steps).
const WINDOW: usize = 8;

/// Most passes over the path.
const PASSES: usize = 4;

/// A shortcut must save this much length (nm) unless it saves a corner.
const GAIN: f64 = 1000.0;

/// `path` smoothed: `clear(segments)` says whether new copper along `segments` (a path
/// from a corner kept to a corner kept) breaks no rule.
pub fn smooth(path: Vec<Point>, clear: &mut dyn FnMut(&[Point]) -> bool) -> Vec<Point> {
    let mut path = simplify(path);
    for _ in 0..PASSES {
        let before = path.len();
        let shortened = shortcuts(&mut path, clear);
        let cut = chamfers(&mut path, clear);
        if !shortened && !cut && path.len() == before {
            break;
        }
    }
    path
}

/// Replace runs of corners by shorter connections; whether any was.
fn shortcuts(path: &mut Vec<Point>, clear: &mut dyn FnMut(&[Point]) -> bool) -> bool {
    let mut any = false;
    let mut i = 0;
    while i + 2 < path.len() {
        let mut taken = false;
        let far = (i + WINDOW + 1).min(path.len() - 1);
        for j in (i + 2..=far).rev() {
            let old = length(&path[i..=j]);
            for way in connections(path[i], path[j]) {
                let corners = way.len() - 2;
                let saves_corner = corners < j - i - 1;
                let new = length(&way);
                if !(new < old - GAIN || (saves_corner && new <= old + 1.0)) {
                    continue;
                }
                if !fits(path, i, j, &way) || !clear(&way) {
                    continue;
                }
                let mut next = path[..i].to_vec();
                next.extend_from_slice(&way);
                next.extend_from_slice(&path[j + 1..]);
                *path = simplify(next);
                taken = true;
                break;
            }
            if taken {
                break;
            }
        }
        any |= taken;
        if !taken {
            i += 1;
        }
    }
    any
}

/// Cut square corners at 45° (the whole shorter side, half or a quarter of it); whether
/// any was.
fn chamfers(path: &mut Vec<Point>, clear: &mut dyn FnMut(&[Point]) -> bool) -> bool {
    let mut any = false;
    let mut i = 1;
    while i + 1 < path.len() {
        match chamfer(path, i, clear) {
            Some(cut) => {
                *path = simplify(cut);
                any = true;
                i = 1; // corners shifted
            }
            None => i += 1,
        }
    }
    any
}

fn chamfer(path: &[Point], i: usize, clear: &mut dyn FnMut(&[Point]) -> bool) -> Option<Vec<Point>> {
    let (a, b, c) = (path[i - 1], path[i], path[i + 1]);
    let (u, v) = (dir(a, b), dir(b, c));
    if u.x * v.x + u.y * v.y != 0 {
        return None; // not a square corner
    }
    let most = steps(a, b).min(steps(b, c));
    for k in [most, most / 2, most / 4] {
        if k < super::SLACK {
            break;
        }
        let (p, q) = (b - Point::new(u.x * k, u.y * k), b + Point::new(v.x * k, v.y * k));
        // Only the new copper needs checking: from the corner before to the one after.
        if clear(&[a, p, q, c]) {
            let mut cut = path[..i].to_vec();
            cut.extend([p, q]);
            cut.extend_from_slice(&path[i + 1..]);
            return Some(cut);
        }
    }
    None
}

/// Octilinear ways from `a` to `b`: one segment if it's a 45° multiple (to rounding),
/// else a straight and a diagonal segment, either way round.
fn connections(a: Point, b: Point) -> Vec<Vec<Point>> {
    let d = b - a;
    let (dx, dy) = (d.x.abs(), d.y.abs());
    if dx == 0 || dy == 0 || dx.abs_diff(dy) <= 2 {
        return vec![vec![a, b]];
    }
    let m = dx.min(dy);
    let diag = Point::new(d.x.signum() * m, d.y.signum() * m);
    vec![vec![a, a + diag, b], vec![a, b - diag, b]]
}

/// Does `way` join the path at corners `i` and `j` without turning more sharply than a
/// square corner?
fn fits(path: &[Point], i: usize, j: usize, way: &[Point]) -> bool {
    let turn_ok = |p: Point, q: Point, r: Point| {
        let (u, v) = (q - p, r - q);
        u.x as i128 * v.x as i128 + u.y as i128 * v.y as i128 >= 0
    };
    (i == 0 || turn_ok(path[i - 1], way[0], way[1]))
        && (j + 1 == path.len() || turn_ok(way[way.len() - 2], way[way.len() - 1], path[j + 1]))
}

fn dir(p: Point, q: Point) -> Point {
    Point::new((q.x - p.x).signum(), (q.y - p.y).signum())
}

fn steps(p: Point, q: Point) -> i64 {
    (q.x - p.x).abs().max((q.y - p.y).abs())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(x: i64, y: i64) -> Point {
        Point::new(x, y)
    }

    #[test]
    fn steps_round_nothing_become_one_dog_leg() {
        // A staircase with nothing in the way.
        let stairs = vec![p(0, 0), p(1000, 0), p(1000, 1000), p(2000, 1000), p(2000, 2000), p(10_000, 2000)];
        let s = smooth(stairs, &mut |_| true);
        assert_eq!(s.len(), 3, "{s:?}");
        assert_eq!((s[0], s[2]), (p(0, 0), p(10_000, 2000)));
    }

    #[test]
    fn keeps_what_is_needed() {
        // A detour round a block between x 4000..6000, y -500..500: nothing may enter it.
        let block = |w: &[Point]| {
            w.windows(2).all(|s| {
                let (a, b) = (s[0], s[1]);
                // Sample the segment.
                (0..=100).all(|k| {
                    let t = k as f64 / 100.0;
                    let x = a.x as f64 + (b.x - a.x) as f64 * t;
                    let y = a.y as f64 + (b.y - a.y) as f64 * t;
                    !(x > 4000.0 && x < 6000.0 && y > -500.0 && y < 500.0)
                })
            })
        };
        let detour = vec![p(0, 0), p(3000, 0), p(3000, 600), p(7000, 600), p(7000, 0), p(10_000, 0)];
        let s = smooth(detour.clone(), &mut |w| block(w));
        assert!(block(&s), "{s:?}");
        assert!(length(&s) < length(&detour), "{s:?}");
        for w in s.windows(3) {
            let (u, v) = (w[1] - w[0], w[2] - w[1]);
            assert!(u.x * v.x + u.y * v.y > 0, "45° turns only: {s:?}");
        }
    }

    #[test]
    fn a_plain_dog_leg_stays_as_it_is() {
        let leg = vec![p(0, 0), p(5000, 0), p(8000, 3000)];
        assert_eq!(smooth(leg.clone(), &mut |_| true), leg);
    }
}
