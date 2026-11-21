//! Serializable editing commands. The GUI, scripts and MCP clients all go through these.

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

use model::*;

use crate::CommandError;
use crate::clip::{Clip, ClipContent};
use crate::transaction::{Setup, Transaction};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(tag = "command", rename_all = "snake_case")]
pub enum Command {
    /// Copy parts (each with exactly the symbols and footprints it references) into the
    /// project. A part's `source` records the library it came from.
    ImportParts {
        parts: Vec<PartBundle>,
    },
    /// Replace the project's copies of parts (with their symbols and footprints) by these
    /// library versions. Components keep their footprint variant if it still exists
    /// (matched by id, then by name), otherwise they get the first variant.
    UpdateParts {
        parts: Vec<PartBundle>,
    },
    /// Add a sheet below `parent` (default: the root sheet), placed there as a sheet
    /// symbol with its lower left corner at `pos` (default size 25.4 × 20.32 mm).
    AddSheet {
        #[serde(default)]
        parent: Option<Id>,
        name: String,
        pos: Point,
        #[serde(default)]
        size: Option<Size>,
    },
    /// Place a part from the project library: creates a component and its symbol. The
    /// footprint appears on the board automatically. Not for board parts.
    PlacePart {
        part: Id,
        #[serde(default)]
        variant: Option<Id>,
        #[serde(default)]
        sheet: Option<Id>,
        pos: Point,
        #[serde(default)]
        rotation: Angle,
        #[serde(default)]
        mirror: bool,
        #[serde(default)]
        refdes: Option<String>,
        #[serde(default)]
        value: Option<String>,
    },
    /// Place a board part (a part without symbol: mounting hole, logo) on the board:
    /// creates its component and footprint. Board parts live on the board only.
    PlaceBoardPart {
        part: Id,
        #[serde(default)]
        variant: Option<Id>,
        pos: Point,
        #[serde(default)]
        rotation: Angle,
        #[serde(default)]
        side: Side,
        #[serde(default)]
        refdes: Option<String>,
        #[serde(default)]
        value: Option<String>,
    },
    /// Place another unit of a multi-unit component (e.g. the second gate of a 74HC00).
    PlaceUnit {
        component: Id,
        unit: u32,
        #[serde(default)]
        sheet: Option<Id>,
        pos: Point,
        #[serde(default)]
        rotation: Angle,
        #[serde(default)]
        mirror: bool,
    },
    /// Polyline of wire segments.
    AddWire {
        #[serde(default)]
        sheet: Option<Id>,
        points: Vec<Point>,
    },
    /// Polyline of bus segments. Name the bus with a label whose name uses bus syntax
    /// (`D[0..7]`, `{SDA SCL}`, `I2C{SDA SCL}`) placed on it.
    AddBus {
        #[serde(default)]
        sheet: Option<Id>,
        points: Vec<Point>,
    },
    /// Bus entry with its bus end at `pos`; a signal's wire attaches at the other end,
    /// 2.54 mm away diagonally in direction `dir`.
    AddBusEntry {
        #[serde(default)]
        sheet: Option<Id>,
        pos: Point,
        #[serde(default)]
        dir: EntryDir,
    },
    AddJunction {
        #[serde(default)]
        sheet: Option<Id>,
        pos: Point,
    },
    /// A no-connect flag (X) on a pin end: the pin stays unconnected on purpose. It moves
    /// and is deleted with its symbol.
    AddNoConnect {
        #[serde(default)]
        sheet: Option<Id>,
        pos: Point,
    },
    /// A power flag on a wire or pin end: its net is powered from elsewhere (KiCad's
    /// PWR_FLAG). It moves and is deleted with the symbol whose pin end it is on.
    AddPowerFlag {
        #[serde(default)]
        sheet: Option<Id>,
        pos: Point,
        /// Direction the flag points (0°: up).
        #[serde(default)]
        rotation: Angle,
    },
    AddLabel {
        #[serde(default)]
        sheet: Option<Id>,
        pos: Point,
        name: String,
        #[serde(default)]
        kind: LabelKind,
        /// Drawn for global and hierarchical labels.
        #[serde(default)]
        shape: LabelShape,
        /// Glyph of a power port; default: the one its name suggests (GND → ground).
        #[serde(default)]
        style: Option<PowerStyle>,
        #[serde(default)]
        rotation: Angle,
    },
    /// A free text: on a board `layer` (silkscreen, copper without a net, fab, …), or
    /// without a layer on a schematic `sheet`. Size (cap height) defaults to 1.27 mm on the
    /// schematic and 1 mm on the board; `bold` defaults to true on the board, false on the
    /// schematic. Newlines break lines.
    AddText {
        #[serde(default)]
        sheet: Option<Id>,
        #[serde(default)]
        layer: Option<LayerId>,
        pos: Point,
        text: String,
        #[serde(default)]
        size: Option<Coord>,
        #[serde(default)]
        bold: Option<bool>,
        #[serde(default)]
        rotation: Angle,
        #[serde(default)]
        halign: HAlign,
        #[serde(default)]
        valign: VAlign,
    },
    SetComponent {
        id: Id,
        #[serde(default)]
        refdes: Option<String>,
        #[serde(default)]
        value: Option<String>,
        #[serde(default)]
        variant: Option<Id>,
    },
    /// Polyline of track segments.
    AddTrack {
        layer: LayerId,
        points: Vec<Point>,
        #[serde(default)]
        width: Option<Coord>,
        /// For arcs: a point on the arc of segment `i` (from `points[i]` to `points[i+1]`);
        /// missing or `None`: straight.
        #[serde(default)]
        mids: Vec<Option<Point>>,
    },
    /// Cut `outline` out of a zone or keepout (`id`). Inside it, it becomes a hole (merged
    /// with holes it overlaps); across the edge, it trims the outline. Refused if it
    /// misses the area, would remove all of it or split it into pieces.
    AddCutout {
        id: ObjectId,
        outline: Vec<Point>,
    },
    /// Round the corners between the given straight tracks (where two of them end at the
    /// same point with nothing else there) with tangent arcs of `radius`. Corners whose
    /// tracks are too short for the radius stay; fails if none could be rounded.
    RoundCorners {
        tracks: Vec<Id>,
        radius: Coord,
    },
    /// Number of copper layers (2 or more, even). Inner layers are added or removed at
    /// the bottom of the inner stack; removing a layer that has copper on it fails.
    SetCopperLayers {
        count: u16,
    },
    /// Replace the project's title block (shown in the frame of every sheet; kept on the
    /// root sheet).
    SetTitleBlock {
        title_block: TitleBlock,
    },
    /// Replace the design rules (track width, via size, clearance, …).
    SetDesignRules {
        rules: DesignRules,
    },
    /// Replace the board's stack-up (thickness, copper, finish, colours).
    SetStackup {
        stackup: Stackup,
    },
    /// Replace the electrical rules check's settings (severities, pin conflict matrix,
    /// exclusions).
    SetErcSettings {
        erc: ErcSettings,
    },
    /// Replace the design rules check's settings (severities, exclusions).
    SetDrcSettings {
        drc: DrcSettings,
    },
    /// Replace the simulation setup (analyses and their probes, ground, supplies).
    SetSimulation {
        simulation: Simulation,
    },
    AddVia {
        pos: Point,
        #[serde(default)]
        diameter: Option<Coord>,
        #[serde(default)]
        drill: Option<Coord>,
        /// The two copper layers a blind or buried via joins (and all between);
        /// `None`: a through via.
        #[serde(default)]
        span: Option<[LayerId; 2]>,
    },
    /// Move objects. With `keep_connected`, what is attached stays attached. Schematic
    /// wires attached to moved pins, wire ends, labels and junctions follow: horizontal and
    /// vertical wires stay so by shifting sideways (stretching the wires at their other
    /// end, or adding a short wire there if needed); other wires stretch. Ends of moved
    /// wires on things that stay put are held there (a short wire joins them). On the
    /// board, tracks ending on moved pads and vias move along, and moved tracks stay on
    /// the copper they ended on (sliding along the next track, or joined by a new one).
    Move {
        ids: Vec<ObjectId>,
        delta: Point,
        #[serde(default)]
        keep_connected: bool,
    },
    Rotate {
        ids: Vec<ObjectId>,
        center: Point,
        angle: Angle,
        /// Attached schematic wires and tracks stretch to stay connected.
        #[serde(default)]
        keep_connected: bool,
    },
    /// Mirror schematic objects across the vertical line `x = axis_x`.
    Mirror {
        ids: Vec<ObjectId>,
        axis_x: Coord,
        /// Attached schematic wires stretch to stay connected.
        #[serde(default)]
        keep_connected: bool,
    },
    /// Move footprints to the other board side, mirrored across `x = axis_x`. With
    /// `keep_connected`, tracks ending on their pads stretch to stay on them.
    FlipSide {
        ids: Vec<ObjectId>,
        axis_x: Coord,
        #[serde(default)]
        keep_connected: bool,
    },
    Delete {
        ids: Vec<ObjectId>,
    },
    /// Insert raw objects (references are validated).
    Insert {
        objects: Vec<Object>,
    },
    /// Paste a clip (see [`crate::clip`]) of schematic objects onto `sheet` (default: the
    /// root sheet), or of board objects, moved so its anchor lands on `at`. Objects get
    /// new ids. Components keep their reference designator if it is free, else get the
    /// next one; parts missing from the project are imported from the clip.
    Paste {
        clip: Clip,
        #[serde(default)]
        sheet: Option<Id>,
        at: Point,
    },
    /// Move schematic objects to another sheet, keeping their ids (components keep their
    /// references, footprints and routing). Reference and value texts and the no-connect
    /// flags on moved symbols' pins go along; wires not listed stay. A sheet symbol
    /// re-parents its sheet (with its sub-sheets) below `sheet`, renamed with a number on a
    /// name clash; a sheet can't move below itself. Positions are kept.
    MoveToSheet {
        ids: Vec<ObjectId>,
        sheet: Id,
    },
    /// Copy and paste in one step, moved by `delta`: schematic objects (sheet symbols with
    /// their sub-sheets) on their sheet, board objects (board parts included) on the board.
    /// Copies of components get the next free references.
    Duplicate {
        ids: Vec<ObjectId>,
        delta: Point,
    },
    /// Replace raw objects by id (references are validated).
    Update {
        objects: Vec<Object>,
    },
    /// Add a library file to the project's External Libraries (an absolute path).
    AddLibrary {
        path: std::path::PathBuf,
    },
    /// Remove a library file from the project's External Libraries.
    RemoveLibrary {
        path: std::path::PathBuf,
    },
    /// Point a library entry at another file, keeping its place in the list (a moved
    /// file was located, or the library was saved under a new name).
    RelinkLibrary {
        from: std::path::PathBuf,
        to: std::path::PathBuf,
    },
}

impl Command {
    /// Short human readable description, used as undo label.
    pub fn label(&self) -> String {
        match self {
            Command::ImportParts { parts, .. } => format!("Import {} part(s)", parts.len()),
            Command::UpdateParts { parts, .. } => format!("Update {} part(s) from library", parts.len()),
            Command::PlacePart { .. } => "Place part".into(),
            Command::PlaceBoardPart { .. } => "Place board part".into(),
            Command::PlaceUnit { .. } => "Place unit".into(),
            Command::AddWire { .. } => "Add wire".into(),
            Command::AddBus { .. } => "Add bus".into(),
            Command::AddBusEntry { .. } => "Add bus entry".into(),
            Command::AddJunction { .. } => "Add junction".into(),
            Command::AddNoConnect { .. } => "Add no-connect flag".into(),
            Command::AddPowerFlag { .. } => "Add power flag".into(),
            Command::AddLabel { name, .. } => format!("Add label {name}"),
            Command::AddText { .. } => "Add text".into(),
            Command::AddSheet { name, .. } => format!("Add sheet {name}"),
            Command::SetComponent { .. } => "Edit component".into(),
            Command::AddTrack { .. } => "Add track".into(),
            Command::AddVia { .. } => "Add via".into(),
            Command::RoundCorners { .. } => "Round corners".into(),
            Command::AddCutout { .. } => "Add cutout".into(),
            Command::SetCopperLayers { count } => format!("{count} copper layers"),
            Command::SetDesignRules { .. } => "Edit design rules".into(),
            Command::SetErcSettings { .. } => "Edit electrical rules check settings".into(),
            Command::SetStackup { .. } => "Edit board stack-up".into(),
            Command::SetDrcSettings { .. } => "Edit design rules check settings".into(),
            Command::SetSimulation { .. } => "Edit simulation setup".into(),
            Command::SetTitleBlock { .. } => "Edit title block".into(),
            Command::Move { ids, .. } => format!("Move {} object(s)", ids.len()),
            Command::Rotate { ids, .. } => format!("Rotate {} object(s)", ids.len()),
            Command::Mirror { ids, .. } => format!("Mirror {} object(s)", ids.len()),
            Command::FlipSide { ids, .. } => format!("Flip {} footprint(s)", ids.len()),
            Command::Delete { ids } => format!("Delete {} object(s)", ids.len()),
            Command::Insert { objects } => format!("Insert {} object(s)", objects.len()),
            Command::Update { objects } => format!("Update {} object(s)", objects.len()),
            Command::Paste { .. } => "Paste".into(),
            Command::MoveToSheet { ids, .. } => format!("Move {} object(s) to another sheet", ids.len()),
            Command::Duplicate { ids, .. } => format!("Duplicate {} object(s)", ids.len()),
            Command::AddLibrary { path } => format!("Add library {}", file_name(path)),
            Command::RemoveLibrary { path } => format!("Remove library {}", file_name(path)),
            Command::RelinkLibrary { to, .. } => format!("Relink library to {}", file_name(to)),
        }
    }

    /// Import the parts `ids` of `library` ([`Command::ImportParts`]).
    pub fn import_parts(library: &Library, ids: &[Id]) -> Result<Command, CommandError> {
        let parts = library.bundles(ids).map_err(|id| CommandError::NotFound(ObjectId::LibPart(id)))?;
        Ok(Command::ImportParts { parts })
    }

    /// Update the project's copies of the parts `ids` from `library` ([`Command::UpdateParts`]).
    pub fn update_parts(library: &Library, ids: &[Id]) -> Result<Command, CommandError> {
        let parts = library.bundles(ids).map_err(|id| CommandError::NotFound(ObjectId::LibPart(id)))?;
        Ok(Command::UpdateParts { parts })
    }

    pub(crate) fn apply(self, tx: &mut Transaction) -> Result<(), CommandError> {
        match self {
            Command::ImportParts { parts } => import_parts(tx, parts),
            Command::UpdateParts { parts } => update_parts(tx, parts),
            Command::AddSheet { parent, name, pos, size } => {
                let parent = resolve_sheet(tx, parent)?;
                let sheet = Sheet {
                    id: Id::new(),
                    name,
                    parent: Some(parent),
                    pos,
                    size: size.unwrap_or(Sheet::SIZE),
                    page: Page::default(),
                    title_block: None,
                };
                check_references(tx.project(), &Object::Sheet(sheet.clone()))?;
                tx.insert(Object::Sheet(sheet)).map(drop)
            }
            Command::PlacePart { part, variant, sheet, pos, rotation, mirror, refdes, value } => {
                place_part(tx, part, variant, sheet, pos, rotation, mirror, refdes, value)
            }
            Command::PlaceBoardPart { part, variant, pos, rotation, side, refdes, value } => {
                place_board_part(tx, part, variant, pos, rotation, side, refdes, value)
            }
            Command::PlaceUnit { component, unit, sheet, pos, rotation, mirror } => {
                let sheet = resolve_sheet(tx, sheet)?;
                let p = tx.project();
                let c = p
                    .schematic
                    .components
                    .get(component)
                    .ok_or(CommandError::NotFound(ObjectId::Component(component)))?;
                let units = p.library.parts.get(c.part).map_or(0, |part| part.units.len());
                if unit as usize >= units {
                    return Err(CommandError::Invalid(format!(
                        "the part has {units} unit(s); unit {unit} does not exist"
                    )));
                }
                if p.schematic.symbols_of(component).any(|s| s.unit == unit) {
                    return Err(CommandError::Invalid(format!("unit {unit} of {} is already placed", c.refdes)));
                }
                let symbol = SymbolInstance { id: Id::new(), sheet, component, unit, pos, rotation, mirror };
                insert_symbol(tx, symbol)
            }
            Command::AddWire { sheet, points } => {
                let sheet = resolve_sheet(tx, sheet)?;
                let segments: Vec<_> = points.windows(2).filter(|w| w[0] != w[1]).collect();
                if segments.is_empty() {
                    return Err(CommandError::Invalid("a wire needs two distinct points".into()));
                }
                for w in segments {
                    tx.insert(Object::Wire(Wire { id: Id::new(), sheet, a: w[0], b: w[1] }))?;
                }
                Ok(())
            }
            Command::AddBus { sheet, points } => {
                let sheet = resolve_sheet(tx, sheet)?;
                let segments: Vec<_> = points.windows(2).filter(|w| w[0] != w[1]).collect();
                if segments.is_empty() {
                    return Err(CommandError::Invalid("a bus needs two distinct points".into()));
                }
                for w in segments {
                    tx.insert(Object::BusWire(BusWire { id: Id::new(), sheet, a: w[0], b: w[1] }))?;
                }
                Ok(())
            }
            Command::AddBusEntry { sheet, pos, dir } => {
                let sheet = resolve_sheet(tx, sheet)?;
                tx.insert(Object::BusEntry(BusEntry { id: Id::new(), sheet, pos, dir })).map(drop)
            }
            Command::AddJunction { sheet, pos } => {
                let sheet = resolve_sheet(tx, sheet)?;
                tx.insert(Object::Junction(Junction { id: Id::new(), sheet, pos })).map(drop)
            }
            Command::AddNoConnect { sheet, pos } => {
                let sheet = resolve_sheet(tx, sheet)?;
                if tx.project().schematic.no_connects.iter().any(|n| n.sheet == sheet && n.pos == pos) {
                    return Err(CommandError::Invalid("there is a no-connect flag here already".into()));
                }
                tx.insert(Object::NoConnect(NoConnect { id: Id::new(), sheet, pos })).map(drop)
            }
            Command::AddPowerFlag { sheet, pos, rotation } => {
                let sheet = resolve_sheet(tx, sheet)?;
                if tx.project().schematic.power_flags.iter().any(|f| f.sheet == sheet && f.pos == pos) {
                    return Err(CommandError::Invalid("there is a power flag here already".into()));
                }
                tx.insert(Object::PowerFlag(PowerFlag { id: Id::new(), sheet, pos, rotation })).map(drop)
            }
            Command::AddLabel { sheet, pos, name, kind, shape, style, rotation } => {
                if name.trim().is_empty() {
                    return Err(CommandError::Invalid("label name must not be empty".into()));
                }
                let sheet = resolve_sheet(tx, sheet)?;
                let label = NetLabel { id: Id::new(), sheet, pos, name, kind, shape, style, rotation };
                tx.insert(Object::Label(label)).map(drop)
            }
            Command::AddText { sheet, layer, pos, text, size, bold, rotation, halign, valign } => {
                if text.trim().is_empty() {
                    return Err(CommandError::Invalid("a text can't be empty".into()));
                }
                let default = if layer.is_some() { mm(1.0) } else { 1_270_000 };
                let mut t = Text::new(text, pos, size.unwrap_or(default)).aligned(halign, valign);
                t.rotation = rotation;
                t.bold = bold.unwrap_or(layer.is_some());
                match layer {
                    Some(layer) => {
                        if !tx.project().board.layers.iter().any(|l| l.id == layer) || layer == LayerId::OUTLINE {
                            return Err(CommandError::Invalid(format!("no layer {layer:?} for texts")));
                        }
                        tx.insert(Object::BoardText(BoardText { id: Id::new(), layer, text: t })).map(drop)
                    }
                    None => {
                        let sheet = resolve_sheet(tx, sheet)?;
                        tx.insert(Object::SchText(SchText { id: Id::new(), sheet, text: t })).map(drop)
                    }
                }
            }
            Command::SetComponent { id, refdes, value, variant } => {
                if let Some(v) = variant {
                    let c = tx.project().schematic.components.get(id);
                    let ok =
                        c.and_then(|c| tx.project().library.parts.get(c.part)).is_some_and(|p| p.variant(v).is_some());
                    if !ok {
                        return Err(CommandError::Invalid("variant does not belong to the part".into()));
                    }
                }
                if let Some(r) = &refdes {
                    if r.trim().is_empty() {
                        return Err(CommandError::Invalid("a reference can't be empty".into()));
                    }
                    // Unannotated references (`R?`) may repeat.
                    if !r.ends_with('?') && tx.project().schematic.component_by_refdes(r).is_some_and(|c| c.id != id) {
                        return Err(CommandError::Invalid(format!("{r} is already used")));
                    }
                }
                tx.update(ObjectId::Component(id), |o| {
                    let Object::Component(c) = o else { unreachable!() };
                    if let Some(r) = refdes {
                        c.refdes = r;
                    }
                    if let Some(v) = value {
                        c.value = v;
                    }
                    if let Some(v) = variant {
                        c.variant = v;
                    }
                    Ok(())
                })
            }
            Command::AddTrack { layer, points, width, mids } => {
                if !layer.is_copper() || !tx.project().board.layers.iter().any(|l| l.id == layer) {
                    return Err(CommandError::Invalid(format!("{layer:?} is not a copper layer of this board")));
                }
                let width = width.unwrap_or(tx.project().board.rules.track_width);
                let segments: Vec<_> = points.windows(2).enumerate().filter(|(_, w)| w[0] != w[1]).collect();
                if segments.is_empty() {
                    return Err(CommandError::Invalid("a track needs two distinct points".into()));
                }
                for (i, w) in segments {
                    let mut t = Track::straight(layer, w[0], w[1], width);
                    t.mid = mids.get(i).copied().flatten();
                    tx.insert(Object::Track(t))?;
                }
                Ok(())
            }
            Command::AddCutout { id, outline } => tx.update(id, |o| {
                let (outer, holes) = match o {
                    Object::Zone(z) => (&mut z.outline, &mut z.holes),
                    Object::Keepout(k) => (&mut k.outline, &mut k.holes),
                    _ => return Err(CommandError::Invalid("only zones and keepouts take cutouts".into())),
                };
                let (o2, h2) = crate::zones::cut(outer, holes, &outline).map_err(CommandError::Invalid)?;
                *outer = o2;
                *holes = h2;
                Ok(())
            }),
            Command::RoundCorners { tracks, radius } => {
                if radius <= 0 {
                    return Err(CommandError::Invalid("the radius must be positive".into()));
                }
                let corners = crate::fillet::corners(tx.project(), &tracks);
                if corners.is_empty() {
                    return Err(CommandError::Invalid(
                        "no corners: two of the tracks must end at the same point, with nothing else there".into(),
                    ));
                }
                let mut rounded = 0;
                for c in &corners {
                    rounded += crate::fillet::round(tx, c, radius)? as usize;
                }
                if rounded == 0 {
                    return Err(CommandError::Invalid(format!(
                        "the tracks are too short for a {} mm radius",
                        radius as f64 / 1e6
                    )));
                }
                Ok(())
            }
            Command::SetCopperLayers { count } => set_copper_layers(tx, count),
            Command::SetTitleBlock { title_block } => {
                let root = tx.project().schematic.root_sheet().ok_or(CommandError::Invalid("no root sheet".into()))?;
                tx.update(ObjectId::Sheet(root), |o| {
                    if let Object::Sheet(s) = o {
                        s.title_block = Some(title_block);
                    }
                    Ok(())
                })
            }
            Command::SetDesignRules { rules } => {
                if rules.track_width <= 0 || rules.via_drill <= 0 || rules.via_diameter <= rules.via_drill {
                    return Err(CommandError::Invalid(
                        "track width and drill must be positive, and vias larger than their drill".into(),
                    ));
                }
                if rules.mask_expansion < 0 {
                    return Err(CommandError::Invalid("the solder mask expansion can't be negative".into()));
                }
                check_net_classes(&rules).map_err(CommandError::Invalid)?;
                let mut setup = Setup::of(tx.project());
                setup.rules = rules;
                tx.set_setup(setup);
                Ok(())
            }
            Command::SetStackup { stackup } => {
                let copper = [stackup.outer_copper, stackup.inner_copper];
                if stackup.thickness <= 0 || copper.iter().any(|&c| c <= 0) {
                    return Err(CommandError::Invalid("thickness and copper must be positive".into()));
                }
                let layers = tx.project().board.copper_layers().count() as Coord;
                if stackup.outer_copper * 2 + stackup.inner_copper * (layers - 2) >= stackup.thickness {
                    return Err(CommandError::Invalid("the copper is thicker than the board".into()));
                }
                let mut setup = Setup::of(tx.project());
                setup.stackup = stackup;
                tx.set_setup(setup);
                Ok(())
            }
            Command::SetErcSettings { erc } => {
                let mut setup = Setup::of(tx.project());
                setup.erc = erc;
                tx.set_setup(setup);
                Ok(())
            }
            Command::SetDrcSettings { drc } => {
                let mut setup = Setup::of(tx.project());
                setup.drc = drc;
                tx.set_setup(setup);
                Ok(())
            }
            Command::SetSimulation { simulation } => {
                check_simulation(tx.project(), &simulation).map_err(CommandError::Invalid)?;
                let mut setup = Setup::of(tx.project());
                setup.simulation = simulation;
                tx.set_setup(setup);
                Ok(())
            }
            Command::AddVia { pos, diameter, drill, span } => {
                let board = &tx.project().board;
                let rules = &board.rules;
                let [from, to] = span.unwrap_or([LayerId::TOP_COPPER, LayerId::BOTTOM_COPPER]);
                let copper = |l: LayerId| board.copper_layers().any(|c| c.id == l);
                if !copper(from) || !copper(to) || from == to {
                    return Err(CommandError::Invalid("a via spans two different copper layers of the board".into()));
                }
                let (from, to) = if from.0 < to.0 { (from, to) } else { (to, from) };
                let via = Via {
                    id: Id::new(),
                    pos,
                    diameter: diameter.unwrap_or(rules.via_diameter),
                    drill: drill.unwrap_or(rules.via_drill),
                    from,
                    to,
                };
                if via.drill >= via.diameter {
                    return Err(CommandError::Invalid("via drill must be smaller than its diameter".into()));
                }
                tx.insert(Object::Via(via)).map(drop)
            }
            Command::Move { ids, delta, keep_connected } => {
                let t = Transform::new(delta, Angle::ZERO, false);
                transform_connected(tx, &ids, t, Edit::Transform, keep_connected)
            }
            Command::Rotate { ids, center, angle, keep_connected } => {
                let offset = center - center.rotated(angle);
                transform_connected(tx, &ids, Transform::new(offset, angle, false), Edit::Transform, keep_connected)
            }
            Command::Mirror { ids, axis_x, keep_connected } => {
                let t = Transform::new(Point::new(2 * axis_x, 0), Angle::ZERO, true);
                transform_connected(tx, &ids, t, Edit::Mirror, keep_connected)
            }
            Command::FlipSide { ids, axis_x, keep_connected } => {
                // Footprint texts flip with their footprint.
                let ids: Vec<ObjectId> =
                    ids.into_iter().filter(|id| !matches!(id, ObjectId::FootprintField(_))).collect();
                if let Some(bad) = ids.iter().find(|id| !matches!(id, ObjectId::Footprint(_))) {
                    return Err(CommandError::Invalid(format!("only footprints can change sides, got {bad:?}")));
                }
                let t = Transform::new(Point::new(2 * axis_x, 0), Angle::ZERO, true);
                if !keep_connected {
                    return transform_objects(tx, &ids, t, Edit::Flip);
                }
                let tracks = crate::track_band::plan(tx.project(), &ids, &t);
                transform_objects(tx, &ids, t, Edit::Flip)?;
                crate::track_band::apply(tx, tracks, &t)
            }
            Command::Delete { ids } => delete(tx, &ids),
            Command::Insert { objects } => {
                for o in objects {
                    check_references(tx.project(), &o)?;
                    if matches!(o, Object::Footprint(_)) {
                        return Err(CommandError::Forbidden(
                            "footprints are created by placing parts in the schematic".into(),
                        ));
                    }
                    tx.insert(o)?;
                }
                Ok(())
            }
            Command::Paste { clip, sheet, at } => paste(tx, clip, sheet, at),
            Command::MoveToSheet { ids, sheet } => move_to_sheet(tx, &ids, sheet),
            Command::Duplicate { ids, delta } => {
                let p = tx.project();
                let (clip, _) = crate::clip::copy(p, &ids, Point::ZERO)
                    .ok_or_else(|| CommandError::Invalid("nothing to duplicate".into()))?;
                let sheet = ids.iter().find_map(|&id| p.schematic.sheet_of(id));
                paste(tx, clip, sheet, delta)
            }
            Command::Update { objects } => {
                for o in objects {
                    check_references(tx.project(), &o)?;
                    if let Object::LibPart(p) = &o {
                        check_part_still_fits(tx.project(), p)?;
                    }
                    if let Object::Footprint(f) = &o {
                        let current = tx.project().board.footprints.get(f.id);
                        if current.is_some_and(|c| c.component != f.component) {
                            return Err(CommandError::Forbidden("a footprint's component cannot change".into()));
                        }
                    }
                    if let Object::Symbol(s) = &o {
                        let sch = &tx.project().schematic;
                        if sch.symbols.get(s.id).is_some_and(|c| c.component != s.component) {
                            return Err(CommandError::Forbidden("a symbol's component cannot change".into()));
                        }
                        if sch.symbols_of(s.component).any(|other| other.id != s.id && other.unit == s.unit) {
                            return Err(CommandError::Invalid(format!("unit {} is already placed", s.unit)));
                        }
                    }
                    tx.replace(o)?;
                }
                Ok(())
            }
            Command::AddLibrary { path } => {
                let path = library_path(path)?;
                let mut list = tx.project().libraries.clone();
                if list.contains(&path) {
                    return Err(CommandError::Invalid(format!("{} is already in the project", path.display())));
                }
                list.push(path);
                tx.set_libraries(list);
                Ok(())
            }
            Command::RemoveLibrary { path } => {
                let mut list = tx.project().libraries.clone();
                let i = list.iter().position(|p| *p == path).ok_or_else(|| not_listed(&path))?;
                list.remove(i);
                tx.set_libraries(list);
                Ok(())
            }
            Command::RelinkLibrary { from, to } => {
                let mut list = tx.project().libraries.clone();
                let i = list.iter().position(|p| *p == from).ok_or_else(|| not_listed(&from))?;
                let to = library_path(to)?;
                if from != to && list.contains(&to) {
                    return Err(CommandError::Invalid(format!("{} is already in the project", to.display())));
                }
                list[i] = to;
                tx.set_libraries(list);
                Ok(())
            }
        }
    }
}

fn resolve_sheet(tx: &Transaction, sheet: Option<Id>) -> Result<Id, CommandError> {
    let sch = &tx.project().schematic;
    match sheet {
        Some(s) if sch.sheets.contains(s) => Ok(s),
        Some(s) => Err(CommandError::NotFound(ObjectId::Sheet(s))),
        None => sch.root_sheet().ok_or_else(|| CommandError::Invalid("schematic has no sheet".into())),
    }
}

fn paste(tx: &mut Transaction, clip: Clip, sheet: Option<Id>, at: Point) -> Result<(), CommandError> {
    let delta = at - clip.anchor;
    // What lands where the clip is pasted (moved by `delta`; the contents of copied
    // sub-sheets keep their places on their own pages).
    let mut placed = Vec::new();
    match clip.content {
        ClipContent::Schematic { objects, library } => {
            let sheet = resolve_sheet(tx, sheet)?;
            import_clip_parts(tx, &library)?;
            let mut map = PasteIds::default();
            // Sheets of copied sheet symbols: old id → new id.
            let mut sheets: HashMap<Id, Id> = HashMap::new();
            for o in objects {
                let on = |old: Id| sheets.get(&old).copied().unwrap_or(sheet);
                let top = |old: Id| !sheets.contains_key(&old);
                let (o, top) = match o {
                    Object::Component(c) => {
                        map.component(tx, c)?;
                        continue;
                    }
                    Object::SymbolField(f) => {
                        map.symbol_field(tx, f)?;
                        continue;
                    }
                    Object::Sheet(mut s) => {
                        let old = s.id;
                        let parent = s.parent.map_or(sheet, on);
                        let is_top = s.parent.is_none_or(top);
                        s.id = Id::new();
                        s.parent = Some(parent);
                        s.title_block = None;
                        if is_top {
                            s.name = unique_sheet_name(&tx.project().schematic, parent, &s.name);
                        }
                        sheets.insert(old, s.id);
                        (Object::Sheet(s), is_top)
                    }
                    Object::Symbol(mut s) => {
                        s.component = map.component_of(s.component, "symbol")?;
                        let old = s.id;
                        let is_top = top(s.sheet);
                        (s.id, s.sheet) = (Id::new(), on(s.sheet));
                        map.symbols.insert(old, s.id);
                        (Object::Symbol(s), is_top)
                    }
                    Object::Wire(mut w) => {
                        let is_top = top(w.sheet);
                        (w.id, w.sheet) = (Id::new(), on(w.sheet));
                        (Object::Wire(w), is_top)
                    }
                    Object::BusWire(mut w) => {
                        let is_top = top(w.sheet);
                        (w.id, w.sheet) = (Id::new(), on(w.sheet));
                        (Object::BusWire(w), is_top)
                    }
                    Object::BusEntry(mut e) => {
                        let is_top = top(e.sheet);
                        (e.id, e.sheet) = (Id::new(), on(e.sheet));
                        (Object::BusEntry(e), is_top)
                    }
                    Object::Junction(mut j) => {
                        let is_top = top(j.sheet);
                        (j.id, j.sheet) = (Id::new(), on(j.sheet));
                        (Object::Junction(j), is_top)
                    }
                    Object::NoConnect(mut n) => {
                        let is_top = top(n.sheet);
                        (n.id, n.sheet) = (Id::new(), on(n.sheet));
                        (Object::NoConnect(n), is_top)
                    }
                    Object::PowerFlag(mut f) => {
                        let is_top = top(f.sheet);
                        (f.id, f.sheet) = (Id::new(), on(f.sheet));
                        (Object::PowerFlag(f), is_top)
                    }
                    Object::Label(mut l) => {
                        let is_top = top(l.sheet);
                        (l.id, l.sheet) = (Id::new(), on(l.sheet));
                        (Object::Label(l), is_top)
                    }
                    Object::SchText(mut t) => {
                        let is_top = top(t.sheet);
                        (t.id, t.sheet) = (Id::new(), on(t.sheet));
                        (Object::SchText(t), is_top)
                    }
                    other => {
                        return Err(CommandError::Invalid(format!(
                            "{:?} can't be pasted into a schematic",
                            other.id().kind()
                        )));
                    }
                };
                check_references(tx.project(), &o)?;
                let id = tx.insert(o)?;
                if top {
                    placed.push(id);
                }
            }
            // Symbols copied without their texts get fresh ones.
            for s in map.symbols.into_values().filter(|s| !map.with_fields.contains(s)) {
                insert_symbol_fields(tx, s)?;
            }
        }
        ClipContent::Board { objects, library } => {
            import_clip_parts(tx, &library)?;
            let mut map = PasteIds::default();
            let mut footprints: HashMap<Id, Id> = HashMap::new();
            let mut with_fields = HashSet::new();
            for o in objects {
                let o = match o {
                    Object::Component(c) => {
                        let part = tx.project().library.parts.get(c.part);
                        if !part.is_some_and(|p| p.is_board_part()) {
                            return Err(CommandError::Invalid(
                                "only board parts can be pasted onto the board; schematic parts are placed in the schematic"
                                    .into(),
                            ));
                        }
                        map.component(tx, c)?;
                        continue;
                    }
                    Object::Footprint(mut f) => {
                        f.component = map.component_of(f.component, "footprint")?;
                        let old = f.id;
                        f.id = Id::new();
                        footprints.insert(old, f.id);
                        Object::Footprint(f)
                    }
                    Object::FootprintField(mut f) => {
                        f.footprint = *footprints
                            .get(&f.footprint)
                            .ok_or_else(|| CommandError::Invalid("the clip lacks the footprint of a text".into()))?;
                        f.id = Id::new();
                        with_fields.insert(f.footprint);
                        tx.insert(Object::FootprintField(f))?;
                        continue;
                    }
                    Object::Track(mut t) => {
                        t.id = Id::new();
                        Object::Track(t)
                    }
                    Object::Via(mut v) => {
                        v.id = Id::new();
                        Object::Via(v)
                    }
                    Object::Zone(mut z) => {
                        z.id = Id::new();
                        Object::Zone(z)
                    }
                    Object::Keepout(mut k) => {
                        k.id = Id::new();
                        Object::Keepout(k)
                    }
                    Object::BoardGraphic(mut g) => {
                        g.id = Id::new();
                        Object::BoardGraphic(g)
                    }
                    Object::BoardText(mut t) => {
                        t.id = Id::new();
                        Object::BoardText(t)
                    }
                    other => {
                        return Err(CommandError::Invalid(format!(
                            "{:?} can't be pasted onto the board",
                            other.id().kind()
                        )));
                    }
                };
                check_references(tx.project(), &o)?;
                placed.push(tx.insert(o)?);
            }
            for f in footprints.into_values().filter(|f| !with_fields.contains(f)) {
                insert_fields(tx, f)?;
            }
        }
        other => {
            return Err(CommandError::Invalid(format!(
                "the clip holds {}; paste them into a symbol or footprint",
                other.describe()
            )));
        }
    }
    if delta != Point::ZERO {
        transform_objects(tx, &placed, Transform::new(delta, Angle::ZERO, false), Edit::Transform)?;
    }
    Ok(())
}

fn move_to_sheet(tx: &mut Transaction, ids: &[ObjectId], target: Id) -> Result<(), CommandError> {
    let sch = &tx.project().schematic;
    if !sch.sheets.contains(target) {
        return Err(CommandError::NotFound(ObjectId::Sheet(target)));
    }
    let mut moves: Vec<ObjectId> = Vec::new();
    for &id in ids {
        match id {
            ObjectId::SymbolField(_) => continue, // drawn on its symbol's sheet
            ObjectId::Sheet(s) => {
                if sch.sheet_and_descendants(s).contains(&target) {
                    return Err(CommandError::Invalid("a sheet can't move onto itself or its own sub-sheets".into()));
                }
            }
            ObjectId::Symbol(sym) => {
                // No-connect and power flags on its pins go along.
                moves.extend(flags_on_pins(tx.project(), sym));
            }
            ObjectId::Wire(_)
            | ObjectId::BusWire(_)
            | ObjectId::BusEntry(_)
            | ObjectId::Junction(_)
            | ObjectId::NoConnect(_)
            | ObjectId::PowerFlag(_)
            | ObjectId::Label(_)
            | ObjectId::SchText(_) => {}
            other => {
                return Err(CommandError::Invalid(format!(
                    "{:?} isn't on a sheet; only schematic objects move between sheets",
                    other.kind()
                )));
            }
        }
        if sch.sheet_of(id).is_none() {
            return Err(CommandError::NotFound(id));
        }
        moves.push(id);
    }
    moves.sort();
    moves.dedup();
    moves.retain(|&id| tx.project().schematic.sheet_of(id) != Some(target));
    if moves.is_empty() {
        return Err(CommandError::Invalid("everything is on that sheet already".into()));
    }
    for id in moves {
        let name = match id {
            ObjectId::Sheet(s) => tx.project().schematic.sheets.get(s).map(|s| s.name.clone()),
            _ => None,
        };
        let name = name.map(|n| unique_sheet_name(&tx.project().schematic, target, &n));
        tx.update(id, |o| {
            match o {
                Object::Sheet(s) => {
                    s.parent = Some(target);
                    if let Some(n) = name {
                        s.name = n;
                    }
                }
                Object::Symbol(x) => x.sheet = target,
                Object::Wire(x) => x.sheet = target,
                Object::BusWire(x) => x.sheet = target,
                Object::BusEntry(x) => x.sheet = target,
                Object::Junction(x) => x.sheet = target,
                Object::NoConnect(x) => x.sheet = target,
                Object::PowerFlag(x) => x.sheet = target,
                Object::Label(x) => x.sheet = target,
                Object::SchText(x) => x.sheet = target,
                _ => {}
            }
            Ok(())
        })?;
    }
    Ok(())
}

/// New ids of pasted components and symbols.
#[derive(Default)]
struct PasteIds {
    components: HashMap<Id, Id>,
    symbols: HashMap<Id, Id>,
    /// Pasted symbols whose texts came along.
    with_fields: HashSet<Id>,
}

impl PasteIds {
    /// Insert a copy of component `c` with a new id, keeping its reference if it is free.
    fn component(&mut self, tx: &mut Transaction, mut c: Component) -> Result<(), CommandError> {
        let old = c.id;
        c.id = Id::new();
        let part = tx.project().library.parts.get(c.part).ok_or(CommandError::NotFound(ObjectId::LibPart(c.part)))?;
        if part.variant(c.variant).is_none() {
            c.variant = part.variants.first().map(|v| v.id).unwrap_or(c.variant);
        }
        if c.refdes.is_empty() || tx.project().schematic.component_by_refdes(&c.refdes).is_some() {
            c.refdes = next_refdes(tx.project(), &part.prefix.clone());
        }
        self.components.insert(old, c.id);
        tx.insert(Object::Component(c)).map(drop)
    }

    fn component_of(&self, old: Id, what: &str) -> Result<Id, CommandError> {
        self.components
            .get(&old)
            .copied()
            .ok_or_else(|| CommandError::Invalid(format!("the clip lacks the component of a {what}")))
    }

    fn symbol_field(&mut self, tx: &mut Transaction, mut f: SymbolField) -> Result<(), CommandError> {
        f.symbol = *self
            .symbols
            .get(&f.symbol)
            .ok_or_else(|| CommandError::Invalid("the clip lacks the symbol of a text".into()))?;
        f.id = Id::new();
        self.with_fields.insert(f.symbol);
        tx.insert(Object::SymbolField(f)).map(drop)
    }
}

/// Import the parts of a clip the project lacks; present ones stay as they are.
fn import_clip_parts(tx: &mut Transaction, library: &Library) -> Result<(), CommandError> {
    for part in library.parts.iter() {
        if tx.project().library.parts.contains(part.id) {
            continue;
        }
        let (part, symbols, footprints) =
            library.extract_part(part.id).ok_or(CommandError::NotFound(ObjectId::LibPart(part.id)))?;
        for s in symbols {
            if !tx.project().library.symbols.contains(s.id) {
                tx.insert(Object::LibSymbol(s))?;
            }
        }
        for f in footprints {
            if !tx.project().library.footprints.contains(f.id) {
                tx.insert(Object::LibFootprint(f))?;
            }
        }
        tx.insert(Object::LibPart(part))?;
    }
    Ok(())
}

/// `name`, or if a sibling below `parent` has it, the name with the next free number
/// (`amp` → `amp2`, `amp2` → `amp3`).
pub(crate) fn unique_sheet_name(sch: &Schematic, parent: Id, name: &str) -> String {
    let taken = |n: &str| sch.children(parent).any(|s| s.name == n);
    if !taken(name) {
        return name.to_string();
    }
    let stem = name.trim_end_matches(|c: char| c.is_ascii_digit());
    let stem = if stem.is_empty() { name } else { stem };
    (2..).map(|n| format!("{stem}{n}")).find(|n| !taken(n)).expect("a free name")
}

fn import_parts(tx: &mut Transaction, parts: Vec<PartBundle>) -> Result<(), CommandError> {
    for b in parts {
        check_bundle(&b)?;
        let PartBundle { part, symbols, footprints } = b;
        for s in symbols {
            insert_or_check(tx, Object::LibSymbol(s))?;
        }
        for f in footprints {
            insert_or_check(tx, Object::LibFootprint(f))?;
        }
        insert_or_check(tx, Object::LibPart(part))?;
    }
    Ok(())
}

/// A bundle holds exactly the symbols and footprints its part references, each once.
fn check_bundle(b: &PartBundle) -> Result<(), CommandError> {
    use std::collections::HashSet;
    let wanted_symbols: HashSet<Id> = b.part.units.iter().map(|u| u.symbol).collect();
    let wanted_footprints: HashSet<Id> = b.part.variants.iter().filter_map(|v| v.footprint).collect();
    let symbols: HashSet<Id> = b.symbols.iter().map(|s| s.id).collect();
    let footprints: HashSet<Id> = b.footprints.iter().map(|f| f.id).collect();
    if symbols != wanted_symbols
        || footprints != wanted_footprints
        || symbols.len() != b.symbols.len()
        || footprints.len() != b.footprints.len()
    {
        return Err(CommandError::Invalid(format!(
            "part {:?}: a bundle must hold exactly the symbols and footprints the part references",
            b.part.name
        )));
    }
    Ok(())
}

fn update_parts(tx: &mut Transaction, parts: Vec<PartBundle>) -> Result<(), CommandError> {
    for b in parts {
        check_bundle(&b)?;
        let PartBundle { part, symbols, footprints } = b;
        let part_id = part.id;
        let old = tx
            .project()
            .library
            .parts
            .get(part_id)
            .cloned()
            .ok_or(CommandError::NotFound(ObjectId::LibPart(part_id)))?;

        // Placed units must still exist.
        let components: Vec<Component> =
            tx.project().schematic.components.iter().filter(|c| c.part == part_id).cloned().collect();
        for c in &components {
            check_kind_kept(tx.project(), c, &part)?;
            if let Some(s) = tx.project().schematic.symbols_of(c.id).find(|s| s.unit as usize >= part.units.len()) {
                return Err(CommandError::Forbidden(format!(
                    "{} has unit {} placed, but the library part now has {} unit(s)",
                    c.refdes,
                    s.unit + 1,
                    part.units.len()
                )));
            }
        }

        for s in symbols {
            upsert(tx, Object::LibSymbol(s))?;
        }
        for f in footprints {
            upsert(tx, Object::LibFootprint(f))?;
        }
        let new_variants = part.variants.clone();
        tx.replace(Object::LibPart(part))?;

        // Keep components on a valid variant.
        for c in components {
            if new_variants.iter().any(|v| v.id == c.variant) {
                continue;
            }
            let old_name = old.variant(c.variant).map(|v| v.name.as_str());
            let Some(v) = new_variants.iter().find(|v| Some(v.name.as_str()) == old_name).or(new_variants.first())
            else {
                return Err(CommandError::Invalid(format!("library part {:?} has no footprint variant", old.name)));
            };
            let variant = v.id;
            tx.update(ObjectId::Component(c.id), |o| {
                let Object::Component(c) = o else { unreachable!() };
                c.variant = variant;
                Ok(())
            })?;
        }

        // Drop symbols/footprints no longer used by any project part.
        let lib = &tx.project().library;
        let unused_symbols: Vec<Id> = old
            .units
            .iter()
            .map(|u| u.symbol)
            .filter(|&s| !lib.parts.iter().any(|p| p.units.iter().any(|u| u.symbol == s)))
            .collect();
        let unused_footprints: Vec<Id> = old
            .variants
            .iter()
            .filter_map(|v| v.footprint)
            .filter(|&f| !lib.parts.iter().any(|p| p.variants.iter().any(|v| v.footprint == Some(f))))
            .collect();
        for s in unused_symbols {
            if tx.project().library.symbols.contains(s) {
                tx.remove(ObjectId::LibSymbol(s))?;
            }
        }
        for f in unused_footprints {
            if tx.project().library.footprints.contains(f) {
                tx.remove(ObjectId::LibFootprint(f))?;
            }
        }
    }
    Ok(())
}

/// Insert or replace a library object.
fn upsert(tx: &mut Transaction, o: Object) -> Result<(), CommandError> {
    match tx.project().get(o.id()) {
        None => tx.insert(o).map(drop),
        Some(existing) if existing == o => Ok(()),
        Some(_) => tx.replace(o),
    }
}

/// Insert a library object unless an identical copy is already present.
fn insert_or_check(tx: &mut Transaction, o: Object) -> Result<(), CommandError> {
    match tx.project().get(o.id()) {
        None => tx.insert(o).map(drop),
        Some(existing) if existing == o => Ok(()),
        Some(_) => Err(CommandError::Invalid(format!(
            "{:?} already exists in the project with different content; use update from library",
            o.id()
        ))),
    }
}

#[allow(clippy::too_many_arguments)]
fn place_part(
    tx: &mut Transaction,
    part_id: Id,
    variant: Option<Id>,
    sheet: Option<Id>,
    pos: Point,
    rotation: Angle,
    mirror: bool,
    refdes: Option<String>,
    value: Option<String>,
) -> Result<(), CommandError> {
    let sheet = resolve_sheet(tx, sheet)?;
    let component = new_component(tx.project(), part_id, variant, refdes, value, false)?;
    let symbol = SymbolInstance { id: Id::new(), sheet, component: component.id, unit: 0, pos, rotation, mirror };
    // Multi-unit parts: only unit 0 is placed for now (see PLAN.md).
    tx.insert(Object::Component(component))?;
    insert_symbol(tx, symbol)
}

/// Insert a symbol with its reference and value texts.
pub(crate) fn insert_symbol(tx: &mut Transaction, symbol: SymbolInstance) -> Result<(), CommandError> {
    let id = symbol.id;
    tx.insert(Object::Symbol(symbol))?;
    insert_symbol_fields(tx, id)
}

/// Add the reference and value texts of a newly placed symbol.
pub(crate) fn insert_symbol_fields(tx: &mut Transaction, symbol: Id) -> Result<(), CommandError> {
    let p = tx.project();
    let Some(def) = p.schematic.symbols.get(symbol).and_then(|s| crate::query::symbol_of_instance(p, s)) else {
        return Ok(());
    };
    for field in SymbolField::for_symbol(symbol, def) {
        tx.insert(Object::SymbolField(field))?;
    }
    Ok(())
}

/// Remove a symbol and its texts.
fn remove_symbol(tx: &mut Transaction, symbol: Id) -> Result<(), CommandError> {
    let fields: Vec<Id> = tx.project().schematic.fields.iter().filter(|f| f.symbol == symbol).map(|f| f.id).collect();
    for f in fields {
        tx.remove(ObjectId::SymbolField(f))?;
    }
    tx.remove(ObjectId::Symbol(symbol)).map(drop)
}

#[allow(clippy::too_many_arguments)]
fn place_board_part(
    tx: &mut Transaction,
    part_id: Id,
    variant: Option<Id>,
    pos: Point,
    rotation: Angle,
    side: Side,
    refdes: Option<String>,
    value: Option<String>,
) -> Result<(), CommandError> {
    let component = new_component(tx.project(), part_id, variant, refdes, value, true)?;
    let footprint = FootprintInstance { id: Id::new(), component: component.id, pos, rotation, side, locked: false };
    // The footprint is in place before sync runs, so sync doesn't stage another one.
    tx.insert(Object::Component(component))?;
    let id = footprint.id;
    tx.insert(Object::Footprint(footprint))?;
    insert_fields(tx, id)
}

/// Add the reference and value texts of a newly placed footprint.
pub(crate) fn insert_fields(tx: &mut Transaction, footprint: Id) -> Result<(), CommandError> {
    let p = tx.project();
    let Some(def) = p.board.footprints.get(footprint).and_then(|f| crate::query::footprint_of_instance(p, f)) else {
        return Ok(());
    };
    for field in FootprintField::for_footprint(footprint, def) {
        tx.insert(Object::FootprintField(field))?;
    }
    Ok(())
}

/// Remove the texts of the footprints `footprints` (one pass over all texts).
pub(crate) fn remove_fields(
    tx: &mut Transaction,
    footprints: &std::collections::HashSet<Id>,
) -> Result<(), CommandError> {
    if footprints.is_empty() {
        return Ok(());
    }
    let ids: Vec<Id> =
        tx.project().board.fields.iter().filter(|f| footprints.contains(&f.footprint)).map(|f| f.id).collect();
    for id in ids {
        tx.remove(ObjectId::FootprintField(id))?;
    }
    Ok(())
}

/// A new component of `part_id` (a board part when `board`), not yet inserted.
fn new_component(
    project: &Project,
    part_id: Id,
    variant: Option<Id>,
    refdes: Option<String>,
    value: Option<String>,
    board: bool,
) -> Result<Component, CommandError> {
    let part = project.library.parts.get(part_id).ok_or(CommandError::NotFound(ObjectId::LibPart(part_id)))?;
    match (board, part.is_board_part()) {
        (false, true) => {
            return Err(CommandError::Invalid(format!(
                "{:?} is a board part (no symbol); place it on the board (place_board_part)",
                part.name
            )));
        }
        (true, false) => {
            return Err(CommandError::Invalid(format!(
                "{:?} has a symbol; place it in the schematic and its footprint follows",
                part.name
            )));
        }
        _ => {}
    }
    let variant = match variant {
        Some(v) if part.variant(v).is_some() => v,
        Some(_) => return Err(CommandError::Invalid("variant does not belong to the part".into())),
        None => part.variants.first().ok_or_else(|| CommandError::Invalid("part has no variant".into()))?.id,
    };
    let refdes = match refdes {
        Some(r) => {
            if project.schematic.component_by_refdes(&r).is_some() {
                return Err(CommandError::Invalid(format!("reference designator {r} is already used")));
            }
            r
        }
        None => next_refdes(project, &part.prefix),
    };
    Ok(Component {
        sim: Default::default(),
        id: Id::new(),
        part: part_id,
        variant,
        refdes,
        value: value.unwrap_or_else(|| part.default_value.clone()),
        properties: Default::default(),
        dnp: false,
    })
}

/// Lowest free number for a reference designator prefix ("R" → "R3").
pub fn next_refdes(project: &Project, prefix: &str) -> String {
    let used: std::collections::HashSet<u32> = project
        .schematic
        .components
        .iter()
        .filter_map(|c| c.refdes.strip_prefix(prefix).and_then(|n| n.parse().ok()))
        .collect();
    let n = (1..).find(|n| !used.contains(n)).unwrap();
    format!("{prefix}{n}")
}

#[derive(Clone, Copy, PartialEq)]
enum Edit {
    Transform,
    Mirror,
    Flip,
}

fn transform_connected(
    tx: &mut Transaction,
    ids: &[ObjectId],
    t: Transform,
    edit: Edit,
    keep_connected: bool,
) -> Result<(), CommandError> {
    // No-connect flags on moved symbols' pins go along.
    let mut all = ids.to_vec();
    for &id in ids {
        if let ObjectId::Symbol(sym) = id {
            for f in flags_on_pins(tx.project(), sym) {
                if !all.contains(&f) {
                    all.push(f);
                }
            }
        }
    }
    let ids = &all[..];
    if !keep_connected {
        return transform_objects(tx, ids, t, edit);
    }
    use crate::rubber_band::{Kind, apply, plan};
    let wires = plan(tx.project(), Kind::Wire, ids, &t);
    let buses = plan(tx.project(), Kind::Bus, ids, &t);
    let tracks = crate::track_band::plan(tx.project(), ids, &t);
    transform_objects(tx, ids, t, edit)?;
    apply(tx, wires, &t)?;
    apply(tx, buses, &t)?;
    crate::track_band::apply(tx, tracks, &t)
}

fn transform_objects(tx: &mut Transaction, ids: &[ObjectId], t: Transform, edit: Edit) -> Result<(), CommandError> {
    let mut seen = std::collections::HashSet::new();
    // Reference and value texts move in their owner's coordinates; with their owner, they
    // just go along. Footprints only flip as a whole.
    let moved: std::collections::HashSet<Id> = ids
        .iter()
        .filter_map(|id| match id {
            ObjectId::Footprint(f) | ObjectId::Symbol(f) => Some(*f),
            _ => None,
        })
        .collect();
    let mut frames: std::collections::HashMap<ObjectId, Transform> = Default::default();
    for &id in ids {
        let p = tx.project();
        let owner = match id {
            ObjectId::FootprintField(i) => {
                p.board.fields.get(i).and_then(|f| p.board.footprints.get(f.footprint)).map(|o| (o.id, o.transform()))
            }
            ObjectId::SymbolField(i) => {
                p.schematic.fields.get(i).and_then(|f| p.schematic.symbols.get(f.symbol)).map(|o| (o.id, o.transform()))
            }
            _ => continue,
        };
        if let Some((owner, frame)) = owner
            && !moved.contains(&owner)
            && edit != Edit::Flip
        {
            frames.insert(id, frame);
        }
    }
    for &id in ids {
        if !seen.insert(id) {
            continue;
        }
        if matches!(id, ObjectId::FootprintField(_) | ObjectId::SymbolField(_)) {
            let Some(&frame) = frames.get(&id) else { continue };
            if edit == Edit::Mirror {
                return Err(CommandError::Invalid("reference and value texts can't be mirrored".into()));
            }
            let move_text = |text: &mut Text| {
                let world = t.point(frame.point(text.pos));
                text.pos = frame.inverse_point(world);
                text.rotation = frame.inverse_angle(t.angle(frame.angle(text.rotation)));
            };
            tx.update(id, |o| {
                match o {
                    Object::FootprintField(f) => move_text(&mut f.text),
                    Object::SymbolField(f) => move_text(&mut f.text),
                    _ => {}
                }
                Ok(())
            })?;
            continue;
        }
        tx.update(id, |o| {
            match o {
                Object::Symbol(s) => {
                    s.pos = t.point(s.pos);
                    if t.mirror {
                        // M ∘ R(θ) = R(-θ) ∘ M: rotation negates, mirror toggles.
                        s.rotation = Angle::deg(-s.rotation.degrees());
                        s.mirror = !s.mirror;
                    } else {
                        s.rotation = s.rotation.rotated_by(t.rotation);
                    }
                }
                Object::Wire(w) => {
                    w.a = t.point(w.a);
                    w.b = t.point(w.b);
                }
                Object::BusWire(w) => {
                    w.a = t.point(w.a);
                    w.b = t.point(w.b);
                }
                Object::BusEntry(e) => {
                    let end = t.point(e.wire_end());
                    e.pos = t.point(e.pos);
                    e.dir = EntryDir::of_offset(end - e.pos);
                }
                Object::Junction(j) => j.pos = t.point(j.pos),
                Object::NoConnect(n) => n.pos = t.point(n.pos),
                Object::PowerFlag(f) => {
                    f.pos = t.point(f.pos);
                    f.rotation = t.angle(f.rotation);
                }
                Object::Sheet(sh) => {
                    if sh.is_root() {
                        return Err(CommandError::Invalid("the root sheet has no symbol to move".into()));
                    }
                    // The symbol stays an axis-aligned rectangle: rotated or mirrored corners.
                    let r = sh.rect();
                    let r = Rect::from_points(t.point(r.min), t.point(r.max));
                    sh.pos = r.min;
                    sh.size = Size { w: r.max.x - r.min.x, h: r.max.y - r.min.y };
                }
                Object::Label(l) => {
                    l.pos = t.point(l.pos);
                    l.rotation = t.angle(l.rotation);
                }
                Object::SchText(st) => st.text = st.text.transformed(&t),
                Object::Footprint(f) => {
                    if f.locked {
                        return Err(CommandError::Forbidden(format!("footprint {:?} is locked", f.id)));
                    }
                    if edit == Edit::Mirror {
                        return Err(CommandError::Invalid("use flip_side to mirror footprints".into()));
                    }
                    f.pos = t.point(f.pos);
                    if t.mirror {
                        f.rotation = Angle::deg(-f.rotation.degrees());
                        f.side = f.side.flipped();
                    } else {
                        f.rotation = f.rotation.rotated_by(t.rotation);
                    }
                }
                Object::Track(tr) => {
                    reject_mirror_on_board(edit)?;
                    tr.a = t.point(tr.a);
                    tr.b = t.point(tr.b);
                    tr.mid = tr.mid.map(|m| t.point(m));
                }
                Object::Via(v) => {
                    reject_mirror_on_board(edit)?;
                    v.pos = t.point(v.pos);
                }
                Object::Zone(z) => {
                    reject_mirror_on_board(edit)?;
                    z.outline.iter_mut().chain(z.holes.iter_mut().flatten()).for_each(|p| *p = t.point(*p));
                }
                Object::Keepout(k) => {
                    reject_mirror_on_board(edit)?;
                    k.outline.iter_mut().chain(k.holes.iter_mut().flatten()).for_each(|p| *p = t.point(*p));
                }
                Object::BoardGraphic(g) => {
                    reject_mirror_on_board(edit)?;
                    g.shape = g.shape.transformed(&t);
                }
                Object::BoardText(bt) => {
                    reject_mirror_on_board(edit)?;
                    bt.text = bt.text.transformed(&t);
                }
                other => {
                    return Err(CommandError::Invalid(format!("{:?} has no position", other.id().kind())));
                }
            }
            Ok(())
        })?;
    }
    Ok(())
}

fn reject_mirror_on_board(edit: Edit) -> Result<(), CommandError> {
    match edit {
        Edit::Transform => Ok(()),
        _ => Err(CommandError::Invalid("board objects cannot be mirrored".into())),
    }
}

/// The no-connect and power flags on the pin ends of symbol `sym`.
fn flags_on_pins(p: &Project, sym: Id) -> Vec<ObjectId> {
    let Some(inst) = p.schematic.symbols.get(sym) else { return Vec::new() };
    let pins: Vec<Point> = crate::query::pin_positions(p, sym).into_iter().map(|(_, _, pt)| pt).collect();
    let s = &p.schematic;
    let on = |sheet: Id, pos: Point| sheet == inst.sheet && pins.contains(&pos);
    s.no_connects
        .iter()
        .filter(|n| on(n.sheet, n.pos))
        .map(|n| ObjectId::NoConnect(n.id))
        .chain(s.power_flags.iter().filter(|f| on(f.sheet, f.pos)).map(|f| ObjectId::PowerFlag(f.id)))
        .collect()
}

fn delete(tx: &mut Transaction, ids: &[ObjectId]) -> Result<(), CommandError> {
    for &id in ids {
        if !tx.project().contains(id) {
            // Already removed together with another object of this command.
            if tx.changes().iter().any(|c| c.id == id && c.after.is_none()) {
                continue;
            }
            return Err(CommandError::NotFound(id));
        }
        match id {
            ObjectId::Footprint(f) => {
                // Board parts live on the board: their footprint goes with the component.
                let p = tx.project();
                let component = p.board.footprints.get(f).map(|f| f.component);
                let board_part = component
                    .and_then(|c| p.schematic.components.get(c))
                    .and_then(|c| p.library.parts.get(c.part))
                    .is_some_and(Part::is_board_part);
                let (true, Some(c)) = (board_part, component) else {
                    return Err(CommandError::Forbidden(
                        "footprints follow the schematic; delete the component there".into(),
                    ));
                };
                tx.remove(id)?;
                remove_fields(tx, &[f].into())?;
                delete_component(tx, c)?;
            }
            ObjectId::FootprintField(_) => {
                return Err(CommandError::Forbidden(
                    "reference and value texts belong to their footprint; hide them instead".into(),
                ));
            }
            ObjectId::SymbolField(_) => {
                return Err(CommandError::Forbidden(
                    "reference and value texts belong to their symbol; hide them instead".into(),
                ));
            }
            ObjectId::Sheet(s) => {
                // A sheet goes with everything on it and its sub-sheets (KiCad does the same).
                let sch = &tx.project().schematic;
                if sch.sheets.get(s).is_some_and(|x| x.is_root()) {
                    return Err(CommandError::Forbidden("the root sheet cannot be deleted".into()));
                }
                let sheets = sch.sheet_and_descendants(s);
                let contents: Vec<ObjectId> = sheets
                    .iter()
                    .flat_map(|&sh| sch.objects_on(sh))
                    .filter(|o| !matches!(o, ObjectId::Sheet(_)))
                    .collect();
                delete(tx, &contents)?;
                for &sh in sheets.iter().rev() {
                    tx.remove(ObjectId::Sheet(sh))?;
                }
            }
            ObjectId::Component(c) => delete_component(tx, c)?,
            ObjectId::Symbol(s) => {
                let component = tx.project().schematic.symbols.get(s).unwrap().component;
                for flag in flags_on_pins(tx.project(), s) {
                    tx.remove(flag)?;
                }
                remove_symbol(tx, s)?;
                // A component without any placed unit is removed with its last symbol.
                if tx.project().schematic.symbols_of(component).next().is_none() {
                    delete_component(tx, component)?;
                }
            }
            ObjectId::LibPart(p) => {
                if tx.project().schematic.components.iter().any(|c| c.part == p) {
                    return Err(CommandError::Forbidden("part is used by components".into()));
                }
                tx.remove(id)?;
            }
            ObjectId::LibSymbol(s) => {
                if tx.project().library.parts.iter().any(|p| p.units.iter().any(|u| u.symbol == s)) {
                    return Err(CommandError::Forbidden("symbol is used by a part".into()));
                }
                tx.remove(id)?;
            }
            ObjectId::LibFootprint(f) => {
                if tx.project().library.parts.iter().any(|p| p.variants.iter().any(|v| v.footprint == Some(f))) {
                    return Err(CommandError::Forbidden("footprint is used by a part".into()));
                }
                tx.remove(id)?;
            }
            _ => {
                tx.remove(id)?;
            }
        }
    }
    Ok(())
}

fn delete_component(tx: &mut Transaction, component: Id) -> Result<(), CommandError> {
    let symbols: Vec<Id> = tx.project().schematic.symbols_of(component).map(|s| s.id).collect();
    for s in symbols {
        remove_symbol(tx, s)?;
    }
    tx.remove(ObjectId::Component(component))?;
    Ok(())
}

/// Check that a raw object only references things that exist.
fn check_references(p: &Project, o: &Object) -> Result<(), CommandError> {
    let missing = |id: ObjectId| Err(CommandError::NotFound(id));
    let sheet_ok = |s: Id| p.schematic.sheets.contains(s);
    match o {
        Object::Sheet(s) => check_sheet(p, s),
        Object::Component(c) => match p.library.parts.get(c.part) {
            None => missing(ObjectId::LibPart(c.part)),
            Some(part) if part.variant(c.variant).is_none() => {
                Err(CommandError::Invalid("component variant does not belong to its part".into()))
            }
            _ => Ok(()),
        },
        Object::Symbol(s) => {
            if !sheet_ok(s.sheet) {
                return missing(ObjectId::Sheet(s.sheet));
            }
            match p.schematic.components.get(s.component) {
                None => missing(ObjectId::Component(s.component)),
                Some(c) => {
                    let units = p.library.parts.get(c.part).map_or(0, |part| part.units.len());
                    if s.unit as usize >= units {
                        return Err(CommandError::Invalid("symbol unit out of range".into()));
                    }
                    Ok(())
                }
            }
        }
        Object::Wire(Wire { sheet, .. })
        | Object::BusWire(BusWire { sheet, .. })
        | Object::BusEntry(BusEntry { sheet, .. })
        | Object::Junction(Junction { sheet, .. })
        | Object::NoConnect(NoConnect { sheet, .. })
        | Object::PowerFlag(PowerFlag { sheet, .. })
        | Object::Label(NetLabel { sheet, .. })
        | Object::SchText(SchText { sheet, .. }) => {
            if sheet_ok(*sheet) {
                Ok(())
            } else {
                missing(ObjectId::Sheet(*sheet))
            }
        }
        Object::Footprint(f) => {
            if p.schematic.components.contains(f.component) {
                Ok(())
            } else {
                missing(ObjectId::Component(f.component))
            }
        }
        Object::FootprintField(f) => {
            if p.board.footprints.contains(f.footprint) {
                Ok(())
            } else {
                missing(ObjectId::Footprint(f.footprint))
            }
        }
        Object::SymbolField(f) => {
            if p.schematic.symbols.contains(f.symbol) {
                Ok(())
            } else {
                missing(ObjectId::Symbol(f.symbol))
            }
        }
        Object::Track(t) => {
            if t.layer.is_copper() && p.board.layers.iter().any(|l| l.id == t.layer) {
                Ok(())
            } else {
                Err(CommandError::Invalid("track must be on a copper layer of the board".into()))
            }
        }
        Object::LibPart(part) => {
            for u in &part.units {
                if !p.library.symbols.contains(u.symbol) {
                    return missing(ObjectId::LibSymbol(u.symbol));
                }
            }
            for f in part.variants.iter().filter_map(|v| v.footprint) {
                if !p.library.footprints.contains(f) {
                    return missing(ObjectId::LibFootprint(f));
                }
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

/// A new or changed sheet: a trimmed name without `/`, unique among its siblings; an
/// existing parent that doesn't change; one root sheet; a symbol with a size.
fn check_sheet(p: &Project, s: &Sheet) -> Result<(), CommandError> {
    let sch = &p.schematic;
    if let Some(issue) = sch.sheet_name_issue(s.parent, &s.name, Some(s.id)) {
        return Err(CommandError::Invalid(issue));
    }
    match (sch.sheets.get(s.id), s.parent) {
        (Some(current), _) if current.parent != s.parent => {
            return Err(CommandError::Forbidden("a sheet stays on its parent sheet".into()));
        }
        (None, None) if sch.root_sheet().is_some() => {
            return Err(CommandError::Invalid("the schematic has a root sheet already".into()));
        }
        (_, Some(parent)) if !sch.sheets.contains(parent) => {
            return Err(CommandError::NotFound(ObjectId::Sheet(parent)));
        }
        _ => {}
    }
    if s.parent.is_some() && (s.size.w <= 0 || s.size.h <= 0) {
        return Err(CommandError::Invalid("a sheet symbol needs a width and a height".into()));
    }
    if let PageSize::Custom { w, h } = s.page.size
        && (w <= 0 || h <= 0)
    {
        return Err(CommandError::Invalid("a custom page needs a positive width and height".into()));
    }
    match &s.title_block {
        Some(_) if s.parent.is_some() => {
            return Err(CommandError::Invalid("only the root sheet has a title block".into()));
        }
        _ => {}
    }
    Ok(())
}

fn set_copper_layers(tx: &mut Transaction, count: u16) -> Result<(), CommandError> {
    if !(2..=32).contains(&count) || !count.is_multiple_of(2) {
        return Err(CommandError::Invalid(format!("a board has an even number of 2 to 32 copper layers, not {count}")));
    }
    let board = &tx.project().board;
    let inner = count - 2;
    let removed: Vec<LayerId> = board
        .layers
        .iter()
        .filter(|l| l.id.is_copper() && l.id != LayerId::TOP_COPPER && l.id != LayerId::BOTTOM_COPPER)
        .filter(|l| l.id.0 > inner)
        .map(|l| l.id)
        .collect();
    let used: Vec<String> = removed
        .iter()
        .filter(|&&id| {
            board.tracks.iter().any(|t| t.layer == id)
                || board.zones.iter().any(|z| z.layer == id)
                || board.keepouts.iter().any(|k| k.layers.contains(&id))
                || board.graphics.iter().any(|g| g.layer == id)
                || board.texts.iter().any(|t| t.layer == id)
        })
        .map(|&id| board.layers.iter().find(|l| l.id == id).map_or(format!("{id:?}"), |l| l.name.clone()))
        .collect();
    if !used.is_empty() {
        return Err(CommandError::Invalid(format!("{} still has copper on it", used.join(", "))));
    }
    let mut setup = Setup::of(tx.project());
    setup.layers.retain(|l| !removed.contains(&l.id));
    for n in 1..=inner {
        if !setup.layers.iter().any(|l| l.id == LayerId::inner(n)) {
            // Inner layers go before the bottom copper layer.
            let at = setup.layers.iter().position(|l| l.id == LayerId::BOTTOM_COPPER).unwrap_or(setup.layers.len());
            let layer =
                BoardLayer { id: LayerId::inner(n), name: format!("Inner {n}"), function: LayerFunction::Copper };
            setup.layers.insert(at, layer);
        }
    }
    tx.set_setup(setup);
    Ok(())
}

fn file_name(path: &std::path::Path) -> String {
    path.file_name().map_or_else(|| path.display().to_string(), |n| n.to_string_lossy().into_owned())
}

/// A library path as the project keeps it: absolute and without `.` / `..`.
fn library_path(path: std::path::PathBuf) -> Result<std::path::PathBuf, CommandError> {
    if !path.is_absolute() {
        return Err(CommandError::Invalid(format!("library path {} must be absolute", path.display())));
    }
    Ok(model::io::normalize(&path))
}

fn not_listed(path: &std::path::Path) -> CommandError {
    CommandError::Invalid(format!("{} is not one of the project's libraries", path.display()))
}

/// A changed project part must keep what placed components use: their variant, every
/// placed unit, and its kind (board part or not).
fn check_part_still_fits(project: &Project, part: &Part) -> Result<(), CommandError> {
    for c in project.schematic.components.iter().filter(|c| c.part == part.id) {
        check_kind_kept(project, c, part)?;
        if part.variant(c.variant).is_none() {
            return Err(CommandError::Forbidden(format!(
                "{} uses this footprint variant; give it another variant first",
                c.refdes
            )));
        }
        if let Some(s) = project.schematic.symbols_of(c.id).find(|s| s.unit as usize >= part.units.len()) {
            return Err(CommandError::Forbidden(format!(
                "{} has unit {} placed; delete that unit first",
                c.refdes,
                s.unit + 1
            )));
        }
    }
    Ok(())
}

/// A placed component's part can't become the other kind: a board part (placed on the
/// board, no symbols) can't get units, a schematic part can't lose all of them.
fn check_kind_kept(project: &Project, c: &Component, part: &Part) -> Result<(), CommandError> {
    let on_board_only = project.schematic.symbols_of(c.id).next().is_none();
    if on_board_only && !part.is_board_part() {
        return Err(CommandError::Forbidden(format!(
            "{} is placed on the board as a board part; a board part can't get a symbol while placed",
            c.refdes
        )));
    }
    if !on_board_only && part.is_board_part() {
        return Err(CommandError::Forbidden(format!(
            "{} is placed in the schematic; its part needs a symbol unit",
            c.refdes
        )));
    }
    Ok(())
}

/// Net classes need distinct names (not "Default"), positive sizes and vias larger than
/// their drill (with the board's values filled in).
/// Analyses have unique ids and names, sensible ranges, and DC sweeps name a component.
fn check_simulation(project: &Project, sim: &Simulation) -> Result<(), String> {
    let mut ids = std::collections::HashSet::new();
    let mut names = std::collections::HashSet::new();
    for a in &sim.analyses {
        if !ids.insert(a.id) {
            return Err(format!("analysis id {} is used twice", a.id));
        }
        let name = a.name.trim().to_lowercase();
        if name.is_empty() {
            return Err("an analysis needs a name".into());
        }
        if !names.insert(name) {
            return Err(format!("analysis name {:?} is used twice", a.name));
        }
        let finite = |v: f64| v.is_finite();
        match &a.kind {
            AnalysisKind::Op => {}
            AnalysisKind::Dc { source, start, stop, step } => {
                if !project.schematic.components.contains(*source) {
                    return Err(format!("{}: the swept source is not a component", a.name));
                }
                if !(finite(*start) && finite(*stop) && finite(*step)) || *step == 0.0 {
                    return Err(format!("{}: start, stop and a non-zero step are needed", a.name));
                }
            }
            AnalysisKind::Ac { start, stop, points_per_decade } => {
                if !(*start > 0.0 && stop > start && finite(*stop)) || *points_per_decade == 0 {
                    return Err(format!("{}: AC needs 0 < start < stop and points per decade", a.name));
                }
            }
            AnalysisKind::Tran { stop, step, start, max_step } => {
                let positive = |v: &Option<f64>| v.is_none_or(|v| v > 0.0 && v.is_finite());
                if !(*stop > 0.0 && finite(*stop))
                    || !positive(step)
                    || !positive(max_step)
                    || !(0.0..*stop).contains(start)
                {
                    return Err(format!("{}: transient needs stop > 0, 0 ≤ start < stop and positive steps", a.name));
                }
            }
        }
    }
    if !(sim.temperature.is_finite() && sim.temperature > -273.15) {
        return Err("temperature below absolute zero".into());
    }
    for a in &sim.analyses {
        if let Some(step) = &a.step {
            if let Some(p) = step.values.problem() {
                return Err(format!("{}: {p}", a.name));
            }
            if step.param.is_log() && step.values.points().iter().any(|v| *v <= 0.0) {
                return Err(format!("{}: a component value's steps must be positive", a.name));
            }
        }
    }
    let mut tuned = HashSet::new();
    for row in &sim.tune {
        if !tuned.insert(&row.param) {
            return Err("a parameter is tuned twice".into());
        }
        if !(row.min.is_finite() && row.max.is_finite() && row.min < row.max) {
            return Err("a tuner row needs min < max".into());
        }
        if row.param.is_log() && row.min <= 0.0 {
            return Err("a component value's tuning range must be positive".into());
        }
    }
    Ok(())
}

fn check_net_classes(rules: &DesignRules) -> Result<(), String> {
    let mut names = HashSet::new();
    for c in &rules.net_classes {
        let name = c.name.trim();
        if name.is_empty() || name.eq_ignore_ascii_case(DEFAULT_NET_CLASS) {
            return Err(format!("a net class needs a name other than {DEFAULT_NET_CLASS:?}"));
        }
        if !names.insert(name.to_lowercase()) {
            return Err(format!("two net classes are named {name:?}"));
        }
        let sizes = [c.clearance, c.track_width, c.via_diameter, c.via_drill];
        if sizes.iter().flatten().any(|&v| v <= 0) {
            return Err(format!("net class {name}: sizes must be positive"));
        }
        let diameter = c.via_diameter.unwrap_or(rules.via_diameter);
        if c.via_drill.unwrap_or(rules.via_drill) >= diameter {
            return Err(format!("net class {name}: vias must be larger than their drill"));
        }
    }
    Ok(())
}
