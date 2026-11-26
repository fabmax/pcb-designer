//! Symbols of the generated parts. Millimetres, Y up; pins end on the 2.54 mm grid.

use super::super::gid;
use crate::geom::{Angle, HAlign, Point, Shape, Text, VAlign};
use crate::library::*;
use crate::units::mm;

pub(super) const G: f64 = 2.54;
const LINE: f64 = 0.254;

pub(super) struct Sym {
    name: String,
    graphics: Vec<Shape>,
    texts: Vec<Text>,
    pins: Vec<Pin>,
}

fn p(x: f64, y: f64) -> Point {
    Point::mm(x, y)
}

impl Sym {
    pub fn new(name: impl Into<String>) -> Sym {
        Sym { name: name.into(), graphics: Vec::new(), texts: Vec::new(), pins: Vec::new() }
    }

    /// A pin ending at (x, y), pointing `dir` degrees towards the body (0: from the left).
    pub fn pin(&mut self, number: &str, name: &str, x: f64, y: f64, dir: f64, len: f64) -> &mut Pin {
        self.pins.push(Pin {
            id: gid(&format!("sym/{}/pin/{number}", self.name)),
            name: name.into(),
            number: number.into(),
            pos: p(x, y),
            direction: Angle::deg(dir),
            length: mm(len),
            kind: PinKind::Passive,
            show_name: false,
            show_number: false,
        });
        self.pins.last_mut().unwrap()
    }

    pub fn line(&mut self, pts: &[(f64, f64)]) {
        self.line_w(pts, LINE);
    }

    pub fn line_w(&mut self, pts: &[(f64, f64)], w: f64) {
        for s in pts.windows(2) {
            self.graphics.push(Shape::line(p(s[0].0, s[0].1), p(s[1].0, s[1].1), mm(w)));
        }
    }

    pub fn rect(&mut self, x0: f64, y0: f64, x1: f64, y1: f64, fill: bool) {
        self.graphics.push(Shape::Rect { a: p(x0, y0), b: p(x1, y1), width: mm(LINE), fill });
    }

    pub fn poly(&mut self, pts: &[(f64, f64)], fill: bool) {
        let points = pts.iter().map(|&(x, y)| p(x, y)).collect();
        self.graphics.push(Shape::Polygon { points, width: mm(LINE), fill });
    }

    pub fn circle(&mut self, x: f64, y: f64, r: f64, fill: bool) {
        self.graphics.push(Shape::Circle { center: p(x, y), radius: mm(r), width: mm(LINE), fill });
    }

    /// Arc around (x, y) from `start` degrees, `sweep` degrees counterclockwise.
    pub fn arc(&mut self, x: f64, y: f64, r: f64, start: f64, sweep: f64) {
        self.graphics.push(Shape::Arc {
            center: p(x, y),
            radius: mm(r),
            start: Angle::deg(start),
            sweep,
            width: mm(LINE),
        });
    }

    pub fn text(&mut self, s: &str, x: f64, y: f64, h: HAlign, v: VAlign) {
        self.texts.push(Text::new(s, p(x, y), mm(1.27)).aligned(h, v));
    }

    /// Reference above `top`, value below `bottom`, centred.
    pub fn ref_value(&mut self, top: f64, bottom: f64) {
        self.text("${REF}", 0.0, top, HAlign::Center, VAlign::Bottom);
        self.text("${VALUE}", 0.0, bottom, HAlign::Center, VAlign::Top);
    }

    /// Reference and value left-aligned at `x`, on either side of `y`.
    pub fn ref_value_right(&mut self, x: f64, y: f64) {
        self.text("${REF}", x, y + 0.6, HAlign::Left, VAlign::Bottom);
        self.text("${VALUE}", x, y - 0.6, HAlign::Left, VAlign::Top);
    }

    pub fn finish(self) -> Symbol {
        Symbol {
            id: gid(&format!("sym/{}", self.name)),
            name: self.name,
            // The part's (`Builder::part`).
            category: String::new(),
            graphics: self.graphics,
            texts: self.texts,
            pins: self.pins.into_iter().collect(),
        }
    }
}

/// Two pins at ±2 G pointing inwards to ±`inner`: (number, name) left, then right.
fn two_pin(name: &str, left: (&str, &str), right: (&str, &str), inner: f64, text: f64) -> Sym {
    let mut s = Sym::new(name);
    s.pin(left.0, left.1, -2.0 * G, 0.0, 0.0, 2.0 * G - inner);
    s.pin(right.0, right.1, 2.0 * G, 0.0, 180.0, 2.0 * G - inner);
    s.ref_value(text, -text);
    s
}

pub fn resistor() -> Symbol {
    let mut s = two_pin("Resistor", ("1", "1"), ("2", "2"), G, 1.4);
    s.rect(-G, -0.9, G, 0.9, false);
    s.finish()
}

pub fn capacitor() -> Symbol {
    let mut s = two_pin("Capacitor", ("1", "1"), ("2", "2"), 0.5, 2.5);
    s.line_w(&[(-0.5, -2.0), (-0.5, 2.0)], 0.4);
    s.line_w(&[(0.5, -2.0), (0.5, 2.0)], 0.4);
    s.finish()
}

/// Pin 1 is "+": the open plate; the solid one is "-" (IEC style).
pub fn capacitor_polarized() -> Symbol {
    let mut s = two_pin("Capacitor_Polarized", ("1", "+"), ("2", "-"), 0.9, 2.5);
    s.rect(-0.9, -2.0, -0.3, 2.0, false);
    // Schematic fills are the light body colour: the solid plate is an outline filled by a
    // wide line.
    s.rect(0.3, -2.0, 0.9, 2.0, false);
    s.line_w(&[(0.6, -1.8), (0.6, 1.8)], 0.4);
    s.text("+", -1.7, 1.7, HAlign::Center, VAlign::Center);
    s.finish()
}

pub fn inductor() -> Symbol {
    let mut s = two_pin("Inductor", ("1", "1"), ("2", "2"), G, 1.4);
    for x in [-1.905, -0.635, 0.635, 1.905] {
        s.arc(x, 0.0, 0.635, 0.0, 180.0);
    }
    s.finish()
}

pub fn ferrite_bead() -> Symbol {
    let mut s = two_pin("FerriteBead", ("1", "1"), ("2", "2"), G, 2.2);
    s.line(&[(-G, 0.0), (G, 0.0)]);
    let h = std::f64::consts::FRAC_1_SQRT_2;
    let (u, v) = ((h, h), (-h, h));
    let corner = |a: f64, b: f64| (u.0 * a + v.0 * b, u.1 * a + v.1 * b);
    s.poly(&[corner(-1.8, -0.6), corner(1.8, -0.6), corner(1.8, 0.6), corner(-1.8, 0.6)], false);
    s.finish()
}

pub fn fuse() -> Symbol {
    let mut s = two_pin("Fuse", ("1", "1"), ("2", "2"), G, 1.4);
    s.rect(-G, -0.76, G, 0.76, false);
    s.line(&[(-G, 0.0), (G, 0.0)]);
    s.finish()
}

/// Anode left (pin 2), cathode right (pin 1), as the pads number them.
fn diode_base(name: &str, fill: bool, text: f64) -> Sym {
    let mut s = two_pin(name, ("2", "A"), ("1", "K"), 1.0, text);
    s.poly(&[(-1.0, 1.2), (-1.0, -1.2), (1.0, 0.0)], fill);
    s
}

pub fn diode() -> Symbol {
    let mut s = diode_base("Diode", true, 1.8);
    s.line(&[(1.0, 1.2), (1.0, -1.2)]);
    s.finish()
}

pub fn schottky() -> Symbol {
    let mut s = diode_base("Diode_Schottky", true, 1.8);
    s.line(&[(1.5, 0.9), (1.5, 1.2), (1.0, 1.2), (1.0, -1.2), (0.5, -1.2), (0.5, -0.9)]);
    s.finish()
}

pub fn zener() -> Symbol {
    let mut s = diode_base("Diode_Zener", true, 1.8);
    s.line(&[(1.5, 1.6), (1.0, 1.2), (1.0, -1.2), (0.5, -1.6)]);
    s.finish()
}

pub fn tvs() -> Symbol {
    let mut s = diode_base("TVS", false, 1.8);
    s.line(&[(1.5, 1.6), (1.0, 1.2), (1.0, -1.2), (0.5, -1.6)]);
    s.finish()
}

/// Bidirectional TVS: no polarity, pins 1 and 2.
pub fn tvs_bidirectional() -> Symbol {
    let mut s = two_pin("TVS_Bidirectional", ("1", "1"), ("2", "2"), 2.0, 1.8);
    s.poly(&[(-2.0, 1.2), (-2.0, -1.2), (0.0, 0.0)], false);
    s.poly(&[(2.0, 1.2), (2.0, -1.2), (0.0, 0.0)], false);
    s.line(&[(0.5, 1.6), (0.0, 1.2), (0.0, -1.2), (-0.5, -1.6)]);
    s.finish()
}

pub fn led() -> Symbol {
    let mut s = diode_base("LED", false, 3.0);
    s.line(&[(1.0, 1.2), (1.0, -1.2)]);
    // Light arrows.
    for x in [-0.4, 0.6] {
        s.line(&[(x, 1.5), (x + 0.9, 2.4)]);
        s.poly(&[(x + 0.9, 2.4), (x + 0.35, 2.25), (x + 0.75, 1.85)], true);
    }
    s.finish()
}

pub fn crystal() -> Symbol {
    let mut s = two_pin("Crystal", ("1", "1"), ("2", "2"), 1.3, 2.2);
    s.rect(-0.76, -1.6, 0.76, 1.6, false);
    s.line(&[(-1.3, -1.0), (-1.3, 1.0)]);
    s.line(&[(1.3, -1.0), (1.3, 1.0)]);
    s.finish()
}

/// Four-pad crystal: 1 and 3 the crystal, 2 and 4 the case (ground).
pub fn crystal_gnd24() -> Symbol {
    let mut s = Sym::new("Crystal_GND24");
    s.pin("1", "1", -2.0 * G, 0.0, 0.0, 2.0 * G - 1.3);
    s.pin("3", "3", 2.0 * G, 0.0, 180.0, 2.0 * G - 1.3);
    s.pin("2", "2", 0.0, -2.0 * G, 90.0, G);
    s.pin("4", "4", 0.0, 2.0 * G, 270.0, G);
    s.rect(-0.76, -1.6, 0.76, 1.6, false);
    s.line(&[(-1.3, -1.0), (-1.3, 1.0)]);
    s.line(&[(1.3, -1.0), (1.3, 1.0)]);
    s.line(&[(0.0, 1.6), (0.0, G)]);
    s.line(&[(0.0, -1.6), (0.0, -G)]);
    s.ref_value_right(1.0, 3.2);
    s.finish()
}

/// Bipolar transistor: B left, C top, E bottom; the arrow on the emitter points out
/// (NPN) or in (PNP).
pub fn bjt(pnp: bool) -> Symbol {
    let mut s = Sym::new(if pnp { "PNP" } else { "NPN" });
    s.pin("B", "B", -G, 0.0, 0.0, G);
    s.pin("C", "C", G, 2.0 * G, 270.0, G);
    s.pin("E", "E", G, -2.0 * G, 90.0, G);
    s.circle(0.6, 0.0, 2.0, false);
    s.line_w(&[(0.0, -1.2), (0.0, 1.2)], 0.4);
    s.line(&[(0.0, 0.5), (G, G)]);
    s.line(&[(0.0, -0.5), (G, -G)]);
    if pnp {
        s.poly(&[(0.25, -0.6), (1.2, -0.7), (0.75, -1.4)], true);
    } else {
        s.poly(&[(G, -G), (1.3, -2.3), (2.1, -1.4)], true);
    }
    s.ref_value_right(3.3, 0.0);
    s.finish()
}

/// MOSFET (enhancement): G left, D top, S bottom, body joined to the source; the arrow
/// points into the channel (N) or out of it (P).
pub fn mosfet(p_channel: bool) -> Symbol {
    let mut s = Sym::new(if p_channel { "PMOS" } else { "NMOS" });
    s.pin("G", "G", -G, 0.0, 0.0, G - 0.75);
    s.pin("D", "D", G, 2.0 * G, 270.0, G);
    s.pin("S", "S", G, -2.0 * G, 90.0, G);
    // Gate plate, then the channel in three segments with clear gaps.
    s.line(&[(-0.75, -1.65), (-0.75, 1.65)]);
    for (a, b) in [(1.05, 1.65), (-0.3, 0.3), (-1.65, -1.05)] {
        s.line_w(&[(0.0, a), (0.0, b)], 0.3);
    }
    s.line(&[(0.0, 1.35), (G, 1.35), (G, G)]);
    s.line(&[(0.0, -1.35), (G, -1.35), (G, -G)]);
    s.line(&[(0.0, 0.0), (G, 0.0), (G, -1.35)]);
    if p_channel {
        s.poly(&[(1.3, 0.0), (0.5, 0.4), (0.5, -0.4)], true);
    } else {
        s.poly(&[(0.4, 0.0), (1.2, 0.4), (1.2, -0.4)], true);
    }
    s.ref_value_right(3.3, 0.0);
    s.finish()
}

/// Connector with `rows` rows of `cols` (1 or 2) columns: pins numbered across the
/// columns (odd left, even right), pin 1 at the top, all on the grid.
pub fn connector(name: &str, rows: usize, cols: usize) -> Symbol {
    let mut s = Sym::new(name);
    let top = ((rows - 1) / 2) as f64 * G;
    let bottom = top - (rows - 1) as f64 * G;
    for r in 0..rows {
        for c in 0..cols {
            let n = r * cols + c + 1;
            let y = top - r as f64 * G;
            let (x, dir) = if c == 0 { (-2.0 * G, 0.0) } else { (2.0 * G, 180.0) };
            let pin = s.pin(&n.to_string(), &n.to_string(), x, y, dir, G);
            pin.show_number = true;
        }
    }
    s.rect(-G, bottom - 1.27, G, top + 1.27, true);
    s.ref_value(top + 1.8, bottom - 1.8);
    s.finish()
}

pub fn test_point() -> Symbol {
    let mut s = Sym::new("TestPoint");
    s.pin("1", "1", 0.0, -G, 90.0, 1.524);
    s.circle(0.0, -0.254, 0.762, false);
    s.ref_value_right(1.2, 0.0);
    s.finish()
}

pub fn push_button() -> Symbol {
    let mut s = Sym::new("SW_Push");
    s.pin("1", "1", -2.0 * G, 0.0, 0.0, 2.0 * G - 2.032);
    s.pin("2", "2", 2.0 * G, 0.0, 180.0, 2.0 * G - 2.032);
    s.circle(-1.524, 0.0, 0.508, false);
    s.circle(1.524, 0.0, 0.508, false);
    s.line(&[(-G, 1.27), (G, 1.27)]);
    s.line(&[(0.0, 1.27), (0.0, G)]);
    s.ref_value(3.2, -1.0);
    s.finish()
}

/// Toggle switch, single throw: pins 1 (left) and 2 (right), the lever open.
pub fn switch_spst() -> Symbol {
    let mut s = Sym::new("SW_SPST");
    s.pin("1", "1", -2.0 * G, 0.0, 0.0, 2.0 * G - 2.032);
    s.pin("2", "2", 2.0 * G, 0.0, 180.0, 2.0 * G - 2.032);
    s.circle(-1.524, 0.0, 0.508, false);
    s.circle(1.524, 0.0, 0.508, false);
    s.line(&[(-1.1, 0.3), (1.6, 1.4)]);
    s.ref_value(2.0, -1.0);
    s.finish()
}

/// Toggle switch, double throw: COM (2) left, A (1) right top, B (3) right bottom; the
/// lever on A.
pub fn switch_spdt() -> Symbol {
    let mut s = Sym::new("SW_SPDT");
    s.pin("2", "COM", -2.0 * G, 0.0, 0.0, 2.0 * G - 2.032);
    s.pin("1", "A", 2.0 * G, G, 180.0, 2.0 * G - 2.032);
    s.pin("3", "B", 2.0 * G, -G, 180.0, 2.0 * G - 2.032);
    s.circle(-1.524, 0.0, 0.508, false);
    s.circle(1.524, G, 0.508, false);
    s.circle(1.524, -G, 0.508, false);
    s.line(&[(-1.1, 0.25), (1.2, 2.0)]);
    s.text("${REF}", 0.0, G + 1.2, HAlign::Center, VAlign::Bottom);
    s.text("${VALUE}", 0.0, -G - 1.2, HAlign::Center, VAlign::Top);
    s.finish()
}

/// Potentiometer: a resistor between pins 1 (left) and 3 (right), the wiper (2) from
/// above with an arrow onto the body.
pub fn potentiometer() -> Symbol {
    let mut s = Sym::new("Potentiometer");
    s.pin("1", "1", -2.0 * G, 0.0, 0.0, G);
    s.pin("3", "3", 2.0 * G, 0.0, 180.0, G);
    s.pin("2", "2", 0.0, G, 270.0, 1.016);
    s.rect(-G, -0.9, G, 0.9, false);
    s.poly(&[(0.0, 0.9), (-0.5, 1.6), (0.5, 1.6)], true);
    s.text("${REF}", 0.8, 1.6, HAlign::Left, VAlign::Bottom);
    s.text("${VALUE}", 0.0, -1.4, HAlign::Center, VAlign::Top);
    s.finish()
}

/// An independent source: a circle between pin 1 (+, top) and pin 2 (−, bottom).
fn source(name: &str) -> Sym {
    let mut s = Sym::new(name);
    s.pin("1", "+", 0.0, 2.0 * G, 270.0, G);
    s.pin("2", "-", 0.0, -2.0 * G, 90.0, G);
    s.circle(0.0, 0.0, G, false);
    s.ref_value_right(G + 0.8, 0.0);
    s
}

/// Voltage source: + and − inside the circle.
pub fn vsource() -> Symbol {
    let mut s = source("VSource");
    s.line(&[(-0.5, 1.2), (0.5, 1.2)]);
    s.line(&[(0.0, 0.7), (0.0, 1.7)]);
    s.line(&[(-0.5, -1.2), (0.5, -1.2)]);
    s.finish()
}

/// Current source: an arrow from + to − (the direction of the current through it).
pub fn isource() -> Symbol {
    let mut s = source("ISource");
    s.line(&[(0.0, 1.5), (0.0, -0.7)]);
    s.poly(&[(0.0, -1.6), (-0.5, -0.7), (0.5, -0.7)], true);
    s.finish()
}
