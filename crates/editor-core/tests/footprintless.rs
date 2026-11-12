//! Components without a footprint (parts without variants, or variant `None`): they stay
//! off the board, and sync follows variant changes.

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

/// A copy of the LED without footprint variants ("Logo").
fn logo_bundle() -> PartBundle {
    let lib = builtin::builtin_library();
    let mut b = lib.bundles(&[PART_LED]).unwrap().remove(0);
    b.part.id = Id::new();
    b.part.name = "Logo".into();
    b.part.variants.clear();
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

fn set_variant(ed: &mut Editor, id: Id, variant: Option<Id>) {
    ed.execute(Command::SetComponent { id, refdes: None, value: None, variant: Some(variant) }).unwrap();
}

#[test]
fn part_without_variants_places_off_board() {
    let mut ed = editor();
    let logo = logo_bundle();
    let part = logo.part.id;
    ed.execute(Command::ImportParts { parts: vec![logo] }).unwrap();
    let c = place(&mut ed, part, 0.0);
    let p = ed.project();
    assert_eq!(p.schematic.components.get(c).unwrap().variant, None);
    assert!(p.board.footprint_of(c).is_none());
    assert_eq!(p.schematic.symbols_of(c).count(), 1);
    assert_eq!(p.validate(), Vec::<String>::new());
}

#[test]
fn variant_changes_add_and_remove_the_placement() {
    let mut ed = editor();
    let r1 = place(&mut ed, PART_RESISTOR, 0.0);
    let fp = ed.project().board.footprint_of(r1).unwrap().clone();
    let v0603 = ed.project().schematic.components.get(r1).unwrap().variant;
    ed.execute(Command::AddTrack {
        layer: LayerId::TOP_COPPER,
        points: vec![fp.pos, fp.pos + Point::mm(5.0, 0.0)],
        width: None,
    })
    .unwrap();
    let before = files(ed.project());

    // To no footprint: the placement goes, the track stays.
    set_variant(&mut ed, r1, None);
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
    let p = ed.project();
    let again = p.board.footprint_of(r1).unwrap();
    assert_ne!(again.id, fp.id);
    assert_eq!(p.validate(), Vec::<String>::new());
}

#[test]
fn set_component_json_tells_null_from_absent() {
    let mut ed = editor();
    let r1 = place(&mut ed, PART_RESISTOR, 0.0);
    ed.execute_json(&format!(r#"{{"command": "set_component", "id": "{r1}", "value": "1k"}}"#)).unwrap();
    assert!(ed.project().board.footprint_of(r1).is_some(), "absent variant: unchanged");
    ed.execute_json(&format!(r#"{{"command": "set_component", "id": "{r1}", "variant": null}}"#)).unwrap();
    assert!(ed.project().board.footprint_of(r1).is_none(), "null variant: no footprint");
    assert_eq!(ed.project().schematic.components.get(r1).unwrap().value, "1k");
}

#[test]
fn update_without_variants_takes_components_off_the_board() {
    let mut lib = builtin::builtin_library();
    let mut ed = Editor::new(Project::new("t"));
    ed.execute(Command::import_parts(&lib, &[PART_RESISTOR]).unwrap()).unwrap();
    let r1 = place(&mut ed, PART_RESISTOR, 0.0);
    let before = files(ed.project());

    lib.parts.get_mut(PART_RESISTOR).unwrap().variants.clear();
    let changes = ed.execute(Command::update_parts(&lib, &[PART_RESISTOR]).unwrap()).unwrap();
    assert!(changes.ids().any(|id| matches!(id, ObjectId::Footprint(_))), "the change set reports the footprint");
    let p = ed.project();
    assert_eq!(p.schematic.components.get(r1).unwrap().variant, None);
    assert!(p.board.footprint_of(r1).is_none());
    assert!(p.library.footprints.is_empty(), "unused footprints dropped");
    assert_eq!(p.validate(), Vec::<String>::new());

    ed.undo();
    assert_eq!(files(ed.project()), before);
}

#[test]
fn files_omit_a_missing_variant() {
    let mut ed = editor();
    let r1 = place(&mut ed, PART_RESISTOR, 0.0);
    set_variant(&mut ed, r1, None);
    let sch = to_file_string(FileKind::Schematic, &ed.project().schematic).unwrap();
    assert!(!sch.contains("\"variant\""));
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("t.pcbproj");
    save_project(ed.project(), &path).unwrap();
    let loaded = load_project(&path).unwrap();
    assert_eq!(&loaded, ed.project());
    assert_eq!(loaded.schematic.components.get(r1).unwrap().variant, None);
}
