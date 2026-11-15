//! No-connect flags: pins meant to stay open, unconnected pins as problems, and flags
//! following their symbol.

use editor_core::connectivity::Connectivity;
use editor_core::query::pin_positions;
use editor_core::{Command, Editor};
use model::builtin::{self, PART_RESISTOR};
use model::io::{FileKind, to_file_string};
use model::*;

fn files(p: &Project) -> [String; 2] {
    [to_file_string(FileKind::Library, &p.library).unwrap(), to_file_string(FileKind::Schematic, &p.schematic).unwrap()]
}

fn editor() -> Editor {
    let mut ed = Editor::new(Project::new("t"));
    ed.execute(Command::import_parts(&builtin::builtin_library(), &[PART_RESISTOR]).unwrap()).unwrap();
    ed
}

/// Place a resistor; returns its symbol and its two pin ends.
fn resistor(ed: &mut Editor, x: f64) -> (Id, Point, Point) {
    let changes = ed
        .execute(Command::PlacePart {
            part: PART_RESISTOR,
            variant: None,
            sheet: None,
            pos: Point::mm(x, 0.0),
            rotation: Angle::ZERO,
            mirror: false,
            refdes: None,
            value: None,
        })
        .unwrap();
    let s = changes.ids().find_map(|id| if let ObjectId::Symbol(s) = id { Some(s) } else { None }).unwrap();
    let pins = pin_positions(ed.project(), s);
    (s, pins[0].2, pins[1].2)
}

fn flag(ed: &mut Editor, pos: Point) -> Id {
    let changes = ed.execute(Command::AddNoConnect { sheet: None, pos }).unwrap();
    changes.ids().find_map(|id| if let ObjectId::NoConnect(n) = id { Some(n) } else { None }).unwrap()
}

fn problems(ed: &Editor) -> Vec<String> {
    Connectivity::build(ed.project()).problems.into_iter().map(|p| p.message).collect()
}

#[test]
fn flags_mark_pins_as_open_on_purpose() {
    let mut ed = editor();
    let (r1, a, b) = resistor(&mut ed, 0.0);
    assert_eq!(problems(&ed), ["Unconnected pin R1-1", "Unconnected pin R1-2"]);

    flag(&mut ed, a);
    let c = Connectivity::build(ed.project());
    assert_eq!(c.unconnected.len(), 1, "only the other end is still open");
    assert_eq!(c.unconnected[0].2, b);
    assert_eq!(c.flagged.len(), 1);
    assert_eq!(c.flagged[0].0, r1);
    assert_eq!(problems(&ed), ["Unconnected pin R1-2"]);

    // One flag per point.
    assert!(ed.execute(Command::AddNoConnect { sheet: None, pos: a }).is_err());
}

#[test]
fn misplaced_flags_are_problems() {
    let mut ed = editor();
    let (_, a, b) = resistor(&mut ed, 0.0);
    flag(&mut ed, Point::mm(20.0, 20.0));
    // On a wired pin.
    ed.execute(Command::AddWire { sheet: None, points: vec![b, b + Point::mm(5.08, 0.0)] }).unwrap();
    flag(&mut ed, b);
    flag(&mut ed, a);
    let mut p = problems(&ed);
    p.sort();
    assert_eq!(p, ["No-connect flag is not on a pin", "No-connect flag on a connected pin"]);
}

#[test]
fn not_connected_pins_need_no_flag() {
    let mut ed = editor();
    resistor(&mut ed, 0.0);
    let part = ed.project().library.parts.get(PART_RESISTOR).unwrap();
    let mut sym = ed.project().library.symbols.get(part.units[0].symbol).unwrap().clone();
    let first = sym.pins.iter().next().unwrap().id;
    sym.pins.get_mut(first).unwrap().kind = PinKind::NotConnected;
    ed.execute(Command::Update { objects: vec![Object::LibSymbol(sym)] }).unwrap();
    assert_eq!(problems(&ed), ["Unconnected pin R1-2"]);
}

#[test]
fn flags_move_and_go_with_their_symbol() {
    let mut ed = editor();
    let (r1, a, _) = resistor(&mut ed, 0.0);
    let f = flag(&mut ed, a);
    let stray = flag(&mut ed, Point::mm(20.0, 20.0));

    let delta = Point::mm(10.16, 5.08);
    ed.execute(Command::Move { ids: vec![ObjectId::Symbol(r1)], delta, keep_connected: false }).unwrap();
    assert_eq!(ed.project().schematic.no_connects.get(f).unwrap().pos, a + delta);
    let center = ed.project().schematic.symbols.get(r1).unwrap().pos;
    ed.execute(Command::Rotate { ids: vec![ObjectId::Symbol(r1)], center, angle: Angle::DEG90, keep_connected: true })
        .unwrap();
    let pins = pin_positions(ed.project(), r1);
    assert_eq!(ed.project().schematic.no_connects.get(f).unwrap().pos, pins[0].2, "still on pin 1");
    assert_eq!(ed.project().schematic.no_connects.get(stray).unwrap().pos, Point::mm(20.0, 20.0));
    assert_eq!(problems(&ed), ["No-connect flag is not on a pin", "Unconnected pin R1-2"]);

    let before = files(ed.project());
    ed.execute(Command::Delete { ids: vec![ObjectId::Symbol(r1)] }).unwrap();
    assert!(!ed.project().schematic.no_connects.contains(f));
    assert!(ed.project().schematic.no_connects.contains(stray));
    ed.undo();
    assert_eq!(files(ed.project()), before);

    // Deleting the symbol and its flag together works too.
    ed.execute(Command::Delete { ids: vec![ObjectId::Symbol(r1), ObjectId::NoConnect(f)] }).unwrap();
    assert!(ed.project().schematic.no_connects.len() == 1);
}
