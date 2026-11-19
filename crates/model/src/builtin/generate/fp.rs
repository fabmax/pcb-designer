//! Footprint construction: pads, a body outline (fabrication), silkscreen kept clear of
//! the pads, a courtyard around everything and the reference above it. Millimetres, Y up,
//! origin at the package centre.

use super::super::gid;
use crate::geom::{Angle, HAlign, Point, Shape, Size, Text, VAlign};
use crate::id::Id;
use crate::library::*;
use crate::units::mm;

const SILK: f64 = 0.12;
const FAB: f64 = 0.1;
const COURT: f64 = 0.05;
/// Silkscreen to pad copper.
const SILK_CLEARANCE: f64 = 0.2;

pub(super) struct Fp {
    name: String,
    description: String,
    pads: Vec<Pad>,
    graphics: Vec<FootprintGraphic>,
    /// Extents the courtyard must cover besides the pads: [x0, y0, x1, y1].
    keep: Vec<[f64; 4]>,
    /// Extra courtyard margin for a through-hole part.
    tht: bool,
    /// `None`: SMD or Through hole by the pads.
    category: Option<&'static str>,
}

fn rect_points(r: [f64; 4]) -> (Point, Point) {
    (Point::mm(r[0], r[1]), Point::mm(r[2], r[3]))
}

impl Fp {
    pub fn new(name: impl Into<String>, description: impl Into<String>) -> Fp {
        Fp {
            name: name.into(),
            description: description.into(),
            pads: Vec::new(),
            graphics: Vec::new(),
            keep: Vec::new(),
            tht: false,
            category: None,
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn pad(&mut self, name: &str, x: f64, y: f64, w: f64, h: f64, shape: PadShape, kind: PadKind) -> &mut Pad {
        self.pads.push(Pad {
            id: Id::from_u128(0),
            name: name.into(),
            pos: Point::mm(x, y),
            rotation: Angle::ZERO,
            size: Size::mm(w, h),
            shape,
            kind,
            back: false,
            mask: true,
            paste: true,
            mask_margin: None,
            paste_margin: None,
        });
        self.pads.last_mut().unwrap()
    }

    /// SMD pad, rounded rectangle.
    pub fn smd(&mut self, name: &str, x: f64, y: f64, w: f64, h: f64) -> &mut Pad {
        self.pad(name, x, y, w, h, PadShape::RoundRect { ratio: 0.25 }, PadKind::Smd)
    }

    /// Plated hole; pad 1 rectangular when `shape` is `None` and it is pad "1", else oval
    /// (round when `w == h`).
    pub fn tht(&mut self, name: &str, x: f64, y: f64, w: f64, h: f64, drill: f64) -> &mut Pad {
        self.tht = true;
        let shape = if name == "1" {
            PadShape::RoundRect { ratio: 0.15 }
        } else if w == h {
            PadShape::Circle
        } else {
            PadShape::Oval
        };
        let pad = self.pad(name, x, y, w, h, shape, PadKind::ThroughHole { drill: Size::square(mm(drill)) });
        pad.paste = false;
        pad
    }

    /// Plated hole with a round pad whatever its name.
    pub fn tht_round(&mut self, name: &str, x: f64, y: f64, d: f64, drill: f64) -> &mut Pad {
        let pad = self.tht(name, x, y, d, d, drill);
        pad.shape = PadShape::Circle;
        pad
    }

    pub fn npth(&mut self, x: f64, y: f64, drill: f64) -> &mut Pad {
        self.tht = true;
        let kind = PadKind::NonPlatedHole { drill: Size::square(mm(drill)) };
        let pad = self.pad("", x, y, drill, drill, PadShape::Circle, kind);
        pad.paste = false;
        pad
    }

    fn graphic(&mut self, layer: FootprintLayer, shape: Shape) {
        self.graphics.push(FootprintGraphic { layer, back: false, shape });
    }

    /// The body outline on the fabrication layer (inside the courtyard).
    pub fn body(&mut self, r: [f64; 4]) {
        let (a, b) = rect_points(r);
        self.graphic(FootprintLayer::Fabrication, Shape::rect(a, b, mm(FAB)));
        self.keep.push(r);
    }

    pub fn fab(&mut self, shape: Shape) {
        self.graphic(FootprintLayer::Fabrication, shape);
    }

    /// Courtyard must also cover `r` (pins sticking out, a connector's mating side).
    pub fn keep(&mut self, r: [f64; 4]) {
        self.keep.push(r);
    }

    /// Silkscreen shape as given (circles, arcs, marks away from pads).
    pub fn silk(&mut self, shape: Shape) {
        self.graphic(FootprintLayer::Silkscreen, shape);
    }

    /// The pads' copper rectangles grown by the silkscreen clearance.
    fn blocked(&self) -> Vec<[f64; 4]> {
        let c = SILK_CLEARANCE + SILK / 2.0;
        self.pads
            .iter()
            .map(|p| {
                let (x, y) = (crate::units::to_mm(p.pos.x), crate::units::to_mm(p.pos.y));
                let (w, h) = (crate::units::to_mm(p.size.w) / 2.0, crate::units::to_mm(p.size.h) / 2.0);
                [x - w - c, y - h - c, x + w + c, y + h + c]
            })
            .collect()
    }

    /// Silkscreen line (horizontal or vertical) with the parts over pads left out.
    pub fn silk_line(&mut self, x0: f64, y0: f64, x1: f64, y1: f64) {
        let horizontal = (y0 - y1).abs() < 1e-9;
        assert!(horizontal || (x0 - x1).abs() < 1e-9, "{}: silk_line is axis-aligned", self.name);
        let (lo, hi) = if horizontal { (x0.min(x1), x0.max(x1)) } else { (y0.min(y1), y0.max(y1)) };
        let mut cuts: Vec<(f64, f64)> = self
            .blocked()
            .into_iter()
            .filter_map(|b| {
                if horizontal {
                    (y0 > b[1] && y0 < b[3]).then_some((b[0], b[2]))
                } else {
                    (x0 > b[0] && x0 < b[2]).then_some((b[1], b[3]))
                }
            })
            .collect();
        cuts.sort_by(|a, b| a.0.total_cmp(&b.0));
        let mut start = lo;
        let mut segments = Vec::new();
        for (c0, c1) in cuts {
            if c0 > start {
                segments.push((start, c0.min(hi)));
            }
            start = start.max(c1);
            if start >= hi {
                break;
            }
        }
        if start < hi {
            segments.push((start, hi));
        }
        for (a, b) in segments.into_iter().filter(|s| s.1 - s.0 > 0.15) {
            let (p, q) =
                if horizontal { (Point::mm(a, y0), Point::mm(b, y0)) } else { (Point::mm(x0, a), Point::mm(x0, b)) };
            self.silk(Shape::line(p, q, mm(SILK)));
        }
    }

    /// Silkscreen rectangle outline, clear of the pads.
    pub fn silk_rect(&mut self, r: [f64; 4]) {
        self.silk_line(r[0], r[1], r[2], r[1]);
        self.silk_line(r[0], r[3], r[2], r[3]);
        self.silk_line(r[0], r[1], r[0], r[3]);
        self.silk_line(r[2], r[1], r[2], r[3]);
    }

    /// The body outline on the fabrication layer and, `grow` outside it, on the silkscreen.
    pub fn outline(&mut self, r: [f64; 4], grow: f64) {
        self.body(r);
        self.silk_rect([r[0] - grow, r[1] - grow, r[2] + grow, r[3] + grow]);
    }

    pub fn silk_circle(&mut self, x: f64, y: f64, r: f64) {
        self.silk(Shape::Circle { center: Point::mm(x, y), radius: mm(r), width: mm(SILK), fill: false });
    }

    pub fn silk_dot(&mut self, x: f64, y: f64) {
        self.silk(Shape::Circle { center: Point::mm(x, y), radius: mm(0.15), width: 0, fill: true });
    }

    /// A "+" on the silkscreen, `s` long.
    pub fn silk_plus(&mut self, x: f64, y: f64, s: f64) {
        let h = s / 2.0;
        self.silk(Shape::line(Point::mm(x - h, y), Point::mm(x + h, y), mm(SILK)));
        self.silk(Shape::line(Point::mm(x, y - h), Point::mm(x, y + h), mm(SILK)));
    }

    /// A category other than SMD / Through hole ("Mechanical").
    pub fn category(&mut self, category: &'static str) {
        self.category = Some(category);
    }

    /// Through hole when any pad is a plated hole, else SMD.
    fn pad_category(&self) -> &'static str {
        if self.pads.iter().any(|p| matches!(p.kind, PadKind::ThroughHole { .. })) { "Through hole" } else { "SMD" }
    }

    pub fn finish(self) -> Footprint {
        let category = self.category.unwrap_or_else(|| self.pad_category()).to_string();
        let margin = if self.tht { 0.5 } else { 0.25 };
        let mut ext = [f64::MAX, f64::MAX, f64::MIN, f64::MIN];
        let pads = self.pads.iter().map(|p| {
            let (x, y) = (crate::units::to_mm(p.pos.x), crate::units::to_mm(p.pos.y));
            let (w, h) = (crate::units::to_mm(p.size.w) / 2.0, crate::units::to_mm(p.size.h) / 2.0);
            [x - w, y - h, x + w, y + h]
        });
        for r in pads.chain(self.keep.iter().copied()) {
            ext = [ext[0].min(r[0]), ext[1].min(r[1]), ext[2].max(r[2]), ext[3].max(r[3])];
        }
        // Out to the 0.05 mm grid.
        let down = |v: f64| ((v - margin) / 0.05 - 1e-6).floor() * 0.05;
        let up = |v: f64| ((v + margin) / 0.05 + 1e-6).ceil() * 0.05;
        let court = [down(ext[0]), down(ext[1]), up(ext[2]), up(ext[3])];
        let mut graphics = self.graphics;
        let (a, b) = rect_points(court);
        graphics.push(FootprintGraphic {
            layer: FootprintLayer::Courtyard,
            back: false,
            shape: Shape::rect(a, b, mm(COURT)),
        });
        let name = self.name;
        let pads = self.pads.into_iter().enumerate().map(|(i, p)| Pad { id: gid(&format!("fp/{name}/pad/{i}")), ..p });
        Footprint {
            id: gid(&format!("fp/{name}")),
            pads: pads.collect(),
            name,
            description: self.description,
            category,
            graphics,
            texts: vec![FootprintText {
                layer: FootprintLayer::Silkscreen,
                back: false,
                text: Text::new("${REF}", Point::mm(0.0, court[3] + 0.2), mm(1.0))
                    .aligned(HAlign::Center, VAlign::Bottom)
                    .with_bold(true),
            }],
        }
    }
}

/// Positions of `n` items `pitch` apart, centred on 0, the first at the most negative
/// coordinate.
pub(super) fn row(n: usize, pitch: f64) -> impl Iterator<Item = f64> {
    (0..n).map(move |i| (i as f64 - (n as f64 - 1.0) / 2.0) * pitch)
}
