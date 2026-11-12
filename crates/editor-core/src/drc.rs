//! Design rule check: clearance between copper of different nets, shorts, unrouted
//! connections, copper not touching any pad, courtyard overlaps, copper at or outside
//! the board edge, and minimum track width and drill.
//!
//! Zone fills are not checked for clearance: they are built to keep it (see
//! [`crate::zones`]).

use model::*;
use rustc_hash::{FxHashMap as HashMap, FxHashSet as HashSet};
use serde::Serialize;

use crate::copper::{BoardConnectivity, CopperItem, Flag, Geo, Grid};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ViolationKind {
    Short,
    Clearance,
    BoardEdge,
    Courtyard,
    TrackWidth,
    Drill,
    /// A zone's net name matches no net on the board (renamed, or its pads are gone),
    /// so it connects nothing.
    ZoneNet,
    Unconnected,
    Unrouted,
}

impl ViolationKind {
    pub fn label(self) -> &'static str {
        match self {
            ViolationKind::Short => "Short",
            ViolationKind::Clearance => "Clearance",
            ViolationKind::BoardEdge => "Board edge",
            ViolationKind::Courtyard => "Courtyard overlap",
            ViolationKind::TrackWidth => "Track width",
            ViolationKind::Drill => "Drill",
            ViolationKind::ZoneNet => "Zone net",
            ViolationKind::Unconnected => "Unconnected copper",
            ViolationKind::Unrouted => "Unrouted",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, schemars::JsonSchema)]
pub struct Violation {
    pub kind: ViolationKind,
    pub message: String,
    /// Where it is (for centering the view).
    pub pos: Point,
    /// Objects involved (footprints for pads).
    pub objects: Vec<ObjectId>,
}

fn fmt_mm(v: f64) -> String {
    format!("{:.3} mm", v * 1e-6)
}

/// Check the board. Violations are sorted by kind (most severe first).
pub fn run(project: &Project, copper: &BoardConnectivity) -> Vec<Violation> {
    let board = &project.board;
    let rules = &board.rules;
    let mut out = Vec::new();
    let refdes = |f: &FootprintInstance| -> &str {
        project.schematic.components.get(f.component).map_or("?", |c| c.refdes.as_str())
    };
    let name = |c: &crate::copper::Copper| {
        let net = c.net.map_or("no net", |n| copper.nets[n as usize].as_str());
        let what = match c.item {
            CopperItem::Pad { footprint, pad } => {
                let fp = board.footprints.get(footprint);
                let pad =
                    fp.and_then(|f| crate::query::footprint_of_instance(project, f)).and_then(|d| d.pads.get(pad));
                format!("pad {}.{}", fp.map_or("?", refdes), pad.map_or("?", |p| p.name.as_str()))
            }
            CopperItem::Track(_) => "track".into(),
            CopperItem::Via(_) => "via".into(),
            CopperItem::Zone { .. } => "zone".into(),
        };
        format!("{what} ({net})")
    };

    // Copper by anchor point.
    let mut at_anchor: HashMap<Point, Vec<usize>> = HashMap::default();
    for (i, c) in copper.items.iter().enumerate() {
        for &p in &c.anchors {
            at_anchor.entry(p).or_default().push(i);
        }
    }
    let mut shorted: HashMap<u32, Vec<ObjectId>> = HashMap::default();
    for c in copper.items.iter().filter(|c| c.flag == Some(Flag::Short)) {
        shorted.entry(c.island).or_default().push(c.item.object());
    }

    // Shorts: one per island with several nets.
    for (nets, at) in &copper.shorts {
        let island = at_anchor.get(at).and_then(|v| v.first()).map(|&i| copper.items[i].island);
        let objects = island.and_then(|i| shorted.get(&i)).cloned().unwrap_or_default();
        out.push(Violation {
            kind: ViolationKind::Short,
            message: format!("Short between {}", nets.join(" and ")),
            pos: *at,
            objects,
        });
    }

    // Clearance between copper of different islands (unless of the same net).
    let checked: Vec<usize> =
        (0..copper.items.len()).filter(|&i| !matches!(copper.items[i].item, CopperItem::Zone { .. })).collect();
    let bounds: Vec<Rect> = checked.iter().map(|&i| copper.items[i].geo.bounds().grow(rules.clearance)).collect();
    let mut seen: HashSet<(usize, usize)> = HashSet::default();
    Grid::new(&bounds).pairs(|i, j| {
        let (i, j) = (checked[i.min(j)], checked[i.max(j)]);
        let (a, b) = (&copper.items[i], &copper.items[j]);
        if a.layers & b.layers == 0
            || a.island == b.island
            || (a.net.is_some() && a.net == b.net)
            || !a.geo.bounds().grow(rules.clearance).intersects(&b.geo.bounds())
        {
            return;
        }
        // Pads of one footprint are the footprint's business.
        if let (CopperItem::Pad { footprint: f, .. }, CopperItem::Pad { footprint: g, .. }) = (a.item, b.item)
            && f == g
        {
            return;
        }
        if !seen.insert((i, j)) {
            return;
        }
        let d = a.geo.distance(&b.geo);
        if d < rules.clearance as f64 {
            out.push(Violation {
                kind: ViolationKind::Clearance,
                message: format!(
                    "{} and {}: {} apart (clearance {})",
                    name(a),
                    name(b),
                    fmt_mm(d),
                    fmt_mm(rules.clearance as f64)
                ),
                pos: midpoint(a, b),
                objects: vec![a.item.object(), b.item.object()],
            });
        }
    });

    // Board edge.
    if let Some(area) = crate::zones::board_area(board) {
        let rings: Vec<Vec<Point>> = area
            .iter()
            .map(|c| c.iter().map(|p| Point::new(p[0].round() as Coord, p[1].round() as Coord)).collect())
            .collect();
        let edges: Vec<Geo> = rings
            .iter()
            .flat_map(|r| (0..r.len()).map(move |i| Geo::Capsule { a: r[i], b: r[(i + 1) % r.len()], r: 0 }))
            .collect();
        for c in copper.items.iter().filter(|c| !matches!(c.item, CopperItem::Zone { .. })) {
            let p = c.anchors[0];
            let inside = rings.iter().any(|r| crate::copper::point_in_polygon(p, r));
            let near = edges.iter().map(|e| e.distance(&c.geo)).fold(f64::MAX, f64::min);
            if !inside || near < rules.clearance as f64 {
                let message = if inside && near <= 0.0 {
                    format!("{} crosses the board edge", name(c))
                } else if inside {
                    format!(
                        "{} is {} from the board edge (clearance {})",
                        name(c),
                        fmt_mm(near),
                        fmt_mm(rules.clearance as f64)
                    )
                } else {
                    format!("{} is outside the board", name(c))
                };
                out.push(Violation { kind: ViolationKind::BoardEdge, message, pos: p, objects: vec![c.item.object()] });
            }
        }
    }

    // Courtyards of footprints on the same side.
    let courtyards: Vec<(Id, &str, Side, Rect)> = board
        .footprints
        .iter()
        .filter_map(|f| {
            let def = crate::query::footprint_of_instance(project, f)?;
            let t = f.transform();
            let r = Rect::union_all(
                def.graphics
                    .iter()
                    .filter(|g| g.layer == FootprintLayer::Courtyard)
                    .map(|g| g.shape.transformed(&t).bounds()),
            )?;
            Some((f.id, refdes(f), f.side, r))
        })
        .collect();
    let rects: Vec<Rect> = courtyards.iter().map(|c| c.3).collect();
    let mut seen: HashSet<(usize, usize)> = HashSet::default();
    Grid::new(&rects).pairs(|i, j| {
        let (i, j) = (i.min(j), i.max(j));
        let (a, b) = (&courtyards[i], &courtyards[j]);
        let overlap = Rect::from_points(
            Point::new(a.3.min.x.max(b.3.min.x), a.3.min.y.max(b.3.min.y)),
            Point::new(a.3.max.x.min(b.3.max.x), a.3.max.y.min(b.3.max.y)),
        );
        let overlaps =
            a.3.min.x.max(b.3.min.x) < a.3.max.x.min(b.3.max.x) && a.3.min.y.max(b.3.min.y) < a.3.max.y.min(b.3.max.y);
        if a.2 == b.2 && overlaps && seen.insert((i, j)) {
            out.push(Violation {
                kind: ViolationKind::Courtyard,
                message: format!("Courtyards of {} and {} overlap", a.1, b.1),
                pos: overlap.center(),
                objects: vec![ObjectId::Footprint(a.0), ObjectId::Footprint(b.0)],
            });
        }
    });

    // Zones bind to a net by name; a renamed net leaves them on none.
    for z in board.zones.iter().filter(|z| copper.net_index(&z.net).is_none()) {
        out.push(Violation {
            kind: ViolationKind::ZoneNet,
            message: format!("Zone net {:?} has no pads on the board", z.net),
            pos: z.outline.first().copied().unwrap_or_default(),
            objects: vec![ObjectId::Zone(z.id)],
        });
    }

    // Sizes.
    for t in board.tracks.iter().filter(|t| t.width < rules.min_track_width) {
        out.push(Violation {
            kind: ViolationKind::TrackWidth,
            message: format!(
                "Track width {} is below the minimum {}",
                fmt_mm(t.width as f64),
                fmt_mm(rules.min_track_width as f64)
            ),
            pos: Point::new((t.a.x + t.b.x) / 2, (t.a.y + t.b.y) / 2),
            objects: vec![ObjectId::Track(t.id)],
        });
    }
    let drill = |what: String, d: Coord, pos: Point, id: ObjectId| Violation {
        kind: ViolationKind::Drill,
        message: format!("{what} drill {} is below the minimum {}", fmt_mm(d as f64), fmt_mm(rules.min_drill as f64)),
        pos,
        objects: vec![id],
    };
    for v in board.vias.iter().filter(|v| v.drill < rules.min_drill) {
        out.push(drill("Via".into(), v.drill, v.pos, ObjectId::Via(v.id)));
    }
    for f in board.footprints.iter() {
        let Some(def) = crate::query::footprint_of_instance(project, f) else { continue };
        let t = f.transform();
        for pad in def.pads.iter() {
            let (PadKind::ThroughHole { drill: d } | PadKind::NonPlatedHole { drill: d }) = pad.kind else { continue };
            if d < rules.min_drill {
                out.push(drill(
                    format!("Pad {}.{}", refdes(f), pad.name),
                    d,
                    t.point(pad.pos),
                    ObjectId::Footprint(f.id),
                ));
            }
        }
    }

    // Copper that touches no pad, and missing connections.
    for c in copper.flagged().filter(|c| c.flag == Some(Flag::Unconnected)) {
        out.push(Violation {
            kind: ViolationKind::Unconnected,
            message: format!("{} touches no pad", name(c)),
            pos: c.anchors[0],
            objects: vec![c.item.object()],
        });
    }
    for r in &copper.ratsnest {
        let net = &copper.nets[r.net as usize];
        let mut objects: Vec<ObjectId> = [r.a, r.b]
            .iter()
            .flat_map(|p| at_anchor.get(p).into_iter().flatten())
            .filter(|&&i| copper.items[i].net == Some(r.net))
            .map(|&i| copper.items[i].item.object())
            .collect();
        objects.dedup();
        out.push(Violation {
            kind: ViolationKind::Unrouted,
            message: format!("{net}: unrouted connection ({})", fmt_mm((r.b - r.a).length())),
            pos: Point::new((r.a.x + r.b.x) / 2, (r.a.y + r.b.y) / 2),
            objects,
        });
    }

    out.sort_by_key(|v| v.kind);
    out
}

fn midpoint(a: &crate::copper::Copper, b: &crate::copper::Copper) -> Point {
    let (ra, rb) = (a.geo.bounds(), b.geo.bounds());
    let (ca, cb) = (ra.center(), rb.center());
    Point::new((ca.x + cb.x) / 2, (ca.y + cb.y) / 2)
}
