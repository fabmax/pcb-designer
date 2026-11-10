//! Netlists of small sample circuits built with editor commands.

use editor_core::connectivity::Connectivity;
use editor_core::query::pin_positions;
use editor_core::{Command, Editor};
use model::builtin::{self, PART_CAPACITOR, PART_LED, PART_OPAMP, PART_RESISTOR};
use model::*;

struct Sch {
    ed: Editor,
}

impl Sch {
    fn new() -> Self {
        let mut ed = Editor::new(Project::new("test"));
        ed.execute(Command::ImportParts {
            library: builtin::builtin_library(),
            parts: vec![PART_RESISTOR, PART_CAPACITOR, PART_LED, PART_OPAMP],
        })
        .unwrap();
        Sch { ed }
    }

    /// Place a part; returns its symbol instance.
    fn place(&mut self, part: Id, x: f64, y: f64, rotation: f64) -> Id {
        let changes = self
            .ed
            .execute(Command::PlacePart {
                part,
                variant: None,
                sheet: None,
                pos: Point::mm(x, y),
                rotation: Angle::deg(rotation),
                mirror: false,
                refdes: None,
                value: None,
            })
            .unwrap();
        changes.ids().find_map(|id| if let ObjectId::Symbol(s) = id { Some(s) } else { None }).unwrap()
    }

    /// World position of a pin (by name).
    fn pin(&self, symbol: Id, name: &str) -> Point {
        pin_positions(self.ed.project(), symbol)
            .into_iter()
            .find(|p| p.1 == name)
            .unwrap_or_else(|| panic!("no pin {name}"))
            .2
    }

    fn wire(&mut self, points: &[Point]) -> Vec<Id> {
        let changes = self.ed.execute(Command::AddWire { sheet: None, points: points.to_vec() }).unwrap();
        changes.ids().filter_map(|id| if let ObjectId::Wire(w) = id { Some(w) } else { None }).collect()
    }

    fn label(&mut self, pos: Point, name: &str, kind: LabelKind) {
        self.ed
            .execute(Command::AddLabel { sheet: None, pos, name: name.into(), kind, rotation: Angle::ZERO })
            .unwrap();
    }

    fn junction(&mut self, pos: Point) {
        self.ed.execute(Command::AddJunction { sheet: None, pos }).unwrap();
    }

    fn conn(&self) -> Connectivity {
        Connectivity::build(self.ed.project())
    }

    fn netlist(&self) -> String {
        self.conn().to_text()
    }
}

/// An L-shaped wire from `a` to `b` (horizontal first).
fn l_path(a: Point, b: Point) -> Vec<Point> {
    if a.x == b.x || a.y == b.y { vec![a, b] } else { vec![a, Point::new(b.x, a.y), b] }
}

#[test]
fn led_with_series_resistor() {
    let mut s = Sch::new();
    let r = s.place(PART_RESISTOR, 0.0, 0.0, 0.0);
    let d = s.place(PART_LED, 20.0, 0.0, 0.0);
    let (r1, r2) = (s.pin(r, "1"), s.pin(r, "2"));
    let (a, k) = (s.pin(d, "A"), s.pin(d, "K"));
    s.wire(&l_path(r2, a));
    s.label(r1, "+5V", LabelKind::Power);
    s.wire(&[k, k + Point::mm(5.08, 0.0)]);
    s.label(k + Point::mm(5.08, 0.0), "GND", LabelKind::Power);
    // LED pins A/K are pads 2/1.
    assert_eq!(s.netlist(), "+5V: R1.1\nGND: D1.1\nNet-(D1-2): D1.2 R1.2\n");
    let c = s.conn();
    assert!(c.unconnected.is_empty());
    assert!(c.problems.is_empty());
    assert!(c.dots.is_empty());
}

#[test]
fn t_connection_joins_and_gets_a_dot() {
    let mut s = Sch::new();
    let r1 = s.place(PART_RESISTOR, 0.0, 0.0, 0.0);
    let r2 = s.place(PART_RESISTOR, 30.0, 0.0, 0.0);
    let r3 = s.place(PART_RESISTOR, 15.24, -15.24, 90.0);
    let (a, b) = (s.pin(r1, "2"), s.pin(r2, "1"));
    s.wire(&[a, b]);
    // R3's upper pin wired straight up onto the middle of that wire.
    let top = s.pin(r3, "2");
    let on_wire = Point::new(top.x, a.y);
    assert!(top.x > a.x && top.x < b.x);
    s.wire(&[top, on_wire]);
    let text = s.netlist();
    assert!(text.contains("Net-(R1-2): R1.2 R2.1 R3.2"), "{text}");
    let c = s.conn();
    assert_eq!(c.dots.len(), 1);
    assert_eq!(c.dots[0].1, on_wire);
    // Unconnected: R1.1, R2.2, R3.1.
    assert_eq!(c.unconnected.len(), 3);
}

#[test]
fn crossing_wires_connect_only_with_a_junction() {
    let mut s = Sch::new();
    let r1 = s.place(PART_RESISTOR, 0.0, 0.0, 0.0);
    let r2 = s.place(PART_RESISTOR, 30.0, 0.0, 0.0);
    let r3 = s.place(PART_RESISTOR, 15.24, -15.24, 90.0);
    let r4 = s.place(PART_RESISTOR, 15.24, 15.24, 90.0);
    let (a, b) = (s.pin(r1, "2"), s.pin(r2, "1"));
    let (c, d) = (s.pin(r3, "2"), s.pin(r4, "1"));
    assert_eq!(c.x, d.x);
    s.wire(&[a, b]);
    s.wire(&[c, d]);
    let text = s.netlist();
    assert!(text.contains("Net-(R1-2): R1.2 R2.1\n"), "{text}");
    assert!(text.contains("Net-(R3-2): R3.2 R4.1\n"), "{text}");
    assert!(s.conn().dots.is_empty());

    s.junction(Point::new(c.x, a.y));
    assert!(s.netlist().contains("Net-(R1-2): R1.2 R2.1 R3.2 R4.1\n"), "{}", s.netlist());
}

#[test]
fn labels_connect_by_name_and_name_the_net() {
    let mut s = Sch::new();
    let r1 = s.place(PART_RESISTOR, 0.0, 0.0, 0.0);
    let r2 = s.place(PART_RESISTOR, 40.0, 0.0, 0.0);
    let p = s.pin(r1, "2");
    let q = s.pin(r2, "1");
    s.wire(&[p, p + Point::mm(5.08, 0.0)]);
    s.label(p + Point::mm(5.08, 0.0), "SDA", LabelKind::Net);
    // A label in the middle of a wire also connects.
    s.wire(&[q - Point::mm(10.16, 0.0), q]);
    s.label(q - Point::mm(5.08, 0.0), "SDA", LabelKind::Net);
    assert!(s.netlist().contains("SDA: R1.2 R2.1\n"), "{}", s.netlist());

    // Two names on one net are reported; power ports win.
    s.label(p, "VCC", LabelKind::Power);
    let c = s.conn();
    assert!(c.net("VCC").is_some_and(|n| n.pins.len() == 2));
    assert_eq!(c.problems.len(), 1, "{:?}", c.problems);
}

#[test]
fn touching_pins_connect_directly() {
    let mut s = Sch::new();
    let r1 = s.place(PART_RESISTOR, 0.0, 0.0, 0.0);
    let p = s.pin(r1, "2");
    // Second resistor placed so that its pin 1 sits on R1's pin 2.
    let r2 = s.place(PART_RESISTOR, 0.0, 0.0, 0.0);
    let q = s.pin(r2, "1");
    s.ed.execute(Command::Move { ids: vec![ObjectId::Symbol(r2)], delta: p - q, keep_connected: false }).unwrap();
    assert_eq!(s.netlist(), "Net-(R1-2): R1.2 R2.1\n");
}

#[test]
fn dangling_wires_and_lookup() {
    let mut s = Sch::new();
    let r1 = s.place(PART_RESISTOR, 0.0, 0.0, 0.0);
    let p = s.pin(r1, "1");
    let w = s.wire(&[p, p - Point::mm(5.08, 0.0)]);
    let stray = s.wire(&[Point::mm(50.0, 50.0), Point::mm(60.0, 50.0)]);
    let c = s.conn();
    let net = c.net_of(ObjectId::Wire(w[0])).unwrap();
    assert_eq!(net.name, "Net-(R1-1)");
    assert_eq!(net.pins.len(), 1);
    let pin = &net.pins[0];
    assert_eq!(c.net_of_pin(pin.symbol, pin.pin).unwrap().name, "Net-(R1-1)");
    // A wire that touches nothing forms an unnamed group, not part of the netlist.
    assert_eq!(c.net_of(ObjectId::Wire(stray[0])).unwrap().name, "");
    assert_eq!(c.netlist().count(), 2);
    // R1.1 has a wire, R1.2 nothing.
    assert_eq!(c.unconnected.len(), 1);
}

#[test]
fn opamp_inverting_amplifier() {
    let mut s = Sch::new();
    let u = s.place(PART_OPAMP, 30.0, 0.0, 0.0);
    let rin = s.place(PART_RESISTOR, 0.0, -2.54, 0.0);
    let rf = s.place(PART_RESISTOR, 20.0, -12.7, 0.0);
    let (inm, inp, out) = (s.pin(u, "-IN"), s.pin(u, "+IN"), s.pin(u, "OUT"));
    let (v_pos, v_neg) = (s.pin(u, "V+"), s.pin(u, "V-"));
    // Input resistor straight into the inverting input; the feedback resistor's left
    // end joins that wire from below (T), its right end goes to the output.
    let (rin2, rf1, rf2) = (s.pin(rin, "2"), s.pin(rf, "1"), s.pin(rf, "2"));
    assert_eq!(rin2.y, inm.y);
    s.wire(&[rin2, inm]);
    assert!(rf1.x > rin2.x && rf1.x < inm.x);
    s.wire(&[rf1, Point::new(rf1.x, inm.y)]);
    s.wire(&l_path(rf2, out));
    s.label(s.pin(rin, "1"), "IN", LabelKind::Net);
    s.label(out, "OUT", LabelKind::Net);
    s.label(inp, "GND", LabelKind::Power);
    s.label(v_neg, "GND", LabelKind::Power);
    s.label(v_pos, "+5V", LabelKind::Power);
    let c = s.conn();
    let text = c.to_text();
    let pad = |name: &str| {
        let p = c.nets.iter().flat_map(|n| &n.pins).find(|p| p.refdes == "U1" && p.pin_name == name).unwrap();
        format!("U1.{}", p.pads[0])
    };
    // SOIC-8 single op amp: +IN 3, -IN 2, OUT 6, V+ 7, V- 4.
    assert_eq!(
        text,
        format!(
            "+5V: {}\nGND: {} {}\nIN: R1.1\nNet-(R1-2): R1.2 R2.1 {}\nOUT: R2.2 {}\n",
            pad("V+"),
            pad("+IN"),
            pad("V-"),
            pad("-IN"),
            pad("OUT")
        )
    );
    assert!(c.problems.is_empty());
    assert!(c.unconnected.is_empty(), "{:?}", c.unconnected);
    assert_eq!(c.dots.len(), 1);
}

// ---- Moving with wires attached ("drag")

fn mv(s: &mut Sch, ids: Vec<ObjectId>, dx: f64, dy: f64) {
    s.ed.execute(Command::Move { ids, delta: Point::mm(dx, dy), keep_connected: true }).unwrap();
}

fn orthogonal(s: &Sch) -> bool {
    s.ed.project().schematic.wires.iter().all(|w| w.a.x == w.b.x || w.a.y == w.b.y)
}

fn two_resistors() -> (Sch, Id, Id) {
    let mut s = Sch::new();
    let r1 = s.place(PART_RESISTOR, 0.0, 0.0, 0.0);
    let r2 = s.place(PART_RESISTOR, 20.32, 0.0, 0.0);
    let (a, b) = (s.pin(r1, "2"), s.pin(r2, "1"));
    s.wire(&[a, b]);
    (s, r1, r2)
}

#[test]
fn dragging_along_a_wire_stretches_it() {
    let (mut s, _, r2) = two_resistors();
    let before = s.netlist();
    mv(&mut s, vec![ObjectId::Symbol(r2)], 5.08, 0.0);
    assert_eq!(s.netlist(), before);
    assert_eq!(s.ed.project().schematic.wires.len(), 1);
    assert!(orthogonal(&s));
}

#[test]
fn dragging_sideways_keeps_wires_orthogonal() {
    let (mut s, _, r2) = two_resistors();
    let before = s.netlist();
    // Many small steps, like a mouse drag, in both directions.
    for _ in 0..8 {
        mv(&mut s, vec![ObjectId::Symbol(r2)], 1.27, 1.27);
    }
    mv(&mut s, vec![ObjectId::Symbol(r2)], 0.0, -2.54);
    assert_eq!(s.netlist(), before, "still connected");
    assert!(orthogonal(&s));
    // The original wire plus one dogleg at R1's pin.
    assert_eq!(s.ed.project().schematic.wires.len(), 2);
    assert_eq!(s.conn().unconnected.len(), 2, "only the outer pins R1.1 and R2.2");
    // Back to the start: the dogleg collapses and disappears.
    mv(&mut s, vec![ObjectId::Symbol(r2)], -10.16, -7.62);
    assert_eq!(s.ed.project().schematic.wires.len(), 1);
    assert_eq!(s.netlist(), before);
}

#[test]
fn dragging_an_l_shaped_connection_stretches_the_corner() {
    let mut s = Sch::new();
    let r1 = s.place(PART_RESISTOR, 0.0, 0.0, 0.0);
    let r2 = s.place(PART_RESISTOR, 20.32, 10.16, 0.0);
    let (a, b) = (s.pin(r1, "2"), s.pin(r2, "1"));
    s.wire(&l_path(a, b));
    let before = s.netlist();
    // R2 moves up: the vertical segment just gets longer (no dogleg needed).
    mv(&mut s, vec![ObjectId::Symbol(r2)], 0.0, 5.08);
    assert_eq!(s.ed.project().schematic.wires.len(), 2);
    // R2 moves right: the vertical segment shifts, the horizontal one stretches.
    mv(&mut s, vec![ObjectId::Symbol(r2)], 2.54, 0.0);
    assert_eq!(s.ed.project().schematic.wires.len(), 2);
    assert!(orthogonal(&s));
    assert_eq!(s.netlist(), before);
}

#[test]
fn moving_a_wire_drags_its_neighbors() {
    let mut s = Sch::new();
    let r1 = s.place(PART_RESISTOR, 0.0, 0.0, 0.0);
    let r2 = s.place(PART_RESISTOR, 20.32, 0.0, 0.0);
    let (a, b) = (s.pin(r1, "2"), s.pin(r2, "1"));
    // A U-shaped detour; move its bottom segment further down.
    let wires = s.wire(&[a, a - Point::mm(0.0, 5.08), b - Point::mm(0.0, 5.08), b]);
    let before = s.netlist();
    mv(&mut s, vec![ObjectId::Wire(wires[1])], 0.0, -5.08);
    assert_eq!(s.ed.project().schematic.wires.len(), 3);
    assert!(orthogonal(&s));
    assert_eq!(s.netlist(), before);
}

#[test]
fn rotating_with_wires_keeps_them_connected_and_undo_restores() {
    let (mut s, _, r2) = two_resistors();
    let before = s.netlist();
    let snapshot = s.ed.project().clone();
    let center = s.ed.project().schematic.symbols.get(r2).unwrap().pos;
    s.ed.execute(Command::Rotate {
        ids: vec![ObjectId::Symbol(r2)],
        center,
        angle: Angle::DEG90,
        keep_connected: true,
    })
    .unwrap();
    assert_eq!(s.netlist(), before);
    s.ed.undo();
    assert_eq!(s.ed.project(), &snapshot);
}

#[test]
fn plain_move_disconnects() {
    let (mut s, _, r2) = two_resistors();
    s.ed.execute(Command::Move { ids: vec![ObjectId::Symbol(r2)], delta: Point::mm(0.0, 2.54), keep_connected: false })
        .unwrap();
    assert_eq!(s.netlist(), "");
}

// ---- Multi-unit parts

#[test]
fn units_of_a_dual_opamp_are_separate_symbols() {
    let mut lib = builtin::builtin_library();
    let single = lib.parts.get(PART_OPAMP).unwrap().clone();
    let sym = single.units[0].symbol;
    let pin = |name: &str| lib.symbols.get(sym).unwrap().pins.iter().find(|p| p.name == name).unwrap().id;
    let pad = |name: &str| {
        lib.footprints.get(single.variants[0].footprint).unwrap().pads.iter().find(|p| p.name == name).unwrap().id
    };
    // Dual op amp in SOIC-8: A = 3/2/1, B = 5/6/7, power 8/4 on unit A.
    let mut dual = single.clone();
    dual.id = Id::new();
    dual.name = "Dual".into();
    dual.units.push(PartUnit { name: "B".into(), symbol: sym });
    let conn =
        |pad_name: &str, unit: u32, pin_name: &str| PadConnection { pad: pad(pad_name), unit, pin: pin(pin_name) };
    dual.variants[0].connections = vec![
        conn("3", 0, "+IN"),
        conn("2", 0, "-IN"),
        conn("1", 0, "OUT"),
        conn("8", 0, "V+"),
        conn("4", 0, "V-"),
        conn("5", 1, "+IN"),
        conn("6", 1, "-IN"),
        conn("7", 1, "OUT"),
    ];
    let dual_id = dual.id;
    lib.parts.insert(dual);
    assert!(lib.validate().is_empty(), "{:?}", lib.validate());

    let mut s = Sch::new();
    s.ed.execute(Command::ImportParts { library: lib, parts: vec![dual_id] }).unwrap();
    let a = s.place(dual_id, 0.0, 0.0, 0.0);
    let component = s.ed.project().schematic.symbols.get(a).unwrap().component;
    let placed =
        s.ed.execute(Command::PlaceUnit {
            component,
            unit: 1,
            sheet: None,
            pos: Point::mm(30.0, 0.0),
            rotation: Angle::ZERO,
            mirror: false,
        })
        .unwrap();
    let b = placed.ids().find_map(|id| if let ObjectId::Symbol(s) = id { Some(s) } else { None }).unwrap();
    // A unit can only be placed once, and must exist.
    for unit in [1, 2] {
        let r = s.ed.execute(Command::PlaceUnit {
            component,
            unit,
            sheet: None,
            pos: Point::ZERO,
            rotation: Angle::ZERO,
            mirror: false,
        });
        assert!(r.is_err());
    }
    // Unit A's output drives unit B's + input.
    s.wire(&l_path(s.pin(a, "OUT"), s.pin(b, "+IN")));
    assert_eq!(s.netlist(), "Net-(U1-1): U1.1 U1.5\n");
    // Deleting one unit keeps the component; deleting the last removes it.
    s.ed.execute(Command::Delete { ids: vec![ObjectId::Symbol(b)] }).unwrap();
    assert!(s.ed.project().schematic.components.contains(component));
    s.ed.execute(Command::Delete { ids: vec![ObjectId::Symbol(a)] }).unwrap();
    assert!(!s.ed.project().schematic.components.contains(component));
}
