//! Low-level Gerber X2 writer: apertures (deduplicated), flashes, strokes, arcs and
//! regions. Units are mm with format 4.6, so a coordinate is exactly the nm value.

use std::collections::HashMap;
use std::fmt::Write;

use model::*;

/// Aperture template.
#[derive(Clone, Debug, PartialEq)]
pub enum Aperture {
    Circle(Coord),
    Rect(Coord, Coord),
    Oval(Coord, Coord),
    /// `RoundRect` macro: corner radius and the four corner circle centres relative to
    /// the flash point, counterclockwise (rotation already applied). Also used for
    /// rotated rectangles (radius 0) and ovals (radius = half the shorter side).
    RoundRect {
        r: Coord,
        corners: [Point; 4],
    },
}

impl Aperture {
    fn template(&self) -> String {
        match self {
            Aperture::Circle(d) => format!("C,{}", mm6(*d)),
            Aperture::Rect(w, h) => format!("R,{}X{}", mm6(*w), mm6(*h)),
            Aperture::Oval(w, h) => format!("O,{}X{}", mm6(*w), mm6(*h)),
            Aperture::RoundRect { r, corners } => {
                let mut s = format!("RoundRect,{}", mm6(*r));
                for c in corners {
                    let _ = write!(s, "X{}X{}", mm6(c.x), mm6(c.y));
                }
                s
            }
        }
    }
}

/// The `RoundRect` aperture macro: `$1` corner radius, `$2`…`$9` the corner circle
/// centres (x, y, counterclockwise). A polygon through the centres, a circle at each and
/// a stroke along each side. Rotation is baked into the centres: circle primitives'
/// own rotation is ignored by some readers.
pub const ROUND_RECT_MACRO: &str = "%AMRoundRect*\n\
0 Rounded rectangle: $1 corner radius, $2 to $9 corner centres*\n\
4,1,4,$2,$3,$4,$5,$6,$7,$8,$9,$2,$3,0*\n\
1,1,$1+$1,$2,$3*\n\
1,1,$1+$1,$4,$5*\n\
1,1,$1+$1,$6,$7*\n\
1,1,$1+$1,$8,$9*\n\
20,1,$1+$1,$2,$3,$4,$5,0*\n\
20,1,$1+$1,$4,$5,$6,$7,0*\n\
20,1,$1+$1,$6,$7,$8,$9,0*\n\
20,1,$1+$1,$8,$9,$2,$3,0*%\n";

/// Millimetres with six decimals, trailing zeros trimmed.
pub fn mm6(v: Coord) -> String {
    let neg = v < 0;
    let a = v.unsigned_abs();
    let mut s = format!("{}{}.{:06}", if neg { "-" } else { "" }, a / 1_000_000, a % 1_000_000);
    while s.ends_with('0') {
        s.pop();
    }
    if s.ends_with('.') {
        s.push('0');
    }
    s
}

/// One Gerber file being written.
pub struct GerberWriter {
    header: Vec<String>,
    apertures: Vec<(Aperture, Option<String>)>,
    index: HashMap<String, usize>,
    body: String,
    current: Option<usize>,
    pos: Option<Point>,
    dark: bool,
    uses_macro: bool,
}

const FIRST_D: usize = 10;

impl GerberWriter {
    /// `attributes` are the X2 file attributes (`.FileFunction,…`), written as `%TF…*%`.
    pub fn new(attributes: &[String]) -> Self {
        let mut header = vec![format!("%TF.GenerationSoftware,pcb-designer,,{}*%", env!("CARGO_PKG_VERSION"))];
        header.extend(attributes.iter().map(|a| format!("%TF{a}*%")));
        GerberWriter {
            header,
            apertures: Vec::new(),
            index: HashMap::new(),
            body: String::new(),
            current: None,
            pos: None,
            dark: true,
            uses_macro: false,
        }
    }

    /// Select an aperture (with an optional `.AperFunction` value), defining it on first use.
    pub fn aperture(&mut self, ap: Aperture, function: Option<&str>) {
        let key = format!("{}|{}", ap.template(), function.unwrap_or(""));
        let i = match self.index.get(&key) {
            Some(&i) => i,
            None => {
                self.uses_macro |= matches!(ap, Aperture::RoundRect { .. });
                self.apertures.push((ap, function.map(str::to_string)));
                self.index.insert(key, self.apertures.len() - 1);
                self.apertures.len() - 1
            }
        };
        if self.current != Some(i) {
            let _ = writeln!(self.body, "D{}*", i + FIRST_D);
            self.current = Some(i);
        }
    }

    /// Dark (adds) or clear (removes what was drawn before) polarity.
    pub fn polarity(&mut self, dark: bool) {
        if self.dark != dark {
            self.body.push_str(if dark { "%LPD*%\n" } else { "%LPC*%\n" });
            self.dark = dark;
        }
    }

    pub fn flash(&mut self, p: Point) {
        let _ = writeln!(self.body, "X{}Y{}D03*", p.x, p.y);
        self.pos = Some(p);
    }

    fn move_to(&mut self, p: Point) {
        if self.pos != Some(p) {
            let _ = writeln!(self.body, "X{}Y{}D02*", p.x, p.y);
            self.pos = Some(p);
        }
    }

    /// A straight stroke with the current (circular) aperture.
    pub fn line(&mut self, a: Point, b: Point) {
        self.move_to(a);
        let _ = writeln!(self.body, "X{}Y{}D01*", b.x, b.y);
        self.pos = Some(b);
    }

    /// A circular stroke from `a` to `b` around `center` (a full circle when `a == b`).
    pub fn arc(&mut self, a: Point, b: Point, center: Point, ccw: bool) {
        self.move_to(a);
        let _ = writeln!(
            self.body,
            "{}X{}Y{}I{}J{}D01*\nG01*",
            if ccw { "G03" } else { "G02" },
            b.x,
            b.y,
            center.x - a.x,
            center.y - a.y
        );
        self.pos = Some(b);
    }

    /// A filled region: one closed contour (holes must be bridged in, see
    /// [`crate::bridge_holes`]).
    pub fn region(&mut self, contour: &[Point], function: Option<&str>) {
        if contour.len() < 3 {
            return;
        }
        if let Some(f) = function {
            let _ = writeln!(self.body, "%TA.AperFunction,{f}*%");
        }
        self.body.push_str("G36*\n");
        let _ = writeln!(self.body, "X{}Y{}D02*", contour[0].x, contour[0].y);
        for p in &contour[1..] {
            let _ = writeln!(self.body, "X{}Y{}D01*", p.x, p.y);
        }
        let _ = writeln!(self.body, "X{}Y{}D01*", contour[0].x, contour[0].y);
        self.body.push_str("G37*\n");
        if function.is_some() {
            self.body.push_str("%TD*%\n");
        }
        self.pos = Some(contour[0]);
    }

    pub fn finish(self) -> String {
        let mut out = String::new();
        for h in &self.header {
            out.push_str(h);
            out.push('\n');
        }
        out.push_str("%FSLAX46Y46*%\n%MOMM*%\n%LPD*%\nG75*\nG01*\n");
        if self.uses_macro {
            out.push_str(ROUND_RECT_MACRO);
        }
        for (i, (ap, function)) in self.apertures.iter().enumerate() {
            if let Some(f) = function {
                let _ = writeln!(out, "%TA.AperFunction,{f}*%");
            }
            let _ = writeln!(out, "%ADD{}{}*%", i + FIRST_D, ap.template());
            if function.is_some() {
                out.push_str("%TD*%\n");
            }
        }
        out.push_str(&self.body);
        out.push_str("M02*\n");
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers() {
        assert_eq!(mm6(1_500_000), "1.5");
        assert_eq!(mm6(-250), "-0.00025");
        assert_eq!(mm6(0), "0.0");
    }

    #[test]
    fn apertures_are_shared() {
        let mut w = GerberWriter::new(&[".FileFunction,Copper,L1,Top".into()]);
        w.aperture(Aperture::Circle(mm(0.25)), Some("Conductor"));
        w.line(Point::ZERO, Point::mm(1.0, 0.0));
        w.aperture(Aperture::Circle(mm(0.25)), Some("Conductor"));
        w.line(Point::mm(1.0, 0.0), Point::mm(1.0, 1.0));
        let s = w.finish();
        assert_eq!(s.matches("%ADD").count(), 1);
        assert!(s.contains("%ADD10C,0.25*%"));
        // The second stroke continues without a move.
        assert_eq!(s.matches("D02*").count(), 1);
        assert!(s.ends_with("M02*\n"));
    }
}
