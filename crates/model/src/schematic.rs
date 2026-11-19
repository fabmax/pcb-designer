//! Schematic: components and their symbol placements, wires, buses, junctions, labels.
//!
//! Components are design-wide (one per reference designator). Each component's units are
//! placed as [`SymbolInstance`]s on a sheet. Graphical objects carry their sheet id, so
//! multi-sheet schematics only need more [`Sheet`] entries.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::checks::ErcSettings;
use crate::geom::{Angle, Point, Rect, Size, Text, Transform};
use crate::id::Id;
use crate::library::{has_id, natural_cmp};
use crate::object::ObjectId;
use crate::store::Store;
use crate::units::Coord;

/// A schematic sheet, as in KiCad: the root sheet, or a sub-sheet placed exactly once on
/// its parent as a sheet symbol (a rectangle; its pins come from the sub-sheet's
/// hierarchical labels). A sheet's path is the names from the root down: `/Power/Regulator`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Sheet {
    pub id: Id,
    /// Unique among the sheets of one parent.
    pub name: String,
    /// The sheet this one is placed on; `None` for the root sheet.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<Id>,
    /// The sheet symbol on the parent: its lower left corner and size. Unused for the root.
    #[serde(default)]
    pub pos: Point,
    #[serde(default)]
    pub size: Size,
    /// The drawing page: its lower left corner is at the origin.
    #[serde(default)]
    pub page: Page,
    /// The project's title block, kept on the root sheet (`None` on the others).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title_block: Option<TitleBlock>,
}

/// A sheet's drawing page: paper size and orientation. The frame (border, zone grid and
/// title block) is drawn around it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Page {
    #[serde(default)]
    pub size: PageSize,
    /// Portrait instead of landscape (custom sizes ignore it).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub portrait: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PageSize {
    /// No page and no frame.
    None,
    A5,
    #[default]
    A4,
    A3,
    A2,
    A1,
    A0,
    Letter,
    Legal,
    Tabloid,
    AnsiC,
    AnsiD,
    AnsiE,
    /// Width × height as given.
    Custom {
        w: Coord,
        h: Coord,
    },
}

impl PageSize {
    /// The named sizes, for pickers.
    pub const NAMED: [PageSize; 12] = [
        PageSize::A5,
        PageSize::A4,
        PageSize::A3,
        PageSize::A2,
        PageSize::A1,
        PageSize::A0,
        PageSize::Letter,
        PageSize::Legal,
        PageSize::Tabloid,
        PageSize::AnsiC,
        PageSize::AnsiD,
        PageSize::AnsiE,
    ];

    pub fn label(self) -> &'static str {
        match self {
            PageSize::None => "None",
            PageSize::A5 => "A5",
            PageSize::A4 => "A4",
            PageSize::A3 => "A3",
            PageSize::A2 => "A2",
            PageSize::A1 => "A1",
            PageSize::A0 => "A0",
            PageSize::Letter => "US Letter",
            PageSize::Legal => "US Legal",
            PageSize::Tabloid => "Tabloid",
            PageSize::AnsiC => "ANSI C",
            PageSize::AnsiD => "ANSI D",
            PageSize::AnsiE => "ANSI E",
            PageSize::Custom { .. } => "Custom",
        }
    }

    /// Landscape width and height (`None` for no page).
    pub fn landscape(self) -> Option<Size> {
        let mm = |w: f64, h: f64| Size { w: (w * 1e6).round() as Coord, h: (h * 1e6).round() as Coord };
        let inch = |w: f64, h: f64| mm(w * 25.4, h * 25.4);
        Some(match self {
            PageSize::None => return None,
            PageSize::A5 => mm(210.0, 148.0),
            PageSize::A4 => mm(297.0, 210.0),
            PageSize::A3 => mm(420.0, 297.0),
            PageSize::A2 => mm(594.0, 420.0),
            PageSize::A1 => mm(841.0, 594.0),
            PageSize::A0 => mm(1189.0, 841.0),
            PageSize::Letter => inch(11.0, 8.5),
            PageSize::Legal => inch(14.0, 8.5),
            PageSize::Tabloid => inch(17.0, 11.0),
            PageSize::AnsiC => inch(22.0, 17.0),
            PageSize::AnsiD => inch(34.0, 22.0),
            PageSize::AnsiE => inch(44.0, 34.0),
            PageSize::Custom { w, h } => Size { w, h },
        })
    }
}

impl Page {
    /// The page's width and height (`None`: no page).
    pub fn dimensions(&self) -> Option<Size> {
        let s = self.size.landscape()?;
        if self.portrait && !matches!(self.size, PageSize::Custom { .. }) {
            Some(Size { w: s.h, h: s.w })
        } else {
            Some(s)
        }
    }

    /// The page rectangle, from the origin (`None`: no page).
    pub fn rect(&self) -> Option<Rect> {
        self.dimensions().map(|s| Rect::from_points(Point::ZERO, Point::new(s.w, s.h)))
    }
}

/// The title block of the drawing frame: free text, as written.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct TitleBlock {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub title: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub date: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub revision: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub company: String,
    /// Free text above the company; each line break starts a line in the frame (the
    /// user keeps it short enough to fit).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub comment: String,
}

impl Sheet {
    /// Default size of a new sheet symbol (KiCad's is similar).
    pub const SIZE: Size = Size { w: 25_400_000, h: 20_320_000 };

    pub fn is_root(&self) -> bool {
        self.parent.is_none()
    }

    /// `size` in whole [`SHEET_PIN_PITCH`]es (the nearest, at least one): the pins sit a
    /// pitch apart from the top edge and on the side edges, so with the corner on the
    /// grid they all land on it.
    pub fn snap_size(size: Size) -> Size {
        let snap = |v: Coord| ((v as f64 / SHEET_PIN_PITCH as f64).round() as Coord).max(1) * SHEET_PIN_PITCH;
        Size { w: snap(size.w), h: snap(size.h) }
    }

    /// The sheet symbol's rectangle on the parent sheet, as stored (see [`Sheet::layout`]
    /// for the one drawn).
    pub fn rect(&self) -> Rect {
        Rect::from_points(self.pos, self.pos + Point::new(self.size.w, self.size.h))
    }

    /// The symbol and its pins for the sheet's hierarchical labels (`ports`, from
    /// [`Schematic::hier_ports`]): inputs on the left, the others on the right, each side
    /// in name order, one [`SHEET_PIN_PITCH`] apart from the top. The symbol grows
    /// downwards when they don't fit.
    pub fn layout(&self, ports: &[(&str, LabelShape)]) -> (Rect, Vec<SheetPin>) {
        let mut left: Vec<&(&str, LabelShape)> = ports.iter().filter(|p| p.1 == LabelShape::Input).collect();
        let mut right: Vec<&(&str, LabelShape)> = ports.iter().filter(|p| p.1 != LabelShape::Input).collect();
        left.sort_by(|a, b| natural_cmp(a.0, b.0));
        right.sort_by(|a, b| natural_cmp(a.0, b.0));
        let rows = left.len().max(right.len()) as Coord;
        let top = self.pos.y + self.size.h;
        let h = self.size.h.max((rows + 1) * SHEET_PIN_PITCH);
        let r = Rect::from_points(Point::new(self.pos.x, top - h), Point::new(self.pos.x + self.size.w, top));
        let mut pins = Vec::with_capacity(ports.len());
        for (side, right) in [(left, false), (right, true)] {
            for (i, &&(name, shape)) in side.iter().enumerate() {
                let x = if right { r.max.x } else { r.min.x };
                let pos = Point::new(x, top - (i as Coord + 1) * SHEET_PIN_PITCH);
                pins.push(SheetPin { name: name.to_string(), shape, pos, right });
            }
        }
        (r, pins)
    }
}

/// Distance between sheet pins, and from the symbol's top edge to the first one.
pub const SHEET_PIN_PITCH: Coord = 2_540_000;

/// A pin of a sheet symbol: where a hierarchical label of the sheet connects on the
/// parent sheet. Derived, not stored ([`Sheet::layout`]).
#[derive(Clone, Debug, PartialEq)]
pub struct SheetPin {
    pub name: String,
    pub shape: LabelShape,
    pub pos: Point,
    /// On the right edge (else the left).
    pub right: bool,
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

/// No-connect flag (an X) on a pin end: the pin is meant to stay unconnected.
/// Documentation only; connectivity ignores it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct NoConnect {
    pub id: Id,
    pub sheet: Id,
    pub pos: Point,
}

/// Power flag (a pennant on a wire or pin end): its net is powered from somewhere the
/// schematic doesn't show (a connector, a supply), so power inputs and inputs on it count
/// as driven. KiCad's PWR_FLAG. Names no net and takes no part in the pin conflict matrix.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct PowerFlag {
    pub id: Id,
    pub sheet: Id,
    pub pos: Point,
    /// Direction the flag points from `pos` (0°: up).
    #[serde(default)]
    pub rotation: Angle,
}

/// What a label connects to, as in KiCad.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum LabelKind {
    /// Connects to same-named labels on the same sheet. The net is named by the sheet's
    /// path: `/NAME` on the root sheet, `/Sub/NAME` below it.
    #[default]
    Local,
    /// Connects to same-named global labels on every sheet; the net keeps the name.
    Global,
    /// Connects to the same-named pin of this sheet's symbol on the parent sheet.
    Hierarchical,
    /// Power port (GND, VCC, …), drawn as a power symbol; global.
    Power,
}

impl LabelKind {
    pub const ALL: [LabelKind; 4] = [LabelKind::Local, LabelKind::Global, LabelKind::Hierarchical, LabelKind::Power];

    pub fn label(self) -> &'static str {
        match self {
            LabelKind::Local => "Local",
            LabelKind::Global => "Global",
            LabelKind::Hierarchical => "Hierarchical",
            LabelKind::Power => "Power",
        }
    }

    /// Same-named labels of this kind connect across sheets.
    pub fn is_global(self) -> bool {
        matches!(self, LabelKind::Global | LabelKind::Power)
    }

    /// Global and hierarchical labels are drawn with a [`LabelShape`].
    pub fn has_shape(self) -> bool {
        matches!(self, LabelKind::Global | LabelKind::Hierarchical)
    }
}

/// The outline of a global or hierarchical label (and of a sheet pin): the signal's
/// direction, drawn as in KiCad. Documentation only; connectivity ignores it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum LabelShape {
    #[default]
    Input,
    Output,
    Bidirectional,
    TriState,
    Passive,
}

impl LabelShape {
    pub const ALL: [LabelShape; 5] =
        [LabelShape::Input, LabelShape::Output, LabelShape::Bidirectional, LabelShape::TriState, LabelShape::Passive];

    pub fn label(self) -> &'static str {
        match self {
            LabelShape::Input => "Input",
            LabelShape::Output => "Output",
            LabelShape::Bidirectional => "Bidirectional",
            LabelShape::TriState => "Tri-state",
            LabelShape::Passive => "Passive",
        }
    }

    /// The shape seen from the other side: a sheet pin for an input label is an output
    /// of the parent sheet's circuit, drawn pointing into the sheet.
    pub fn reversed(self) -> LabelShape {
        match self {
            LabelShape::Input => LabelShape::Output,
            LabelShape::Output => LabelShape::Input,
            other => other,
        }
    }

    /// Which ends of the outline are pointed: (at the connection point, away from it).
    pub fn points(self) -> (bool, bool) {
        match self {
            LabelShape::Input => (true, false),
            LabelShape::Output => (false, true),
            LabelShape::Bidirectional | LabelShape::TriState => (true, true),
            LabelShape::Passive => (false, false),
        }
    }
}

/// The glyph of a power port, modelled on KiCad's power symbols. Documentation only;
/// the net comes from the name.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PowerStyle {
    /// A bar across the wire (+5V, VCC).
    Bar,
    /// An arrow head.
    Arrow,
    /// A small circle.
    Circle,
    /// Three bars getting shorter (GND).
    Ground,
    /// An open triangle (signal ground).
    SignalGround,
    /// A rake (chassis ground).
    Chassis,
    /// Ground bars in a circle (protective earth).
    ProtectiveEarth,
}

impl PowerStyle {
    pub const ALL: [PowerStyle; 7] = [
        PowerStyle::Bar,
        PowerStyle::Arrow,
        PowerStyle::Circle,
        PowerStyle::Ground,
        PowerStyle::SignalGround,
        PowerStyle::Chassis,
        PowerStyle::ProtectiveEarth,
    ];

    pub fn label(self) -> &'static str {
        match self {
            PowerStyle::Bar => "Bar",
            PowerStyle::Arrow => "Arrow",
            PowerStyle::Circle => "Circle",
            PowerStyle::Ground => "Ground",
            PowerStyle::SignalGround => "Signal ground",
            PowerStyle::Chassis => "Chassis",
            PowerStyle::ProtectiveEarth => "Protective earth",
        }
    }

    /// The glyph a power port named `name` gets unless one is chosen: ground for names
    /// containing GND or EARTH (GND, AGND, GNDD, …), chassis for CHASSIS, protective earth
    /// for PE, else a bar.
    pub fn for_name(name: &str) -> PowerStyle {
        let n = name.to_uppercase();
        if n == "PE" {
            PowerStyle::ProtectiveEarth
        } else if n.contains("CHASSIS") {
            PowerStyle::Chassis
        } else if n.contains("GND") || n.contains("EARTH") {
            PowerStyle::Ground
        } else {
            PowerStyle::Bar
        }
    }

    /// Ground glyphs hang below the connection point, the others stand above it.
    pub fn points_down(self) -> bool {
        matches!(
            self,
            PowerStyle::Ground | PowerStyle::SignalGround | PowerStyle::Chassis | PowerStyle::ProtectiveEarth
        )
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct NetLabel {
    pub id: Id,
    pub sheet: Id,
    pub pos: Point,
    pub name: String,
    #[serde(default)]
    pub kind: LabelKind,
    /// Drawn for global and hierarchical labels only.
    #[serde(default)]
    pub shape: LabelShape,
    /// The glyph of a power port; without one it's the one its name suggests
    /// ([`PowerStyle::for_name`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub style: Option<PowerStyle>,
    #[serde(default)]
    pub rotation: Angle,
}

impl NetLabel {
    /// The glyph drawn for this power port.
    pub fn power_style(&self) -> PowerStyle {
        self.style.unwrap_or_else(|| PowerStyle::for_name(&self.name))
    }
}

/// Free text on a sheet.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct SchText {
    pub id: Id,
    pub sheet: Id,
    pub text: Text,
}

/// A placed symbol's own copy of a reference or value text (exactly `${REF}` or
/// `${VALUE}`) of its library symbol, so it can be moved, turned or hidden per symbol.
/// Its text is in symbol coordinates: it moves with the symbol. Created and removed
/// together with the symbol. Drawn upright (never upside down).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct SymbolField {
    pub id: Id,
    pub symbol: Id,
    pub text: Text,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub hidden: bool,
}

impl SymbolField {
    /// Fresh fields for symbol instance `symbol` from the field texts of `def`.
    pub fn for_symbol(symbol: Id, def: &crate::library::Symbol) -> Vec<SymbolField> {
        def.texts
            .iter()
            .filter(|t| crate::geom::is_field_text(&t.text))
            .map(|t| SymbolField { id: Id::new(), symbol, text: t.clone(), hidden: false })
            .collect()
    }

    /// "Reference" or "Value".
    pub fn label(&self) -> &'static str {
        if self.text.text == "${VALUE}" { "Value" } else { "Reference" }
    }
}

has_id!(
    Sheet,
    Component,
    SymbolInstance,
    Wire,
    BusWire,
    BusEntry,
    Junction,
    NoConnect,
    PowerFlag,
    NetLabel,
    SchText,
    SymbolField
);

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
    #[serde(default, skip_serializing_if = "Store::is_empty")]
    pub no_connects: Store<NoConnect>,
    #[serde(default, skip_serializing_if = "Store::is_empty")]
    pub power_flags: Store<PowerFlag>,
    #[serde(default)]
    pub labels: Store<NetLabel>,
    #[serde(default)]
    pub texts: Store<SchText>,
    /// Reference and value texts of placed symbols.
    #[serde(default)]
    pub fields: Store<SymbolField>,
    /// Severities, pin conflict matrix and exclusions of the electrical rules check.
    #[serde(default, skip_serializing_if = "ErcSettings::is_default")]
    pub erc: ErcSettings,
}

impl Schematic {
    /// A schematic with just the root sheet.
    pub fn new() -> Self {
        let mut s = Schematic::default();
        s.sheets.insert(Sheet {
            id: Id::new(),
            name: "Schematic".into(),
            parent: None,
            pos: Point::ZERO,
            size: Size::default(),
            page: Page::default(),
            title_block: Some(TitleBlock::default()),
        });
        s
    }

    /// The project's title block (on the root sheet).
    pub fn title_block(&self) -> TitleBlock {
        self.root_sheet().and_then(|r| self.sheets.get(r)).and_then(|s| s.title_block.clone()).unwrap_or_default()
    }

    /// Sheets in page order: the root first, then depth first with the sub-sheets of each
    /// sheet sorted by name (the Project panel's order).
    pub fn page_order(&self) -> Vec<Id> {
        let mut out = Vec::new();
        let mut stack: Vec<Id> = self.root_sheet().into_iter().collect();
        while let Some(id) = stack.pop() {
            if out.len() > self.sheets.len() {
                break; // a cycle (invalid project); don't hang
            }
            out.push(id);
            let mut children: Vec<&Sheet> = self.children(id).collect();
            children.sort_by(|a, b| natural_cmp(&a.name, &b.name));
            stack.extend(children.iter().rev().map(|s| s.id));
        }
        out
    }

    /// The root sheet (the one without a parent).
    pub fn root_sheet(&self) -> Option<Id> {
        self.sheets.iter().find(|s| s.is_root()).map(|s| s.id)
    }

    /// The sheets placed on `sheet`, in insertion order.
    pub fn children(&self, sheet: Id) -> impl Iterator<Item = &Sheet> {
        self.sheets.iter().filter(move |s| s.parent == Some(sheet))
    }

    /// `sheet` and every sheet below it, parents before their children.
    pub fn sheet_and_descendants(&self, sheet: Id) -> Vec<Id> {
        let mut out = vec![sheet];
        let mut i = 0;
        while i < out.len() {
            let parent = out[i];
            out.extend(self.children(parent).map(|s| s.id));
            i += 1;
        }
        out
    }

    /// The sheet's path in the hierarchy, as a prefix for net names: empty for the root,
    /// `/Power/Regulator` below it (names of the sheets from the root down).
    pub fn sheet_path(&self, sheet: Id) -> String {
        let mut names = Vec::new();
        let mut at = self.sheets.get(sheet);
        while let Some(s) = at {
            let Some(parent) = s.parent else { break };
            names.push(s.name.as_str());
            if names.len() > self.sheets.len() {
                break; // a cycle (invalid project); don't hang
            }
            at = self.sheets.get(parent);
        }
        names.iter().rev().map(|n| format!("/{n}")).collect()
    }

    /// The hierarchical labels of `sheet` as ports of its symbol: one per name, with the
    /// first label's shape.
    pub fn hier_ports(&self, sheet: Id) -> Vec<(&str, LabelShape)> {
        let mut out: Vec<(&str, LabelShape)> = Vec::new();
        for l in self.labels.iter().filter(|l| l.sheet == sheet && l.kind == LabelKind::Hierarchical) {
            if !out.iter().any(|p| p.0 == l.name) {
                out.push((l.name.as_str(), l.shape));
            }
        }
        out
    }

    /// [`Sheet::layout`] of a sheet's symbol with its ports.
    pub fn sheet_symbol(&self, sheet: Id) -> Option<(Rect, Vec<SheetPin>)> {
        let s = self.sheets.get(sheet)?;
        Some(s.layout(&self.hier_ports(sheet)))
    }

    /// What is wrong with naming a sheet below `parent` `name` (`sheet`: the sheet being
    /// renamed, which may keep its name): names are not empty, have no surrounding spaces
    /// and no `/`, and are unique among their siblings.
    pub fn sheet_name_issue(&self, parent: Option<Id>, name: &str, sheet: Option<Id>) -> Option<String> {
        if name.trim().is_empty() || name.trim() != name || name.contains('/') {
            return Some(format!("sheet name \"{name}\": not empty, no surrounding spaces, no /"));
        }
        self.sheets
            .iter()
            .any(|o| Some(o.id) != sheet && o.parent == parent && o.name == name)
            .then(|| format!("there is a sheet named {name} there already"))
    }

    /// The sheet's path for display: `/` for the root, else [`sheet_path`](Self::sheet_path).
    pub fn display_path(&self, sheet: Id) -> String {
        let path = self.sheet_path(sheet);
        if path.is_empty() { "/".into() } else { path }
    }

    /// A sheet by id, path (`/Power/Regulator`, `/` for the root) or name (if unique).
    pub fn find_sheet(&self, spec: &str) -> Option<Id> {
        let spec = spec.trim();
        if let Ok(id) = spec.parse::<Id>()
            && self.sheets.contains(id)
        {
            return Some(id);
        }
        if spec.starts_with('/') {
            let wanted = spec.trim_end_matches('/');
            return self.sheets.iter().find(|s| self.sheet_path(s.id) == wanted).map(|s| s.id);
        }
        let mut named = self.sheets.iter().filter(|s| s.name == spec);
        match (named.next(), named.next()) {
            (Some(s), None) => Some(s.id),
            _ => None,
        }
    }

    /// The sheet an object is drawn on: its `sheet`, or for a sheet (its symbol) the
    /// parent. `None` for components and for the root sheet.
    pub fn sheet_of(&self, id: ObjectId) -> Option<Id> {
        match id {
            ObjectId::Sheet(s) => self.sheets.get(s).and_then(|s| s.parent),
            ObjectId::Symbol(o) => self.symbols.get(o).map(|o| o.sheet),
            ObjectId::Wire(o) => self.wires.get(o).map(|o| o.sheet),
            ObjectId::BusWire(o) => self.bus_wires.get(o).map(|o| o.sheet),
            ObjectId::BusEntry(o) => self.bus_entries.get(o).map(|o| o.sheet),
            ObjectId::Junction(o) => self.junctions.get(o).map(|o| o.sheet),
            ObjectId::NoConnect(o) => self.no_connects.get(o).map(|o| o.sheet),
            ObjectId::PowerFlag(o) => self.power_flags.get(o).map(|o| o.sheet),
            ObjectId::Label(o) => self.labels.get(o).map(|o| o.sheet),
            ObjectId::SchText(o) => self.texts.get(o).map(|o| o.sheet),
            ObjectId::SymbolField(o) => self.fields.get(o).and_then(|f| self.symbols.get(f.symbol)).map(|s| s.sheet),
            _ => None,
        }
    }

    /// Everything drawn on `sheet` (sheet symbols included).
    pub fn objects_on(&self, sheet: Id) -> Vec<ObjectId> {
        let on = |s: Id| s == sheet;
        self.children(sheet)
            .map(|s| ObjectId::Sheet(s.id))
            .chain(self.symbols.iter().filter(|o| on(o.sheet)).map(|o| ObjectId::Symbol(o.id)))
            .chain(self.wires.iter().filter(|o| on(o.sheet)).map(|o| ObjectId::Wire(o.id)))
            .chain(self.bus_wires.iter().filter(|o| on(o.sheet)).map(|o| ObjectId::BusWire(o.id)))
            .chain(self.bus_entries.iter().filter(|o| on(o.sheet)).map(|o| ObjectId::BusEntry(o.id)))
            .chain(self.junctions.iter().filter(|o| on(o.sheet)).map(|o| ObjectId::Junction(o.id)))
            .chain(self.no_connects.iter().filter(|o| on(o.sheet)).map(|o| ObjectId::NoConnect(o.id)))
            .chain(self.power_flags.iter().filter(|o| on(o.sheet)).map(|o| ObjectId::PowerFlag(o.id)))
            .chain(self.labels.iter().filter(|o| on(o.sheet)).map(|o| ObjectId::Label(o.id)))
            .chain(self.texts.iter().filter(|o| on(o.sheet)).map(|o| ObjectId::SchText(o.id)))
            .chain(
                self.fields
                    .iter()
                    .filter(|f| self.symbols.get(f.symbol).is_some_and(|s| on(s.sheet)))
                    .map(|o| ObjectId::SymbolField(o.id)),
            )
            .collect()
    }

    /// Problems with the sheet tree: exactly one root, existing parents, no cycles,
    /// non-empty names unique among siblings.
    pub fn sheet_issues(&self) -> Vec<String> {
        let mut issues = Vec::new();
        let roots = self.sheets.iter().filter(|s| s.is_root()).count();
        if roots != 1 {
            issues.push(format!("{roots} root sheets (exactly one expected)"));
        }
        for s in self.sheets.iter() {
            if s.name.trim().is_empty() {
                issues.push(format!("sheet {:?}: empty name", s.id));
            }
            if let Some(p) = s.parent {
                if !self.sheets.contains(p) {
                    issues.push(format!("sheet {}: missing parent", s.name));
                } else if !self.sheet_and_descendants(self.root_sheet().unwrap_or(p)).contains(&s.id) {
                    issues.push(format!("sheet {}: not below the root (cycle)", s.name));
                }
            }
            if let PageSize::Custom { w, h } = s.page.size
                && (w <= 0 || h <= 0)
            {
                issues.push(format!("sheet {}: the page needs a positive width and height", s.name));
            }
            if self.sheets.iter().any(|o| o.id != s.id && o.parent == s.parent && o.name == s.name) {
                issues.push(format!("sheet name {} is used twice on one sheet", s.name));
            }
        }
        issues.dedup();
        issues
    }

    pub fn component_by_refdes(&self, refdes: &str) -> Option<&Component> {
        self.components.iter().find(|c| c.refdes == refdes)
    }

    pub fn symbols_of(&self, component: Id) -> impl Iterator<Item = &SymbolInstance> {
        self.symbols.iter().filter(move |s| s.component == component)
    }
}
