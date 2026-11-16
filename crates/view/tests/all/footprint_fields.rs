//! Reference and value texts of placed footprints: created with the footprint, selected
//! and dragged on their own, carried along by the footprint.

use editor_core::Command;
use model::*;
use view::demo::demo_project;
use view::{Document, Interaction, KeyAction, PointerInput, ViewId};

fn input(p: Point) -> PointerInput {
    PointerInput { world: Some(p), tolerance: mm(0.1), drag_threshold: mm(0.2), ..Default::default() }
}

/// R1's reference text and where it is on the board.
fn r1_reference(doc: &Document) -> (Id, Point) {
    let p = doc.project();
    let r1 = p.schematic.component_by_refdes("R1").unwrap().id;
    let fp = p.board.footprint_of(r1).unwrap();
    let field =
        p.board.fields.iter().find(|f| f.footprint == fp.id && f.text.text == "${REF}" && !f.back).expect("reference");
    (field.id, fp.transform().point(field.text.pos))
}

#[test]
fn every_placed_footprint_has_its_texts() {
    let doc = Document::new(demo_project());
    let p = doc.project();
    for f in p.board.footprints.iter() {
        let def = editor_core::query::footprint_of_instance(p, f).unwrap();
        let templates = def.texts.iter().filter(|t| t.is_field()).count();
        assert!(templates > 0);
        assert_eq!(p.board.fields.iter().filter(|x| x.footprint == f.id).count(), templates);
    }
    assert_eq!(p.validate(), Vec::<String>::new());
}

#[test]
fn drag_a_reference_then_move_its_footprint() {
    let mut doc = Document::new(demo_project());
    let v = ViewId::Board;
    let (field, at) = r1_reference(&doc);
    assert_eq!(doc.view(v).hit(doc.project(), at, mm(0.1)), Some(ObjectId::FootprintField(field)), "text wins");

    // Drag the text 2 mm to the right: only the text moves.
    let mut it = Interaction::new(mm(0.1));
    let fp_before = doc.project().board.fields.get(field).unwrap().footprint;
    let fp_pos = doc.project().board.footprints.get(fp_before).unwrap().pos;
    let to = at + Point::mm(2.0, 0.0);
    it.pointer(&mut doc.target(v), &PointerInput { pressed: true, down: true, ..input(at) });
    it.pointer(&mut doc.target(v), &PointerInput { down: true, ..input(to) });
    it.pointer(&mut doc.target(v), &PointerInput { released: true, ..input(to) });
    assert_eq!(it.selection, [ObjectId::FootprintField(field)]);
    // It snaps to the grid.
    let moved = r1_reference(&doc).1;
    assert!((moved - to).x.abs() <= mm(0.1) && (moved - to).y.abs() <= mm(0.1), "{moved:?}");
    let to = moved;
    assert_eq!(doc.project().board.footprints.get(fp_before).unwrap().pos, fp_pos);

    // R turns it in place; Delete leaves it (texts are hidden, not deleted).
    it.key(&mut doc.target(v), KeyAction::Rotate);
    assert_eq!(doc.project().board.fields.get(field).unwrap().text.rotation.degrees().rem_euclid(360.0), 90.0);
    assert_eq!(r1_reference(&doc).1, to, "turned in place");
    it.key(&mut doc.target(v), KeyAction::Delete);
    assert!(doc.project().board.fields.contains(field));

    // Moving and rotating the footprint carries the text along.
    let delta = Point::mm(5.0, 3.0);
    doc.execute(Command::Move { ids: vec![ObjectId::Footprint(fp_before)], delta, keep_connected: false }).unwrap();
    assert_eq!(r1_reference(&doc).1, to + delta);
    let center = doc.project().board.footprints.get(fp_before).unwrap().pos;
    doc.execute(Command::Rotate {
        ids: vec![ObjectId::Footprint(fp_before), ObjectId::FootprintField(field)],
        center,
        angle: Angle::DEG90,
        keep_connected: false,
    })
    .unwrap();
    let expected = Transform::new(Point::ZERO, Angle::DEG90, false).point(to + delta - center) + center;
    let got = r1_reference(&doc).1;
    assert!((got.x - expected.x).abs() <= 1 && (got.y - expected.y).abs() <= 1, "{got:?} vs {expected:?}");
    // Flipping the footprint takes the text to the bottom silkscreen.
    doc.execute(Command::FlipSide {
        ids: vec![ObjectId::Footprint(fp_before), ObjectId::FootprintField(field)],
        axis_x: center.x,
        keep_connected: false,
    })
    .unwrap();
    let p = doc.project();
    let fp = p.board.footprints.get(fp_before).unwrap();
    assert_eq!(p.board.fields.get(field).unwrap().board_layer(fp.side), LayerId::BOTTOM_SILK);
}

#[test]
fn board_parts_are_deleted_with_their_texts() {
    let mut doc = Document::new(demo_project());
    doc.execute(Command::import_parts(&builtin::builtin_library(), &[builtin::PART_MOUNTING_HOLE]).unwrap()).unwrap();
    let changes = doc
        .execute(Command::PlaceBoardPart {
            part: builtin::PART_MOUNTING_HOLE,
            variant: None,
            pos: Point::mm(3.0, 3.0),
            rotation: Angle::ZERO,
            side: Side::Top,
            refdes: None,
            value: None,
        })
        .unwrap();
    let f = changes.ids().find_map(|id| if let ObjectId::Footprint(f) = id { Some(f) } else { None }).unwrap();
    assert!(doc.project().board.fields.iter().any(|x| x.footprint == f));
    let v = ViewId::Board;
    let mut it = Interaction::new(mm(0.1));
    it.selection = vec![ObjectId::Footprint(f)];
    it.key(&mut doc.target(v), KeyAction::Delete);
    let p = doc.project();
    assert!(!p.board.footprints.contains(f), "the Delete key removes board parts");
    assert!(!p.board.fields.iter().any(|x| x.footprint == f));
    assert_eq!(p.validate(), Vec::<String>::new());
}
