//! Which symbol and footprint the part form's preview shows.

use model::*;
use view::LibraryDoc;
use view::item::{Item, PartPreview};

#[test]
fn preview_follows_choice_and_falls_back_to_first() {
    let mut lib = LibraryDoc::new(Library::new("L"));
    let s1 = lib.new_symbol("A").unwrap();
    let s2 = lib.new_symbol("B").unwrap();
    let f1 = lib.new_footprint("F1").unwrap();
    let f2 = lib.new_footprint("F2").unwrap();
    let id = lib.new_part("P", Some(s1), Some(f1)).unwrap();
    let mut part = lib.library().parts.get(id).unwrap().clone();
    part.units.push(PartUnit { name: "B".into(), symbol: s2 });
    let v2 = Id::new();
    part.variants.push(PartVariant { id: v2, name: "F2".into(), footprint: Some(f2), connections: Vec::new() });
    let none = Id::new();
    part.variants.push(PartVariant::no_footprint(none));
    let l = lib.library();

    // Default: first unit, first variant.
    let p = PartPreview::default();
    assert_eq!(p.items(l, &part), (Some(Item::Symbol(s1)), Some(Item::Footprint(f1))));

    // A chosen unit and variant.
    let p = PartPreview { unit: 1, variant: Some(v2) };
    assert_eq!(p.items(l, &part), (Some(Item::Symbol(s2)), Some(Item::Footprint(f2))));

    // A variant without footprint shows none.
    let p = PartPreview { unit: 1, variant: Some(none) };
    assert_eq!(p.items(l, &part).1, None);

    // A removed unit or variant falls back to the first.
    let p = PartPreview { unit: 5, variant: Some(Id::new()) };
    assert_eq!(p.unit_index(&part), 0);
    assert_eq!(p.items(l, &part), (Some(Item::Symbol(s1)), Some(Item::Footprint(f1))));

    // A symbol missing from the library shows none.
    part.units[0].symbol = Id::new();
    assert_eq!(PartPreview::default().items(l, &part).0, None);

    // A board part has no symbol.
    part.units.clear();
    assert_eq!(PartPreview::default().items(l, &part).0, None);
}
