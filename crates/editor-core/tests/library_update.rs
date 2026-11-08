use editor_core::query::{PartState, part_statuses};
use editor_core::{Command, Editor};
use model::builtin::{self, PART_LED, PART_RESISTOR};
use model::*;

fn place(ed: &mut Editor, part: Id, x: f64) -> Id {
    ed.execute(Command::PlacePart {
        part,
        variant: None,
        sheet: None,
        pos: Point::mm(x, 0.0),
        rotation: Angle::ZERO,
        mirror: false,
        refdes: None,
        value: None,
    })
    .unwrap();
    ed.project().schematic.components.iter().last().unwrap().id
}

#[test]
fn update_parts_from_changed_library() {
    let mut lib = builtin::starter_library();
    let mut ed = Editor::new(Project::new("t"));
    ed.execute(Command::ImportParts { library: lib.clone(), parts: vec![PART_RESISTOR, PART_LED] }).unwrap();
    let r1 = place(&mut ed, PART_RESISTOR, 0.0);
    let original = ed.project().clone();
    assert!(part_statuses(ed.project(), &[&lib]).iter().all(|s| s.state == PartState::UpToDate));
    assert!(part_statuses(ed.project(), &[]).iter().all(|s| s.state == PartState::LibraryMissing));

    // Library edits: bigger pads on the 0805 footprint, the 0603 variant (in use) removed,
    // and a brand-new symbol.
    let part = lib.parts.get(PART_RESISTOR).unwrap().clone();
    let v0805 = part.variants.iter().find(|v| v.name == "0805").unwrap().clone();
    let fp = lib.footprints.get_mut(v0805.footprint).unwrap();
    let pad_ids: Vec<Id> = fp.pads.ids().collect();
    for id in &pad_ids {
        fp.pads.get_mut(*id).unwrap().size = Size::mm(1.5, 1.6);
    }
    let old_symbol = part.units[0].symbol;
    let mut sym = lib.symbols.get(old_symbol).unwrap().clone();
    sym.id = Id::new();
    sym.name = "Resistor (US)".into();
    let new_symbol = sym.id;
    lib.symbols.insert(sym);
    let p = lib.parts.get_mut(PART_RESISTOR).unwrap();
    p.variants.retain(|v| v.name == "0805");
    p.units[0].symbol = new_symbol;

    let statuses = part_statuses(ed.project(), &[&lib]);
    let r = statuses.iter().find(|s| s.part == PART_RESISTOR).unwrap();
    assert_eq!(r.state, PartState::LibraryChanged);
    assert_eq!(statuses.iter().find(|s| s.part == PART_LED).unwrap().state, PartState::UpToDate);

    ed.execute(Command::UpdateParts { library: lib.clone(), parts: vec![PART_RESISTOR] }).unwrap();
    let p = ed.project();
    assert_eq!(p.validate(), Vec::<String>::new());
    assert!(part_statuses(p, &[&lib]).iter().all(|s| s.state == PartState::UpToDate));
    let c = p.schematic.components.get(r1).unwrap();
    assert_eq!(c.variant, v0805.id, "component moved to the remaining variant");
    let fp = p.library.footprint_of(c.part, c.variant).unwrap();
    assert!(fp.pads.iter().all(|pad| pad.size == Size::mm(1.5, 1.6)));
    assert!(p.library.symbols.contains(new_symbol));
    assert!(!p.library.symbols.contains(old_symbol), "unused old symbol removed");

    // One undo step restores the project exactly.
    ed.undo();
    assert_eq!(ed.project(), &original);
}

#[test]
fn update_refuses_to_drop_placed_units() {
    let mut lib = builtin::starter_library();
    let mut ed = Editor::new(Project::new("t"));
    ed.execute(Command::ImportParts { library: lib.clone(), parts: vec![PART_RESISTOR] }).unwrap();
    place(&mut ed, PART_RESISTOR, 0.0);
    lib.parts.get_mut(PART_RESISTOR).unwrap().units.clear();
    let err = ed.execute(Command::UpdateParts { library: lib, parts: vec![PART_RESISTOR] }).unwrap_err();
    assert!(err.to_string().contains("unit"), "{err}");
}
