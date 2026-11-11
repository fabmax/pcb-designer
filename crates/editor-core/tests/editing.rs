use editor_core::query::{pad_positions, pin_pad_names, pin_positions};
use editor_core::{Command, CommandError, Editor};
use model::builtin::{self, PART_CAPACITOR, PART_HEADER_1X4, PART_LED, PART_RESISTOR};
use model::io::{FileKind, load_project, save_project, to_file_string};
use model::*;

/// Byte-exact file representation of a project (what would be written to disk).
fn files(p: &Project) -> [String; 3] {
    [
        to_file_string(FileKind::Library, &p.library).unwrap(),
        to_file_string(FileKind::Schematic, &p.schematic).unwrap(),
        to_file_string(FileKind::Board, &p.board).unwrap(),
    ]
}

fn editor_with_parts() -> Editor {
    let mut ed = Editor::new(Project::new("test"));
    ed.execute(Command::ImportParts {
        library: builtin::builtin_library(),
        parts: vec![PART_RESISTOR, PART_CAPACITOR, PART_LED, PART_HEADER_1X4],
    })
    .unwrap();
    ed
}

fn place(ed: &mut Editor, part: Id, x: f64, y: f64) -> Id {
    let changes = ed
        .execute(Command::PlacePart {
            part,
            variant: None,
            sheet: None,
            pos: Point::mm(x, y),
            rotation: Angle::ZERO,
            mirror: false,
            refdes: None,
            value: None,
        })
        .unwrap();
    changes.ids().find_map(|id| if let ObjectId::Component(c) = id { Some(c) } else { None }).unwrap()
}

#[test]
fn import_records_source_and_is_idempotent() {
    let mut ed = editor_with_parts();
    let part = ed.project().library.parts.get(PART_RESISTOR).unwrap();
    let src = part.source.as_ref().unwrap();
    assert_eq!(src.library, builtin::LIBRARY_ID);
    assert_eq!(src.content_hash, builtin::builtin_library().content_hash(PART_RESISTOR).unwrap());
    // Importing again changes nothing.
    let again =
        ed.execute(Command::ImportParts { library: builtin::builtin_library(), parts: vec![PART_RESISTOR] }).unwrap();
    assert!(again.is_empty());
    assert_eq!(ed.project().validate(), Vec::<String>::new());
}

#[test]
fn placing_a_part_creates_component_symbol_and_footprint() {
    let mut ed = editor_with_parts();
    let r1 = place(&mut ed, PART_RESISTOR, 10.16, 0.0);
    let r2 = place(&mut ed, PART_RESISTOR, 20.32, 0.0);
    let p = ed.project();
    assert_eq!(p.schematic.components.get(r1).unwrap().refdes, "R1");
    assert_eq!(p.schematic.components.get(r2).unwrap().refdes, "R2");
    assert_eq!(p.schematic.components.get(r1).unwrap().value, "10k");
    // Footprints were created by the sync, stacked in the staging column.
    let f1 = p.board.footprint_of(r1).unwrap();
    let f2 = p.board.footprint_of(r2).unwrap();
    assert_eq!(f1.pos.x, f2.pos.x);
    assert!(f2.pos.y < f1.pos.y);
    assert_eq!(p.validate(), Vec::<String>::new());
}

#[test]
fn undo_reverts_schematic_and_board_byte_for_byte() {
    let mut ed = editor_with_parts();
    place(&mut ed, PART_RESISTOR, 0.0, 0.0);
    let before = files(ed.project());
    let led = place(&mut ed, PART_LED, 10.0, 0.0);
    ed.execute(Command::AddWire { sheet: None, points: vec![Point::mm(5.08, 0.0), Point::mm(4.92, 0.0)] }).unwrap();
    let after = files(ed.project());
    assert!(ed.project().board.footprint_of(led).is_some());

    ed.undo();
    ed.undo();
    assert_eq!(files(ed.project()), before);
    assert!(ed.project().board.footprint_of(led).is_none(), "undo also removed the footprint");

    ed.redo();
    ed.redo();
    assert_eq!(files(ed.project()), after);
}

#[test]
fn deleting_the_symbol_removes_component_and_footprint_and_undo_restores_order() {
    let mut ed = editor_with_parts();
    let ids: Vec<Id> = (0..4).map(|i| place(&mut ed, PART_CAPACITOR, i as f64 * 10.0, 0.0)).collect();
    let before = files(ed.project());
    let sym = ed.project().schematic.symbols_of(ids[1]).next().unwrap().id;
    ed.execute(Command::Delete { ids: vec![ObjectId::Symbol(sym)] }).unwrap();
    assert!(!ed.project().schematic.components.contains(ids[1]));
    assert!(ed.project().board.footprint_of(ids[1]).is_none());
    assert_eq!(ed.project().validate(), Vec::<String>::new());
    ed.undo();
    assert_eq!(files(ed.project()), before, "order of all collections restored");
}

#[test]
fn deleting_component_and_its_symbol_together_is_fine() {
    let mut ed = editor_with_parts();
    let c = place(&mut ed, PART_RESISTOR, 0.0, 0.0);
    let sym = ed.project().schematic.symbols_of(c).next().unwrap().id;
    ed.execute(Command::Delete { ids: vec![ObjectId::Component(c), ObjectId::Symbol(sym)] }).unwrap();
    assert!(ed.project().schematic.components.is_empty());
}

#[test]
fn footprints_cannot_be_deleted_or_inserted_directly() {
    let mut ed = editor_with_parts();
    let c = place(&mut ed, PART_RESISTOR, 0.0, 0.0);
    let fp = ed.project().board.footprint_of(c).unwrap().clone();
    assert!(matches!(
        ed.execute(Command::Delete { ids: vec![ObjectId::Footprint(fp.id)] }),
        Err(CommandError::Forbidden(_))
    ));
    let copy = FootprintInstance { id: Id::new(), ..fp };
    assert!(matches!(
        ed.execute(Command::Insert { objects: vec![Object::Footprint(copy)] }),
        Err(CommandError::Forbidden(_))
    ));
}

#[test]
fn failed_command_changes_nothing() {
    let mut ed = editor_with_parts();
    let c = place(&mut ed, PART_RESISTOR, 0.0, 0.0);
    let before = files(ed.project());
    let sym = ed.project().schematic.symbols_of(c).next().unwrap().id;
    // First id is valid and gets deleted, second does not exist → whole command rolls back.
    let err = ed.execute(Command::Delete { ids: vec![ObjectId::Symbol(sym), ObjectId::Wire(Id::new())] });
    assert!(matches!(err, Err(CommandError::NotFound(_))));
    assert_eq!(files(ed.project()), before);
    assert_eq!(ed.history().undo_label(), Some("Place part"));
}

#[test]
fn rotate_and_mirror_move_pins_correctly() {
    let mut ed = editor_with_parts();
    let c = place(&mut ed, PART_RESISTOR, 10.0, 0.0);
    let sym = ed.project().schematic.symbols_of(c).next().unwrap().id;
    let pins = |ed: &Editor| -> Vec<Point> { pin_positions(ed.project(), sym).into_iter().map(|p| p.2).collect() };
    assert_eq!(pins(&ed), vec![Point::mm(4.92, 0.0), Point::mm(15.08, 0.0)]);

    ed.execute(Command::Rotate {
        ids: vec![ObjectId::Symbol(sym)],
        center: Point::mm(10.0, 0.0),
        angle: Angle::DEG90,
        keep_connected: false,
    })
    .unwrap();
    assert_eq!(pins(&ed), vec![Point::mm(10.0, -5.08), Point::mm(10.0, 5.08)]);

    // Mirror across x = 0: symbol moves to x = -10, pins stay vertical.
    ed.execute(Command::Mirror { ids: vec![ObjectId::Symbol(sym)], axis_x: 0, keep_connected: false }).unwrap();
    assert_eq!(pins(&ed), vec![Point::mm(-10.0, -5.08), Point::mm(-10.0, 5.08)]);
    // Mirroring twice is the identity.
    ed.execute(Command::Mirror { ids: vec![ObjectId::Symbol(sym)], axis_x: 0, keep_connected: false }).unwrap();
    assert_eq!(pins(&ed), vec![Point::mm(10.0, -5.08), Point::mm(10.0, 5.08)]);
}

#[test]
fn footprint_flip_mirrors_pads() {
    let mut ed = editor_with_parts();
    let c = place(&mut ed, PART_LED, 0.0, 0.0);
    let fp = ed.project().board.footprint_of(c).unwrap().id;
    let pads = |ed: &Editor| pad_positions(ed.project(), fp);
    let x0 = ed.project().board.footprints.get(fp).unwrap().pos.x;
    let before = pads(&ed);
    ed.execute(Command::FlipSide { ids: vec![ObjectId::Footprint(fp)], axis_x: x0 }).unwrap();
    let after = pads(&ed);
    assert_eq!(ed.project().board.footprints.get(fp).unwrap().side, Side::Bottom);
    // Pad 1 moved from the left to the right of the footprint origin.
    assert_eq!(after[0].2.x - x0, -(before[0].2.x - x0));
}

#[test]
fn led_pins_map_to_named_pads() {
    let mut ed = editor_with_parts();
    let c = place(&mut ed, PART_LED, 0.0, 0.0);
    let map = pin_pad_names(ed.project(), c);
    assert_eq!(map, vec![("A".to_string(), vec!["2".to_string()]), ("K".to_string(), vec!["1".to_string()])]);
}

#[test]
fn changing_the_variant_keeps_the_placement() {
    let mut ed = editor_with_parts();
    let c = place(&mut ed, PART_RESISTOR, 0.0, 0.0);
    let fp_before = ed.project().board.footprint_of(c).unwrap().clone();
    let v0805 = ed.project().library.parts.get(PART_RESISTOR).unwrap().variants[1].id;
    ed.execute(Command::SetComponent { id: c, refdes: None, value: Some("4k7".into()), variant: Some(v0805) }).unwrap();
    assert_eq!(ed.project().board.footprint_of(c).unwrap(), &fp_before);
    let fp = editor_core::query::footprint_of_instance(ed.project(), &fp_before).unwrap();
    assert_eq!(fp.name, "0805");
    // Variants of other parts are rejected.
    let led_variant = ed.project().library.parts.get(PART_LED).unwrap().variants[0].id;
    assert!(
        ed.execute(Command::SetComponent { id: c, refdes: None, value: None, variant: Some(led_variant) }).is_err()
    );
}

#[test]
fn commands_roundtrip_through_json() {
    let mut ed = editor_with_parts();
    let json = format!(r#"{{"command": "place_part", "part": "{PART_LED}", "pos": [0, 5080000], "refdes": "D7"}}"#);
    ed.execute_json(&json).unwrap();
    assert!(ed.project().schematic.component_by_refdes("D7").is_some());
    ed.execute_json(r#"{"command": "add_label", "pos": [0, 0], "name": "GND", "kind": "power"}"#).unwrap();
    ed.execute_json(r#"{"command": "add_track", "layer": 0, "points": [[0, 0], [1000000, 0], [2000000, 1000000]]}"#)
        .unwrap();
    assert_eq!(ed.project().board.tracks.len(), 2);
    assert!(ed.execute_json(r#"{"command": "add_track", "layer": 100, "points": [[0, 0], [1, 0]]}"#).is_err());
    // Every command serializes and parses back unchanged.
    let cmd = Command::Move { ids: vec![ObjectId::Wire(Id::new())], delta: Point::mm(1.0, 2.0), keep_connected: false };
    let text = serde_json::to_string(&cmd).unwrap();
    assert_eq!(serde_json::from_str::<Command>(&text).unwrap(), cmd);
}

#[test]
fn modified_flag_follows_save_point() {
    let mut ed = editor_with_parts();
    ed.mark_saved();
    assert!(!ed.is_modified());
    place(&mut ed, PART_RESISTOR, 0.0, 0.0);
    assert!(ed.is_modified());
    ed.undo();
    assert!(!ed.is_modified());
    ed.redo();
    assert!(ed.is_modified());
}

/// Deterministic pseudo random numbers for the sequence test.
struct Rng(u64);
impl Rng {
    fn next(&mut self, n: usize) -> usize {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 % n as u64) as usize
    }
}

#[test]
fn random_command_sequences_undo_and_redo_exactly() {
    for seed in 1..=5u64 {
        let mut ed = editor_with_parts();
        let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15));
        let start = files(ed.project());
        let parts = [PART_RESISTOR, PART_CAPACITOR, PART_LED, PART_HEADER_1X4];
        let mut applied = 0;
        for _ in 0..200 {
            let p = ed.project();
            let symbols: Vec<Id> = p.schematic.symbols.ids().collect();
            let wires: Vec<Id> = p.schematic.wires.ids().collect();
            let footprints: Vec<Id> = p.board.footprints.ids().collect();
            let pt = |rng: &mut Rng| Point::mm(rng.next(100) as f64 * 1.27, rng.next(100) as f64 * 1.27);
            let cmd = match rng.next(7) {
                0 | 1 => Command::PlacePart {
                    part: parts[rng.next(4)],
                    variant: None,
                    sheet: None,
                    pos: pt(&mut rng),
                    rotation: Angle::deg(90.0 * rng.next(4) as f64),
                    mirror: rng.next(2) == 1,
                    refdes: None,
                    value: None,
                },
                2 => Command::AddWire { sheet: None, points: vec![pt(&mut rng), pt(&mut rng), pt(&mut rng)] },
                3 if !symbols.is_empty() => Command::Move {
                    ids: vec![ObjectId::Symbol(symbols[rng.next(symbols.len())])],
                    delta: pt(&mut rng),
                    keep_connected: true,
                },
                4 if !symbols.is_empty() || !wires.is_empty() => {
                    let mut ids = Vec::new();
                    if !symbols.is_empty() {
                        ids.push(ObjectId::Symbol(symbols[rng.next(symbols.len())]));
                    }
                    if !wires.is_empty() {
                        ids.push(ObjectId::Wire(wires[rng.next(wires.len())]));
                    }
                    Command::Delete { ids }
                }
                5 if !footprints.is_empty() => Command::Rotate {
                    ids: vec![ObjectId::Footprint(footprints[rng.next(footprints.len())])],
                    center: pt(&mut rng),
                    angle: Angle::DEG90,
                    keep_connected: false,
                },
                _ => Command::AddLabel {
                    sheet: None,
                    pos: pt(&mut rng),
                    name: "GND".into(),
                    kind: LabelKind::Power,
                    shape: LabelShape::default(),
                    rotation: Angle::ZERO,
                },
            };
            if ed.execute(cmd).is_ok() {
                applied += 1;
            }
            // Occasionally undo/redo in between.
            if rng.next(10) == 0 {
                ed.undo();
                ed.redo();
            }
        }
        assert!(applied > 150, "seed {seed}: only {applied} commands applied");
        assert_eq!(ed.project().validate(), Vec::<String>::new(), "seed {seed}");
        let end = files(ed.project());
        while ed.undo().is_some() {}
        // Undo everything except the initial import.
        ed.redo();
        assert_eq!(files(ed.project()), start, "seed {seed}: undo all");
        while ed.redo().is_some() {}
        assert_eq!(files(ed.project()), end, "seed {seed}: redo all");
    }
}

#[test]
fn project_folder_roundtrip() {
    let mut ed = editor_with_parts();
    place(&mut ed, PART_RESISTOR, 0.0, 0.0);
    place(&mut ed, PART_LED, 10.0, 0.0);
    ed.execute(Command::AddWire { sheet: None, points: vec![Point::mm(5.08, 0.0), Point::mm(4.92, 0.0)] }).unwrap();
    ed.execute(Command::AddVia { pos: Point::mm(3.0, 3.0), diameter: None, drill: None }).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("demo.pcbproj");
    save_project(ed.project(), &path).unwrap();
    let loaded = load_project(&path).unwrap();
    assert_eq!(&loaded, ed.project());
    assert_eq!(loaded.validate(), Vec::<String>::new());
    // Saving the loaded project produces identical files.
    let again = dir.path().join("again.pcbproj");
    save_project(&loaded, &again).unwrap();
    for f in ["project.json", "library.json", "schematic.json", "board.json"] {
        assert_eq!(std::fs::read(path.join(f)).unwrap(), std::fs::read(again.join(f)).unwrap(), "{f}");
    }
}

#[test]
fn grouped_moves_form_one_undo_step() {
    let mut ed = editor_with_parts();
    let c = place(&mut ed, PART_RESISTOR, 0.0, 0.0);
    let before = files(ed.project());
    let sym = ed.project().schematic.symbols_of(c).next().unwrap().id;
    for _ in 0..10 {
        let cmd = Command::Move { ids: vec![ObjectId::Symbol(sym)], delta: Point::mm(1.0, 0.0), keep_connected: false };
        ed.execute_grouped(cmd, Some(7)).unwrap();
    }
    ed.close_group();
    assert_eq!(ed.project().schematic.symbols.get(sym).unwrap().pos, Point::mm(10.0, 0.0));
    ed.undo();
    assert_eq!(files(ed.project()), before);
    assert_eq!(ed.history().undo_label(), Some("Place part"));
}
