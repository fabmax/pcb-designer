//! Serializable editing commands. The GUI, scripts and MCP clients all go through these.

use serde::{Deserialize, Serialize};

use model::*;

use crate::CommandError;
use crate::transaction::{BoardSetup, Transaction};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(tag = "command", rename_all = "snake_case")]
pub enum Command {
    /// Copy parts (with their symbols and footprints) from a library into the project.
    ImportParts {
        library: Library,
        parts: Vec<Id>,
    },
    /// Replace the project's copies of parts (with their symbols and footprints) by the
    /// current library versions. Components keep their footprint variant if it still
    /// exists (matched by id, then by name), otherwise they get the first variant.
    UpdateParts {
        library: Library,
        parts: Vec<Id>,
    },
    /// Place a part from the project library: creates a component and its symbol. The
    /// footprint appears on the board automatically.
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
    AddLabel {
        #[serde(default)]
        sheet: Option<Id>,
        pos: Point,
        name: String,
        #[serde(default)]
        kind: LabelKind,
        #[serde(default)]
        rotation: Angle,
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
    },
    /// Number of copper layers (2 or more, even). Inner layers are added or removed at
    /// the bottom of the inner stack; removing a layer that has copper on it fails.
    SetCopperLayers {
        count: u16,
    },
    /// Replace the design rules (track width, via size, clearance, …).
    SetDesignRules {
        rules: DesignRules,
    },
    AddVia {
        pos: Point,
        #[serde(default)]
        diameter: Option<Coord>,
        #[serde(default)]
        drill: Option<Coord>,
    },
    /// Move objects. With `keep_connected`, schematic wires attached to moved pins,
    /// wire ends, labels and junctions follow: horizontal and vertical wires stay so by
    /// shifting sideways (stretching the wires at their other end, or adding a short
    /// wire there if needed); other wires stretch.
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
        /// Attached schematic wires stretch to stay connected.
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
    /// Move footprints to the other board side, mirrored across `x = axis_x`.
    FlipSide {
        ids: Vec<ObjectId>,
        axis_x: Coord,
    },
    Delete {
        ids: Vec<ObjectId>,
    },
    /// Insert raw objects (references are validated).
    Insert {
        objects: Vec<Object>,
    },
    /// Replace raw objects by id (references are validated).
    Update {
        objects: Vec<Object>,
    },
}

impl Command {
    /// Short human readable description, used as undo label.
    pub fn label(&self) -> String {
        match self {
            Command::ImportParts { parts, .. } => format!("Import {} part(s)", parts.len()),
            Command::UpdateParts { parts, .. } => format!("Update {} part(s) from library", parts.len()),
            Command::PlacePart { .. } => "Place part".into(),
            Command::PlaceUnit { .. } => "Place unit".into(),
            Command::AddWire { .. } => "Add wire".into(),
            Command::AddBus { .. } => "Add bus".into(),
            Command::AddBusEntry { .. } => "Add bus entry".into(),
            Command::AddJunction { .. } => "Add junction".into(),
            Command::AddLabel { name, .. } => format!("Add label {name}"),
            Command::SetComponent { .. } => "Edit component".into(),
            Command::AddTrack { .. } => "Add track".into(),
            Command::AddVia { .. } => "Add via".into(),
            Command::SetCopperLayers { count } => format!("{count} copper layers"),
            Command::SetDesignRules { .. } => "Edit design rules".into(),
            Command::Move { ids, .. } => format!("Move {} object(s)", ids.len()),
            Command::Rotate { ids, .. } => format!("Rotate {} object(s)", ids.len()),
            Command::Mirror { ids, .. } => format!("Mirror {} object(s)", ids.len()),
            Command::FlipSide { ids, .. } => format!("Flip {} footprint(s)", ids.len()),
            Command::Delete { ids } => format!("Delete {} object(s)", ids.len()),
            Command::Insert { objects } => format!("Insert {} object(s)", objects.len()),
            Command::Update { objects } => format!("Update {} object(s)", objects.len()),
        }
    }

    pub(crate) fn apply(self, tx: &mut Transaction) -> Result<(), CommandError> {
        match self {
            Command::ImportParts { library, parts } => import_parts(tx, &library, &parts),
            Command::UpdateParts { library, parts } => update_parts(tx, &library, &parts),
            Command::PlacePart { part, variant, sheet, pos, rotation, mirror, refdes, value } => {
                place_part(tx, part, variant, sheet, pos, rotation, mirror, refdes, value)
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
                tx.insert(Object::Symbol(symbol)).map(drop)
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
            Command::AddLabel { sheet, pos, name, kind, rotation } => {
                if name.trim().is_empty() {
                    return Err(CommandError::Invalid("label name must not be empty".into()));
                }
                let sheet = resolve_sheet(tx, sheet)?;
                let label = NetLabel { id: Id::new(), sheet, pos, name, kind, rotation };
                tx.insert(Object::Label(label)).map(drop)
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
            Command::AddTrack { layer, points, width } => {
                if !layer.is_copper() || !tx.project().board.layers.iter().any(|l| l.id == layer) {
                    return Err(CommandError::Invalid(format!("{layer:?} is not a copper layer of this board")));
                }
                let width = width.unwrap_or(tx.project().board.rules.track_width);
                let segments: Vec<_> = points.windows(2).filter(|w| w[0] != w[1]).collect();
                if segments.is_empty() {
                    return Err(CommandError::Invalid("a track needs two distinct points".into()));
                }
                for w in segments {
                    tx.insert(Object::Track(Track { id: Id::new(), layer, a: w[0], b: w[1], width }))?;
                }
                Ok(())
            }
            Command::SetCopperLayers { count } => set_copper_layers(tx, count),
            Command::SetDesignRules { rules } => {
                if rules.track_width <= 0 || rules.via_drill <= 0 || rules.via_diameter <= rules.via_drill {
                    return Err(CommandError::Invalid(
                        "track width and drill must be positive, and vias larger than their drill".into(),
                    ));
                }
                let mut setup = BoardSetup::of(tx.project());
                setup.rules = rules;
                tx.set_setup(setup);
                Ok(())
            }
            Command::AddVia { pos, diameter, drill } => {
                let rules = &tx.project().board.rules;
                let via = Via {
                    id: Id::new(),
                    pos,
                    diameter: diameter.unwrap_or(rules.via_diameter),
                    drill: drill.unwrap_or(rules.via_drill),
                    from: LayerId::TOP_COPPER,
                    to: LayerId::BOTTOM_COPPER,
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
            Command::FlipSide { ids, axis_x } => {
                if let Some(bad) = ids.iter().find(|id| !matches!(id, ObjectId::Footprint(_))) {
                    return Err(CommandError::Invalid(format!("only footprints can change sides, got {bad:?}")));
                }
                let t = Transform::new(Point::new(2 * axis_x, 0), Angle::ZERO, true);
                transform_objects(tx, &ids, t, Edit::Flip)
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
            Command::Update { objects } => {
                for o in objects {
                    check_references(tx.project(), &o)?;
                    if let Object::Footprint(f) = &o {
                        let current = tx.project().board.footprints.get(f.id);
                        if current.is_some_and(|c| c.component != f.component) {
                            return Err(CommandError::Forbidden("a footprint's component cannot change".into()));
                        }
                    }
                    tx.replace(o)?;
                }
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
        None => sch.first_sheet().ok_or_else(|| CommandError::Invalid("schematic has no sheet".into())),
    }
}

fn import_parts(tx: &mut Transaction, library: &Library, parts: &[Id]) -> Result<(), CommandError> {
    for &part_id in parts {
        let (mut part, symbols, footprints) =
            library.extract_part(part_id).ok_or(CommandError::NotFound(ObjectId::LibPart(part_id)))?;
        part.source = source_of(library, part_id);
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

fn source_of(library: &Library, part_id: Id) -> Option<PartSource> {
    library.id.map(|lib| PartSource {
        library: lib,
        library_name: library.name.clone(),
        part: part_id,
        content_hash: library.content_hash(part_id).unwrap_or_default(),
    })
}

fn update_parts(tx: &mut Transaction, library: &Library, parts: &[Id]) -> Result<(), CommandError> {
    for &part_id in parts {
        let (mut part, symbols, footprints) =
            library.extract_part(part_id).ok_or(CommandError::NotFound(ObjectId::LibPart(part_id)))?;
        let old = tx
            .project()
            .library
            .parts
            .get(part_id)
            .cloned()
            .ok_or(CommandError::NotFound(ObjectId::LibPart(part_id)))?;
        part.source = source_of(library, part_id);

        // Placed units must still exist.
        let components: Vec<Component> =
            tx.project().schematic.components.iter().filter(|c| c.part == part_id).cloned().collect();
        for c in &components {
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
            .map(|v| v.footprint)
            .filter(|&f| !lib.parts.iter().any(|p| p.variants.iter().any(|v| v.footprint == f)))
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
    let part = tx.project().library.parts.get(part_id).ok_or(CommandError::NotFound(ObjectId::LibPart(part_id)))?;
    let variant = match variant {
        Some(v) if part.variant(v).is_some() => v,
        Some(_) => return Err(CommandError::Invalid("variant does not belong to the part".into())),
        None => part.variants.first().ok_or_else(|| CommandError::Invalid("part has no variant".into()))?.id,
    };
    let refdes = match refdes {
        Some(r) => {
            if tx.project().schematic.component_by_refdes(&r).is_some() {
                return Err(CommandError::Invalid(format!("reference designator {r} is already used")));
            }
            r
        }
        None => next_refdes(tx.project(), &part.prefix),
    };
    let component = Component {
        id: Id::new(),
        part: part_id,
        variant,
        refdes,
        value: value.unwrap_or_else(|| part.default_value.clone()),
        properties: Default::default(),
    };
    let symbol = SymbolInstance { id: Id::new(), sheet, component: component.id, unit: 0, pos, rotation, mirror };
    // Multi-unit parts: only unit 0 is placed for now (see PLAN.md).
    tx.insert(Object::Component(component))?;
    tx.insert(Object::Symbol(symbol))?;
    Ok(())
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
    if !keep_connected {
        return transform_objects(tx, ids, t, edit);
    }
    use crate::rubber_band::{Kind, follow};
    let wire_points = connection_points(tx.project(), ids, Kind::Wire);
    let bus_points = connection_points(tx.project(), ids, Kind::Bus);
    transform_objects(tx, ids, t, edit)?;
    follow(tx, Kind::Wire, ids, &wire_points, &t)?;
    follow(tx, Kind::Bus, ids, &bus_points, &t)
}

/// Points of schematic objects where wires (or bus wires) attach: (sheet, point).
fn connection_points(p: &Project, ids: &[ObjectId], kind: crate::rubber_band::Kind) -> Vec<(Id, Point)> {
    use crate::rubber_band::Kind;
    let s = &p.schematic;
    let mut out = Vec::new();
    for &id in ids {
        match (kind, id) {
            (Kind::Wire, ObjectId::Symbol(i)) => {
                if let Some(inst) = s.symbols.get(i) {
                    out.extend(crate::query::pin_positions(p, i).into_iter().map(|(_, _, pt)| (inst.sheet, pt)));
                }
            }
            (Kind::Wire, ObjectId::Wire(i)) => {
                out.extend(s.wires.get(i).into_iter().flat_map(|w| [(w.sheet, w.a), (w.sheet, w.b)]))
            }
            (Kind::Bus, ObjectId::BusWire(i)) => {
                out.extend(s.bus_wires.get(i).into_iter().flat_map(|w| [(w.sheet, w.a), (w.sheet, w.b)]))
            }
            (Kind::Wire, ObjectId::BusEntry(i)) => out.extend(s.bus_entries.get(i).map(|e| (e.sheet, e.wire_end()))),
            (Kind::Bus, ObjectId::BusEntry(i)) => out.extend(s.bus_entries.get(i).map(|e| (e.sheet, e.pos))),
            (_, ObjectId::Label(i)) => out.extend(s.labels.get(i).map(|l| (l.sheet, l.pos))),
            (_, ObjectId::Junction(i)) => out.extend(s.junctions.get(i).map(|j| (j.sheet, j.pos))),
            _ => {}
        }
    }
    out.sort_by_key(|&(s, p)| (s, p.x, p.y));
    out.dedup();
    out
}

fn transform_objects(tx: &mut Transaction, ids: &[ObjectId], t: Transform, edit: Edit) -> Result<(), CommandError> {
    let mut seen = std::collections::HashSet::new();
    for &id in ids {
        if !seen.insert(id) {
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
                }
                Object::Via(v) => {
                    reject_mirror_on_board(edit)?;
                    v.pos = t.point(v.pos);
                }
                Object::Zone(z) => {
                    reject_mirror_on_board(edit)?;
                    z.outline.iter_mut().for_each(|p| *p = t.point(*p));
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
            ObjectId::Footprint(_) => {
                return Err(CommandError::Forbidden(
                    "footprints follow the schematic; delete the component there".into(),
                ));
            }
            ObjectId::Sheet(s) => {
                let sch = &tx.project().schematic;
                if sch.sheets.len() == 1 {
                    return Err(CommandError::Forbidden("the last sheet cannot be deleted".into()));
                }
                let used = sch.symbols.iter().any(|o| o.sheet == s)
                    || sch.wires.iter().any(|o| o.sheet == s)
                    || sch.bus_wires.iter().any(|o| o.sheet == s)
                    || sch.bus_entries.iter().any(|o| o.sheet == s)
                    || sch.labels.iter().any(|o| o.sheet == s)
                    || sch.junctions.iter().any(|o| o.sheet == s)
                    || sch.texts.iter().any(|o| o.sheet == s);
                if used {
                    return Err(CommandError::Forbidden("only empty sheets can be deleted".into()));
                }
                tx.remove(id)?;
            }
            ObjectId::Component(c) => delete_component(tx, c)?,
            ObjectId::Symbol(s) => {
                let component = tx.project().schematic.symbols.get(s).unwrap().component;
                tx.remove(id)?;
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
                if tx.project().library.parts.iter().any(|p| p.variants.iter().any(|v| v.footprint == f)) {
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
        tx.remove(ObjectId::Symbol(s))?;
    }
    tx.remove(ObjectId::Component(component))?;
    Ok(())
}

/// Check that a raw object only references things that exist.
fn check_references(p: &Project, o: &Object) -> Result<(), CommandError> {
    let missing = |id: ObjectId| Err(CommandError::NotFound(id));
    let sheet_ok = |s: Id| p.schematic.sheets.contains(s);
    match o {
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
            for v in &part.variants {
                if !p.library.footprints.contains(v.footprint) {
                    return missing(ObjectId::LibFootprint(v.footprint));
                }
            }
            Ok(())
        }
        _ => Ok(()),
    }
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
                || board.graphics.iter().any(|g| g.layer == id)
                || board.texts.iter().any(|t| t.layer == id)
        })
        .map(|&id| board.layers.iter().find(|l| l.id == id).map_or(format!("{id:?}"), |l| l.name.clone()))
        .collect();
    if !used.is_empty() {
        return Err(CommandError::Invalid(format!("{} still has copper on it", used.join(", "))));
    }
    let mut setup = BoardSetup::of(tx.project());
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
