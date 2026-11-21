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
    use editor_core::command::TrackPath;
    ed.execute(Command::AddTrack { layer: L, points: r.points.clone(), width: Some(W), mids: Vec::new() }).unwrap();
    let paths = r.pushed.iter().map(|p| TrackPath { tracks: p.line.tracks.clone(), path: p.path.clone() }).collect();
    ed.execute(Command::SetTrackPaths { paths }).unwrap();
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
    assert_eq!(r.pushed.len(), 1);
    let p = &r.pushed[0];
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
    assert_eq!(r.pushed.len(), 3, "{r:?}");
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
    assert!(r.pushed.is_empty());
    let end = *r.points.last().unwrap();
    assert!(end.y < mm(-0.45) && end.y > mm(-1.0), "stops below it: {r:?}");
    apply_and_check(&mut ed, &r);
    // Past it: the head goes round it (it can't be pushed), nothing moves.
    let mut ed = Editor::new(Project::new("T"));
    ed.execute(Command::Insert { objects: vec![Object::Track(ed_track_locked())] }).unwrap();
    let copper = conn(ed.project());
    let r = shove(ed.project(), &copper, L, W, None, &[Point::mm(10.0, -5.0), Point::mm(10.0, 1.0)]);
    assert!(r.reached && r.pushed.is_empty(), "{r:?}");
    assert!(r.points.len() > 2);
    apply_and_check(&mut ed, &r);
}
