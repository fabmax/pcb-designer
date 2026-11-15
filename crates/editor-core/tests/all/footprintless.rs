//! "No footprint" part variants: components using one stay off the board, and sync
//! follows variant changes and part edits.

use editor_core::{Command, Editor};
use model::builtin::{self, PART_LED, PART_RESISTOR};
use model::io::{FileKind, load_project, save_project, to_file_string};
use model::*;

fn files(p: &Project) -> [String; 3] {
    [
        to_file_string(FileKind::Library, &p.library).unwrap(),
        to_file_string(FileKind::Schematic, &p.schematic).unwrap(),
        to_file_string(FileKind::Board, &p.board).unwrap(),
    ]
}

fn editor() -> Editor {
    let mut ed = Editor::new(Project::new("t"));
    ed.execute(Command::import_parts(&builtin::builtin_library(), &[PART_RESISTOR, PART_LED]).unwrap()).unwrap();
    ed
}

/// A copy of the LED with only a "no footprint" variant ("Logo").
fn logo_bundle() -> PartBundle {
    let lib = builtin::builtin_library();
    let mut b = lib.bundles(&[PART_LED]).unwrap().remove(0);
    b.part.id = Id::new();
    b.part.name = "Logo".into();
    b.part.variants = vec![PartVariant::no_footprint(Id::new())];
    b.footprints.clear();
    b
}

fn place(ed: &mut Editor, part: Id, x: f64) -> Id {
    let changes = ed
        .execute(Command::PlacePart {
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
    changes.ids().find_map(|id| if let ObjectId::Component(c) = id { Some(c) } else { None }).unwrap()
}

/// Gives the project's resistor a "no footprint" variant; returns its id.
fn add_no_footprint_variant(ed: &mut Editor) -> Id {
    let mut part = ed.project().library.parts.get(PART_RESISTOR).unwrap().clone();
    let v = PartVariant::no_footprint(Id::new());
    let id = v.id;
    part.variants.push(v);
    ed.execute(Command::Update { objects: vec![Object::LibPart(part)] }).unwrap();
    id
}

fn set_variant(ed: &mut Editor, id: Id, variant: Id) {
    ed.execute(Command::SetComponent { id, refdes: None, value: None, variant: Some(variant) }).unwrap();
}

#[test]
fn a_no_footprint_part_places_off_board() {
    let mut ed = editor();
    let logo = logo_bundle();
    let part = logo.part.id;
    ed.execute(Command::ImportParts { parts: vec![logo] }).unwrap();
    let c = place(&mut ed, part, 0.0);
    let p = ed.project();
    assert!(p.board.footprint_of(c).is_none());
    assert_eq!(p.schematic.symbols_of(c).count(), 1);
    assert_eq!(p.validate(), Vec::<String>::new());
}

#[test]
fn variant_changes_add_and_remove_the_placement() {
    let mut ed = editor();
    let r1 = place(&mut ed, PART_RESISTOR, 0.0);
    let none = add_no_footprint_variant(&mut ed);
    let fp = ed.project().board.footprint_of(r1).unwrap().clone();
    let v0603 = ed.project().schematic.components.get(r1).unwrap().variant;
    let track = vec![fp.pos, fp.pos + Point::mm(5.0, 0.0)];
    ed.execute(Command::AddTrack { layer: LayerId::TOP_COPPER, points: track, width: None }).unwrap();
    let before = files(ed.project());

    // To the "no footprint" variant: the placement goes, the track stays.
    set_variant(&mut ed, r1, none);
    let p = ed.project();
    assert!(p.board.footprint_of(r1).is_none());
    assert_eq!(p.board.tracks.len(), 1);
    assert_eq!(p.validate(), Vec::<String>::new());

    // One undo step restores footprint and position exactly; redo removes it again.
    ed.undo();
    assert_eq!(files(ed.project()), before);
    ed.redo();
    assert!(ed.project().board.footprint_of(r1).is_none());

    // Back to a footprint: a new placement in the staging area.
    set_variant(&mut ed, r1, v0603);
    let again = ed.project().board.footprint_of(r1).unwrap();
    assert_ne!(again.id, fp.id);
    assert_eq!(ed.project().validate(), Vec::<String>::new());
}

#[test]
fn part_edits_that_drop_a_footprint_move_components_off_the_board() {
    let mut ed = editor();
    let r1 = place(&mut ed, PART_RESISTOR, 0.0);
    let r2 = place(&mut ed, PART_RESISTOR, 10.0);
    let before = files(ed.project());

    // The variant both use loses its footprint.
    let mut part = ed.project().library.parts.get(PART_RESISTOR).unwrap().clone();
    let used = ed.project().schematic.components.get(r1).unwrap().variant;
    let v = part.variants.iter_mut().find(|v| v.id == used).unwrap();
    v.footprint = None;
    v.connections.clear();
    let restore = part.clone();
    ed.execute(Command::Update { objects: vec![Object::LibPart(part)] }).unwrap();
    let p = ed.project();
    assert!(p.board.footprint_of(r1).is_none() && p.board.footprint_of(r2).is_none());
    assert_eq!(p.validate(), Vec::<String>::new());

    ed.undo();
    assert_eq!(files(ed.project()), before);
    ed.redo();
    // And back: both get a placement again.
    let mut part = restore;
    let lib = builtin::builtin_library();
    let original = lib.parts.get(PART_RESISTOR).unwrap().variants.iter().find(|v| v.id == used).unwrap().clone();
    *part.variants.iter_mut().find(|v| v.id == used).unwrap() = original;
    ed.execute(Command::Update { objects: vec![Object::LibPart(part)] }).unwrap();
    let p = ed.project();
    assert!(p.board.footprint_of(r1).is_some() && p.board.footprint_of(r2).is_some());
    assert_eq!(p.validate(), Vec::<String>::new());
}

#[test]
fn library_update_to_a_no_footprint_part() {
    let mut lib = builtin::builtin_library();
    let mut ed = Editor::new(Project::new("t"));
    ed.execute(Command::import_parts(&lib, &[PART_RESISTOR]).unwrap()).unwrap();
    let r1 = place(&mut ed, PART_RESISTOR, 0.0);
    let before = files(ed.project());

    let none = PartVariant::no_footprint(Id::new());
    lib.parts.get_mut(PART_RESISTOR).unwrap().variants = vec![none.clone()];
    ed.execute(Command::update_parts(&lib, &[PART_RESISTOR]).unwrap()).unwrap();
    let p = ed.project();
    assert_eq!(p.schematic.components.get(r1).unwrap().variant, none.id, "fell back to the first variant");
    assert!(p.board.footprint_of(r1).is_none());
    assert!(p.library.footprints.is_empty(), "unused footprints dropped");
    assert_eq!(p.validate(), Vec::<String>::new());

    ed.undo();
    assert_eq!(files(ed.project()), before);
}

#[test]
fn no_footprint_variants_have_no_connections() {
    let mut p = Project::new("t");
    let lib = builtin::builtin_library();
    let mut b = lib.bundles(&[PART_RESISTOR]).unwrap().remove(0);
    let v = &mut b.part.variants[0];
    v.footprint = None;
    for s in b.symbols {
        p.library.symbols.insert(s);
    }
    for f in b.footprints {
        p.library.footprints.insert(f);
    }
    p.library.parts.insert(b.part);
    assert!(p.library.validate().iter().any(|i| i.contains("pads without footprint")), "{:?}", p.library.validate());
}

#[test]
fn files_omit_a_missing_footprint() {
    let mut ed = editor();
    let r1 = place(&mut ed, PART_RESISTOR, 0.0);
    let none = add_no_footprint_variant(&mut ed);
    set_variant(&mut ed, r1, none);
    let lib = to_file_string(FileKind::Library, &ed.project().library).unwrap();
    // Resistor 0603 and 0805, LED 0603; the "no footprint" variant has no field.
    assert_eq!(lib.matches("\"footprint\"").count(), 3);
    assert!(!lib.contains("\"footprint\": null"));
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("t.pcbproj");
    save_project(ed.project(), &path).unwrap();
    let loaded = load_project(&path).unwrap();
    assert_eq!(&loaded, ed.project());
}
