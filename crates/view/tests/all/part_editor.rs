//! Building a part from scratch in a library and using/updating it in a project.

use editor_core::Command;
use editor_core::query::{PartState, part_statuses};
use model::*;
use view::item::{Content, Elem, Item, ItemTarget, ItemView};
use view::tools::{Placer, Tool};
use view::{Document, Interaction, KeyAction, LibraryDoc, PointerInput};

fn input(p: Point) -> PointerInput {
    PointerInput { world: Some(p), tolerance: mm(0.3), drag_threshold: mm(0.5), ..Default::default() }
}

fn click(it: &mut Interaction<Elem>, t: &mut ItemTarget, p: Point) {
    it.pointer(t, &PointerInput { pressed: true, down: true, ..input(p) });
    it.pointer(t, &PointerInput { released: true, ..input(p) });
}

fn drag(it: &mut Interaction<Elem>, t: &mut ItemTarget, from: Point, to: Point) {
    it.pointer(t, &PointerInput { pressed: true, down: true, ..input(from) });
    it.pointer(t, &PointerInput { down: true, ..input(to) });
    it.pointer(t, &PointerInput { released: true, ..input(to) });
}

fn symbol(lib: &LibraryDoc, id: Id) -> &Symbol {
    lib.library().symbols.get(id).unwrap()
}

#[test]
fn build_part_from_scratch_then_update_project() {
    let mut lib = LibraryDoc::new(Library::new("Mine"));
    let part_id = lib.new_part("Buffer", None, None).unwrap();
    let part = lib.library().parts.get(part_id).unwrap().clone();
    let (sym_id, fp_id) = (part.units[0].symbol, part.variants[0].footprint.unwrap());
    assert!(part.variants[0].connections.is_empty());
    lib.doc.mark_saved();

    // Symbol: body rectangle (two clicks) and three pins named by auto-increment.
    let mut view = ItemView::new();
    view.show(Some(Item::Symbol(sym_id)));
    view.sync(&lib.doc);
    let mut placer = Placer::default();
    placer.set_tool(Tool::Rect, view.content());
    placer.click(&mut lib.doc, &mut view, Point::mm(-5.08, -5.08)).unwrap();
    let rect = placer.click(&mut lib.doc, &mut view, Point::mm(5.08, 5.08)).unwrap();
    assert_eq!(rect, Some(Elem::Graphic(0)));
    placer.set_tool(Tool::Pin, view.content());
    assert_eq!(placer.settings.name, "1");
    for y in [2.54, 0.0] {
        placer.click(&mut lib.doc, &mut view, Point::mm(-10.16, y)).unwrap();
    }
    placer.rotate();
    placer.rotate(); // pointing left: pin on the right side
    placer.click(&mut lib.doc, &mut view, Point::mm(10.16, 0.0)).unwrap();
    let names: Vec<_> = symbol(&lib, sym_id).pins.iter().map(|p| p.name.clone()).collect();
    assert_eq!(names, ["1", "2", "3"]);
    let out = symbol(&lib, sym_id).pins.iter().last().unwrap().clone();
    assert_eq!(out.inner_end(), Point::mm(10.16 - 2.54, 0.0));
    assert!(placer.escape(), "escape leaves the tool when nothing is in progress");

    // Footprint: three pads.
    view.show(Some(Item::Footprint(fp_id)));
    view.sync(&lib.doc);
    placer.set_tool(Tool::Pad, view.content());
    for x in [-1.0, 0.0, 1.0] {
        placer.click(&mut lib.doc, &mut view, Point::mm(x, 0.0)).unwrap();
    }
    let Some(Content::Footprint(fp)) = view.content() else { panic!() };
    assert_eq!(fp.pads.len(), 3);

    // Map pins to pads by name.
    let mut part = lib.library().parts.get(part_id).unwrap().clone();
    part.variants[0].connections = lib.library().map_by_name(&part.units, fp_id);
    assert_eq!(part.variants[0].connections.len(), 3);
    lib.update_part(part).unwrap();
    assert_eq!(lib.library().validate(), Vec::<String>::new());

    // Use it in a project.
    let mut project = Document::new(Project::new("p"));
    project.execute(Command::import_parts(lib.library(), &[part_id]).unwrap()).unwrap();
    project
        .execute(Command::PlacePart {
            part: part_id,
            variant: None,
            sheet: None,
            pos: Point::ZERO,
            rotation: Angle::ZERO,
            mirror: false,
            refdes: None,
            value: None,
        })
        .unwrap();
    assert_eq!(project.project().board.footprints.len(), 1);
    assert_eq!(part_statuses(project.project(), &[lib.library()])[0].state, PartState::UpToDate);

    // Edit the footprint: drag pad 3 away and rotate it, using the generic interaction.
    let mut it: Interaction<Elem> = Interaction::new(mm(0.5));
    let pad3 = fp_pad(&lib, fp_id, "3");
    {
        let mut t = ItemTarget { doc: &mut lib.doc, view: &mut view };
        click(&mut it, &mut t, Point::mm(1.0, 0.0));
        assert_eq!(it.selection, vec![Elem::Pad(pad3.id)]);
        drag(&mut it, &mut t, Point::mm(1.0, 0.0), Point::mm(1.0, 2.1));
        it.key(&mut t, KeyAction::Rotate);
    }
    let moved = fp_pad(&lib, fp_id, "3");
    assert_eq!(moved.pos, Point::mm(1.0, 2.0), "snapped to the 0.5 mm grid");
    assert_eq!(moved.rotation, Angle::DEG90);

    // The project notices and takes the update.
    let status = &part_statuses(project.project(), &[lib.library()])[0];
    assert_eq!(status.state, PartState::LibraryChanged);
    project.execute(Command::update_parts(lib.library(), &[part_id]).unwrap()).unwrap();
    let embedded = project.project().library.footprints.get(fp_id).unwrap();
    assert_eq!(embedded.pads.get(pad3.id).unwrap().pos, Point::mm(1.0, 2.0));
    assert_eq!(part_statuses(project.project(), &[lib.library()])[0].state, PartState::UpToDate);
    assert_eq!(project.project().validate(), Vec::<String>::new());

    // Library undo: rotate, then move.
    lib.doc.undo();
    lib.doc.undo();
    assert_eq!(fp_pad(&lib, fp_id, "3").pos, Point::mm(1.0, 0.0));
}

fn fp_pad(lib: &LibraryDoc, fp: Id, name: &str) -> Pad {
    lib.library().footprints.get(fp).unwrap().pads.iter().find(|p| p.name == name).unwrap().clone()
}

#[test]
fn symbol_edits_mirror_delete_and_undo() {
    let mut lib = LibraryDoc::new(model::builtin::builtin_library());
    let led = lib.library().parts.get(model::builtin::PART_LED).unwrap().clone();
    let sym = led.units[0].symbol;
    let before = symbol(&lib, sym).clone();
    let mut view = ItemView::new();
    view.show(Some(Item::Symbol(sym)));
    view.sync(&lib.doc);
    let mut it: Interaction<Elem> = Interaction::new(mm(1.27));
    {
        let mut t = ItemTarget { doc: &mut lib.doc, view: &mut view };
        it.key(&mut t, KeyAction::SelectAll);
        assert_eq!(it.selection.len(), before.pins.len() + before.graphics.len() + before.texts.len());
        it.key(&mut t, KeyAction::Mirror);
    }
    let mirrored = symbol(&lib, sym).clone();
    let anode = before.pins.iter().find(|p| p.name == "A").unwrap();
    let m = mirrored.pins.get(anode.id).unwrap();
    assert_eq!(m.direction, anode.direction.mirrored());
    assert_eq!(mirrored.texts[0].pos, before.texts[0].pos, "centered texts stay on the axis");

    // Delete a graphic by clicking its outline, then a pin; texts keep their indices valid.
    {
        let mut t = ItemTarget { doc: &mut lib.doc, view: &mut view };
        it.key(&mut t, KeyAction::Escape);
        let bar = match &mirrored.graphics[1] {
            Shape::Line { a, .. } => *a,
            other => panic!("{other:?}"),
        };
        click(&mut it, &mut t, bar);
        assert_eq!(it.selection, vec![Elem::Graphic(1)]);
        it.key(&mut t, KeyAction::Delete);
    }
    assert_eq!(symbol(&lib, sym).graphics.len(), before.graphics.len() - 1);
    lib.doc.undo();
    lib.doc.undo();
    assert_eq!(symbol(&lib, sym), &before);
}

#[test]
fn library_objects_create_duplicate_delete() {
    let mut lib = LibraryDoc::new(Library::new("L"));
    let s = lib.new_symbol("Op amp").unwrap();
    let s2 = lib.new_symbol("Op amp").unwrap();
    assert_eq!(symbol(&lib, s2).name, "Op amp 2");
    let f = lib.new_footprint("SOIC-8").unwrap();
    let p = lib.new_part("LM358", Some(s), Some(f)).unwrap();
    assert_eq!(lib.library().parts.get(p).unwrap().variants[0].name, "SOIC-8");
    assert!(lib.delete(ObjectId::LibSymbol(s)).unwrap_err().contains("used by a part"));
    let ObjectId::LibPart(copy) = lib.duplicate(ObjectId::LibPart(p)).unwrap() else { panic!() };
    assert_eq!(lib.library().parts.get(copy).unwrap().name, "LM358 copy");
    lib.delete(ObjectId::LibPart(copy)).unwrap();
    lib.delete(ObjectId::LibPart(p)).unwrap();
    lib.delete(ObjectId::LibSymbol(s)).unwrap();
    assert_eq!(lib.library().symbols.len(), 1);
    // Each operation is one undo step.
    for _ in 0..3 {
        lib.doc.undo();
    }
    assert_eq!(lib.library().parts.len(), 2);
}

#[test]
fn natural_order_mapping() {
    let mut lib = LibraryDoc::new(Library::new("L"));
    let p = lib.new_part("X", None, None).unwrap();
    let part = lib.library().parts.get(p).unwrap().clone();
    let (sym, fp) = (part.units[0].symbol, part.variants[0].footprint.unwrap());
    let mut view = ItemView::new();
    let mut placer = Placer::default();
    view.show(Some(Item::Symbol(sym)));
    view.sync(&lib.doc);
    placer.set_tool(Tool::Pin, view.content());
    for (i, name) in ["IN", "OUT", "GND"].iter().enumerate() {
        placer.settings.name = name.to_string();
        placer.click(&mut lib.doc, &mut view, Point::mm(0.0, i as f64 * 2.54)).unwrap();
    }
    view.show(Some(Item::Footprint(fp)));
    view.sync(&lib.doc);
    placer.set_tool(Tool::Pad, view.content());
    placer.settings.name = "10".into();
    placer.click(&mut lib.doc, &mut view, Point::mm(2.0, 0.0)).unwrap();
    placer.settings.name = "2".into();
    placer.click(&mut lib.doc, &mut view, Point::mm(1.0, 0.0)).unwrap();
    placer.settings.name = "1".into();
    placer.click(&mut lib.doc, &mut view, Point::mm(0.0, 0.0)).unwrap();
    let l = lib.library();
    assert!(l.map_by_name(&part.units, fp).is_empty());
    let map = l.map_in_order(&part.units, fp);
    let pad_name = |id| l.footprints.get(fp).unwrap().pads.get(id).unwrap().name.clone();
    let pin_name = |id| l.symbols.get(sym).unwrap().pins.get(id).unwrap().name.clone();
    let pairs: Vec<_> = map.iter().map(|c| (pad_name(c.pad), pin_name(c.pin))).collect();
    assert_eq!(pairs, [("1".into(), "IN".to_string()), ("2".into(), "OUT".into()), ("10".into(), "GND".into())]);
}

#[test]
fn library_file_round_trip_takes_file_name() {
    let dir = std::env::temp_dir().join(format!("pcb-lib-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("My Parts.pcblib");
    let mut lib = LibraryDoc::new(model::builtin::builtin_library());
    lib.new_part("Extra", None, None).unwrap();
    assert!(lib.is_modified());
    lib.save_as(&path).unwrap();
    assert!(!lib.is_modified());
    assert_eq!(lib.name(), "My Parts");
    let reopened = LibraryDoc::open(&path).unwrap();
    assert_eq!(reopened.library(), lib.library());
    assert_eq!(reopened.path.as_deref(), Some(path.as_path()));
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn rename_keeps_names_unique_and_the_builtin_library_is_read_only() {
    let mut lib = LibraryDoc::new(Library::new("lib"));
    let a = lib.new_symbol("A").unwrap();
    lib.new_symbol("B").unwrap();
    lib.rename(ObjectId::LibSymbol(a), " C ").unwrap();
    assert_eq!(symbol(&lib, a).name, "C");
    assert!(lib.rename(ObjectId::LibSymbol(a), "B").unwrap_err().contains("taken"));
    assert!(lib.rename(ObjectId::LibSymbol(a), "").is_err());
    lib.rename(ObjectId::LibSymbol(a), "C").unwrap();
    assert_eq!(lib.doc.editor().history().undo_label(), Some("Update 1 object(s)"));

    let mut builtin = LibraryDoc::builtin();
    assert!(builtin.is_builtin());
    let err = builtin.new_symbol("X").unwrap_err();
    assert!(err.contains("read-only"), "{err}");
    assert!(!builtin.is_modified());
}
