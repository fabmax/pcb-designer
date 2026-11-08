//! Polygon triangulation for filled polygons (ear clipping).

use crate::primitive::{Color, Instance};

fn cross(o: [f64; 2], a: [f64; 2], b: [f64; 2]) -> f64 {
    (a[0] - o[0]) * (b[1] - o[1]) - (a[1] - o[1]) * (b[0] - o[0])
}

fn inside(p: [f64; 2], a: [f64; 2], b: [f64; 2], c: [f64; 2]) -> bool {
    cross(a, b, p) >= 0.0 && cross(b, c, p) >= 0.0 && cross(c, a, p) >= 0.0
}

/// Triangulate a simple polygon (either winding; duplicate and collinear points are fine).
/// Returns vertex index triples. Self-intersecting input still terminates and covers
/// roughly the right area.
pub fn triangulate(points: &[[f32; 2]]) -> Vec<[usize; 3]> {
    let p: Vec<[f64; 2]> = points.iter().map(|q| [q[0] as f64, q[1] as f64]).collect();
    let n = p.len();
    if n < 3 {
        return Vec::new();
    }
    let area: f64 = (0..n).map(|i| p[i][0] * p[(i + 1) % n][1] - p[(i + 1) % n][0] * p[i][1]).sum();
    // Work counterclockwise.
    let mut idx: Vec<usize> = if area >= 0.0 { (0..n).collect() } else { (0..n).rev().collect() };
    let mut out = Vec::with_capacity(n - 2);
    let mut i = 0;
    let mut misses = 0;
    while idx.len() > 3 {
        let m = idx.len();
        let (ia, ib, ic) = (idx[(i + m - 1) % m], idx[i % m], idx[(i + 1) % m]);
        let (a, b, c) = (p[ia], p[ib], p[ic]);
        let convex = cross(a, b, c) > 0.0;
        let is_ear = convex
            && !idx.iter().any(|&j| {
                j != ia && j != ib && j != ic && p[j] != a && p[j] != b && p[j] != c && inside(p[j], a, b, c)
            });
        // Degenerate (zero area) corners are removed without a triangle.
        let degenerate = cross(a, b, c).abs() <= 1e-18;
        if is_ear || degenerate || misses >= m {
            if !degenerate {
                out.push([ia, ib, ic]);
            }
            idx.remove(i % m);
            misses = 0;
            i %= idx.len();
        } else {
            i = (i + 1) % m;
            misses += 1;
        }
    }
    if cross(p[idx[0]], p[idx[1]], p[idx[2]]).abs() > 1e-18 {
        out.push([idx[0], idx[1], idx[2]]);
    }
    out
}

/// Instances that fill the polygon. Outline edges are anti-aliased, inner edges not.
pub fn fill_polygon(points: &[[f32; 2]], color: Color) -> Vec<Instance> {
    let n = points.len();
    let outer = |a: usize, b: usize| (a + 1) % n == b || (b + 1) % n == a;
    triangulate(points)
        .into_iter()
        .map(|[a, b, c]| {
            Instance::triangle(points[a], points[b], points[c], [outer(a, b), outer(b, c), outer(c, a)], color)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn area(points: &[[f32; 2]], tris: &[[usize; 3]]) -> f64 {
        tris.iter()
            .map(|t| {
                let [a, b, c] = t.map(|i| [points[i][0] as f64, points[i][1] as f64]);
                cross(a, b, c).abs() * 0.5
            })
            .sum()
    }

    #[test]
    fn square_and_concave() {
        let sq = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]];
        assert_eq!(triangulate(&sq).len(), 2);
        assert!((area(&sq, &triangulate(&sq)) - 1.0).abs() < 1e-6);
        // L shape, clockwise.
        let l = [[0.0, 0.0], [0.0, 2.0], [1.0, 2.0], [1.0, 1.0], [2.0, 1.0], [2.0, 0.0]];
        let t = triangulate(&l);
        assert_eq!(t.len(), 4);
        assert!((area(&l, &t) - 3.0).abs() < 1e-6);
    }

    #[test]
    fn degenerate_input_terminates() {
        assert!(triangulate(&[[0.0, 0.0], [1.0, 0.0]]).is_empty());
        let line = [[0.0, 0.0], [1.0, 0.0], [2.0, 0.0], [3.0, 0.0]];
        assert!(triangulate(&line).is_empty());
        let bow = [[0.0, 0.0], [1.0, 1.0], [1.0, 0.0], [0.0, 1.0]];
        assert!(triangulate(&bow).len() <= 2);
        let dup = [[0.0, 0.0], [1.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]];
        assert!((area(&dup, &triangulate(&dup)) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn outer_edges_flagged() {
        let sq = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]];
        let inst = fill_polygon(&sq, Color::rgb(1, 2, 3));
        // 4 outer edges + the shared diagonal counted twice as inner.
        let outer: u32 = inst.iter().map(|i| (i.kind >> 8).count_ones()).sum();
        assert_eq!(outer, 4);
    }
}
