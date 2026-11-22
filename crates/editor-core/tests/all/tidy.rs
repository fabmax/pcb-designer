//! Wires and tracks are tidied after every command: split at T connections (and
//! junctions, vias), joined when collinear with nothing between them, and never stacked.

use editor_core::{Command, Editor};
use model::builtin;
use model::*;

fn p(x: f64, y: f64) -> Point {
    Point::mm(x, y)
}

fn wire(ed: &mut Editor, points: &[Point]) {
    ed.execute(Command::AddWire { sheet: None, points: points.to_vec() }).unwrap();
}

/// All wires as sorted (a, b) pairs, each with its ends sorted.
fn wires(ed: &Editor) -> Vec<(Point, Point)> {
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

fn id_at(ed: &Editor, a: Point, b: Point) -> Id {
    ed.project().schematic.wires.iter().find(|w| (w.a, w.b) == (a, b) || (w.a, w.b) == (b, a)).unwrap().id
}

fn label(ed: &mut Editor, pos: Point) {
    ed.execute(Command::AddLabel {
        sheet: None,
        pos,
        name: "N".into(),
        kind: LabelKind::Local,
        shape: LabelShape::Passive,
        style: None,
        rotation: Angle::ZERO,
    })
    .unwrap();
}

#[test]
fn a_tee_splits_the_wire_it_lands_on() {
    let mut ed = Editor::new(Project::new("t"));
    wire(&mut ed, &[p(0.0, 0.0), p(20.0, 0.0)]);
    let id = ed.project().schematic.wires.iter().next().unwrap().id;
    wire(&mut ed, &[p(10.0, 10.0), p(10.0, 0.0)]);
    assert_eq!(wires(&ed), [(p(0.0, 0.0), p(10.0, 0.0)), (p(10.0, 0.0), p(10.0, 10.0)), (p(10.0, 0.0), p(20.0, 0.0))]);
    // The existing wire keeps its id for one piece.
    assert!(ed.project().schematic.wires.contains(id));
    // A new wire passing over an existing wire's end is split there too.
    wire(&mut ed, &[p(30.0, -10.0), p(30.0, 10.0)]);
    wire(&mut ed, &[p(20.0, 0.0), p(30.0, 0.0)]);
    assert!(wires(&ed).contains(&(p(30.0, -10.0), p(30.0, 0.0))));
    assert!(wires(&ed).contains(&(p(30.0, 0.0), p(30.0, 10.0))));
    // Undo puts it back in one step.
    ed.undo();
    assert!(wires(&ed).contains(&(p(30.0, -10.0), p(30.0, 10.0))));
}

#[test]
fn a_junction_splits_crossing_wires() {
    let mut ed = Editor::new(Project::new("t"));
    wire(&mut ed, &[p(0.0, 0.0), p(20.0, 0.0)]);
    wire(&mut ed, &[p(10.0, -10.0), p(10.0, 10.0)]);
    // A crossing without junction doesn't connect: nothing splits.
    assert_eq!(wires(&ed).len(), 2);
    let c = ed.execute(Command::AddJunction { sheet: None, pos: p(10.0, 0.0) }).unwrap();
    assert_eq!(wires(&ed).len(), 4);
    // Without the junction the four ends still meet, so they stay connected (joined, they
    // would only cross).
    let j = c.ids().find(|id| matches!(id, ObjectId::Junction(_))).unwrap();
    ed.execute(Command::Delete { ids: vec![j] }).unwrap();
    assert_eq!(wires(&ed).len(), 4);
    // A junction on a straight wire alone splits it too, and joins it back when deleted.
    wire(&mut ed, &[p(0.0, 20.0), p(20.0, 20.0)]);
    let c = ed.execute(Command::AddJunction { sheet: None, pos: p(5.0, 20.0) }).unwrap();
    assert!(wires(&ed).contains(&(p(0.0, 20.0), p(5.0, 20.0))));
    let j = c.ids().find(|id| matches!(id, ObjectId::Junction(_))).unwrap();
    ed.execute(Command::Delete { ids: vec![j] }).unwrap();
    assert!(wires(&ed).contains(&(p(0.0, 20.0), p(20.0, 20.0))));
}

#[test]
fn collinear_pieces_join() {
    let mut ed = Editor::new(Project::new("t"));
    // One polyline with a point in the middle of a straight run.
    wire(&mut ed, &[p(0.0, 0.0), p(5.0, 0.0), p(10.0, 0.0), p(10.0, 10.0)]);
    assert_eq!(wires(&ed), [(p(0.0, 0.0), p(10.0, 0.0)), (p(10.0, 0.0), p(10.0, 10.0))]);
    // Piece by piece, keeping the first wire's id.
    let first = id_at(&ed, p(0.0, 0.0), p(10.0, 0.0));
    wire(&mut ed, &[p(-10.0, 0.0), p(0.0, 0.0)]);
    assert_eq!(id_at(&ed, p(-10.0, 0.0), p(10.0, 0.0)), first);
    // Not across a label (or anything else attached there).
    label(&mut ed, p(10.0, 20.0));
    wire(&mut ed, &[p(10.0, 10.0), p(10.0, 20.0)]);
    wire(&mut ed, &[p(10.0, 20.0), p(10.0, 30.0)]);
    assert_eq!(
        wires(&ed),
        [(p(-10.0, 0.0), p(10.0, 0.0)), (p(10.0, 0.0), p(10.0, 20.0)), (p(10.0, 20.0), p(10.0, 30.0))]
    );
}

#[test]
fn deleting_a_branch_joins_what_it_split() {
    let mut ed = Editor::new(Project::new("t"));
    wire(&mut ed, &[p(0.0, 0.0), p(20.0, 0.0)]);
    wire(&mut ed, &[p(10.0, 0.0), p(10.0, 10.0)]);
    let branch = id_at(&ed, p(10.0, 0.0), p(10.0, 10.0));
    ed.execute(Command::Delete { ids: vec![ObjectId::Wire(branch)] }).unwrap();
    assert_eq!(wires(&ed), [(p(0.0, 0.0), p(20.0, 0.0))]);
}

#[test]
fn stacked_wires_become_one() {
    let mut ed = Editor::new(Project::new("t"));
    wire(&mut ed, &[p(0.0, 0.0), p(20.0, 0.0)]);
    let id = ed.project().schematic.wires.iter().next().unwrap().id;
    // On top, inside: absorbed.
    wire(&mut ed, &[p(5.0, 0.0), p(15.0, 0.0)]);
    assert_eq!(wires(&ed), [(p(0.0, 0.0), p(20.0, 0.0))]);
    // Overlapping and reaching beyond: one longer wire, still the first one.
    wire(&mut ed, &[p(30.0, 0.0), p(15.0, 0.0)]);
    assert_eq!(wires(&ed), [(p(0.0, 0.0), p(30.0, 0.0))]);
    assert!(ed.project().schematic.wires.contains(id));
    // Diagonal wires too.
    wire(&mut ed, &[p(0.0, 10.0), p(10.0, 20.0)]);
    wire(&mut ed, &[p(5.0, 15.0), p(15.0, 25.0)]);
    assert!(wires(&ed).contains(&(p(0.0, 10.0), p(15.0, 25.0))));
    assert_eq!(wires(&ed).len(), 2);
}

#[test]
fn moved_wires_keep_their_ids() {
    let mut ed = Editor::new(Project::new("t"));
    wire(&mut ed, &[p(0.0, 0.0), p(20.0, 0.0)]);
    wire(&mut ed, &[p(10.0, 5.0), p(10.0, 15.0)]);
    let v = id_at(&ed, p(10.0, 5.0), p(10.0, 15.0));
    // Dropped onto the line: the line splits, the moved wire stays whole.
    let mv = |delta| Command::Move { ids: vec![ObjectId::Wire(v)], delta, keep_connected: true };
    ed.execute(mv(p(0.0, -5.0))).unwrap();
    assert_eq!(id_at(&ed, p(10.0, 0.0), p(10.0, 10.0)), v);
    assert_eq!(wires(&ed).len(), 3);
    // Undone (a drag replays from its start), the line is one wire again.
    ed.undo();
    assert_eq!(wires(&ed), [(p(0.0, 0.0), p(20.0, 0.0)), (p(10.0, 5.0), p(10.0, 15.0))]);
    // A move without keep_connected (a carried paste's steps) leaves wires alone.
    let carried = Command::Move { ids: vec![ObjectId::Wire(v)], delta: p(0.0, -5.0), keep_connected: false };
    ed.execute(carried).unwrap();
    assert_eq!(wires(&ed).len(), 2);
}

#[test]
fn bus_wires_are_tidied_too() {
    let mut ed = Editor::new(Project::new("t"));
    for points in [[p(0.0, 0.0), p(10.0, 0.0)], [p(10.0, 0.0), p(20.0, 0.0)], [p(5.0, 0.0), p(15.0, 0.0)]] {
        ed.execute(Command::AddBus { sheet: None, points: points.to_vec() }).unwrap();
    }
    let b: Vec<_> = ed.project().schematic.bus_wires.iter().map(|w| (w.a, w.b)).collect();
    assert_eq!(b, [(p(0.0, 0.0), p(20.0, 0.0))]);
}

const TOP: LayerId = LayerId::TOP_COPPER;

fn track(ed: &mut Editor, points: &[Point], width: f64) {
    ed.execute(Command::AddTrack { layer: TOP, points: points.to_vec(), width: Some(mm(width)), mids: Vec::new() })
        .unwrap();
}

/// Tracks on the top layer as sorted (a, b, width) triples, each with its ends sorted.
fn tracks(ed: &Editor) -> Vec<(Point, Point, Coord)> {
    let mut out: Vec<_> = ed
        .project()
        .board
        .tracks
        .iter()
        .filter(|t| t.layer == TOP)
        .map(|t| {
            let (a, b) = if (t.a.x, t.a.y) <= (t.b.x, t.b.y) { (t.a, t.b) } else { (t.b, t.a) };
            (a, b, t.width)
        })
        .collect();
    out.sort_by_key(|(a, b, _)| (a.x, a.y, b.x, b.y));
    out
}

#[test]
fn tracks_split_at_tees_and_vias_and_join_in_line() {
    let mut ed = Editor::new(Project::new("t"));
    let w = mm(0.25);
    // Piece by piece in a line: one track.
    track(&mut ed, &[p(0.0, 0.0), p(10.0, 0.0)], 0.25);
    track(&mut ed, &[p(10.0, 0.0), p(20.0, 0.0)], 0.25);
    assert_eq!(tracks(&ed), [(p(0.0, 0.0), p(20.0, 0.0), w)]);
    // A tee splits it; a track on the other layer doesn't.
    track(&mut ed, &[p(5.0, 10.0), p(5.0, 0.0)], 0.25);
    assert_eq!(tracks(&ed).len(), 3);
    ed.execute(Command::AddTrack {
        layer: LayerId::BOTTOM_COPPER,
        points: vec![p(15.0, 10.0), p(15.0, 0.0)],
        width: Some(w),
        mids: Vec::new(),
    })
    .unwrap();
    assert_eq!(tracks(&ed).len(), 3);
    // A via on it splits it too, and joins it back when deleted.
    let c = ed.execute(Command::AddVia { pos: p(15.0, 0.0), diameter: None, drill: None, span: None }).unwrap();
    assert!(tracks(&ed).contains(&(p(5.0, 0.0), p(15.0, 0.0), w)));
    let via = c.ids().find(|id| matches!(id, ObjectId::Via(_))).unwrap();
    ed.execute(Command::Delete { ids: vec![via] }).unwrap();
    assert!(tracks(&ed).contains(&(p(5.0, 0.0), p(20.0, 0.0), w)));
}

#[test]
fn tracks_join_only_alike_and_not_on_pads() {
    let mut ed = Editor::new(Project::new("t"));
    // Different widths meet but stay two.
    track(&mut ed, &[p(0.0, 0.0), p(10.0, 0.0)], 0.25);
    track(&mut ed, &[p(10.0, 0.0), p(20.0, 0.0)], 0.5);
    assert_eq!(tracks(&ed).len(), 2);
    // Arcs are left alone.
    ed.execute(Command::AddTrack {
        layer: TOP,
        points: vec![p(20.0, 0.0), p(30.0, 0.0)],
        width: Some(mm(0.5)),
        mids: vec![Some(p(25.0, 5.0))],
    })
    .unwrap();
    assert_eq!(tracks(&ed).len(), 3);
    // Not across a pad.
    ed.execute(Command::import_parts(&builtin::builtin_library(), &[builtin::PART_MOUNTING_HOLE]).unwrap()).unwrap();
    ed.execute(Command::PlaceBoardPart {
        part: builtin::PART_MOUNTING_HOLE,
        variant: None,
        pos: p(50.0, 0.0),
        rotation: Angle::ZERO,
        side: Side::Top,
        refdes: None,
        value: None,
    })
    .unwrap();
    track(&mut ed, &[p(40.0, 0.0), p(50.0, 0.0)], 0.25);
    track(&mut ed, &[p(50.0, 0.0), p(60.0, 0.0)], 0.25);
    assert!(tracks(&ed).contains(&(p(40.0, 0.0), p(50.0, 0.0), mm(0.25))));
}

#[test]
fn stacked_tracks_of_one_width_become_one() {
    let mut ed = Editor::new(Project::new("t"));
    track(&mut ed, &[p(0.0, 0.0), p(20.0, 0.0)], 0.25);
    track(&mut ed, &[p(5.0, 0.0), p(30.0, 0.0)], 0.25);
    assert_eq!(tracks(&ed), [(p(0.0, 0.0), p(30.0, 0.0), mm(0.25))]);
    // A wider one on top stays (it's more copper); its ends split the one below.
    track(&mut ed, &[p(5.0, 0.0), p(15.0, 0.0)], 0.5);
    assert!(tracks(&ed).contains(&(p(5.0, 0.0), p(15.0, 0.0), mm(0.5))));
    assert_eq!(tracks(&ed).len(), 4, "{:?}", tracks(&ed));
}
