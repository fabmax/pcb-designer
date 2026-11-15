//! Every schematic sheet is drawn on a page with a frame; the title block is the
//! project's.

use editor_core::Command;
use model::{Object, Page, PageSize, Point, Project, Rect, TitleBlock};
use view::demo::demo_project;
use view::{Document, ViewId};

fn scene_rect(doc: &Document, view: ViewId) -> Option<Rect> {
    doc.view(view).scene.bounds().map(|[x0, y0, x1, y1]| Rect::from_points(Point::mm(x0, y0), Point::mm(x1, y1)))
}

fn set_page(doc: &mut Document, view: ViewId, page: Page) {
    let mut sheet = doc.project().schematic.sheets.get(view.sheet().unwrap()).unwrap().clone();
    sheet.page = page;
    doc.execute(Command::Update { objects: vec![Object::Sheet(sheet)] }).unwrap();
}

#[test]
fn the_frame_spans_the_page_and_follows_its_size() {
    let mut doc = Document::new(Project::new("Frames"));
    let root = doc.root();
    // An empty A4 sheet: the scene is the page (the frame's line width aside).
    let r = scene_rect(&doc, root).expect("a frame is drawn");
    assert!((r.width() as f64 * 1e-6 - 297.0).abs() < 0.5, "{r:?}");
    assert!((r.height() as f64 * 1e-6 - 210.0).abs() < 0.5, "{r:?}");
    assert!(r.min.x.abs() < 500_000 && r.min.y.abs() < 500_000, "the page stands on the origin: {r:?}");

    set_page(&mut doc, root, Page { size: PageSize::A3, portrait: true });
    let r = scene_rect(&doc, root).unwrap();
    assert!((r.width() as f64 * 1e-6 - 297.0).abs() < 0.5 && (r.height() as f64 * 1e-6 - 420.0).abs() < 0.5);

    // No page: nothing is drawn.
    set_page(&mut doc, root, Page { size: PageSize::None, portrait: false });
    assert!(scene_rect(&doc, root).is_none());
    doc.undo();
    assert!(scene_rect(&doc, root).is_some(), "undo brings the frame back");

    // A custom page needs a positive size.
    let bad = Page { size: PageSize::Custom { w: 0, h: 100_000_000 }, portrait: false };
    let mut sheet = doc.project().schematic.sheets.get(root.sheet().unwrap()).unwrap().clone();
    sheet.page = bad;
    assert!(doc.execute(Command::Update { objects: vec![Object::Sheet(sheet)] }).is_err());
}

#[test]
fn the_title_block_is_shared_and_undoable() {
    let mut doc = Document::new(demo_project());
    let root = doc.root();
    let sub = doc.project().schematic.page_order()[1];
    let instances = |doc: &Document, v: ViewId| doc.view(v).scene.instance_count();
    let (root_before, sub_before) = (instances(&doc, root), instances(&doc, ViewId::Sheet(sub)));

    let tb = TitleBlock { title: "LED DEMO".into(), revision: "A".into(), ..Default::default() };
    doc.execute(Command::SetTitleBlock { title_block: tb.clone() }).unwrap();
    assert_eq!(doc.project().schematic.title_block(), tb);
    // Every sheet's frame shows it.
    assert!(instances(&doc, root) > root_before);
    assert!(instances(&doc, ViewId::Sheet(sub)) > sub_before);

    doc.undo();
    assert_eq!(doc.project().schematic.title_block(), TitleBlock::default());
    assert_eq!(instances(&doc, root), root_before);

    // Each line of the comment is a line in the frame: more lines, more strokes.
    let one = TitleBlock { comment: "AB".into(), ..Default::default() };
    doc.execute(Command::SetTitleBlock { title_block: one }).unwrap();
    let after_one = instances(&doc, root);
    let two = TitleBlock { comment: "AB\nAB".into(), ..Default::default() };
    doc.execute(Command::SetTitleBlock { title_block: two }).unwrap();
    assert!(instances(&doc, root) > after_one);
}

#[test]
fn pages_are_numbered_depth_first_by_name() {
    let mut doc = Document::new(Project::new("Pages"));
    let root = doc.project().schematic.root_sheet().unwrap();
    for name in ["Sheet 10", "Sheet 2"] {
        doc.execute(Command::AddSheet { parent: None, name: name.into(), pos: Point::mm(20.0, 20.0), size: None })
            .unwrap();
    }
    let s = &doc.project().schematic;
    let names: Vec<&str> = s.page_order().iter().map(|&id| s.sheets.get(id).unwrap().name.as_str()).collect();
    assert_eq!(names, ["Schematic", "Sheet 2", "Sheet 10"]);
    assert_eq!(s.page_order()[0], root);
}

#[test]
fn the_demo_sits_inside_its_pages() {
    let doc = Document::new(demo_project());
    for &sheet in &doc.project().schematic.page_order() {
        let view = ViewId::Sheet(sheet);
        let page = doc.project().schematic.sheets.get(sheet).unwrap().page.rect().unwrap();
        let content = doc.view(view).bounds().unwrap();
        assert!(page.contains(content.min) && page.contains(content.max), "{content:?} outside {page:?}");
    }
}
