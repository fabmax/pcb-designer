//! The part editors' Arc and Polygon tools, and pad arrays.

use model::*;
use view::LibraryDoc;
use view::item::{Item, ItemView};
use view::tools::{Placer, Tool};

fn footprint_doc() -> (LibraryDoc, ItemView, Id) {
    let mut lib = LibraryDoc::new(Library::new("L"));
    let p = lib.new_part("X", None, None).unwrap();
    let fp = lib.library().parts.get(p).unwrap().variants[0].footprint.unwrap();
    let mut view = ItemView::new();
    view.show(Some(Item::Footprint(fp)));
    view.sync(&lib.doc);
    (lib, view, fp)
}

fn last_graphic(lib: &LibraryDoc, fp: Id) -> Shape {
    lib.library().footprints.get(fp).unwrap().graphics.last().unwrap().shape.clone()
}

#[test]
fn arcs_follow_the_pointer_around_the_centre() {
    let (mut lib, mut view, fp) = footprint_doc();
    let mut placer = Placer::default();
    placer.set_tool(Tool::Arc, view.content());
    placer.click(&mut lib.doc, &mut view, Point::ZERO).unwrap();
    placer.click(&mut lib.doc, &mut view, Point::mm(2.0, 0.0)).unwrap();
    // Counterclockwise to the top, then on to the left: 180°.
    for p in [Point::mm(2.0, 0.0), Point::mm(1.4, 1.4), Point::mm(0.0, 2.0), Point::mm(-1.4, 1.4)] {
        placer.hover(Some(p));
    }
    placer.hover(Some(Point::mm(-2.0, 0.0)));
    placer.click(&mut lib.doc, &mut view, Point::mm(-2.0, 0.0)).unwrap();
    let Shape::Arc { center, radius, start, sweep, .. } = last_graphic(&lib, fp) else { panic!() };
    assert_eq!((center, radius, start.degrees()), (Point::ZERO, mm(2.0), 0.0));
    assert!((sweep - 180.0).abs() < 1e-6, "{sweep}");

    // Clockwise a quarter, then / takes the long way round.
    placer.click(&mut lib.doc, &mut view, Point::ZERO).unwrap();
    placer.click(&mut lib.doc, &mut view, Point::mm(2.0, 0.0)).unwrap();
    placer.hover(Some(Point::mm(1.4, -1.4)));
    placer.hover(Some(Point::mm(0.0, -2.0)));
    placer.flip_arc();
    placer.click(&mut lib.doc, &mut view, Point::mm(0.0, -2.0)).unwrap();
    let Shape::Arc { sweep, .. } = last_graphic(&lib, fp) else { panic!() };
    assert!((sweep - 270.0).abs() < 1e-6, "{sweep}");
}

#[test]
fn polygons_close_on_the_first_or_last_corner() {
    let (mut lib, mut view, fp) = footprint_doc();
    let mut placer = Placer::default();
    placer.set_tool(Tool::Polygon, view.content());
    placer.settings.fill = true;
    let n = lib.library().footprints.get(fp).unwrap().graphics.len();
    placer.click(&mut lib.doc, &mut view, Point::ZERO).unwrap();
    placer.click(&mut lib.doc, &mut view, Point::mm(1.0, 0.0)).unwrap();
    assert!(placer.click(&mut lib.doc, &mut view, Point::mm(1.0, 0.0)).is_err(), "two corners aren't enough");
    placer.click(&mut lib.doc, &mut view, Point::mm(1.0, 1.0)).unwrap();
    placer.click(&mut lib.doc, &mut view, Point::ZERO).unwrap();
    assert_eq!(lib.library().footprints.get(fp).unwrap().graphics.len(), n + 1);
    let Shape::Polygon { points, fill, .. } = last_graphic(&lib, fp) else { panic!() };
    assert_eq!((points.len(), fill), (3, true));
    assert!(!placer.in_progress());
}

#[test]
fn pad_arrays_are_one_undo_step() {
    use view::array::{Dir, Layout, apply};
    use view::item::Elem;
    let (mut lib, mut view, fp) = footprint_doc();
    let mut placer = Placer::default();
    placer.set_tool(Tool::Pad, view.content());
    placer.click(&mut lib.doc, &mut view, Point::ZERO).unwrap();
    let pads = |lib: &LibraryDoc| lib.library().footprints.get(fp).unwrap().pads.iter().cloned().collect::<Vec<Pad>>();
    let first = pads(&lib)[0].clone();
    let layout = Layout::DualRow { per_row: 3, pitch: mm(1.27), span: mm(5.0), dir: Dir::Down };
    let elems = apply(&mut lib.doc, Item::Footprint(fp), Elem::Pad(first.id), &layout).unwrap();
    assert_eq!(elems.len(), 6);
    assert_eq!(elems[0], Elem::Pad(first.id));
    let all = pads(&lib);
    let names: Vec<&str> = all.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, ["1", "2", "3", "4", "5", "6"]);
    assert_eq!(all[5].pos, Point::mm(5.0, 0.0));
    lib.doc.undo();
    assert_eq!(pads(&lib).len(), 1);
}
