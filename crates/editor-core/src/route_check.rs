//! Checks while routing and placing vias: the copper about to be added keeps clearance
//! to copper of other nets (pads, tracks, vias; zone fills make room by themselves),
//! stays out of keepouts that forbid it, and stays inside the board outline.

use model::*;

use crate::copper::{BoardConnectivity, CopperItem, Geo, LayerMask, RegionIndex, layer_bit, track_geo};

/// Copper the route or via tool would add.
#[derive(Clone, Debug, PartialEq)]
pub enum Probe {
    Track { layer: LayerId, a: Point, b: Point, mid: Option<Point>, width: Coord },
    Via { pos: Point, diameter: Coord, layers: LayerMask },
}

impl Probe {
    fn geo(&self) -> Geo {
        match *self {
            Probe::Track { layer, a, b, mid, width } => {
                let mut t = Track::straight(layer, a, b, width);
                t.mid = mid;
                track_geo(&t)
            }
            Probe::Via { pos, diameter, .. } => Geo::Capsule { a: pos, b: pos, r: diameter / 2 },
        }
    }

    fn layers(&self) -> LayerMask {
        match *self {
            Probe::Track { layer, .. } => layer_bit(layer),
            Probe::Via { layers, .. } => layers,
        }
    }
}

/// A rule the new copper breaks: where, and what.
#[derive(Clone, Debug, PartialEq)]
pub struct Clash {
    pub at: Point,
    pub message: String,
}

/// The first rule `probes` break. They belong to `net` (`None`: not known yet, a route
/// started in free space), and connect to the copper at `ends` (the leg's start and end
/// points), which with all copper touching it is never a clash.
pub fn check(
    project: &Project,
    copper: &BoardConnectivity,
    probes: &[Probe],
    net: Option<&str>,
    ends: &[(LayerId, Point)],
) -> Option<Clash> {
    let board = &project.board;
    let clear = copper.clearances(&board.rules);
    let net_rules = board.rules.for_net(net);
    let net = net.and_then(|n| copper.net_index(n));
    // Islands the route connects to: at its start (and at its end if the net isn't
    // known, as the end then decides it).
    let islands: Vec<u32> = ends
        .iter()
        .enumerate()
        .filter(|(i, _)| *i == 0 || net.is_none())
        .filter_map(|(_, &(l, p))| copper.at(p, l))
        .filter(|c| !matches!(c.item, CopperItem::Zone { .. }))
        .map(|c| c.island)
        .collect();
    let mm = |v: f64| format!("{:.3} mm", v * 1e-6);
    let geos: Vec<(Geo, &Probe)> = probes.iter().map(|p| (p.geo(), p)).collect();

    for (geo, probe) in &geos {
        let bits = probe.layers();
        let reach = geo.bounds().grow(clear.max.max(net_rules.clearance));
        for c in &copper.items {
            if c.layers & bits == 0
                || matches!(c.item, CopperItem::Zone { .. })
                || islands.contains(&c.island)
                || (net.is_some() && c.net == net)
                || !c.geo.bounds().intersects(&reach)
            {
                continue;
            }
            // The new copper's class is known by name even before its net has copper.
            let clearance = net_rules.clearance.max(clear.of(c.net));
            let d = c.geo.distance(geo);
            if d < clearance as f64 {
                let other = c.net.map_or("unconnected copper".to_string(), |n| copper.nets[n as usize].clone());
                let what = match c.item {
                    CopperItem::Pad { .. } => "a pad",
                    CopperItem::Via(_) => "a via",
                    _ => "a track",
                };
                let message = if d <= 0.0 {
                    format!("touches {what} of {other}")
                } else {
                    format!("{} from {what} of {other} (clearance {})", mm(d), mm(clearance as f64))
                };
                return Some(Clash { at: nearest(geo, &c.geo), message });
            }
        }
    }

    for k in board.keepouts.iter().filter(|k| k.outline.len() >= 3) {
        let kbits = board.copper_layers().filter(|l| k.on_layer(l.id)).fold(0, |a, l| a | layer_bit(l.id));
        let region = RegionIndex::new(&k.outline, &k.holes);
        for (geo, probe) in &geos {
            let (forbidden, what) = match probe {
                Probe::Track { .. } => (k.no_tracks, "tracks"),
                Probe::Via { .. } => (k.no_vias, "vias"),
            };
            if forbidden && probe.layers() & kbits != 0 && region.touches(geo) {
                let at = nearest(geo, &Geo::Polygon(k.outline.clone()));
                return Some(Clash { at, message: format!("in a keepout that forbids {what}") });
            }
        }
    }

    if let Some(area) = crate::zones::board_area(board) {
        let rings: Vec<Vec<Point>> = area
            .iter()
            .map(|c| c.iter().map(|p| Point::new(p[0].round() as Coord, p[1].round() as Coord)).collect())
            .collect();
        for (geo, _) in &geos {
            let inside = |p: Point| rings.iter().filter(|r| crate::copper::point_in_polygon(p, r)).count() % 2 == 1;
            let edge = rings
                .iter()
                .flat_map(|r| (0..r.len()).map(move |i| Geo::Capsule { a: r[i], b: r[(i + 1) % r.len()], r: 0 }))
                .find(|e| e.distance(geo) < clear.default as f64);
            let probe_point = match geo {
                Geo::Capsule { a, .. } => *a,
                _ => geo.bounds().center(),
            };
            if let Some(e) = edge {
                return Some(Clash {
                    at: nearest(geo, &e),
                    message: format!("too close to the board edge (clearance {})", mm(clear.default as f64)),
                });
            }
            if !inside(probe_point) {
                return Some(Clash { at: probe_point, message: "outside the board".into() });
            }
        }
    }
    None
}

/// The point on the axis of `geo` (a capsule or path) nearest to `other`, for a marker.
fn nearest(geo: &Geo, other: &Geo) -> Point {
    let axis: Vec<Point> = match geo {
        Geo::Capsule { a, b, .. } => vec![*a, *b],
        Geo::Path { points, .. } => points.clone(),
        _ => return geo.bounds().center(),
    };
    let mut best = (f64::MAX, axis[0]);
    for w in axis.windows(2) {
        const N: i64 = 16;
        for i in 0..=N {
            let p = Point::new(w[0].x + (w[1].x - w[0].x) * i / N, w[0].y + (w[1].y - w[0].y) * i / N);
            let d = Geo::Capsule { a: p, b: p, r: 0 }.distance(other);
            if d < best.0 {
                best = (d, p);
            }
        }
    }
    best.1
}
