//! Arc tracks: geometry, copper, picking, transforms and Gerber export.

use editor_core::Command;
use editor_core::copper::{ARC_ERROR, CopperItem};
use gerber::FileKind;
use gerber::reader::parse_gerber;
use model::board::arc_through;
use model::*;
use view::demo::demo_project;
use view::{Document, ViewId};

#[test]
fn arc_through_three_points() {
    // A quarter circle around the origin, counterclockwise.
    let r = 10_000_000.0;
    let m = Point::new((r * std::f64::consts::FRAC_1_SQRT_2) as Coord, (r * std::f64::consts::FRAC_1_SQRT_2) as Coord);
    let g = arc_through(Point::mm(10.0, 0.0), m, Point::mm(0.0, 10.0)).unwrap();
    // The mid point is rounded to the nanometre: a few nanometres off.
    assert!(g.center.0.abs() < 5.0 && g.center.1.abs() < 5.0, "{g:?}");
    assert!((g.radius - r).abs() < 5.0);
    assert!((g.sweep - std::f64::consts::FRAC_PI_2).abs() < 1e-6);
    // The other way round: the three-quarter arc through the opposite side, clockwise.
    let g = arc_through(Point::mm(10.0, 0.0), Point::mm(-10.0, 0.0), Point::mm(0.0, 10.0)).unwrap();
    assert!((g.sweep + 1.5 * std::f64::consts::PI).abs() < 1e-6, "{}", g.sweep);
    // On a line: straight.
    assert!(arc_through(Point::mm(0.0, 0.0), Point::mm(1.0, 0.0), Point::mm(2.0, 0.0)).is_none());

    let t = Track {
        mid: Some(m),
        ..Track::straight(LayerId::TOP_COPPER, Point::mm(10.0, 0.0), Point::mm(0.0, 10.0), mm(0.25))
    };
    let path = t.path(ARC_ERROR);
    assert_eq!((path[0], *path.last().unwrap()), (t.a, t.b));
    for w in path.windows(2) {
        let c = Point::new((w[0].x + w[1].x) / 2, (w[0].y + w[1].y) / 2);
        assert!(r - (c - Point::ZERO).length() <= ARC_ERROR as f64 + 2.0, "chord sagitta");
    }
    assert!((t.length() - r * std::f64::consts::FRAC_PI_2).abs() < 20.0);
}

/// The demo with an arc track on the top layer, near the bottom edge: from (20, 3) over
/// (25, 6) to (30, 3) mm. Returns its id.
fn demo_with_arc() -> (Document, Id) {
    let mut doc = Document::new(demo_project());
    let t = Track {
        mid: Some(Point::mm(25.0, 6.0)),
        ..Track::straight(LayerId::TOP_COPPER, Point::mm(20.0, 3.0), Point::mm(30.0, 3.0), mm(0.3))
    };
    let id = t.id;
    doc.execute(Command::Insert { objects: vec![Object::Track(t)] }).unwrap();
    (doc, id)
}

#[test]
fn arc_copper_follows_the_arc() {
    let (mut doc, id) = demo_with_arc();
    let chord_mid = Point::mm(25.0, 3.0);
    let copper = doc.current_copper();
    let c = copper.get(CopperItem::Track(id)).unwrap();
    assert!(c.geo.contains(Point::mm(25.0, 6.0)));
    assert!(c.geo.contains(Point::mm(25.0, 6.14)), "the width around the arc");
    assert!(!c.geo.contains(chord_mid));
    assert_eq!(c.anchors, vec![Point::mm(20.0, 3.0), Point::mm(30.0, 3.0)], "ends connect as for straight tracks");

    let view = doc.view(ViewId::Board);
    assert_eq!(view.hit(doc.project(), Point::mm(25.0, 6.0), mm(0.05)), Some(ObjectId::Track(id)));
    assert_ne!(view.hit(doc.project(), chord_mid, mm(0.05)), Some(ObjectId::Track(id)));

    // Rotating keeps it an arc of the same radius.
    let radius = |doc: &Document| match doc.project().board.tracks.get(id).unwrap().arc() {
        Some(Shape::Arc { radius, .. }) => radius,
        _ => panic!("not an arc"),
    };
    let r = radius(&doc);
    doc.execute(Command::Rotate {
        ids: vec![ObjectId::Track(id)],
        center: Point::mm(25.0, 3.0),
        angle: Angle::DEG90,
        keep_connected: false,
    })
    .unwrap();
    assert!((radius(&doc) - r).abs() <= 1);
}

#[test]
fn arcs_are_exported_as_arcs() {
    let (mut doc, id) = demo_with_arc();
    let copper = doc.current_copper().clone();
    let files = gerber::generate(doc.project(), &copper).unwrap();
    let top = &files.iter().find(|f| f.kind == FileKind::Layer(LayerId::TOP_COPPER)).unwrap().content;
    assert!(top.contains("G03") || top.contains("G02"), "an arc command");
    let img = parse_gerber(top).unwrap();
    let t = doc.project().board.tracks.get(id).unwrap();
    for p in t.path(10_000) {
        assert!(img.covers(p), "{p:?}");
    }
    assert!(!img.covers(Point::mm(25.0, 3.0)), "not the chord");
}
