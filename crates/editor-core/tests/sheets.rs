//! Hierarchical sheets: adding, renaming, moving and deleting them.

use editor_core::{Command, Editor};
use model::builtin::{self, PART_RESISTOR};
use model::*;

fn add_sheet(ed: &mut Editor, parent: Option<Id>, name: &str, x: f64) -> Result<Id, String> {
    let cmd = Command::AddSheet { parent, name: name.into(), pos: Point::mm(x, 0.0), size: None };
    let changes = ed.execute(cmd).map_err(|e| e.to_string())?;
    Ok(changes.ids().find_map(|id| if let ObjectId::Sheet(s) = id { Some(s) } else { None }).unwrap())
}

fn sheet(ed: &Editor, id: Id) -> Sheet {
    ed.project().schematic.sheets.get(id).unwrap().clone()
}

#[test]
fn sheets_form_a_tree_with_unique_names() {
    let mut ed = Editor::new(Project::new("t"));
    let root = ed.project().schematic.root_sheet().unwrap();
    let power = add_sheet(&mut ed, None, "Power", 0.0).unwrap();
    let reg = add_sheet(&mut ed, Some(power), "Regulator", 0.0).unwrap();
    let io = add_sheet(&mut ed, Some(root), "IO", 40.0).unwrap();
    let sch = &ed.project().schematic;
    assert_eq!(sheet(&ed, power).parent, Some(root), "below the root by default");
    assert_eq!(sheet(&ed, power).size, Sheet::SIZE);
    assert_eq!(sch.sheet_path(root), "");
    assert_eq!(sch.sheet_path(reg), "/Power/Regulator");
    assert_eq!(sch.find_sheet("/Power/Regulator"), Some(reg));
    assert_eq!(sch.find_sheet("/"), Some(root));
    assert_eq!(sch.find_sheet("IO"), Some(io));
    assert_eq!(sch.find_sheet(&power.to_string()), Some(power));
    assert_eq!(sch.sheet_and_descendants(power), [power, reg]);
    assert_eq!(ed.project().validate(), Vec::<String>::new());

    // Names: unique among siblings (a sub-sheet may repeat a name elsewhere), no '/'.
    assert!(add_sheet(&mut ed, None, "Power", 80.0).unwrap_err().contains("already"));
    add_sheet(&mut ed, Some(power), "IO", 40.0).unwrap();
    assert!(add_sheet(&mut ed, None, "A/B", 80.0).is_err());
    assert!(add_sheet(&mut ed, None, " ", 80.0).is_err());

    // Renaming is an update; moving to another parent isn't allowed.
    let mut s = sheet(&ed, reg);
    s.name = "LDO".into();
    ed.execute(Command::Update { objects: vec![Object::Sheet(s.clone())] }).unwrap();
    assert_eq!(ed.project().schematic.sheet_path(reg), "/Power/LDO");
    s.parent = Some(root);
    assert!(ed.execute(Command::Update { objects: vec![Object::Sheet(s)] }).is_err());
    let mut s = sheet(&ed, io);
    s.name = "Power".into();
    assert!(ed.execute(Command::Update { objects: vec![Object::Sheet(s)] }).is_err());
}

#[test]
fn deleting_a_sheet_takes_its_contents_along() {
    let mut ed = Editor::new(Project::new("t"));
    ed.execute(Command::ImportParts { library: builtin::builtin_library(), parts: vec![PART_RESISTOR] }).unwrap();
    let power = add_sheet(&mut ed, None, "Power", 0.0).unwrap();
    let reg = add_sheet(&mut ed, Some(power), "Regulator", 0.0).unwrap();
    let place = |ed: &mut Editor, sheet: Id| {
        let cmd = Command::PlacePart {
            part: PART_RESISTOR,
            variant: None,
            sheet: Some(sheet),
            pos: Point::mm(10.0, 10.0),
            rotation: Angle::ZERO,
            mirror: false,
            refdes: None,
            value: None,
        };
        ed.execute(cmd).unwrap();
    };
    place(&mut ed, power);
    place(&mut ed, reg);
    ed.execute(Command::AddWire { sheet: Some(reg), points: vec![Point::mm(0.0, 0.0), Point::mm(5.0, 0.0)] }).unwrap();
    let root = ed.project().schematic.root_sheet().unwrap();
    place(&mut ed, root);
    let before = ed.project().clone();
    assert_eq!(ed.project().board.footprints.len(), 3);

    ed.execute(Command::Delete { ids: vec![ObjectId::Sheet(power)] }).unwrap();
    let p = ed.project();
    assert_eq!(p.schematic.sheets.len(), 1);
    assert_eq!(p.schematic.components.len(), 1, "only the root's resistor is left");
    assert_eq!(p.board.footprints.len(), 1, "the board follows");
    assert!(p.schematic.wires.is_empty());
    assert_eq!(p.validate(), Vec::<String>::new());
    ed.undo();
    assert_eq!(ed.project(), &before, "one undo step");

    let err = ed.execute(Command::Delete { ids: vec![ObjectId::Sheet(root)] }).unwrap_err();
    assert!(err.to_string().contains("root"), "{err}");
}

#[test]
fn sheet_symbols_move_and_turn() {
    let mut ed = Editor::new(Project::new("t"));
    let s = add_sheet(&mut ed, None, "A", 0.0).unwrap();
    let ids = vec![ObjectId::Sheet(s)];
    ed.execute(Command::Move { ids: ids.clone(), delta: Point::mm(10.0, 5.0), keep_connected: false }).unwrap();
    assert_eq!(sheet(&ed, s).pos, Point::mm(10.0, 5.0));
    ed.execute(Command::Rotate { ids, center: Point::mm(10.0, 5.0), angle: Angle::DEG90, keep_connected: false })
        .unwrap();
    let r = sheet(&ed, s);
    assert_eq!((r.size.w, r.size.h), (Sheet::SIZE.h, Sheet::SIZE.w), "stays an upright rectangle");
    let root = ed.project().schematic.root_sheet().unwrap();
    let ids = vec![ObjectId::Sheet(root)];
    assert!(ed.execute(Command::Move { ids, delta: Point::mm(1.0, 0.0), keep_connected: false }).is_err());
}
