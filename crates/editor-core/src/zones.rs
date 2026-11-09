//! Copper zone filling.
//!
//! A zone fills its outline (clipped to the board outline, minus the edge clearance)
//! except around copper of other nets, which keeps the clearance. Pads of the zone's
//! own net connect through thermal reliefs (a gap with four spokes); its tracks and vias
//! are simply flooded. Higher-priority zones of other nets keep the clearance too.
//! Fill islands that touch no copper of the zone's net are removed.
//!
//! Fills are derived data (like connectivity), computed from the project; they are not
//! stored in the project files.

use i_overlay::core::fill_rule::FillRule;
use i_overlay::core::overlay_rule::OverlayRule;
use i_overlay::float::single::SingleFloatOverlay;
use model::*;

use crate::copper::{BoardConnectivity, CopperItem, Geo, RegionIndex, layer_bit};

/// The filled copper of one zone: shapes of `[outline, holes…]`.
#[derive(Clone, Debug, PartialEq)]
pub struct ZoneFill {
    pub zone: Id,
    pub layer: LayerId,
    pub net: String,
    pub shapes: Vec<Vec<Vec<Point>>>,
}

type Contour = Vec<[f64; 2]>;

fn pt(p: Point) -> [f64; 2] {
    [p.x as f64, p.y as f64]
}

fn unpt(p: [f64; 2]) -> Point {
    Point::new(p[0].round() as Coord, p[1].round() as Coord)
}

fn area(c: &[[f64; 2]]) -> f64 {
    let n = c.len();
    (0..n).map(|i| c[i][0] * c[(i + 1) % n][1] - c[(i + 1) % n][0] * c[i][1]).sum::<f64>() / 2.0
}

/// The contour with the given winding (counterclockwise: outlines, clockwise: holes).
fn oriented(mut c: Contour, ccw: bool) -> Contour {
    if (area(&c) > 0.0) != ccw {
        c.reverse();
    }
    c
}

/// Segments per full circle for a radius (nm): finer for larger arcs.
fn segments(r: f64) -> usize {
    ((r / 50_000.0).sqrt() * 8.0).clamp(16.0, 64.0) as usize
}

/// A stadium (segment with round ends), slightly larger than the exact shape so that
/// clearances are never undercut. Counterclockwise.
pub fn capsule_contour(a: Point, b: Point, r: Coord) -> Contour {
    let r = r as f64;
    let n = segments(r);
    let rr = r / (std::f64::consts::PI / n as f64).cos();
    let (a, b) = (pt(a), pt(b));
    let dir = (b[1] - a[1]).atan2(b[0] - a[0]);
    let half = n / 2;
    let mut out = Vec::with_capacity(n + 2);
    // Around b from -90° to +90° (relative to the direction), then around a.
    for (c, start) in [(b, dir - std::f64::consts::FRAC_PI_2), (a, dir + std::f64::consts::FRAC_PI_2)] {
        for i in 0..=half {
            let t = start + std::f64::consts::PI * i as f64 / half as f64;
            out.push([c[0] + rr * t.cos(), c[1] + rr * t.sin()]);
        }
    }
    if a == b {
        out.truncate(half + 1);
        // A full circle.
        out = (0..n)
            .map(|i| {
                let t = 2.0 * std::f64::consts::PI * i as f64 / n as f64;
                [a[0] + rr * t.cos(), a[1] + rr * t.sin()]
            })
            .collect();
    }
    out
}

/// Contours covering `g` grown by `d` (union under the non-zero fill rule).
fn inflate(g: &Geo, d: Coord) -> Vec<Contour> {
    match g {
        Geo::Capsule { a, b, r } => vec![capsule_contour(*a, *b, r + d)],
        Geo::Polygon(p) => grown(p, &[], d),
        Geo::Region { outer, holes } => grown(outer, holes, d),
    }
}

fn grown(outer: &[Point], holes: &[Vec<Point>], d: Coord) -> Vec<Contour> {
    let mut out = vec![oriented(outer.iter().map(|&p| pt(p)).collect(), true)];
    out.extend(holes.iter().map(|h| oriented(h.iter().map(|&p| pt(p)).collect(), false)));
    if d > 0 {
        for ring in std::iter::once(outer).chain(holes.iter().map(Vec::as_slice)) {
            for i in 0..ring.len() {
                out.push(capsule_contour(ring[i], ring[(i + 1) % ring.len()], d));
            }
        }
    }
    out
}

fn geo_contours(g: &Geo) -> Vec<Contour> {
    inflate(g, 0)
}

/// Contours of the board outline (rectangles, polygons and circles on the outline
/// layer; lines only give their bounding box). `None` without an outline.
pub fn board_area(board: &Board) -> Option<Vec<Contour>> {
    let mut out = Vec::new();
    let mut lines: Option<Rect> = None;
    for g in board.graphics.iter().filter(|g| g.layer == LayerId::OUTLINE) {
        match &g.shape {
            Shape::Rect { a, b, .. } => {
                let r = Rect::from_points(*a, *b);
                out.push(rect_contour(r));
            }
            Shape::Polygon { points, .. } if points.len() >= 3 => {
                out.push(oriented(points.iter().map(|&p| pt(p)).collect(), true))
            }
            Shape::Circle { center, radius, .. } => out.push(capsule_contour(*center, *center, *radius)),
            other => {
                let b = other.bounds();
                lines = Some(lines.map_or(b, |l| l.union(b)));
            }
        }
    }
    if out.is_empty()
        && let Some(r) = lines
    {
        out.push(rect_contour(r));
    }
    (!out.is_empty()).then_some(out)
}

fn rect_contour(r: Rect) -> Contour {
    vec![pt(r.min), [r.max.x as f64, r.min.y as f64], pt(r.max), [r.min.x as f64, r.max.y as f64]]
}

/// Thermal relief for a pad: the ring around it (pad grown by `gap`) minus four spokes.
fn thermal_cut(g: &Geo, center: Point, gap: Coord, spoke: Coord) -> Vec<Contour> {
    let ring = inflate(g, gap);
    let b = g.bounds();
    let len = b.width().max(b.height()) / 2 + gap + 500_000;
    let w = spoke / 2;
    let mut keep = geo_contours(g);
    keep.push(rect_contour(Rect::from_points(center - Point::new(len, w), center + Point::new(len, w))));
    keep.push(rect_contour(Rect::from_points(center - Point::new(w, len), center + Point::new(w, len))));
    let cut = vec![ring].overlay(&vec![keep], OverlayRule::Difference, FillRule::NonZero);
    cut.into_iter().flatten().collect()
}

/// Fill every zone of the board. `copper` gives the nets of pads, tracks and vias (zones
/// are not part of it yet).
pub fn fill_zones(project: &Project, copper: &BoardConnectivity) -> Vec<ZoneFill> {
    let board = &project.board;
    let rules = &board.rules;
    let outline_area = board_area(board);
    let mut zones: Vec<&Zone> = board.zones.iter().filter(|z| z.outline.len() >= 3).collect();
    zones.sort_by_key(|z| std::cmp::Reverse(z.priority));
    let mut fills: Vec<ZoneFill> = Vec::new();
    for z in zones {
        let clearance = z.clearance.unwrap_or(rules.clearance);
        let bit = layer_bit(z.layer);
        let net = copper.net_index(&z.net);
        let outline: Contour = oriented(z.outline.iter().map(|&p| pt(p)).collect(), true);
        let mut base: Vec<Vec<Contour>> = vec![vec![outline]];
        let mut clip: Vec<Contour> = Vec::new();
        if let Some(area) = &outline_area {
            base = base.overlay(&vec![area.clone()], OverlayRule::Intersect, FillRule::NonZero);
            // Edge clearance.
            for c in area {
                let ring: Vec<Point> = c.iter().map(|&p| unpt(p)).collect();
                for i in 0..ring.len() {
                    clip.push(capsule_contour(ring[i], ring[(i + 1) % ring.len()], clearance));
                }
            }
        }
        let zb = Rect::union_all(z.outline.iter().map(|&p| Rect::from_points(p, p))).unwrap().grow(clearance * 4);
        for c in copper.items.iter().filter(|c| c.layers & bit != 0 && c.geo.bounds().intersects(&zb)) {
            let same = net.is_some() && c.net == net;
            match c.item {
                CopperItem::Pad { .. } if same => {
                    clip.extend(thermal_cut(&c.geo, c.anchors[0], rules.thermal_gap, rules.thermal_spoke))
                }
                CopperItem::Pad { .. } => clip.extend(inflate(&c.geo, clearance)),
                // Own tracks and vias are flooded; so is copper without a net (stitching vias).
                _ if same || c.net.is_none() => {}
                _ => clip.extend(inflate(&c.geo, clearance)),
            }
        }
        for f in fills.iter().filter(|f| f.layer == z.layer && f.net != z.net) {
            for shape in &f.shapes {
                let (outer, holes) = shape.split_first().expect("shapes have an outline");
                clip.extend(grown(outer, holes, clearance));
            }
        }
        let filled =
            if clip.is_empty() { base } else { base.overlay(&vec![clip], OverlayRule::Difference, FillRule::NonZero) };

        // Keep the islands that reach copper of the zone's net.
        let own: Vec<&Geo> = copper
            .items
            .iter()
            .filter(|c| c.layers & bit != 0 && net.is_some() && c.net == net)
            .map(|c| &c.geo)
            .collect();
        let shapes: Vec<Vec<Vec<Point>>> = filled
            .into_iter()
            .filter(|s| !s.is_empty() && area(&s[0]).abs() > 1e6)
            .map(|s| s.into_iter().map(|c| c.into_iter().map(unpt).collect()).collect::<Vec<Vec<Point>>>())
            .filter(|s| {
                let b = Rect::union_all(s[0].iter().map(|&p| Rect::from_points(p, p))).unwrap();
                let region = RegionIndex::new(&s[0], &s[1..]);
                own.iter().any(|g| g.bounds().intersects(&b) && region.touches(g))
            })
            .collect();
        fills.push(ZoneFill { zone: z.id, layer: z.layer, net: z.net.clone(), shapes });
    }
    fills
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capsules_are_closed_and_counterclockwise() {
        let c = capsule_contour(Point::mm(0.0, 0.0), Point::mm(5.0, 0.0), 500_000);
        assert!(area(&c) > 0.0);
        // Covers the exact stadium area (circumscribed).
        let exact = 5e6 * 1e6 + std::f64::consts::PI * 0.5e6 * 0.5e6;
        assert!(area(&c) >= exact && area(&c) < exact * 1.02);
        let dot = capsule_contour(Point::mm(1.0, 1.0), Point::mm(1.0, 1.0), 300_000);
        assert!(area(&dot) > std::f64::consts::PI * 0.3e6 * 0.3e6);
    }
}
