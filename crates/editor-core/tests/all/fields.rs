//! Reference and value texts of placed symbols (`SymbolField`).

use editor_core::{Command, Editor, clip};
use model::builtin::{self, PART_RESISTOR};
use model::*;

fn placed() -> (Editor, Id) {
    let mut ed = Editor::new(Project::new("T"));
    ed.execute(Command::import_parts(&builtin::builtin_library(), &[PART_RESISTOR]).unwrap()).unwrap();
    let c = ed
        .execute(Command::PlacePart {
            part: PART_RESISTOR,
            variant: None,
            sheet: None,
            pos: Point::mm(10.0, 10.0),
            rotation: Angle::ZERO,
            mirror: false,
            refdes: None,
            value: None,
        })
        .unwrap();
    let s = c.ids().find_map(|id| if let ObjectId::Symbol(s) = id { Some(s) } else { None }).unwrap();
    (ed, s)
}

fn fields(ed: &Editor, s: Id) -> Vec<SymbolField> {
    ed.project().schematic.fields.iter().filter(|f| f.symbol == s).cloned().collect()
}

#[test]
fn placing_a_symbol_copies_its_reference_and_value() {
    let (mut ed, s) = placed();
    let f = fields(&ed, s);
    assert_eq!(f.len(), 2);
    assert!(f.iter().any(|f| f.text.text == "${REF}") && f.iter().any(|f| f.text.text == "${VALUE}"));
    assert!(ed.project().validate().is_empty(), "{:?}", ed.project().validate());
    // Deleting one is refused; deleting the symbol takes them along.
    assert!(ed.execute(Command::Delete { ids: vec![ObjectId::SymbolField(f[0].id)] }).is_err());
    ed.execute(Command::Delete { ids: vec![ObjectId::Symbol(s)] }).unwrap();
    assert!(ed.project().schematic.fields.is_empty());
    ed.undo().unwrap();
    assert_eq!(fields(&ed, s).len(), 2);
}

#[test]
fn fields_move_with_their_symbol_and_on_their_own() {
    let (mut ed, s) = placed();
    let before = fields(&ed, s);
    // With the symbol: unchanged in symbol coordinates.
    ed.execute(Command::Move { ids: vec![ObjectId::Symbol(s)], delta: Point::mm(5.0, 0.0), keep_connected: true })
        .unwrap();
    assert_eq!(fields(&ed, s), before);
    // On their own, in the symbol's frame (here turned by 90°).
    ed.execute(Command::Rotate {
        ids: vec![ObjectId::Symbol(s)],
        center: Point::mm(15.0, 10.0),
        angle: Angle::DEG90,
        keep_connected: false,
    })
    .unwrap();
    let f = &fields(&ed, s)[0];
    ed.execute(Command::Move {
        ids: vec![ObjectId::SymbolField(f.id)],
        delta: Point::mm(1.0, 0.0),
        keep_connected: true,
    })
    .unwrap();
    let moved = ed.project().schematic.fields.get(f.id).unwrap();
    // World +x is symbol −y after a quarter turn.
    assert_eq!(moved.text.pos - f.text.pos, Point::mm(0.0, -1.0));
    // A field and its symbol selected together move once.
    let both = vec![ObjectId::Symbol(s), ObjectId::SymbolField(f.id)];
    let at = ed.project().schematic.fields.get(f.id).unwrap().text.pos;
    ed.execute(Command::Move { ids: both, delta: Point::mm(2.0, 2.0), keep_connected: false }).unwrap();
    assert_eq!(ed.project().schematic.fields.get(f.id).unwrap().text.pos, at);
}

#[test]
fn copied_symbols_keep_their_field_positions() {
    let (mut ed, s) = placed();
    let f = fields(&ed, s)[0].clone();
    ed.execute(Command::Move {
        ids: vec![ObjectId::SymbolField(f.id)],
        delta: Point::mm(3.0, 0.0),
        keep_connected: false,
    })
    .unwrap();
    let moved = ed.project().schematic.fields.get(f.id).unwrap().text.pos;
    let (clip, skipped) =
        clip::copy(ed.project(), &[ObjectId::Symbol(s), ObjectId::SymbolField(f.id)], Point::ZERO).unwrap();
    assert_eq!(skipped, 0, "a selected field goes with its symbol");
    ed.execute(Command::Paste { clip, sheet: None, at: Point::mm(0.0, 20.0) }).unwrap();
    let copy = ed.project().schematic.symbols.iter().find(|x| x.id != s).unwrap().id;
    let copied = fields(&ed, copy);
    assert_eq!(copied.len(), 2);
    assert!(copied.iter().any(|c| c.text.text == f.text.text && c.text.pos == moved));
}
