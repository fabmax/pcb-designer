//! Schematic: components and their symbol placements, wires, buses, junctions, labels.
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

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Sheet {
    pub id: Id,
    pub name: String,
}

/// A part used in the design. Its footprint on the board follows automatically.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
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
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
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
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Wire {
    pub id: Id,
    pub sheet: Id,
    pub a: Point,
    pub b: Point,
}

/// Straight bus segment. A bus groups signals; it never connects anything electrically.
/// Touching bus wires form one bus, named by the bus labels on it (see
/// `editor_core::bus`). Signals leave it through [`BusEntry`]s and connect by label.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct BusWire {
    pub id: Id,
    pub sheet: Id,
    pub a: Point,
    pub b: Point,
}

/// Where a bus entry goes from the bus.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum EntryDir {
    #[default]
    UpRight,
    DownRight,
    DownLeft,
    UpLeft,
}

impl EntryDir {
    /// Clockwise order (as seen on screen), starting at [`EntryDir::UpRight`].
    pub const ALL: [EntryDir; 4] = [EntryDir::UpRight, EntryDir::DownRight, EntryDir::DownLeft, EntryDir::UpLeft];

    /// Offset from the bus end to the wire end.
    pub fn offset(self) -> Point {
        let s = BusEntry::SIZE;
        match self {
            EntryDir::UpRight => Point::new(s, s),
            EntryDir::DownRight => Point::new(s, -s),
            EntryDir::DownLeft => Point::new(-s, -s),
            EntryDir::UpLeft => Point::new(-s, s),
        }
    }

    /// The direction closest to `offset`.
    pub fn of_offset(offset: Point) -> EntryDir {
        match (offset.x >= 0, offset.y >= 0) {
            (true, true) => EntryDir::UpRight,
            (true, false) => EntryDir::DownRight,
            (false, false) => EntryDir::DownLeft,
            (false, true) => EntryDir::UpLeft,
        }
    }

    /// [`EntryDir::UpRight`] rotated by `angle` (like the rotate command turns entries).
    pub fn from_rotation(angle: Angle) -> EntryDir {
        EntryDir::of_offset(EntryDir::UpRight.offset().rotated(angle))
    }
}

/// A short diagonal from a bus to the wire of one of its signals.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct BusEntry {
    pub id: Id,
    pub sheet: Id,
    /// The end on the bus.
    pub pos: Point,
    #[serde(default)]
    pub dir: EntryDir,
}

impl BusEntry {
    /// Width and height of the diagonal: 100 mil.
    pub const SIZE: crate::units::Coord = 2_540_000;

    /// The end where the signal's wire attaches.
    pub fn wire_end(&self) -> Point {
        self.pos + self.dir.offset()
    }
}

/// Explicit connection dot where wires cross.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Junction {
    pub id: Id,
    pub sheet: Id,
    pub pos: Point,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum LabelKind {
    /// Connects to same-named labels.
    #[default]
    Net,
    /// Power port (GND, VCC, …), drawn as a power symbol.
    Power,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
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
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct SchText {
    pub id: Id,
    pub sheet: Id,
    pub text: Text,
}

has_id!(Sheet, Component, SymbolInstance, Wire, BusWire, BusEntry, Junction, NetLabel, SchText);

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Schematic {
    pub sheets: Store<Sheet>,
    #[serde(default)]
    pub components: Store<Component>,
    #[serde(default)]
    pub symbols: Store<SymbolInstance>,
    #[serde(default)]
    pub wires: Store<Wire>,
    #[serde(default, skip_serializing_if = "Store::is_empty")]
    pub bus_wires: Store<BusWire>,
    #[serde(default, skip_serializing_if = "Store::is_empty")]
    pub bus_entries: Store<BusEntry>,
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
