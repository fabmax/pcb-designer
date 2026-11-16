//! Routing: rounding track corners, the checks while routing, blind and buried vias.

use editor_core::connectivity::Connectivity;
use editor_core::copper::BoardConnectivity;
use editor_core::{Command, Editor};
use model::*;

fn conn(p: &Project) -> BoardConnectivity {
    BoardConnectivity::build(p, &Connectivity::build(p))
}

fn track(ed: &mut Editor, points: &[Point]) -> Vec<Id> {
    let c = ed
        .execute(Command::AddTrack {
            layer: LayerId::TOP_COPPER,
            points: points.to_vec(),
            width: None,
            mids: Vec::new(),
        })
        .unwrap();
    c.ids().filter_map(|id| if let ObjectId::Track(t) = id { Some(t) } else { None }).collect()
}

#[test]
fn add_track_with_an_arc_segment() {
    let mut ed = Editor::new(Project::new("T"));
    let pts = vec![Point::mm(0.0, 0.0), Point::mm(5.0, 0.0), Point::mm(7.0, 2.0)];
    let mid = Point::mm(6.2, 0.6);
    ed.execute(Command::AddTrack { layer: LayerId::TOP_COPPER, points: pts, width: None, mids: vec![None, Some(mid)] })
        .unwrap();
    let t: Vec<&Track> = ed.project().board.tracks.iter().collect();
    assert_eq!((t[0].mid, t[1].mid), (None, Some(mid)));
}

#[test]
fn round_corners_of_a_path() {
    let mut ed = Editor::new(Project::new("T"));
    // Two corners: one 90°, one 45°.
    let ids =
        track(&mut ed, &[Point::mm(0.0, 0.0), Point::mm(10.0, 0.0), Point::mm(10.0, 10.0), Point::mm(15.0, 15.0)]);
    ed.execute(Command::RoundCorners { tracks: ids.clone(), radius: mm(1.0) }).unwrap();
    let b = &ed.project().board;
    let arcs: Vec<&Track> = b.tracks.iter().filter(|t| t.mid.is_some()).collect();
    assert_eq!(arcs.len(), 2);
    assert_eq!(b.tracks.len(), 5);
    // The 90° arc starts 1 mm before the corner and ends 1 mm after it.
    assert!(arcs.iter().any(|a| a.a == Point::mm(9.0, 0.0) && a.b == Point::mm(10.0, 1.0)));
    // Still one connected path: every end but the two outer ones is shared.
    let mut ends = std::collections::HashMap::new();
    for t in b.tracks.iter() {
        *ends.entry(t.a).or_insert(0) += 1;
        *ends.entry(t.b).or_insert(0) += 1;
    }
    assert_eq!(ends.values().filter(|&&n| n == 1).count(), 2, "{ends:?}");
    ed.undo().unwrap();
    assert_eq!(ed.project().board.tracks.len(), 3);
}

#[test]
fn round_corners_needs_room_and_corners() {
    let mut ed = Editor::new(Project::new("T"));
    let ids = track(&mut ed, &[Point::mm(0.0, 0.0), Point::mm(1.0, 0.0), Point::mm(1.0, 1.0)]);
    assert!(ed.execute(Command::RoundCorners { tracks: ids.clone(), radius: mm(5.0) }).is_err(), "too short");
    assert!(ed.execute(Command::RoundCorners { tracks: ids[..1].to_vec(), radius: mm(0.5) }).is_err(), "one track");
    // A branch at the corner: not a corner.
    let mut more = ids.clone();
    more.extend(track(&mut ed, &[Point::mm(1.0, 0.0), Point::mm(3.0, 0.0)]));
    assert!(ed.execute(Command::RoundCorners { tracks: more, radius: mm(0.5) }).is_err());
}

#[test]
fn keepouts_forbid_routes_and_vias() {
    use editor_core::route_check::{Probe, check};
    let mut ed = Editor::new(Project::new("T"));
    let k = Keepout::new(
        Vec::new(),
        vec![Point::mm(0.0, 0.0), Point::mm(10.0, 0.0), Point::mm(10.0, 10.0), Point::mm(0.0, 10.0)],
    );
    ed.execute(Command::Insert { objects: vec![Object::Keepout(k)] }).unwrap();
    let copper = conn(ed.project());
    let track = |a: Point, b: Point| Probe::Track { layer: LayerId::TOP_COPPER, a, b, mid: None, width: mm(0.25) };
    let p = ed.project();
    let c = check(p, &copper, &[track(Point::mm(-5.0, 5.0), Point::mm(5.0, 5.0))], None, &[]).unwrap();
    assert!(c.message.contains("keepout that forbids tracks"), "{}", c.message);
    assert!(c.at.x >= -1_000_000 && c.at.x <= 1_000_000, "marked where it enters: {:?}", c.at);
    assert!(check(p, &copper, &[track(Point::mm(-5.0, 5.0), Point::mm(-1.0, 5.0))], None, &[]).is_none());
    let via = Probe::Via { pos: Point::mm(5.0, 5.0), diameter: mm(0.6), layers: 1 };
    assert!(check(p, &copper, &[via], None, &[]).unwrap().message.contains("vias"));
}

#[test]
fn blind_and_buried_vias() {
    let mut ed = Editor::new(Project::new("T"));
    ed.execute(Command::SetCopperLayers { count: 4 }).unwrap();
    let (top, in1, in2) = (LayerId::TOP_COPPER, LayerId::inner(1), LayerId::inner(2));
    let add = |ed: &mut Editor, span| {
        ed.execute(Command::AddVia { pos: Point::mm(1.0, 1.0), diameter: None, drill: None, span })
    };
    add(&mut ed, Some([in2, in1])).unwrap();
    let v = ed.project().board.vias.iter().next().unwrap().clone();
    assert_eq!((v.from, v.to), (in1, in2), "stored top layer first");
    assert!(!v.is_through());
    assert!(add(&mut ed, Some([top, top])).is_err());
    assert!(add(&mut ed, Some([top, LayerId::inner(5)])).is_err(), "not on this board");
    add(&mut ed, None).unwrap();
    assert!(ed.project().board.vias.iter().any(|v| v.is_through()));
    // A blind via joins only its layers.
    add(&mut ed, Some([top, in1])).unwrap();
    let blind = ed.project().board.vias.iter().last().unwrap().clone();
    let mask = editor_core::copper::via_layers(&ed.project().board, &blind);
    assert_eq!(mask, editor_core::copper::layer_bit(top) | editor_core::copper::layer_bit(in1));

    // DRC flags spans edited into a bad state.
    let mut bad = blind.clone();
    (bad.from, bad.to) = (in1, top);
    ed.execute(Command::Update { objects: vec![Object::Via(bad)] }).unwrap();
    let copper = conn(ed.project());
    let v = editor_core::drc::run(ed.project(), &copper);
    assert!(v.iter().any(|v| v.kind == editor_core::drc::ViolationKind::ViaSpan), "{v:?}");
}
