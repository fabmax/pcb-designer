//! Resistors, capacitors, inductors, ferrite beads, fuses.

use super::fp::Fp;
use super::{Builder, Map, PartSpec, sym, variant};
use crate::geom::{Point, Shape};
use crate::units::mm;

/// Chip sizes: imperial and metric code, pad centre, pad width (along the part) and
/// height, body length and width (IPC-7351B nominal).
pub(super) const CHIPS: [(&str, &str, f64, f64, f64, f64, f64); 7] = [
    ("0402", "1005", 0.51, 0.54, 0.64, 1.0, 0.5),
    ("0603", "1608", 0.825, 0.8, 0.95, 1.6, 0.8),
    ("0805", "2012", 0.9125, 1.025, 1.4, 2.0, 1.25),
    ("1206", "3216", 1.4625, 1.125, 1.75, 3.2, 1.6),
    ("1210", "3225", 1.4625, 1.125, 2.65, 3.2, 2.5),
    ("1812", "4532", 2.1375, 1.275, 3.35, 4.5, 3.2),
    ("2512", "6332", 2.9625, 1.525, 3.35, 6.3, 3.2),
];

/// Two-pad SMD footprint: pads at ±`c`, body `bl` × `bw`. A polarity mark (a line left
/// of pad 1) when `polar`.
#[allow(clippy::too_many_arguments)]
pub(super) fn two_pad(name: &str, description: &str, c: f64, pw: f64, ph: f64, bl: f64, bw: f64, polar: bool) -> Fp {
    let mut f = Fp::new(name, description);
    f.smd("1", -c, 0.0, pw, ph);
    f.smd("2", c, 0.0, pw, ph);
    f.outline([-bl / 2.0, -bw / 2.0, bl / 2.0, bw / 2.0], 0.11);
    if polar {
        let x = -(c + pw / 2.0 + 0.3).max(bl / 2.0 + 0.2);
        let h = (ph / 2.0).max(bw / 2.0);
        f.silk(Shape::line(Point::mm(x, -h), Point::mm(x, h), mm(0.12)));
        f.keep([x - 0.1, -h, x, h]);
    }
    f
}

/// The chip footprint of `size` (imperial code), named `{prefix}{size}_{metric}Metric`.
pub(super) fn chip(b: &mut Builder, prefix: &str, size: &str, polar: bool) -> String {
    let &(imp, met, c, pw, ph, bl, bw) = CHIPS.iter().find(|c| c.0 == size).expect("a chip size");
    let name = format!("{prefix}{imp}_{met}Metric");
    let kind = if polar { "polarised chip (pad 1 marked)" } else { "chip" };
    let description = format!("SMD {kind}, {imp} ({met} metric), IPC-7351B nominal");
    b.footprint(two_pad(&name, &description, c, pw, ph, bl, bw, polar).finish());
    name
}

/// Two-terminal through-hole part lying flat: pads `pitch` apart, body `l` × `d`.
pub(super) fn axial(name: &str, description: &str, pitch: f64, pad: f64, drill: f64, l: f64, d: f64) -> Fp {
    let mut f = Fp::new(name, description);
    let x = pitch / 2.0;
    f.tht("1", -x, 0.0, pad, pad, drill);
    f.tht("2", x, 0.0, pad, pad, drill);
    f.outline([-l / 2.0, -d / 2.0, l / 2.0, d / 2.0], 0.12);
    for s in [-1.0, 1.0] {
        f.fab(Shape::line(Point::mm(s * x, 0.0), Point::mm(s * l / 2.0, 0.0), mm(0.1)));
        f.silk_line(s * x, 0.0, s * (l / 2.0 + 0.12), 0.0);
    }
    f
}

fn chip_variants(b: &mut Builder, prefix: &str, sizes: &[&str]) -> Vec<(String, String)> {
    sizes.iter().map(|s| (s.to_string(), chip(b, prefix, s, false))).collect()
}

pub(super) fn add(b: &mut Builder) {
    resistors(b);
    capacitors(b);
    polarized(b);
    inductors(b);
    beads_and_fuses(b);
}

fn resistors(b: &mut Builder) {
    b.symbol(sym::resistor());
    let mut v = chip_variants(b, "Chip_", &["0603", "0402", "0805", "1206", "2512"]);
    let axial_name = "R_Axial_DIN0207_L6.3mm_D2.5mm_P10.16mm_Horizontal";
    let description = "Resistor, axial DIN0207 (1/4 W), lying, 10.16 mm pitch";
    b.footprint(axial(axial_name, description, 10.16, 1.6, 0.8, 6.3, 2.5).finish());
    v.push(("Axial DIN0207, 10.16 mm".into(), axial_name.into()));
    let standing = "R_Axial_DIN0207_L6.3mm_D2.5mm_P2.54mm_Vertical";
    let mut f = Fp::new(standing, "Resistor, axial DIN0207 (1/4 W), standing, 2.54 mm pitch");
    f.tht("1", -1.27, 0.0, 1.6, 1.6, 0.8);
    f.tht("2", 1.27, 0.0, 1.6, 1.6, 0.8);
    f.fab(Shape::Circle { center: Point::mm(-1.27, 0.0), radius: mm(1.25), width: mm(0.1), fill: false });
    f.silk_circle(-1.27, 0.0, 1.37);
    f.keep([-2.64, -1.37, 0.1, 1.37]);
    b.footprint(f.finish());
    v.push(("Axial DIN0207, standing 2.54 mm".into(), standing.into()));
    let variants = v.into_iter().map(|(n, f)| variant(n, f, Map::ByNumber)).collect();
    b.part(PartSpec {
        name: "Resistor".into(),
        category: "Passives",
        description: "Resistor: chip 0402 to 2512, axial through-hole".into(),
        prefix: "R",
        value: "10k".into(),
        symbol: Some("Resistor"),
        variants,
    });
}

fn capacitors(b: &mut Builder) {
    b.symbol(sym::capacitor());
    let mut v = chip_variants(b, "Chip_", &["0603", "0402", "0805", "1206", "1210"]);
    for (pitch, d) in [(2.54, 5.0), (5.08, 7.5)] {
        let name = format!("C_Disc_D{d:.1}mm_W2.5mm_P{pitch:.2}mm");
        let mut f = Fp::new(&name, format!("Ceramic disc capacitor, {pitch} mm pitch"));
        f.tht("1", -pitch / 2.0, 0.0, 1.6, 1.6, 0.8);
        f.tht("2", pitch / 2.0, 0.0, 1.6, 1.6, 0.8);
        f.outline([-d / 2.0, -1.25, d / 2.0, 1.25], 0.12);
        b.footprint(f.finish());
        v.push((format!("Disc, {pitch} mm"), name));
    }
    let variants = v.into_iter().map(|(n, f)| variant(n, f, Map::ByNumber)).collect();
    b.part(PartSpec {
        name: "Capacitor".into(),
        category: "Passives",
        description: "Ceramic capacitor: chip 0402 to 1210, through-hole disc".into(),
        prefix: "C",
        value: "100n".into(),
        symbol: Some("Capacitor"),
        variants,
    });
}

fn polarized(b: &mut Builder) {
    b.symbol(sym::capacitor_polarized());
    let mut v: Vec<(String, String)> = Vec::new();
    // Tantalum: case, EIA code, pad centre, pad w × h, body l × w. Pad 1 is the anode.
    for (case, eia, c, pw, ph, bl, bw) in [
        ("A", "3216-18", 1.35, 1.4, 1.3, 3.2, 1.6),
        ("B", "3528-21", 1.5, 1.45, 2.35, 3.5, 2.8),
        ("C", "6032-28", 2.45, 2.25, 2.35, 6.0, 3.2),
        ("D", "7343-31", 3.1, 2.25, 2.55, 7.3, 4.3),
    ] {
        let name = format!("CP_Tantalum_Case-{case}_EIA-{eia}");
        let description = format!("Tantalum capacitor, case {case} (EIA {eia}), pad 1 is +");
        b.footprint(two_pad(&name, &description, c, pw, ph, bl, bw, true).finish());
        v.push((format!("Tantalum {case} ({})", &eia[..4]), name));
    }
    // SMD electrolytic: diameter, height, pad centre, pad w × h.
    for (d, h, c, pw, ph) in [
        (4.0, 5.4, 1.8, 2.4, 1.6),
        (5.0, 5.4, 2.2, 2.6, 1.6),
        (6.3, 5.4, 2.7, 3.0, 1.6),
        (8.0, 10.0, 3.45, 3.5, 2.5),
        (10.0, 10.0, 4.25, 4.0, 2.5),
    ] {
        let name = format!("CP_Elec_{d}x{h}");
        let description = format!(
            "SMD aluminium electrolytic capacitor, Ø{d} × {h} mm, pad 1 is +; generic land pattern, \
             check the capacitor's datasheet"
        );
        let s = d / 2.0 + 0.15;
        let mut f = two_pad(&name, &description, c, pw, ph, 2.0 * s, 2.0 * s, false);
        f.silk_plus(-s - 0.7, s - 0.2, 0.8);
        f.keep([-s - 1.1, 0.0, -s, s + 0.2]);
        b.footprint(f.finish());
        v.push((format!("Electrolytic SMD Ø{d} × {h}"), name));
    }
    // Radial through-hole: diameter, pitch, pad, drill.
    for (d, pitch, pad, drill) in
        [(5.0, 2.0, 1.6, 0.8), (6.3, 2.5, 1.6, 0.8), (8.0, 3.5, 1.6, 0.8), (10.0, 5.0, 2.0, 1.0)]
    {
        let name = format!("CP_Radial_D{d:.1}mm_P{pitch:.2}mm");
        let mut f = Fp::new(&name, format!("Radial electrolytic capacitor, Ø{d} mm, {pitch} mm pitch, pad 1 is +"));
        f.tht("1", -pitch / 2.0, 0.0, pad, pad, drill);
        f.tht("2", pitch / 2.0, 0.0, pad, pad, drill);
        let r = d / 2.0;
        f.fab(Shape::Circle { center: Point::ZERO, radius: mm(r), width: mm(0.1), fill: false });
        f.silk_circle(0.0, 0.0, r + 0.12);
        f.silk_plus(-r - 0.6, r - 0.4, 0.8);
        f.keep([-r - 1.0, -r - 0.12, r + 0.12, r + 0.12]);
        b.footprint(f.finish());
        v.push((format!("Radial Ø{d}, {pitch} mm"), name));
    }
    let variants = v.into_iter().map(|(n, f)| variant(n, f, Map::ByNumber)).collect();
    b.part(PartSpec {
        name: "Capacitor_Polarized".into(),
        category: "Passives",
        description: "Polarised capacitor (tantalum, electrolytic); pin 1 is +".into(),
        prefix: "C",
        value: "10u".into(),
        symbol: Some("Capacitor_Polarized"),
        variants,
    });
}

fn inductors(b: &mut Builder) {
    b.symbol(sym::inductor());
    let mut v = chip_variants(b, "Chip_", &["0603", "0805", "1210"]);
    // Shielded power inductors: body size, pad centre, pad w × h.
    for (s, c, pw, ph) in [
        (3.0, 1.2, 1.0, 2.7),
        (4.0, 1.55, 1.3, 3.6),
        (5.0, 1.95, 1.5, 4.4),
        (6.0, 2.3, 1.9, 5.4),
        (10.0, 3.9, 2.6, 5.2),
    ] {
        let name = format!("L_Shielded_{s}x{s}mm");
        let description = format!(
            "Shielded SMD power inductor, {s} × {s} mm body; generic land pattern, check the \
             inductor's datasheet"
        );
        b.footprint(two_pad(&name, &description, c, pw, ph, s, s, false).finish());
        v.push((format!("Power, {s} × {s} mm"), name));
    }
    let variants = v.into_iter().map(|(n, f)| variant(n, f, Map::ByNumber)).collect();
    b.part(PartSpec {
        name: "Inductor".into(),
        category: "Passives",
        description: "Inductor: chip 0603 to 1210, shielded SMD power inductors".into(),
        prefix: "L",
        value: "10u".into(),
        symbol: Some("Inductor"),
        variants,
    });
}

fn beads_and_fuses(b: &mut Builder) {
    b.symbol(sym::ferrite_bead());
    let v = chip_variants(b, "Chip_", &["0603", "0402", "0805"]);
    b.part(PartSpec {
        name: "FerriteBead".into(),
        category: "Passives",
        description: "Ferrite bead, chip 0402 to 0805".into(),
        prefix: "FB",
        value: "600R@100MHz".into(),
        symbol: Some("FerriteBead"),
        variants: v.into_iter().map(|(n, f)| variant(n, f, Map::ByNumber)).collect(),
    });
    b.symbol(sym::fuse());
    let v = chip_variants(b, "Chip_", &["1206", "0603", "1812"]);
    b.part(PartSpec {
        name: "Fuse".into(),
        category: "Passives",
        description: "Fuse or resettable PTC fuse, chip 0603 to 1812".into(),
        prefix: "F",
        value: "500mA".into(),
        symbol: Some("Fuse"),
        variants: v.into_iter().map(|(n, f)| variant(n, f, Map::ByNumber)).collect(),
    });
}
