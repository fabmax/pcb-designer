//! Board outlines drawn as separate lines and arcs (as KiCad draws them) close into
//! loops; loops inside others are cutouts.

use editor_core::connectivity::Connectivity;
use editor_core::copper::{BoardConnectivity, CopperItem};
use editor_core::{Command, Editor};
use model::*;

fn outline(ed: &mut Editor, shape: Shape) {
    let g = BoardGraphic { id: Id::new(), layer: LayerId::OUTLINE, shape };
    ed.execute(Command::Insert { objects: vec![Object::BoardGraphic(g)] }).unwrap();
}

fn arc(cx: f64, cy: f64, r: f64, start: f64, sweep: f64) -> Shape {
    Shape::Arc { center: Point::mm(cx, cy), radius: mm(r), start: Angle::deg(start), sweep, width: 100_000 }
}

#[test]
fn lines_and_arcs_close_into_a_rounded_board_with_a_cutout() {
    let mut ed = Editor::new(Project::new("t"));
    // 20 × 10 mm with 2 mm corner radii, shapes in no particular order or direction.
    let l = |a: (f64, f64), b: (f64, f64)| Shape::line(Point::mm(a.0, a.1), Point::mm(b.0, b.1), 100_000);
    outline(&mut ed, l((2.0, 0.0), (18.0, 0.0)));
    outline(&mut ed, arc(18.0, 8.0, 2.0, 0.0, 90.0));
    outline(&mut ed, l((0.0, 8.0), (0.0, 2.0)));
    outline(&mut ed, arc(2.0, 2.0, 2.0, 180.0, 90.0));
    outline(&mut ed, l((18.0, 10.0), (2.0, 10.0)));
    outline(&mut ed, arc(18.0, 2.0, 2.0, 270.0, 90.0));
    outline(&mut ed, l((20.0, 2.0), (20.0, 8.0)));
    outline(&mut ed, arc(2.0, 8.0, 2.0, 180.0, -90.0));
    // A round cutout of two half arcs.
    outline(&mut ed, arc(10.0, 5.0, 1.5, 0.0, 180.0));
    outline(&mut ed, arc(10.0, 5.0, 1.5, 180.0, 180.0));

    let area = editor_core::zones::board_area(&ed.project().board).unwrap();
    assert_eq!(area.len(), 2, "outline and cutout");

    // A zone without a net over everything fills the board but not the cutout or corners.
    let big = vec![Point::mm(-5.0, -5.0), Point::mm(25.0, -5.0), Point::mm(25.0, 15.0), Point::mm(-5.0, 15.0)];
    let zone = Zone::new(LayerId::TOP_COPPER, "", big);
    let z = zone.id;
    ed.execute(Command::Insert { objects: vec![Object::Zone(zone)] }).unwrap();
    let p = ed.project();
    let bc = BoardConnectivity::build(p, &Connectivity::build(p));
    let filled = |q: Point| {
        bc.items.iter().any(|c| matches!(c.item, CopperItem::Zone { zone, .. } if zone == z) && c.geo.contains(q))
    };
    assert!(filled(Point::mm(5.0, 5.0)));
    assert!(filled(Point::mm(10.0, 2.0)), "beside the cutout");
    assert!(!filled(Point::mm(10.0, 5.0)), "in the cutout");
    assert!(!filled(Point::mm(0.3, 0.3)), "outside the rounded corner");
    assert!(filled(Point::mm(2.0, 0.5)), "inside along the edge");

    // DRC: a via in the cutout is outside the board.
    ed.execute(Command::AddVia { pos: Point::mm(10.0, 5.0), diameter: None, drill: None, span: None }).unwrap();
    let p = ed.project();
    let bc = BoardConnectivity::build(p, &Connectivity::build(p));
    let v = editor_core::drc::run(p, &bc);
    assert!(v.iter().any(|v| v.kind == editor_core::drc::ViolationKind::BoardEdge), "{v:?}");
}
