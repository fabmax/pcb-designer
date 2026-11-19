//! Crystals, test points, push buttons, mounting holes, fiducials, and the IC packages
//! of the hand-made parts (SOIC, DIP, SOT-23-5).

use super::fp::{Fp, row};
use super::semis::sot23_5;
use super::{Builder, Map, PartSpec, sym, variant};
use crate::geom::{Angle, Point, Shape};
use crate::library::PadShape;
use crate::units::mm;

pub(super) fn add(b: &mut Builder) {
    crystals(b);
    test_points(b);
    switches(b);
    mounting_holes(b);
    packages(b);
}

fn crystals(b: &mut Builder) {
    let hc49 = "Crystal_HC49-U_Vertical";
    let mut f = Fp::new(hc49, "Crystal, HC-49/US, 4.88 mm pitch");
    f.tht("1", -2.44, 0.0, 1.5, 1.5, 0.8);
    f.tht("2", 2.44, 0.0, 1.5, 1.5, 0.8);
    let (r, c, w) = (2.325, 3.2, mm(0.12));
    f.fab(Shape::rect(Point::mm(-c - r, -r), Point::mm(c + r, r), mm(0.1)));
    let rs = r + 0.12;
    for s in [-1.0, 1.0] {
        f.silk(Shape::line(Point::mm(-c, s * rs), Point::mm(c, s * rs), w));
    }
    f.silk(Shape::Arc { center: Point::mm(c, 0.0), radius: mm(rs), start: Angle::deg(-90.0), sweep: 180.0, width: w });
    f.silk(Shape::Arc { center: Point::mm(-c, 0.0), radius: mm(rs), start: Angle::deg(90.0), sweep: 180.0, width: w });
    f.keep([-c - rs, -rs, c + rs, rs]);
    b.footprint(f.finish());
    // Four pads counterclockwise from the bottom left: 1 and 3 the crystal, 2 and 4 the
    // case.
    let mut four = Vec::new();
    for (name, size, (x, y), (pw, ph)) in [
        ("Crystal_SMD_3225-4Pin_3.2x2.5mm", "3.2 × 2.5", (1.1, 0.8), (1.4, 1.15)),
        ("Crystal_SMD_2016-4Pin_2.0x1.6mm", "2.0 × 1.6", (0.7, 0.55), (0.9, 0.8)),
    ] {
        let mut f = Fp::new(name, format!("Crystal, SMD {size} mm, 4 pads (2 and 4 are the case)"));
        for (n, sx, sy) in [("1", -1.0, -1.0), ("2", 1.0, -1.0), ("3", 1.0, 1.0), ("4", -1.0, 1.0)] {
            f.smd(n, sx * x, sy * y, pw, ph);
        }
        let (bl, bw) = if size.starts_with('3') { (3.2, 2.5) } else { (2.0, 1.6) };
        f.outline([-bl / 2.0, -bw / 2.0, bl / 2.0, bw / 2.0], 0.11);
        let (dx, dy) = (-x - pw / 2.0 - 0.4, -y - ph / 2.0);
        f.silk_dot(dx, dy);
        f.keep([dx - 0.15, dy - 0.15, dx, dy]);
        b.footprint(f.finish());
        four.push((size, name));
    }
    b.symbol(sym::crystal());
    const XTAL_ON_FOUR: &[(&str, &str)] = &[("1", "1"), ("3", "2")];
    let mut variants = vec![variant("HC-49/US", hc49, Map::ByNumber)];
    for (size, name) in &four {
        variants.push(variant(format!("SMD {size} mm"), *name, Map::Pairs(XTAL_ON_FOUR)));
    }
    b.part(PartSpec {
        name: "Crystal".into(),
        description: "Quartz crystal, two pins (on 4-pad packages the case pads stay open)".into(),
        prefix: "Y",
        value: "16MHz".into(),
        symbol: Some("Crystal"),
        variants,
    });
    b.symbol(sym::crystal_gnd24());
    b.part(PartSpec {
        name: "Crystal_GND24".into(),
        description: "Quartz crystal, 4 pads: 1 and 3 the crystal, 2 and 4 the case (ground)".into(),
        prefix: "Y",
        value: "16MHz".into(),
        symbol: Some("Crystal_GND24"),
        variants: four.iter().map(|(size, name)| variant(format!("SMD {size} mm"), *name, Map::ByNumber)).collect(),
    });
}

fn test_points(b: &mut Builder) {
    let smd = "TestPoint_Pad_D1.5mm";
    let mut f = Fp::new(smd, "Test point, SMD pad Ø1.5 mm");
    let pad = f.smd("1", 0.0, 0.0, 1.5, 1.5);
    pad.shape = PadShape::Circle;
    pad.paste = false;
    b.footprint(f.finish());
    let tht = "TestPoint_THTPad_D2.0mm_Drill1.0mm";
    let mut f = Fp::new(tht, "Test point, through-hole pad Ø2.0 mm, 1.0 mm drill");
    f.tht_round("1", 0.0, 0.0, 2.0, 1.0);
    b.footprint(f.finish());
    b.symbol(sym::test_point());
    b.part(PartSpec {
        name: "TestPoint".into(),
        description: "Test point: SMD pad or through-hole pad".into(),
        prefix: "TP",
        value: "TestPoint".into(),
        symbol: Some("TestPoint"),
        variants: vec![variant("SMD pad Ø1.5 mm", smd, Map::ByNumber), variant("THT pad Ø2.0 mm", tht, Map::ByNumber)],
    });
}

fn switches(b: &mut Builder) {
    let name = "SW_PUSH_6mm";
    let mut f = Fp::new(name, "Tactile switch, 6 × 6 mm, through-hole; pads 1 and 2 come in pairs");
    for (n, y) in [("1", 2.25), ("2", -2.25)] {
        for x in [-3.25, 3.25] {
            f.tht_round(n, x, y, 2.0, 1.1);
        }
    }
    f.outline([-3.0, -3.0, 3.0, 3.0], 0.12);
    f.silk_circle(0.0, 0.0, 1.75);
    b.footprint(f.finish());
    b.symbol(sym::push_button());
    b.part(PartSpec {
        name: "SW_Push".into(),
        description: "Push button, normally open".into(),
        prefix: "SW",
        value: "SW_Push".into(),
        symbol: Some("SW_Push"),
        variants: vec![variant("Tactile 6 × 6 mm THT", name, Map::ByNumber)],
    });
}

/// Board parts: mounting holes with and without a plated pad, a fiducial.
fn mounting_holes(b: &mut Builder) {
    for (m, drill, head) in [("M2", 2.2, 4.0), ("M2.5", 2.7, 5.0), ("M3", 3.2, 6.0), ("M4", 4.3, 8.0)] {
        let r = head / 2.0;
        let pad_name = format!("MountingHole_{drill}mm_{m}_Pad");
        let mut f = Fp::new(&pad_name, format!("Mounting hole for {m} screws, plated, Ø{head} mm pad"));
        f.tht_round("1", 0.0, 0.0, head, drill);
        b.footprint(f.finish());
        let plain = format!("MountingHole_{drill}mm_{m}");
        let mut f = Fp::new(&plain, format!("Mounting hole for {m} screws, not plated"));
        f.npth(0.0, 0.0, drill);
        f.fab(Shape::Circle { center: Point::ZERO, radius: mm(r), width: mm(0.1), fill: false });
        f.keep([-r, -r, r, r]);
        b.footprint(f.finish());
        b.part(PartSpec {
            name: format!("MountingHole_{m}"),
            description: format!("Mounting hole for {m} screws (board part: placed on the board only)"),
            prefix: "H",
            value: "MountingHole".into(),
            symbol: None,
            variants: vec![variant("Plated pad", pad_name, Map::ByNumber), variant("Plain hole", plain, Map::ByNumber)],
        });
    }
    let name = "Fiducial_1mm_Mask2mm";
    let mut f = Fp::new(name, "Fiducial, Ø1 mm copper in a Ø2 mm mask opening");
    let pad = f.smd("", 0.0, 0.0, 1.0, 1.0);
    pad.shape = PadShape::Circle;
    pad.paste = false;
    pad.mask_margin = Some(mm(0.5));
    f.keep([-1.0, -1.0, 1.0, 1.0]);
    b.footprint(f.finish());
    b.part(PartSpec {
        name: "Fiducial".into(),
        description: "Fiducial mark for assembly (board part: placed on the board only)".into(),
        prefix: "FID",
        value: "Fiducial".into(),
        symbol: None,
        variants: vec![variant("Ø1 mm", name, Map::ByNumber)],
    });
}

/// Dual-row IC package: pins 1…n/2 down the left side, the rest up the right.
fn dual_row(f: &mut Fp, n: usize, x: f64, pitch: f64, pad: &mut dyn FnMut(&mut Fp, &str, f64, f64)) -> f64 {
    let ys: Vec<f64> = row(n / 2, pitch).collect();
    for i in 0..n {
        let (px, py) = if i < n / 2 { (-x, -ys[i]) } else { (x, ys[i - n / 2]) };
        pad(f, &(i + 1).to_string(), px, py);
    }
    -ys[0]
}

fn packages(b: &mut Builder) {
    sot23_5(b);
    for (n, length) in [(8, 4.9), (14, 8.65)] {
        let name = format!("SOIC-{n}_3.9x{:.1}mm_P1.27mm", length);
        let mut f = Fp::new(&name, format!("SOIC-{n}, 3.9 mm body, 1.27 mm pitch"));
        let y1 = dual_row(&mut f, n, 2.475, 1.27, &mut |f, name, x, y| {
            f.smd(name, x, y, 1.95, 0.6);
        });
        f.outline([-1.95, -length / 2.0, 1.95, length / 2.0], 0.11);
        f.silk_dot(-4.0, y1);
        f.keep([-4.15, y1 - 0.15, -3.85, y1 + 0.15]);
        b.footprint(f.finish());
    }
    for n in [8, 14] {
        let name = format!("DIP-{n}_W7.62mm");
        let mut f = Fp::new(&name, format!("DIP-{n}, 7.62 mm row spacing, 2.54 mm pitch"));
        dual_row(&mut f, n, 3.81, 2.54, &mut |f, name, x, y| {
            f.tht(name, x, y, 1.6, 1.6, 0.8);
        });
        let half = (n / 2) as f64 * 1.27 + 0.3;
        f.body([-3.175, -half, 3.175, half]);
        f.silk_rect([-2.6, -half - 0.12, 2.6, half + 0.12]);
        let w = mm(0.12);
        f.silk(Shape::Arc {
            center: Point::mm(0.0, half + 0.12),
            radius: mm(1.0),
            start: Angle::DEG180,
            sweep: 180.0,
            width: w,
        });
        b.footprint(f.finish());
    }
}
