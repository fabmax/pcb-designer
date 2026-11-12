//! Generated projects: a small demo circuit and large stress-test designs.

use editor_core::query::{pad_positions, pin_positions};
use editor_core::{Command, Editor};
use model::builtin::{self, PART_CAPACITOR, PART_HEADER_1X4, PART_LED, PART_RESISTOR};
use model::*;

fn place(ed: &mut Editor, part: Id, x: f64, y: f64, rotation: Angle, refdes: &str, value: &str) -> Id {
    place_on(ed, None, part, x, y, rotation, refdes, value)
}

#[allow(clippy::too_many_arguments)]
fn place_on(
    ed: &mut Editor,
    sheet: Option<Id>,
    part: Id,
    x: f64,
    y: f64,
    rotation: Angle,
    refdes: &str,
    value: &str,
) -> Id {
    ed.execute(Command::PlacePart {
        part,
        variant: None,
        sheet,
        pos: Point::mm(x, y),
        rotation,
        mirror: false,
        refdes: Some(refdes.into()),
        value: Some(value.into()),
    })
    .expect("place part");
    ed.project().schematic.component_by_refdes(refdes).expect("placed").id
}

fn symbol_of(p: &Project, component: Id) -> Id {
    p.schematic.symbols_of(component).next().expect("symbol").id
}

fn footprint_of(p: &Project, component: Id) -> Id {
    p.board.footprint_of(component).expect("footprint").id
}

/// Wire stub from `pin` of `component` pointing away from the symbol, ending in a label
/// (a power port for GND and supplies, else a local label).
fn stub_label(ed: &mut Editor, component: Id, pin: &str, net: &str) {
    let power = net == "GND" || net.starts_with('+') || net.starts_with("VCC");
    stub(ed, component, pin, net, if power { LabelKind::Power } else { LabelKind::Local });
}

/// Wire stub from `pin` of `component` pointing away from the symbol, ending in a label.
fn stub(ed: &mut Editor, component: Id, pin: &str, net: &str, kind: LabelKind) {
    let sheet = ed.project().schematic.symbols_of(component).next().map(|s| s.sheet);
    let p = ed.project();
    let sym = symbol_of(p, component);
    let center = p.schematic.symbols.get(sym).unwrap().pos;
    let (_, _, at) = pin_positions(p, sym).into_iter().find(|(_, name, _)| name == pin).expect("pin");
    let d = at - center;
    let step = if d.x.abs() >= d.y.abs() {
        Point::new(d.x.signum() * mm(2.54), 0)
    } else {
        Point::new(0, d.y.signum() * mm(2.54))
    };
    let end = at + step;
    ed.execute(Command::AddWire { sheet, points: vec![at, end] }).unwrap();
    // Labels on the left of their wire face left.
    let rotation = if step.x < 0 && kind != LabelKind::Power { Angle::DEG180 } else { Angle::ZERO };
    ed.execute(Command::AddLabel { sheet, pos: end, name: net.into(), kind, shape: LabelShape::Input, rotation })
        .unwrap();
}

fn pad(p: &Project, footprint: Id, name: &str) -> Point {
    pad_positions(p, footprint).into_iter().find(|(_, n, _)| n == name).expect("pad").2
}

fn move_footprint(ed: &mut Editor, footprint: Id, to: Point, rotation: Angle) {
    let from = ed.project().board.footprints.get(footprint).unwrap().pos;
    let ids = vec![ObjectId::Footprint(footprint)];
    ed.execute(Command::Move { ids: ids.clone(), delta: to - from, keep_connected: false }).unwrap();
    if rotation != Angle::ZERO {
        ed.execute(Command::Rotate { ids, center: to, angle: rotation, keep_connected: false }).unwrap();
    }
}

/// A small LED circuit with a board outline, placed footprints, tracks, a via and a zone.
/// The LED and its resistor are on a sub-sheet, "Indicator", fed through its pin IN.
pub fn demo_project() -> Project {
    let mut ed = Editor::new(Project::new("Demo"));
    let parts = vec![PART_RESISTOR, PART_CAPACITOR, PART_LED, PART_HEADER_1X4];
    ed.execute(Command::import_parts(&builtin::builtin_library(), &parts).unwrap()).unwrap();

    // Schematic.
    let j1 = place(&mut ed, PART_HEADER_1X4, 0.0, 0.0, Angle::ZERO, "J1", "Conn");
    let changes = ed
        .execute(Command::AddSheet { parent: None, name: "Indicator".into(), pos: Point::mm(25.4, 5.08), size: None })
        .unwrap();
    let indicator = changes.changes.iter().find_map(|c| if let ObjectId::Sheet(s) = c.id { Some(s) } else { None });
    let r1 = place_on(&mut ed, indicator, PART_RESISTOR, 25.4, 7.62, Angle::ZERO, "R1", "330");
    let d1 = place_on(&mut ed, indicator, PART_LED, 45.72, 7.62, Angle::ZERO, "D1", "red");
    let c1 = place(&mut ed, PART_CAPACITOR, 25.4, -7.62, Angle::DEG90, "C1", "100n");
    let r2 = place(&mut ed, PART_RESISTOR, 45.72, -7.62, Angle::DEG90, "R2", "10k");
    // R1 pin 2 to the LED anode with a direct wire.
    let (a, b) = {
        let p = ed.project();
        let r = pin_positions(p, symbol_of(p, r1)).into_iter().find(|x| x.1 == "2").unwrap().2;
        let d = pin_positions(p, symbol_of(p, d1)).into_iter().find(|x| x.1 == "A").unwrap().2;
        (r, d)
    };
    ed.execute(Command::AddWire { sheet: indicator, points: vec![a, b] }).unwrap();
    for (c, pin, net) in [
        (j1, "1", "+5V"),
        (j1, "2", "SIG"),
        (j1, "4", "GND"),
        (d1, "K", "GND"),
        // Rotated by 90°: pin 2 is on top, pin 1 at the bottom.
        (c1, "2", "+5V"),
        (c1, "1", "GND"),
        (r2, "2", "SIG"),
        (r2, "1", "GND"),
    ] {
        stub_label(&mut ed, c, pin, net);
    }
    // The indicator's supply comes in through a hierarchical label (the sheet's pin IN),
    // wired to +5V on the root sheet.
    stub(&mut ed, r1, "1", "IN", LabelKind::Hierarchical);
    let p = ed.project();
    let indicator = indicator.unwrap();
    let pin = p.schematic.sheet_symbol(indicator).unwrap().1.into_iter().find(|p| p.name == "IN").unwrap().pos;
    let end = pin - Point::mm(5.08, 0.0);
    ed.execute(Command::AddWire { sheet: None, points: vec![pin, end] }).unwrap();
    ed.execute(Command::AddLabel {
        sheet: None,
        pos: end,
        name: "+5V".into(),
        kind: LabelKind::Power,
        shape: LabelShape::Input,
        rotation: Angle::ZERO,
    })
    .unwrap();
    ed.execute(Command::Insert {
        objects: vec![Object::SchText(SchText {
            id: Id::new(),
            sheet: ed.project().schematic.root_sheet().unwrap(),
            text: Text::new("LED DEMO", Point::mm(0.0, 17.78), mm(2.54)),
        })],
    })
    .unwrap();

    // Board.
    ed.execute(Command::Insert {
        objects: vec![Object::BoardGraphic(BoardGraphic {
            id: Id::new(),
            layer: LayerId::OUTLINE,
            shape: Shape::rect(Point::mm(0.0, 0.0), Point::mm(40.0, 30.0), mm(0.1)),
        })],
    })
    .unwrap();
    let p = ed.project();
    let (fj1, fr1, fd1, fc1, fr2) =
        (footprint_of(p, j1), footprint_of(p, r1), footprint_of(p, d1), footprint_of(p, c1), footprint_of(p, r2));
    move_footprint(&mut ed, fj1, Point::mm(5.0, 15.0), Angle::ZERO);
    move_footprint(&mut ed, fr1, Point::mm(18.0, 20.0), Angle::ZERO);
    move_footprint(&mut ed, fd1, Point::mm(30.0, 20.0), Angle::DEG180);
    move_footprint(&mut ed, fc1, Point::mm(18.0, 10.0), Angle::DEG90);
    move_footprint(&mut ed, fr2, Point::mm(30.0, 10.0), Angle::DEG90);

    let p = ed.project();
    let tracks = [
        (pad(p, fj1, "1"), pad(p, fr1, "1")),
        (pad(p, fr1, "2"), pad(p, fd1, "2")),
        (pad(p, fj1, "2"), pad(p, fr2, "2")),
    ];
    for (a, b) in tracks {
        let mid = Point::new(b.x, a.y);
        ed.execute(Command::AddTrack { layer: LayerId::TOP_COPPER, points: vec![a, mid, b], width: None }).unwrap();
    }
    let gnd = pad(ed.project(), fj1, "4");
    let via = Point::new(gnd.x + mm(4.0), gnd.y);
    ed.execute(Command::AddTrack { layer: LayerId::TOP_COPPER, points: vec![gnd, via], width: Some(mm(0.4)) }).unwrap();
    ed.execute(Command::AddVia { pos: via, diameter: None, drill: None }).unwrap();
    ed.execute(Command::Insert {
        objects: vec![Object::Zone(Zone {
            id: Id::new(),
            layer: LayerId::BOTTOM_COPPER,
            net: "GND".into(),
            outline: vec![Point::mm(1.0, 1.0), Point::mm(39.0, 1.0), Point::mm(39.0, 29.0), Point::mm(1.0, 29.0)],
            priority: 0,
            clearance: None,
        })],
    })
    .unwrap();
    ed.into_project()
}

/// `n` resistors/capacitors in a grid, connected in chains by wires and labels, with
/// their footprints spread over a board. For performance testing.
pub fn stress_project(n: usize) -> Project {
    let mut ed = Editor::new(Project::new(format!("Stress {n}")));
    let parts = vec![PART_RESISTOR, PART_CAPACITOR];
    ed.execute(Command::import_parts(&builtin::builtin_library(), &parts).unwrap()).unwrap();
    let cols = (n as f64).sqrt().ceil().max(1.0) as usize;
    let board_w = cols as f64 * 4.0;
    ed.execute(Command::Insert {
        objects: vec![Object::BoardGraphic(BoardGraphic {
            id: Id::new(),
            layer: LayerId::OUTLINE,
            shape: Shape::rect(Point::mm(-2.0, -2.0), Point::mm(board_w, board_w), mm(0.1)),
        })],
    })
    .unwrap();
    for i in 0..n {
        let (col, row) = ((i % cols) as f64, (i / cols) as f64);
        let (part, refdes) =
            if i % 3 == 2 { (PART_CAPACITOR, format!("C{}", i + 1)) } else { (PART_RESISTOR, format!("R{}", i + 1)) };
        let (x, y) = (col * 20.32, row * 10.16);
        let c = place(&mut ed, part, x, y, Angle::ZERO, &refdes, "1k");
        // Wire from pin 2 to the next column's pin 1, labels at the row ends.
        let right = Point::mm(x + 5.08, y);
        if col as usize + 1 < cols && i + 1 < n {
            ed.execute(Command::AddWire { sheet: None, points: vec![right, Point::mm(x + 15.24, y)] }).unwrap();
        } else {
            let name = if (row as usize).is_multiple_of(2) { "GND".to_string() } else { format!("N{}", row as usize) };
            let kind = if name == "GND" { LabelKind::Power } else { LabelKind::Local };
            ed.execute(Command::AddLabel {
                sheet: None,
                pos: right,
                name,
                kind,
                shape: LabelShape::default(),
                rotation: Angle::ZERO,
            })
            .unwrap();
        }
        let f = footprint_of(ed.project(), c);
        let to = Point::mm(col * 4.0, row * 4.0);
        let from = ed.project().board.footprints.get(f).unwrap().pos;
        ed.execute(Command::Move { ids: vec![ObjectId::Footprint(f)], delta: to - from, keep_connected: false })
            .unwrap();
    }
    ed.into_project()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn demo_is_valid() {
        let p = demo_project();
        assert_eq!(p.validate(), Vec::<String>::new());
        assert_eq!(p.schematic.components.len(), 5);
        assert_eq!(p.board.footprints.len(), 5);
        assert!(p.board.tracks.len() >= 4);
    }

    #[test]
    fn stress_is_valid() {
        let p = stress_project(50);
        assert_eq!(p.validate(), Vec::<String>::new());
        assert_eq!(p.board.footprints.len(), 50);
    }
}

/// Id of [`demo_library`] (fixed, so re-writing the demo file keeps the same library).
pub const DEMO_LIBRARY_ID: Id = Id::from_u128(0xde11_0000_0000_4000_8000_0000_0000_0001);

/// A small library for the demo project's External Libraries: the built-in diode, NPN
/// and op amp, with ids of their own (so it is a library in its own right).
pub fn demo_library() -> Library {
    let starter = builtin::builtin_library();
    let keep = [builtin::PART_DIODE, builtin::PART_NPN, builtin::PART_OPAMP];
    let mut lib = Library { id: Some(DEMO_LIBRARY_ID), name: "Demo parts".into(), ..Default::default() };
    for part in starter.parts.iter().filter(|p| keep.contains(&p.id)) {
        for u in &part.units {
            if !lib.symbols.contains(u.symbol) {
                lib.symbols.insert(starter.symbols.get(u.symbol).unwrap().clone());
            }
        }
        for f in part.variants.iter().filter_map(|v| v.footprint) {
            if !lib.footprints.contains(f) {
                lib.footprints.insert(starter.footprints.get(f).unwrap().clone());
            }
        }
        lib.parts.insert(part.clone());
    }
    // Move every built-in id (parts, symbols, pins, pads, …) into the demo namespace.
    use model::io::{FileKind, MIGRATIONS, from_file_str, to_file_string};
    let text = to_file_string(FileKind::Library, &lib).unwrap().replace("\"5ce10000-", "\"de110001-");
    let mut lib: Library = from_file_str(&text, FileKind::Library, MIGRATIONS, std::path::Path::new("demo")).unwrap();
    lib.id = Some(DEMO_LIBRARY_ID);
    lib
}

#[cfg(test)]
mod demo_library_tests {
    use super::*;

    #[test]
    fn demo_library_is_valid_and_shares_no_ids_with_the_builtin_library() {
        let lib = demo_library();
        assert_eq!(lib.validate(), Vec::<String>::new());
        assert_eq!(lib.parts.len(), 3);
        let starter = builtin::builtin_library();
        assert!(lib.parts.iter().all(|p| !starter.parts.contains(p.id)));
        assert!(lib.symbols.iter().all(|s| !starter.symbols.contains(s.id)));
        assert!(lib.footprints.iter().all(|f| !starter.footprints.contains(f.id)));
    }
}
