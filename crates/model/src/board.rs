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

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Zone {
    pub id: Id,
    pub layer: LayerId,
    /// Name of the net the zone connects to.
    pub net: String,
    pub outline: Vec<Point>,
    #[serde(default)]
    pub priority: u32,
    /// Overrides the design rule clearance when set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub clearance: Option<Coord>,
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

has_id!(FootprintInstance, Track, Via, Zone, BoardGraphic, BoardText);

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
    #[serde(default)]
    pub tracks: Store<Track>,
    #[serde(default)]
    pub vias: Store<Via>,
    #[serde(default)]
    pub zones: Store<Zone>,
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
            tracks: Store::default(),
            vias: Store::default(),
            zones: Store::default(),
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
