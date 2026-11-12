//! Board tools driven like the GUI does: hover (raw + snapped position) and click.

use editor_core::Command;
use editor_core::query::{pad_positions, pin_positions};
use model::builtin::{self, PART_HEADER_1X4, PART_RESISTOR};
use model::*;
use view::Document;
use view::pcb_tools::{PcbGhost, PcbPlacer, PcbTool};

fn place(doc: &mut Document, part: Id, x: f64, at: Point) -> (Id, Id) {
    let c = doc
        .execute(Command::PlacePart {
            part,
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
    let pos = doc.project().board.footprints.get(fp).unwrap().pos;
    doc.execute(Command::Move { ids: vec![ObjectId::Footprint(fp)], delta: at - pos, keep_connected: false }).unwrap();
    (sym, fp)
}

fn pin(doc: &Document, s: Id, name: &str) -> Point {
    pin_positions(doc.project(), s).into_iter().find(|p| p.1 == name).unwrap().2
}

fn pad(doc: &Document, f: Id, name: &str) -> Point {
    pad_positions(doc.project(), f).into_iter().find(|p| p.1 == name).unwrap().2
}

/// Hover near `p` (slightly off, as a hand would) and click.
fn click_near(t: &mut PcbPlacer, doc: &mut Document, p: Point) -> bool {
    let raw = p + Point::new(70_000, -40_000);
    let snapped = Point::new((raw.x / 500_000) * 500_000, (raw.y / 500_000) * 500_000);
    t.hover(doc, Some((raw, snapped)), mm(0.3));
    t.click(doc).unwrap()
}

#[test]
fn route_two_nets_with_a_layer_change() {
    let mut doc = Document::new(Project::new("T"));
    let parts = vec![PART_RESISTOR, PART_HEADER_1X4];
    doc.execute(Command::ImportParts { library: builtin::builtin_library(), parts }).unwrap();
    let (sj, fj) = place(&mut doc, PART_HEADER_1X4, 0.0, Point::mm(0.0, 0.0));
    let (sr, fr) = place(&mut doc, PART_RESISTOR, 20.32, Point::mm(10.0, 0.0));
    for (a, b) in [((sj, "1"), (sr, "1")), ((sj, "2"), (sr, "2"))] {
        let (pa, pb) = (pin(&doc, a.0, a.1), pin(&doc, b.0, b.1));
        let pts = if pa.x == pb.x || pa.y == pb.y { vec![pa, pb] } else { vec![pa, Point::new(pb.x, pa.y), pb] };
        doc.execute(Command::AddWire { sheet: None, points: pts }).unwrap();
    }
    assert_eq!(doc.current_copper().unrouted(), 2);

    let mut t = PcbPlacer::default();
    t.set_tool(PcbTool::Route);
    // J1.1 → R1.1 on top: start on the pad (snaps to its center), one corner, end on the pad.
    let (j1, r1) = (pad(&doc, fj, "1"), pad(&doc, fr, "1"));
    assert!(!click_near(&mut t, &mut doc, j1));
    assert!(t.routing_net().is_some(), "the net being routed is known");
    let corner = Point::new(j1.x + mm(3.0), j1.y + mm(3.0));
    assert!(!click_near(&mut t, &mut doc, corner));
    t.hover(&mut doc, Some((r1, r1)), mm(0.3));
    assert!(t.ghosts(&doc).iter().any(|g| matches!(g, PcbGhost::Target(_))));
    assert!(t.click(&mut doc).unwrap(), "ends on the pad");
    assert_eq!(doc.current_copper().unrouted(), 1);
    // Tracks start exactly at the pad center.
    assert!(doc.project().board.tracks.iter().any(|tr| tr.a == j1));

    // J1.2 → R1.2: start on top, switch to the bottom (via), come back up with a via.
    let (j2, r2) = (pad(&doc, fj, "2"), pad(&doc, fr, "2"));
    click_near(&mut t, &mut doc, j2);
    let down = Point::new(j2.x + mm(2.0), j2.y - mm(2.0));
    t.hover(&mut doc, Some((down, down)), mm(0.3));
    t.click(&mut doc).unwrap();
    t.next_layer(&mut doc).unwrap();
    assert_eq!(t.settings.layer, LayerId::BOTTOM_COPPER);
    assert_eq!(doc.project().board.vias.len(), 1);
    let up = Point::new(r2.x, down.y);
    t.hover(&mut doc, Some((up, up)), mm(0.3));
    t.click(&mut doc).unwrap();
    t.next_layer(&mut doc).unwrap();
    assert_eq!(doc.project().board.vias.len(), 2);
    t.hover(&mut doc, Some((r2, r2)), mm(0.3));
    assert!(t.click(&mut doc).unwrap());
    let copper = doc.current_copper();
    assert_eq!(copper.unrouted(), 0, "{:?}", copper.ratsnest);
    assert!(copper.shorts.is_empty());
    assert!(doc.project().board.tracks.iter().any(|tr| tr.layer == LayerId::BOTTOM_COPPER));
}

#[test]
fn outline_and_vias() {
    let mut doc = Document::new(Project::new("T"));
    let mut t = PcbPlacer::default();
    t.set_tool(PcbTool::Outline);
    let (a, b) = (Point::mm(0.0, 0.0), Point::mm(30.0, 20.0));
    t.hover(&mut doc, Some((a, a)), mm(0.3));
    assert!(!t.click(&mut doc).unwrap());
    t.hover(&mut doc, Some((b, b)), mm(0.3));
    assert!(t.ghosts(&doc).contains(&PcbGhost::Outline(Rect::from_points(a, b))));
    assert!(t.click(&mut doc).unwrap());
    let g = doc.project().board.graphics.iter().next().unwrap();
    assert_eq!(g.layer, LayerId::OUTLINE);
    assert_eq!(g.shape.bounds().width(), mm(30.0) + 100_000);

    t.set_tool(PcbTool::Via);
    t.hover(&mut doc, Some((b, b)), mm(0.3));
    t.click(&mut doc).unwrap();
    assert_eq!(doc.project().board.vias.len(), 1);
    assert!(t.escape());
    assert_eq!(t.tool(), PcbTool::Select);
}

#[test]
fn draw_a_zone_that_connects_its_net() {
    let mut doc = Document::new(Project::new("T"));
    let parts = vec![PART_RESISTOR, PART_HEADER_1X4];
    doc.execute(Command::ImportParts { library: builtin::builtin_library(), parts }).unwrap();
    let (sj, fj) = place(&mut doc, PART_HEADER_1X4, 0.0, Point::mm(0.0, 0.0));
    let (sr, _) = place(&mut doc, PART_RESISTOR, 20.32, Point::mm(10.0, 0.0));
    for (a, b) in [((sj, "1"), (sr, "1")), ((sj, "2"), (sr, "2"))] {
        let (pa, pb) = (pin(&doc, a.0, a.1), pin(&doc, b.0, b.1));
        let pts = if pa.x == pb.x || pa.y == pb.y { vec![pa, pb] } else { vec![pa, Point::new(pb.x, pa.y), pb] };
        doc.execute(Command::AddWire { sheet: None, points: pts }).unwrap();
    }
    assert_eq!(doc.current_copper().unrouted(), 2);

    let mut t = PcbPlacer::default();
    t.set_tool(PcbTool::Zone);
    // Without a net (free space, no net chosen) the zone can't start.
    let free = Point::mm(-20.0, -20.0);
    t.hover(&mut doc, Some((free, free)), mm(0.3));
    assert!(t.click(&mut doc).is_err());
    // Start on J1.1: the zone takes its net.
    let j1 = pad(&doc, fj, "1");
    let net = doc.current_copper().nets.clone();
    let corners = [j1, Point::mm(-5.0, -10.0), Point::mm(20.0, -10.0), Point::mm(20.0, 10.0), Point::mm(-5.0, 10.0)];
    for c in corners {
        t.hover(&mut doc, Some((c, c)), mm(0.3));
        assert!(!t.click(&mut doc).unwrap());
    }
    let zone_net = t.zone_net().unwrap().to_string();
    assert!(net.contains(&zone_net));
    assert!(matches!(&t.ghosts(&doc)[0], PcbGhost::Zone { points, .. } if points.len() == 6));
    // Click the last corner again: closed.
    let last = corners[4];
    t.hover(&mut doc, Some((last, last)), mm(0.3));
    assert!(t.click(&mut doc).unwrap());
    let z = doc.project().board.zones.iter().next().unwrap();
    assert_eq!((z.net.as_str(), z.outline.len(), z.layer), (zone_net.as_str(), 5, LayerId::TOP_COPPER));
    let copper = doc.current_copper();
    assert!(copper.fills[0].shapes.len() == 1);
    assert_eq!(copper.unrouted(), 1, "the zone connects J1.1 and R1.1: {:?}", copper.ratsnest);
    assert!(copper.shorts.is_empty());
}

#[test]
fn measure_between_pads_snaps_and_leaves_the_board_unchanged() {
    let mut doc = Document::new(Project::new("T"));
    let parts = vec![PART_RESISTOR];
    doc.execute(Command::ImportParts { library: builtin::builtin_library(), parts }).unwrap();
    let r = doc.project().library.parts.iter().next().unwrap().id;
    let (_, f) = place(&mut doc, r, 0.0, Point::mm(10.0, 10.0));
    let (a, b) = (pad(&doc, f, "1"), pad(&doc, f, "2"));
    let revision = doc.revision;
    let mut t = PcbPlacer::default();
    t.set_tool(PcbTool::Measure);
    assert!(!click_near(&mut t, &mut doc, a));
    assert_eq!(t.measure.target, Some(a), "snapped to the pad centre");
    assert!(!click_near(&mut t, &mut doc, b));
    assert!(t.measure.is_pinned());
    assert_eq!(t.measure.segment(), Some((a, b)));
    assert_eq!(t.measure.reading().unwrap().length, (b - a).length());
    // Alt: the raw pointer position.
    t.free = true;
    t.hover(&mut doc, Some((a + Point::new(70_000, 0), a)), mm(0.3));
    assert_eq!(t.measure.target, None);
    // Esc clears, a second Esc leaves the tool; the board never changed.
    assert!(!t.escape());
    assert_eq!(t.measure.segment(), None);
    assert!(t.escape());
    assert_eq!(doc.revision, revision);
}
