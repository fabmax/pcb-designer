//! Design rule check on a small board with seeded errors.

use editor_core::connectivity::Connectivity;
use editor_core::copper::BoardConnectivity;
use editor_core::drc::{self, ViolationKind as K};
use editor_core::query::{pad_positions, pin_positions};
use editor_core::{Command, Editor};
use model::builtin::{self, PART_RESISTOR};
use model::*;

fn place(ed: &mut Editor, x: f64, at: Point) -> (Id, Id) {
    let c = ed
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
    let sym = c.ids().find_map(|id| if let ObjectId::Symbol(s) = id { Some(s) } else { None }).unwrap();
    let fp = c.ids().find_map(|id| if let ObjectId::Footprint(f) = id { Some(f) } else { None }).unwrap();
    let pos = ed.project().board.footprints.get(fp).unwrap().pos;
    ed.execute(Command::Move { ids: vec![ObjectId::Footprint(fp)], delta: at - pos, keep_connected: false }).unwrap();
    (sym, fp)
}

fn pad(ed: &Editor, f: Id, name: &str) -> Point {
    pad_positions(ed.project(), f).into_iter().find(|p| p.1 == name).unwrap().2
}

fn check(ed: &Editor) -> Vec<drc::Violation> {
    let p = ed.project();
    drc::run(p, &BoardConnectivity::build(p, &Connectivity::build(p)))
}

fn kinds(v: &[drc::Violation]) -> Vec<K> {
    v.iter().map(|v| v.kind).collect()
}

/// R1 and R2 in series on a 30 × 20 mm board.
fn board() -> (Editor, [Id; 2]) {
    let mut ed = Editor::new(Project::new("drc"));
    ed.execute(Command::ImportParts { library: builtin::starter_library(), parts: vec![PART_RESISTOR] }).unwrap();
    let (s1, f1) = place(&mut ed, 0.0, Point::mm(10.0, 10.0));
    let (s2, f2) = place(&mut ed, 20.32, Point::mm(20.0, 10.0));
    let pin = |ed: &Editor, s, n| pin_positions(ed.project(), s).into_iter().find(|p| p.1 == n).unwrap().2;
    let (a, b) = (pin(&ed, s1, "2"), pin(&ed, s2, "1"));
    ed.execute(Command::AddWire { sheet: None, points: vec![a, b] }).unwrap();
    let outline = Shape::rect(Point::mm(0.0, 0.0), Point::mm(30.0, 20.0), 100_000);
    let g = BoardGraphic { id: Id::new(), layer: LayerId::OUTLINE, shape: outline };
    ed.execute(Command::Insert { objects: vec![Object::BoardGraphic(g)] }).unwrap();
    (ed, [f1, f2])
}

#[test]
fn clean_board_after_routing() {
    let (mut ed, [f1, f2]) = board();
    assert_eq!(kinds(&check(&ed)), vec![K::Unrouted]);
    let (a, b) = (pad(&ed, f1, "2"), pad(&ed, f2, "1"));
    ed.execute(Command::AddTrack { layer: LayerId::TOP_COPPER, points: vec![a, b], width: None }).unwrap();
    assert_eq!(check(&ed), vec![]);
}

#[test]
fn finds_seeded_errors() {
    let (mut ed, [f1, f2]) = board();
    let (p12, p21, p11) = (pad(&ed, f1, "2"), pad(&ed, f2, "1"), pad(&ed, f1, "1"));
    let top = LayerId::TOP_COPPER;
    ed.execute(Command::AddTrack { layer: top, points: vec![p12, p21], width: None }).unwrap();
    // 1. A stub of R1.2's net passing 0.1 mm above R1.1 (pad top edge at y + 0.5 mm).
    let y = p11.y + mm(0.5) + mm(0.125) + mm(0.1);
    ed.execute(Command::AddTrack {
        layer: top,
        points: vec![p12, Point::new(p12.x, y), Point::new(p11.x, y)],
        width: None,
    })
    .unwrap();
    // 2. A thin, dangling track.
    let thin = vec![Point::mm(5.0, 15.0), Point::mm(8.0, 15.0)];
    ed.execute(Command::AddTrack { layer: top, points: thin, width: Some(mm(0.1)) }).unwrap();
    // 3. A via with a tiny drill, 4. one at the board edge (both dangling).
    ed.execute(Command::AddVia { pos: Point::mm(25.0, 15.0), diameter: None, drill: Some(mm(0.1)) }).unwrap();
    ed.execute(Command::AddVia { pos: Point::mm(0.2, 5.0), diameter: None, drill: None }).unwrap();
    // 5. R3 overlapping R2's courtyard (pads keep their clearance).
    let (_, f3) = place(&mut ed, 40.0, Point::mm(20.0, 11.3));
    let v = check(&ed);
    let count = |k| v.iter().filter(|x| x.kind == k).count();
    assert_eq!(count(K::Clearance), 1);
    let c = v.iter().find(|x| x.kind == K::Clearance).unwrap();
    assert!(c.message.contains("R1.1") && c.message.contains("0.100 mm"), "{}", c.message);
    assert!(c.objects.contains(&ObjectId::Footprint(f1)));
    assert_eq!(count(K::TrackWidth), 1);
    assert_eq!(count(K::Drill), 1);
    assert_eq!(count(K::BoardEdge), 1);
    assert_eq!(count(K::Courtyard), 1);
    let cy = v.iter().find(|x| x.kind == K::Courtyard).unwrap();
    assert!(cy.objects.contains(&ObjectId::Footprint(f2)) && cy.objects.contains(&ObjectId::Footprint(f3)));
    assert_eq!(count(K::Unconnected), 3, "thin track and both vias");
    assert_eq!(count(K::Unrouted), 0);
    assert_eq!(count(K::Short), 0);
    // Most severe first.
    assert!(v.windows(2).all(|w| w[0].kind <= w[1].kind));
}

#[test]
fn shorts_are_violations() {
    let (mut ed, [f1, f2]) = board();
    // R1.1 (unnamed net of its own) to R2.1.
    let (a, b) = (pad(&ed, f1, "1"), pad(&ed, f2, "1"));
    let y = a.y - mm(2.0);
    let pts = vec![a, Point::new(a.x, y), Point::new(b.x, y), b];
    ed.execute(Command::AddTrack { layer: LayerId::TOP_COPPER, points: pts, width: None }).unwrap();
    let v = check(&ed);
    let s: Vec<_> = v.iter().filter(|x| x.kind == K::Short).collect();
    assert_eq!(s.len(), 1, "{v:?}");
    assert_eq!(s[0].objects.len(), 3, "the three track segments");
}
