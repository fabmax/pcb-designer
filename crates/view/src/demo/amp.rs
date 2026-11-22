//! The audio amplifier example: a discrete class-AB power amplifier for an 8 Ω speaker on
//! 12 V. A common-emitter driver (Q1) with a bootstrapped collector load (R5, R6, C3)
//! drives complementary emitter followers (Q2, Q3), biased by two diodes and R7. R2 / R1
//! set the gain (about 4) and, with R3, the output's DC level (about half the supply).
//! A test bench sheet feeds a 1 kHz sine; the simulation setup powers +12V and loads the
//! output with 8 Ω.

use editor_core::query::{pad_positions, pin_pad_names, pin_positions};
use editor_core::router::{self, RouteMode};
use editor_core::{Command, Editor};
use model::builtin::{self, gid};
use model::*;

use super::{footprint_of, move_footprint, symbol_of};
use crate::Document;

/// A point on the schematic grid (2.54 mm), shifted onto the page.
fn g(x: f64, y: f64) -> Point {
    Point::mm((x + 8.0) * 2.54, y * 2.54)
}

/// Draws on one sheet.
struct Sch<'a> {
    ed: &'a mut Editor,
    sheet: Option<Id>,
}

impl Sch<'_> {
    /// Places the Built-in `part` as `variant` at grid `(x, y)`, turned by `rot`°
    /// (counterclockwise) after mirroring.
    #[allow(clippy::too_many_arguments)]
    fn place(
        &mut self,
        part: &str,
        variant: &str,
        x: f64,
        y: f64,
        rot: f64,
        mirror: bool,
        refdes: &str,
        value: &str,
    ) -> Id {
        let part = gid(&format!("part/{part}"));
        let p = self.ed.project().library.parts.get(part).expect("part imported");
        let variant = p.variants.iter().find(|v| v.name == variant).map(|v| v.id);
        assert!(variant.is_some(), "{refdes}: no variant {variant:?}");
        self.ed
            .execute(Command::PlacePart {
                part,
                variant,
                sheet: self.sheet,
                pos: g(x, y),
                rotation: Angle::deg(rot),
                mirror,
                refdes: Some(refdes.into()),
                value: Some(value.into()),
            })
            .expect("place part");
        self.ed.project().schematic.component_by_refdes(refdes).expect("placed").id
    }

    /// A wire through grid points.
    fn wire(&mut self, pts: &[(f64, f64)]) {
        let points = pts.iter().map(|&(x, y)| g(x, y)).collect();
        self.ed.execute(Command::AddWire { sheet: self.sheet, points }).expect("wire");
    }

    fn label(&mut self, x: f64, y: f64, name: &str, kind: LabelKind, rot: f64) {
        self.ed
            .execute(Command::AddLabel {
                sheet: self.sheet,
                pos: g(x, y),
                name: name.into(),
                kind,
                shape: LabelShape::Input,
                style: None,
                rotation: Angle::deg(rot),
            })
            .expect("label");
    }

    fn power(&mut self, x: f64, y: f64, net: &str) {
        self.label(x, y, net, LabelKind::Power, 0.0);
    }

    fn text(&mut self, x: f64, y: f64, text: &str, size: f64) {
        let sheet = match self.sheet {
            Some(s) => s,
            None => self.ed.project().schematic.root_sheet().expect("root"),
        };
        let text = Text::new(text, g(x, y), mm(size));
        let object = Object::SchText(SchText { id: Id::new(), sheet, text });
        self.ed.execute(Command::Insert { objects: vec![object] }).expect("text");
    }

    /// Asserts that pin `pin` of `component` is at grid `(x, y)` (the wires assume it).
    fn at(&self, component: Id, pin: &str, x: f64, y: f64) {
        let p = self.ed.project();
        let (_, _, at) = pin_positions(p, symbol_of(p, component)).into_iter().find(|x| x.1 == pin).expect("pin");
        assert_eq!(at, g(x, y), "{pin} of {:?}", p.schematic.components.get(component).map(|c| &c.refdes));
    }
}

const PARTS: [&str; 7] = ["Resistor", "Capacitor", "Capacitor_Polarized", "Diode", "NPN", "PNP", "Conn_01x02"];

/// The amplifier's schematic and simulation setup (the board comes from [`board`]).
fn schematic(ed: &mut Editor) {
    let mut ids: Vec<Id> = PARTS.iter().map(|n| gid(&format!("part/{n}"))).collect();
    ids.push(builtin::PART_VSOURCE);
    ed.execute(Command::import_parts(&builtin::builtin_library(), &ids).unwrap()).unwrap();
    ed.execute(Command::SetTitleBlock {
        title_block: TitleBlock {
            title: "Discrete class-AB audio amplifier".into(),
            revision: "A".into(),
            comment: "12 V supply, 8 Ω speaker, gain about 4 (12 dB), about 0.5 W".into(),
            ..Default::default()
        },
    })
    .unwrap();

    let mut s = Sch { ed, sheet: None };
    // Input: J1, coupling C1, R1 into the driver's base; R2 feeds the output back, R3
    // sets the bias with it.
    let j1 = s.place("Conn_01x02", "Pin header", 16.0, 40.0, 0.0, true, "J1", "Input");
    s.at(j1, "1", 18.0, 40.0);
    s.place("Capacitor", "0805", 22.0, 40.0, 0.0, false, "C1", "4.7u");
    s.place("Resistor", "0805", 28.0, 40.0, 0.0, false, "R1", "4.7k");
    s.place("Resistor", "0805", 32.0, 36.0, 90.0, false, "R3", "5.6k");
    let q1 = s.place("NPN", "SOT-23", 35.0, 40.0, 0.0, false, "Q1", "BC847");
    s.at(q1, "C", 36.0, 42.0);
    s.place("Capacitor", "0805", 34.0, 44.0, 0.0, false, "C4", "100p");
    s.place("Resistor", "0805", 32.0, 48.0, 90.0, false, "R2", "22k");
    s.place("Resistor", "0805", 36.0, 33.0, 90.0, false, "R4", "100");
    let c2 = s.place("Capacitor_Polarized", "Radial Ø6.3, 2.5 mm", 39.0, 33.0, 270.0, false, "C2", "100u");
    s.at(c2, "+", 39.0, 35.0);
    // The bias string and the bootstrapped collector load above Q1.
    s.place("Resistor", "0805", 36.0, 46.0, 90.0, false, "R7", "47");
    let d2 = s.place("Diode", "SOD-123", 36.0, 51.0, 270.0, false, "D2", "1N4148");
    s.at(d2, "A", 36.0, 53.0);
    s.place("Diode", "SOD-123", 36.0, 57.0, 270.0, false, "D1", "1N4148");
    s.place("Resistor", "0805", 36.0, 62.0, 90.0, false, "R6", "470");
    s.place("Resistor", "0805", 36.0, 67.0, 90.0, false, "R5", "470");
    s.place("Capacitor_Polarized", "Radial Ø6.3, 2.5 mm", 40.0, 64.0, 0.0, false, "C3", "100u");
    // The output pair, the speaker's coupling capacitor and connector.
    let q2 = s.place("NPN", "SOT-223", 45.0, 59.0, 0.0, false, "Q2", "BCP56");
    s.at(q2, "B", 44.0, 59.0);
    let q3 = s.place("PNP", "SOT-223", 45.0, 44.0, 180.0, true, "Q3", "BCP53");
    s.at(q3, "E", 46.0, 46.0);
    s.at(q3, "C", 46.0, 42.0);
    // Turned and mirrored, its texts would be drawn over the symbol: aligned the other way
    // round in symbol coordinates, they read like Q1's beside it, reference on top.
    let fields: Vec<Object> = {
        let p = s.ed.project();
        let sym = symbol_of(p, q3);
        p.schematic
            .fields
            .iter()
            .filter(|f| f.symbol == sym)
            .map(|f| {
                let mut f = f.clone();
                f.text.halign = HAlign::Right;
                f.text.valign = match f.text.valign {
                    VAlign::Bottom => VAlign::Top,
                    VAlign::Top => VAlign::Bottom,
                    v => v,
                };
                f.text.pos.y = -f.text.pos.y;
                Object::SymbolField(f)
            })
            .collect()
    };
    s.ed.execute(Command::Update { objects: fields }).unwrap();
    s.place("Resistor", "1206", 46.0, 54.0, 90.0, false, "R8", "1");
    s.place("Resistor", "1206", 46.0, 49.0, 90.0, false, "R9", "1");
    s.place("Capacitor_Polarized", "Radial Ø10, 5 mm", 53.0, 52.0, 0.0, false, "C5", "1000u");
    let j3 = s.place("Conn_01x02", "Screw terminal 5.00 mm", 60.0, 52.0, 0.0, false, "J3", "Speaker");
    s.at(j3, "2", 58.0, 51.0);
    // Supply: J2 with its reservoir and decoupling capacitors.
    let j2 = s.place("Conn_01x02", "Screw terminal 5.00 mm", 16.0, 24.0, 0.0, true, "J2", "12 V");
    s.at(j2, "2", 18.0, 23.0);
    s.place("Capacitor_Polarized", "Radial Ø8, 3.5 mm", 24.0, 24.0, 270.0, false, "C6", "470u");
    s.place("Capacitor", "0805", 30.0, 24.0, 90.0, false, "C7", "100n");

    // Input to the base.
    s.wire(&[(18.0, 40.0), (19.0, 40.0)]);
    s.label(19.0, 40.0, "IN", LabelKind::Global, 0.0);
    s.wire(&[(19.0, 40.0), (20.0, 40.0)]);
    s.wire(&[(24.0, 40.0), (26.0, 40.0)]);
    s.wire(&[(30.0, 40.0), (32.0, 40.0)]);
    s.wire(&[(32.0, 40.0), (34.0, 40.0)]);
    s.wire(&[(32.0, 38.0), (32.0, 40.0)]);
    s.wire(&[(32.0, 40.0), (32.0, 44.0)]);
    s.wire(&[(32.0, 44.0), (32.0, 46.0)]);
    s.wire(&[(32.0, 50.0), (32.0, 51.0)]);
    s.label(32.0, 51.0, "OUT", LabelKind::Local, 0.0);
    s.wire(&[(18.0, 39.0), (19.0, 39.0), (19.0, 37.0)]);
    s.power(19.0, 37.0, "GND");
    s.wire(&[(32.0, 34.0), (32.0, 33.0)]);
    s.power(32.0, 33.0, "GND");
    // Emitter.
    s.wire(&[(36.0, 38.0), (36.0, 35.0)]);
    s.wire(&[(36.0, 35.0), (39.0, 35.0)]);
    s.wire(&[(36.0, 31.0), (36.0, 30.0)]);
    s.power(36.0, 30.0, "GND");
    s.wire(&[(39.0, 31.0), (39.0, 30.0)]);
    s.power(39.0, 30.0, "GND");
    // Collector up through the bias string and the load to +12V.
    s.wire(&[(36.0, 42.0), (36.0, 44.0)]);
    s.wire(&[(36.0, 44.0), (44.0, 44.0)]);
    s.wire(&[(36.0, 48.0), (36.0, 49.0)]);
    s.wire(&[(36.0, 53.0), (36.0, 55.0)]);
    s.wire(&[(36.0, 59.0), (36.0, 60.0)]);
    s.wire(&[(36.0, 59.0), (44.0, 59.0)]);
    s.wire(&[(36.0, 64.0), (36.0, 65.0)]);
    s.wire(&[(36.0, 64.0), (38.0, 64.0)]);
    s.wire(&[(42.0, 64.0), (43.0, 64.0)]);
    s.label(43.0, 64.0, "OUT", LabelKind::Local, 0.0);
    s.wire(&[(36.0, 69.0), (36.0, 70.0)]);
    s.power(36.0, 70.0, "+12V");
    // Output stage.
    s.wire(&[(46.0, 61.0), (46.0, 62.0)]);
    s.power(46.0, 62.0, "+12V");
    s.wire(&[(46.0, 57.0), (46.0, 56.0)]);
    s.wire(&[(46.0, 51.0), (46.0, 52.0)]);
    s.wire(&[(46.0, 46.0), (46.0, 47.0)]);
    s.wire(&[(46.0, 42.0), (46.0, 41.0)]);
    s.power(46.0, 41.0, "GND");
    s.wire(&[(46.0, 52.0), (48.0, 52.0)]);
    s.label(48.0, 52.0, "OUT", LabelKind::Local, 0.0);
    s.wire(&[(48.0, 52.0), (51.0, 52.0)]);
    s.wire(&[(55.0, 52.0), (56.0, 52.0)]);
    s.label(56.0, 52.0, "SPK", LabelKind::Local, 0.0);
    s.wire(&[(56.0, 52.0), (58.0, 52.0)]);
    s.wire(&[(58.0, 51.0), (58.0, 49.0)]);
    s.power(58.0, 49.0, "GND");
    // Supply.
    s.wire(&[(18.0, 24.0), (20.0, 24.0), (20.0, 26.0)]);
    s.power(20.0, 26.0, "+12V");
    s.wire(&[(18.0, 23.0), (19.0, 23.0), (19.0, 21.0)]);
    s.power(19.0, 21.0, "GND");
    for x in [24.0, 30.0] {
        s.wire(&[(x, 26.0), (x, 27.0)]);
        s.power(x, 27.0, "+12V");
        s.wire(&[(x, 22.0), (x, 21.0)]);
        s.power(x, 21.0, "GND");
    }
    s.text(16.0, 76.0, "DISCRETE CLASS-AB AUDIO AMPLIFIER", 3.0);
    s.text(16.0, 30.0, "SUPPLY", 2.0);

    // The output transistors are power types: more current gain at high currents than the
    // small-signal model (2N3904 / 2N3906) the Built-in transistors share.
    for refdes in ["Q2", "Q3"] {
        let mut c = s.ed.project().schematic.component_by_refdes(refdes).unwrap().clone();
        c.sim.params.insert("BF".into(), "150".into());
        c.sim.params.insert("IKF".into(), "1".into());
        s.ed.execute(Command::Update { objects: vec![Object::Component(c)] }).unwrap();
    }

    // Test bench: a 1 kHz sine of 0.25 V into IN (1 V for AC).
    let changes =
        s.ed.execute(Command::AddSheet { parent: None, name: "Testbench".into(), pos: g(56.0, 70.0), size: None })
            .unwrap();
    let bench = changes.changes.iter().find_map(|c| if let ObjectId::Sheet(s) = c.id { Some(s) } else { None });
    let mut t = Sch { ed: s.ed, sheet: bench };
    let v1 = t.place("VSource", "No footprint", 30.0, 40.0, 0.0, false, "V1", "");
    t.at(v1, "+", 30.0, 42.0);
    t.wire(&[(30.0, 42.0), (30.0, 44.0), (32.0, 44.0)]);
    t.label(32.0, 44.0, "IN", LabelKind::Global, 0.0);
    t.wire(&[(30.0, 38.0), (30.0, 37.0)]);
    t.power(30.0, 37.0, "GND");
    t.text(24.0, 50.0, "TEST BENCH: THE SIGNAL SOURCE", 2.0);
    let mut c = t.ed.project().schematic.components.get(v1).unwrap().clone();
    let waveform =
        Waveform::Sine { offset: 0.0, amplitude: 0.25, frequency: 1e3, delay: 0.0, damping: 0.0, phase: 0.0 };
    c.sim.source = Some(Source { waveform, ac: 1.0, ac_phase: 0.0 });
    t.ed.execute(Command::Update { objects: vec![Object::Component(c)] }).unwrap();

    let probes = |names: &[&str]| names.iter().map(|n| Probe::voltage(*n, None)).collect::<Vec<_>>();
    t.ed.execute(Command::SetSimulation {
        simulation: Simulation {
            analyses: vec![
                Analysis::new("Operating point", AnalysisKind::Op),
                Analysis {
                    probes: probes(&["IN", "/SPK"]),
                    ..Analysis::new(
                        "Transient",
                        AnalysisKind::Tran { stop: 0.1, step: Some(1e-5), start: 0.09, max_step: Some(1e-5) },
                    )
                },
                Analysis {
                    probes: probes(&["/SPK"]),
                    ..Analysis::new(
                        "Frequency response",
                        AnalysisKind::Ac { start: 10.0, stop: 1e6, points_per_decade: 20 },
                    )
                },
            ],
            supplies: vec![Supply { net: "+12V".into(), voltage: 12.0 }],
            extra: "* The 8 Ω speaker on J3\nRLOAD spk 0 8".into(),
            ..Default::default()
        },
    })
    .unwrap();
}

/// Board size (mm).
const W: f64 = 76.0;
const H: f64 = 50.0;

/// Where each footprint goes on the board: centre (mm) and rotation (°).
const PLACEMENT: [(&str, f64, f64, f64); 24] = [
    ("J1", 4.0, 30.0, 0.0),
    ("C1", 9.0, 31.27, 0.0),
    ("R1", 14.0, 31.27, 0.0),
    ("R3", 17.0, 27.0, 90.0),
    ("R2", 17.0, 36.0, 90.0),
    ("Q1", 22.0, 31.0, 0.0),
    ("C4", 21.0, 36.0, 0.0),
    ("R4", 21.0, 26.0, 90.0),
    ("C2", 25.5, 20.0, 0.0),
    ("R7", 26.0, 33.0, 90.0),
    ("D2", 26.0, 37.5, 90.0),
    ("D1", 26.0, 43.0, 90.0),
    ("R6", 30.0, 45.0, 0.0),
    ("R5", 35.0, 45.0, 0.0),
    ("C3", 33.0, 38.5, 270.0),
    ("Q2", 44.0, 40.0, 0.0),
    ("R8", 38.5, 34.0, 90.0),
    ("R9", 38.5, 28.6, 90.0),
    ("Q3", 44.0, 22.0, 0.0),
    ("C5", 58.0, 31.0, 0.0),
    ("J3", 70.0, 31.0, 90.0),
    ("J2", 7.0, 7.0, 180.0),
    ("C6", 18.0, 8.0, 90.0),
    ("C7", 25.5, 6.9, 90.0),
];

/// The centre of the (first) pad of `pin` of component `refdes`.
fn pad_at(p: &Project, refdes: &str, pin: &str) -> Point {
    let c = p.schematic.component_by_refdes(refdes).unwrap_or_else(|| panic!("no {refdes}")).id;
    let pads = pin_pad_names(p, c).into_iter().find(|(n, _)| n == pin).map(|(_, pads)| pads);
    let pad = pads.and_then(|pads| pads.first().cloned()).unwrap_or_else(|| panic!("{refdes} has no pin {pin}"));
    let fp = footprint_of(p, c);
    pad_positions(p, fp).into_iter().find(|(_, n, _)| *n == pad).expect("pad").2
}

/// A point on the board: a pad (`"R1.2"`) or coordinates in mm.
enum At {
    Pin(&'static str),
    Mm(f64, f64),
}

fn at(p: &Project, a: &At) -> Point {
    match a {
        At::Pin(pin) => {
            let (refdes, pin) = pin.split_once('.').expect("REFDES.PIN");
            pad_at(p, refdes, pin)
        }
        At::Mm(x, y) => Point::mm(*x, *y),
    }
}

use At::{Mm, Pin};

/// Lays a track along `path` (pads and waypoints) on `layer`, walked round what's in the
/// way. Panics if it can't get through (the example must stay routable).
fn track(doc: &mut Document, layer: LayerId, path: &[At]) {
    let points: Vec<Point> = path.iter().map(|a| at(doc.project(), a)).collect();
    let copper = doc.current_copper();
    let net = copper.at(points[0], layer).and_then(|c| copper.net_of(c.item)).map(str::to_string);
    let width = doc.project().board.rules.for_net(net.as_deref()).track_width;
    // Each leg between waypoints is walked round on its own, so the track keeps to them.
    let mut laid = vec![points[0]];
    for leg in points.windows(2) {
        let routed =
            router::route(doc.project(), doc.copper(), RouteMode::Walkaround, layer, width, net.as_deref(), leg, None);
        assert!(
            routed.reached,
            "track {:?} on {layer:?} blocked: {:?}",
            path.iter()
                .map(|a| match a {
                    Pin(p) => p.to_string(),
                    Mm(x, y) => format!("({x}, {y})"),
                })
                .collect::<Vec<_>>(),
            routed.blocked.map(|c| c.message)
        );
        laid.extend(routed.points.into_iter().skip(1));
    }
    doc.execute(Command::AddTrack { layer, points: laid, width: Some(width), mids: Vec::new() }).expect("track");
}

fn via(doc: &mut Document, x: f64, y: f64) {
    doc.execute(Command::AddVia { pos: Point::mm(x, y), diameter: None, drill: None, span: None }).expect("via");
}

/// Places the footprints, routes the board and pours ground on the bottom.
fn board(project: Project) -> Project {
    let mut ed = Editor::new(project);
    let corners = [(0.0, 0.0), (W, 0.0), (W, H), (0.0, H)].map(|(x, y)| Point::mm(x, y));
    ed.execute(Command::Insert {
        objects: vec![Object::BoardGraphic(BoardGraphic {
            id: Id::new(),
            layer: LayerId::OUTLINE,
            shape: Shape::Polygon { points: corners.to_vec(), width: mm(0.1), fill: false },
        })],
    })
    .unwrap();
    let mut rules = ed.project().board.rules.clone();
    rules.net_classes.push(NetClass {
        name: "Power".into(),
        track_width: Some(mm(0.8)),
        nets: vec!["+12V".into(), "GND".into(), "/OUT".into(), "/SPK".into()],
        ..Default::default()
    });
    ed.execute(Command::SetDesignRules { rules }).unwrap();
    for (refdes, x, y, rot) in PLACEMENT {
        let p = ed.project();
        let c = p.schematic.component_by_refdes(refdes).unwrap_or_else(|| panic!("no {refdes}")).id;
        let fp = footprint_of(p, c);
        move_footprint(&mut ed, fp, Point::mm(x, y), Angle::deg(rot));
    }

    let mut doc = Document::new(ed.into_project());
    let top = LayerId::TOP_COPPER;
    // Ground: small pads drop to the bottom pour through a via beside them.
    for (pin, x, y) in [("R3.1", 17.0, 24.5), ("R4.1", 21.0, 23.5), ("C7.1", 25.5, 3.6)] {
        track(&mut doc, top, &[Pin(pin), Mm(x, y)]);
        via(&mut doc, x, y);
    }
    // Input and the driver.
    track(&mut doc, top, &[Pin("J1.1"), Pin("C1.1")]);
    track(&mut doc, top, &[Pin("C1.2"), Pin("R1.1")]);
    track(&mut doc, top, &[Pin("R1.2"), Pin("Q1.B")]);
    track(&mut doc, top, &[Pin("R3.2"), Pin("R1.2")]);
    track(&mut doc, top, &[Pin("R2.1"), Pin("R1.2")]);
    track(&mut doc, top, &[Pin("C4.1"), Pin("R2.1")]);
    track(&mut doc, top, &[Pin("Q1.E"), Pin("R4.2")]);
    track(&mut doc, top, &[Pin("R4.2"), Pin("C2.+")]);
    // Collector, bias string, load.
    track(&mut doc, top, &[Pin("Q1.C"), Pin("R7.1")]);
    track(&mut doc, top, &[Pin("C4.2"), Pin("R7.1")]);
    track(&mut doc, top, &[Pin("R7.2"), Pin("D2.K")]);
    track(&mut doc, top, &[Pin("D2.A"), Pin("D1.K")]);
    track(&mut doc, top, &[Pin("D1.A"), Pin("R6.1")]);
    track(&mut doc, top, &[Pin("R6.2"), Pin("R5.1")]);
    track(&mut doc, top, &[Pin("C3.+"), Pin("R6.2")]);
    // Output stage.
    track(&mut doc, top, &[Pin("D1.A"), Pin("Q2.B")]);
    track(&mut doc, top, &[Pin("Q2.E"), Pin("R8.2")]);
    track(&mut doc, top, &[Pin("R8.1"), Pin("R9.2")]);
    track(&mut doc, top, &[Pin("Q3.E"), Mm(38.5, 19.7), Pin("R9.1")]);
    track(&mut doc, top, &[Pin("R7.1"), Pin("Q3.B")]);
    track(&mut doc, top, &[Pin("R8.1"), Pin("C5.+")]);
    track(&mut doc, top, &[Pin("C3.-"), Pin("R8.1")]);
    track(&mut doc, top, &[Pin("C5.-"), Pin("J3.1")]);
    // The feedback crosses the bias string on the bottom.
    track(&mut doc, top, &[Pin("R2.2"), Mm(17.0, 39.0)]);
    via(&mut doc, 17.0, 39.0);
    track(&mut doc, LayerId::BOTTOM_COPPER, &[Mm(17.0, 39.0), Mm(30.0, 37.0)]);
    via(&mut doc, 30.0, 37.0);
    track(&mut doc, top, &[Mm(30.0, 37.0), Pin("C3.-")]);
    // Supply.
    track(&mut doc, top, &[Pin("J2.1"), Pin("C6.+")]);
    track(&mut doc, top, &[Pin("C6.+"), Pin("C7.2")]);
    // The supply passes under the output on the bottom.
    let c7 = pad_at(doc.project(), "C7", "2");
    let y = c7.to_mm()[1];
    track(&mut doc, top, &[Pin("C7.2"), Mm(52.0, y)]);
    via(&mut doc, 52.0, y);
    track(&mut doc, LayerId::BOTTOM_COPPER, &[Mm(52.0, y), Mm(52.0, 40.0)]);
    via(&mut doc, 52.0, 40.0);
    track(&mut doc, top, &[Mm(52.0, 40.0), Pin("Q2.C")]);
    track(&mut doc, top, &[Pin("R5.2"), Pin("Q2.C")]);
    // Q3's collector to ground.
    track(&mut doc, top, &[Pin("Q3.C"), Mm(47.15, 22.0)]);
    via(&mut doc, 47.15, 22.0);

    let inset = [(0.5, 0.5), (W - 0.5, 0.5), (W - 0.5, H - 0.5), (0.5, H - 0.5)];
    let outline = inset.map(|(x, y)| Point::mm(x, y)).to_vec();
    doc.execute(Command::Insert { objects: vec![Object::Zone(Zone::new(LayerId::BOTTOM_COPPER, "GND", outline))] })
        .unwrap();
    doc.project().clone()
}

/// The audio amplifier example project.
pub fn amp_project() -> Project {
    let mut ed = Editor::new(Project::new("Audio amplifier"));
    schematic(&mut ed);
    board(ed.into_project())
}
