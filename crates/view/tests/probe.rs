//! Cross-probing between schematic and board, and board view consistency after
//! schematic edits.

use editor_core::Command;
use model::*;
use view::demo::demo_project;
use view::probe::cross_probe;
use view::{Document, ViewKind};

fn component(doc: &Document, refdes: &str) -> Id {
    doc.project().schematic.component_by_refdes(refdes).unwrap().id
}

fn probe(doc: &mut Document, from: ViewKind, sel: &[ObjectId]) -> view::probe::Probe {
    doc.update_connectivity(false);
    cross_probe(doc.project(), doc.connectivity(), doc.copper(), from, sel)
}

#[test]
fn symbols_and_footprints_probe_each_other() {
    let mut doc = Document::new(demo_project());
    let r1 = component(&doc, "R1");
    let sym = doc.project().schematic.symbols_of(r1).next().unwrap().id;
    let fp = doc.project().board.footprint_of(r1).unwrap().id;
    let p = probe(&mut doc, ViewKind::Schematic, &[ObjectId::Symbol(sym)]);
    assert_eq!(p.objects, vec![ObjectId::Footprint(fp)]);
    let p = probe(&mut doc, ViewKind::Board, &[ObjectId::Footprint(fp)]);
    assert_eq!(p.objects, vec![ObjectId::Symbol(sym)]);
}

#[test]
fn nets_probe_between_wires_and_tracks() {
    let mut doc = Document::new(demo_project());
    // The R1–D1 wire's net is routed on the board.
    let wire = doc.project().schematic.wires.iter().next().unwrap().id;
    let p = probe(&mut doc, ViewKind::Schematic, &[ObjectId::Wire(wire)]);
    assert_eq!(p.nets.len(), 1);
    let net = p.nets[0].clone();
    let copper = doc.copper();
    let idx = copper.net_index(&net).expect("net is on the board");
    let track = copper
        .items_of_net(idx)
        .find_map(|c| if let editor_core::copper::CopperItem::Track(t) = c.item { Some(t) } else { None })
        .expect("routed");
    // And back: the track shows the net's wires in the schematic.
    let p = probe(&mut doc, ViewKind::Board, &[ObjectId::Track(track)]);
    assert_eq!(p.nets, vec![net]);
    assert!(p.objects.contains(&ObjectId::Wire(wire)));
    // Highlights reach the views.
    doc.highlight_nets(&p.nets);
    assert_eq!(doc.board.highlighted_nets(), p.nets.as_slice());
    let before = doc.schematic.scene.instance_count();
    let (project, view) = doc.project_and_view(ViewKind::Schematic);
    view.set_probe(project, &p.objects);
    assert!(doc.schematic.scene.instance_count() > before);
}

#[test]
fn board_view_stays_consistent_through_schematic_edits() {
    let mut doc = Document::new(demo_project());
    let r1 = component(&doc, "R1");
    let r2 = component(&doc, "R2");
    let v0805 = doc.project().library.parts.get(builtin::PART_RESISTOR).unwrap().variants[1].id;
    doc.execute(Command::SetComponent { id: r1, refdes: Some("R7".into()), value: None, variant: Some(v0805) })
        .unwrap();
    let s2 = doc.project().schematic.symbols_of(r2).next().unwrap().id;
    doc.execute(Command::Delete { ids: vec![ObjectId::Symbol(s2)] }).unwrap();
    doc.execute(Command::SetCopperLayers { count: 4 }).unwrap();
    doc.undo();
    doc.undo();
    doc.redo();
    doc.update_connectivity(false);
    let mut fresh = view::View::new(ViewKind::Board, doc.project());
    fresh.set_copper(doc.project(), doc.copper());
    assert_eq!(doc.board.scene.instance_count(), fresh.scene.instance_count());
    assert_eq!(doc.board.index.len(), fresh.index.len());
    // R2's routed copper is still there, flagged where it lost its pads.
    assert!(!doc.project().board.tracks.is_empty());
}
