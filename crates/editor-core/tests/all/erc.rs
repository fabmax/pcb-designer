//! Electrical rules: pin kinds, wired "not connected" pins and reused references.

use editor_core::connectivity::Connectivity;
use editor_core::query::pin_positions;
use editor_core::{Command, Editor};
use model::builtin::{self, PART_74HC00, PART_RESISTOR};
use model::*;

fn editor() -> Editor {
    let mut ed = Editor::new(Project::new("t"));
    ed.execute(Command::import_parts(&builtin::builtin_library(), &[PART_RESISTOR, PART_74HC00]).unwrap()).unwrap();
    ed
}

fn new_symbol(changes: &editor_core::transaction::ChangeSet) -> Id {
    changes.ids().find_map(|id| if let ObjectId::Symbol(s) = id { Some(s) } else { None }).unwrap()
}

/// Place a part; returns its symbol and its pins by name.
fn place(ed: &mut Editor, part: Id, x: f64) -> (Id, Vec<(String, Point)>) {
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
    let s = new_symbol(&changes);
    (s, pins(ed, s))
}

fn pins(ed: &Editor, symbol: Id) -> Vec<(String, Point)> {
    pin_positions(ed.project(), symbol).into_iter().map(|(_, name, p)| (name, p)).collect()
}

fn pin(pins: &[(String, Point)], name: &str) -> Point {
    pins.iter().find(|p| p.0 == name).unwrap().1
}

fn wire(ed: &mut Editor, a: Point, b: Point) {
    // An L so that wires never run through other pins.
    let corner = Point { x: a.x, y: b.y.max(a.y) + 20 * MM };
    let corner2 = Point { x: b.x, y: corner.y };
    ed.execute(Command::AddWire { sheet: None, points: vec![a, corner, corner2, b] }).unwrap();
}

/// Problems other than unconnected pins.
fn problems(ed: &Editor) -> Vec<String> {
    Connectivity::build(ed.project())
        .problems
        .into_iter()
        .map(|p| p.message)
        .filter(|m| !m.starts_with("Unconnected pin"))
        .collect()
}

#[test]
fn outputs_driving_each_other() {
    let mut ed = editor();
    let (_, u1) = place(&mut ed, PART_74HC00, 0.0);
    let (_, u2) = place(&mut ed, PART_74HC00, 30.0);
    wire(&mut ed, pin(&u1, "Y"), pin(&u2, "Y"));
    let p = problems(&ed);
    assert_eq!(p.len(), 1, "{p:?}");
    assert!(p[0].contains("Outputs drive against each other: U1-3 (Y), U2-3 (Y)"), "{p:?}");

    // An output driving an input is fine.
    let mut ed = editor();
    let (_, u1) = place(&mut ed, PART_74HC00, 0.0);
    let (_, u2) = place(&mut ed, PART_74HC00, 30.0);
    wire(&mut ed, pin(&u1, "Y"), pin(&u2, "A"));
    assert_eq!(problems(&ed), Vec::<String>::new());
}

#[test]
fn inputs_need_a_driver() {
    let mut ed = editor();
    let (_, u1) = place(&mut ed, PART_74HC00, 0.0);
    wire(&mut ed, pin(&u1, "A"), pin(&u1, "B"));
    let p = problems(&ed);
    assert_eq!(p.len(), 1, "{p:?}");
    assert!(p[0].ends_with("Inputs U1-1 (A), U1-2 (B) are not driven"), "{p:?}");

    // A passive pin can drive (a pull-up resistor).
    let (_, r) = place(&mut ed, PART_RESISTOR, -30.0);
    wire(&mut ed, r[1].1, pin(&u1, "A"));
    assert_eq!(problems(&ed), Vec::<String>::new());
}

#[test]
fn power_inputs_need_a_source() {
    let mut ed = editor();
    let (u1, _) = place(&mut ed, PART_74HC00, 0.0);
    let c = ed.project().schematic.symbols.get(u1).unwrap().component;
    let changes = ed
        .execute(Command::PlaceUnit {
            component: c,
            unit: 4,
            sheet: None,
            pos: Point::mm(0.0, 40.0),
            rotation: Angle::ZERO,
            mirror: false,
        })
        .unwrap();
    let power = pins(&ed, new_symbol(&changes));
    let (_, r) = place(&mut ed, PART_RESISTOR, -30.0);
    wire(&mut ed, r[0].1, pin(&power, "VCC"));
    let p = problems(&ed);
    assert_eq!(p.len(), 1, "{p:?}");
    assert!(p[0].contains("Power input U1-14 (VCC) has no power source"), "{p:?}");

    // A power port is a source.
    let vcc = pin(&power, "VCC");
    ed.execute(Command::AddLabel {
        sheet: None,
        pos: vcc,
        name: "VCC".into(),
        kind: LabelKind::Power,
        shape: LabelShape::default(),
        style: None,
        rotation: Angle::ZERO,
    })
    .unwrap();
    assert_eq!(problems(&ed), Vec::<String>::new());
}

#[test]
fn wired_not_connected_pins() {
    let mut ed = Editor::new(Project::new("t"));
    let lib = builtin::builtin_library();
    let mut bundle = lib.bundles(&[PART_RESISTOR]).unwrap().remove(0);
    bundle.symbols[0].pins.iter_mut().nth(1).unwrap().kind = PinKind::NotConnected;
    ed.execute(Command::ImportParts { parts: vec![bundle] }).unwrap();
    let (_, r1) = place(&mut ed, PART_RESISTOR, 0.0);
    // Open: meant to be (no problem, not even "unconnected").
    assert!(Connectivity::build(ed.project()).problems.iter().all(|p| !p.message.contains("R1-2")));

    let (_, r2) = place(&mut ed, PART_RESISTOR, 30.0);
    wire(&mut ed, r1[1].1, r2[0].1);
    let p = problems(&ed);
    assert_eq!(p, ["Pin R1-2 is of kind \"not connected\" but is wired"]);
}

#[test]
fn reused_references_are_reported() {
    let mut ed = editor();
    place(&mut ed, PART_RESISTOR, 0.0);
    let (r2, _) = place(&mut ed, PART_RESISTOR, 30.0);
    assert_eq!(problems(&ed), Vec::<String>::new());
    let c = ed.project().schematic.symbols.get(r2).unwrap().component;
    // SetComponent refuses a used reference; other edits (imports, raw updates) may not.
    assert!(
        ed.execute(Command::SetComponent { id: c, refdes: Some("R1".into()), value: None, variant: None }).is_err()
    );
    let mut comp = ed.project().schematic.components.get(c).unwrap().clone();
    comp.refdes = "R1".into();
    ed.execute(Command::Update { objects: vec![Object::Component(comp)] }).unwrap();
    let problems = Connectivity::build(ed.project()).problems;
    let dup = problems.iter().find(|p| p.message == "Reference R1 is used by 2 parts").expect("reported");
    assert_eq!(dup.objects.len(), 2);

    // Unannotated references are not duplicates.
    for s in ed.project().schematic.symbols.iter().map(|s| s.component).collect::<Vec<_>>() {
        ed.execute(Command::SetComponent { id: s, refdes: Some("R?".into()), value: None, variant: None }).unwrap();
    }
    assert!(Connectivity::build(ed.project()).problems.iter().all(|p| !p.message.starts_with("Reference")));
}
