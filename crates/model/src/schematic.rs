//! Schematic: components and their symbol placements, wires, junctions, labels.
//!
//! Components are design-wide (one per reference designator). Each component's units are
//! placed as [`SymbolInstance`]s on a sheet. Graphical objects carry their sheet id, so
//! multi-sheet schematics only need more [`Sheet`] entries.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::geom::{Angle, Point, Text, Transform};
use crate::id::Id;
use crate::library::has_id;
use crate::store::Store;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Sheet {
    pub id: Id,
    pub name: String,
}

/// A part used in the design. Its footprint on the board follows automatically.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Component {
    pub id: Id,
    /// Part in the project's embedded library.
    pub part: Id,
    /// Footprint variant of the part.
    pub variant: Id,
    pub refdes: String,
    #[serde(default)]
    pub value: String,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub properties: BTreeMap<String, String>,
}

/// Placement of one unit of a component on a sheet.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SymbolInstance {
    pub id: Id,
    pub sheet: Id,
    pub component: Id,
    #[serde(default)]
    pub unit: u32,
    pub pos: Point,
    #[serde(default)]
    pub rotation: Angle,
    #[serde(default)]
    pub mirror: bool,
}

impl SymbolInstance {
    pub fn transform(&self) -> Transform {
        Transform::new(self.pos, self.rotation, self.mirror)
    }
}

/// Straight wire segment.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Wire {
    pub id: Id,
    pub sheet: Id,
    pub a: Point,
    pub b: Point,
}

/// Explicit connection dot where wires cross.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Junction {
    pub id: Id,
    pub sheet: Id,
    pub pos: Point,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LabelKind {
    /// Connects to same-named labels.
    #[default]
    Net,
    /// Power port (GND, VCC, …), drawn as a power symbol.
    Power,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct NetLabel {
    pub id: Id,
    pub sheet: Id,
    pub pos: Point,
    pub name: String,
    #[serde(default)]
    pub kind: LabelKind,
    #[serde(default)]
    pub rotation: Angle,
}

/// Free text on a sheet.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SchText {
    pub id: Id,
    pub sheet: Id,
    pub text: Text,
}

has_id!(Sheet, Component, SymbolInstance, Wire, Junction, NetLabel, SchText);

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Schematic {
    pub sheets: Store<Sheet>,
    #[serde(default)]
    pub components: Store<Component>,
    #[serde(default)]
    pub symbols: Store<SymbolInstance>,
    #[serde(default)]
    pub wires: Store<Wire>,
    #[serde(default)]
    pub junctions: Store<Junction>,
    #[serde(default)]
    pub labels: Store<NetLabel>,
    #[serde(default)]
    pub texts: Store<SchText>,
}

impl Schematic {
    /// A schematic with a single sheet.
    pub fn new() -> Self {
        let mut s = Schematic::default();
        s.sheets.insert(Sheet { id: Id::new(), name: "Sheet 1".into() });
        s
    }

    pub fn first_sheet(&self) -> Option<Id> {
        self.sheets.ids().next()
    }

    pub fn component_by_refdes(&self, refdes: &str) -> Option<&Component> {
        self.components.iter().find(|c| c.refdes == refdes)
    }

    pub fn symbols_of(&self, component: Id) -> impl Iterator<Item = &SymbolInstance> {
        self.symbols.iter().filter(move |s| s.component == component)
    }
}
