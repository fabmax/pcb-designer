//! Built-in starter library. Ids are fixed so that projects can later be updated from
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

const LINE: Coord = 254_000; // 0.254 mm symbol line width
const G: f64 = 2.54;

fn pin(id: Id, name: &str, pos: Point, direction: Angle, length: Coord) -> Pin {
    Pin { id, name: name.into(), pos, direction, length, kind: PinKind::Passive, show_name: false, show_number: true }
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
                layer: FootprintLayer::Silkscreen,
                shape: Shape::line(Point::mm(-bw * 0.3, silk_y), Point::mm(bw * 0.3, silk_y), mm(0.12)),
            },
            FootprintGraphic {
                layer: FootprintLayer::Silkscreen,
                shape: Shape::line(Point::mm(-bw * 0.3, -silk_y), Point::mm(bw * 0.3, -silk_y), mm(0.12)),
            },
            FootprintGraphic {
                layer: FootprintLayer::Fabrication,
                shape: Shape::rect(Point::mm(-bw, -bh), Point::mm(bw, bh), mm(0.1)),
            },
            FootprintGraphic {
                layer: FootprintLayer::Courtyard,
                shape: Shape::rect(Point::mm(-court_x, -court_y), Point::mm(court_x, court_y), mm(0.05)),
            },
        ],
        texts: vec![FootprintText {
            layer: FootprintLayer::Silkscreen,
            text: Text::new("${REF}", Point::mm(0.0, court_y + 0.2), mm(1.0)).aligned(HAlign::Center, VAlign::Bottom),
        }],
    }
}

fn connections(pads: [Id; 2], pins: [Id; 2]) -> Vec<PadConnection> {
    pads.iter().zip(pins).map(|(&pad, pin)| PadConnection { pad, unit: 0, pin }).collect()
}

pub fn starter_library() -> Library {
    let mut lib = Library {
        id: Some(LIBRARY_ID),
        name: "Starter".into(),
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
                footprint: fp0603.id,
                connections: connections([bid(0x1001), bid(0x1002)], r_pins),
            },
            PartVariant {
                id: bid(0x121),
                name: "0805".into(),
                footprint: fp0805.id,
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
            footprint: fp0603.id,
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
            footprint: led0603.id,
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
                id: hdr_pads[i],
                name: format!("{}", i + 1),
                pos: Point::mm(0.0, top - i as f64 * 2.54),
                rotation: Angle::ZERO,
                size: Size::mm(1.7, 1.7),
                shape: if i == 0 { PadShape::Rect } else { PadShape::Oval },
                kind: PadKind::ThroughHole { drill: mm(1.0) },
            })
            .collect(),
        graphics: vec![
            FootprintGraphic {
                layer: FootprintLayer::Silkscreen,
                shape: Shape::rect(Point::mm(-1.33, -top - 1.33), Point::mm(1.33, top + 1.33), mm(0.12)),
            },
            FootprintGraphic {
                layer: FootprintLayer::Courtyard,
                shape: Shape::rect(Point::mm(-1.8, -top - 1.8), Point::mm(1.8, top + 1.8), mm(0.05)),
            },
        ],
        texts: vec![FootprintText {
            layer: FootprintLayer::Silkscreen,
            text: Text::new("${REF}", Point::mm(0.0, top + 2.0), mm(1.0)).aligned(HAlign::Center, VAlign::Bottom),
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
            footprint: header_fp.id,
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
    lib
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn starter_library_is_consistent() {
        let lib = starter_library();
        assert_eq!(lib.validate(), Vec::<String>::new());
        assert_eq!(lib.parts.len(), 4);
        // Ids are deterministic.
        assert_eq!(starter_library(), lib);
    }

    #[test]
    fn led_cathode_is_pad_1() {
        let lib = starter_library();
        let led = lib.parts.get(PART_LED).unwrap();
        let v = &led.variants[0];
        let fp = lib.footprints.get(v.footprint).unwrap();
        let sym = lib.symbols.get(led.units[0].symbol).unwrap();
        let k = sym.pins.iter().find(|p| p.name == "K").unwrap();
        let conn = v.connections.iter().find(|c| c.pin == k.id).unwrap();
        assert_eq!(fp.pads.get(conn.pad).unwrap().name, "1");
    }
}
