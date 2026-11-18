//! Clearance measuring: the gap between two pieces of a board (copper, drill holes, the
//! board edge), the points it lies between and the design rule that applies to it. Used
//! by the Measure tool's Clearance mode and MCP's `measure_clearance`.

use model::*;
use serde::{Deserialize, Serialize};

use crate::copper::{BoardConnectivity, CopperItem, Geo, layer_bit};
use crate::fab::pad_hole;

/// Something a clearance is measured from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Feature {
    /// A pad.
    Pad {
        footprint: Id,
        pad: Id,
    },
    Track {
        id: Id,
    },
    Via {
        id: Id,
    },
    /// The fill of a zone on its layer (all its pieces).
    Zone {
        id: Id,
    },
    /// The drill hole of a pad.
    PadHole {
        footprint: Id,
        pad: Id,
    },
    /// The drill hole of a via.
    ViaHole {
        id: Id,
    },
    /// The board outline (all its contours).
    Edge,
}

/// The gap between two features.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Gap {
    /// nm; 0 when they touch or overlap.
    pub distance: f64,
    /// The nearest points: on the first feature and on the second.
    pub a: Point,
    pub b: Point,
    /// The clearance the design rules ask for between them; `None` when none applies
    /// (the same net, different layers, drill holes).
    pub rule: Option<Coord>,
}

impl Gap {
    /// Closer than the rule allows.
    pub fn violates(&self) -> bool {
        self.rule.is_some_and(|r| self.distance < r as f64)
    }
}

impl Feature {
    fn copper(self) -> Option<CopperItem> {
        match self {
            Feature::Pad { footprint, pad } => Some(CopperItem::Pad { footprint, pad }),
            Feature::Track { id } => Some(CopperItem::Track(id)),
            Feature::Via { id } => Some(CopperItem::Via(id)),
            _ => None,
        }
    }

    fn of(item: CopperItem) -> Feature {
        match item {
            CopperItem::Pad { footprint, pad } => Feature::Pad { footprint, pad },
            CopperItem::Track(id) => Feature::Track { id },
            CopperItem::Via(id) => Feature::Via { id },
            CopperItem::Zone { zone, .. } => Feature::Zone { id: zone },
        }
    }
}

/// What a click at `p` on copper layer `layer` measures from: a drill hole under it, else
/// the copper (pads, vias, tracks, then zone fills), else the board edge within `tol`.
pub fn feature_at(
    project: &Project,
    copper: &BoardConnectivity,
    p: Point,
    layer: LayerId,
    tol: Coord,
) -> Option<Feature> {
    let board = &project.board;
    let under = copper.at(p, layer);
    match under.map(|c| c.item) {
        Some(CopperItem::Via(id)) => {
            if let Some(v) = board.vias.get(id)
                && (v.pos - p).length() <= v.drill as f64 / 2.0
            {
                return Some(Feature::ViaHole { id });
            }
        }
        Some(CopperItem::Pad { footprint, pad })
            if pad_hole_geo(project, footprint, pad).is_some_and(|g| g.contains(p)) =>
        {
            return Some(Feature::PadHole { footprint, pad });
        }
        _ => {}
    }
    if let Some(c) = under {
        return Some(Feature::of(c.item));
    }
    // Non-plated holes have no copper.
    for f in board.footprints.iter() {
        let Some(fp) = crate::query::footprint_of_instance(project, f) else { continue };
        let t = f.transform();
        for pad in fp.pads.iter().filter(|pad| matches!(pad.kind, PadKind::NonPlatedHole { .. })) {
            if pad_hole(pad, &t).is_some_and(|h| hole_geo(h).contains(p)) {
                return Some(Feature::PadHole { footprint: f.id, pad: pad.id });
            }
        }
    }
    let near_edge = edge_geos(board).iter().any(|e| e.nearest(&Geo::Capsule { a: p, b: p, r: 0 }).0 <= tol as f64);
    near_edge.then_some(Feature::Edge)
}

/// The shapes of a feature in board coordinates (empty if it no longer exists).
pub fn geometry(project: &Project, copper: &BoardConnectivity, f: Feature) -> Vec<Geo> {
    match f {
        Feature::Zone { id } => copper
            .items
            .iter()
            .filter(|c| matches!(c.item, CopperItem::Zone { zone, .. } if zone == id))
            .map(|c| c.geo.clone())
            .collect(),
        Feature::PadHole { footprint, pad } => pad_hole_geo(project, footprint, pad).into_iter().collect(),
        Feature::ViaHole { id } => project
            .board
            .vias
            .get(id)
            .map(|v| Geo::Capsule { a: v.pos, b: v.pos, r: v.drill / 2 })
            .into_iter()
            .collect(),
        Feature::Edge => edge_geos(&project.board),
        _ => f.copper().and_then(|item| copper.get(item)).map(|c| c.geo.clone()).into_iter().collect(),
    }
}

/// The gap between `a` and `b`. `None` if either is gone.
pub fn gap(project: &Project, copper: &BoardConnectivity, a: Feature, b: Feature) -> Option<Gap> {
    let (ga, gb) = (geometry(project, copper, a), geometry(project, copper, b));
    let (distance, pa, pb) =
        ga.iter().flat_map(|x| gb.iter().map(move |y| x.nearest(y))).min_by(|x, y| x.0.total_cmp(&y.0))?;
    Some(Gap { distance, a: pa, b: pb, rule: rule(project, copper, a, b) })
}

/// The design rule clearance between two features: copper of different nets (or
/// without a net) on a common layer, and copper to the board edge.
fn rule(project: &Project, copper: &BoardConnectivity, a: Feature, b: Feature) -> Option<Coord> {
    let clear = copper.clearances(&project.board.rules);
    let info = |f: Feature| -> Option<(u64, Option<u32>)> {
        let item = match f {
            Feature::Zone { id } => {
                copper.items.iter().find(|c| matches!(c.item, CopperItem::Zone { zone, .. } if zone == id))?.item
            }
            _ => f.copper()?,
        };
        let c = copper.get(item)?;
        Some((c.layers, c.net))
    };
    match (a, b) {
        (Feature::Edge, Feature::Edge) => None,
        (Feature::Edge, other) | (other, Feature::Edge) => info(other).map(|_| clear.default),
        _ => {
            let ((la, na), (lb, nb)) = (info(a)?, info(b)?);
            let same_net = na.is_some() && na == nb;
            (la & lb != 0 && !same_net).then_some(clear.between(na, nb))
        }
    }
}

/// "pad R1.2 (GND)", "hole of via", "board edge".
pub fn name(project: &Project, copper: &BoardConnectivity, f: Feature) -> String {
    let refdes = |footprint: Id| {
        project
            .board
            .footprints
            .get(footprint)
            .and_then(|f| project.schematic.components.get(f.component))
            .map_or("?".to_string(), |c| c.refdes.clone())
    };
    match f {
        Feature::Edge => "board edge".into(),
        Feature::ViaHole { .. } => "via hole".into(),
        Feature::PadHole { footprint, pad } => {
            let name = project
                .board
                .footprints
                .get(footprint)
                .and_then(|i| crate::query::footprint_of_instance(project, i))
                .and_then(|fp| fp.pads.get(pad))
                .map_or("?".to_string(), |p| p.name.clone());
            format!("hole of pad {}.{name}", refdes(footprint))
        }
        Feature::Zone { id } => {
            let net = project.board.zones.get(id).map_or("", |z| z.net.as_str());
            if net.is_empty() { "zone (no net)".into() } else { format!("zone ({net})") }
        }
        _ => f
            .copper()
            .and_then(|item| copper.get(item))
            .map_or("?".into(), |c| crate::drc::copper_name(project, copper, c)),
    }
}

/// Is `f` on copper layer `layer` (holes and the edge are on all)?
pub fn on_layer(copper: &BoardConnectivity, f: Feature, layer: LayerId) -> bool {
    match f.copper() {
        Some(item) => copper.get(item).is_some_and(|c| c.layers & layer_bit(layer) != 0),
        None => true,
    }
}

fn hole_geo(h: crate::fab::Hole) -> Geo {
    Geo::Capsule { a: h.a, b: h.b, r: h.diameter / 2 }
}

fn pad_hole_geo(project: &Project, footprint: Id, pad: Id) -> Option<Geo> {
    let f = project.board.footprints.get(footprint)?;
    let fp = crate::query::footprint_of_instance(project, f)?;
    pad_hole(fp.pads.get(pad)?, &f.transform()).map(hole_geo)
}

/// The board outline's contours as closed paths of zero width.
fn edge_geos(board: &Board) -> Vec<Geo> {
    let Some(area) = crate::zones::board_area(board) else { return Vec::new() };
    area.iter()
        .map(|c| {
            let mut points: Vec<Point> =
                c.iter().map(|p| Point::new(p[0].round() as Coord, p[1].round() as Coord)).collect();
            if let Some(&first) = points.first() {
                points.push(first);
            }
            Geo::Path { points, r: 0 }
        })
        .collect()
}
