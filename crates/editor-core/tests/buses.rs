//! Buses: grouping, bus labels, entries, the bus checks and dragging.

use editor_core::connectivity::Connectivity;
use editor_core::query::pin_positions;
use editor_core::{Command, Editor};
use model::builtin::{self, PART_RESISTOR};
use model::*;

fn editor() -> Editor {
    let mut ed = Editor::new(Project::new("buses"));
    ed.execute(Command::ImportParts { library: builtin::builtin_library(), parts: vec![PART_RESISTOR] }).unwrap();
    ed
}

fn created<T>(ed: &mut Editor, cmd: Command, pick: impl Fn(ObjectId) -> Option<T>) -> Vec<T> {
    ed.execute(cmd).unwrap().ids().filter_map(pick).collect()
}

fn bus(ed: &mut Editor, points: &[Point]) -> Vec<Id> {
    created(ed, Command::AddBus { sheet: None, points: points.to_vec() }, |id| match id {
        ObjectId::BusWire(w) => Some(w),
        _ => None,
    })
}

fn wire(ed: &mut Editor, points: &[Point]) -> Vec<Id> {
    created(ed, Command::AddWire { sheet: None, points: points.to_vec() }, |id| match id {
        ObjectId::Wire(w) => Some(w),
        _ => None,
    })
}

fn entry(ed: &mut Editor, pos: Point, dir: EntryDir) -> Id {
    created(ed, Command::AddBusEntry { sheet: None, pos, dir }, |id| match id {
        ObjectId::BusEntry(e) => Some(e),
        _ => None,
    })[0]
}

fn label(ed: &mut Editor, pos: Point, name: &str) -> Id {
    let cmd = Command::AddLabel { sheet: None, pos, name: name.into(), kind: LabelKind::Net, rotation: Angle::ZERO };
    created(ed, cmd, |id| match id {
        ObjectId::Label(l) => Some(l),
        _ => None,
    })[0]
}

fn resistor(ed: &mut Editor, x: f64, y: f64) -> (Id, Point, Point) {
    let cmd = Command::PlacePart {
        part: PART_RESISTOR,
        variant: None,
        sheet: None,
        pos: Point::mm(x, y),
        rotation: Angle::ZERO,
        mirror: false,
        refdes: None,
        value: None,
    };
    let s = created(ed, cmd, |id| match id {
        ObjectId::Symbol(s) => Some(s),
        _ => None,
    })[0];
    let pins = pin_positions(ed.project(), s);
    (s, pins[0].2, pins[1].2)
}

fn messages(c: &Connectivity) -> Vec<String> {
    c.problems.iter().map(|p| p.message.clone()).collect()
}

const P: fn(f64, f64) -> Point = Point::mm;

#[test]
fn touching_bus_wires_form_one_bus_named_by_its_labels() {
    let mut ed = editor();
    // An L, a T-joint onto it, and a separate bus.
    let l = bus(&mut ed, &[P(0.0, 0.0), P(20.32, 0.0), P(20.32, -20.32)]);
    let t = bus(&mut ed, &[P(10.16, 0.0), P(10.16, 10.16)]);
    let other = bus(&mut ed, &[P(0.0, 30.48), P(20.32, 30.48)]);
    let named = label(&mut ed, P(5.08, 0.0), "D[0..3]");
    label(&mut ed, P(20.32, -10.16), "{CLK D3}");
    label(&mut ed, P(5.08, 30.48), "I2C{SDA SCL}");

    let c = Connectivity::build(ed.project());
    assert_eq!(c.buses.len(), 2);
    let main = c.bus_of(ObjectId::BusWire(t[0])).unwrap();
    assert_eq!(main.name, "D[0..3]");
    assert_eq!(main.members, ["D0", "D1", "D2", "D3", "CLK"]);
    assert_eq!(main.wires.len(), l.len() + t.len());
    assert_eq!(c.bus_of(ObjectId::Label(named)), Some(main));
    let i2c = c.bus_of(ObjectId::BusWire(other[0])).unwrap();
    assert_eq!(i2c.members, ["I2C.SDA", "I2C.SCL"]);
    // The T-joint gets a dot; bus labels make no nets.
    assert_eq!(c.dots, vec![(ed.project().schematic.first_sheet().unwrap(), P(10.16, 0.0))]);
    assert!(c.nets.is_empty(), "{:?}", c.nets);
    assert!(c.problems.is_empty(), "{:?}", messages(&c));
}

#[test]
fn signals_connect_through_entries_by_label_only() {
    let mut ed = editor();
    bus(&mut ed, &[P(0.0, 0.0), P(0.0, 50.8)]);
    label(&mut ed, P(0.0, 25.4), "D[0..1]");
    // Two resistors, each wired to an entry and labelled; R1 and R2 on D0, R3 on D1.
    let (_, r1, _) = resistor(&mut ed, 20.0, 10.16);
    let (_, r2, _) = resistor(&mut ed, 20.0, 40.64);
    let e0 = entry(&mut ed, P(0.0, 7.62), EntryDir::UpRight);
    let e1 = entry(&mut ed, P(0.0, 38.1), EntryDir::UpRight);
    let w0 = wire(&mut ed, &[P(2.54, 10.16), r1]);
    let w1 = wire(&mut ed, &[P(2.54, 40.64), r2]);
    label(&mut ed, P(5.08, 10.16), "D0");
    label(&mut ed, P(5.08, 40.64), "D1");

    let c = Connectivity::build(ed.project());
    assert!(c.problems.is_empty(), "{:?}", messages(&c));
    // The bus doesn't join D0 and D1; each entry knows its signal.
    assert_eq!(c.net_of(ObjectId::BusEntry(e0)).unwrap().name, "D0");
    assert_eq!(c.net_of(ObjectId::BusEntry(e1)).unwrap().name, "D1");
    assert_eq!(c.net_of(ObjectId::Wire(w0[0])).unwrap().pins.len(), 1);
    assert_eq!(c.net_of(ObjectId::Wire(w1[0])).unwrap().pins.len(), 1);
    assert_eq!(c.bus_of(ObjectId::BusEntry(e0)).unwrap().entries.len(), 2);
}

#[test]
fn bus_checks() {
    let mut ed = editor();
    let unnamed = bus(&mut ed, &[P(0.0, 0.0), P(0.0, 50.8)]);
    let named = bus(&mut ed, &[P(50.8, 0.0), P(50.8, 50.8)]);
    label(&mut ed, P(50.8, 25.4), "A[0..1]");
    let c = Connectivity::build(ed.project());
    assert_eq!(messages(&c), ["Bus has no bus label (like D[0..7] or {SDA SCL})"]);
    assert_eq!(c.problems[0].objects, vec![ObjectId::BusWire(unnamed[0])]);

    // A wire ending on the bus and a pin on it, two unlabelled or wrongly labelled wires
    // leaving entries, an entry and a bus label off any bus.
    let mut ed2 = Editor::new(ed.project().clone());
    let w = wire(&mut ed2, &[P(30.48, 10.16), P(50.8, 10.16)]);
    let (_, pin, _) = resistor(&mut ed2, 100.0, 100.0);
    bus(&mut ed2, &[pin, pin + P(0.0, 10.16)]);
    let e = entry(&mut ed2, P(50.8, 20.32), EntryDir::UpRight);
    wire(&mut ed2, &[P(53.34, 22.86), P(60.96, 22.86)]);
    entry(&mut ed2, P(50.8, 30.48), EntryDir::UpRight);
    wire(&mut ed2, &[P(53.34, 33.02), P(60.96, 33.02)]);
    label(&mut ed2, P(60.96, 33.02), "B7");
    entry(&mut ed2, P(80.0, 80.0), EntryDir::UpRight);
    label(&mut ed2, P(90.0, 90.0), "{X Y}");
    let c = Connectivity::build(ed2.project());
    let m = messages(&c);
    assert!(m.contains(&"Wire touches bus A[0..1] directly; connect it through a bus entry".to_string()), "{m:?}");
    assert!(m.iter().any(|m| m.starts_with("Pin R1.") && m.contains("an unnamed bus")), "{m:?}");
    assert!(m.contains(&"Wire from a bus entry of bus A[0..1] has no label".to_string()), "{m:?}");
    assert!(m.contains(&"B7 is not a member of bus A[0..1]".to_string()), "{m:?}");
    assert!(m.contains(&"Bus entry is not on a bus".to_string()), "{m:?}");
    assert!(m.contains(&"Bus label {X Y} is not on a bus".to_string()), "{m:?}");
    let touching = c.problems.iter().find(|p| p.message.starts_with("Wire touches")).unwrap();
    assert_eq!(touching.objects, vec![ObjectId::Wire(w[0]), ObjectId::BusWire(named[0])]);
    assert_eq!(touching.pos, Some(P(50.8, 10.16)));
    let unlabelled = c.problems.iter().find(|p| p.message.ends_with("has no label")).unwrap();
    assert_eq!(unlabelled.objects[0], ObjectId::BusEntry(e));
    // The wire touching the bus is still its own net: nothing is joined through the bus.
    assert!(c.net_of(ObjectId::Wire(w[0])).unwrap().pins.is_empty());
}

#[test]
fn bus_wires_follow_drags_and_entries_turn() {
    let mut ed = editor();
    let a = bus(&mut ed, &[P(0.0, 0.0), P(20.32, 0.0)]);
    let b = bus(&mut ed, &[P(20.32, 0.0), P(20.32, 20.32)]);
    let e = entry(&mut ed, P(10.16, 0.0), EntryDir::UpRight);
    let w = wire(&mut ed, &[P(12.7, 2.54), P(12.7, 12.7)]);

    // Dragging the vertical bus sideways stretches the horizontal one.
    let d = P(5.08, 0.0);
    ed.execute(Command::Move { ids: vec![ObjectId::BusWire(b[0])], delta: d, keep_connected: true }).unwrap();
    let s = &ed.project().schematic;
    assert_eq!((s.bus_wires.get(a[0]).unwrap().a, s.bus_wires.get(a[0]).unwrap().b), (P(0.0, 0.0), P(25.4, 0.0)));
    // The entry stays where it is (entries only move when selected).
    assert_eq!(s.bus_entries.get(e).unwrap().pos, P(10.16, 0.0));

    // Moving the entry drags the signal's wire, not the bus.
    ed.execute(Command::Move { ids: vec![ObjectId::BusEntry(e)], delta: P(2.54, 0.0), keep_connected: true }).unwrap();
    let s = &ed.project().schematic;
    let wire = s.wires.get(w[0]).unwrap();
    assert_eq!((wire.a, wire.b), (P(15.24, 2.54), P(15.24, 12.7)));
    assert_eq!(s.bus_wires.get(a[0]).unwrap().b, P(25.4, 0.0));

    // Rotating and mirroring turn the entry.
    let rot = Command::Rotate {
        ids: vec![ObjectId::BusEntry(e)],
        center: P(12.7, 0.0),
        angle: Angle::DEG90,
        keep_connected: false,
    };
    ed.execute(rot).unwrap();
    let entry = ed.project().schematic.bus_entries.get(e).unwrap().clone();
    assert_eq!((entry.pos, entry.dir), (P(12.7, 0.0), EntryDir::UpLeft));
    ed.execute(Command::Mirror { ids: vec![ObjectId::BusEntry(e)], axis_x: entry.pos.x, keep_connected: false })
        .unwrap();
    assert_eq!(ed.project().schematic.bus_entries.get(e).unwrap().dir, EntryDir::UpRight);

    // A symbol moving next to a bus never drags it.
    let (r, _, _) = resistor(&mut ed, 0.0, 30.0);
    let before = ed.project().schematic.bus_wires.clone();
    ed.execute(Command::Move { ids: vec![ObjectId::Symbol(r)], delta: P(2.54, 0.0), keep_connected: true }).unwrap();
    assert_eq!(ed.project().schematic.bus_wires, before);
}

#[test]
fn entry_directions() {
    assert_eq!(EntryDir::from_rotation(Angle::ZERO), EntryDir::UpRight);
    assert_eq!(EntryDir::from_rotation(Angle::DEG90), EntryDir::UpLeft);
    for d in EntryDir::ALL {
        assert_eq!(EntryDir::of_offset(d.offset()), d);
    }
}
