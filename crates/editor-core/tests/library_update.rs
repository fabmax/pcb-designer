use editor_core::query::{PartState, part_statuses};
use editor_core::{Command, Editor};
use model::builtin::{self, PART_LED, PART_RESISTOR};
use model::*;

fn place(ed: &mut Editor, part: Id, x: f64) -> Id {
    ed.execute(Command::PlacePart {
        part,
        variant: None,
        sheet: None,
        pos: Point::mm(x, 0.0),
        rotation: Angle::ZERO,
        mirror: false,
        refdes: None,
        value: None,
    })
    .unwrap();
    ed.project().schematic.components.iter().last().unwrap().id
}

#[test]
fn update_parts_from_changed_library() {
    let mut lib = builtin::builtin_library();
    let mut ed = Editor::new(Project::new("t"));
    ed.execute(Command::import_parts(&lib, &[PART_RESISTOR, PART_LED]).unwrap()).unwrap();
    let r1 = place(&mut ed, PART_RESISTOR, 0.0);
    let original = ed.project().clone();
    assert!(part_statuses(ed.project(), &[&lib]).iter().all(|s| s.state == PartState::UpToDate));
    assert!(part_statuses(ed.project(), &[]).iter().all(|s| s.state == PartState::LibraryMissing));

    // Library edits: bigger pads on the 0805 footprint, the 0603 variant (in use) removed,
    // and a brand-new symbol.
    let part = lib.parts.get(PART_RESISTOR).unwrap().clone();
    let v0805 = part.variants.iter().find(|v| v.name == "0805").unwrap().clone();
    let fp = lib.footprints.get_mut(v0805.footprint).unwrap();
    let pad_ids: Vec<Id> = fp.pads.ids().collect();
    for id in &pad_ids {
        fp.pads.get_mut(*id).unwrap().size = Size::mm(1.5, 1.6);
    }
    let old_symbol = part.units[0].symbol;
    let mut sym = lib.symbols.get(old_symbol).unwrap().clone();
    sym.id = Id::new();
    sym.name = "Resistor (US)".into();
    let new_symbol = sym.id;
    lib.symbols.insert(sym);
    let p = lib.parts.get_mut(PART_RESISTOR).unwrap();
    p.variants.retain(|v| v.name == "0805");
    p.units[0].symbol = new_symbol;

    let statuses = part_statuses(ed.project(), &[&lib]);
    let r = statuses.iter().find(|s| s.part == PART_RESISTOR).unwrap();
    assert_eq!(r.state, PartState::LibraryChanged);
    assert_eq!(statuses.iter().find(|s| s.part == PART_LED).unwrap().state, PartState::UpToDate);

    ed.execute(Command::update_parts(&lib, &[PART_RESISTOR]).unwrap()).unwrap();
    let p = ed.project();
    assert_eq!(p.validate(), Vec::<String>::new());
    assert!(part_statuses(p, &[&lib]).iter().all(|s| s.state == PartState::UpToDate));
    let c = p.schematic.components.get(r1).unwrap();
    assert_eq!(c.variant, Some(v0805.id), "component moved to the remaining variant");
    let fp = p.library.footprint_of(c.part, c.variant).unwrap();
    assert!(fp.pads.iter().all(|pad| pad.size == Size::mm(1.5, 1.6)));
    assert!(p.library.symbols.contains(new_symbol));
    assert!(!p.library.symbols.contains(old_symbol), "unused old symbol removed");

    // One undo step restores the project exactly.
    ed.undo();
    assert_eq!(ed.project(), &original);
}

#[test]
fn update_refuses_to_drop_placed_units() {
    let mut lib = builtin::builtin_library();
    let mut ed = Editor::new(Project::new("t"));
    ed.execute(Command::import_parts(&lib, &[PART_RESISTOR]).unwrap()).unwrap();
    place(&mut ed, PART_RESISTOR, 0.0);
    lib.parts.get_mut(PART_RESISTOR).unwrap().units.clear();
    let err = ed.execute(Command::update_parts(&lib, &[PART_RESISTOR]).unwrap()).unwrap_err();
    assert!(err.to_string().contains("unit"), "{err}");
}

#[test]
fn local_edits_are_told_apart_from_library_changes() {
    let mut lib = builtin::builtin_library();
    let mut ed = Editor::new(Project::new("t"));
    ed.execute(Command::import_parts(&lib, &[PART_RESISTOR, PART_LED]).unwrap()).unwrap();
    let state = |ed: &Editor, lib: &Library, part| {
        part_statuses(ed.project(), &[lib]).into_iter().find(|s| s.part == part).unwrap().state
    };

    // Edit the project's copy of the resistor.
    let mut r = ed.project().library.parts.get(PART_RESISTOR).unwrap().clone();
    r.description = "Edited in the project".into();
    ed.execute(Command::Update { objects: vec![Object::LibPart(r)] }).unwrap();
    assert_eq!(state(&ed, &lib, PART_RESISTOR), PartState::ModifiedLocally);
    assert_eq!(state(&ed, &lib, PART_LED), PartState::UpToDate);

    // The library changes too: both changed. The LED only in the library.
    lib.parts.get_mut(PART_RESISTOR).unwrap().default_value = "1k".into();
    lib.parts.get_mut(PART_LED).unwrap().default_value = "GREEN".into();
    assert_eq!(state(&ed, &lib, PART_RESISTOR), PartState::BothChanged);
    assert_eq!(state(&ed, &lib, PART_LED), PartState::LibraryChanged);

    // Updating takes the library's version (and drops the local edit).
    ed.execute(Command::update_parts(&lib, &[PART_RESISTOR]).unwrap()).unwrap();
    assert_eq!(state(&ed, &lib, PART_RESISTOR), PartState::UpToDate);
}

#[test]
fn project_part_edits_keep_what_components_use() {
    let lib = builtin::builtin_library();
    let mut ed = Editor::new(Project::new("t"));
    ed.execute(Command::import_parts(&lib, &[PART_RESISTOR, PART_LED]).unwrap()).unwrap();
    let r1 = place(&mut ed, PART_RESISTOR, 0.0);
    let used = ed.project().schematic.components.get(r1).unwrap().variant.unwrap();

    // Dropping the variant R1 uses is refused; dropping another one is fine.
    let part = ed.project().library.parts.get(PART_RESISTOR).unwrap().clone();
    let mut without_used = part.clone();
    without_used.variants.retain(|v| v.id != used);
    let err = ed.execute(Command::Update { objects: vec![Object::LibPart(without_used)] }).unwrap_err();
    assert!(err.to_string().contains("R1"), "{err}");
    let mut without_other = part.clone();
    without_other.variants.retain(|v| v.id == used);
    ed.execute(Command::Update { objects: vec![Object::LibPart(without_other)] }).unwrap();

    // Removing the placed unit is refused.
    let mut no_units = ed.project().library.parts.get(PART_RESISTOR).unwrap().clone();
    no_units.units.clear();
    assert!(ed.execute(Command::Update { objects: vec![Object::LibPart(no_units)] }).is_err());
    // So is deleting the part or its symbol.
    assert!(ed.execute(Command::Delete { ids: vec![ObjectId::LibPart(PART_RESISTOR)] }).is_err());
    let symbol = part.units[0].symbol;
    assert!(ed.execute(Command::Delete { ids: vec![ObjectId::LibSymbol(symbol)] }).is_err());
}

#[test]
fn unused_library_objects_can_be_removed_in_one_step() {
    use editor_core::query::unused_library_objects;
    let lib = builtin::builtin_library();
    let mut ed = Editor::new(Project::new("t"));
    ed.execute(Command::import_parts(&lib, &[PART_RESISTOR, PART_LED]).unwrap()).unwrap();
    place(&mut ed, PART_RESISTOR, 0.0);
    let unused = unused_library_objects(ed.project());
    assert_eq!(unused[0], ObjectId::LibPart(PART_LED));
    assert!(unused.iter().all(|id| !matches!(id, ObjectId::LibPart(p) if *p == PART_RESISTOR)));
    ed.execute(Command::Delete { ids: unused }).unwrap();
    let p = ed.project();
    assert_eq!(p.library.parts.len(), 1);
    assert_eq!(p.validate(), Vec::<String>::new());
    assert!(unused_library_objects(p).is_empty());
}

#[test]
fn only_connecting_library_edits_affect_connectivity() {
    let lib = builtin::builtin_library();
    let mut ed = Editor::new(Project::new("t"));
    ed.execute(Command::import_parts(&lib, &[PART_RESISTOR]).unwrap()).unwrap();
    let affects = |ed: &mut Editor, o: Object| {
        let changes = ed.execute(Command::Update { objects: vec![o] }).unwrap();
        changes.changes.iter().any(|c| c.affects_connectivity())
    };
    let part = ed.project().library.parts.get(PART_RESISTOR).unwrap().clone();
    let mut sym = ed.project().library.symbols.get(part.units[0].symbol).unwrap().clone();

    let mut renamed = part.clone();
    renamed.name = "R".into();
    renamed.description = "renamed".into();
    assert!(!affects(&mut ed, Object::LibPart(renamed.clone())));
    let mut unmapped = renamed;
    unmapped.variants[0].connections.clear();
    assert!(affects(&mut ed, Object::LibPart(unmapped)));

    sym.graphics.clear();
    assert!(!affects(&mut ed, Object::LibSymbol(sym.clone())));
    let pin = sym.pins.ids().next().unwrap();
    sym.pins.get_mut(pin).unwrap().name = "X".into();
    assert!(affects(&mut ed, Object::LibSymbol(sym)));
}
