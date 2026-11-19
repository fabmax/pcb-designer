use std::collections::HashSet;

use super::*;
use crate::geom::Shape;
use crate::library::*;
use crate::units::{Coord, mm};

/// The pad name each pin of `part`'s variant `variant` connects to, by pin number/name.
fn pinout(lib: &Library, part: Id, variant: &str) -> Vec<(String, String)> {
    let part = lib.parts.get(part).unwrap();
    let v = part.variants.iter().find(|v| v.name == variant).unwrap_or_else(|| panic!("no variant {variant}"));
    let fp = lib.footprints.get(v.footprint.unwrap()).unwrap();
    let mut out: Vec<(String, String)> = v
        .connections
        .iter()
        .map(|c| {
            let sym = lib.symbols.get(part.units[c.unit as usize].symbol).unwrap();
            let pin = sym.pins.get(c.pin).unwrap();
            (pin.key().to_string(), fp.pads.get(c.pad).unwrap().name.clone())
        })
        .collect();
    out.sort();
    out
}

fn pairs(v: &[(&str, &str)]) -> Vec<(String, String)> {
    let mut out: Vec<_> = v.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect();
    out.sort();
    out
}

#[test]
fn bundled_library_is_up_to_date() {
    let lib = builtin_library();
    assert_eq!(lib.validate(), Vec::<String>::new());
    assert!(
        generate::merge(&lib) == lib,
        "the bundled library differs from the generator: run `cargo run -p model --example gen_builtin`"
    );
}

#[test]
fn ids_are_unique() {
    let lib = generate::library();
    let mut seen = HashSet::new();
    let mut add = |id: Id, what: &str| assert!(seen.insert(id), "{what}: id {id} twice");
    for s in lib.symbols.iter() {
        add(s.id, &s.name);
        for p in s.pins.iter() {
            add(p.id, &s.name);
        }
    }
    for f in lib.footprints.iter() {
        add(f.id, &f.name);
        for p in f.pads.iter() {
            add(p.id, &f.name);
        }
    }
    for p in lib.parts.iter() {
        add(p.id, &p.name);
        for v in &p.variants {
            add(v.id, &p.name);
        }
    }
    assert!(seen.iter().all(|&id| is_generated(id)));
    assert!(!is_generated(LIBRARY_ID) && !is_generated(PART_OPAMP));
}

fn pad_rect(p: &Pad) -> [Coord; 4] {
    let (w, h) = (p.size.w / 2, p.size.h / 2);
    [p.pos.x - w, p.pos.y - h, p.pos.x + w, p.pos.y + h]
}

fn gap(a: [Coord; 4], b: [Coord; 4]) -> Coord {
    let dx = (b[0] - a[2]).max(a[0] - b[2]);
    let dy = (b[1] - a[3]).max(a[1] - b[3]);
    dx.max(dy)
}

/// Pads keep apart, the courtyard holds them, and silkscreen lines stay off the copper.
#[test]
fn footprints_are_sound() {
    let lib = generate::library();
    for f in lib.footprints.iter() {
        let pads: Vec<&Pad> = f.pads.iter().collect();
        for (i, a) in pads.iter().enumerate() {
            for b in &pads[i + 1..] {
                assert!(
                    gap(pad_rect(a), pad_rect(b)) >= mm(0.2),
                    "{}: pads {} and {} too close",
                    f.name,
                    a.name,
                    b.name
                );
            }
        }
        let court = f.graphics.iter().find(|g| g.layer == FootprintLayer::Courtyard).expect("a courtyard");
        let Shape::Rect { a, b, .. } = court.shape else { panic!("{}: courtyard is a rect", f.name) };
        for p in &pads {
            let r = pad_rect(p);
            assert!(
                r[0] >= a.x && r[1] >= a.y && r[2] <= b.x && r[3] <= b.y,
                "{}: pad {} outside the courtyard",
                f.name,
                p.name
            );
        }
        for g in f.graphics.iter().filter(|g| g.layer == FootprintLayer::Silkscreen) {
            if let Shape::Line { a, b, width } = g.shape {
                let line = [
                    a.x.min(b.x) - width / 2,
                    a.y.min(b.y) - width / 2,
                    a.x.max(b.x) + width / 2,
                    a.y.max(b.y) + width / 2,
                ];
                for p in &pads {
                    assert!(gap(line, pad_rect(p)) >= mm(0.19), "{}: silkscreen on pad {}", f.name, p.name);
                }
            }
        }
    }
}

#[test]
fn symbol_pins_are_on_the_grid() {
    let lib = generate::library();
    for s in lib.symbols.iter() {
        for p in s.pins.iter() {
            assert!(p.pos.x % mm(2.54) == 0 && p.pos.y % mm(2.54) == 0, "{} pin {}: {:?}", s.name, p.key(), p.pos);
        }
    }
}

#[test]
fn every_part_maps_its_pins() {
    let lib = generate::library();
    for part in lib.parts.iter().filter(|p| !p.is_board_part()) {
        let sym = lib.symbols.get(part.units[0].symbol).unwrap();
        for v in &part.variants {
            let mapped: HashSet<Id> = v.connections.iter().map(|c| c.pin).collect();
            // Two-pin crystals leave the case pads open, connectors their mounting pads.
            assert_eq!(mapped.len(), sym.pins.len(), "{} {}: a pin without pad", part.name, v.name);
        }
    }
}

#[test]
fn polarities() {
    let lib = builtin_library();
    for v in ["0603", "0402", "1206", "THT Ø5 mm"] {
        assert_eq!(pinout(&lib, PART_LED, v), pairs(&[("1", "1"), ("2", "2")]), "LED {v}: pad 1 is the cathode");
    }
    let led = lib.symbols.iter().find(|s| s.name == "LED").unwrap();
    assert_eq!(led.pins.iter().find(|p| p.number == "1").unwrap().name, "K");
    let cp = gid("part/Capacitor_Polarized");
    assert_eq!(pinout(&lib, cp, "Tantalum A (3216)"), pairs(&[("1", "1"), ("2", "2")]));
}

#[test]
fn transistor_pinouts() {
    let lib = builtin_library();
    assert_eq!(pinout(&lib, PART_NPN, "SOT-23"), pairs(&[("B", "1"), ("E", "2"), ("C", "3")]));
    assert_eq!(pinout(&lib, PART_NPN, "TO-92 (EBC)"), pairs(&[("E", "1"), ("B", "2"), ("C", "3")]));
    // The SOT-223 tab is pin 2 as well.
    assert_eq!(pinout(&lib, PART_NPN, "SOT-223"), pairs(&[("B", "1"), ("C", "2"), ("C", "2"), ("E", "3")]));
    let nmos = gid("part/NMOS");
    assert_eq!(pinout(&lib, nmos, "SOT-23"), pairs(&[("G", "1"), ("S", "2"), ("D", "3")]));
    assert_eq!(pinout(&lib, nmos, "DPAK"), pairs(&[("G", "1"), ("D", "2"), ("S", "3")]));
}

#[test]
fn connectors() {
    let lib = builtin_library();
    let names: Vec<&str> = lib.parts.get(PART_HEADER_1X4).unwrap().variants.iter().map(|v| v.name.as_str()).collect();
    assert_eq!(
        names,
        [
            "Pin header",
            "Pin header, right angle",
            "Pin socket",
            "Pin header SMD",
            "JST PH",
            "JST XH",
            "JST SH",
            "Screw terminal 5.00 mm",
            "Screw terminal 3.50 mm"
        ]
    );
    // JST SH's mounting pads stay unconnected.
    assert_eq!(pinout(&lib, PART_HEADER_1X4, "JST SH").len(), 4);
    let parts = |prefix: &str| lib.parts.iter().filter(|p| p.name.starts_with(prefix)).count();
    assert_eq!((parts("Conn_01x"), parts("Conn_02x")), (40, 20));
    // Two rows: odd pins on the left, numbered across.
    let c = lib.parts.get(gid("part/Conn_02x03")).unwrap();
    let fp = lib.footprints.get(c.variants[0].footprint.unwrap()).unwrap();
    let pos = |n: &str| fp.pads.iter().find(|p| p.name == n).unwrap().pos;
    assert!(pos("1").x < pos("2").x && pos("1").y == pos("2").y && pos("3").y < pos("1").y);
}

#[test]
fn merging_keeps_hand_made_parts() {
    let mut file = builtin_library();
    let mut part = file.parts.get(PART_RESISTOR).unwrap().clone();
    part.id = Id::new();
    part.name = "My resistor".into();
    file.parts.insert(part.clone());
    let merged = generate::merge(&file);
    assert_eq!(merged.parts.get(part.id), Some(&part));
    assert_eq!(merged.parts.len(), builtin_library().parts.len() + 1);
}
