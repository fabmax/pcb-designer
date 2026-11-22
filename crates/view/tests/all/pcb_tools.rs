//! Board tools driven like the GUI does: hover (raw + snapped position) and click.

use editor_core::Command;
use editor_core::query::{pad_positions, pin_positions};
use model::builtin::{self, PART_HEADER_1X4, PART_RESISTOR};
use model::*;
use view::Document;
use view::pcb_tools::{PcbGhost, PcbPlacer, PcbTool};

fn place(doc: &mut Document, part: Id, x: f64, at: Point) -> (Id, Id) {
    place_at(doc, part, Point::mm(x, 0.0), at)
}

/// Places `part`'s symbol at `sch` and its footprint at `at`.
fn place_at(doc: &mut Document, part: Id, sch: Point, at: Point) -> (Id, Id) {
    let c = doc
        .execute(Command::PlacePart {
            part,
            variant: None,
            sheet: None,
            pos: sch,
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
    doc.execute(Command::import_parts(&builtin::builtin_library(), &parts).unwrap()).unwrap();
    let (sj, fj) = place(&mut doc, PART_HEADER_1X4, 0.0, Point::mm(0.0, 0.0));
    let (sr, fr) = place_at(&mut doc, PART_RESISTOR, Point::mm(20.32, -7.62), Point::mm(10.0, 0.0));
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
    // On the board edge: refused, shown red with a marker.
    t.hover(&mut doc, Some((b, b)), mm(0.3));
    assert!(t.ghosts(&doc).iter().any(|g| matches!(g, PcbGhost::Via { bad: true, .. })));
    assert!(t.ghosts(&doc).iter().any(|g| matches!(g, PcbGhost::Clash(_))));
    let refused = t.click(&mut doc).unwrap_err();
    assert!(refused.contains("board edge") && refused.contains("Shift+click"), "{refused}");
    assert!(doc.project().board.vias.is_empty());
    // Unless forced this once (Shift) …
    t.force = true;
    t.click(&mut doc).unwrap();
    assert_eq!(doc.project().board.vias.len(), 1);
    t.force = false;
    assert!(t.click(&mut doc).is_err());
    // … or violations are allowed.
    t.settings.allow_violations = true;
    t.click(&mut doc).unwrap();
    assert_eq!(doc.project().board.vias.len(), 2);
    t.settings.allow_violations = false;
    let inside = Point::mm(10.0, 10.0);
    t.hover(&mut doc, Some((inside, inside)), mm(0.3));
    assert!(t.clash().is_none());
    t.click(&mut doc).unwrap();
    assert_eq!(doc.project().board.vias.len(), 3);
    assert!(t.escape());
    assert_eq!(t.tool(), PcbTool::Select);
}

#[test]
fn draw_a_zone_that_connects_its_net() {
    let mut doc = Document::new(Project::new("T"));
    let parts = vec![PART_RESISTOR, PART_HEADER_1X4];
    doc.execute(Command::import_parts(&builtin::builtin_library(), &parts).unwrap()).unwrap();
    let (sj, fj) = place(&mut doc, PART_HEADER_1X4, 0.0, Point::mm(0.0, 0.0));
    let (sr, _) = place_at(&mut doc, PART_RESISTOR, Point::mm(20.32, -7.62), Point::mm(10.0, 0.0));
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
    doc.execute(Command::import_parts(&builtin::builtin_library(), &parts).unwrap()).unwrap();
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

#[test]
fn place_board_parts_with_rotation_and_side() {
    // The header's footprint as a board part (no symbol).
    let lib = builtin::builtin_library();
    let mut b = lib.bundles(&[PART_HEADER_1X4]).unwrap().remove(0);
    b.part.id = Id::new();
    b.part.prefix = "H".into();
    b.part.units.clear();
    b.symbols.clear();
    b.part.variants.iter_mut().for_each(|v| v.connections.clear());
    let part = b.part.id;
    let mut doc = Document::new(Project::new("t"));
    doc.execute(Command::ImportParts { parts: vec![b] }).unwrap();

    let mut t = PcbPlacer::default();
    t.place(part);
    assert_eq!(t.tool(), PcbTool::Place);
    t.rotate();
    t.flip();
    let at = Point::mm(10.0, 10.0);
    t.hover(&mut doc, Some((at, at)), mm(0.3));
    t.click(&mut doc).unwrap();
    assert!(matches!(t.ghosts(&doc)[..], [PcbGhost::Footprint { side: Side::Bottom, .. }]));
    click_near(&mut t, &mut doc, Point::mm(20.0, 10.0));
    let p = doc.project();
    let refs: Vec<&str> = p.schematic.components.iter().map(|c| c.refdes.as_str()).collect();
    assert_eq!(refs, ["H1", "H2"], "placing continues after a click");
    let f = p.board.footprints.iter().next().unwrap();
    assert_eq!((f.pos, f.rotation, f.side), (at, Angle::DEG90, Side::Bottom));
    assert!(p.schematic.symbols.is_empty());
    // Esc leaves the tool, and a new placement starts on top again.
    assert!(t.escape());
    t.place(part);
    assert_eq!((t.settings.rotation, t.settings.side), (Angle::ZERO, Side::Top));
}

#[test]
fn route_with_rounded_corners() {
    let mut doc = Document::new(Project::new("T"));
    let mut t = PcbPlacer::default();
    t.set_tool(PcbTool::Route);
    t.settings.corners = view::bend::Corners::Arc90;
    let (a, b) = (Point::mm(0.0, 0.0), Point::mm(10.0, 4.0));
    t.hover(&mut doc, Some((a, a)), mm(0.3));
    t.click(&mut doc).unwrap();
    t.hover(&mut doc, Some((b, b)), mm(0.3));
    let ghosts = t.ghosts(&doc);
    assert!(ghosts.iter().any(|g| matches!(g, PcbGhost::Track { mid: Some(_), .. })), "{ghosts:?}");
    t.click(&mut doc).unwrap();
    let tracks: Vec<&Track> = doc.project().board.tracks.iter().collect();
    assert_eq!(tracks.len(), 2);
    assert_eq!((tracks[0].a, tracks[0].b, tracks[0].mid), (a, Point::mm(6.0, 0.0), None));
    assert!(tracks[1].mid.is_some() && tracks[1].b == b);
}

#[test]
fn routing_keeps_clearance_to_other_nets() {
    let mut doc = Document::new(Project::new("T"));
    let parts = vec![PART_RESISTOR, PART_HEADER_1X4];
    doc.execute(Command::import_parts(&builtin::builtin_library(), &parts).unwrap()).unwrap();
    let (sj, fj) = place(&mut doc, PART_HEADER_1X4, 0.0, Point::mm(0.0, 0.0));
    let (sr, _) = place_at(&mut doc, PART_RESISTOR, Point::mm(20.32, -7.62), Point::mm(10.0, 0.0));
    for (a, b) in [((sj, "1"), (sr, "1")), ((sj, "2"), (sr, "2"))] {
        let (pa, pb) = (pin(&doc, a.0, a.1), pin(&doc, b.0, b.1));
        let pts = if pa.x == pb.x || pa.y == pb.y { vec![pa, pb] } else { vec![pa, Point::new(pb.x, pa.y), pb] };
        doc.execute(Command::AddWire { sheet: None, points: pts }).unwrap();
    }
    let (j1, j2) = (pad(&doc, fj, "1"), pad(&doc, fj, "2"));
    let mut t = PcbPlacer::default();
    t.set_tool(PcbTool::Route);
    t.settings.route_mode = view::pcb_tools::RouteMode::Highlight;
    t.settings.corners = view::bend::Corners::Deg90;
    t.hover(&mut doc, Some((j1, j1)), mm(0.3));
    t.click(&mut doc).unwrap();
    // Straight onto the pad of the other net: refused.
    t.hover(&mut doc, Some((j2, j2)), mm(0.3));
    let clash = t.clash().expect("a clash").clone();
    assert!(clash.message.contains("touch"), "{}", clash.message);
    assert!(t.click(&mut doc).is_err());
    assert!(doc.project().board.tracks.is_empty());
    // Away from it: fine.
    let away = j1 + Point::mm(-5.0, 0.0);
    t.hover(&mut doc, Some((away, away)), mm(0.3));
    assert!(t.clash().is_none(), "{:?}", t.clash());
    t.click(&mut doc).unwrap();
    assert_eq!(doc.project().board.tracks.len(), 1);
    t.escape();

    // Started in free space, a route may end on any pad (it takes that net)...
    let free = j2 + Point::mm(-5.0, 0.0);
    t.hover(&mut doc, Some((free, free)), mm(0.3));
    t.click(&mut doc).unwrap();
    t.hover(&mut doc, Some((j2, j2)), mm(0.3));
    assert!(t.clash().is_none(), "{:?}", t.clash());
    // ...but not cross copper it doesn't connect to (the first track).
    t.settings.bend = view::bend::Bend::First;
    let across = j1 + (j1 - j2) + Point::mm(-4.0, 0.0);
    t.hover(&mut doc, Some((across, across)), mm(0.0));
    assert!(t.clash().is_some_and(|c| c.message.contains("touch")), "{:?}", t.clash());
}

#[test]
fn routes_and_vias_take_their_net_class_sizes() {
    let mut doc = Document::new(Project::new("T"));
    let parts = vec![PART_RESISTOR, PART_HEADER_1X4];
    doc.execute(Command::import_parts(&builtin::builtin_library(), &parts).unwrap()).unwrap();
    let (sj, fj) = place(&mut doc, PART_HEADER_1X4, 0.0, Point::mm(0.0, 0.0));
    let (sr, fr) = place_at(&mut doc, PART_RESISTOR, Point::mm(20.32, -7.62), Point::mm(10.0, 0.0));
    let (pa, pb) = (pin(&doc, sj, "1"), pin(&doc, sr, "1"));
    let pts = if pa.x == pb.x || pa.y == pb.y { vec![pa, pb] } else { vec![pa, Point::new(pb.x, pa.y), pb] };
    doc.execute(Command::AddWire { sheet: None, points: pts }).unwrap();
    let mut rules = doc.project().board.rules.clone();
    rules.net_classes = vec![NetClass {
        name: "Power".into(),
        track_width: Some(mm(0.6)),
        via_diameter: Some(mm(0.9)),
        via_drill: Some(mm(0.4)),
        nets: vec!["*".into()],
        ..Default::default()
    }];
    doc.execute(Command::SetDesignRules { rules }).unwrap();

    // The Route tool: J1.1 → a corner, a via there.
    let mut t = PcbPlacer::default();
    t.set_tool(PcbTool::Route);
    let j1 = pad(&doc, fj, "1");
    click_near(&mut t, &mut doc, j1);
    let corner = Point::new(j1.x + mm(3.0), j1.y + mm(3.0));
    t.hover(&mut doc, Some((corner, corner)), mm(0.3));
    t.click(&mut doc).unwrap();
    t.next_layer(&mut doc).unwrap();
    let board = &doc.project().board;
    assert!(board.tracks.iter().all(|tr| tr.width == mm(0.6)));
    let via = board.vias.iter().next().unwrap();
    assert_eq!((via.diameter, via.drill), (mm(0.9), mm(0.4)));

    // Commands without sizes (as an agent sends them) too; copper without a net doesn't.
    let r1 = pad(&doc, fr, "1");
    let points = vec![r1, Point::new(r1.x, r1.y + mm(2.0))];
    doc.execute(Command::AddTrack { layer: LayerId::TOP_COPPER, points, width: None, mids: Vec::new() }).unwrap();
    let free = vec![Point::mm(30.0, 30.0), Point::mm(32.0, 30.0)];
    doc.execute(Command::AddTrack { layer: LayerId::TOP_COPPER, points: free, width: None, mids: Vec::new() }).unwrap();
    let widths: Vec<Coord> = doc.project().board.tracks.iter().map(|t| t.width).collect();
    assert_eq!(widths.iter().filter(|&&w| w == mm(0.6)).count(), widths.len() - 1, "{widths:?}");
    assert!(widths.contains(&doc.project().board.rules.track_width));
}

/// The first leg follows the way the raw cursor leaves the start on screen, not the
/// first grid point it snaps to.
#[test]
fn route_posture_follows_the_cursor_on_screen() {
    let mut doc = Document::new(Project::new("T"));
    let mut t = PcbPlacer::default();
    t.set_tool(PcbTool::Route);
    t.settings.corners = view::bend::Corners::Deg45;
    let a = Point::mm(0.0, 0.0);
    // A point is 25 µm (40 points: 1 mm). The cursor goes off diagonally; snapped to a coarse grid it
    // would still read as straight right.
    t.steer(Point::mm(0.02, 0.01), 2.5e4);
    t.hover(&mut doc, Some((a, a)), mm(0.3));
    t.click(&mut doc).unwrap();
    t.steer(Point::mm(0.9, 0.8), 2.5e4);
    let b = Point::mm(5.0, 2.0);
    t.hover(&mut doc, Some((b, b)), mm(0.3));
    let ghosts = t.ghosts(&doc);
    let first = ghosts.iter().find_map(|g| match g {
        PcbGhost::Track { a, b, .. } => Some((*a, *b)),
        _ => None,
    });
    assert_eq!(first, Some((a, Point::mm(2.0, 2.0))), "diagonal first: {ghosts:?}");
}

#[test]
fn walk_around_mode_routes_round_copper_and_stops_short() {
    let mut doc = Document::new(Project::new("T"));
    let wall = Track::straight(LayerId::TOP_COPPER, Point::mm(10.0, -3.0), Point::mm(10.0, 3.0), mm(0.25));
    let k = Keepout::new(
        Vec::new(),
        vec![Point::mm(28.0, -2.0), Point::mm(32.0, -2.0), Point::mm(32.0, 2.0), Point::mm(28.0, 2.0)],
    );
    doc.execute(Command::Insert { objects: vec![Object::Track(wall), Object::Keepout(k)] }).unwrap();
    let mut t = PcbPlacer::default();
    t.set_tool(PcbTool::Route);
    t.settings.route_mode = view::pcb_tools::RouteMode::Walkaround;
    let (a, b) = (Point::mm(0.0, 0.0), Point::mm(20.0, 0.0));
    t.hover(&mut doc, Some((a, a)), mm(0.0));
    t.click(&mut doc).unwrap();
    t.hover(&mut doc, Some((b, b)), mm(0.0));
    assert!(t.clash().is_none(), "nothing to refuse: {:?}", t.clash());
    let ghosts = t.ghosts(&doc);
    let legs = ghosts.iter().filter(|g| matches!(g, PcbGhost::Track { bad: false, .. })).count();
    assert!(legs > 2, "round the wall: {ghosts:?}");
    // Shift: straight through, as in Highlight mode.
    t.force = true;
    t.hover(&mut doc, Some((b, b)), mm(0.0));
    assert!(t.clash().is_some());
    t.force = false;
    t.hover(&mut doc, Some((b, b)), mm(0.0));
    t.click(&mut doc).unwrap();
    assert_eq!(doc.project().board.tracks.len(), 1 + legs, "one track per corner");
    let bad: Vec<_> = doc
        .run_drc()
        .violations
        .iter()
        .filter(|v| matches!(v.kind, ViolationKind::Clearance | ViolationKind::Short | ViolationKind::Keepout))
        .map(|v| v.message.clone())
        .collect();
    assert!(bad.is_empty(), "{bad:?}");
    // Into the keepout: stops short; a click adds up to there and goes on from it.
    let c = Point::mm(30.0, 0.0);
    t.hover(&mut doc, Some((c, c)), mm(0.0));
    let stop = *t.stopped_short().expect("stops short").points.last().unwrap();
    assert!(stop.x < mm(28.0), "{stop:?}");
    assert!(!t.click(&mut doc).unwrap(), "the route goes on");
    let ends: Vec<Point> = doc.project().board.tracks.iter().map(|t| t.b).collect();
    assert!(ends.contains(&stop));
}

#[test]
fn shove_mode_pushes_tracks_live_and_commits_one_undo_step() {
    let mut doc = Document::new(Project::new("T"));
    let wall = Track::straight(LayerId::TOP_COPPER, Point::mm(0.0, 0.0), Point::mm(20.0, 0.0), mm(0.25));
    doc.execute(Command::Insert { objects: vec![Object::Track(wall.clone())] }).unwrap();
    let original: Vec<Track> = doc.project().board.tracks.iter().cloned().collect();
    let mut t = PcbPlacer::default();
    t.set_tool(PcbTool::Route);
    assert_eq!(t.settings.route_mode, view::pcb_tools::RouteMode::Shove, "the default");
    let (a, b) = (Point::mm(10.0, -5.0), Point::mm(10.0, 1.0));
    t.hover(&mut doc, Some((a, a)), mm(0.0));
    t.click(&mut doc).unwrap();
    assert!(t.wants_route_preview());
    // The wall bulges round the head's end, live.
    t.hover(&mut doc, Some((b, b)), mm(0.0));
    assert!(doc.project().board.tracks.len() > 1, "pushed: {:?}", doc.project().board.tracks);
    assert!(doc.project().board.tracks.iter().any(|t| t.a.y > mm(1.0) || t.b.y > mm(1.0)));
    // Back below it: it springs back.
    let low = Point::mm(10.0, -2.0);
    t.hover(&mut doc, Some((low, low)), mm(0.0));
    assert_eq!(doc.project().board.tracks.iter().cloned().collect::<Vec<_>>(), original);
    // Up again and click: the leg and the pushed wall, as one undo step.
    t.hover(&mut doc, Some((b, b)), mm(0.0));
    let pushed = doc.project().board.tracks.len();
    t.click(&mut doc).unwrap();
    assert_eq!(doc.project().board.tracks.len(), pushed + 1);
    assert!(!t.wants_route_preview() || t.clash().is_none());
    doc.undo();
    assert_eq!(doc.project().board.tracks.iter().cloned().collect::<Vec<_>>(), original);
    // A preview left behind (Esc) is dropped, never kept.
    doc.redo();
    doc.undo();
    t.escape();
    t.hover(&mut doc, Some((a, a)), mm(0.0));
    t.click(&mut doc).unwrap();
    t.hover(&mut doc, Some((b, b)), mm(0.0));
    assert!(doc.project().board.tracks.len() > 1);
    t.escape();
    assert!(!t.wants_route_preview());
    doc.drop_route_preview();
    assert_eq!(doc.project().board.tracks.iter().cloned().collect::<Vec<_>>(), original);
    // Nothing of it kept: one undo takes back the wall itself.
    doc.undo();
    assert!(doc.project().board.tracks.is_empty());
}

#[test]
fn a_via_added_while_routing_pushes_tracks_aside() {
    let mut doc = Document::new(Project::new("T"));
    let wall = Track::straight(LayerId::TOP_COPPER, Point::mm(10.6, -3.0), Point::mm(10.6, 3.0), mm(0.25));
    doc.execute(Command::Insert { objects: vec![Object::Track(wall)] }).unwrap();
    let mut t = PcbPlacer::default();
    t.set_tool(PcbTool::Route);
    assert_eq!(t.settings.route_mode, view::pcb_tools::RouteMode::Shove);
    let (a, b) = (Point::mm(0.0, 0.0), Point::mm(10.0, 0.0));
    t.hover(&mut doc, Some((a, a)), mm(0.0));
    t.click(&mut doc).unwrap();
    t.hover(&mut doc, Some((b, b)), mm(0.0));
    t.click(&mut doc).unwrap();
    let before: Vec<Track> = doc.project().board.tracks.iter().cloned().collect();
    t.next_layer(&mut doc).unwrap();
    let board = &doc.project().board;
    assert_eq!(board.vias.len(), 1);
    assert_eq!(board.vias.iter().next().unwrap().pos, b);
    assert!(board.tracks.len() > before.len(), "the wall goes round the via");
    let bad: Vec<_> = doc
        .run_drc()
        .violations
        .iter()
        .filter(|v| matches!(v.kind, ViolationKind::Clearance | ViolationKind::Short))
        .map(|v| v.message.clone())
        .collect();
    assert!(bad.is_empty(), "{bad:?}");
    // One undo takes the via and its pushes back.
    doc.undo();
    assert_eq!(doc.project().board.vias.len(), 0);
    let after: Vec<Track> = doc.project().board.tracks.iter().cloned().collect();
    assert_eq!(after, before);
}

#[test]
fn shove_mode_keeps_square_corners_where_nothing_is_walked_round() {
    // Nothing in the way, and a track the leg pushes: either way the corner stays 90°.
    for wall in [false, true] {
        let mut doc = Document::new(Project::new("T"));
        if wall {
            // Below the first leg, its ends well clear of it: pushed down.
            let t = Track::straight(LayerId::TOP_COPPER, Point::mm(-5.0, -0.3), Point::mm(20.0, -0.3), mm(0.25));
            doc.execute(Command::Insert { objects: vec![Object::Track(t)] }).unwrap();
        }
        let mut t = PcbPlacer::default();
        t.set_tool(PcbTool::Route);
        t.settings.corners = view::bend::Corners::Deg90;
        let (a, b) = (Point::mm(0.0, 0.0), Point::mm(10.0, 5.0));
        t.hover(&mut doc, Some((a, a)), mm(0.0));
        t.click(&mut doc).unwrap();
        t.hover(&mut doc, Some((b, b)), mm(0.0));
        t.click(&mut doc).unwrap();
        let new: Vec<&Track> = doc.project().board.tracks.iter().filter(|t| t.a == a || t.b == b).collect();
        assert_eq!(new.len(), 2, "two legs (wall: {wall}): {new:?}");
        if wall {
            assert!(doc.project().board.tracks.len() > 3, "the wall was pushed");
        }
        for t in new {
            assert!(t.a.x == t.b.x || t.a.y == t.b.y, "horizontal or vertical (wall: {wall}): {t:?}");
        }
    }
}

#[test]
fn the_route_tool_snaps_to_the_middle_of_off_grid_tracks() {
    let mut doc = Document::new(Project::new("T"));
    let l = LayerId::TOP_COPPER;
    // Off the 0.5 mm grid (as between header pins), and a wide one.
    let thin = Track::straight(l, Point::mm(0.0, 1.27), Point::mm(20.0, 1.27), mm(0.25));
    let wide = Track::straight(l, Point::mm(0.0, 5.19), Point::mm(20.0, 5.19), mm(0.4));
    let diag = Track::straight(l, Point::mm(0.0, 10.0), Point::mm(5.0, 15.0), mm(0.25));
    let ids = [thin.id, wide.id, diag.id];
    doc.execute(Command::Insert { objects: [thin, wide, diag].map(Object::Track).to_vec() }).unwrap();
    let snap =
        |doc: &mut Document, raw: Point, grid: Point| view::pcb_tools::snap_to_copper(doc, raw, grid, l, mm(0.3));
    // On the thin one: no grid point lies on it.
    let (p, item) = snap(&mut doc, Point::mm(10.1, 1.3), Point::mm(10.0, 1.5));
    assert_eq!(p, Point::mm(10.0, 1.27));
    assert_eq!(item, Some(editor_core::copper::CopperItem::Track(ids[0])));
    // Beside it, within the tolerance: still its middle.
    assert_eq!(snap(&mut doc, Point::mm(10.0, 1.6), Point::mm(10.0, 1.5)).0, Point::mm(10.0, 1.27));
    // On the wide one, a grid point at its edge: its middle, not the edge.
    assert_eq!(snap(&mut doc, Point::mm(10.0, 5.05), Point::mm(10.0, 5.0)).0, Point::mm(10.0, 5.19));
    // A diagonal one: the grid point projected onto it.
    let (p, item) = snap(&mut doc, Point::mm(2.6, 12.5), Point::mm(2.5, 12.5));
    assert_eq!(item, Some(editor_core::copper::CopperItem::Track(ids[2])));
    assert_eq!(p.y - p.x, mm(10.0), "on the line: {p:?}");
    // Track ends still win, and free space keeps the grid point.
    assert_eq!(snap(&mut doc, Point::mm(0.1, 1.3), Point::mm(0.0, 1.5)).0, Point::mm(0.0, 1.27));
    assert_eq!(snap(&mut doc, Point::mm(10.1, 3.1), Point::mm(10.0, 3.0)).0, Point::mm(10.0, 3.0));
}

#[test]
fn switching_back_to_the_layer_takes_the_via_back() {
    for mode in [view::pcb_tools::RouteMode::Shove, view::pcb_tools::RouteMode::Highlight] {
        let mut doc = Document::new(Project::new("T"));
        let mut t = PcbPlacer::default();
        t.set_tool(PcbTool::Route);
        t.settings.route_mode = mode;
        t.settings.via_at_cursor = false;
        let (a, b, c) = (Point::mm(0.0, 0.0), Point::mm(10.0, 0.0), Point::mm(10.0, 8.0));
        t.hover(&mut doc, Some((a, a)), mm(0.0));
        t.click(&mut doc).unwrap();
        t.hover(&mut doc, Some((b, b)), mm(0.0));
        t.click(&mut doc).unwrap();
        let top = t.settings.layer;
        t.next_layer(&mut doc).unwrap();
        assert_eq!(doc.project().board.vias.len(), 1, "{mode:?}");
        assert_ne!(t.settings.layer, top);
        // The pointer moves on (a route preview in Shove mode), then L again.
        t.hover(&mut doc, Some((c, c)), mm(0.0));
        t.next_layer(&mut doc).unwrap();
        assert_eq!(t.settings.layer, top, "{mode:?}");
        assert_eq!(doc.project().board.vias.len(), 0, "the via goes again ({mode:?})");
        assert_eq!(doc.project().board.tracks.len(), 1, "the track stays ({mode:?})");
        // And L once more brings it back; a leg after it keeps it.
        t.next_layer(&mut doc).unwrap();
        assert_eq!(doc.project().board.vias.len(), 1, "{mode:?}");
        t.hover(&mut doc, Some((c, c)), mm(0.0));
        t.click(&mut doc).unwrap();
        t.next_layer(&mut doc).unwrap();
        assert_eq!(doc.project().board.vias.len(), 2, "a new via at the leg's end, the old one stays ({mode:?})");
    }
}

#[test]
fn l_arms_a_via_at_the_pointer_that_the_click_places() {
    use view::pcb_tools::RouteMode;
    for mode in [RouteMode::Shove, RouteMode::Walkaround, RouteMode::Highlight] {
        let mut doc = Document::new(Project::new("T"));
        // A track the via comes close to (pushed in Shove mode).
        let wall = Track::straight(LayerId::TOP_COPPER, Point::mm(0.0, 8.6), Point::mm(20.0, 8.6), mm(0.25));
        doc.execute(Command::Insert { objects: vec![Object::Track(wall.clone())] }).unwrap();
        let mut t = PcbPlacer::default();
        t.set_tool(PcbTool::Route);
        t.settings.route_mode = mode;
        assert!(t.settings.via_at_cursor, "the default");
        let (a, b, c) = (Point::mm(0.0, 0.0), Point::mm(10.0, 0.0), Point::mm(10.0, 8.0));
        t.hover(&mut doc, Some((a, a)), mm(0.0));
        t.click(&mut doc).unwrap();
        let top = t.settings.layer;
        t.hover(&mut doc, Some((b, b)), mm(0.0));
        // L arms it: nothing placed, the leg stays on top, the via follows the pointer.
        t.next_layer(&mut doc).unwrap();
        assert_eq!(t.settings.layer, top);
        assert!(t.via_to().is_some_and(|l| l != top));
        assert_eq!((doc.project().board.vias.len(), doc.project().board.tracks.len()), (0, 1), "{mode:?}");
        let via_at = |t: &PcbPlacer, doc: &Document| {
            t.ghosts(doc).iter().find_map(|g| match g {
                PcbGhost::Via { pos, .. } => Some(*pos),
                _ => None,
            })
        };
        assert_eq!(via_at(&t, &doc), Some(b));
        // Esc takes it off; L again arms it.
        assert!(!t.escape());
        assert_eq!(t.via_to(), None);
        t.next_layer(&mut doc).unwrap();
        // On to c, by the wall.
        t.hover(&mut doc, Some((c, c)), mm(0.0));
        assert_eq!(via_at(&t, &doc), Some(c), "{mode:?}");
        if mode == RouteMode::Shove {
            assert!(t.clash().is_none(), "{:?}", t.clash());
            assert_ne!(doc.project().board.tracks.get(wall.id), Some(&wall), "the via pushes the wall");
        } else {
            assert!(t.clash().is_some(), "the via is too close to the wall ({mode:?})");
            t.force = true;
            t.hover(&mut doc, Some((c, c)), mm(0.0));
        }
        let before = doc.project().board.tracks.len();
        t.click(&mut doc).unwrap();
        t.force = false;
        let board = &doc.project().board;
        assert_eq!(board.vias.iter().map(|v| v.pos).collect::<Vec<_>>(), vec![c], "{mode:?}");
        assert!(board.tracks.iter().any(|t| t.layer == top && (t.b == c || t.a == c)), "the leg to it on top");
        assert_ne!(t.settings.layer, top, "goes on on the other layer");
        assert_eq!(t.via_to(), None);
        if mode == RouteMode::Shove {
            let bad: Vec<_> = doc
                .run_drc()
                .violations
                .iter()
                .filter(|v| matches!(v.kind, ViolationKind::Clearance | ViolationKind::Short))
                .map(|v| v.message.clone())
                .collect();
            assert!(bad.is_empty(), "{bad:?}");
        }
        // One undo step: leg, via and pushes.
        doc.undo();
        assert_eq!(doc.project().board.vias.len(), 0, "{mode:?}");
        assert!(doc.project().board.tracks.len() < before + 1);
        assert_eq!(doc.project().board.tracks.get(wall.id), Some(&wall), "{mode:?}");
        // L twice: off again (two layers).
        let mut t2 = PcbPlacer::default();
        t2.set_tool(PcbTool::Route);
        t2.hover(&mut doc, Some((a, a)), mm(0.0));
        t2.click(&mut doc).unwrap();
        t2.hover(&mut doc, Some((b, b)), mm(0.0));
        t2.next_layer(&mut doc).unwrap();
        t2.next_layer(&mut doc).unwrap();
        assert_eq!(t2.via_to(), None);
        assert_eq!(t2.settings.layer, top);
    }
}

#[test]
fn routing_inside_a_zone_goes_on_after_each_leg() {
    // The demo's bottom layer is a GND zone.
    let mut doc = Document::new(view::demo::demo_project());
    let mut t = PcbPlacer::default();
    t.set_tool(PcbTool::Route);
    t.settings.layer = LayerId::BOTTOM_COPPER;
    let (a, b, c) = (Point::mm(20.0, 5.0), Point::mm(25.0, 5.0), Point::mm(25.0, 7.0));
    t.hover(&mut doc, Some((a, a)), mm(0.0));
    assert!(!t.click(&mut doc).unwrap());
    assert_eq!(t.routing_net(), None, "free space in a zone is no net");
    t.hover(&mut doc, Some((b, b)), mm(0.0));
    assert!(!t.click(&mut doc).unwrap(), "the track goes on");
    t.hover(&mut doc, Some((c, c)), mm(0.0));
    assert!(!t.click(&mut doc).unwrap(), "and on");
    assert!(t.ghosts(&doc).iter().all(|g| !matches!(g, PcbGhost::Target(_))), "no target in the zone");
}
