//! Built-in built-in library. Ids are fixed so that projects can later be updated from
//! newer versions of the same parts.
//!
//! Text fields may contain `${REF}` and `${VALUE}`, which are replaced by the component's
//! reference designator and value when drawn.

use crate::geom::{Angle, HAlign, Point, Shape, Size, Text, VAlign};
use crate::id::Id;
use crate::library::*;
use crate::store::Store;
use crate::units::{Coord, mm};

/// Deterministic id in the built-in namespace.
const fn bid(n: u128) -> Id {
    Id::from_u128(0x5ce1_0000_0000_4000_8000_0000_0000_0000 | n)
}

pub const LIBRARY_ID: Id = bid(1);
pub const PART_RESISTOR: Id = bid(0x100);
pub const PART_CAPACITOR: Id = bid(0x200);
pub const PART_LED: Id = bid(0x300);
pub const PART_HEADER_1X4: Id = bid(0x400);
pub const PART_DIODE: Id = bid(0x500);
pub const PART_NPN: Id = bid(0x600);
pub const PART_OPAMP: Id = bid(0x700);
pub const PART_MOUNTING_HOLE: Id = bid(0x800);
pub const PART_74HC00: Id = bid(0x900);

const LINE: Coord = 254_000; // 0.254 mm symbol line width
const G: f64 = 2.54;

fn pin(id: Id, name: &str, pos: Point, direction: Angle, length: Coord) -> Pin {
    Pin {
        id,
        name: name.into(),
        number: String::new(),
        pos,
        direction,
        length,
        kind: PinKind::Passive,
        show_name: false,
        show_number: true,
    }
}

fn ref_value_texts(top: f64, bottom: f64) -> Vec<Text> {
    vec![
        Text::new("${REF}", Point::mm(0.0, top), mm(1.27)).aligned(HAlign::Center, VAlign::Bottom),
        Text::new("${VALUE}", Point::mm(0.0, bottom), mm(1.27)).aligned(HAlign::Center, VAlign::Top),
    ]
}

fn two_pin_symbol(id: Id, name: &str, pins: [(Id, &str); 2], body: Vec<Shape>, inner: f64) -> Symbol {
    let len = mm(2.0 * G - inner);
    Symbol {
        id,
        name: name.into(),
        graphics: body,
        texts: ref_value_texts(1.4, -1.4),
        pins: [
            pin(pins[0].0, pins[0].1, Point::mm(-2.0 * G, 0.0), Angle::ZERO, len),
            pin(pins[1].0, pins[1].1, Point::mm(2.0 * G, 0.0), Angle::DEG180, len),
        ]
        .into_iter()
        .collect(),
    }
}

/// Two-pad SMD chip footprint.
fn chip(id: Id, name: &str, pad_ids: [Id; 2], pad_x: f64, pad: Size, body: (f64, f64)) -> Footprint {
    let pads = [(pad_ids[0], "1", -pad_x), (pad_ids[1], "2", pad_x)]
        .into_iter()
        .map(|(id, n, x)| Pad {
            back: false,
            mask: true,
            paste: true,
            id,
            name: n.into(),
            pos: Point::mm(x, 0.0),
            rotation: Angle::ZERO,
            size: pad,
            shape: PadShape::RoundRect { ratio: 0.25 },
            kind: PadKind::Smd,
        })
        .collect();
    let (bw, bh) = (body.0 / 2.0, body.1 / 2.0);
    let court_x = pad_x + crate::units::to_mm(pad.w) / 2.0 + 0.25;
    let court_y = bh.max(crate::units::to_mm(pad.h) / 2.0) + 0.25;
    let silk_y = bh + 0.12;
    Footprint {
        id,
        name: name.into(),
        description: String::new(),
        pads,
        graphics: vec![
            FootprintGraphic {
                back: false,
                layer: FootprintLayer::Silkscreen,
                shape: Shape::line(Point::mm(-bw * 0.3, silk_y), Point::mm(bw * 0.3, silk_y), mm(0.12)),
            },
            FootprintGraphic {
                back: false,
                layer: FootprintLayer::Silkscreen,
                shape: Shape::line(Point::mm(-bw * 0.3, -silk_y), Point::mm(bw * 0.3, -silk_y), mm(0.12)),
            },
            FootprintGraphic {
                back: false,
                layer: FootprintLayer::Fabrication,
                shape: Shape::rect(Point::mm(-bw, -bh), Point::mm(bw, bh), mm(0.1)),
            },
            FootprintGraphic {
                back: false,
                layer: FootprintLayer::Courtyard,
                shape: Shape::rect(Point::mm(-court_x, -court_y), Point::mm(court_x, court_y), mm(0.05)),
            },
        ],
        texts: vec![FootprintText {
            back: false,
            layer: FootprintLayer::Silkscreen,
            text: Text::new("${REF}", Point::mm(0.0, court_y + 0.2), mm(1.0))
                .aligned(HAlign::Center, VAlign::Bottom)
                .with_bold(true),
        }],
    }
}

fn connections(pads: [Id; 2], pins: [Id; 2]) -> Vec<PadConnection> {
    pads.iter().zip(pins).map(|(&pad, pin)| PadConnection { pad, unit: 0, pin }).collect()
}

pub fn builtin_library() -> Library {
    let mut lib = Library {
        id: Some(LIBRARY_ID),
        name: "Built-in".into(),
        symbols: Store::default(),
        footprints: Store::default(),
        parts: Store::default(),
    };

    // Footprints shared by resistors and capacitors.
    let fp0603 = chip(bid(0x1000), "0603", [bid(0x1001), bid(0x1002)], 0.8, Size::mm(0.95, 1.0), (1.6, 0.8));
    let fp0805 = chip(bid(0x1100), "0805", [bid(0x1101), bid(0x1102)], 0.95, Size::mm(1.0, 1.45), (2.0, 1.25));
    let led0603 = chip(bid(0x1200), "LED_0603", [bid(0x1201), bid(0x1202)], 0.8, Size::mm(0.95, 1.0), (1.6, 0.8));

    // Resistor.
    let r_pins = [bid(0x111), bid(0x112)];
    lib.symbols.insert(two_pin_symbol(
        bid(0x110),
        "Resistor",
        [(r_pins[0], "1"), (r_pins[1], "2")],
        vec![Shape::rect(Point::mm(-G, -0.9), Point::mm(G, 0.9), LINE)],
        G,
    ));
    lib.parts.insert(Part {
        id: PART_RESISTOR,
        name: "Resistor".into(),
        description: "Chip resistor".into(),
        prefix: "R".into(),
        default_value: "10k".into(),
        units: vec![PartUnit { name: "A".into(), symbol: bid(0x110) }],
        variants: vec![
            PartVariant {
                id: bid(0x120),
                name: "0603".into(),
                footprint: Some(fp0603.id),
                connections: connections([bid(0x1001), bid(0x1002)], r_pins),
            },
            PartVariant {
                id: bid(0x121),
                name: "0805".into(),
                footprint: Some(fp0805.id),
                connections: connections([bid(0x1101), bid(0x1102)], r_pins),
            },
        ],
        properties: Default::default(),
        source: None,
    });

    // Capacitor.
    let c_pins = [bid(0x211), bid(0x212)];
    let plate = |x: f64| Shape::line(Point::mm(x, -2.0), Point::mm(x, 2.0), mm(0.4));
    lib.symbols.insert(two_pin_symbol(
        bid(0x210),
        "Capacitor",
        [(c_pins[0], "1"), (c_pins[1], "2")],
        vec![plate(-0.5), plate(0.5)],
        0.5,
    ));
    lib.parts.insert(Part {
        id: PART_CAPACITOR,
        name: "Capacitor".into(),
        description: "Ceramic chip capacitor".into(),
        prefix: "C".into(),
        default_value: "100n".into(),
        units: vec![PartUnit { name: "A".into(), symbol: bid(0x210) }],
        variants: vec![PartVariant {
            id: bid(0x220),
            name: "0603".into(),
            footprint: Some(fp0603.id),
            connections: connections([bid(0x1001), bid(0x1002)], c_pins),
        }],
        properties: Default::default(),
        source: None,
    });

    // LED: pin names A/K, pad 1 is the cathode.
    let (anode, cathode) = (bid(0x311), bid(0x312));
    let tri = Shape::Polygon {
        points: vec![Point::mm(-1.0, 1.2), Point::mm(-1.0, -1.2), Point::mm(1.0, 0.0)],
        width: LINE,
        fill: false,
    };
    let bar = Shape::line(Point::mm(1.0, 1.2), Point::mm(1.0, -1.2), LINE);
    lib.symbols.insert(two_pin_symbol(bid(0x310), "LED", [(anode, "A"), (cathode, "K")], vec![tri, bar], 1.0));
    lib.parts.insert(Part {
        id: PART_LED,
        name: "LED".into(),
        description: "Chip LED".into(),
        prefix: "D".into(),
        default_value: "red".into(),
        units: vec![PartUnit { name: "A".into(), symbol: bid(0x310) }],
        variants: vec![PartVariant {
            id: bid(0x320),
            name: "0603".into(),
            footprint: Some(led0603.id),
            connections: connections([bid(0x1202), bid(0x1201)], [anode, cathode]),
        }],
        properties: Default::default(),
        source: None,
    });

    // 1x4 pin header.
    let n = 4;
    let top = (n as f64 - 1.0) * 0.5 * G;
    let hdr_pins: Vec<Id> = (0..n).map(|i| bid(0x411 + i as u128)).collect();
    let hdr_pads: Vec<Id> = (0..n).map(|i| bid(0x1301 + i as u128)).collect();
    lib.symbols.insert(Symbol {
        id: bid(0x410),
        name: "Conn_01x04".into(),
        graphics: vec![Shape::rect(Point::mm(-G, -top - G), Point::mm(G, top + G), LINE)],
        texts: ref_value_texts(top + G + 0.5, -top - G - 0.5),
        pins: (0..n)
            .map(|i| {
                pin(hdr_pins[i], &format!("{}", i + 1), Point::mm(-2.0 * G, top - i as f64 * G), Angle::ZERO, mm(G))
            })
            .collect(),
    });
    let header_fp = Footprint {
        id: bid(0x1300),
        name: "PinHeader_1x04_P2.54mm".into(),
        description: "Through hole pin header, 2.54 mm pitch".into(),
        pads: (0..n)
            .map(|i| Pad {
                back: false,
                mask: true,
                paste: true,
                id: hdr_pads[i],
                name: format!("{}", i + 1),
                pos: Point::mm(0.0, top - i as f64 * 2.54),
                rotation: Angle::ZERO,
                size: Size::mm(1.7, 1.7),
                shape: if i == 0 { PadShape::Rect } else { PadShape::Oval },
                kind: PadKind::ThroughHole { drill: Size::square(mm(1.0)) },
            })
            .collect(),
        graphics: vec![
            FootprintGraphic {
                back: false,
                layer: FootprintLayer::Silkscreen,
                shape: Shape::rect(Point::mm(-1.33, -top - 1.33), Point::mm(1.33, top + 1.33), mm(0.12)),
            },
            FootprintGraphic {
                back: false,
                layer: FootprintLayer::Courtyard,
                shape: Shape::rect(Point::mm(-1.8, -top - 1.8), Point::mm(1.8, top + 1.8), mm(0.05)),
            },
        ],
        texts: vec![FootprintText {
            back: false,
            layer: FootprintLayer::Silkscreen,
            text: Text::new("${REF}", Point::mm(0.0, top + 2.0), mm(1.0))
                .aligned(HAlign::Center, VAlign::Bottom)
                .with_bold(true),
        }],
    };
    lib.parts.insert(Part {
        id: PART_HEADER_1X4,
        name: "Conn_01x04".into(),
        description: "Generic 4 pin connector".into(),
        prefix: "J".into(),
        default_value: "Conn_01x04".into(),
        units: vec![PartUnit { name: "A".into(), symbol: bid(0x410) }],
        variants: vec![PartVariant {
            id: bid(0x420),
            name: "PinHeader 2.54mm".into(),
            footprint: Some(header_fp.id),
            connections: hdr_pads
                .iter()
                .zip(&hdr_pins)
                .map(|(&pad, &pin)| PadConnection { pad, unit: 0, pin })
                .collect(),
        }],
        properties: Default::default(),
        source: None,
    });

    for fp in [fp0603, fp0805, led0603, header_fp] {
        lib.footprints.insert(fp);
    }
    add_diode(&mut lib);
    add_npn(&mut lib);
    add_opamp(&mut lib);
    add_mounting_hole(&mut lib);
    add_74hc00(&mut lib);
    lib
}

/// 74HC00, quad 2-input NAND: four gate units (A–D) and a power unit (E), in DIP-14
/// and SOIC-14. One symbol per unit, so each gate shows its own pin numbers.
fn add_74hc00(lib: &mut Library) {
    // (unit name, [(pin name, pad number)]): gates are A, B inputs and Y output.
    let gates = [
        ("A", [("A", 1), ("B", 2), ("Y", 3)]),
        ("B", [("A", 4), ("B", 5), ("Y", 6)]),
        ("C", [("A", 9), ("B", 10), ("Y", 8)]),
        ("D", [("A", 12), ("B", 13), ("Y", 11)]),
    ];
    // Pin ids by unit: 0x9u0 + pin index; symbols 0x910 + unit.
    let pin_id = |unit: usize, i: usize| bid(0x9000 + 0x10 * unit as u128 + i as u128);
    let symbol_id = |unit: usize| bid(0x910 + unit as u128);
    // NAND body: flat back at -1.5 G, a half circle of radius 1.5 G, the bubble after it.
    let r = 1.5 * G;
    let mut body = vec![Point::mm(0.0, -r), Point::mm(-r, -r), Point::mm(-r, r), Point::mm(0.0, r)];
    body.extend((1..16).map(|i| {
        let a = std::f64::consts::FRAC_PI_2 - std::f64::consts::PI * i as f64 / 16.0;
        Point::mm(r * a.cos(), r * a.sin())
    }));
    let bubble = 0.635;
    let mut units = Vec::new();
    let mut pads_of: Vec<(Id, usize, u32)> = Vec::new(); // (pin, pad number, unit)
    for (u, (unit, pins)) in gates.iter().enumerate() {
        let gate_pin = |i: usize| {
            let (name, number) = pins[i];
            let (pos, direction, length, kind) = match name {
                "A" => (Point::mm(-3.0 * G, G), Angle::ZERO, mm(3.0 * G - r), PinKind::Input),
                "B" => (Point::mm(-3.0 * G, -G), Angle::ZERO, mm(3.0 * G - r), PinKind::Input),
                _ => (Point::mm(3.0 * G, 0.0), Angle::DEG180, mm(3.0 * G - r - 2.0 * bubble), PinKind::Output),
            };
            Pin { number: number.to_string(), kind, ..pin(pin_id(u, i), name, pos, direction, length) }
        };
        lib.symbols.insert(Symbol {
            id: symbol_id(u),
            name: format!("74HC00 {unit}"),
            graphics: vec![
                Shape::Polygon { points: body.clone(), width: LINE, fill: true },
                Shape::Circle { center: Point::mm(r + bubble, 0.0), radius: mm(bubble), width: LINE, fill: false },
            ],
            texts: ref_value_texts(r + 0.5, -r - 0.5),
            pins: (0..3).map(gate_pin).collect(),
        });
        units.push(PartUnit { name: unit.to_string(), symbol: symbol_id(u) });
        pads_of.extend(pins.iter().enumerate().map(|(i, &(_, n))| (pin_id(u, i), n, u as u32)));
    }
    // Power unit: VCC (14) on top, GND (7) below.
    let power = [("VCC", 14, G * 2.5, Angle::DEG270), ("GND", 7, -G * 2.5, Angle::DEG90)];
    lib.symbols.insert(Symbol {
        id: symbol_id(4),
        name: "74HC00 E".into(),
        graphics: vec![Shape::rect(Point::mm(-G, -1.5 * G), Point::mm(G, 1.5 * G), LINE)],
        texts: vec![
            Text::new("${REF}", Point::mm(1.5 * G, 0.5), mm(1.27)).aligned(HAlign::Left, VAlign::Bottom),
            Text::new("${VALUE}", Point::mm(1.5 * G, -0.5), mm(1.27)).aligned(HAlign::Left, VAlign::Top),
        ],
        pins: power
            .iter()
            .enumerate()
            .map(|(i, &(name, number, y, dir))| Pin {
                number: number.to_string(),
                kind: PinKind::PowerIn,
                show_name: true,
                ..pin(pin_id(4, i), name, Point::mm(0.0, y), dir, mm(G))
            })
            .collect(),
    });
    units.push(PartUnit { name: "E".into(), symbol: symbol_id(4) });
    pads_of.extend(power.iter().enumerate().map(|(i, &(_, n, _, _))| (pin_id(4, i), n, 4)));

    // Pins 1-7 on the left (top to bottom), 8-14 on the right (bottom to top).
    let dual_row = |pad_ids: &[Id], x: f64, pitch: f64, pad: &dyn Fn(Id, &str, f64, f64) -> Pad| -> Vec<Pad> {
        (0..14)
            .map(|i| {
                let (x, y) = if i < 7 {
                    (-x, 3.0 * pitch - i as f64 * pitch)
                } else {
                    (x, -3.0 * pitch + (i - 7) as f64 * pitch)
                };
                pad(pad_ids[i], &(i + 1).to_string(), x, y)
            })
            .collect()
    };
    let dip_pads: Vec<Id> = (0..14).map(|i| bid(0x1901 + i)).collect();
    let tht = |id: Id, name: &str, x: f64, y: f64| Pad {
        back: false,
        mask: true,
        paste: false,
        id,
        name: name.into(),
        pos: Point::mm(x, y),
        rotation: Angle::ZERO,
        size: Size::mm(1.6, 1.6),
        shape: if name == "1" { PadShape::Rect } else { PadShape::Oval },
        kind: PadKind::ThroughHole { drill: Size::square(mm(0.8)) },
    };
    let mut dip = smd_footprint(
        bid(0x1900),
        "DIP-14",
        "DIP-14, 7.62 mm row spacing, 2.54 mm pitch",
        dual_row(&dip_pads, 3.81, 2.54, &tht),
        (6.35, 19.05),
        (9.8, 19.6),
    );
    dip.graphics.push(FootprintGraphic {
        back: false,
        layer: FootprintLayer::Silkscreen,
        shape: Shape::rect(Point::mm(-2.6, -9.65), Point::mm(2.6, 9.65), mm(0.12)),
    });
    dip.graphics.push(FootprintGraphic {
        back: false,
        layer: FootprintLayer::Silkscreen,
        shape: Shape::Arc {
            center: Point::mm(0.0, 9.65),
            radius: mm(1.0),
            start: Angle::DEG180,
            sweep: 180.0,
            width: mm(0.12),
        },
    });
    let soic_pads: Vec<Id> = (0..14).map(|i| bid(0x1a01 + i)).collect();
    let smd = |id: Id, name: &str, x: f64, y: f64| smd_pad(id, name, x, y, Size::mm(1.95, 0.6));
    let mut soic = smd_footprint(
        bid(0x1a00),
        "SOIC-14",
        "SOIC-14, 3.9 mm body, 1.27 mm pitch",
        dual_row(&soic_pads, 2.475, 1.27, &smd),
        (3.9, 8.65),
        (7.4, 9.2),
    );
    soic.graphics.push(FootprintGraphic {
        back: false,
        layer: FootprintLayer::Silkscreen,
        shape: Shape::Circle { center: Point::mm(-3.6, 4.4), radius: mm(0.15), width: 0, fill: true },
    });
    let variant = |id: Id, fp: &Footprint, pad_ids: &[Id]| PartVariant {
        id,
        name: fp.name.clone(),
        footprint: Some(fp.id),
        connections: pads_of
            .iter()
            .map(|&(pin, number, unit)| PadConnection { pad: pad_ids[number - 1], unit, pin })
            .collect(),
    };
    let variants = vec![variant(bid(0x920), &dip, &dip_pads), variant(bid(0x921), &soic, &soic_pads)];
    lib.parts.insert(Part {
        id: PART_74HC00,
        name: "74HC00".into(),
        description: "Quad 2-input NAND gate".into(),
        prefix: "U".into(),
        default_value: "74HC00".into(),
        units,
        variants,
        properties: Default::default(),
        source: None,
    });
    lib.footprints.insert(dip);
    lib.footprints.insert(soic);
}

/// M3 mounting hole: a board part (no symbol), a plated 3.2 mm hole with a 6 mm pad.
fn add_mounting_hole(lib: &mut Library) {
    let fp = Footprint {
        id: bid(0x1800),
        name: "MountingHole_3.2mm_M3_Pad".into(),
        description: "Plated mounting hole for M3 screws".into(),
        pads: [Pad {
            back: false,
            mask: true,
            paste: true,
            id: bid(0x1801),
            name: "1".into(),
            pos: Point::ZERO,
            rotation: Angle::ZERO,
            size: Size::mm(6.0, 6.0),
            shape: PadShape::Circle,
            kind: PadKind::ThroughHole { drill: Size::square(mm(3.2)) },
        }]
        .into_iter()
        .collect(),
        graphics: vec![FootprintGraphic {
            back: false,
            layer: FootprintLayer::Courtyard,
            shape: Shape::Circle { center: Point::ZERO, radius: mm(3.45), width: mm(0.05), fill: false },
        }],
        texts: vec![FootprintText {
            back: false,
            layer: FootprintLayer::Fabrication,
            text: Text::new("${REF}", Point::mm(0.0, 4.0), mm(1.0))
                .aligned(HAlign::Center, VAlign::Bottom)
                .with_bold(true),
        }],
    };
    lib.parts.insert(Part {
        id: PART_MOUNTING_HOLE,
        name: "MountingHole_M3".into(),
        description: "M3 mounting hole (board part: placed on the board only)".into(),
        prefix: "H".into(),
        default_value: "MountingHole".into(),
        units: Vec::new(),
        variants: vec![PartVariant {
            id: bid(0x820),
            name: "M3 Pad".into(),
            footprint: Some(fp.id),
            connections: Vec::new(),
        }],
        properties: Default::default(),
        source: None,
    });
    lib.footprints.insert(fp);
}

fn smd_pad(id: Id, name: &str, x: f64, y: f64, size: Size) -> Pad {
    Pad {
        back: false,
        mask: true,
        paste: true,
        id,
        name: name.into(),
        pos: Point::mm(x, y),
        rotation: Angle::ZERO,
        size,
        shape: PadShape::RoundRect { ratio: 0.25 },
        kind: PadKind::Smd,
    }
}

/// SMD footprint from pads, a body outline (fabrication + silkscreen) and a courtyard.
fn smd_footprint(
    id: Id,
    name: &str,
    description: &str,
    pads: Vec<Pad>,
    body: (f64, f64),
    court: (f64, f64),
) -> Footprint {
    let (bw, bh) = (body.0 / 2.0, body.1 / 2.0);
    let (cw, ch) = (court.0 / 2.0, court.1 / 2.0);
    Footprint {
        id,
        name: name.into(),
        description: description.into(),
        pads: pads.into_iter().collect(),
        graphics: vec![
            FootprintGraphic {
                back: false,
                layer: FootprintLayer::Fabrication,
                shape: Shape::rect(Point::mm(-bw, -bh), Point::mm(bw, bh), mm(0.1)),
            },
            FootprintGraphic {
                back: false,
                layer: FootprintLayer::Courtyard,
                shape: Shape::rect(Point::mm(-cw, -ch), Point::mm(cw, ch), mm(0.05)),
            },
        ],
        texts: vec![FootprintText {
            back: false,
            layer: FootprintLayer::Silkscreen,
            text: Text::new("${REF}", Point::mm(0.0, ch + 0.2), mm(1.0))
                .aligned(HAlign::Center, VAlign::Bottom)
                .with_bold(true),
        }],
    }
}

fn simple_part(
    id: Id,
    name: &str,
    description: &str,
    prefix: &str,
    value: &str,
    symbol: Id,
    variant: PartVariant,
) -> Part {
    Part {
        id,
        name: name.into(),
        description: description.into(),
        prefix: prefix.into(),
        default_value: value.into(),
        units: vec![PartUnit { name: "A".into(), symbol }],
        variants: vec![variant],
        properties: Default::default(),
        source: None,
    }
}

/// Diode, SOD-123: pad 1 is the cathode.
fn add_diode(lib: &mut Library) {
    let (anode, cathode) = (bid(0x511), bid(0x512));
    let tri = Shape::Polygon {
        points: vec![Point::mm(-1.0, 1.2), Point::mm(-1.0, -1.2), Point::mm(1.0, 0.0)],
        width: LINE,
        fill: true,
    };
    let bar = Shape::line(Point::mm(1.0, 1.2), Point::mm(1.0, -1.2), LINE);
    lib.symbols.insert(two_pin_symbol(bid(0x510), "Diode", [(anode, "A"), (cathode, "K")], vec![tri, bar], 1.0));
    let (p1, p2) = (bid(0x1401), bid(0x1402));
    let pad = Size::mm(0.9, 1.2);
    let mut fp = smd_footprint(
        bid(0x1400),
        "SOD-123",
        "SOD-123 diode package",
        vec![smd_pad(p1, "1", -1.635, 0.0, pad), smd_pad(p2, "2", 1.635, 0.0, pad)],
        (2.7, 1.6),
        (4.7, 2.3),
    );
    // Cathode bar on the silkscreen.
    fp.graphics.push(FootprintGraphic {
        back: false,
        layer: FootprintLayer::Silkscreen,
        shape: Shape::line(Point::mm(-2.4, -0.9), Point::mm(-2.4, 0.9), mm(0.12)),
    });
    lib.footprints.insert(fp);
    let variant = PartVariant {
        id: bid(0x520),
        name: "SOD-123".into(),
        footprint: Some(bid(0x1400)),
        connections: vec![
            PadConnection { pad: p1, unit: 0, pin: cathode },
            PadConnection { pad: p2, unit: 0, pin: anode },
        ],
    };
    lib.parts.insert(simple_part(PART_DIODE, "Diode", "General purpose diode", "D", "1N4148W", bid(0x510), variant));
}

/// NPN transistor, SOT-23: 1 = B, 2 = E, 3 = C.
fn add_npn(lib: &mut Library) {
    let (b, c, e) = (bid(0x611), bid(0x612), bid(0x613));
    let mut symbol = Symbol {
        id: bid(0x610),
        name: "NPN".into(),
        graphics: vec![
            Shape::Circle { center: Point::mm(0.6, 0.0), radius: mm(2.0), width: LINE, fill: false },
            Shape::line(Point::mm(0.0, -1.2), Point::mm(0.0, 1.2), mm(0.4)),
            Shape::line(Point::mm(0.0, 0.5), Point::mm(G, G), LINE),
            Shape::line(Point::mm(0.0, -0.5), Point::mm(G, -G), LINE),
            Shape::Polygon {
                points: vec![Point::mm(G, -G), Point::mm(1.3, -2.3), Point::mm(2.1, -1.4)],
                width: LINE,
                fill: true,
            },
        ],
        texts: vec![
            Text::new("${REF}", Point::mm(3.3, 0.6), mm(1.27)).aligned(HAlign::Left, VAlign::Bottom),
            Text::new("${VALUE}", Point::mm(3.3, -0.6), mm(1.27)).aligned(HAlign::Left, VAlign::Top),
        ],
        pins: Store::default(),
    };
    symbol.pins.insert(pin(b, "B", Point::mm(-G, 0.0), Angle::ZERO, mm(G)));
    symbol.pins.insert(pin(c, "C", Point::mm(G, 2.0 * G), Angle::DEG270, mm(G)));
    symbol.pins.insert(pin(e, "E", Point::mm(G, -2.0 * G), Angle::DEG90, mm(G)));
    lib.symbols.insert(symbol);
    let pads = [bid(0x1501), bid(0x1502), bid(0x1503)];
    let size = Size::mm(0.9, 0.8);
    lib.footprints.insert(smd_footprint(
        bid(0x1500),
        "SOT-23",
        "SOT-23 3 pin package",
        vec![
            smd_pad(pads[0], "1", -1.0, -1.1, Size { w: size.h, h: size.w }),
            smd_pad(pads[1], "2", 1.0, -1.1, Size { w: size.h, h: size.w }),
            smd_pad(pads[2], "3", 0.0, 1.1, Size { w: size.h, h: size.w }),
        ],
        (2.9, 1.3),
        (3.4, 3.4),
    ));
    let variant = PartVariant {
        id: bid(0x620),
        name: "SOT-23".into(),
        footprint: Some(bid(0x1500)),
        connections: vec![
            PadConnection { pad: pads[0], unit: 0, pin: b },
            PadConnection { pad: pads[1], unit: 0, pin: e },
            PadConnection { pad: pads[2], unit: 0, pin: c },
        ],
    };
    lib.parts.insert(simple_part(PART_NPN, "NPN", "NPN bipolar transistor", "Q", "MMBT3904", bid(0x610), variant));
}

/// Single op amp, SOIC-8: 2 = -IN, 3 = +IN, 4 = V-, 6 = OUT, 7 = V+ (1, 5, 8 not connected).
fn add_opamp(lib: &mut Library) {
    let ids: Vec<Id> = (0..5).map(|i| bid(0x711 + i)).collect();
    let named = |i: usize, name: &str, pos: Point, dir: Angle| Pin {
        kind: match name {
            "OUT" => PinKind::Output,
            "V+" | "V-" => PinKind::PowerIn,
            _ => PinKind::Input,
        },
        ..pin(ids[i], name, pos, dir, mm(G))
    };
    let mut symbol = Symbol {
        id: bid(0x710),
        name: "OpAmp".into(),
        graphics: vec![Shape::Polygon {
            points: vec![Point::mm(-2.0 * G, 2.0 * G), Point::mm(-2.0 * G, -2.0 * G), Point::mm(2.0 * G, 0.0)],
            width: LINE,
            fill: true,
        }],
        texts: vec![
            Text::new("${REF}", Point::mm(G, 2.0 * G), mm(1.27)).aligned(HAlign::Left, VAlign::Bottom),
            Text::new("${VALUE}", Point::mm(G, -2.0 * G), mm(1.27)).aligned(HAlign::Left, VAlign::Top),
            // Input polarity marks (pin names stay hidden, as on most op amp symbols).
            Text::new("+", Point::mm(-1.6 * G, G), mm(1.27)).aligned(HAlign::Center, VAlign::Center),
            Text::new("-", Point::mm(-1.6 * G, -G), mm(1.27)).aligned(HAlign::Center, VAlign::Center),
        ],
        pins: Store::default(),
    };
    symbol.pins.insert(named(0, "+IN", Point::mm(-3.0 * G, G), Angle::ZERO));
    symbol.pins.insert(named(1, "-IN", Point::mm(-3.0 * G, -G), Angle::ZERO));
    symbol.pins.insert(named(2, "OUT", Point::mm(3.0 * G, 0.0), Angle::DEG180));
    symbol.pins.insert(named(3, "V+", Point::mm(0.0, 3.0 * G), Angle::DEG270));
    symbol.pins.insert(named(4, "V-", Point::mm(0.0, -3.0 * G), Angle::DEG90));
    lib.symbols.insert(symbol);
    // SOIC-8: pins 1-4 on the left (top to bottom), 5-8 on the right (bottom to top).
    let pad_ids: Vec<Id> = (0..8).map(|i| bid(0x1601 + i)).collect();
    let size = Size::mm(1.95, 0.6);
    let pads = (0..8)
        .map(|i| {
            let (x, y) =
                if i < 4 { (-2.475, 1.905 - i as f64 * 1.27) } else { (2.475, -1.905 + (i - 4) as f64 * 1.27) };
            smd_pad(pad_ids[i], &(i + 1).to_string(), x, y, size)
        })
        .collect();
    let mut fp =
        smd_footprint(bid(0x1600), "SOIC-8", "SOIC-8, 3.9 mm body, 1.27 mm pitch", pads, (3.9, 4.9), (7.4, 5.4));
    fp.graphics.push(FootprintGraphic {
        back: false,
        layer: FootprintLayer::Silkscreen,
        shape: Shape::Circle { center: Point::mm(-3.6, 2.6), radius: mm(0.15), width: 0, fill: true },
    });
    lib.footprints.insert(fp);
    let map = [(1, 1), (2, 0), (3, 4), (5, 2), (6, 3)]; // (pad index, pin index)
    let variant = PartVariant {
        id: bid(0x720),
        name: "SOIC-8".into(),
        footprint: Some(bid(0x1600)),
        connections: map
            .iter()
            .map(|&(pad, pin)| PadConnection { pad: pad_ids[pad], unit: 0, pin: ids[pin] })
            .collect(),
    };
    lib.parts.insert(simple_part(
        PART_OPAMP,
        "OpAmp",
        "Single operational amplifier",
        "U",
        "OPA171",
        bid(0x710),
        variant,
    ));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_library_is_consistent() {
        let lib = builtin_library();
        assert_eq!(lib.validate(), Vec::<String>::new());
        assert_eq!(lib.parts.len(), 9);
        // Ids are deterministic.
        assert_eq!(builtin_library(), lib);
    }

    #[test]
    fn led_cathode_is_pad_1() {
        let lib = builtin_library();
        let led = lib.parts.get(PART_LED).unwrap();
        let v = &led.variants[0];
        let fp = lib.footprints.get(v.footprint.unwrap()).unwrap();
        let sym = lib.symbols.get(led.units[0].symbol).unwrap();
        let k = sym.pins.iter().find(|p| p.name == "K").unwrap();
        let conn = v.connections.iter().find(|c| c.pin == k.id).unwrap();
        assert_eq!(fp.pads.get(conn.pad).unwrap().name, "1");
    }

    #[test]
    fn opamp_pinout() {
        let lib = builtin_library();
        let part = lib.parts.get(PART_OPAMP).unwrap();
        let sym = lib.symbols.get(part.units[0].symbol).unwrap();
        let fp = lib.footprints.get(part.variants[0].footprint.unwrap()).unwrap();
        let pad_of = |pin: &str| {
            let pin = sym.pins.iter().find(|p| p.name == pin).unwrap().id;
            let c = part.variants[0].connections.iter().find(|c| c.pin == pin).unwrap();
            fp.pads.get(c.pad).unwrap().name.clone()
        };
        let got: Vec<String> = ["-IN", "+IN", "V-", "OUT", "V+"].iter().map(|p| pad_of(p)).collect();
        assert_eq!(got, ["2", "3", "4", "6", "7"]);
        // Name matching finds nothing here: the mapping UI's "Map by name" is not enough.
        assert!(lib.map_by_name(&part.units, fp.id).is_empty());
    }

    #[test]
    fn hc00_pinout() {
        let lib = builtin_library();
        let part = lib.parts.get(PART_74HC00).unwrap();
        assert_eq!(part.units.len(), 5);
        assert_eq!(part.variants.len(), 2);
        for v in &part.variants {
            let fp = lib.footprints.get(v.footprint.unwrap()).unwrap();
            assert_eq!(v.connections.len(), 14, "{}", v.name);
            // Every pin goes to the pad of its number.
            for c in &v.connections {
                let sym = lib.symbols.get(part.units[c.unit as usize].symbol).unwrap();
                let pin = sym.pins.get(c.pin).unwrap();
                assert_eq!(fp.pads.get(c.pad).unwrap().name, pin.number, "{}", v.name);
            }
        }
        // Gate C's output is pin 8; the power unit has VCC 14 and GND 7.
        let pins = |u: usize| {
            lib.symbols
                .get(part.units[u].symbol)
                .unwrap()
                .pins
                .iter()
                .map(|p| (p.name.clone(), p.number.clone()))
                .collect::<Vec<_>>()
        };
        assert!(pins(2).contains(&("Y".into(), "8".into())));
        assert_eq!(pins(4), [("VCC".into(), "14".into()), ("GND".into(), "7".into())]);
    }
}
