//! Built-in test parts for the spike.

use crate::model::{Nm, PadDef, PadShape, PartDef, PinDef, Pt};

const G: f64 = 2.54; // schematic grid (100 mil)

fn mm(v: f64) -> Nm {
    (v * 1_000_000.0).round() as Nm
}

fn rect_lines(min: (f64, f64), max: (f64, f64)) -> Vec<(Pt, Pt)> {
    let (a, b, c, d) = (Pt::mm(min.0, min.1), Pt::mm(max.0, min.1), Pt::mm(max.0, max.1), Pt::mm(min.0, max.1));
    vec![(a, b), (b, c), (c, d), (d, a)]
}

/// Two-pin passive (0603).
fn passive(name: &str, prefix: &str) -> PartDef {
    PartDef {
        name: name.into(),
        prefix: prefix.into(),
        body: (Pt::mm(-G, -0.9), Pt::mm(G, 0.9)),
        pins: vec![
            PinDef { pos: Pt::mm(-2.0 * G, 0.0), inner: Pt::mm(-G, 0.0) },
            PinDef { pos: Pt::mm(2.0 * G, 0.0), inner: Pt::mm(G, 0.0) },
        ],
        pads: [-0.8, 0.8]
            .iter()
            .map(|&x| PadDef {
                pos: Pt::mm(x, 0.0),
                size: Pt::mm(0.95, 1.0),
                shape: PadShape::RoundRect(0.25),
                drill: 0,
            })
            .collect(),
        silk: vec![(Pt::mm(-0.2, 0.75), Pt::mm(0.2, 0.75)), (Pt::mm(-0.2, -0.75), Pt::mm(0.2, -0.75))],
        silk_dots: vec![],
        pin_to_pad: vec![0, 1],
        board_pitch: Pt::mm(3.4, 2.6),
    }
}

/// Dual row IC symbol with `n` pins (n/2 per side, DIP numbering).
fn dual_symbol(n: usize) -> (Vec<PinDef>, (Pt, Pt)) {
    let per_side = n / 2;
    let top = (per_side as f64 - 1.0) * 0.5 * G;
    let body_x = 3.0 * G;
    let mut pins = Vec::with_capacity(n);
    for i in 0..per_side {
        let y = top - i as f64 * G;
        pins.push(PinDef { pos: Pt::mm(-body_x - G, y), inner: Pt::mm(-body_x, y) });
    }
    for i in 0..per_side {
        let y = -top + i as f64 * G;
        pins.push(PinDef { pos: Pt::mm(body_x + G, y), inner: Pt::mm(body_x, y) });
    }
    (pins, (Pt::mm(-body_x, -top - G), Pt::mm(body_x, top + G)))
}

fn soic8() -> PartDef {
    let (pins, body) = dual_symbol(8);
    let mut pads = Vec::new();
    for i in 0..4 {
        pads.push(PadDef {
            pos: Pt::mm(-2.7, 1.905 - i as f64 * 1.27),
            size: Pt::mm(1.55, 0.6),
            shape: PadShape::RoundRect(0.25),
            drill: 0,
        });
    }
    for i in 0..4 {
        pads.push(PadDef {
            pos: Pt::mm(2.7, -1.905 + i as f64 * 1.27),
            size: Pt::mm(1.55, 0.6),
            shape: PadShape::RoundRect(0.25),
            drill: 0,
        });
    }
    PartDef {
        name: "SOIC-8".into(),
        prefix: "U".into(),
        body,
        pins,
        pads,
        silk: rect_lines((-1.95, -2.45), (1.95, 2.45)),
        silk_dots: vec![(Pt::mm(-3.4, 2.7), mm(0.2))],
        pin_to_pad: (0..8).collect(),
        board_pitch: Pt::mm(8.4, 6.8),
    }
}

fn qfp48() -> PartDef {
    let (pins, body) = dual_symbol(48);
    let mut pads = Vec::new();
    let (d, long, short) = (4.15, 1.5, 0.3);
    for side in 0..4 {
        for i in 0..12 {
            let t = (5.5 - i as f64) * 0.5;
            let (pos, size) = match side {
                0 => ((-d, t), (long, short)),
                1 => ((-t, -d), (short, long)),
                2 => ((d, -t), (long, short)),
                _ => ((t, d), (short, long)),
            };
            pads.push(PadDef {
                pos: Pt::mm(pos.0, pos.1),
                size: Pt::mm(size.0, size.1),
                shape: PadShape::RoundRect(0.25),
                drill: 0,
            });
        }
    }
    PartDef {
        name: "LQFP-48".into(),
        prefix: "U".into(),
        body,
        pins,
        pads,
        silk: rect_lines((-3.6, -3.6), (3.6, 3.6)),
        silk_dots: vec![(Pt::mm(-2.9, 2.9), mm(0.3))],
        pin_to_pad: (0..48).collect(),
        board_pitch: Pt::mm(11.5, 11.5),
    }
}

fn header10() -> PartDef {
    let n = 10;
    let top = (n as f64 - 1.0) * 0.5 * G;
    let pins = (0..n)
        .map(|i| {
            let y = top - i as f64 * G;
            PinDef { pos: Pt::mm(-2.0 * G, y), inner: Pt::mm(-G, y) }
        })
        .collect();
    let pads = (0..n)
        .map(|i| PadDef {
            pos: Pt::mm(0.0, top - i as f64 * 2.54),
            size: Pt::mm(1.7, 1.7),
            shape: if i == 0 { PadShape::Rect } else { PadShape::Circle },
            drill: mm(1.0),
        })
        .collect();
    PartDef {
        name: "PinHeader_1x10".into(),
        prefix: "J".into(),
        body: (Pt::mm(-G, -top - G), Pt::mm(G, top + G)),
        pins,
        pads,
        silk: rect_lines((-1.33, -top - 1.33), (1.33, top + 1.33)),
        silk_dots: vec![],
        pin_to_pad: (0..10).collect(),
        board_pitch: Pt::mm(4.2, 27.0),
    }
}

pub const PART_R: u16 = 0;
pub const PART_C: u16 = 1;
pub const PART_SOIC8: u16 = 2;
pub const PART_QFP48: u16 = 3;
pub const PART_HDR10: u16 = 4;

pub fn standard_parts() -> Vec<PartDef> {
    vec![passive("R_0603", "R"), passive("C_0603", "C"), soic8(), qfp48(), header10()]
}
