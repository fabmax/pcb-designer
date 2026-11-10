use editor_core::Command;
use model::*;
use view::demo::demo_project;
use view::{Document, Interaction, KeyAction, PointerInput, ProjectTarget, ViewKind};

fn input(p: Point) -> PointerInput {
    PointerInput { world: Some(p), tolerance: mm(0.3), drag_threshold: mm(0.5), ..Default::default() }
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

fn symbol(doc: &Document, refdes: &str) -> SymbolInstance {
    let c = doc.project().schematic.component_by_refdes(refdes).unwrap().id;
    doc.project().schematic.symbols_of(c).next().unwrap().clone()
}

#[test]
fn click_selects_and_drag_is_one_undo_step() {
    let mut doc = Document::new(demo_project());
    let mut it = Interaction::new(mm(1.27));
    let r1 = symbol(&doc, "R1");
    let id = ObjectId::Symbol(r1.id);

    press(&mut it, &mut doc.target(ViewKind::Schematic), r1.pos);
    assert_eq!(it.selection, vec![id]);
    // Several intermediate positions, final one off-grid: the result snaps.
    for x in [1.0, 2.0, 3.0, 5.3] {
        drag(&mut it, &mut doc.target(ViewKind::Schematic), r1.pos + Point::mm(x, 0.0));
    }
    assert!(it.is_dragging());
    release(&mut it, &mut doc.target(ViewKind::Schematic), r1.pos + Point::mm(5.3, 0.0));
    let moved = symbol(&doc, "R1");
    assert_eq!(moved.pos, r1.pos + Point::mm(5.08, 0.0));

    assert_eq!(doc.editor().history().undo_label(), Some("Move 1 object(s)"));
    doc.undo();
    assert_eq!(symbol(&doc, "R1").pos, r1.pos);
    assert!(!doc.is_modified());

    // Without snapping the drag lands exactly where the pointer went.
    it.snap = false;
    press(&mut it, &mut doc.target(ViewKind::Schematic), r1.pos);
    drag(&mut it, &mut doc.target(ViewKind::Schematic), r1.pos + Point::mm(5.3, 0.0));
    release(&mut it, &mut doc.target(ViewKind::Schematic), r1.pos + Point::mm(5.3, 0.0));
    assert_eq!(symbol(&doc, "R1").pos, r1.pos + Point::mm(5.3, 0.0));
    assert_eq!(it.snap(Point::mm(0.123, 0.0)), Point::mm(0.123, 0.0));
}

#[test]
fn escape_cancels_drag_including_rotation() {
    let mut doc = Document::new(demo_project());
    let mut it = Interaction::new(mm(1.27));
    let r1 = symbol(&doc, "R1");
    press(&mut it, &mut doc.target(ViewKind::Schematic), r1.pos);
    drag(&mut it, &mut doc.target(ViewKind::Schematic), r1.pos + Point::mm(10.0, 0.0));
    it.key(&mut doc.target(ViewKind::Schematic), KeyAction::Rotate);
    assert_ne!(symbol(&doc, "R1").rotation, r1.rotation);
    it.key(&mut doc.target(ViewKind::Schematic), KeyAction::Escape);
    assert_eq!(symbol(&doc, "R1"), r1);
    assert!(!doc.is_modified());
    assert!(!it.is_dragging());
}

#[test]
fn box_select_inside_and_crossing() {
    let mut doc = Document::new(demo_project());
    let mut it = Interaction::new(mm(1.27));
    let r1 = symbol(&doc, "R1");
    // Empty area far away from everything, dragging left to right around R1 only.
    let a = r1.pos + Point::mm(-6.5, 6.0);
    let b = r1.pos + Point::mm(6.5, -6.0);
    press(&mut it, &mut doc.target(ViewKind::Schematic), a);
    drag(&mut it, &mut doc.target(ViewKind::Schematic), b);
    assert!(it.box_rect().is_some_and(|(_, crossing)| !crossing));
    release(&mut it, &mut doc.target(ViewKind::Schematic), b);
    assert!(it.selection.contains(&ObjectId::Symbol(r1.id)));
    let window = it.selection.len();

    // Crossing (right to left) selects at least as much.
    press(&mut it, &mut doc.target(ViewKind::Schematic), b);
    drag(&mut it, &mut doc.target(ViewKind::Schematic), a);
    release(&mut it, &mut doc.target(ViewKind::Schematic), a);
    assert!(it.selection.len() >= window);

    // Clicking empty space clears.
    let far = Point::mm(500.0, 500.0);
    press(&mut it, &mut doc.target(ViewKind::Schematic), far);
    release(&mut it, &mut doc.target(ViewKind::Schematic), far);
    assert!(it.selection.is_empty());
}

#[test]
fn locked_footprints_do_not_move_and_delete_explains() {
    let mut doc = Document::new(demo_project());
    let mut it = Interaction::new(mm(0.5));
    let fp = doc.project().board.footprints.iter().next().unwrap().clone();
    let mut locked = fp.clone();
    locked.locked = true;
    doc.execute(editor_core::Command::Update { objects: vec![Object::Footprint(locked)] }).unwrap();

    it.set_selection(&mut doc.target(ViewKind::Board), vec![ObjectId::Footprint(fp.id)]);
    press(&mut it, &mut doc.target(ViewKind::Board), fp.pos);
    drag(&mut it, &mut doc.target(ViewKind::Board), fp.pos + Point::mm(5.0, 0.0));
    release(&mut it, &mut doc.target(ViewKind::Board), fp.pos + Point::mm(5.0, 0.0));
    assert_eq!(doc.project().board.footprints.get(fp.id).unwrap().pos, fp.pos);
    assert!(it.message.is_some());

    it.set_selection(&mut doc.target(ViewKind::Board), vec![ObjectId::Footprint(fp.id)]);
    it.key(&mut doc.target(ViewKind::Board), KeyAction::Delete);
    assert!(doc.project().board.footprints.contains(fp.id));
    assert!(it.message.as_deref().unwrap().contains("schematic"));
}

#[test]
fn incremental_views_match_rebuild() {
    let mut doc = Document::new(demo_project());
    let mut it = Interaction::new(mm(1.27));
    it.key(&mut doc.target(ViewKind::Schematic), KeyAction::SelectAll);
    it.key(&mut doc.target(ViewKind::Schematic), KeyAction::Rotate);
    it.key(&mut doc.target(ViewKind::Schematic), KeyAction::Mirror);
    let r1 = symbol(&doc, "R1");
    it.set_selection(&mut doc.target(ViewKind::Schematic), vec![ObjectId::Symbol(r1.id)]);
    it.key(&mut doc.target(ViewKind::Schematic), KeyAction::Delete);
    doc.undo();
    it.set_selection(&mut doc.target(ViewKind::Schematic), vec![]);

    doc.update_connectivity(false);
    for kind in [ViewKind::Schematic, ViewKind::Board] {
        let mut fresh = view::View::new(kind, doc.project());
        fresh.set_connectivity(doc.connectivity());
        fresh.set_copper(doc.project(), doc.copper());
        let live = doc.view(kind);
        assert_eq!(live.scene.instance_count(), fresh.scene.instance_count(), "{kind:?}");
        assert_eq!(live.index.len(), fresh.index.len(), "{kind:?}");
        assert_eq!(live.bounds(), fresh.bounds(), "{kind:?}");
    }
}

#[test]
fn hit_prefers_wires_over_symbol_bodies() {
    let doc = Document::new(demo_project());
    let w = doc.project().schematic.wires.iter().next().unwrap().clone();
    let mid = Point::new((w.a.x + w.b.x) / 2, (w.a.y + w.b.y) / 2);
    assert_eq!(doc.schematic.hit(doc.project(), mid, mm(0.2)), Some(ObjectId::Wire(w.id)));
}

#[test]
fn save_and_reopen_document() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("demo.pcbproj");
    let mut doc = Document::new(demo_project());
    assert!(doc.save().is_err(), "no path yet");
    let mut it = Interaction::new(mm(1.27));
    it.key(&mut doc.target(ViewKind::Schematic), KeyAction::SelectAll);
    it.key(&mut doc.target(ViewKind::Schematic), KeyAction::Rotate);
    assert!(doc.is_modified());
    doc.save_as(&path).unwrap();
    assert!(!doc.is_modified());
    let reopened = Document::open(&path).unwrap();
    assert_eq!(reopened.project(), doc.project());
    let mut fresh = view::View::new(ViewKind::Schematic, doc.project());
    fresh.set_connectivity(reopened.connectivity());
    assert_eq!(reopened.schematic.scene.instance_count(), fresh.scene.instance_count());
}

/// Library edits redraw only the symbols and footprints drawn from them, and must leave
/// the views as a full rebuild would. Graphics don't change connectivity; pads do.
#[test]
fn library_edits_redraw_their_users() {
    let mut doc = Document::new(demo_project());
    doc.update_connectivity(false);
    let r1 = symbol(&doc, "R1");
    let project = doc.project();
    let c = project.schematic.components.get(r1.component).unwrap();
    let part = project.library.parts.get(c.part).unwrap();
    let mut sym = project.library.symbols.get(part.units[r1.unit as usize].symbol).unwrap().clone();
    let mut fp = project.library.footprint_of(c.part, c.variant).unwrap().clone();
    let fp_inst = project.board.footprints.iter().find(|f| f.component == r1.component).unwrap().id;
    let (c_part, c_variant) = (c.part, c.variant);

    let matches_rebuild = |doc: &mut Document, step: &str| {
        doc.update_connectivity(false);
        for kind in [ViewKind::Schematic, ViewKind::Board] {
            let mut fresh = view::View::new(kind, doc.project());
            fresh.set_connectivity(doc.connectivity());
            fresh.set_copper(doc.project(), doc.copper());
            let live = doc.view(kind);
            assert_eq!(live.scene.instance_count(), fresh.scene.instance_count(), "{step} {kind:?}");
            assert_eq!(live.bounds(), fresh.bounds(), "{kind:?}");
            for id in [ObjectId::Symbol(r1.id), ObjectId::Footprint(fp_inst)] {
                assert_eq!(live.index.bounds(id), fresh.index.bounds(id), "{kind:?} {id:?}");
            }
        }
    };

    // A line in the symbol: every resistor grows, nets stay as they are.
    let before = doc.schematic.index.bounds(ObjectId::Symbol(r1.id)).unwrap();
    sym.graphics.push(Shape::Line { a: Point::mm(0.0, 0.0), b: Point::mm(20.0, 0.0), width: mm(0.25) });
    doc.execute(Command::Update { objects: vec![Object::LibSymbol(sym)] }).unwrap();
    assert!(!doc.update_connectivity(false), "graphics don't change connectivity");
    assert_ne!(doc.schematic.index.bounds(ObjectId::Symbol(r1.id)).unwrap(), before);
    matches_rebuild(&mut doc, "symbol line");

    // Moved pads: the footprints change, and so does the copper.
    let ids: Vec<Id> = fp.pads.ids().collect();
    for id in ids {
        fp.pads.get_mut(id).unwrap().pos.x += mm(3.0);
    }
    doc.execute(Command::Update { objects: vec![Object::LibFootprint(fp)] }).unwrap();
    assert!(doc.update_connectivity(false), "pads change connectivity");
    matches_rebuild(&mut doc, "pads moved");

    doc.undo();
    doc.undo();
    matches_rebuild(&mut doc, "undone");

    // Symbols show pad names, so a footprint redraws its symbols too.
    let fp_id = doc.project().library.footprint_of(c_part, c_variant).unwrap().id;
    let users = doc.schematic.users_of(doc.project(), &[ObjectId::LibFootprint(fp_id)]);
    assert!(users.contains(&ObjectId::Symbol(r1.id)));
    assert!(
        doc.board.users_of(doc.project(), &[ObjectId::LibFootprint(fp_id)]).contains(&ObjectId::Footprint(fp_inst))
    );
    let sym_id = doc.project().library.parts.get(c_part).unwrap().units[0].symbol;
    assert!(doc.board.users_of(doc.project(), &[ObjectId::LibSymbol(sym_id)]).is_empty());
}

/// While dragging, slow library redraws wait for the drag to end.
#[test]
fn slow_library_redraws_wait_for_the_drag() {
    let mut doc = Document::new(demo_project());
    doc.slow = std::time::Duration::ZERO;
    let r1 = symbol(&doc, "R1");
    let c = doc.project().schematic.components.get(r1.component).unwrap();
    let part = doc.project().library.parts.get(c.part).unwrap();
    let mut sym = doc.project().library.symbols.get(part.units[r1.unit as usize].symbol).unwrap().clone();
    let bounds = |doc: &Document| doc.schematic.index.bounds(ObjectId::Symbol(r1.id)).unwrap();
    let mut grow = |doc: &mut Document, x: f64| {
        sym.graphics.push(Shape::Line { a: Point::ZERO, b: Point::mm(x, 0.0), width: mm(0.25) });
        doc.execute_grouped(Command::Update { objects: vec![Object::LibSymbol(sym.clone())] }, Some(1)).unwrap();
    };

    doc.update_connectivity(true);
    let before = bounds(&doc);
    grow(&mut doc, 20.0);
    let first = bounds(&doc);
    assert_ne!(first, before, "the first step is drawn (and timed)");
    grow(&mut doc, 40.0);
    assert!(!doc.update_connectivity(true), "still dragging");
    assert_eq!(bounds(&doc), first, "waits for the drag");
    assert!(doc.update_connectivity(false));
    assert_ne!(bounds(&doc), first, "caught up");
    let fresh = view::View::new(ViewKind::Schematic, doc.project());
    assert_eq!(fresh.index.bounds(ObjectId::Symbol(r1.id)), Some(bounds(&doc)));
}
