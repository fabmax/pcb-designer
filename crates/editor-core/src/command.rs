//! Serializable editing commands. The GUI, scripts and MCP clients all go through these.

use serde::{Deserialize, Serialize};

use model::*;

use crate::CommandError;
use crate::transaction::Transaction;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(tag = "command", rename_all = "snake_case")]
pub enum Command {
    /// Copy parts (with their symbols and footprints) from a library into the project.
    ImportParts {
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
    /// Polyline of wire segments.
    AddWire {
        #[serde(default)]
        sheet: Option<Id>,
        points: Vec<Point>,
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
    AddVia {
        pos: Point,
        #[serde(default)]
        diameter: Option<Coord>,
        #[serde(default)]
        drill: Option<Coord>,
    },
    Move {
        ids: Vec<ObjectId>,
        delta: Point,
    },
    Rotate {
        ids: Vec<ObjectId>,
        center: Point,
        angle: Angle,
    },
    /// Mirror schematic objects across the vertical line `x = axis_x`.
    Mirror {
        ids: Vec<ObjectId>,
        axis_x: Coord,
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
            Command::PlacePart { .. } => "Place part".into(),
            Command::AddWire { .. } => "Add wire".into(),
            Command::AddJunction { .. } => "Add junction".into(),
            Command::AddLabel { name, .. } => format!("Add label {name}"),
            Command::SetComponent { .. } => "Edit component".into(),
            Command::AddTrack { .. } => "Add track".into(),
            Command::AddVia { .. } => "Add via".into(),
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
            Command::PlacePart { part, variant, sheet, pos, rotation, mirror, refdes, value } => {
                place_part(tx, part, variant, sheet, pos, rotation, mirror, refdes, value)
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
            Command::AddJunction { sheet, pos } => {
                let sheet = resolve_sheet(tx, sheet)?;
                tx.insert(Object::Junction(Junction { id: Id::new(), sheet, pos })).map(drop)
            }
            Command::AddLabel { sheet, pos, name, kind } => {
                if name.trim().is_empty() {
                    return Err(CommandError::Invalid("label name must not be empty".into()));
                }
                let sheet = resolve_sheet(tx, sheet)?;
                let label = NetLabel { id: Id::new(), sheet, pos, name, kind, rotation: Angle::ZERO };
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
            Command::Move { ids, delta } => {
                transform_objects(tx, &ids, Transform::new(delta, Angle::ZERO, false), Edit::Transform)
            }
            Command::Rotate { ids, center, angle } => {
                let offset = center - center.rotated(angle);
                transform_objects(tx, &ids, Transform::new(offset, angle, false), Edit::Transform)
            }
            Command::Mirror { ids, axis_x } => {
                let t = Transform::new(Point::new(2 * axis_x, 0), Angle::ZERO, true);
                transform_objects(tx, &ids, t, Edit::Mirror)
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
        let hash = library.content_hash(part_id).unwrap_or_default();
        part.source = library.id.map(|lib| PartSource {
            library: lib,
            library_name: library.name.clone(),
            part: part_id,
            content_hash: hash,
        });
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
