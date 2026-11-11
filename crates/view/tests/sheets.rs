//! One view per schematic sheet: each draws only its own sheet's objects.

use editor_core::Command;
use model::{Angle, LabelKind, LabelShape, ObjectId, Point};
use view::demo::demo_project;
use view::{Document, View, ViewId};

#[test]
fn each_sheet_view_draws_its_own_objects() {
    let mut doc = Document::new(demo_project());
    let root = doc.root();
    let before = doc.sheet_views().count();
    let changes = doc
        .execute(Command::AddSheet { parent: None, name: "Sub".into(), pos: Point::mm(100.0, 0.0), size: None })
        .unwrap();
    let sub = changes.changes.iter().find_map(|c| match c.id {
        ObjectId::Sheet(id) => Some(id),
        _ => None,
    });
    let sub = sub.unwrap();
    assert_eq!(doc.sheet_views().count(), before + 1);

    // The sheet symbol is drawn on the parent sheet, not inside the sheet itself.
    assert!(doc.view(root).index.bounds(ObjectId::Sheet(sub)).is_some());
    assert!(doc.view(ViewId::Sheet(sub)).index.bounds(ObjectId::Sheet(sub)).is_none());

    // A wire on the sub-sheet shows only in its view.
    let changes = doc
        .execute(Command::AddWire { sheet: Some(sub), points: vec![Point::mm(0.0, 0.0), Point::mm(10.0, 0.0)] })
        .unwrap();
    let wire = changes.changes[0].id;
    assert!(doc.view(ViewId::Sheet(sub)).index.bounds(wire).is_some());
    assert!(doc.view(root).index.bounds(wire).is_none());

    // Undoing the sheet drops its view.
    doc.undo();
    doc.undo();
    assert_eq!(doc.sheet_views().count(), before);
    assert!(doc.view(root).index.bounds(ObjectId::Sheet(sub)).is_none());
    doc.redo();
    assert_eq!(doc.sheet_views().count(), before + 1);
    assert!(doc.view(root).index.bounds(ObjectId::Sheet(sub)).is_some());
}

#[test]
fn hierarchical_labels_redraw_the_sheet_symbol() {
    let mut doc = Document::new(demo_project());
    let root = doc.root();
    let s = &doc.project().schematic;
    let sub = s.sheets.iter().find(|sh| !sh.is_root()).unwrap().id;
    let bounds = |doc: &Document| doc.view(root).index.bounds(ObjectId::Sheet(sub)).unwrap();
    let before = bounds(&doc);
    // Pins that don't fit: the symbol grows.
    for i in 0..10 {
        doc.execute(Command::AddLabel {
            sheet: Some(sub),
            pos: Point::mm(100.0, i as f64 * 2.54),
            name: format!("A{i}"),
            kind: LabelKind::Hierarchical,
            shape: LabelShape::Input,
            rotation: Angle::ZERO,
        })
        .unwrap();
    }
    let grown = bounds(&doc);
    assert!(grown.min.y < before.min.y, "{grown:?} {before:?}");
    doc.update_connectivity(false);
    let mut fresh = View::new(root, doc.project());
    fresh.set_connectivity(doc.connectivity());
    assert_eq!(fresh.index.bounds(ObjectId::Sheet(sub)), Some(grown));
    assert_eq!(fresh.scene.instance_count(), doc.view(root).scene.instance_count());
    for _ in 0..10 {
        doc.undo();
    }
    assert_eq!(bounds(&doc), before);
}
