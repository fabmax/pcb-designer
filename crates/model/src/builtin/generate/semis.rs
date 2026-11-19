//! Diodes, LEDs and transistors, and the transistor packages.

use super::fp::{Fp, row};
use super::passives::{axial, chip, two_pad};
use super::{Builder, Map, PartSpec, sym, variant};
use crate::geom::{Angle, Point, Shape};
use crate::units::mm;

pub(super) fn add(b: &mut Builder) {
    diodes(b);
    leds(b);
    transistors(b);
}

/// Diode packages (pad 1 is the cathode), as (variant name, footprint name).
fn diode_footprints(b: &mut Builder) -> Vec<(&'static str, String)> {
    let mut out = Vec::new();
    // SMD: package, pad centre, pad w × h, body l × w.
    for (pkg, c, pw, ph, bl, bw) in [
        ("SOD-123", 1.635, 0.91, 1.22, 2.7, 1.6),
        ("SOD-323", 1.15, 0.6, 0.45, 1.7, 1.25),
        ("MiniMELF", 1.75, 0.9, 1.7, 3.5, 1.5),
        ("SMA", 2.0, 2.5, 1.7, 4.3, 2.6),
        ("SMB", 2.15, 2.5, 2.3, 4.3, 3.6),
        ("SMC", 3.4, 2.5, 3.3, 6.9, 5.9),
    ] {
        let name = format!("D_{pkg}");
        let description = format!("Diode, {pkg}, pad 1 is the cathode");
        b.footprint(two_pad(&name, &description, c, pw, ph, bl, bw, true).finish());
        out.push((pkg, name));
    }
    // Through-hole, lying: package, footprint name, pitch, pad, drill, body l × d.
    for (pkg, name, pitch, pad, drill, l, d) in [
        ("DO-35", "D_DO-35_SOD27_P7.62mm_Horizontal", 7.62, 1.6, 0.8, 4.0, 2.0),
        ("DO-41", "D_DO-41_SOD81_P10.16mm_Horizontal", 10.16, 2.2, 1.1, 5.2, 2.7),
        ("DO-201AD", "D_DO-201AD_P15.24mm_Horizontal", 15.24, 3.2, 1.6, 9.5, 5.3),
    ] {
        let mut f = axial(
            name,
            &format!("Diode, {pkg}, lying, {pitch} mm pitch, pad 1 is the cathode"),
            pitch,
            pad,
            drill,
            l,
            d,
        );
        // Cathode band.
        let x = -l / 2.0 + 0.15 * l;
        f.silk(Shape::line(Point::mm(x, -d / 2.0 - 0.12), Point::mm(x, d / 2.0 + 0.12), mm(0.12)));
        f.fab(Shape::line(Point::mm(x, -d / 2.0), Point::mm(x, d / 2.0), mm(0.1)));
        b.footprint(f.finish());
        out.push((pkg, name.to_string()));
    }
    out
}

fn diodes(b: &mut Builder) {
    let packages = diode_footprints(b);
    // Part and symbol name, symbol, description, value, first package.
    type Kind = (&'static str, fn() -> crate::library::Symbol, &'static str, &'static str, &'static str);
    let kinds: [Kind; 5] = [
        ("Diode", sym::diode, "General purpose rectifier or signal diode", "1N4148W", "SOD-123"),
        ("Diode_Schottky", sym::schottky, "Schottky diode", "B5819W", "SOD-123"),
        ("Diode_Zener", sym::zener, "Zener diode", "BZT52C3V3", "SOD-123"),
        ("TVS", sym::tvs, "Unidirectional TVS diode", "SMAJ5.0A", "SMA"),
        ("TVS_Bidirectional", sym::tvs_bidirectional, "Bidirectional TVS diode", "SMAJ5.0CA", "SMA"),
    ];
    for (name, symbol, description, value, first) in kinds {
        b.symbol(symbol());
        let mut variants: Vec<_> = packages.iter().map(|(pkg, fp)| variant(*pkg, fp.clone(), Map::ByNumber)).collect();
        let at = packages.iter().position(|(p, _)| *p == first).unwrap();
        let v = variants.remove(at);
        variants.insert(0, v);
        b.part(PartSpec {
            name: name.into(),
            description: format!("{description}: SOD, MiniMELF, SMA/SMB/SMC, DO-35/41/201AD"),
            prefix: "D",
            value: value.into(),
            symbol: Some(name),
            variants,
        });
    }
}

fn leds(b: &mut Builder) {
    b.symbol(sym::led());
    let mut variants = Vec::new();
    for size in ["0603", "0402", "0805", "1206"] {
        variants.push(variant(size, chip(b, "LED_", size, true), Map::ByNumber));
    }
    for d in [3.0, 5.0] {
        let name = format!("LED_D{d:.1}mm");
        let mut f = Fp::new(&name, format!("LED, Ø{d} mm through-hole, 2.54 mm pitch, pad 1 is the cathode"));
        f.tht("1", -1.27, 0.0, 1.8, 1.8, 0.9);
        f.tht("2", 1.27, 0.0, 1.8, 1.8, 0.9);
        f.fab(Shape::Circle { center: Point::ZERO, radius: mm(d / 2.0), width: mm(0.1), fill: false });
        // The flange with its flat side at the cathode.
        let r = (d / 2.0 + 0.4_f64).max(2.9);
        let w = mm(0.12);
        f.silk(Shape::Arc { center: Point::ZERO, radius: mm(r), start: Angle::deg(210.0), sweep: 300.0, width: w });
        let x = -r * 0.866;
        f.silk(Shape::line(Point::mm(x, -r / 2.0), Point::mm(x, r / 2.0), w));
        f.keep([-r, -r, r, r]);
        b.footprint(f.finish());
        variants.push(variant(format!("THT Ø{d} mm"), name, Map::ByNumber));
    }
    b.part(PartSpec {
        name: "LED".into(),
        description: "Light emitting diode: chip 0402 to 1206, Ø3 and Ø5 mm through-hole".into(),
        prefix: "D",
        value: "red".into(),
        symbol: Some("LED"),
        variants,
    });
}

/// SOT-23 (3 pins): 1 and 2 on the left, 3 on the right.
pub(super) fn sot23(b: &mut Builder) -> &'static str {
    let mut f = Fp::new("SOT-23", "SOT-23, 3 pins");
    for (n, x, y) in [("1", -1.1375, 0.95), ("2", -1.1375, -0.95), ("3", 1.1375, 0.0)] {
        f.smd(n, x, y, 1.325, 0.6);
    }
    f.outline([-0.65, -1.45, 0.65, 1.45], 0.11);
    b.footprint(f.finish());
    "SOT-23"
}

/// SOT-23-5: 1–3 on the left (top to bottom), 4–5 on the right (bottom to top).
pub(super) fn sot23_5(b: &mut Builder) -> &'static str {
    let mut f = Fp::new("SOT-23-5", "SOT-23-5, 0.95 mm pitch");
    for (n, x, y) in
        [("1", -1.1375, 0.95), ("2", -1.1375, 0.0), ("3", -1.1375, -0.95), ("4", 1.1375, -0.95), ("5", 1.1375, 0.95)]
    {
        f.smd(n, x, y, 1.325, 0.6);
    }
    f.outline([-0.8, -1.45, 0.8, 1.45], 0.11);
    f.silk_dot(-1.9, 1.65);
    f.keep([-2.05, 1.5, -1.75, 1.8]);
    b.footprint(f.finish());
    "SOT-23-5"
}

/// SOT-223: pins 1–3 on the left, the tab (pad "2" too, as pin 2) on the right.
pub(super) fn sot223(b: &mut Builder) -> &'static str {
    let name = "SOT-223-3_TabPin2";
    let mut f = Fp::new(name, "SOT-223, 3 pins, the tab is pin 2");
    for (n, y) in [("1", 2.3), ("2", 0.0), ("3", -2.3)] {
        f.smd(n, -3.15, y, 2.0, 1.5);
    }
    f.smd("2", 3.15, 0.0, 2.0, 3.8);
    f.outline([-1.75, -3.25, 1.75, 3.25], 0.11);
    b.footprint(f.finish());
    name
}

/// TO-92, three pins in line, 1.27 mm pitch, flat side down.
pub(super) fn to92(b: &mut Builder) -> &'static str {
    let name = "TO-92_Inline";
    let mut f = Fp::new(name, "TO-92, pins in line, 1.27 mm pitch");
    for (n, x) in ["1", "2", "3"].iter().zip(row(3, 1.27)) {
        f.tht(n, x, 0.0, 1.05, 1.5, 0.75);
    }
    let w = mm(0.12);
    f.silk(Shape::Arc { center: Point::ZERO, radius: mm(2.6), start: Angle::deg(-45.3), sweep: 270.6, width: w });
    f.silk_line(-1.83, -1.85, 1.83, -1.85);
    f.fab(Shape::line(Point::mm(-2.0, -1.6), Point::mm(2.0, -1.6), mm(0.1)));
    f.fab(Shape::Arc { center: Point::ZERO, radius: mm(2.4), start: Angle::deg(-41.8), sweep: 263.6, width: mm(0.1) });
    f.keep([-2.6, -1.85, 2.6, 2.6]);
    b.footprint(f.finish());
    name
}

/// TO-220-3 standing, 2.54 mm pitch, the tab at the top.
pub(super) fn to220(b: &mut Builder) -> &'static str {
    let name = "TO-220-3_Vertical";
    let mut f = Fp::new(name, "TO-220-3, standing, 2.54 mm pitch");
    for (n, x) in ["1", "2", "3"].iter().zip(row(3, 2.54)) {
        f.tht(n, x, 0.0, 1.9, 2.4, 1.1);
    }
    f.outline([-5.0, -1.6, 5.0, 3.0], 0.12);
    f.silk_line(-5.12, 1.7, 5.12, 1.7);
    b.footprint(f.finish());
    name
}

/// DPAK: pins 1 and 3 at the bottom, the tab (pin 2) at the top.
pub(super) fn dpak(b: &mut Builder) -> &'static str {
    let name = "TO-252-2";
    let mut f = Fp::new(name, "DPAK (TO-252-2), the tab is pin 2; generic land pattern");
    f.smd("1", -2.28, -3.9, 1.4, 2.6);
    f.smd("3", 2.28, -3.9, 1.4, 2.6);
    f.smd("2", 0.0, 2.3, 5.8, 6.2);
    f.outline([-3.3, -2.4, 3.3, 3.7], 0.11);
    b.footprint(f.finish());
    name
}

fn transistors(b: &mut Builder) {
    let (s23, s223, t92, t220, dp) = (sot23(b), sot223(b), to92(b), to220(b), dpak(b));
    const BJT_SOT23: &[(&str, &str)] = &[("1", "B"), ("2", "E"), ("3", "C")];
    const BJT_SOT223: &[(&str, &str)] = &[("1", "B"), ("2", "C"), ("3", "E")];
    const BJT_TO92: &[(&str, &str)] = &[("1", "E"), ("2", "B"), ("3", "C")];
    const FET_SOT23: &[(&str, &str)] = &[("1", "G"), ("2", "S"), ("3", "D")];
    const FET_GDS: &[(&str, &str)] = &[("1", "G"), ("2", "D"), ("3", "S")];
    for (pnp, value, others) in
        [(false, "BC847", "BCP56 in SOT-223, 2N3904 in TO-92"), (true, "BC857", "BCP53, 2N3906")]
    {
        let symbol = sym::bjt(pnp);
        let name = symbol.name.clone();
        b.symbol(symbol);
        b.part(PartSpec {
            name: name.clone(),
            description: format!("{name} bipolar transistor ({value} in SOT-23; {others})"),
            prefix: "Q",
            value: value.into(),
            symbol: Some(&name),
            variants: vec![
                variant("SOT-23", s23, Map::Pairs(BJT_SOT23)),
                variant("SOT-223", s223, Map::Pairs(BJT_SOT223)),
                variant("TO-92 (EBC)", t92, Map::Pairs(BJT_TO92)),
            ],
        });
    }
    for (p, value, others) in [
        (false, "2N7002", "AO3400 in SOT-23, IRLZ44N in TO-220"),
        (true, "BSS84", "AO3401 in SOT-23, IRF9540 in TO-220"),
    ] {
        let symbol = sym::mosfet(p);
        let name = symbol.name.clone();
        b.symbol(symbol);
        let kind = if p { "P-channel" } else { "N-channel" };
        b.part(PartSpec {
            name: name.clone(),
            description: format!("{kind} MOSFET ({value}; {others})"),
            prefix: "Q",
            value: value.into(),
            symbol: Some(&name),
            variants: vec![
                variant("SOT-23", s23, Map::Pairs(FET_SOT23)),
                variant("SOT-223", s223, Map::Pairs(FET_GDS)),
                variant("DPAK", dp, Map::Pairs(FET_GDS)),
                variant("TO-220", t220, Map::Pairs(FET_GDS)),
            ],
        });
    }
}
