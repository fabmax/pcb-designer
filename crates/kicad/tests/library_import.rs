//! KiCad libraries on disk → catalog → import into a library document, in both symbol
//! library formats; re-importing updates instead of duplicating.

use std::fs;

use kicad::Catalog;
use kicad::import::{Request, import, import_with_defaults};
use view::LibraryDoc;

const SYM: &str = r#"(kicad_symbol_lib (version 20231120) (generator "kicad_symbol_editor")
  (symbol "R" (pin_numbers hide) (pin_names (offset 0) hide) (in_bom yes) (on_board yes)
    (property "Reference" "R" (at 2.032 0 90) (effects (font (size 1.27 1.27))))
    (property "Value" "R" (at 0 0 90) (effects (font (size 1.27 1.27))))
    (property "Footprint" "" (at 0 0 0) (effects (font (size 1.27 1.27)) hide))
    (property "ki_fp_filters" "R_*" (at 0 0 0) (effects (font (size 1.27 1.27)) hide))
    (symbol "R_0_1" (rectangle (start -1.016 -2.54) (end 1.016 2.54) (stroke (width 0.254) (type default)) (fill (type none))))
    (symbol "R_1_1"
      (pin passive line (at 0 3.81 270) (length 1.27) (name "~" (effects (font (size 1.27 1.27)))) (number "1" (effects (font (size 1.27 1.27)))))
      (pin passive line (at 0 -3.81 90) (length 1.27) (name "~" (effects (font (size 1.27 1.27)))) (number "2" (effects (font (size 1.27 1.27)))))
    )
  )
  (symbol "R_Small" (extends "R")
    (property "Reference" "R" (at 0 0 0) (effects (font (size 1.27 1.27))))
    (property "Value" "R_Small" (at 0 0 0) (effects (font (size 1.27 1.27))))
    (property "Footprint" "Resistor_SMD:R_0805_2012Metric" (at 0 0 0) (effects (font (size 1.27 1.27)) hide))
  )
)"#;

fn footprint(name: &str, pitch: f64) -> String {
    format!(
        r#"(footprint "{name}" (version 20240108) (generator "pcbnew") (layer "F.Cu")
  (property "Reference" "REF**" (at 0 -1.5 0) (layer "F.SilkS") (effects (font (size 1 1) (thickness 0.15))))
  (fp_rect (start -1.5 -0.8) (end 1.5 0.8) (stroke (width 0.05) (type solid)) (fill none) (layer "F.CrtYd"))
  (pad "1" smd roundrect (at -{pitch} 0) (size 0.8 0.9) (layers "F.Cu" "F.Paste" "F.Mask") (roundrect_rratio 0.25))
  (pad "2" smd roundrect (at {pitch} 0) (size 0.8 0.9) (layers "F.Cu" "F.Paste" "F.Mask") (roundrect_rratio 0.25))
)"#
    )
}

fn setup(dir: &std::path::Path, symdir: bool) {
    if symdir {
        // KiCad 10: one file per symbol; derived symbols refer to their parent's file.
        let d = dir.join("Device.kicad_symdir");
        fs::create_dir_all(&d).unwrap();
        let (r, small) = SYM.split_at(SYM.find("  (symbol \"R_Small\"").unwrap());
        let header = "(kicad_symbol_lib (version 20251024) (generator \"kicad_symbol_editor\")\n";
        fs::write(d.join("R.kicad_sym"), format!("{r})")).unwrap();
        fs::write(d.join("R_Small.kicad_sym"), format!("{header}{small}")).unwrap();
    } else {
        fs::write(dir.join("Device.kicad_sym"), SYM).unwrap();
    }
    let fp = dir.join("Resistor_SMD.pretty");
    fs::create_dir_all(&fp).unwrap();
    fs::write(fp.join("R_0603_1608Metric.kicad_mod"), footprint("R_0603_1608Metric", 0.825)).unwrap();
    fs::write(fp.join("R_0805_2012Metric.kicad_mod"), footprint("R_0805_2012Metric", 0.9125)).unwrap();
}

fn check(symdir: bool) {
    let tmp = tempfile::tempdir().unwrap();
    setup(tmp.path(), symdir);
    let mut cat = Catalog::scan(&[tmp.path().to_path_buf()]);
    assert_eq!(cat.symbol_count(), 2);
    assert_eq!(cat.footprint_count(), 2);
    assert_eq!(cat.search_symbols("device small", 10), ["Device:R_Small"]);
    assert_eq!(cat.search_footprints("0805", 10), ["Resistor_SMD:R_0805_2012Metric"]);

    // R: chosen by filter (0603 preferred); R_Small: its default footprint.
    let imported = import_with_defaults(&mut cat, &["Device:R", "Device:R_Small"]).unwrap();
    assert!(imported.errors.is_empty(), "{:?}", imported.errors);
    assert_eq!(imported.parts.len(), 2);
    assert_eq!(imported.footprints.len(), 2);
    let small = imported.parts.iter().find(|p| p.name == "R_Small").unwrap();
    assert_eq!(small.variants[0].name, "R_0805_2012Metric");
    assert_eq!(small.variants[0].connections.len(), 2);

    let mut lib = LibraryDoc::new(model::Library::new("Imported"));
    let (added, replaced) =
        lib.upsert(imported.symbols.clone(), imported.footprints.clone(), imported.parts.clone()).unwrap();
    assert_eq!((added, replaced), (6, 0));
    assert!(lib.library().validate().is_empty());

    // Saved and reloaded, then imported again: everything is updated in place.
    let path = tmp.path().join("imported.pcblib");
    lib.save_as(&path).unwrap();
    let mut lib = LibraryDoc::open(&path).unwrap();
    let again = import(
        &mut cat,
        &Request {
            symbols: vec![(
                "Device:R".into(),
                vec!["Resistor_SMD:R_0603_1608Metric".into(), "Resistor_SMD:R_0805_2012Metric".into()],
            )],
            footprints: vec![],
        },
    );
    let (added, replaced) = lib.upsert(again.symbols, again.footprints, again.parts).unwrap();
    assert_eq!((added, replaced), (0, 4));
    let r = lib.library().parts.iter().find(|p| p.name == "R").unwrap();
    assert_eq!(r.variants.len(), 2, "the re-import added the second variant");
    assert_eq!(lib.library().parts.len(), 2);
    assert!(lib.library().validate().is_empty());
    // One undo step reverts the whole import.
    lib.doc.undo();
    let r = lib.library().parts.iter().find(|p| p.name == "R").unwrap();
    assert_eq!(r.variants.len(), 1);
}

#[test]
fn kicad_sym_files() {
    check(false);
}

#[test]
fn kicad_symdir_folders() {
    check(true);
}

#[test]
fn name_clashes_get_unique_names() {
    let tmp = tempfile::tempdir().unwrap();
    setup(tmp.path(), false);
    let mut cat = Catalog::scan(&[tmp.path().to_path_buf()]);
    let imported = import_with_defaults(&mut cat, &["Device:R"]).unwrap();
    // A library that already has an unrelated part named "R".
    let mut lib = LibraryDoc::new(model::Library::new("Mine"));
    lib.new_part("R", None, None).unwrap();
    lib.upsert(imported.symbols, imported.footprints, imported.parts).unwrap();
    let mut names: Vec<&str> = lib.library().parts.iter().map(|p| p.name.as_str()).collect();
    names.sort();
    assert_eq!(names, ["R", "R 2"]);
}
