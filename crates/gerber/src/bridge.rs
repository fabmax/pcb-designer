//! Holes joined into their outline with cut-in lines, since a Gerber region is one
//! contour. Drawing holes with clear polarity instead would also erase other copper
//! that lies inside them (another zone's island around a pad, say).

use model::{Coord, Point};

fn area2(c: &[Point]) -> i128 {
    let n = c.len();
    (0..n).map(|i| c[i].x as i128 * c[(i + 1) % n].y as i128 - c[(i + 1) % n].x as i128 * c[i].y as i128).sum()
}

/// One contour covering `outline` minus `holes` (holes inside the outline, disjoint).
/// Each hole, rightmost first, is connected from its rightmost vertex by a horizontal
/// cut to the nearest edge on its right; the cut runs there and back.
pub fn bridge_holes(outline: &[Point], holes: &[Vec<Point>]) -> Vec<Point> {
    let mut out: Vec<Point> = outline.to_vec();
    if area2(&out) < 0 {
        out.reverse();
    }
    let mut holes: Vec<Vec<Point>> = holes.iter().filter(|h| h.len() >= 3).cloned().collect();
    let max_x = |h: &Vec<Point>| h.iter().map(|p| p.x).max().unwrap_or(Coord::MIN);
    holes.sort_by_key(|h| std::cmp::Reverse(max_x(h)));
    for mut hole in holes {
        if area2(&hole) > 0 {
            hole.reverse(); // holes run clockwise
        }
        let hi = (0..hole.len()).max_by_key(|&i| (hole[i].x, -hole[i].y)).unwrap();
        let h = hole[hi];
        // Nearest crossing of the ray h → +x with an edge of the contour so far.
        let mut best: Option<(f64, usize, Point)> = None;
        let n = out.len();
        for i in 0..n {
            let (a, b) = (out[i], out[(i + 1) % n]);
            if (a.y > h.y) == (b.y > h.y) {
                continue;
            }
            let t = (h.y - a.y) as f64 / (b.y - a.y) as f64;
            let x = a.x as f64 + t * (b.x - a.x) as f64;
            if x >= h.x as f64 && best.is_none_or(|(d, ..)| x - (h.x as f64) < d) {
                best = Some((x - h.x as f64, i, Point::new(x.round() as Coord, h.y)));
            }
        }
        let Some((_, i, p)) = best else { continue };
        // out[..=i], p, hole from h around back to h, p, out[i+1..]
        let mut merged = Vec::with_capacity(out.len() + hole.len() + 3);
        merged.extend_from_slice(&out[..=i]);
        if merged.last() != Some(&p) {
            merged.push(p);
        }
        merged.extend(hole[hi..].iter().chain(&hole[..hi]).copied());
        merged.push(h);
        merged.push(p);
        if out.get(i + 1) == Some(&p) {
            merged.extend_from_slice(&out[i + 2..]);
        } else {
            merged.extend_from_slice(&out[i + 1..]);
        }
        out = merged;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn square(x: i64, y: i64, s: i64) -> Vec<Point> {
        vec![Point::new(x, y), Point::new(x + s, y), Point::new(x + s, y + s), Point::new(x, y + s)]
    }

    /// Non-zero winding number (Gerber fills a region's inside; cut-ins cancel out).
    fn winding(c: &[Point], p: (f64, f64)) -> i32 {
        let mut w = 0;
        for i in 0..c.len() {
            let (a, b) = (c[i], c[(i + 1) % c.len()]);
            let (ax, ay, bx, by) = (a.x as f64, a.y as f64, b.x as f64, b.y as f64);
            let cross = (bx - ax) * (p.1 - ay) - (p.0 - ax) * (by - ay);
            if ay <= p.1 && by > p.1 && cross > 0.0 {
                w += 1;
            } else if ay > p.1 && by <= p.1 && cross < 0.0 {
                w -= 1;
            }
        }
        w
    }

    #[test]
    fn holes_are_cut_in() {
        let outer = square(0, 0, 100);
        let holes = vec![square(10, 10, 20), square(60, 50, 20), square(20, 60, 10)];
        let c = bridge_holes(&outer, &holes);
        assert_eq!(c.len(), 4 + 3 * (4 + 3));
        for (p, inside) in [
            ((5.5, 5.5), true),
            ((15.5, 15.5), false),
            ((65.5, 55.5), false),
            ((25.5, 65.5), false),
            ((50.5, 50.5), true),
            ((95.5, 21.0), true),
            ((150.0, 50.0), false),
        ] {
            assert_eq!(winding(&c, p) != 0, inside, "{p:?}");
        }
        assert_eq!(area2(&c), area2(&outer) - (400 + 400 + 100) * 2);
    }
}
