//! Copying sheet symbols and board parts, duplicating, and moving objects between sheets.

use editor_core::{Command, Editor, clip};
use model::builtin::{self, PART_MOUNTING_HOLE, PART_RESISTOR};
use model::*;

fn editor() -> Editor {
    let mut ed = Editor::new(Project::new("T"));
    let parts = [PART_RESISTOR, PART_MOUNTING_HOLE];
    ed.execute(Command::import_parts(&builtin::builtin_library(), &parts).unwrap()).unwrap();
    ed
}

fn place(ed: &mut Editor, sheet: Option<Id>, x: f64) -> Id {
    let c = ed
        .execute(Command::PlacePart {
            part: PART_RESISTOR,
            variant: None,
            sheet,
            pos: Point::mm(x, 50.0),
            rotation: Angle::ZERO,
            mirror: false,
            refdes: None,
            value: None,
        })
        .unwrap();
    c.ids().find_map(|id| if let ObjectId::Symbol(s) = id { Some(s) } else { None }).unwrap()
}

fn add_sheet(ed: &mut Editor, parent: Option<Id>, name: &str) -> Id {
    let c =
        ed.execute(Command::AddSheet { parent, name: name.into(), pos: Point::mm(100.0, 100.0), size: None }).unwrap();
    c.ids().find_map(|id| if let ObjectId::Sheet(s) = id { Some(s) } else { None }).unwrap()
}

fn sheet_named(ed: &Editor, path: &str) -> Id {
    ed.project().schematic.find_sheet(path).unwrap_or_else(|| panic!("no sheet {path}"))
}

#[test]
fn a_copied_sheet_symbol_copies_its_tree() {
    let mut ed = editor();
    let amp = add_sheet(&mut ed, None, "amp");
    let inner = add_sheet(&mut ed, Some(amp), "bias");
    place(&mut ed, Some(amp), 50.0);
    place(&mut ed, Some(inner), 60.0);
    ed.execute(Command::AddLabel {
        sheet: Some(amp),
        pos: Point::mm(40.0, 40.0),
        name: "IN".into(),
        kind: LabelKind::Hierarchical,
        shape: LabelShape::Input,
        style: None,
        rotation: Angle::ZERO,
    })
    .unwrap();
    let footprints = ed.project().board.footprints.len();

    ed.execute(Command::Duplicate { ids: vec![ObjectId::Sheet(amp)], delta: Point::mm(0.0, -40.0) }).unwrap();
    let p = ed.project();
    let copy = sheet_named(&ed, "/amp2");
    let copy_inner = sheet_named(&ed, "/amp2/bias");
    assert_eq!(p.schematic.sheets.get(copy).unwrap().pos, Point::mm(100.0, 60.0), "moved by delta");
    // The contents keep their places on their own pages.
    let on = |s: Id| p.schematic.symbols.iter().filter(|x| x.sheet == s).map(|x| x.pos).collect::<Vec<_>>();
    assert_eq!(on(copy), vec![Point::mm(50.0, 50.0)]);
    assert_eq!(on(copy_inner), vec![Point::mm(60.0, 50.0)]);
    assert_eq!(p.schematic.hier_ports(copy).len(), 1, "the copy has the sheet pin");
    // New components with new references, and their footprints on the board.
    let refs: Vec<&str> = p.schematic.components.iter().map(|c| c.refdes.as_str()).collect();
    assert_eq!(refs, ["R1", "R2", "R3", "R4"]);
    assert_eq!(p.board.footprints.len(), footprints + 2);
    assert!(p.validate().is_empty(), "{:?}", p.validate());
    // Undo takes it all back.
    ed.undo();
    assert_eq!(ed.project().schematic.sheets.len(), 3);
    assert_eq!(ed.project().schematic.components.len(), 2);
}

#[test]
fn board_parts_are_copied_on_the_board() {
    let mut ed = editor();
    let c = ed
        .execute(Command::PlaceBoardPart {
            part: PART_MOUNTING_HOLE,
            variant: None,
            pos: Point::mm(5.0, 5.0),
            rotation: Angle::ZERO,
            side: Side::Top,
            refdes: None,
            value: None,
        })
        .unwrap();
    let fp = c.ids().find_map(|id| if let ObjectId::Footprint(f) = id { Some(f) } else { None }).unwrap();
    place(&mut ed, None, 10.0);
    let schematic_fp =
        ed.project().board.footprints.iter().find(|f| f.id != fp).map(|f| ObjectId::Footprint(f.id)).unwrap();

    // Footprints of schematic parts are left out.
    let (clip, skipped) = clip::copy(ed.project(), &[ObjectId::Footprint(fp), schematic_fp], Point::ZERO).unwrap();
    assert_eq!((clip.content.len(), skipped), (1, 1));
    // Into another project: the part comes along.
    let mut other = Editor::new(Project::new("O"));
    other.execute(Command::Paste { clip, sheet: None, at: Point::mm(20.0, 0.0) }).unwrap();
    let p = other.project();
    assert_eq!(p.board.footprints.iter().next().unwrap().pos, Point::mm(25.0, 5.0));
    assert_eq!(p.board.fields.len(), 1, "its reference text came along");
    assert_eq!(p.schematic.components.iter().next().unwrap().refdes, "H1", "a free reference is kept");

    // Duplicated in place: the next reference.
    ed.execute(Command::Duplicate { ids: vec![ObjectId::Footprint(fp)], delta: Point::mm(5.0, 0.0) }).unwrap();
    assert!(ed.project().schematic.component_by_refdes("H2").is_some());
    assert!(ed.project().validate().is_empty());
}

#[test]
fn objects_move_between_sheets_keeping_their_ids() {
    let mut ed = editor();
    let sub = add_sheet(&mut ed, None, "power");
    let s = place(&mut ed, None, 50.0);
    let pin = editor_core::query::pin_positions(ed.project(), s)[0].2;
    ed.execute(Command::AddNoConnect { sheet: None, pos: pin }).unwrap();
    let footprints: Vec<Id> = ed.project().board.footprints.iter().map(|f| f.id).collect();

    ed.execute(Command::MoveToSheet { ids: vec![ObjectId::Symbol(s)], sheet: sub }).unwrap();
    let p = ed.project();
    assert_eq!(p.schematic.symbols.get(s).unwrap().sheet, sub);
    assert!(p.schematic.no_connects.iter().all(|n| n.sheet == sub), "the flag on its pin goes along");
    assert_eq!(p.board.footprints.iter().map(|f| f.id).collect::<Vec<_>>(), footprints, "the footprint stays");
    assert_eq!(p.schematic.components.iter().next().unwrap().refdes, "R1");
    assert!(ed.execute(Command::MoveToSheet { ids: vec![ObjectId::Symbol(s)], sheet: sub }).is_err(), "already there");
}

#[test]
fn sheets_are_re_parented_and_renamed_on_a_clash() {
    let mut ed = editor();
    let a = add_sheet(&mut ed, None, "a");
    let b = add_sheet(&mut ed, None, "b");
    let a_inner = add_sheet(&mut ed, Some(a), "b");
    // Moving a below its own sub-sheet is refused.
    assert!(ed.execute(Command::MoveToSheet { ids: vec![ObjectId::Sheet(a)], sheet: a_inner }).is_err());
    assert!(ed.execute(Command::MoveToSheet { ids: vec![ObjectId::Sheet(a)], sheet: a }).is_err());
    // b moves below a, next to a's own "b".
    ed.execute(Command::MoveToSheet { ids: vec![ObjectId::Sheet(b)], sheet: a }).unwrap();
    assert_eq!(sheet_named(&ed, "/a/b2"), b);
    assert_eq!(sheet_named(&ed, "/a/b"), a_inner);
    assert!(ed.project().schematic.sheet_issues().is_empty());
}
