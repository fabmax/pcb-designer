//! Import of the complete official KiCad libraries. Skipped unless the library locations
//! are given:
//!
//! ```sh
//! KICAD_SYMBOL_DIR=/path/to/symbols KICAD_FOOTPRINT_DIR=/path/to/footprints \
//!     cargo test -p kicad --release --test corpus -- --nocapture
//! ```

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Instant;

use kicad::Catalog;
use kicad::import::{Request, import, suggested_footprints};
use model::*;

fn catalog() -> Option<Catalog> {
    let s = std::env::var("KICAD_SYMBOL_DIR").ok()?;
    let f = std::env::var("KICAD_FOOTPRINT_DIR").ok()?;
    let t = Instant::now();
    let c = Catalog::scan(&[PathBuf::from(s), PathBuf::from(f)]);
    eprintln!(
        "indexed {} symbols in {} libraries, {} footprints in {} libraries in {:?}",
        c.symbol_count(),
        c.symbol_libs.len(),
        c.footprint_count(),
        c.footprint_libs.len(),
        t.elapsed()
    );
    Some(c)
}

fn summarize(kind: &str, warnings: &[String]) {
    // Count warnings by message (without the item name).
    let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
    for w in warnings {
        *counts.entry(w.split_once(": ").map_or(w.as_str(), |x| x.1)).or_default() += 1;
    }
    let mut v: Vec<_> = counts.into_iter().collect();
    v.sort_by_key(|x| std::cmp::Reverse(x.1));
    eprintln!("{kind} warnings:");
    for (w, n) in v.iter().take(25) {
        eprintln!("  {n:6}  {w}");
    }
}

#[test]
fn all_footprints_convert() {
    let Some(c) = catalog() else { return };
    let t = Instant::now();
    let mut warnings = Vec::new();
    let mut failures = Vec::new();
    let mut n = 0;
    for l in &c.footprint_libs {
        for f in &l.footprints {
            let full = format!("{}:{f}", l.name);
            match c.load_footprint(&full) {
                Ok(fp) => {
                    n += 1;
                    warnings.extend(fp.warnings.iter().map(|w| format!("{full}: {w}")));
                    for pad in fp.footprint.pads.iter() {
                        assert!(pad.size.w >= 0 && pad.size.h >= 0, "{full}: pad {}", pad.name);
                        if let PadShape::Polygon { points } = &pad.shape {
                            assert!(points.len() >= 3, "{full}: pad {} polygon", pad.name);
                        }
                    }
                    let json = serde_json::to_string(&fp.footprint).unwrap();
                    let back: Footprint = serde_json::from_str(&json).unwrap();
                    assert_eq!(back, fp.footprint, "{full}");
                }
                Err(e) => failures.push(e.to_string()),
            }
        }
    }
    eprintln!("converted {n} footprints in {:?}", t.elapsed());
    summarize("footprint", &warnings);
    assert!(failures.is_empty(), "{} failures, e.g. {:?}", failures.len(), &failures[..failures.len().min(5)]);
}

#[test]
fn all_symbols_convert_into_valid_parts() {
    let Some(mut c) = catalog() else { return };
    let t = Instant::now();
    let names: Vec<String> =
        c.symbol_libs.iter().flat_map(|l| l.symbols.iter().map(move |s| format!("{}:{s}", l.name))).collect();
    let mut req = Request::default();
    let mut failures = Vec::new();
    let mut power = 0;
    let mut no_fp = 0;
    for full in &names {
        match c.load_symbol(full) {
            Ok(sym) => {
                if sym.power {
                    power += 1;
                    continue;
                }
                assert!(!sym.units.is_empty(), "{full}");
                let fps = suggested_footprints(&mut c, &sym);
                if fps.is_empty() {
                    no_fp += 1;
                }
                req.symbols.push((full.clone(), fps));
            }
            Err(e) => failures.push(e.to_string()),
        }
    }
    eprintln!(
        "converted {} symbols ({power} power symbols skipped, {no_fp} without a footprint) in {:?}",
        names.len() - failures.len(),
        t.elapsed()
    );
    assert!(failures.is_empty(), "{} failures, e.g. {:?}", failures.len(), &failures[..failures.len().min(5)]);

    let t = Instant::now();
    let imported = import(&mut c, &req);
    eprintln!(
        "built {} symbols, {} footprints, {} parts in {:?}",
        imported.symbols.len(),
        imported.footprints.len(),
        imported.parts.len(),
        t.elapsed()
    );
    summarize("symbol", &imported.warnings);
    assert!(imported.errors.is_empty(), "{:?}", &imported.errors[..imported.errors.len().min(5)]);

    let mut lib = Library::new("KiCad");
    for s in imported.symbols {
        assert!(lib.symbols.insert(s).is_none(), "duplicate symbol id");
    }
    for f in imported.footprints {
        lib.footprints.insert(f);
    }
    // Pads without a pin are normal (unconnected pins are often not drawn); pins without
    // a pad mean a wrong footprint choice or numbering.
    let mut pinless = Vec::new();
    for p in imported.parts {
        let v = &p.variants[0];
        let fp = lib.footprints.get(v.footprint.unwrap()).unwrap();
        let mut missing = Vec::new();
        for (u, unit) in p.units.iter().enumerate() {
            for pin in lib.symbols.get(unit.symbol).unwrap().pins.iter() {
                if !v.connections.iter().any(|c| c.unit == u as u32 && c.pin == pin.id) {
                    missing.push(pin.key().to_string());
                }
            }
        }
        if !missing.is_empty() {
            pinless.push(format!("{} / {}: pins {missing:?}", p.properties["KiCad symbol"], fp.name));
        }
        assert!(lib.parts.insert(p).is_none(), "duplicate part id");
    }
    eprintln!("{} parts have pins without a pad:", pinless.len());
    for p in pinless.iter().take(25) {
        eprintln!("  {p}");
    }
    // These come from KiCad's own data (pins numbered differently than the pads of the
    // default footprint, filters naming the wrong package); keep them rare.
    assert!(pinless.len() * 20 < lib.parts.len(), "{} of {} parts", pinless.len(), lib.parts.len());
    let issues = lib.validate();
    assert!(issues.is_empty(), "{} issues, e.g. {:?}", issues.len(), &issues[..issues.len().min(10)]);

    // Spot checks.
    let r = lib.parts.iter().find(|p| p.name == "R" && p.properties["KiCad symbol"] == "Device:R").unwrap();
    assert_eq!(r.prefix, "R");
    assert_eq!(r.units.len(), 1);
    assert_eq!(r.variants[0].connections.len(), 2);
    assert_eq!(r.variants[0].name, "R_0603_1608Metric");
    let lm = lib.parts.iter().find(|p| p.properties["KiCad symbol"] == "Amplifier_Operational:LM2904").unwrap();
    assert_eq!(lm.units.len(), 3, "two op-amps and the power unit");
    let fp = lib.footprints.get(lm.variants[0].footprint.unwrap()).unwrap();
    assert_eq!(lm.variants[0].connections.len(), fp.pads.len(), "{}", fp.name);
    let pins: usize = lm.units.iter().map(|u| lib.symbols.get(u.symbol).unwrap().pins.len()).sum();
    assert_eq!(pins, 8);

    let t = Instant::now();
    let json = serde_json::to_string(&lib).unwrap();
    eprintln!("library JSON: {} MB in {:?}", json.len() / 1_000_000, t.elapsed());
}
