//! Moving with `keep_connected`: wires and tracks stay attached, and chains for double
//! click selection.

use editor_core::query::{pad_positions, pin_positions, track_chain, wire_chain};
use editor_core::{Command, Editor};
use model::builtin::{self, PART_RESISTOR};
use model::*;

fn p(x: f64, y: f64) -> Point {
    Point::mm(x, y)
}

fn editor() -> Editor {
    let mut ed = Editor::new(Project::new("test"));
    ed.execute(Command::import_parts(&builtin::builtin_library(), &[PART_RESISTOR]).unwrap()).unwrap();
    ed
}

/// Add a polyline of wires; returns their ids in order.
fn wires(ed: &mut Editor, points: &[Point]) -> Vec<Id> {
    let c = ed.execute(Command::AddWire { sheet: None, points: points.to_vec() }).unwrap();
    c.ids().filter_map(|id| if let ObjectId::Wire(w) = id { Some(w) } else { None }).collect()
}

fn mv(ed: &mut Editor, ids: Vec<ObjectId>, delta: Point) {
    ed.execute(Command::Move { ids, delta, keep_connected: true }).unwrap();
}

/// All wires as sorted (a, b) pairs, each with its ends sorted.
fn wire_set(ed: &Editor) -> Vec<(Point, Point)> {
    let mut out: Vec<(Point, Point)> = ed
        .project()
        .schematic
        .wires
        .iter()
        .map(|w| if (w.a.x, w.a.y) <= (w.b.x, w.b.y) { (w.a, w.b) } else { (w.b, w.a) })
        .collect();
    out.sort_by_key(|(a, b)| (a.x, a.y, b.x, b.y));
    out
}

fn place_resistor(ed: &mut Editor) -> (Id, Point) {
    let part = ed.project().library.parts.iter().next().unwrap().id;
    let c = ed
        .execute(Command::PlacePart {
            part,
            variant: None,
            sheet: None,
            pos: p(0.0, 0.0),
            rotation: Angle::ZERO,
            mirror: false,
            refdes: None,
            value: None,
        })
        .unwrap();
    let sym = c.ids().find_map(|id| if let ObjectId::Symbol(s) = id { Some(s) } else { None }).unwrap();
    let pin = pin_positions(ed.project(), sym)[0].2;
    (sym, pin)
}

#[test]
fn dragged_segment_stretches_perpendicular_neighbours() {
    let mut ed = editor();
    let w = wires(&mut ed, &[p(0.0, 0.0), p(0.0, 10.0), p(10.0, 10.0), p(10.0, 0.0)]);
    mv(&mut ed, vec![ObjectId::Wire(w[1])], p(0.0, 5.08));
    let y = p(0.0, 15.08).y;
    assert_eq!(
        wire_set(&ed),
        vec![
            (p(0.0, 0.0), Point::new(0, y)),
            (Point::new(0, y), Point::new(p(10.0, 0.0).x, y)),
            (p(10.0, 0.0), Point::new(p(10.0, 0.0).x, y))
        ]
    );
}

#[test]
fn wire_end_on_a_pin_is_held_by_a_dogleg() {
    let mut ed = editor();
    let (_, pin) = place_resistor(&mut ed);
    let far = pin + p(-10.16, 0.0);
    let w = wires(&mut ed, &[pin, far])[0];
    // A free move: the wire keeps its direction, its end slides back to the pin's row
    // and a vertical wire joins it to the pin.
    mv(&mut ed, vec![ObjectId::Wire(w)], p(2.54, 5.08));
    let up = pin + p(0.0, 5.08);
    let moved = ed.project().schematic.wires.get(w).unwrap();
    assert_eq!((moved.a, moved.b), (up, far + p(2.54, 5.08)));
    assert!(wire_set(&ed).contains(&if (pin.x, pin.y) <= (up.x, up.y) { (pin, up) } else { (up, pin) }));
    assert_eq!(ed.project().schematic.wires.len(), 2);
}

#[test]
fn collinear_neighbour_stays_and_a_dogleg_joins() {
    let mut ed = editor();
    let w = wires(&mut ed, &[p(0.0, 0.0), p(10.0, 0.0)]);
    let o = wires(&mut ed, &[p(10.0, 0.0), p(20.0, 0.0)]);
    mv(&mut ed, vec![ObjectId::Wire(w[0])], p(0.0, 5.0));
    let s = &ed.project().schematic;
    assert_eq!(s.wires.get(o[0]).map(|w| (w.a, w.b)), Some((p(10.0, 0.0), p(20.0, 0.0))));
    assert_eq!(s.wires.get(w[0]).map(|w| (w.a, w.b)), Some((p(0.0, 5.0), p(10.0, 5.0))));
    assert!(wire_set(&ed).contains(&(p(10.0, 0.0), p(10.0, 5.0))));
}

#[test]
fn tees_and_labels_on_a_moved_wire_slide_along() {
    let mut ed = editor();
    let h = wires(&mut ed, &[p(0.0, 0.0), p(20.0, 0.0)])[0];
    let v = wires(&mut ed, &[p(10.0, -10.0), p(10.0, 0.0)])[0];
    let c = ed
        .execute(Command::AddLabel {
            sheet: None,
            pos: p(5.0, 0.0),
            name: "N".into(),
            kind: LabelKind::Local,
            shape: LabelShape::Passive,
            style: None,
            rotation: Angle::ZERO,
        })
        .unwrap();
    let label = c.ids().find_map(|id| if let ObjectId::Label(l) = id { Some(l) } else { None }).unwrap();
    mv(&mut ed, vec![ObjectId::Wire(h)], p(3.0, 5.0));
    let s = &ed.project().schematic;
    assert_eq!(s.wires.get(h).map(|w| (w.a, w.b)), Some((p(3.0, 5.0), p(23.0, 5.0))));
    // Only the part across the wire: the tee stays vertical, the label stays put along it.
    assert_eq!(s.wires.get(v).map(|w| (w.a, w.b)), Some((p(10.0, -10.0), p(10.0, 5.0))));
    assert_eq!(s.labels.get(label).unwrap().pos, p(5.0, 5.0));
}

#[test]
fn collinear_wires_merge_after_a_drag() {
    // A Z: dragging its first leg level with the last makes the middle one vanish and
    // the two legs one wire, keeping the dragged wire's id.
    let mut ed = editor();
    let w = wires(&mut ed, &[p(0.0, 0.0), p(10.0, 0.0), p(10.0, 5.0), p(20.0, 5.0)]);
    mv(&mut ed, vec![ObjectId::Wire(w[0])], p(0.0, 5.0));
    let s = &ed.project().schematic;
    assert_eq!(s.wires.len(), 1);
    assert_eq!(s.wires.get(w[0]).map(|w| (w.a, w.b)), Some((p(0.0, 5.0), p(20.0, 5.0))));
}

#[test]
fn wire_chain_stops_at_branches_and_pins() {
    let mut ed = editor();
    let (_, pin) = place_resistor(&mut ed);
    // pin → corner → branch point, then two branches.
    let corner = pin + p(-10.0, 0.0);
    let branch = corner + p(0.0, 10.0);
    let run = wires(&mut ed, &[pin, corner, branch]);
    let b1 = wires(&mut ed, &[branch, branch + p(-10.0, 0.0)]);
    let b2 = wires(&mut ed, &[branch, branch + p(0.0, 10.0), branch + p(5.0, 10.0)]);
    let ids = |v: &[Id]| {
        let mut v: Vec<ObjectId> = v.iter().map(|&i| ObjectId::Wire(i)).collect();
        v.sort();
        v
    };
    let chain = |id: Id| {
        let mut c = wire_chain(ed.project(), ObjectId::Wire(id));
        c.sort();
        c
    };
    assert_eq!(chain(run[1]), ids(&run));
    assert_eq!(chain(b1[0]), ids(&b1));
    assert_eq!(chain(b2[1]), ids(&b2));
}

#[test]
fn retracted_step_leaves_no_redo() {
    let mut ed = editor();
    let w = wires(&mut ed, &[p(0.0, 0.0), p(10.0, 0.0)])[0];
    let mv = Command::Move { ids: vec![ObjectId::Wire(w)], delta: p(1.0, 0.0), keep_connected: true };
    ed.execute_grouped(mv, Some(42)).unwrap();
    assert!(ed.retract(7).is_none(), "only the open group");
    assert!(ed.retract(42).is_some());
    assert_eq!(ed.project().schematic.wires.get(w).unwrap().a, p(0.0, 0.0));
    assert_eq!(ed.history().redo_label(), None);
}

// ---- Board

fn track(ed: &mut Editor, points: &[Point]) -> Vec<Id> {
    let c = ed.execute(Command::AddTrack { layer: LayerId::TOP_COPPER, points: points.to_vec(), width: None }).unwrap();
    c.ids().filter_map(|id| if let ObjectId::Track(t) = id { Some(t) } else { None }).collect()
}

fn via(ed: &mut Editor, pos: Point) -> Id {
    let c = ed.execute(Command::AddVia { pos, diameter: None, drill: None }).unwrap();
    c.ids().find_map(|id| if let ObjectId::Via(v) = id { Some(v) } else { None }).unwrap()
}

fn ends(ed: &Editor, t: Id) -> (Point, Point) {
    let t = ed.project().board.tracks.get(t).unwrap();
    (t.a, t.b)
}

#[test]
fn tracks_follow_a_moved_via_and_slide_along_the_next() {
    let mut ed = editor();
    let v = via(&mut ed, p(0.0, 0.0));
    let t = track(&mut ed, &[p(0.0, 0.0), p(10.0, 0.0), p(20.0, 10.0)]);
    mv(&mut ed, vec![ObjectId::Via(v)], p(0.0, 3.0));
    // The first track moves along, staying horizontal; its far end slides down the 45°
    // track, which keeps its direction.
    assert_eq!(ends(&ed, t[0]), (p(0.0, 3.0), p(13.0, 3.0)));
    assert_eq!(ends(&ed, t[1]), (p(13.0, 3.0), p(20.0, 10.0)));
}

#[test]
fn dragged_track_slides_along_neighbours_and_is_joined_to_a_via() {
    let mut ed = editor();
    via(&mut ed, p(0.0, 0.0));
    let t = track(&mut ed, &[p(0.0, 0.0), p(10.0, 0.0), p(10.0, 10.0)]);
    mv(&mut ed, vec![ObjectId::Track(t[0])], p(0.0, 3.0));
    assert_eq!(ends(&ed, t[0]), (p(0.0, 3.0), p(10.0, 3.0)));
    assert_eq!(ends(&ed, t[1]), (p(10.0, 3.0), p(10.0, 10.0)));
    // A new track joins the via to the moved end.
    let b = &ed.project().board;
    assert_eq!(b.tracks.len(), 3);
    assert!(b.tracks.iter().any(|tr| (tr.a, tr.b) == (p(0.0, 0.0), p(0.0, 3.0))));
}

#[test]
fn moved_footprint_keeps_its_track() {
    let mut ed = editor();
    place_resistor(&mut ed);
    let fp = ed.project().board.footprints.iter().next().unwrap().id;
    let pad = pad_positions(ed.project(), fp)[0].2;
    let t = track(&mut ed, &[pad, pad + p(10.0, 0.0)])[0];
    mv(&mut ed, vec![ObjectId::Footprint(fp)], p(0.0, 2.0));
    // Nothing to slide along at the far end: the track stretches.
    assert_eq!(ends(&ed, t), (pad + p(0.0, 2.0), pad + p(10.0, 0.0)));
    // Without keep_connected it stays behind.
    ed.execute(Command::Move { ids: vec![ObjectId::Footprint(fp)], delta: p(0.0, 2.0), keep_connected: false })
        .unwrap();
    assert_eq!(ends(&ed, t), (pad + p(0.0, 2.0), pad + p(10.0, 0.0)));
}

#[test]
fn arc_tracks_keep_their_bulge() {
    let mut t = Track::straight(LayerId::TOP_COPPER, p(0.0, 0.0), p(10.0, 0.0), mm(0.2));
    t.mid = Some(p(5.0, 2.0));
    t.set_ends(p(0.0, 0.0), p(20.0, 0.0));
    assert_eq!(t.mid, Some(p(10.0, 4.0)));
    t.set_ends(p(1.0, 1.0), p(21.0, 1.0));
    assert_eq!(t.mid, Some(p(11.0, 5.0)));
}

#[test]
fn track_chain_stops_at_vias_and_branches() {
    let mut ed = editor();
    via(&mut ed, p(10.0, 0.0));
    let run = track(&mut ed, &[p(0.0, 0.0), p(5.0, 0.0), p(10.0, 0.0)]);
    let after = track(&mut ed, &[p(10.0, 0.0), p(20.0, 0.0), p(20.0, 10.0)]);
    let branch = track(&mut ed, &[p(20.0, 0.0), p(30.0, 0.0)]);
    let sorted = |mut v: Vec<Id>| {
        v.sort();
        v
    };
    assert_eq!(sorted(track_chain(ed.project(), run[0])), sorted(run.clone()));
    assert_eq!(track_chain(ed.project(), after[0]), vec![after[0]]);
    assert_eq!(track_chain(ed.project(), branch[0]), branch);
}

#[test]
fn three_collinear_wires_merge_into_one() {
    // Two Zs in a row: levelling the middle leg makes all three legs one wire.
    let mut ed = editor();
    let w = wires(&mut ed, &[p(0.0, 0.0), p(10.0, 0.0), p(10.0, 5.0), p(20.0, 5.0), p(20.0, 0.0), p(30.0, 0.0)]);
    mv(&mut ed, vec![ObjectId::Wire(w[2])], p(0.0, -5.0));
    let s = &ed.project().schematic;
    assert_eq!(s.wires.len(), 1);
    assert_eq!(s.wires.get(w[2]).map(|w| (w.a.x.min(w.b.x), w.a.x.max(w.b.x))), Some((0, p(30.0, 0.0).x)));
}
