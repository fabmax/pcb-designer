//! One view per schematic sheet: each draws only its own sheet's objects.

use editor_core::Command;
use model::{ObjectId, Point};
use view::demo::demo_project;
use view::{Document, ViewId};

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
