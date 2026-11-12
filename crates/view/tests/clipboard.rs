//! Copy and paste: schematic (new components, parts travel with the clip), board, library
//! items, and carrying a paste with the pointer.

use editor_core::Command;
use editor_core::clip::{Clip, ClipContent};
use model::*;
use view::demo::demo_project;
use view::item::{Content, copy_elems, paste_elems};
use view::{Document, Elem, Interaction, KeyAction, PointerInput, ViewId, clipboard, interaction};

fn component<'a>(doc: &'a Document, refdes: &str) -> &'a Component {
    doc.project().schematic.component_by_refdes(refdes).unwrap()
}

fn symbol_of(doc: &Document, refdes: &str) -> SymbolInstance {
    let c = component(doc, refdes).id;
    doc.project().schematic.symbols_of(c).next().unwrap().clone()
}

/// R1 and the wires on its sheet, through the clipboard text.
fn copy_r1_and_wires(doc: &Document) -> (Clip, SymbolInstance, usize) {
    let r1 = symbol_of(doc, "R1");
    let wires: Vec<ObjectId> =
        doc.project().schematic.wires.iter().filter(|w| w.sheet == r1.sheet).map(|w| ObjectId::Wire(w.id)).collect();
    let mut ids = vec![ObjectId::Symbol(r1.id)];
    ids.extend(&wires);
    let (clip, note) = clipboard::copy(doc.project(), &ids, |p| p).unwrap();
    assert_eq!(note, None);
    assert_eq!(clip.anchor, r1.pos, "held by the first object");
    let text = clip.to_text();
    let back = Clip::from_text(&text).unwrap().unwrap();
    assert_eq!(back, clip);
    (back, r1, wires.len())
}

#[test]
fn paste_makes_new_components_on_another_sheet() {
    let mut doc = Document::new(demo_project());
    let (clip, r1, wires) = copy_r1_and_wires(&doc);
    let root = doc.project().schematic.root_sheet().unwrap();
    assert_ne!(root, r1.sheet);
    let footprints = doc.project().board.footprints.len();
    let at = Point::mm(100.0, 50.0);

    let placed = clipboard::paste(&mut doc, ViewId::Sheet(root), clip.clone(), at, None).unwrap();
    assert_eq!(placed.len(), 1 + wires);
    let p = doc.project();
    let ObjectId::Symbol(s) = placed[0] else { panic!("{placed:?}") };
    let s = p.schematic.symbols.get(s).unwrap();
    assert_eq!((s.sheet, s.pos, s.rotation), (root, at, r1.rotation));
    let c = p.schematic.components.get(s.component).unwrap();
    assert_ne!(c.refdes, "R1", "R1 still exists: the copy is renumbered");
    assert!(c.refdes.starts_with('R'));
    assert_eq!(c.value, component(&doc, "R1").value);
    assert_eq!(doc.project().board.footprints.len(), footprints + 1, "sync adds its footprint");
    assert!(doc.project().schematic.wires.iter().filter(|w| w.sheet == root).count() >= wires);

    // One undo step.
    doc.undo();
    assert_eq!(doc.project().board.footprints.len(), footprints);
    assert!(placed.iter().all(|&id| !doc.project().contains(id)));
}

#[test]
fn cut_and_paste_keeps_the_reference() {
    let mut doc = Document::new(demo_project());
    let (clip, r1, _) = copy_r1_and_wires(&doc);
    doc.execute(Command::Delete { ids: vec![ObjectId::Symbol(r1.id)] }).unwrap();
    assert!(doc.project().schematic.component_by_refdes("R1").is_none());
    let root = doc.project().schematic.root_sheet().unwrap();
    clipboard::paste(&mut doc, ViewId::Sheet(root), clip, Point::ZERO, None).unwrap();
    assert_eq!(symbol_of(&doc, "R1").sheet, root);
}

#[test]
fn clips_carry_their_parts_into_other_projects() {
    let doc = Document::new(demo_project());
    let (clip, _, _) = copy_r1_and_wires(&doc);
    let part = component(&doc, "R1").part;
    let mut other = Document::new(Project::new("Other"));
    assert!(!other.project().library.parts.contains(part));
    let root = other.project().schematic.root_sheet().unwrap();
    clipboard::paste(&mut other, ViewId::Sheet(root), clip.clone(), Point::ZERO, None).unwrap();
    assert!(other.project().library.parts.contains(part));
    assert_eq!(component(&other, "R1").part, part);
    assert_eq!(other.project().board.footprints.len(), 1);
    // Again: the part is there now, the reference is taken.
    clipboard::paste(&mut other, ViewId::Sheet(root), clip, Point::mm(20.0, 0.0), None).unwrap();
    assert!(other.project().schematic.component_by_refdes("R2").is_some());
}

#[test]
fn board_clips_keep_layers_and_stay_on_the_board() {
    let mut doc = Document::new(demo_project());
    let track = doc.project().board.tracks.iter().next().unwrap().clone();
    let (clip, _) = clipboard::copy(doc.project(), &[ObjectId::Track(track.id)], |p| p).unwrap();
    let at = track.a + Point::mm(0.0, 5.0);
    let placed = clipboard::paste(&mut doc, ViewId::Board, clip.clone(), at, None).unwrap();
    let [ObjectId::Track(t)] = placed[..] else { panic!("{placed:?}") };
    let t = doc.project().board.tracks.get(t).unwrap();
    assert_eq!((t.layer, t.width, t.a, t.b - t.a), (track.layer, track.width, at, track.b - track.a));

    let root = doc.project().schematic.root_sheet().unwrap();
    let err = clipboard::paste(&mut doc, ViewId::Sheet(root), clip, at, None).unwrap_err();
    assert!(err.contains("board objects"), "{err}");
    // Footprints aren't copied.
    let fp = doc.project().board.footprints.iter().next().unwrap().id;
    assert!(clipboard::copy(doc.project(), &[ObjectId::Footprint(fp)], |p| p).is_err());
}

#[test]
fn pads_are_pasted_with_new_numbers() {
    let p = demo_project();
    let fp = p.library.footprints.iter().find(|f| f.pads.len() >= 2).unwrap().clone();
    let n = fp.pads.len();
    let mut content = Content::Footprint(fp.clone());
    let first: Vec<Pad> = fp.pads.iter().take(2).cloned().collect();
    let elems: Vec<Elem> = first.iter().map(|p| Elem::Pad(p.id)).collect();
    let clip = copy_elems(&content, &elems, first[0].pos).unwrap();
    let added = paste_elems(&mut content, &clip, first[0].pos + Point::mm(0.0, 10.0)).unwrap();
    let Content::Footprint(after) = &content else { unreachable!() };
    assert_eq!(after.pads.len(), n + 2);
    let names: Vec<&str> = added
        .iter()
        .map(|e| match e {
            Elem::Pad(id) => after.pads.get(*id).unwrap().name.as_str(),
            _ => panic!(),
        })
        .collect();
    let max: u64 = fp.pads.iter().filter_map(|p| p.name.parse().ok()).max().unwrap();
    assert_eq!(names, [(max + 1).to_string(), (max + 2).to_string()]);
    let Elem::Pad(second) = added[1] else { panic!() };
    assert_eq!(after.pads.get(second).unwrap().pos, first[1].pos + Point::mm(0.0, 10.0));

    // A footprint clip doesn't go into a symbol.
    let sym = p.library.symbols.iter().next().unwrap().clone();
    assert!(paste_elems(&mut Content::Symbol(sym), &clip, Point::ZERO).is_err());
    assert!(matches!(clip.content, ClipContent::Footprint { .. }));
}

#[test]
fn other_text_is_not_a_clip() {
    assert!(Clip::from_text("hello").is_none());
    assert!(Clip::from_text(r#"{"type": "wire"}"#).is_none());
    assert!(Clip::from_text(r#"{"pcb_designer_clip": 99}"#).unwrap().is_err());
}

fn input(p: Point) -> PointerInput {
    PointerInput { world: Some(p), tolerance: mm(0.3), drag_threshold: mm(0.5), ..Default::default() }
}

#[test]
fn a_paste_follows_the_pointer_until_a_click() {
    let mut doc = Document::new(demo_project());
    let (clip, _, wires) = copy_r1_and_wires(&doc);
    let root = doc.project().schematic.root_sheet().unwrap();
    let v = ViewId::Sheet(root);
    let mut it = Interaction::new(mm(1.27));
    let at = Point::mm(50.8, 50.8);
    let group = interaction::new_group();
    let placed = clipboard::paste(&mut doc, v, clip.clone(), at, Some(group)).unwrap();
    it.carry(&mut doc.target(v), placed.clone(), at, at, group);
    assert!(it.is_carrying());
    let ObjectId::Symbol(s) = placed[0] else { panic!() };

    // Moves without a button, snapped; R rotates.
    it.pointer(&mut doc.target(v), &input(at + Point::mm(2.6, 0.0)));
    assert_eq!(doc.project().schematic.symbols.get(s).unwrap().pos, at + Point::mm(2.54, 0.0));
    it.key(&mut doc.target(v), KeyAction::Rotate);
    // A click drops it: one undo step for paste, moves and rotation.
    it.pointer(&mut doc.target(v), &PointerInput { pressed: true, down: true, ..input(at + Point::mm(2.6, 0.0)) });
    assert!(!it.is_carrying());
    it.pointer(&mut doc.target(v), &PointerInput { released: true, ..input(at + Point::mm(2.6, 0.0)) });
    assert_eq!(it.selection, placed);
    assert_eq!(placed.len(), 1 + wires);
    doc.undo();
    assert!(!doc.project().contains(placed[0]));

    // Esc cancels the paste.
    let group = interaction::new_group();
    let placed = clipboard::paste(&mut doc, v, clip, at, Some(group)).unwrap();
    it.carry(&mut doc.target(v), placed.clone(), at, at, group);
    it.pointer(&mut doc.target(v), &input(at + Point::mm(5.0, 0.0)));
    it.key(&mut doc.target(v), KeyAction::Escape);
    assert!(!it.is_carrying());
    assert!(!doc.project().contains(placed[0]));
}
