//! Dragging wires and tracks through the interaction: replayed from the start, single
//! segments constrained, double click chains.

use editor_core::Command;
use model::*;
use view::{Document, Interaction, KeyAction, PointerInput, ProjectTarget, ViewId};

fn input(p: Point) -> PointerInput {
    PointerInput { world: Some(p), tolerance: mm(0.3), drag_threshold: mm(0.5), ..Default::default() }
}

fn hover(it: &mut Interaction<ObjectId>, t: &mut ProjectTarget, p: Point) {
    it.pointer(t, &input(p));
}

fn press(it: &mut Interaction<ObjectId>, t: &mut ProjectTarget, p: Point) {
    it.pointer(t, &PointerInput { pressed: true, down: true, ..input(p) });
}

fn drag(it: &mut Interaction<ObjectId>, t: &mut ProjectTarget, p: Point) {
    it.pointer(t, &PointerInput { down: true, ..input(p) });
}

fn release(it: &mut Interaction<ObjectId>, t: &mut ProjectTarget, p: Point) {
    it.pointer(t, &PointerInput { released: true, ..input(p) });
}

fn p(x: f64, y: f64) -> Point {
    Point::mm(x, y)
}

fn wires(doc: &mut Document, points: &[Point]) -> Vec<Id> {
    let c = doc.execute(Command::AddWire { sheet: None, points: points.to_vec() }).unwrap();
    c.ids().filter_map(|id| if let ObjectId::Wire(w) = id { Some(w) } else { None }).collect()
}

fn ends(doc: &Document, w: Id) -> (Point, Point) {
    let w = doc.project().schematic.wires.get(w).unwrap();
    (w.a, w.b)
}

#[test]
fn a_single_segment_moves_across_itself_only() {
    let mut doc = Document::new(Project::new("t"));
    let v = doc.root();
    let w = wires(&mut doc, &[p(0.0, 0.0), p(0.0, 10.16), p(10.16, 10.16), p(10.16, 0.0)]);
    let mut it = Interaction::new(mm(1.27));
    press(&mut it, &mut doc.target(v), p(5.08, 10.16));
    for q in [p(6.0, 12.0), p(7.62, 15.24)] {
        drag(&mut it, &mut doc.target(v), q);
    }
    release(&mut it, &mut doc.target(v), p(7.62, 15.24));
    assert_eq!(ends(&doc, w[1]), (p(0.0, 15.24), p(10.16, 15.24)));
    assert_eq!(ends(&doc, w[0]), (p(0.0, 0.0), p(0.0, 15.24)));
    // One undo step, nothing to redo from the replayed steps.
    doc.undo();
    assert_eq!(ends(&doc, w[1]), (p(0.0, 10.16), p(10.16, 10.16)));
    assert_eq!(doc.editor().history().undo_label(), Some("Add wire"));
    assert_eq!(doc.editor().history().redo_label(), Some("Move 1 object(s)"));
}

#[test]
fn a_drag_never_picks_up_what_it_passes_over() {
    let mut doc = Document::new(Project::new("t"));
    let v = doc.root();
    // A diagonal wire drags freely; its end passes over the end of another wire.
    let d = wires(&mut doc, &[p(0.0, 0.0), p(5.08, 5.08)])[0];
    let other = wires(&mut doc, &[p(10.16, 0.0), p(10.16, 10.16)])[0];
    let mut it = Interaction::new(mm(1.27));
    press(&mut it, &mut doc.target(v), p(2.54, 2.54));
    assert_eq!(it.selection, vec![ObjectId::Wire(d)]);
    drag(&mut it, &mut doc.target(v), p(3.0, 2.0));
    drag(&mut it, &mut doc.target(v), p(7.62, -2.54));
    assert_eq!(ends(&doc, d).1, p(10.16, 0.0), "touching the other wire's end on the way");
    drag(&mut it, &mut doc.target(v), p(10.16, -2.54));
    release(&mut it, &mut doc.target(v), p(10.16, -2.54));
    assert_eq!(ends(&doc, d), (p(7.62, -5.08), p(12.7, 0.0)));
    assert_eq!(ends(&doc, other), (p(10.16, 0.0), p(10.16, 10.16)));
    assert_eq!(doc.project().schematic.wires.len(), 2);
}

#[test]
fn escape_after_a_replayed_drag_leaves_nothing_to_redo() {
    let mut doc = Document::new(Project::new("t"));
    let v = doc.root();
    let w = wires(&mut doc, &[p(0.0, 0.0), p(10.16, 0.0)])[0];
    let mut it = Interaction::new(mm(1.27));
    press(&mut it, &mut doc.target(v), p(5.08, 0.0));
    drag(&mut it, &mut doc.target(v), p(5.08, 5.08));
    drag(&mut it, &mut doc.target(v), p(5.08, 7.62));
    it.key(&mut doc.target(v), KeyAction::Escape);
    assert_eq!(ends(&doc, w), (p(0.0, 0.0), p(10.16, 0.0)));
    assert_eq!(doc.editor().history().redo_label(), None);
    assert_eq!(doc.editor().history().undo_label(), Some("Add wire"));
}

#[test]
fn double_click_selects_the_chain() {
    let mut doc = Document::new(Project::new("t"));
    let v = doc.root();
    let run = wires(&mut doc, &[p(0.0, 0.0), p(10.16, 0.0), p(10.16, 10.16)]);
    let branch = wires(&mut doc, &[p(10.16, 10.16), p(20.32, 10.16)]);
    let other = wires(&mut doc, &[p(10.16, 10.16), p(10.16, 20.32)]);
    let mut it = Interaction::new(mm(1.27));
    hover(&mut it, &mut doc.target(v), p(5.08, 0.0));
    it.double_click(&mut doc.target(v), false);
    let mut sel = it.selection.clone();
    sel.sort();
    let mut want: Vec<ObjectId> = run.iter().map(|&w| ObjectId::Wire(w)).collect();
    want.sort();
    assert_eq!(sel, want);
    // Adding another chain.
    hover(&mut it, &mut doc.target(v), p(15.24, 10.16));
    it.double_click(&mut doc.target(v), true);
    assert_eq!(it.selection.len(), 3);
    assert!(it.selection.contains(&ObjectId::Wire(branch[0])));
    assert!(!it.selection.contains(&ObjectId::Wire(other[0])));
}

#[test]
fn board_drag_keeps_tracks_on_a_via() {
    let mut doc = Document::new(Project::new("t"));
    let c = doc.execute(Command::AddVia { pos: p(0.0, 0.0), diameter: None, drill: None, span: None }).unwrap();
    let via = c.ids().find_map(|id| if let ObjectId::Via(v) = id { Some(v) } else { None }).unwrap();
    let c = doc
        .execute(Command::AddTrack {
            layer: LayerId::TOP_COPPER,
            points: vec![p(0.0, 0.0), p(10.0, 0.0)],
            width: None,
            mids: Vec::new(),
        })
        .unwrap();
    let t = c.ids().find_map(|id| if let ObjectId::Track(t) = id { Some(t) } else { None }).unwrap();
    let mut it = Interaction::new(mm(1.0));
    let v = ViewId::Board;
    it.set_selection(&mut doc.target(v), vec![ObjectId::Via(via)]);
    press(&mut it, &mut doc.target(v), p(0.0, 0.0));
    drag(&mut it, &mut doc.target(v), p(0.0, 2.0));
    drag(&mut it, &mut doc.target(v), p(0.0, 3.0));
    release(&mut it, &mut doc.target(v), p(0.0, 3.0));
    let tr = doc.project().board.tracks.get(t).unwrap();
    assert_eq!((tr.a, tr.b), (p(0.0, 3.0), p(10.0, 0.0)));
}
