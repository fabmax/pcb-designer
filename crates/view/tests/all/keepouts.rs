//! Keepouts on the board: drawn hatched, drawn with the keepout tool.

use model::*;
use view::demo::demo_project;
use view::pcb_tools::{PcbPlacer, PcbTool};
use view::view::keepout_hatch;
use view::{Document, ViewId};

#[test]
fn hatch_lines_stay_inside_and_skip_holes() {
    let outline = vec![Point::mm(0.0, 0.0), Point::mm(10.0, 0.0), Point::mm(10.0, 10.0), Point::mm(0.0, 10.0)];
    let hole = vec![Point::mm(3.0, 3.0), Point::mm(7.0, 3.0), Point::mm(7.0, 7.0), Point::mm(3.0, 7.0)];
    let lines = keepout_hatch(&outline, std::slice::from_ref(&hole), mm(1.0));
    assert!(lines.len() > 15, "{}", lines.len());
    let inside =
        |p: Point, r: &[Point]| p.x >= r[0].x - 1 && p.x <= r[2].x + 1 && p.y >= r[0].y - 1 && p.y <= r[2].y + 1;
    for &(a, b) in &lines {
        let mid = Point::new((a.x + b.x) / 2, (a.y + b.y) / 2);
        assert!(inside(a, &outline) && inside(b, &outline));
        assert!(!(inside(mid, &hole) && mid.x > hole[0].x + 10 && mid.x < hole[2].x - 10), "{a:?}–{b:?}");
    }
}

#[test]
fn the_keepout_tool_draws_a_polygon() {
    let mut doc = Document::new(demo_project());
    let mut t = PcbPlacer::default();
    t.set_tool(PcbTool::Keepout);
    for p in [Point::mm(2.0, 2.0), Point::mm(8.0, 2.0), Point::mm(8.0, 8.0), Point::mm(8.0, 8.0)] {
        t.hover(&mut doc, Some((p, p)), mm(0.3));
        t.click(&mut doc).unwrap();
    }
    let k = doc.project().board.keepouts.iter().next().expect("a keepout");
    assert_eq!(k.outline.len(), 3);
    assert!(k.layers.is_empty(), "all layers by default");
    assert!(k.no_pour && k.no_tracks && !k.no_footprints);
    // It's drawn and can be picked.
    let id = ObjectId::Keepout(k.id);
    assert!(
        doc.view(ViewId::Board)
            .select_rect(doc.project(), Rect::from_points(Point::mm(1.0, 1.0), Point::mm(9.0, 9.0)), false)
            .contains(&id)
    );
}
