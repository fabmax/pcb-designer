//! Board parts: parts without symbol, placed and deleted on the board only; their pads
//! have a net of their own.

use editor_core::connectivity::Connectivity;
use editor_core::copper::BoardConnectivity;
use editor_core::drc::{self, ViolationKind};
use editor_core::query::pad_positions;
use editor_core::{Command, CommandError, Editor};
use model::builtin::{self, PART_LED, PART_RESISTOR};
use model::io::{FileKind, to_file_string};
use model::*;

fn files(p: &Project) -> [String; 3] {
    [
        to_file_string(FileKind::Library, &p.library).unwrap(),
        to_file_string(FileKind::Schematic, &p.schematic).unwrap(),
        to_file_string(FileKind::Board, &p.board).unwrap(),
    ]
}

/// The LED's footprint as a board part "Fiducial" (prefix FID, no units, no connections).
fn fiducial() -> PartBundle {
    let lib = builtin::builtin_library();
    let mut b = lib.bundles(&[PART_LED]).unwrap().remove(0);
    b.part.id = Id::new();
    b.part.name = "Fiducial".into();
    b.part.prefix = "FID".into();
    b.part.units.clear();
    b.symbols.clear();
    for v in &mut b.part.variants {
        v.connections.clear();
    }
    b
}

fn editor() -> (Editor, Id) {
    let mut ed = Editor::new(Project::new("t"));
    ed.execute(Command::import_parts(&builtin::builtin_library(), &[PART_RESISTOR]).unwrap()).unwrap();
    let b = fiducial();
    let id = b.part.id;
    ed.execute(Command::ImportParts { parts: vec![b] }).unwrap();
    (ed, id)
}

fn place_board(ed: &mut Editor, part: Id, at: Point) -> Result<(Id, Id), CommandError> {
    let changes = ed.execute(Command::PlaceBoardPart {
        part,
        variant: None,
        pos: at,
        rotation: Angle::DEG90,
        side: Side::Bottom,
        refdes: None,
        value: None,
    })?;
    let c = changes.ids().find_map(|id| if let ObjectId::Component(c) = id { Some(c) } else { None }).unwrap();
    let f = changes.ids().find_map(|id| if let ObjectId::Footprint(f) = id { Some(f) } else { None }).unwrap();
    Ok((c, f))
}

fn place_sch(ed: &mut Editor, part: Id) -> Result<(), CommandError> {
    ed.execute(Command::PlacePart {
        part,
        variant: None,
        sheet: None,
        pos: Point::ZERO,
        rotation: Angle::ZERO,
        mirror: false,
        refdes: None,
        value: None,
    })
    .map(drop)
}

#[test]
fn board_parts_are_placed_on_the_board() {
    let (mut ed, fid) = editor();
    let before = files(ed.project());
    let (c, f) = place_board(&mut ed, fid, Point::mm(5.0, 5.0)).unwrap();
    let p = ed.project();
    let comp = p.schematic.components.get(c).unwrap();
    assert_eq!(comp.refdes, "FID1");
    assert_eq!(p.schematic.symbols_of(c).count(), 0);
    let fp = p.board.footprints.get(f).unwrap();
    assert_eq!((fp.pos, fp.rotation, fp.side), (Point::mm(5.0, 5.0), Angle::DEG90, Side::Bottom));
    assert_eq!(p.board.footprints.len(), 1, "no extra placement from sync");
    assert_eq!(p.validate(), Vec::<String>::new());
    let after = files(ed.project());
    ed.undo();
    assert_eq!(files(ed.project()), before);
    ed.redo();
    assert_eq!(files(ed.project()), after);
    // The next one takes the next number.
    let (c2, _) = place_board(&mut ed, fid, Point::mm(10.0, 5.0)).unwrap();
    assert_eq!(ed.project().schematic.components.get(c2).unwrap().refdes, "FID2");
}

#[test]
fn each_kind_has_its_own_place_command() {
    let (mut ed, fid) = editor();
    let err = place_sch(&mut ed, fid).unwrap_err();
    assert!(err.to_string().contains("board part"), "{err}");
    let err = place_board(&mut ed, PART_RESISTOR, Point::ZERO).unwrap_err();
    assert!(err.to_string().contains("has a symbol"), "{err}");
    assert!(ed.project().schematic.components.is_empty());
}

#[test]
fn deleting_a_board_part_footprint_deletes_its_component() {
    let (mut ed, fid) = editor();
    place_sch(&mut ed, PART_RESISTOR).unwrap();
    let before = files(ed.project());
    let (c, f) = place_board(&mut ed, fid, Point::mm(5.0, 5.0)).unwrap();
    let placed = files(ed.project());
    ed.execute(Command::Delete { ids: vec![ObjectId::Footprint(f)] }).unwrap();
    assert!(!ed.project().schematic.components.contains(c));
    assert_eq!(files(ed.project()), before);
    ed.undo();
    assert_eq!(files(ed.project()), placed);

    // Footprints of schematic parts still follow the schematic.
    let r1 = ed.project().schematic.component_by_refdes("R1").unwrap().id;
    let rf = ed.project().board.footprint_of(r1).unwrap().id;
    let err = ed.execute(Command::Delete { ids: vec![ObjectId::Footprint(rf)] }).unwrap_err();
    assert!(matches!(err, CommandError::Forbidden(_)), "{err}");
}

#[test]
fn placed_parts_keep_their_kind() {
    let (mut ed, fid) = editor();
    place_sch(&mut ed, PART_RESISTOR).unwrap();
    place_board(&mut ed, fid, Point::mm(5.0, 5.0)).unwrap();

    // A placed board part can't get a symbol.
    let mut board = ed.project().library.parts.get(fid).unwrap().clone();
    let resistor = ed.project().library.parts.get(PART_RESISTOR).unwrap().clone();
    board.units = resistor.units.clone();
    let err = ed.execute(Command::Update { objects: vec![Object::LibPart(board)] }).unwrap_err();
    assert!(matches!(err, CommandError::Forbidden(_)), "{err}");
    // A part placed in the schematic can't lose all its units.
    let mut sch = resistor;
    sch.units.clear();
    for v in &mut sch.variants {
        v.connections.clear();
    }
    let err = ed.execute(Command::Update { objects: vec![Object::LibPart(sch)] }).unwrap_err();
    assert!(matches!(err, CommandError::Forbidden(_)), "{err}");
}

#[test]
fn board_parts_need_footprints() {
    let mut b = fiducial();
    b.part.variants.push(PartVariant::no_footprint(Id::new()));
    let mut lib = Library::new("t");
    for f in b.footprints {
        lib.footprints.insert(f);
    }
    lib.parts.insert(b.part);
    assert!(lib.validate().iter().any(|i| i.contains("board part")), "{:?}", lib.validate());
}

#[test]
fn copper_touching_a_board_part_pad_is_a_short() {
    let (mut ed, fid) = editor();
    place_sch(&mut ed, PART_RESISTOR).unwrap();
    let r1 = ed.project().schematic.component_by_refdes("R1").unwrap().id;
    let rf = ed.project().board.footprint_of(r1).unwrap().id;
    let (_, f) = place_board(&mut ed, fid, Point::mm(5.0, 5.0)).unwrap();
    let pad_of = |ed: &Editor, f: Id| pad_positions(ed.project(), f)[0].2;
    let check = |ed: &Editor| {
        let p = ed.project();
        drc::run(p, &BoardConnectivity::build(p, &Connectivity::build(p)))
    };
    assert!(check(&ed).iter().all(|v| v.kind != ViolationKind::Short), "nothing touches yet");
    // Flip it to the top, where the resistor's pads are, and run a track between them.
    ed.execute(Command::FlipSide {
        ids: vec![ObjectId::Footprint(f)],
        axis_x: Point::mm(5.0, 5.0).x,
        keep_connected: false,
    })
    .unwrap();
    let (a, b) = (pad_of(&ed, rf), pad_of(&ed, f));
    ed.execute(Command::AddTrack { layer: LayerId::TOP_COPPER, points: vec![a, b], width: None }).unwrap();
    let v = check(&ed);
    let shorts: Vec<_> = v.iter().filter(|v| v.kind == ViolationKind::Short).collect();
    assert_eq!(shorts.len(), 1, "{v:?}");
    assert!(shorts[0].message.contains("unconnected-(FID1-Pad"), "{}", shorts[0].message);
}

#[test]
fn copper_touching_an_unmapped_pad_is_a_short() {
    // A resistor whose second pad has no pin.
    let mut ed = Editor::new(Project::new("t"));
    let mut b = builtin::builtin_library().bundles(&[PART_RESISTOR]).unwrap().remove(0);
    let second = b.symbols[0].pins.iter().nth(1).unwrap().id;
    for v in &mut b.part.variants {
        v.connections.retain(|c| c.pin != second);
    }
    ed.execute(Command::ImportParts { parts: vec![b] }).unwrap();
    place_sch(&mut ed, PART_RESISTOR).unwrap();
    let r1 = ed.project().schematic.component_by_refdes("R1").unwrap().id;
    let f = ed.project().board.footprint_of(r1).unwrap().id;
    let pads = pad_positions(ed.project(), f);
    let p = ed.project();
    let bc = BoardConnectivity::build(p, &Connectivity::build(p));
    assert!(bc.nets.iter().any(|n| n == "unconnected-(R1-Pad2)"), "{:?}", bc.nets);

    ed.execute(Command::AddTrack { layer: LayerId::TOP_COPPER, points: vec![pads[0].2, pads[1].2], width: None })
        .unwrap();
    let p = ed.project();
    let bc = BoardConnectivity::build(p, &Connectivity::build(p));
    assert_eq!(bc.shorts.len(), 1, "{:?}", bc.shorts);
    assert!(bc.shorts[0].0.contains(&"unconnected-(R1-Pad2)".to_string()));
}
