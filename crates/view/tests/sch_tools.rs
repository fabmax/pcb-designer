//! Schematic creation tools driven like the GUI does: hover and click at snapped points.

use editor_core::Command;
use editor_core::query::pin_positions;
use model::builtin::{self, PART_OPAMP, PART_RESISTOR};
use model::*;
use view::Document;
use view::sch_tools::{SchGhost, SchPlacer, SchPrompt, SchTool};

fn doc_with(parts: Vec<Id>, library: Library) -> Document {
    let mut doc = Document::new(Project::new("T"));
    doc.execute(Command::import_parts(&library, &parts).unwrap()).unwrap();
    doc
}

fn pin(doc: &Document, symbol: Id, name: &str) -> Point {
    pin_positions(doc.project(), symbol).into_iter().find(|p| p.1 == name).unwrap().2
}

fn symbols(doc: &Document) -> Vec<SymbolInstance> {
    let mut s: Vec<SymbolInstance> = doc.project().schematic.symbols.iter().cloned().collect();
    s.sort_by_key(|s| s.pos.x);
    s
}

#[test]
fn place_wire_and_label_a_circuit() {
    let mut doc = doc_with(vec![PART_RESISTOR], builtin::builtin_library());
    let mut t = SchPlacer::default();

    // Place two resistors with repeated clicks.
    t.place(PART_RESISTOR);
    t.hover(&mut doc, Some(Point::mm(0.0, 0.0)));
    assert!(matches!(t.ghosts()[..], [SchGhost::Symbol { component: None, unit: 0, .. }]));
    t.click(&mut doc, Point::mm(0.0, 0.0)).unwrap();
    t.rotate();
    t.click(&mut doc, Point::mm(20.32, 0.0)).unwrap();
    let s = symbols(&doc);
    assert_eq!(s.len(), 2);
    assert_eq!(s[1].rotation, Angle::DEG90);
    let refdes: Vec<String> = doc.project().schematic.components.iter().map(|c| c.refdes.clone()).collect();
    assert!(refdes.contains(&"R1".to_string()) && refdes.contains(&"R2".to_string()), "{refdes:?}");
    assert!(t.escape(), "Esc leaves the place tool");
    assert_eq!(t.tool(), SchTool::Select);

    // Wire from R1 pin 2 to R2 pin 1: start, then one click on the far pin draws an L
    // and ends the chain there.
    let (a, b) = (pin(&doc, s[0].id, "2"), pin(&doc, s[1].id, "1"));
    assert!(a.x != b.x && a.y != b.y, "needs a corner");
    t.set_tool(SchTool::Wire);
    t.click(&mut doc, a).unwrap();
    assert!(t.in_progress());
    t.hover(&mut doc, Some(b));
    let ghosts = t.ghosts();
    assert_eq!(ghosts.iter().filter(|g| matches!(g, SchGhost::Wire(..))).count(), 2);
    assert!(ghosts.contains(&SchGhost::Target(b)), "the pin is highlighted as a target");
    let placed = t.click(&mut doc, b).unwrap();
    assert!(placed.finished && !t.in_progress());
    assert_eq!(doc.project().schematic.wires.len(), 2);
    let text = doc.current_connectivity().to_text();
    assert_eq!(text, "Net-(R1-2): R1.2 R2.1\n");

    // Labels count up; power ports use the chosen net.
    t.set_tool(SchTool::Label);
    t.settings.label = "IN1".into();
    let p = pin(&doc, s[0].id, "1");
    t.click(&mut doc, p).unwrap();
    assert_eq!(t.settings.label, "IN2");
    t.set_tool(SchTool::Power);
    t.settings.power = "GND".into();
    let p = pin(&doc, s[1].id, "2");
    t.click(&mut doc, p).unwrap();
    let text = doc.current_connectivity().to_text();
    assert!(text.contains("GND: R2.2\n") && text.contains("IN1: R1.1\n"), "{text}");
    assert!(doc.connectivity().unconnected.is_empty());
}

#[test]
fn wire_chain_continues_in_free_space_and_ends_on_double_click() {
    let mut doc = Document::new(Project::new("T"));
    let mut t = SchPlacer::default();
    t.set_tool(SchTool::Wire);
    let pts = [Point::mm(0.0, 0.0), Point::mm(10.16, 0.0), Point::mm(10.16, 10.16)];
    for p in pts {
        assert!(!t.click(&mut doc, p).unwrap().finished);
    }
    assert!(t.in_progress());
    assert!(t.click(&mut doc, pts[2]).unwrap().finished, "clicking the last point again ends the chain");
    assert_eq!(doc.project().schematic.wires.len(), 2);

    // A new wire ending inside an existing one (T connection) ends there.
    t.click(&mut doc, Point::mm(5.08, -5.08)).unwrap();
    let placed = t.click(&mut doc, Point::mm(5.08, 0.0)).unwrap();
    assert!(placed.finished);
    // Posture: vertical first.
    t.toggle_posture();
    t.click(&mut doc, Point::mm(20.32, 0.0)).unwrap();
    t.hover(&mut doc, Some(Point::mm(25.4, 5.08)));
    assert_eq!(t.ghosts()[0], SchGhost::Wire(Point::mm(20.32, 0.0), Point::mm(20.32, 5.08)));
    // Esc ends the chain first, then the tool.
    assert!(!t.escape());
    assert_eq!(t.tool(), SchTool::Wire);
    assert!(t.escape());
}

#[test]
fn multi_unit_parts_place_unit_by_unit() {
    // Dual op amp: the built-in op amp with a second unit.
    let mut lib = builtin::builtin_library();
    let mut dual = lib.parts.get(PART_OPAMP).unwrap().clone();
    dual.id = Id::new();
    dual.name = "Dual".into();
    let sym = dual.units[0].symbol;
    dual.units.push(PartUnit { name: "B".into(), symbol: sym });
    let dual_id = dual.id;
    lib.parts.insert(dual);
    let mut doc = doc_with(vec![dual_id], lib);

    let mut t = SchPlacer::default();
    t.place(dual_id);
    t.click(&mut doc, Point::mm(0.0, 0.0)).unwrap();
    let pl = t.placing.unwrap();
    assert_eq!(pl.unit, 1);
    let first = pl.component.expect("next click places unit B of the same component");
    t.hover(&mut doc, Some(Point::mm(20.32, 0.0)));
    assert!(matches!(t.ghosts()[..], [SchGhost::Symbol { component: Some(c), unit: 1, .. }] if c == first));
    t.click(&mut doc, Point::mm(20.32, 0.0)).unwrap();
    // All units placed: the next click starts a new component.
    assert_eq!(t.placing.unwrap().component, None);
    t.click(&mut doc, Point::mm(0.0, 20.32)).unwrap();
    let s = &doc.project().schematic;
    assert_eq!(s.components.len(), 2);
    assert_eq!(s.symbols_of(first).count(), 2);
    assert_eq!(s.symbols.len(), 3);
}

#[test]
fn draw_a_bus_with_entries() {
    let mut doc = Document::new(Project::new("T"));
    let mut t = SchPlacer::default();
    let p = Point::mm;

    // A bus chain continues in free space and ends on another bus.
    t.set_tool(SchTool::Bus);
    t.click(&mut doc, p(0.0, 0.0)).unwrap();
    t.hover(&mut doc, Some(p(0.0, 25.4)));
    assert_eq!(t.ghosts(), vec![SchGhost::Bus(p(0.0, 0.0), p(0.0, 25.4))]);
    assert!(!t.click(&mut doc, p(0.0, 25.4)).unwrap().finished);
    assert!(t.click(&mut doc, p(0.0, 25.4)).unwrap().finished);
    t.click(&mut doc, p(10.16, 10.16)).unwrap();
    t.hover(&mut doc, Some(p(0.0, 10.16)));
    assert!(t.ghosts().contains(&SchGhost::Target(p(0.0, 10.16))), "the bus is a target");
    assert!(t.click(&mut doc, p(0.0, 10.16)).unwrap().finished, "ends on the bus");
    assert_eq!(doc.project().schematic.bus_wires.len(), 2);
    assert!(doc.project().schematic.wires.is_empty());

    // Entries: R turns them counter-clockwise, X mirrors.
    t.set_tool(SchTool::BusEntry);
    t.hover(&mut doc, Some(p(0.0, 5.08)));
    assert_eq!(t.ghosts(), vec![SchGhost::Entry(p(0.0, 5.08), EntryDir::UpRight), SchGhost::Target(p(0.0, 5.08))]);
    t.rotate();
    assert_eq!(t.settings.entry, EntryDir::UpLeft);
    t.mirror();
    assert_eq!(t.settings.entry, EntryDir::UpRight);
    t.click(&mut doc, p(0.0, 5.08)).unwrap();

    // A wire drawn from the entry's free end ends on the next connection point.
    t.set_tool(SchTool::Wire);
    t.click(&mut doc, p(10.16, 7.62)).unwrap();
    assert!(t.click(&mut doc, p(2.54, 7.62)).unwrap().finished, "ends on the entry");
    t.set_tool(SchTool::Label);
    t.settings.label = "D[0..3]".into();
    t.click(&mut doc, p(0.0, 20.32)).unwrap();
    t.settings.label = "D2".into();
    t.click(&mut doc, p(10.16, 7.62)).unwrap();
    let c = doc.current_connectivity();
    assert_eq!(c.buses.len(), 1);
    assert_eq!(c.buses[0].members, ["D0", "D1", "D2", "D3"]);
    assert!(c.problems.is_empty(), "{:?}", c.problems);
    assert_eq!(c.nets.iter().filter(|n| n.named).map(|n| n.name.as_str()).collect::<Vec<_>>(), ["/D2"]);
}

#[test]
fn new_buses_and_signals_leaving_them_ask_for_a_name() {
    let mut doc = doc_with(vec![PART_RESISTOR], builtin::builtin_library());
    let mut t = SchPlacer::default();
    let p = Point::mm;
    let grid = mm(1.27);

    // A finished bus without a name asks for one; the label goes to its middle.
    t.set_tool(SchTool::Bus);
    t.click(&mut doc, p(0.0, 0.0)).unwrap();
    t.click(&mut doc, p(0.0, 22.86)).unwrap();
    assert_eq!(t.take_prompt(&mut doc, grid), None, "still drawing");
    assert!(!t.escape(), "Esc ends the chain");
    let prompt = t.take_prompt(&mut doc, grid).expect("asks for the bus name");
    let sheet = doc.project().schematic.root_sheet().unwrap();
    assert_eq!(prompt, SchPrompt::BusName { sheet, pos: p(0.0, 11.43), rotation: Angle::DEG90 });
    assert_eq!(t.take_prompt(&mut doc, grid), None, "asked once");
    assert!(prompt.label("D0").is_err(), "not a bus name");
    doc.execute(prompt.label(" D[0..3] ").unwrap()).unwrap();
    assert_eq!(doc.current_connectivity().buses[0].name, "D[0..3]");

    // A bus continuing the named one doesn't ask.
    t.click(&mut doc, p(0.0, 22.86)).unwrap();
    t.click(&mut doc, p(10.16, 22.86)).unwrap();
    t.escape();
    assert_eq!(t.take_prompt(&mut doc, grid), None);

    // A wire from a pin to an entry asks for the signal, offering the bus's members.
    t.set_tool(SchTool::BusEntry);
    t.click(&mut doc, p(0.0, 5.08)).unwrap();
    assert_eq!(t.take_prompt(&mut doc, grid), None, "nothing leaves the entry yet");
    t.set_tool(SchTool::Select);
    let r = doc
        .execute(Command::PlacePart {
            part: PART_RESISTOR,
            variant: None,
            sheet: None,
            pos: p(20.32, 7.62),
            rotation: Angle::ZERO,
            mirror: false,
            refdes: None,
            value: None,
        })
        .unwrap()
        .ids()
        .find_map(|id| if let ObjectId::Symbol(s) = id { Some(s) } else { None })
        .unwrap();
    let pin1 = pin(&doc, r, "1");
    t.set_tool(SchTool::Wire);
    t.click(&mut doc, pin1).unwrap();
    assert!(t.click(&mut doc, p(2.54, 7.62)).unwrap().finished);
    let prompt = t.take_prompt(&mut doc, grid).expect("asks for the signal");
    let SchPrompt::EntryNet { bus, members, uses, pos, rotation, .. } = &prompt else { panic!("{prompt:?}") };
    assert_eq!((bus.as_str(), members.len(), uses.as_slice()), ("D[0..3]", 4, &[0, 0, 0, 0][..]));
    assert_eq!((*pos, *rotation), (p(5.08, 7.62), Angle::ZERO), "on the wire, next to the entry");
    doc.execute(prompt.label("D1").unwrap()).unwrap();
    let c = doc.current_connectivity();
    assert!(c.problems.is_empty(), "{:?}", c.problems);
    assert_eq!(c.net("/D1").unwrap().pins.len(), 1);

    // An entry placed onto an unnamed wire asks too, now with D1 in use; a named one doesn't.
    t.set_tool(SchTool::Wire);
    t.click(&mut doc, p(2.54, 15.24)).unwrap();
    t.click(&mut doc, p(12.7, 15.24)).unwrap();
    t.escape();
    assert_eq!(t.take_prompt(&mut doc, grid), None, "not on an entry yet");
    t.set_tool(SchTool::BusEntry);
    t.click(&mut doc, p(0.0, 12.7)).unwrap();
    let Some(SchPrompt::EntryNet { uses, .. }) = t.take_prompt(&mut doc, grid) else { panic!("asks") };
    assert_eq!(uses, [0, 1, 0, 0], "D1 is labelled once");
    t.click(&mut doc, p(0.0, 10.16)).unwrap();
    assert_eq!(t.take_prompt(&mut doc, grid), None, "nothing leaves this one");
}
