//! The schematic as a PDF.

use editor_core::Command;
use model::*;
use view::Document;

#[test]
fn pages_follow_the_hierarchy_depth_first() {
    let mut doc = Document::new(view::demo::demo_project());
    let s = &doc.project().schematic;
    let indicator = s.sheets.iter().find(|x| x.name == "Indicator").unwrap().id;
    // A sheet under Indicator comes before Testbench.
    let add =
        Command::AddSheet { parent: Some(indicator), name: "Deep".into(), pos: Point::mm(20.0, 20.0), size: None };
    doc.execute(add).unwrap();
    let order: Vec<String> = view::print::sheets_in_order(doc.project())
        .into_iter()
        .map(|id| doc.project().schematic.sheet_path(id))
        .collect();
    assert_eq!(order, ["", "/Indicator", "/Indicator/Deep", "/Testbench"]);
}

#[test]
fn a_sheet_without_page_gets_its_drawing_and_a_margin() {
    let mut doc = Document::new(view::demo::demo_project());
    let mut root = doc.project().schematic.sheets.get(doc.project().schematic.root_sheet().unwrap()).unwrap().clone();
    root.page.size = PageSize::None;
    doc.execute(Command::Update { objects: vec![Object::Sheet(root)] }).unwrap();
    let conn = doc.current_connectivity().clone();
    let pdf = view::print::schematic_pdf(doc.project(), &conn);
    let text = String::from_utf8_lossy(&pdf);
    let boxes: Vec<&str> = text.match_indices("/MediaBox [").map(|(i, _)| &text[i + 11..i + 40]).collect();
    assert_eq!(boxes.len(), 3);
    assert!(boxes[1].starts_with("0 0 841.8898 595.2756"), "the sub-sheets stay A4: {boxes:?}");
    let w: f32 = boxes[0].split(' ').nth(2).unwrap().parse().unwrap();
    assert!(w > 50.0 && w < 600.0, "about the drawing: {boxes:?}");
}
