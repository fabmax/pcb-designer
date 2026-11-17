//! Zone cutouts (AddCutout) and clearance measuring between board features.

use editor_core::clearance::{self, Feature};
use editor_core::connectivity::Connectivity;
use editor_core::copper::BoardConnectivity;
use editor_core::{Command, Editor};
use model::*;

fn square(x0: f64, y0: f64, x1: f64, y1: f64) -> Vec<Point> {
    vec![Point::mm(x0, y0), Point::mm(x1, y0), Point::mm(x1, y1), Point::mm(x0, y1)]
}

fn zone_editor() -> (Editor, Id) {
    let mut ed = Editor::new(Project::new("t"));
    let zone = Zone::new(LayerId::TOP_COPPER, "", square(0.0, 0.0, 10.0, 10.0));
    let id = zone.id;
    ed.execute(Command::Insert { objects: vec![Object::Zone(zone)] }).unwrap();
    (ed, id)
}

fn cut(ed: &mut Editor, z: Id, outline: Vec<Point>) -> Result<(), String> {
    ed.execute(Command::AddCutout { id: ObjectId::Zone(z), outline }).map(|_| ()).map_err(|e| e.to_string())
}

#[test]
fn a_cutout_inside_becomes_a_hole_and_merges_with_holes_it_overlaps() {
    let (mut ed, z) = zone_editor();
    cut(&mut ed, z, square(2.0, 2.0, 4.0, 4.0)).unwrap();
    assert_eq!(ed.project().board.zones.get(z).unwrap().holes.len(), 1);
    cut(&mut ed, z, square(3.0, 3.0, 5.0, 5.0)).unwrap();
    let zone = ed.project().board.zones.get(z).unwrap();
    assert_eq!(zone.holes.len(), 1, "merged with the first hole");
    assert!(zone.holes[0].len() > 4, "an L-shaped hole: {:?}", zone.holes[0]);
    ed.undo();
    ed.undo();
    assert!(ed.project().board.zones.get(z).unwrap().holes.is_empty());
}

#[test]
fn a_cutout_across_the_edge_trims_the_outline() {
    let (mut ed, z) = zone_editor();
    cut(&mut ed, z, square(8.0, -1.0, 12.0, 11.0)).unwrap();
    let zone = ed.project().board.zones.get(z).unwrap();
    assert!(zone.holes.is_empty());
    let max_x = zone.outline.iter().map(|p| p.x).max().unwrap();
    assert_eq!(max_x, mm(8.0));
}

#[test]
fn cutouts_that_split_miss_or_remove_the_zone_are_refused() {
    let (mut ed, z) = zone_editor();
    let split = cut(&mut ed, z, square(4.0, -1.0, 6.0, 11.0)).unwrap_err();
    assert!(split.contains("pieces"), "{split}");
    let miss = cut(&mut ed, z, square(20.0, 20.0, 22.0, 22.0)).unwrap_err();
    assert!(miss.contains("doesn't overlap"), "{miss}");
    let all = cut(&mut ed, z, square(-1.0, -1.0, 11.0, 11.0)).unwrap_err();
    assert!(all.contains("whole"), "{all}");
    let r = ed.execute(Command::AddCutout { id: ObjectId::Track(Id::new()), outline: square(0.0, 0.0, 1.0, 1.0) });
    assert!(r.is_err());
}

#[test]
fn the_gap_between_tracks_and_to_the_board_edge() {
    let mut ed = Editor::new(Project::new("t"));
    let g = BoardGraphic {
        id: Id::new(),
        layer: LayerId::OUTLINE,
        shape: Shape::rect(Point::ZERO, Point::mm(20.0, 10.0), 100_000),
    };
    let a = Track::straight(LayerId::TOP_COPPER, Point::mm(2.0, 2.0), Point::mm(10.0, 2.0), mm(0.2));
    let b = Track::straight(LayerId::TOP_COPPER, Point::mm(2.0, 3.0), Point::mm(10.0, 3.0), mm(0.4));
    let (ta, tb) = (a.id, b.id);
    ed.execute(Command::Insert { objects: vec![Object::BoardGraphic(g), Object::Track(a), Object::Track(b)] }).unwrap();
    let p = ed.project();
    let bc = BoardConnectivity::build(p, &Connectivity::build(p));
    // Picking: the track under the pointer, the edge near it.
    let tol = mm(0.1);
    assert_eq!(
        clearance::feature_at(p, &bc, Point::mm(5.0, 2.05), LayerId::TOP_COPPER, tol),
        Some(Feature::Track { id: ta })
    );
    assert_eq!(clearance::feature_at(p, &bc, Point::mm(5.0, 9.95), LayerId::TOP_COPPER, tol), Some(Feature::Edge));
    assert_eq!(clearance::feature_at(p, &bc, Point::mm(5.0, 6.0), LayerId::TOP_COPPER, tol), None);

    // 1 mm between the axes, minus 0.1 and 0.2 mm half widths. Tracks without a net:
    // the clearance rule applies.
    let gap = clearance::gap(p, &bc, Feature::Track { id: ta }, Feature::Track { id: tb }).unwrap();
    assert!((gap.distance - 0.7e6).abs() < 2.0, "{gap:?}");
    assert_eq!((gap.a.y, gap.b.y), (mm(2.1), mm(2.8)));
    assert_eq!(gap.rule, Some(p.board.rules.clearance));
    assert!(!gap.violates());
    let edge = clearance::gap(p, &bc, Feature::Track { id: ta }, Feature::Edge).unwrap();
    assert!((edge.distance - 1.9e6).abs() < 2.0, "{edge:?}");
    assert_eq!(edge.b, Point::mm(edge.a.x as f64 * 1e-6, 0.0));
    assert_eq!(clearance::name(p, &bc, Feature::Edge), "board edge");
}
