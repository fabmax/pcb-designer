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

/// Instances that fill a polygon with holes (`contours[0]` the outline, then holes; any
/// winding). Edges along the contours are anti-aliased. Fast for many holes.
pub fn fill_region(contours: &[Vec<[f32; 2]>], color: Color) -> Vec<Instance> {
    use i_triangle::i_overlay::i_float::int::point::IntPoint;
    use i_triangle::int::triangulatable::IntTriangulatable;

    let max = contours.iter().flatten().fold(0f32, |m, p| m.max(p[0].abs()).max(p[1].abs())) as f64;
    if max == 0.0 {
        return Vec::new();
    }
    // Integer coordinates for the robust triangulator.
    let scale = 1e9 / max;
    let shape: Vec<Vec<IntPoint<i32>>> = contours
        .iter()
        .enumerate()
        .filter(|(_, c)| c.len() >= 3)
        .map(|(i, c)| {
            let mut c: Vec<IntPoint<i32>> = c
                .iter()
                .map(|p| IntPoint::new((p[0] as f64 * scale).round() as i32, (p[1] as f64 * scale).round() as i32))
                .collect();
            // Outline counterclockwise, holes clockwise (non-zero rule).
            let area: i128 = (0..c.len())
                .map(|k| {
                    let (a, b) = (c[k], c[(k + 1) % c.len()]);
                    a.x as i128 * b.y as i128 - b.x as i128 * a.y as i128
                })
                .sum();
            if (area > 0) != (i == 0) {
                c.reverse();
            }
            c
        })
        .collect();
    let t = shape.triangulate().into_triangulation::<u32>();
    // Boundary edges belong to one triangle only.
    let mut uses: rustc_hash::FxHashMap<(u32, u32), u8> = Default::default();
    for tri in t.indices.as_chunks::<3>().0 {
        for k in 0..3 {
            let (a, b) = (tri[k], tri[(k + 1) % 3]);
            *uses.entry((a.min(b), a.max(b))).or_default() += 1;
        }
    }
    let boundary = |a: u32, b: u32| uses.get(&(a.min(b), a.max(b))) == Some(&1);
    let pt = |i: u32| {
        let p = t.points[i as usize];
        [(p.x as f64 / scale) as f32, (p.y as f64 / scale) as f32]
    };
    t.indices
        .as_chunks::<3>()
        .0
        .iter()
        .map(|&[a, b, c]| {
            Instance::triangle(pt(a), pt(b), pt(c), [boundary(a, b), boundary(b, c), boundary(c, a)], color)
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
    fn region_with_a_hole() {
        let outer = vec![[0.0, 0.0], [4.0, 0.0], [4.0, 4.0], [0.0, 4.0]];
        let hole = vec![[1.0, 1.0], [1.0, 3.0], [3.0, 3.0], [3.0, 1.0]];
        let inst = fill_region(&[outer, hole], Color::rgb(1, 2, 3));
        assert_eq!(inst.len(), 8);
        let edges: u32 = inst.iter().map(|i| (i.kind >> 8).count_ones()).sum();
        assert_eq!(edges, 8, "4 outline + 4 hole edges anti-aliased");
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
