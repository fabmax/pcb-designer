//! Board: layer stack, footprint placements, copper (tracks, vias, zones), graphics.

use serde::{Deserialize, Serialize};

use crate::geom::{Angle, Point, Shape, Text, Transform};
use crate::id::Id;
use crate::library::{FootprintLayer, has_id};
use crate::store::Store;
use crate::units::{Coord, mm};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Side {
    #[default]
    Top,
    Bottom,
}

impl Side {
    pub fn flipped(self) -> Side {
        match self {
            Side::Top => Side::Bottom,
            Side::Bottom => Side::Top,
        }
    }
}

/// Layer id. Copper layers are numbered from the top: 0 = top, `COPPER_BOTTOM` = bottom,
/// 1… = inner layers. Non-copper layers have fixed ids ≥ 100.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(transparent)]
pub struct LayerId(pub u16);

impl LayerId {
    pub const TOP_COPPER: LayerId = LayerId(0);
    pub const BOTTOM_COPPER: LayerId = LayerId(63);
    pub const TOP_SILK: LayerId = LayerId(100);
    pub const BOTTOM_SILK: LayerId = LayerId(101);
    pub const TOP_MASK: LayerId = LayerId(102);
    pub const BOTTOM_MASK: LayerId = LayerId(103);
    pub const TOP_PASTE: LayerId = LayerId(104);
    pub const BOTTOM_PASTE: LayerId = LayerId(105);
    pub const TOP_FAB: LayerId = LayerId(106);
    pub const BOTTOM_FAB: LayerId = LayerId(107);
    pub const TOP_COURTYARD: LayerId = LayerId(108);
    pub const BOTTOM_COURTYARD: LayerId = LayerId(109);
    pub const OUTLINE: LayerId = LayerId(120);

    /// The matching layer on the other side (top ↔ bottom); inner and other layers stay.
    pub fn other_side(self) -> LayerId {
        match self {
            LayerId::TOP_COPPER => LayerId::BOTTOM_COPPER,
            LayerId::BOTTOM_COPPER => LayerId::TOP_COPPER,
            l if (100..=109).contains(&l.0) => LayerId(l.0 ^ 1),
            l => l,
        }
    }

    pub fn inner(n: u16) -> LayerId {
        assert!((1..63).contains(&n));
        LayerId(n)
    }

    pub fn is_copper(self) -> bool {
        self.0 <= 63
    }

    /// Board layer for a footprint layer on the given side.
    pub fn for_footprint(layer: FootprintLayer, side: Side) -> LayerId {
        let top = side == Side::Top;
        match layer {
            FootprintLayer::Copper => {
                if top {
                    Self::TOP_COPPER
                } else {
                    Self::BOTTOM_COPPER
                }
            }
            FootprintLayer::Silkscreen => {
                if top {
                    Self::TOP_SILK
                } else {
                    Self::BOTTOM_SILK
                }
            }
            FootprintLayer::SolderMask => {
                if top {
                    Self::TOP_MASK
                } else {
                    Self::BOTTOM_MASK
                }
            }
            FootprintLayer::SolderPaste => {
                if top {
                    Self::TOP_PASTE
                } else {
                    Self::BOTTOM_PASTE
                }
            }
            FootprintLayer::Fabrication => {
                if top {
                    Self::TOP_FAB
                } else {
                    Self::BOTTOM_FAB
                }
            }
            FootprintLayer::Courtyard => {
                if top {
                    Self::TOP_COURTYARD
                } else {
                    Self::BOTTOM_COURTYARD
                }
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum LayerFunction {
    Copper,
    Silkscreen,
    SolderMask,
    SolderPaste,
    Fabrication,
    Courtyard,
    Outline,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct BoardLayer {
    pub id: LayerId,
    pub name: String,
    pub function: LayerFunction,
}

/// Physical placement of a schematic component. The footprint itself comes from the
/// component's part variant.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct FootprintInstance {
    pub id: Id,
    pub component: Id,
    pub pos: Point,
    #[serde(default)]
    pub rotation: Angle,
    #[serde(default)]
    pub side: Side,
    #[serde(default)]
    pub locked: bool,
}

impl FootprintInstance {
    /// Footprint → board transform. Bottom side footprints are mirrored.
    pub fn transform(&self) -> Transform {
        Transform::new(self.pos, self.rotation, self.side == Side::Bottom)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Track {
    pub id: Id,
    pub layer: LayerId,
    pub a: Point,
    pub b: Point,
    pub width: Coord,
    /// A point on the arc between `a` and `b` (as in KiCad); `None`: straight.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mid: Option<Point>,
}

impl Track {
    pub fn straight(layer: LayerId, a: Point, b: Point, width: Coord) -> Track {
        Track { id: Id::new(), layer, a, b, width, mid: None }
    }

    /// Move the ends to `a` and `b`. An arc keeps its bulge: the mid point's offset from
    /// the chord, relative to the chord's length (a translation moves it along).
    pub fn set_ends(&mut self, a: Point, b: Point) {
        if let Some(m) = self.mid {
            let (old, new) = (self.b - self.a, b - a);
            let (ol, nl) = (old.length(), new.length());
            self.mid = if ol == 0.0 || nl == 0.0 {
                None
            } else {
                // Mid point in chord coordinates (along, across), scaled to the new chord.
                let rel = m - self.a;
                let (ox, oy) = (old.x as f64 / ol, old.y as f64 / ol);
                let along = (rel.x as f64 * ox + rel.y as f64 * oy) / ol;
                let across = (-rel.x as f64 * oy + rel.y as f64 * ox) / ol;
                let (nx, ny) = (new.x as f64 / nl, new.y as f64 / nl);
                let (u, v) = (along * nl, across * nl);
                Some(a + Point::new((u * nx - v * ny).round() as Coord, (u * ny + v * nx).round() as Coord))
            };
        }
        self.a = a;
        self.b = b;
    }

    /// The arc through `a`, `mid` and `b` as a [`Shape::Arc`]; `None` when straight (no
    /// mid point, or the three points on a line).
    pub fn arc(&self) -> Option<Shape> {
        let ArcGeom { center, radius, start, sweep } = arc_through(self.a, self.mid?, self.b)?;
        Some(Shape::Arc {
            center: Point::new(center.0.round() as Coord, center.1.round() as Coord),
            radius: radius.round() as Coord,
            start: Angle::deg(start.to_degrees()),
            sweep: sweep.to_degrees(),
            width: self.width,
        })
    }

    /// Corner points along the track: its ends, and for an arc points in between, at
    /// most `max_err` from the arc (chords). Starts at `a` and ends at `b` exactly.
    pub fn path(&self, max_err: Coord) -> Vec<Point> {
        let Some(g) = self.mid.and_then(|m| arc_through(self.a, m, self.b)) else { return vec![self.a, self.b] };
        let step = 2.0 * (1.0 - (max_err as f64 / g.radius).min(1.0)).acos();
        let n = ((g.sweep.abs() / step.max(1e-6)).ceil() as usize).clamp(2, 512);
        let mut out = Vec::with_capacity(n + 1);
        out.push(self.a);
        for i in 1..n {
            let t = g.start + g.sweep * i as f64 / n as f64;
            out.push(Point::new(
                (g.center.0 + g.radius * t.cos()).round() as Coord,
                (g.center.1 + g.radius * t.sin()).round() as Coord,
            ));
        }
        out.push(self.b);
        out
    }

    /// Length along the track.
    pub fn length(&self) -> f64 {
        match self.mid.and_then(|m| arc_through(self.a, m, self.b)) {
            Some(g) => g.radius * g.sweep.abs(),
            None => (self.b - self.a).length(),
        }
    }
}

/// A circular arc in floating point (nm, radians; sweep positive = counterclockwise in
/// x/y).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ArcGeom {
    pub center: (f64, f64),
    pub radius: f64,
    pub start: f64,
    pub sweep: f64,
}

/// The arc from `a` through `m` to `b`; `None` if the points are on a line (or equal).
pub fn arc_through(a: Point, m: Point, b: Point) -> Option<ArcGeom> {
    let (ax, ay, mx, my, bx, by) = (a.x as f64, a.y as f64, m.x as f64, m.y as f64, b.x as f64, b.y as f64);
    let d = 2.0 * (ax * (my - by) + mx * (by - ay) + bx * (ay - my));
    // Collinear (within a nanometre of bulge over the chord).
    let chord = ((bx - ax).powi(2) + (by - ay).powi(2)).sqrt();
    if d.abs() <= chord.max(1.0) * 2.0 || a == b {
        return None;
    }
    let (a2, m2, b2) = (ax * ax + ay * ay, mx * mx + my * my, bx * bx + by * by);
    let cx = (a2 * (my - by) + m2 * (by - ay) + b2 * (ay - my)) / d;
    let cy = (a2 * (bx - mx) + m2 * (ax - bx) + b2 * (mx - ax)) / d;
    let radius = ((ax - cx).powi(2) + (ay - cy).powi(2)).sqrt();
    let angle = |x: f64, y: f64| (y - cy).atan2(x - cx);
    let tau = std::f64::consts::TAU;
    let start = angle(ax, ay);
    let ccw_to_end = (angle(bx, by) - start).rem_euclid(tau);
    let ccw_to_mid = (angle(mx, my) - start).rem_euclid(tau);
    let sweep = if ccw_to_mid < ccw_to_end { ccw_to_end } else { ccw_to_end - tau };
    Some(ArcGeom { center: (cx, cy), radius, start, sweep })
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Via {
    pub id: Id,
    pub pos: Point,
    pub diameter: Coord,
    pub drill: Coord,
    /// Copper span (through vias: top to bottom).
    pub from: LayerId,
    pub to: LayerId,
}

impl Via {
    /// Spans all copper layers (top to bottom).
    pub fn is_through(&self) -> bool {
        self.from.0.min(self.to.0) == LayerId::TOP_COPPER.0 && self.from.0.max(self.to.0) == LayerId::BOTTOM_COPPER.0
    }

    /// "Through", "Blind" (reaching the top or bottom) or "Buried".
    pub fn kind(&self) -> &'static str {
        let outer = |l: LayerId| l == LayerId::TOP_COPPER || l == LayerId::BOTTOM_COPPER;
        if self.is_through() {
            "Through"
        } else if outer(self.from) || outer(self.to) {
            "Blind"
        } else {
            "Buried"
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Zone {
    pub id: Id,
    pub layer: LayerId,
    /// Name of the net the zone connects to. Empty: a zone without a net (isolated
    /// copper), which keeps clearance to all copper.
    pub net: String,
    pub outline: Vec<Point>,
    /// Cutouts: areas inside the outline this zone leaves empty (other zones may fill
    /// them; a [`Keepout`] keeps every zone out).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub holes: Vec<Vec<Point>>,
    #[serde(default)]
    pub priority: u32,
    /// Overrides the design rule clearance when set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub clearance: Option<Coord>,
    /// How pads of the zone's net connect to it.
    #[serde(default, skip_serializing_if = "is_default")]
    pub connection: ZoneConnection,
    /// Override the design rules' thermal relief gap.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thermal_gap: Option<Coord>,
    /// Override the design rules' thermal spoke width.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thermal_spoke: Option<Coord>,
    /// Which fill pieces that reach no copper of the zone's net are removed.
    #[serde(default, skip_serializing_if = "is_default")]
    pub islands: IslandRemoval,
}

fn is_default<T: Default + PartialEq>(v: &T) -> bool {
    *v == T::default()
}

impl Zone {
    /// A thermal-relief zone with default settings.
    pub fn new(layer: LayerId, net: impl Into<String>, outline: Vec<Point>) -> Self {
        Zone {
            id: Id::new(),
            layer,
            net: net.into(),
            outline,
            holes: Vec::new(),
            priority: 0,
            clearance: None,
            connection: ZoneConnection::default(),
            thermal_gap: None,
            thermal_spoke: None,
            islands: IslandRemoval::default(),
        }
    }
}

/// How pads of a zone's own net connect to its fill.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ZoneConnection {
    /// A gap around the pad bridged by four spokes (easier to solder).
    #[default]
    Thermal,
    /// Thermal reliefs for through-hole pads, SMD pads flooded.
    ThermalThroughHole,
    /// Pads flooded.
    Solid,
    /// Pads keep the clearance (connect by tracks).
    None,
}

impl ZoneConnection {
    pub const ALL: [ZoneConnection; 4] =
        [ZoneConnection::Thermal, ZoneConnection::ThermalThroughHole, ZoneConnection::Solid, ZoneConnection::None];

    pub fn label(self) -> &'static str {
        match self {
            ZoneConnection::Thermal => "Thermal reliefs",
            ZoneConnection::ThermalThroughHole => "Thermal reliefs (through-hole only)",
            ZoneConnection::Solid => "Solid",
            ZoneConnection::None => "Not connected",
        }
    }
}

/// Which pieces of a zone's fill that reach no copper of its net are removed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum IslandRemoval {
    #[default]
    Always,
    Never,
    /// Those smaller than this area (nm²).
    BelowArea(i64),
}

/// An area where copper (or footprints) must not be (KiCad: rule area). Zone fills leave
/// it out (`no_pour`); the other flags are checked by DRC.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Keepout {
    pub id: Id,
    /// Copper layers it applies to; empty: all copper layers.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub layers: Vec<LayerId>,
    pub outline: Vec<Point>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub holes: Vec<Vec<Point>>,
    #[serde(default)]
    pub no_tracks: bool,
    #[serde(default)]
    pub no_vias: bool,
    #[serde(default)]
    pub no_pads: bool,
    #[serde(default)]
    pub no_pour: bool,
    #[serde(default)]
    pub no_footprints: bool,
}

impl Keepout {
    /// A keepout for tracks, vias, pads and zone fills (KiCad's defaults) on `layers`.
    pub fn new(layers: Vec<LayerId>, outline: Vec<Point>) -> Self {
        Keepout {
            id: Id::new(),
            layers,
            outline,
            holes: Vec::new(),
            no_tracks: true,
            no_vias: true,
            no_pads: true,
            no_pour: true,
            no_footprints: false,
        }
    }

    /// Does it apply to copper layer `layer`?
    pub fn on_layer(&self, layer: LayerId) -> bool {
        self.layers.is_empty() || self.layers.contains(&layer)
    }

    /// What it keeps out, for messages ("tracks, vias").
    pub fn rules_text(&self) -> String {
        let names: Vec<&str> = [
            (self.no_tracks, "tracks"),
            (self.no_vias, "vias"),
            (self.no_pads, "pads"),
            (self.no_pour, "zone fills"),
            (self.no_footprints, "footprints"),
        ]
        .into_iter()
        .filter_map(|(on, n)| on.then_some(n))
        .collect();
        if names.is_empty() { "nothing".into() } else { names.join(", ") }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct BoardGraphic {
    pub id: Id,
    pub layer: LayerId,
    pub shape: Shape,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct BoardText {
    pub id: Id,
    pub layer: LayerId,
    pub text: Text,
}

/// A placed footprint's own copy of a reference or value text ([`FootprintText::is_field`])
/// of its library footprint, so it can be moved, resized or hidden per footprint (as
/// KiCad does). Its text is in footprint coordinates: it moves with the footprint.
/// Created and removed together with the footprint.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct FootprintField {
    pub id: Id,
    pub footprint: Id,
    pub layer: FootprintLayer,
    /// On the footprint's other side.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub back: bool,
    /// `${REF}` or `${VALUE}`, placed relative to the footprint.
    pub text: Text,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub hidden: bool,
}

impl FootprintField {
    /// Fresh fields for footprint instance `footprint` from the field texts of `def`.
    pub fn for_footprint(footprint: Id, def: &crate::library::Footprint) -> Vec<FootprintField> {
        def.texts
            .iter()
            .filter(|t| t.is_field())
            .map(|t| FootprintField {
                id: Id::new(),
                footprint,
                layer: t.layer,
                back: t.back,
                text: t.text.clone(),
                hidden: false,
            })
            .collect()
    }

    /// Board layer for a footprint placed on `side`.
    pub fn board_layer(&self, side: Side) -> LayerId {
        LayerId::for_footprint(self.layer, if self.back { side.flipped() } else { side })
    }

    /// "Reference" or "Value".
    pub fn label(&self) -> &'static str {
        if self.text.text == "${VALUE}" { "Value" } else { "Reference" }
    }
}

has_id!(FootprintInstance, FootprintField, Track, Via, Zone, Keepout, BoardGraphic, BoardText);

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct DesignRules {
    pub clearance: Coord,
    pub track_width: Coord,
    pub via_diameter: Coord,
    pub via_drill: Coord,
    pub min_track_width: Coord,
    pub min_drill: Coord,
    /// Gap between a pad and the surrounding zone of its own net (thermal relief).
    #[serde(default = "default_thermal_gap")]
    pub thermal_gap: Coord,
    /// Width of the thermal relief spokes.
    #[serde(default = "default_thermal_spoke")]
    pub thermal_spoke: Coord,
    /// How far solder mask openings extend beyond their pads.
    #[serde(default = "default_mask_expansion")]
    pub mask_expansion: Coord,
    /// Added to SMD pads for their paste (negative shrinks, as in KiCad).
    #[serde(default)]
    pub paste_margin: Coord,
}

fn default_mask_expansion() -> Coord {
    mm(0.05)
}

fn default_thermal_gap() -> Coord {
    mm(0.5)
}

fn default_thermal_spoke() -> Coord {
    mm(0.5)
}

impl Default for DesignRules {
    fn default() -> Self {
        DesignRules {
            clearance: mm(0.2),
            track_width: mm(0.25),
            via_diameter: mm(0.6),
            via_drill: mm(0.3),
            min_track_width: mm(0.15),
            min_drill: mm(0.2),
            thermal_gap: default_thermal_gap(),
            thermal_spoke: default_thermal_spoke(),
            mask_expansion: default_mask_expansion(),
            paste_margin: 0,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Board {
    pub layers: Vec<BoardLayer>,
    #[serde(default)]
    pub rules: DesignRules,
    #[serde(default)]
    pub footprints: Store<FootprintInstance>,
    /// Reference and value texts of the footprints.
    #[serde(default)]
    pub fields: Store<FootprintField>,
    #[serde(default)]
    pub tracks: Store<Track>,
    #[serde(default)]
    pub vias: Store<Via>,
    #[serde(default)]
    pub zones: Store<Zone>,
    #[serde(default)]
    pub keepouts: Store<Keepout>,
    /// Board outline (on [`LayerId::OUTLINE`]) and other drawings.
    #[serde(default)]
    pub graphics: Store<BoardGraphic>,
    #[serde(default)]
    pub texts: Store<BoardText>,
}

impl Default for Board {
    fn default() -> Self {
        Board::with_copper_layers(2)
    }
}

impl Board {
    /// Empty board with `copper` copper layers (≥ 2) and the standard technical layers.
    pub fn with_copper_layers(copper: u16) -> Self {
        assert!(copper >= 2, "a board needs at least two copper layers");
        let mut layers =
            vec![BoardLayer { id: LayerId::TOP_COPPER, name: "Top".into(), function: LayerFunction::Copper }];
        for n in 1..copper - 1 {
            layers.push(BoardLayer {
                id: LayerId::inner(n),
                name: format!("Inner {n}"),
                function: LayerFunction::Copper,
            });
        }
        layers.push(BoardLayer { id: LayerId::BOTTOM_COPPER, name: "Bottom".into(), function: LayerFunction::Copper });
        use LayerFunction as F;
        for (id, name, function) in [
            (LayerId::TOP_SILK, "Top Silkscreen", F::Silkscreen),
            (LayerId::BOTTOM_SILK, "Bottom Silkscreen", F::Silkscreen),
            (LayerId::TOP_MASK, "Top Solder Mask", F::SolderMask),
            (LayerId::BOTTOM_MASK, "Bottom Solder Mask", F::SolderMask),
            (LayerId::TOP_PASTE, "Top Paste", F::SolderPaste),
            (LayerId::BOTTOM_PASTE, "Bottom Paste", F::SolderPaste),
            (LayerId::TOP_FAB, "Top Fabrication", F::Fabrication),
            (LayerId::BOTTOM_FAB, "Bottom Fabrication", F::Fabrication),
            (LayerId::TOP_COURTYARD, "Top Courtyard", F::Courtyard),
            (LayerId::BOTTOM_COURTYARD, "Bottom Courtyard", F::Courtyard),
            (LayerId::OUTLINE, "Board Outline", F::Outline),
        ] {
            layers.push(BoardLayer { id, name: name.into(), function });
        }
        Board {
            layers,
            rules: DesignRules::default(),
            footprints: Store::default(),
            fields: Store::default(),
            tracks: Store::default(),
            vias: Store::default(),
            zones: Store::default(),
            keepouts: Store::default(),
            graphics: Store::default(),
            texts: Store::default(),
        }
    }

    pub fn copper_layers(&self) -> impl Iterator<Item = &BoardLayer> {
        self.layers.iter().filter(|l| l.function == LayerFunction::Copper)
    }

    pub fn footprint_of(&self, component: Id) -> Option<&FootprintInstance> {
        self.footprints.iter().find(|f| f.component == component)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn four_layer_stack() {
        let b = Board::with_copper_layers(4);
        let copper: Vec<LayerId> = b.copper_layers().map(|l| l.id).collect();
        assert_eq!(copper, vec![LayerId(0), LayerId(1), LayerId(2), LayerId::BOTTOM_COPPER]);
    }
}
