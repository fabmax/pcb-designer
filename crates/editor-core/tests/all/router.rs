//! The router's walkaround on real boards: round tracks and keepouts, stopping short, the
//! board edge.

use editor_core::connectivity::Connectivity;
use editor_core::copper::BoardConnectivity;
use editor_core::route_check::{Probe, check};
use editor_core::router::line::line_of;
use editor_core::router::{shove, walkaround};
use editor_core::{Command, Editor};
use model::*;

const W: Coord = 250_000;
const L: LayerId = LayerId::TOP_COPPER;

fn conn(p: &Project) -> BoardConnectivity {
    BoardConnectivity::build(p, &Connectivity::build(p))
}

fn add_track(ed: &mut Editor, a: Point, b: Point) {
    ed.execute(Command::AddTrack { layer: L, points: vec![a, b], width: Some(W), mids: Vec::new() }).unwrap();
}

/// No rule broken along `path`.
fn clean(p: &Project, copper: &BoardConnectivity, path: &[Point]) {
    let probes: Vec<Probe> =
        path.windows(2).map(|w| Probe::Track { layer: L, a: w[0], b: w[1], mid: None, width: W }).collect();
    let ends = [(L, path[0]), (L, *path.last().unwrap())];
    assert_eq!(check(p, copper, &probes, None, &ends), None, "{path:?}");
}

/// 0°, 45° or 90° segments only.
fn octilinear(path: &[Point]) {
    for w in path.windows(2) {
        let (dx, dy) = ((w[1].x - w[0].x).abs(), (w[1].y - w[0].y).abs());
        assert!(dx == 0 || dy == 0 || dx.abs_diff(dy) <= 2, "{path:?}");
    }
}

#[test]
fn round_a_track_in_the_way() {
    let mut ed = Editor::new(Project::new("T"));
    add_track(&mut ed, Point::mm(10.0, -3.0), Point::mm(10.0, 3.0));
    let copper = conn(ed.project());
    let leg = [Point::mm(0.0, 0.0), Point::mm(20.0, 0.0)];
    let r = walkaround(ed.project(), &copper, L, W, None, &leg);
    assert!(r.reached, "{r:?}");
    assert_eq!((r.points[0], *r.points.last().unwrap()), (leg[0], leg[1]));
    assert!(r.points.len() > 2, "it bends: {r:?}");
    clean(ed.project(), &copper, &r.points);
    octilinear(&r.points);
    // The shorter way: round one end of the track, not far off.
    let far = r.points.iter().map(|p| p.y.abs()).max().unwrap();
    assert!(far < mm(4.0), "{r:?}");
    // Nothing in the way: the leg as asked.
    let r = walkaround(ed.project(), &copper, L, W, None, &[Point::mm(0.0, 5.0), Point::mm(20.0, 5.0)]);
    assert_eq!(r.points, vec![Point::mm(0.0, 5.0), Point::mm(20.0, 5.0)]);
}

#[test]
fn round_a_keepout() {
    let mut ed = Editor::new(Project::new("T"));
    let k = Keepout::new(
        Vec::new(),
        vec![Point::mm(8.0, -2.0), Point::mm(12.0, -2.0), Point::mm(12.0, 2.0), Point::mm(8.0, 2.0)],
    );
    ed.execute(Command::Insert { objects: vec![Object::Keepout(k)] }).unwrap();
    let copper = conn(ed.project());
    let r = walkaround(ed.project(), &copper, L, W, None, &[Point::mm(0.0, 0.0), Point::mm(20.0, 0.0)]);
    assert!(r.reached, "{r:?}");
    clean(ed.project(), &copper, &r.points);
    octilinear(&r.points);
}

#[test]
fn stops_short_of_copper_at_the_pointer() {
    let mut ed = Editor::new(Project::new("T"));
    add_track(&mut ed, Point::mm(10.0, -3.0), Point::mm(10.0, 3.0));
    // The pointer is inside a keepout: the leg goes round the track, then stops at it.
    let k = Keepout::new(
        Vec::new(),
        vec![Point::mm(18.0, -2.0), Point::mm(22.0, -2.0), Point::mm(22.0, 2.0), Point::mm(18.0, 2.0)],
    );
    ed.execute(Command::Insert { objects: vec![Object::Keepout(k)] }).unwrap();
    let copper = conn(ed.project());
    let r = walkaround(ed.project(), &copper, L, W, None, &[Point::mm(0.0, 0.0), Point::mm(20.0, 0.0)]);
    assert!(!r.reached, "{r:?}");
    let end = *r.points.last().unwrap();
    assert!(end.x < mm(18.0) && end.x > mm(15.0), "stops before the keepout: {r:?}");
    clean(ed.project(), &copper, &r.points);
    assert!(r.blocked.is_some());
}

#[test]
fn the_board_edge_picks_the_way_round() {
    let mut ed = Editor::new(Project::new("T"));
    let outline = BoardGraphic {
        id: Id::new(),
        layer: LayerId::OUTLINE,
        shape: Shape::rect(Point::mm(0.0, 0.0), Point::mm(30.0, 20.0), 100_000),
    };
    ed.execute(Command::Insert { objects: vec![Object::BoardGraphic(outline)] }).unwrap();
    // A wall from near the top edge down to y = 6: only below it is there room.
    add_track(&mut ed, Point::mm(15.0, 6.0), Point::mm(15.0, 19.6));
    let copper = conn(ed.project());
    let r = walkaround(ed.project(), &copper, L, W, None, &[Point::mm(5.0, 10.0), Point::mm(25.0, 10.0)]);
    assert!(r.reached, "{r:?}");
    assert!(r.points.iter().any(|p| p.y < mm(6.0)), "below the wall: {r:?}");
    clean(ed.project(), &copper, &r.points);
}

fn ids(ed: &mut Editor, points: &[Point]) -> Vec<Id> {
    let c =
        ed.execute(Command::AddTrack { layer: L, points: points.to_vec(), width: Some(W), mids: Vec::new() }).unwrap();
    c.ids().filter_map(|id| if let ObjectId::Track(t) = id { Some(t) } else { None }).collect()
}

/// Add the routed leg and lay the pushed lines, then: no clearance violations or shorts.
fn apply_and_check(ed: &mut Editor, r: &editor_core::router::Routed) {
    ed.execute(Command::AddTrack { layer: L, points: r.points.clone(), width: Some(W), mids: Vec::new() }).unwrap();
    for c in r.shoved.commands(ed.project()) {
        ed.execute(c).unwrap();
    }
    let copper = conn(ed.project());
    let bad: Vec<String> = editor_core::drc::run(ed.project(), &copper)
        .into_iter()
        .filter(|v| matches!(v.kind, ViolationKind::Clearance | ViolationKind::Short))
        .map(|v| v.message)
        .collect();
    assert!(bad.is_empty(), "{bad:?}\n{r:?}");
}

#[test]
fn lines_run_between_joints() {
    let mut ed = Editor::new(Project::new("T"));
    let chain = ids(&mut ed, &[Point::mm(0.0, 0.0), Point::mm(5.0, 0.0), Point::mm(8.0, 3.0), Point::mm(12.0, 3.0)]);
    // A branch at (12, 3) doesn't matter to the chain (it's its end); one at (5, 0) ends it.
    let copper = conn(ed.project());
    let l = line_of(ed.project(), &copper, chain[1]).unwrap();
    assert_eq!(l.tracks, chain);
    assert_eq!(l.path, vec![Point::mm(0.0, 0.0), Point::mm(5.0, 0.0), Point::mm(8.0, 3.0), Point::mm(12.0, 3.0)]);
    assert!(!l.fixed);
    ids(&mut ed, &[Point::mm(5.0, 0.0), Point::mm(5.0, -4.0)]);
    let copper = conn(ed.project());
    let l = line_of(ed.project(), &copper, chain[2]).unwrap();
    assert_eq!(l.tracks, chain[1..].to_vec());
    assert_eq!(l.path[0], Point::mm(5.0, 0.0));
}

#[test]
fn shove_a_track_ahead_of_the_head() {
    let mut ed = Editor::new(Project::new("T"));
    let wall = ids(&mut ed, &[Point::mm(0.0, 0.0), Point::mm(20.0, 0.0)]);
    let copper = conn(ed.project());
    let leg = [Point::mm(10.0, -5.0), Point::mm(10.0, 1.0)];
    let r = shove(ed.project(), &copper, L, W, None, &leg);
    assert!(r.reached, "{r:?}");
    assert_eq!(r.points, leg.to_vec());
    assert_eq!(r.shoved.lines.len(), 1);
    let p = &r.shoved.lines[0];
    assert_eq!(p.line.tracks, wall);
    assert_eq!((p.path[0], *p.path.last().unwrap()), (Point::mm(0.0, 0.0), Point::mm(20.0, 0.0)), "ends stay");
    assert!(p.path.iter().any(|q| q.y > mm(1.0)), "bulges over the head's end: {p:?}");
    octilinear(&p.path);
    apply_and_check(&mut ed, &r);
}

#[test]
fn shove_cascades_through_parallel_tracks() {
    let mut ed = Editor::new(Project::new("T"));
    for y in [0.0, 0.6, 1.2] {
        ids(&mut ed, &[Point::mm(0.0, y), Point::mm(20.0, y)]);
    }
    let copper = conn(ed.project());
    let r = shove(ed.project(), &copper, L, W, None, &[Point::mm(10.0, -5.0), Point::mm(10.0, 0.5)]);
    assert!(r.reached, "{r:?}");
    assert_eq!(r.shoved.lines.len(), 3, "{r:?}");
    apply_and_check(&mut ed, &r);
}

fn ed_track_locked() -> Track {
    Track { locked: true, ..Track::straight(L, Point::mm(0.0, 0.0), Point::mm(20.0, 0.0), W) }
}

#[test]
fn a_locked_track_stops_the_head() {
    let mut ed = Editor::new(Project::new("T"));
    let wall = ids(&mut ed, &[Point::mm(0.0, 0.0), Point::mm(20.0, 0.0)]);
    let mut t = ed.project().board.tracks.get(wall[0]).unwrap().clone();
    t.locked = true;
    ed.execute(Command::Update { objects: vec![Object::Track(t)] }).unwrap();
    let copper = conn(ed.project());
    // Ending beside it, too close (on it would connect to it): stops below it.
    let r = shove(ed.project(), &copper, L, W, None, &[Point::mm(10.0, -5.0), Point::mm(10.0, -0.2)]);
    assert!(!r.reached);
    assert!(r.shoved.is_empty());
    let end = *r.points.last().unwrap();
    assert!(end.y < mm(-0.45) && end.y > mm(-1.0), "stops below it: {r:?}");
    apply_and_check(&mut ed, &r);
    // Past it: the head goes round it (it can't be pushed), nothing moves.
    let mut ed = Editor::new(Project::new("T"));
    ed.execute(Command::Insert { objects: vec![Object::Track(ed_track_locked())] }).unwrap();
    let copper = conn(ed.project());
    let r = shove(ed.project(), &copper, L, W, None, &[Point::mm(10.0, -5.0), Point::mm(10.0, 1.0)]);
    assert!(r.reached && r.shoved.is_empty(), "{r:?}");
    assert!(r.points.len() > 2);
    apply_and_check(&mut ed, &r);
}

fn via(ed: &mut Editor, at: Point) -> Id {
    let c = ed.execute(Command::AddVia { pos: at, diameter: Some(mm(0.6)), drill: Some(mm(0.3)), span: None }).unwrap();
    c.ids().find_map(|id| if let ObjectId::Via(v) = id { Some(v) } else { None }).unwrap()
}

#[test]
fn shove_a_via_with_its_tracks_on_both_layers() {
    let mut ed = Editor::new(Project::new("T"));
    let at = Point::mm(10.0, 0.5);
    let v = via(&mut ed, at);
    let top = ids(&mut ed, &[at, Point::mm(20.0, 0.5)]);
    let bottom = ed
        .execute(Command::AddTrack {
            layer: LayerId::BOTTOM_COPPER,
            points: vec![at, Point::mm(0.0, 0.5)],
            width: Some(W),
            mids: Vec::new(),
        })
        .unwrap();
    let bottom: Vec<Id> =
        bottom.ids().filter_map(|id| if let ObjectId::Track(t) = id { Some(t) } else { None }).collect();
    let copper = conn(ed.project());
    let r = shove(ed.project(), &copper, L, W, None, &[Point::mm(10.0, -5.0), Point::mm(10.0, 0.0)]);
    assert!(r.reached, "{r:?}");
    assert_eq!(r.shoved.vias.len(), 1, "{r:?}");
    let to = r.shoved.vias[0].to;
    assert_eq!(r.shoved.vias[0].id, v);
    assert!(to.y >= mm(0.62), "moved up, clear of the head's end: {to:?}");
    // Its tracks come along, on both layers.
    for chain in [&top, &bottom] {
        let p = r.shoved.lines.iter().find(|p| &p.line.tracks == chain).expect("dragged");
        assert!(p.path.contains(&to), "{p:?}");
    }
    apply_and_check(&mut ed, &r);
    // The via is still connected to both tracks.
    let b = &ed.project().board;
    assert!(b.tracks.iter().filter(|t| t.a == to || t.b == to).count() >= 2);
}

#[test]
fn a_locked_via_stops_the_head() {
    let mut ed = Editor::new(Project::new("T"));
    let v = via(&mut ed, Point::mm(10.0, 0.5));
    let mut o = ed.project().board.vias.get(v).unwrap().clone();
    o.locked = true;
    ed.execute(Command::Update { objects: vec![Object::Via(o)] }).unwrap();
    let copper = conn(ed.project());
    let r = shove(ed.project(), &copper, L, W, None, &[Point::mm(10.0, -5.0), Point::mm(10.0, 0.0)]);
    assert!(!r.reached, "{r:?}");
    assert!(r.shoved.is_empty());
    apply_and_check(&mut ed, &r);
}

#[test]
fn a_new_via_pushes_tracks() {
    let mut ed = Editor::new(Project::new("T"));
    let wall = ids(&mut ed, &[Point::mm(0.0, 0.0), Point::mm(20.0, 0.0)]);
    let copper = conn(ed.project());
    let all = (1u64 << 63) | 1; // both outer layers
    let mask = copper.items.iter().map(|c| c.layers).fold(0, |a, l| a | l) | all;
    let at = Point::mm(10.0, -0.3);
    let s = editor_core::router::shove_via(ed.project(), &copper, L, None, at, mm(0.6), mask).unwrap();
    assert_eq!(s.lines.len(), 1);
    assert_eq!(s.lines[0].line.tracks, wall);
    ed.execute(Command::AddVia { pos: at, diameter: Some(mm(0.6)), drill: Some(mm(0.3)), span: None }).unwrap();
    for c in s.commands(ed.project()) {
        ed.execute(c).unwrap();
    }
    let copper = conn(ed.project());
    let bad: Vec<String> = editor_core::drc::run(ed.project(), &copper)
        .into_iter()
        .filter(|v| matches!(v.kind, ViolationKind::Clearance | ViolationKind::Short))
        .map(|v| v.message)
        .collect();
    assert!(bad.is_empty(), "{bad:?}");
}

fn no_clashes(ed: &Editor) {
    let copper = conn(ed.project());
    let bad: Vec<String> = editor_core::drc::run(ed.project(), &copper)
        .into_iter()
        .filter(|v| matches!(v.kind, ViolationKind::Clearance | ViolationKind::Short))
        .map(|v| v.message)
        .collect();
    assert!(bad.is_empty(), "{bad:?}");
}

/// Move `ids` by `delta` (keeping what's attached), then push aside what they hit.
/// One transaction, as the app's drag steps (so the tidy comes after the pushes).
fn drag(ed: &mut Editor, ids: Vec<ObjectId>, delta: Point) -> Result<editor_core::router::shove::Shoved, String> {
    let before = conn(ed.project());
    let mut shoved = None;
    ed.execute_then(Command::Move { ids, delta, keep_connected: true }, None, |project, moved| {
        let s = editor_core::router::shove_moved(project, &before, moved)
            .map_err(|c| editor_core::CommandError::Invalid(c.message))?;
        let commands = s.commands(project);
        shoved = Some(s);
        Ok(commands)
    })
    .map_err(|e| e.to_string())?;
    Ok(shoved.expect("set"))
}

#[test]
fn a_dragged_segment_pushes_tracks() {
    let mut ed = Editor::new(Project::new("T"));
    let wall = ids(&mut ed, &[Point::mm(0.0, 0.0), Point::mm(20.0, 0.0)]);
    // A dog-leg: its middle segment is dragged down onto the wall.
    let leg = ids(&mut ed, &[Point::mm(0.0, 7.0), Point::mm(4.0, 3.0), Point::mm(16.0, 3.0), Point::mm(20.0, 7.0)]);
    let s = drag(&mut ed, vec![ObjectId::Track(leg[1])], Point::mm(0.0, -3.0)).unwrap();
    assert_eq!(s.lines.len(), 1, "{s:?}");
    assert_eq!(s.lines[0].line.tracks, wall);
    no_clashes(&ed);
    // Square corners (where it ran into the hull's sides) are cut at 45°.
    let path = &s.lines[0].path;
    octilinear(path);
    for w in path.windows(3) {
        let (u, v) = (w[1] - w[0], w[2] - w[1]);
        let dot = u.x as f64 * v.x as f64 + u.y as f64 * v.y as f64;
        assert!(dot > 0.0, "turns by 45° only: {path:?}");
    }
}

#[test]
fn a_dragged_via_pushes_tracks_and_a_locked_via_stops_it() {
    let mut ed = Editor::new(Project::new("T"));
    let wall = ids(&mut ed, &[Point::mm(0.0, 0.0), Point::mm(20.0, 0.0)]);
    let v = via(&mut ed, Point::mm(10.0, 3.0));
    ids(&mut ed, &[Point::mm(10.0, 3.0), Point::mm(10.0, 8.0)]);
    let s = drag(&mut ed, vec![ObjectId::Via(v)], Point::mm(0.0, -2.6)).unwrap();
    assert_eq!(s.lines.len(), 1, "{s:?}");
    assert_eq!(s.lines[0].line.tracks, wall);
    no_clashes(&ed);
    // A locked via can't be pushed.
    let mut ed = Editor::new(Project::new("T"));
    let fixed = via(&mut ed, Point::mm(10.0, 0.0));
    let mut o = ed.project().board.vias.get(fixed).unwrap().clone();
    o.locked = true;
    ed.execute(Command::Update { objects: vec![Object::Via(o)] }).unwrap();
    let v = via(&mut ed, Point::mm(10.0, 3.0));
    assert!(drag(&mut ed, vec![ObjectId::Via(v)], Point::mm(0.0, -2.6)).is_err());
}

/// `cargo test --release -p editor-core --test all deep_cascade -- --ignored --nocapture`
#[test]
#[ignore]
fn deep_cascade_timing() {
    for n in [10, 20, 40] {
        let mut ed = Editor::new(Project::new("T"));
        for i in 0..n {
            let y = 0.46 * i as f64;
            ids(&mut ed, &[Point::mm(0.0, y), Point::mm(40.0, y)]);
        }
        let copper = conn(ed.project());
        let _ = copper.query(Rect::around(Point::ZERO, 0)).count(); // build the index
        let t = std::time::Instant::now();
        let r = shove(ed.project(), &copper, L, W, None, &[Point::mm(20.0, -5.0), Point::mm(20.0, 0.5)]);
        eprintln!("{n} lines: {:?}, reached {}, pushed {}", t.elapsed(), r.reached, r.shoved.lines.len());
    }
}

#[test]
fn walking_round_a_row_of_vias_is_one_detour() {
    let mut ed = Editor::new(Project::new("T"));
    for x in [8.0, 9.5, 11.0, 12.5] {
        via(&mut ed, Point::mm(x, 0.0));
    }
    let copper = conn(ed.project());
    let r = walkaround(ed.project(), &copper, L, W, None, &[Point::mm(0.0, 0.0), Point::mm(20.0, 0.0)]);
    assert!(r.reached, "{r:?}");
    clean(ed.project(), &copper, &r.points);
    octilinear(&r.points);
    // Up, along, down: no steps between the vias.
    assert!(r.points.len() <= 6, "{:?}", r.points);
    for w in r.points.windows(3) {
        let (u, v) = (w[1] - w[0], w[2] - w[1]);
        assert!(u.x as f64 * v.x as f64 + u.y as f64 * v.y as f64 > 0.0, "45° turns: {:?}", r.points);
    }
}

#[test]
fn hugs_a_track_that_cannot_give_way_and_pushes_the_rest() {
    let mut ed = Editor::new(Project::new("T"));
    // A short track pinned under a locked one: it can't be pushed up.
    let pinned = ids(&mut ed, &[Point::mm(8.0, 0.0), Point::mm(12.0, 0.0)]);
    let lock = Track { locked: true, ..Track::straight(L, Point::mm(5.0, 0.5), Point::mm(15.0, 0.5), W) };
    ed.execute(Command::Insert { objects: vec![Object::Track(lock)] }).unwrap();
    // Further along, a free track the head can push.
    let free =
        ids(&mut ed, &[Point::mm(15.0, -3.0), Point::mm(17.0, -0.6), Point::mm(19.0, -0.6), Point::mm(21.0, -3.0)]);
    let copper = conn(ed.project());
    let r = shove(ed.project(), &copper, L, W, None, &[Point::mm(0.0, -0.3), Point::mm(25.0, -0.3)]);
    assert!(r.reached, "goes round the pinned track: {r:?}");
    assert!(r.shoved.lines.iter().all(|p| p.line.tracks != pinned), "{r:?}");
    assert!(r.shoved.lines.iter().any(|p| p.line.tracks == free), "pushes the free one: {r:?}");
    apply_and_check(&mut ed, &r);
}
