//! Ratsnest: minimum spanning tree over the pads of a net, ignoring pads that are already
//! connected by copper (same island).

fn find(parent: &mut [u32], mut i: u32) -> u32 {
    while parent[i as usize] != i {
        parent[i as usize] = parent[parent[i as usize] as usize];
        i = parent[i as usize];
    }
    i
}

/// Returns index pairs of the unrouted connections. `island[i]` identifies the copper island
/// of pad `i`. Pads with equal island are considered connected already.
pub fn mst(points: &[[f64; 2]], island: &[u32]) -> Vec<(usize, usize)> {
    let k = points.len();
    if k < 2 {
        return Vec::new();
    }
    let mut parent: Vec<u32> = (0..k as u32).collect();
    {
        let mut first: rustc_hash::FxHashMap<u32, u32> = Default::default();
        for (i, &isl) in island.iter().enumerate() {
            match first.get(&isl) {
                Some(&j) => {
                    let (a, b) = (find(&mut parent, i as u32), find(&mut parent, j));
                    parent[a as usize] = b;
                }
                None => {
                    first.insert(isl, i as u32);
                }
            }
        }
    }

    let d2 = |a: usize, b: usize| {
        let (p, q) = (points[a], points[b]);
        (p[0] - q[0]).powi(2) + (p[1] - q[1]).powi(2)
    };

    let mut edges: Vec<(f64, u32, u32)> = Vec::new();
    if k <= 24 {
        for a in 0..k {
            for b in a + 1..k {
                edges.push((d2(a, b), a as u32, b as u32));
            }
        }
    } else {
        let pts: Vec<delaunator::Point> = points.iter().map(|p| delaunator::Point { x: p[0], y: p[1] }).collect();
        let tri = delaunator::triangulate(&pts);
        if tri.triangles.is_empty() {
            // Collinear: chain the points in sorted order.
            let mut order: Vec<usize> = (0..k).collect();
            order.sort_by(|&a, &b| points[a].partial_cmp(&points[b]).unwrap());
            for w in order.windows(2) {
                edges.push((d2(w[0], w[1]), w[0] as u32, w[1] as u32));
            }
        } else {
            for t in tri.triangles.as_chunks::<3>().0 {
                // Interior edges show up twice (once per triangle); hull edges only once.
                // Duplicates are harmless for Kruskal, so keep them all.
                for (a, b) in [(t[0], t[1]), (t[1], t[2]), (t[2], t[0])] {
                    edges.push((d2(a, b), a as u32, b as u32));
                }
            }
            // Delaunator can skip points (duplicates, some degenerate grid configurations).
            // Connect each skipped point to its nearest neighbor so the MST stays complete.
            let mut seen = vec![false; k];
            for &i in &tri.triangles {
                seen[i] = true;
            }
            for i in (0..k).filter(|&i| !seen[i]) {
                if let Some(j) = (0..k).filter(|&j| j != i).min_by(|&a, &b| d2(i, a).total_cmp(&d2(i, b))) {
                    edges.push((d2(i, j), i as u32, j as u32));
                }
            }
        }
    }
    edges.sort_unstable_by(|a, b| a.0.total_cmp(&b.0));

    let mut out = Vec::new();
    for (_, a, b) in edges {
        let (ra, rb) = (find(&mut parent, a), find(&mut parent, b));
        if ra != rb {
            parent[ra as usize] = rb;
            out.push((a as usize, b as usize));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mst_connects_all_islands() {
        let pts: Vec<[f64; 2]> = (0..100).map(|i| [(i % 10) as f64, (i / 10) as f64]).collect();
        let islands: Vec<u32> = (0..100).collect();
        assert_eq!(mst(&pts, &islands).len(), 99);
    }

    #[test]
    fn same_island_needs_no_line() {
        let pts = [[0.0, 0.0], [1.0, 0.0], [5.0, 0.0]];
        let lines = mst(&pts, &[7, 7, 9]);
        assert_eq!(lines.len(), 1);
        assert!(lines[0] == (1, 2) || lines[0] == (2, 1));
    }

    #[test]
    fn collinear_large_net() {
        let pts: Vec<[f64; 2]> = (0..50).map(|i| [i as f64, 0.0]).collect();
        let islands: Vec<u32> = (0..50).collect();
        assert_eq!(mst(&pts, &islands).len(), 49);
    }
}
