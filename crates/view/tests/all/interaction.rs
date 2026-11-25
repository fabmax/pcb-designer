use editor_core::Command;
use model::*;
use view::demo::demo_project;
use view::{Document, Interaction, KeyAction, PointerInput, ProjectTarget, Target, ViewId};

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
    // R1 is on the demo's Indicator sheet.
    let v = ViewId::Sheet(r1.sheet);
    let id = ObjectId::Symbol(r1.id);

    press(&mut it, &mut doc.target(v), r1.pos);
    assert_eq!(it.selection, vec![id]);
    // Several intermediate positions, final one off-grid: the result snaps.
    for x in [1.0, 2.0, 3.0, 5.3] {
        drag(&mut it, &mut doc.target(v), r1.pos + Point::mm(x, 0.0));
    }
    assert!(it.is_dragging());
    release(&mut it, &mut doc.target(v), r1.pos + Point::mm(5.3, 0.0));
    let moved = symbol(&doc, "R1");
    assert_eq!(moved.pos, r1.pos + Point::mm(5.08, 0.0));

    assert_eq!(doc.editor().history().undo_label(), Some("Move 1 object(s)"));
    doc.undo();
    assert_eq!(symbol(&doc, "R1").pos, r1.pos);
    assert!(!doc.is_modified());

    // Without snapping the drag lands exactly where the pointer went.
    it.snap = false;
    press(&mut it, &mut doc.target(v), r1.pos);
    drag(&mut it, &mut doc.target(v), r1.pos + Point::mm(5.3, 0.0));
    release(&mut it, &mut doc.target(v), r1.pos + Point::mm(5.3, 0.0));
    assert_eq!(symbol(&doc, "R1").pos, r1.pos + Point::mm(5.3, 0.0));
    assert_eq!(it.snap(Point::mm(0.123, 0.0)), Point::mm(0.123, 0.0));
}

#[test]
fn escape_cancels_drag_including_rotation() {
    let mut doc = Document::new(demo_project());
    let mut it = Interaction::new(mm(1.27));
    let r1 = symbol(&doc, "R1");
    // R1 is on the demo's Indicator sheet.
    let v = ViewId::Sheet(r1.sheet);
    press(&mut it, &mut doc.target(v), r1.pos);
    drag(&mut it, &mut doc.target(v), r1.pos + Point::mm(10.0, 0.0));
    it.key(&mut doc.target(v), KeyAction::Rotate);
    assert_ne!(symbol(&doc, "R1").rotation, r1.rotation);
    it.key(&mut doc.target(v), KeyAction::Escape);
    assert_eq!(symbol(&doc, "R1"), r1);
    assert!(!doc.is_modified());
    assert!(!it.is_dragging());
}

#[test]
fn box_select_inside_and_crossing() {
    let mut doc = Document::new(demo_project());
    let mut it = Interaction::new(mm(1.27));
    let r1 = symbol(&doc, "R1");
    // R1 is on the demo's Indicator sheet.
    let v = ViewId::Sheet(r1.sheet);
    // A box around R1 only, from empty space.
    let a = r1.pos + Point::mm(-6.5, 6.0);
    let b = r1.pos + Point::mm(6.5, -6.0);
    // Without Shift, either direction selects what the box encloses.
    let mut inside = Vec::new();
    for (from, to) in [(a, b), (b, a)] {
        press(&mut it, &mut doc.target(v), from);
        drag(&mut it, &mut doc.target(v), to);
        assert!(it.box_rect().is_some_and(|(_, crossing)| !crossing));
        release(&mut it, &mut doc.target(v), to);
        assert!(it.selection.contains(&ObjectId::Symbol(r1.id)));
        inside.push(it.selection.clone());
    }
    assert_eq!(inside[0], inside[1]);

    // A thin box across R1's middle encloses nothing; with Alt it touches R1, which shows
    // as selected while dragging.
    let (c, d) = (r1.pos + Point::mm(-0.3, 6.0), r1.pos + Point::mm(0.3, -6.0));
    press(&mut it, &mut doc.target(v), c);
    drag(&mut it, &mut doc.target(v), d);
    release(&mut it, &mut doc.target(v), d);
    assert!(!it.selection.contains(&ObjectId::Symbol(r1.id)));
    let alt = |p| PointerInput { alt: true, ..input(p) };
    press(&mut it, &mut doc.target(v), c);
    it.pointer(&mut doc.target(v), &PointerInput { down: true, ..alt(d) });
    assert!(it.box_rect().is_some_and(|(_, crossing)| crossing));
    assert!(doc.view(v).is_highlighted(ObjectId::Symbol(r1.id)), "live preview");
    assert!(it.selection.is_empty(), "the box replaces the selection");
    it.pointer(&mut doc.target(v), &PointerInput { released: true, ..alt(d) });
    assert!(it.selection.contains(&ObjectId::Symbol(r1.id)));

    // Shift adds to the selection; the preview shows both.
    let selected = it.selection.clone();
    let shift = |p| PointerInput { shift: true, ..input(p) };
    let (e, f) = (a + Point::mm(30.0, 0.0), b + Point::mm(30.0, 0.0));
    press(&mut it, &mut doc.target(v), e);
    it.pointer(&mut doc.target(v), &PointerInput { down: true, ..shift(f) });
    assert!(selected.iter().all(|id| doc.view(v).is_highlighted(*id)));
    it.pointer(&mut doc.target(v), &PointerInput { released: true, ..shift(f) });
    assert!(selected.iter().all(|id| it.selection.contains(id)));

    // Clicking empty space clears.
    let far = Point::mm(500.0, 500.0);
    press(&mut it, &mut doc.target(v), far);
    release(&mut it, &mut doc.target(v), far);
    assert!(it.selection.is_empty());
}

#[test]
fn box_select_takes_reference_and_value_only_with_their_part() {
    let mut doc = Document::new(demo_project());
    let mut it = Interaction::new(mm(1.27));
    let r1 = symbol(&doc, "R1");
    let v = ViewId::Sheet(r1.sheet);
    let fields: Vec<ObjectId> = doc
        .project()
        .schematic
        .fields
        .iter()
        .filter(|f| f.symbol == r1.id)
        .map(|f| ObjectId::SymbolField(f.id))
        .collect();
    assert!(!fields.is_empty());
    let alt = |p| PointerInput { alt: true, ..input(p) };
    let alt_box = |it: &mut Interaction<ObjectId>, doc: &mut Document, a: Point, b: Point| {
        press(it, &mut doc.target(v), a);
        it.pointer(&mut doc.target(v), &PointerInput { down: true, ..alt(b) });
        it.pointer(&mut doc.target(v), &PointerInput { released: true, ..alt(b) });
    };

    // A box enclosing just one field selects nothing.
    let field = doc.view(v).index.bounds(fields[0]).unwrap();
    // From a corner of the box in empty space, to the opposite one.
    let (lo, hi) = (field.min - Point::mm(0.5, 0.5), field.max + Point::mm(0.5, 0.5));
    let corners = [
        (lo, hi),
        (hi, lo),
        (Point::new(lo.x, hi.y), Point::new(hi.x, lo.y)),
        (Point::new(hi.x, lo.y), Point::new(lo.x, hi.y)),
    ];
    let (a, b) = corners.into_iter().find(|&(a, _)| doc.target(v).hit(a, mm(0.3)).is_none()).unwrap();
    press(&mut it, &mut doc.target(v), a);
    drag(&mut it, &mut doc.target(v), b);
    release(&mut it, &mut doc.target(v), b);
    assert!(fields.iter().all(|f| !it.selection.contains(f)));
    // Touching may catch the symbol's body next to it, but never the field alone.
    alt_box(&mut it, &mut doc, a, b);
    let with_symbol = it.selection.contains(&ObjectId::Symbol(r1.id));
    assert!(fields.iter().all(|f| it.selection.contains(f) == with_symbol));

    // A thin box touching the symbol's body brings all its fields along.
    let body = doc.view(v).index.bounds(ObjectId::Symbol(r1.id)).unwrap();
    let c = Point::new((body.min.x + body.max.x) / 2, (body.min.y + body.max.y) / 2);
    alt_box(&mut it, &mut doc, c + Point::mm(-0.2, 8.0), c + Point::mm(0.2, -8.0));
    assert!(it.selection.contains(&ObjectId::Symbol(r1.id)));
    assert!(fields.iter().all(|f| it.selection.contains(f)));

    // On the board, the same for a footprint's texts.
    let fp = doc.project().board.footprints.iter().next().unwrap().clone();
    let texts: Vec<ObjectId> = doc
        .project()
        .board
        .fields
        .iter()
        .filter(|f| f.footprint == fp.id && !f.hidden)
        .map(|f| ObjectId::FootprintField(f.id))
        .collect();
    assert!(!texts.is_empty());
    let board = doc.view(ViewId::Board);
    let text = board.index.bounds(texts[0]).unwrap();
    assert!(!board.select_rect(doc.project(), text, false).iter().any(|id| texts.contains(id)));
    let whole = board.index.bounds(ObjectId::Footprint(fp.id)).unwrap();
    let found = board.select_rect(doc.project(), whole, false);
    assert!(texts.iter().all(|t| found.contains(t)));
}

#[test]
fn zones_drag_by_their_outline_and_box_from_inside() {
    let mut doc = Document::new(demo_project());
    let mut it = Interaction::new(mm(0.5));
    let v = ViewId::Board;
    let zone = doc.project().board.zones.iter().next().expect("the demo has a zone").clone();
    let id = ObjectId::Zone(zone.id);
    let outline = |doc: &Document| doc.project().board.zones.get(zone.id).unwrap().outline.clone();
    // A point inside where nothing but the zone is (a 0.5 mm search grid).
    let b = doc.view(v).index.bounds(id).unwrap();
    let inside = (0..)
        .map(|i| Point::new(b.min.x + mm(0.5) * (i % 200), b.min.y + mm(0.5) * (i / 200)))
        .take(200 * 200)
        .find(|&p| doc.target(v).hit(p, mm(0.3)) == Some(id) && !doc.target(v).grips(id, p, mm(0.3)))
        .expect("an empty spot in the zone");
    let away = inside + Point::mm(1.0, 0.6);

    // From inside, a drag draws a box and leaves the zone alone, selected or not.
    for selected in [false, true] {
        it.set_selection(&mut doc.target(v), if selected { vec![id] } else { Vec::new() });
        press(&mut it, &mut doc.target(v), inside);
        drag(&mut it, &mut doc.target(v), away);
        assert!(it.box_rect().is_some(), "boxing (zone selected: {selected})");
        release(&mut it, &mut doc.target(v), away);
        assert_eq!(outline(&doc), zone.outline);
        assert!(!it.selection.contains(&id), "a box inside the zone doesn't take it");
    }

    // A click inside selects it.
    press(&mut it, &mut doc.target(v), inside);
    release(&mut it, &mut doc.target(v), inside);
    assert_eq!(it.selection, vec![id]);

    // By its outline, it drags.
    let (a, c) = (zone.outline[0], zone.outline[1]);
    let edge = Point::new((a.x + c.x) / 2, (a.y + c.y) / 2);
    assert!(doc.target(v).grips(id, edge, mm(0.3)));
    it.set_selection(&mut doc.target(v), Vec::new());
    press(&mut it, &mut doc.target(v), edge);
    drag(&mut it, &mut doc.target(v), edge + Point::mm(2.0, 0.0));
    release(&mut it, &mut doc.target(v), edge + Point::mm(2.0, 0.0));
    assert_eq!(outline(&doc)[0], zone.outline[0] + Point::mm(2.0, 0.0));
    doc.undo();
    assert_eq!(outline(&doc), zone.outline);

    // Selected with something else, it moves along from inside too.
    let via = doc.project().board.vias.iter().next().expect("the demo has a via").clone();
    it.set_selection(&mut doc.target(v), vec![id, ObjectId::Via(via.id)]);
    press(&mut it, &mut doc.target(v), inside);
    drag(&mut it, &mut doc.target(v), inside + Point::mm(1.0, 0.0));
    release(&mut it, &mut doc.target(v), inside + Point::mm(1.0, 0.0));
    assert_eq!(outline(&doc)[0], zone.outline[0] + Point::mm(1.0, 0.0));
    assert_eq!(doc.project().board.vias.get(via.id).unwrap().pos, via.pos + Point::mm(1.0, 0.0));
    doc.undo();

    // Touching (Alt) takes it where the box crosses its outline, not inside it.
    let t = doc.target(v);
    let small = Rect::from_points(inside, away);
    assert!(!t.in_rect(small, true).contains(&id));
    assert!(t.in_rect(Rect::around(edge, mm(0.2)), true).contains(&id));
}

#[test]
fn unfilled_drawings_are_hit_on_their_line_only() {
    // A polygon board outline (as KiCad imports and the amplifier have) around everything
    // else, and an arc.
    let mut doc = Document::new(Project::new("T"));
    let v = ViewId::Board;
    let corners = vec![Point::mm(0.0, 0.0), Point::mm(50.0, 0.0), Point::mm(50.0, 40.0), Point::mm(0.0, 40.0)];
    let outline = BoardGraphic {
        id: Id::new(),
        layer: LayerId::OUTLINE,
        shape: Shape::Polygon { points: corners, width: mm(0.1), fill: false },
    };
    let arc = BoardGraphic {
        id: Id::new(),
        layer: LayerId::OUTLINE,
        shape: Shape::Arc {
            center: Point::mm(25.0, 20.0),
            radius: mm(5.0),
            start: Angle::ZERO,
            sweep: 90.0,
            width: mm(0.1),
        },
    };
    let (o, a) = (ObjectId::BoardGraphic(outline.id), ObjectId::BoardGraphic(arc.id));
    doc.execute(Command::Insert { objects: vec![Object::BoardGraphic(outline.clone()), Object::BoardGraphic(arc)] })
        .unwrap();
    let tol = mm(0.3);

    // Inside the outline, away from its line: nothing to grab, so a drag boxes.
    let inside = Point::mm(10.0, 10.0);
    assert_eq!(doc.target(v).hit(inside, tol), None);
    assert_eq!(doc.target(v).hit(Point::mm(50.1, 20.0), tol), Some(o));
    let mut it = Interaction::new(mm(0.5));
    press(&mut it, &mut doc.target(v), inside);
    drag(&mut it, &mut doc.target(v), inside + Point::mm(5.0, 5.0));
    assert!(it.box_rect().is_some());
    release(&mut it, &mut doc.target(v), inside + Point::mm(5.0, 5.0));
    assert_eq!(doc.project().board.graphics.get(outline.id).unwrap(), &outline);

    // The arc: on it, not on the rest of its circle.
    assert_eq!(doc.target(v).hit(Point::mm(25.0, 25.0), tol), Some(a), "at 90°");
    assert_eq!(doc.target(v).hit(Point::mm(25.0, 15.0), tol), None, "at 270°: off the arc");

    // A touching box takes them only where it meets their line.
    let t = doc.target(v);
    let small = Rect::from_points(Point::mm(5.0, 5.0), Point::mm(15.0, 15.0));
    assert!(t.in_rect(small, true).is_empty());
    assert_eq!(t.in_rect(Rect::from_points(Point::mm(45.0, 5.0), Point::mm(55.0, 10.0)), true), vec![o]);
    assert_eq!(t.in_rect(Rect::from_points(Point::mm(24.0, 24.0), Point::mm(26.0, 26.0)), true), vec![a]);
}

#[test]
fn locked_footprints_do_not_move_and_delete_explains() {
    let mut doc = Document::new(demo_project());
    let mut it = Interaction::new(mm(0.5));
    let fp = doc.project().board.footprints.iter().next().unwrap().clone();
    let mut locked = fp.clone();
    locked.locked = true;
    doc.execute(editor_core::Command::Update { objects: vec![Object::Footprint(locked)] }).unwrap();

    it.set_selection(&mut doc.target(ViewId::Board), vec![ObjectId::Footprint(fp.id)]);
    press(&mut it, &mut doc.target(ViewId::Board), fp.pos);
    drag(&mut it, &mut doc.target(ViewId::Board), fp.pos + Point::mm(5.0, 0.0));
    release(&mut it, &mut doc.target(ViewId::Board), fp.pos + Point::mm(5.0, 0.0));
    assert_eq!(doc.project().board.footprints.get(fp.id).unwrap().pos, fp.pos);
    assert!(it.message.is_some());

    it.set_selection(&mut doc.target(ViewId::Board), vec![ObjectId::Footprint(fp.id)]);
    it.key(&mut doc.target(ViewId::Board), KeyAction::Delete);
    assert!(doc.project().board.footprints.contains(fp.id));
    assert!(it.message.as_deref().unwrap().contains("schematic"));
}

#[test]
fn incremental_views_match_rebuild() {
    let mut doc = Document::new(demo_project());
    let mut it = Interaction::new(mm(1.27));
    it.key(&mut doc.target(doc.root()), KeyAction::SelectAll);
    it.key(&mut doc.target(doc.root()), KeyAction::Rotate);
    it.key(&mut doc.target(doc.root()), KeyAction::Mirror);
    it.set_selection(&mut doc.target(doc.root()), vec![]);
    let r1 = symbol(&doc, "R1");
    let v = ViewId::Sheet(r1.sheet);
    it.set_selection(&mut doc.target(v), vec![ObjectId::Symbol(r1.id)]);
    it.key(&mut doc.target(v), KeyAction::Delete);
    doc.undo();
    it.set_selection(&mut doc.target(v), vec![]);

    doc.update_connectivity(false);
    let views: Vec<ViewId> = doc.sheet_views().map(|v| v.id).chain([ViewId::Board]).collect();
    for kind in views {
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
    let view = doc.view(ViewId::Sheet(w.sheet));
    assert_eq!(view.hit(doc.project(), mid, mm(0.2)), Some(ObjectId::Wire(w.id)));
    // Other sheets don't have it.
    for other in doc.sheet_views().filter(|v| v.id != ViewId::Sheet(w.sheet)) {
        assert_ne!(other.hit(doc.project(), mid, mm(0.2)), Some(ObjectId::Wire(w.id)));
    }
}

#[test]
fn save_and_reopen_document() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("demo");
    let mut doc = Document::new(demo_project());
    assert!(doc.save().is_err(), "no path yet");
    let mut it = Interaction::new(mm(1.27));
    it.key(&mut doc.target(doc.root()), KeyAction::SelectAll);
    it.key(&mut doc.target(doc.root()), KeyAction::Rotate);
    assert!(doc.is_modified());
    doc.save_as(&path).unwrap();
    assert!(!doc.is_modified());
    let reopened = Document::open(&path).unwrap();
    assert_eq!(reopened.project(), doc.project());
    let mut fresh = view::View::new(doc.root(), doc.project());
    fresh.set_connectivity(reopened.connectivity());
    assert_eq!(reopened.schematic().scene.instance_count(), fresh.scene.instance_count());
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
        let views: Vec<ViewId> = doc.sheet_views().map(|v| v.id).chain([ViewId::Board]).collect();
        for kind in views {
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
    let before = doc.view(ViewId::Sheet(r1.sheet)).index.bounds(ObjectId::Symbol(r1.id)).unwrap();
    sym.graphics.push(Shape::Line { a: Point::mm(0.0, 0.0), b: Point::mm(20.0, 0.0), width: mm(0.25) });
    doc.execute(Command::Update { objects: vec![Object::LibSymbol(sym)] }).unwrap();
    assert!(!doc.update_connectivity(false), "graphics don't change connectivity");
    assert_ne!(doc.view(ViewId::Sheet(r1.sheet)).index.bounds(ObjectId::Symbol(r1.id)).unwrap(), before);
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
    let users = doc.view(ViewId::Sheet(r1.sheet)).users_of(doc.project(), &[ObjectId::LibFootprint(fp_id)]);
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
    let bounds = |doc: &Document| doc.view(ViewId::Sheet(r1.sheet)).index.bounds(ObjectId::Symbol(r1.id)).unwrap();
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
    let fresh = view::View::new(ViewId::Sheet(r1.sheet), doc.project());
    assert_eq!(fresh.index.bounds(ObjectId::Symbol(r1.id)), Some(bounds(&doc)));
}

/// Pointer input with a screen position, at 10 points per mm.
fn on_screen(world: Point, screen: [f64; 2]) -> PointerInput {
    PointerInput { screen: Some(screen), nm_per_point: 100_000.0, ..input(world) }
}

#[test]
fn a_click_never_moves_when_the_view_shifts() {
    let mut doc = Document::new(demo_project());
    let mut it = Interaction::new(mm(1.27));
    let r1 = symbol(&doc, "R1");
    let v = ViewId::Sheet(r1.sheet);
    let at = [500.0, 300.0];

    // Pressed, then the canvas shifts 6 mm (a panel got wider) under the still pointer.
    it.pointer(&mut doc.target(v), &PointerInput { pressed: true, down: true, ..on_screen(r1.pos, at) });
    let shifted = r1.pos + Point::mm(6.0, 0.0);
    it.pointer(&mut doc.target(v), &PointerInput { down: true, ..on_screen(shifted, at) });
    assert!(!it.is_dragging(), "the hand didn't move");
    it.pointer(&mut doc.target(v), &PointerInput { released: true, ..on_screen(shifted, at) });
    assert_eq!(symbol(&doc, "R1").pos, r1.pos);
    assert!(!doc.is_modified());
    assert_eq!(it.selection, vec![ObjectId::Symbol(r1.id)]);

    // A real drag after such a shift moves by the hand's motion only (5.08 mm right).
    it.pointer(&mut doc.target(v), &PointerInput { pressed: true, down: true, ..on_screen(r1.pos, at) });
    it.pointer(&mut doc.target(v), &PointerInput { down: true, ..on_screen(shifted, at) });
    let hand = [at[0] + 50.8, at[1]];
    let pointer = shifted + Point::mm(5.08, 0.0);
    it.pointer(&mut doc.target(v), &PointerInput { down: true, ..on_screen(pointer, hand) });
    assert!(it.is_dragging());
    it.pointer(&mut doc.target(v), &PointerInput { released: true, ..on_screen(pointer, hand) });
    assert_eq!(symbol(&doc, "R1").pos, r1.pos + Point::mm(5.08, 0.0));
}

/// A wall along y = 0 and a dog-leg whose middle segment, along y = 3, is dragged.
fn shove_board(locked: bool) -> (Document, Vec<Track>, Id) {
    let mut doc = Document::new(Project::new("T"));
    let mut wall = Track::straight(LayerId::TOP_COPPER, Point::mm(0.0, 0.0), Point::mm(20.0, 0.0), mm(0.25));
    wall.locked = locked;
    let corners = [(0.0, 7.0), (4.0, 3.0), (16.0, 3.0), (20.0, 7.0)].map(|(x, y)| Point::mm(x, y));
    let leg: Vec<Track> =
        corners.windows(2).map(|w| Track::straight(LayerId::TOP_COPPER, w[0], w[1], mm(0.25))).collect();
    let middle = leg[1].id;
    let objects = std::iter::once(&wall).chain(&leg).cloned().map(Object::Track).collect();
    doc.execute(Command::Insert { objects }).unwrap();
    doc.drag_shove = true;
    let before = doc.project().board.tracks.iter().cloned().collect();
    (doc, before, middle)
}

fn copper_clashes(doc: &mut Document) -> Vec<String> {
    doc.run_drc()
        .violations
        .iter()
        .filter(|v| matches!(v.kind, ViolationKind::Clearance | ViolationKind::Short))
        .map(|v| v.message.clone())
        .collect()
}

#[test]
fn dragging_a_segment_shoves_and_is_one_undo_step() {
    let (mut doc, before, middle) = shove_board(false);
    let mut it = Interaction::new(mm(0.5));
    let t = &mut doc.target(ViewId::Board);
    press(&mut it, t, Point::mm(10.0, 3.0));
    for y in [2.5, 1.5, 0.0, 1.0, 0.0] {
        drag(&mut it, t, Point::mm(10.0, y));
    }
    release(&mut it, t, Point::mm(10.0, 0.0));
    assert_eq!(doc.project().board.tracks.get(middle).unwrap().a.y, 0);
    let wall = before[0].id;
    assert_ne!(doc.project().board.tracks.get(wall), Some(&before[0]), "the wall goes round");
    assert!(copper_clashes(&mut doc).is_empty());
    doc.undo();
    assert_eq!(doc.project().board.tracks.iter().cloned().collect::<Vec<_>>(), before);
}

#[test]
fn a_shove_that_cannot_push_stops_the_drag() {
    let (mut doc, _, middle) = shove_board(true);
    let mut it = Interaction::new(mm(0.5));
    let t = &mut doc.target(ViewId::Board);
    press(&mut it, t, Point::mm(10.0, 3.0));
    for y in [2.5, 2.0, 1.5, 1.0, 0.5, 0.0] {
        drag(&mut it, t, Point::mm(10.0, y));
    }
    release(&mut it, t, Point::mm(10.0, 0.0));
    let y = doc.project().board.tracks.get(middle).unwrap().a.y;
    assert_eq!(y, mm(0.5), "stopped at the last step that worked");
    assert!(it.message.as_deref().unwrap_or("").contains("push"), "{:?}", it.message);
    assert!(copper_clashes(&mut doc).is_empty());
}
