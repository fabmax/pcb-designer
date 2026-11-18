//! Text in JetBrains Mono (Regular and Bold, embedded; SIL Open Font License, see
//! `fonts/OFL.txt`). Glyph outlines are flattened, cleaned (non-zero fill) and
//! triangulated once per glyph, then placed per text: triangles for drawing, outline
//! polygons for Gerber regions.
//!
//! Units: `height` is the cap height. Lines stack downwards (−y) by the font's line
//! spacing. Both weights share their metrics (the font is monospaced), so the layout
//! functions don't take the weight. No shaping or kerning.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use crate::tess::{Triangle, region_triangles};

/// JetBrains Mono Regular (also the UI's fallback font for arrows and key symbols).
pub static REGULAR: &[u8] = include_bytes!("../fonts/JetBrainsMono-Regular.ttf");
static BOLD: &[u8] = include_bytes!("../fonts/JetBrainsMono-Bold.ttf");

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HAlign {
    Left,
    Center,
    Right,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VAlign {
    Bottom,
    Center,
    Top,
}

/// A glyph in cap-height units (baseline at y = 0, cap height 1).
#[derive(Debug, Default)]
pub struct Glyph {
    pub advance: f32,
    /// Cleaned outlines: each an outer contour followed by its holes.
    pub shapes: Vec<Vec<Vec<[f32; 2]>>>,
    pub triangles: Vec<Triangle>,
}

struct Font {
    face: ttf_parser::Face<'static>,
    /// Font units per cap height.
    cap: f32,
    glyphs: Mutex<HashMap<char, Arc<Glyph>>>,
}

fn fonts() -> &'static [Font; 2] {
    static FONTS: OnceLock<[Font; 2]> = OnceLock::new();
    FONTS.get_or_init(|| {
        [REGULAR, BOLD].map(|data| {
            let face = ttf_parser::Face::parse(data, 0).expect("embedded font parses");
            let cap = face.capital_height().map_or(0.7 * face.units_per_em() as f32, |c| c as f32);
            Font { face, cap, glyphs: Mutex::default() }
        })
    })
}

fn font(bold: bool) -> &'static Font {
    &fonts()[bold as usize]
}

/// Distance between the baselines of two lines, in cap heights.
pub fn line_spacing() -> f32 {
    let f = font(false);
    let face = &f.face;
    (face.ascender() as f32 - face.descender() as f32 + face.line_gap() as f32) / f.cap
}

/// Depth of descenders below the baseline, in cap heights (positive).
pub fn descent() -> f32 {
    let f = font(false);
    -(f.face.descender() as f32) / f.cap
}

/// The glyph of `c` (the font's missing-glyph box if it has none).
pub fn glyph(c: char, bold: bool) -> Arc<Glyph> {
    let f = font(bold);
    if let Some(g) = f.glyphs.lock().unwrap().get(&c) {
        return g.clone();
    }
    let g = Arc::new(build_glyph(f, c));
    f.glyphs.lock().unwrap().insert(c, g.clone());
    g
}

/// Flattens an outline into closed polygons (font units).
struct Flatten {
    contours: Vec<Vec<[f32; 2]>>,
    at: [f32; 2],
}

/// Largest deviation of a flattened curve from the outline, in font units.
const TOLERANCE: f32 = 2.0;

impl Flatten {
    fn segments(dev: f32) -> usize {
        ((dev / TOLERANCE).sqrt().ceil() as usize).clamp(1, 32)
    }

    fn push(&mut self, p: [f32; 2]) {
        if let Some(c) = self.contours.last_mut() {
            c.push(p);
        }
        self.at = p;
    }
}

impl ttf_parser::OutlineBuilder for Flatten {
    fn move_to(&mut self, x: f32, y: f32) {
        self.contours.push(Vec::new());
        self.push([x, y]);
    }

    fn line_to(&mut self, x: f32, y: f32) {
        self.push([x, y]);
    }

    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        let p0 = self.at;
        // A quadratic's distance from its chord is a quarter of |p0 - 2p1 + p2|; it shrinks
        // with the square of the number of segments.
        let dev = ((p0[0] - 2.0 * x1 + x).hypot(p0[1] - 2.0 * y1 + y)) * 0.25;
        let n = Self::segments(dev);
        for i in 1..=n {
            let t = i as f32 / n as f32;
            let u = 1.0 - t;
            self.push([u * u * p0[0] + 2.0 * u * t * x1 + t * t * x, u * u * p0[1] + 2.0 * u * t * y1 + t * t * y]);
        }
    }

    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        let p0 = self.at;
        let dev =
            (p0[0] - 2.0 * x1 + x2).hypot(p0[1] - 2.0 * y1 + y2).max((x1 - 2.0 * x2 + x).hypot(y1 - 2.0 * y2 + y))
                * 0.75;
        let n = Self::segments(dev);
        for i in 1..=n {
            let t = i as f32 / n as f32;
            let u = 1.0 - t;
            let (a, b, c, d) = (u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
            self.push([a * p0[0] + b * x1 + c * x2 + d * x, a * p0[1] + b * y1 + c * y2 + d * y]);
        }
    }

    fn close(&mut self) {}
}

fn build_glyph(f: &Font, c: char) -> Glyph {
    use i_triangle::i_overlay::core::fill_rule::FillRule;
    use i_triangle::i_overlay::float::simplify::SimplifyShape;

    let face = &f.face;
    let id = face.glyph_index(c).unwrap_or(ttf_parser::GlyphId(0));
    let advance = face.glyph_hor_advance(id).unwrap_or(0) as f32 / f.cap;
    let mut flat = Flatten { contours: Vec::new(), at: [0.0; 2] };
    face.outline_glyph(id, &mut flat);
    let contours: Vec<Vec<[f32; 2]>> = flat
        .contours
        .into_iter()
        .map(|mut c| {
            if c.len() > 1 && c.first() == c.last() {
                c.pop();
            }
            c.into_iter().map(|[x, y]| [x / f.cap, y / f.cap]).collect::<Vec<_>>()
        })
        .filter(|c| c.len() >= 3)
        .collect();
    if contours.is_empty() {
        return Glyph { advance, ..Glyph::default() };
    }
    // TrueType outlines fill by the non-zero rule and may overlap (composite glyphs).
    let shapes = contours.simplify_shape(FillRule::NonZero);
    let triangles = shapes.iter().flat_map(|s| region_triangles(s)).collect();
    Glyph { advance, shapes, triangles }
}

/// Width of `text` in world units for the given cap height: its widest line.
pub fn text_width(text: &str, height: f32) -> f32 {
    text.split('\n').map(|l| line_width(l, height)).fold(0.0, f32::max)
}

fn line_width(line: &str, height: f32) -> f32 {
    line.chars().map(|c| glyph(c, false).advance).sum::<f32>() * height
}

/// Caret positions of a line: the x of every character boundary (`len + 1` values, the
/// first 0), in world units for cap `height`, before alignment.
pub fn caret_positions(line: &str, height: f32) -> Vec<f32> {
    let mut x = 0.0;
    let mut out = vec![0.0];
    for c in line.chars() {
        x += glyph(c, false).advance * height;
        out.push(x);
    }
    out
}

/// Where each line of `text` starts: its left end on the baseline, relative to the anchor
/// (before rotation). Lines stack downwards (−y), aligned by `halign`; `valign` places the
/// block (`Top`: the first line's cap top, `Bottom`: the last line's baseline, `Center`:
/// halfway between).
pub fn line_origins(text: &str, height: f32, halign: HAlign, valign: VAlign) -> Vec<[f32; 2]> {
    let lines: Vec<&str> = text.split('\n').collect();
    let pitch = height * line_spacing();
    let below = (lines.len() - 1) as f32 * pitch;
    let first = match valign {
        VAlign::Bottom => below,
        VAlign::Center => (below - height) * 0.5,
        VAlign::Top => -height,
    };
    lines
        .iter()
        .enumerate()
        .map(|(row, line)| {
            let width = line_width(line, height);
            let dx = match halign {
                HAlign::Left => 0.0,
                HAlign::Center => -width * 0.5,
                HAlign::Right => -width,
            };
            [dx, first - row as f32 * pitch]
        })
        .collect()
}

/// The box of `text` relative to its anchor, before rotation: `[min x, min y, max x,
/// max y]` from the advances, the descender of the last line and the cap top of the
/// first. Used for hit testing.
pub fn text_box(text: &str, height: f32, halign: HAlign, valign: VAlign) -> [f32; 4] {
    let origins = line_origins(text, height, halign, valign);
    let mut b = [f32::MAX, f32::MAX, f32::MIN, f32::MIN];
    for (line, [x, y]) in text.split('\n').zip(origins) {
        let w = line_width(line, height);
        b = [b[0].min(x), b[1].min(y - descent() * height), b[2].max(x + w), b[3].max(y + height)];
    }
    b
}

/// How a text is placed: anchor, cap height, rotation (radians, about the anchor),
/// alignment, weight, and mirrored (x negated about the anchor, before rotation).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placement {
    pub origin: [f32; 2],
    pub height: f32,
    pub rotation: f32,
    pub halign: HAlign,
    pub valign: VAlign,
    pub bold: bool,
    pub mirror: bool,
}

/// Every glyph of `text` with the transform from its cap-height units to the world.
fn place(text: &str, p: &Placement, mut out: impl FnMut(&Glyph, &dyn Fn([f32; 2]) -> [f32; 2])) {
    let (s, c) = p.rotation.sin_cos();
    let origins = line_origins(text, p.height, p.halign, p.valign);
    for (line, [dx, dy]) in text.split('\n').zip(origins) {
        let mut x0 = dx;
        for ch in line.chars() {
            let g = glyph(ch, p.bold);
            let xf = |q: [f32; 2]| {
                let x = x0 + q[0] * p.height;
                let x = if p.mirror { -x } else { x };
                let y = dy + q[1] * p.height;
                [p.origin[0] + x * c - y * s, p.origin[1] + x * s + y * c]
            };
            out(&g, &xf);
            x0 += g.advance * p.height;
        }
    }
}

/// The filled triangles of `text`.
pub fn text_triangles(text: &str, p: &Placement, mut out: impl FnMut(Triangle)) {
    place(text, p, |g, xf| {
        for (t, edges) in &g.triangles {
            out((t.map(xf), *edges));
        }
    });
}

/// The outlines of `text`, one shape (an outer contour, then its holes) at a time.
pub fn text_shapes(text: &str, p: &Placement, mut out: impl FnMut(Vec<Vec<[f32; 2]>>)) {
    place(text, p, |g, xf| {
        for shape in &g.shapes {
            out(shape.iter().map(|c| c.iter().map(|&q| xf(q)).collect()).collect());
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn placement(height: f32, halign: HAlign, valign: VAlign) -> Placement {
        Placement { origin: [0.0; 2], height, rotation: 0.0, halign, valign, bold: false, mirror: false }
    }

    fn bounds(text: &str, p: &Placement) -> [f32; 4] {
        let mut b = [f32::MAX, f32::MAX, f32::MIN, f32::MIN];
        text_triangles(text, p, |(t, _)| {
            for q in t {
                b = [b[0].min(q[0]), b[1].min(q[1]), b[2].max(q[0]), b[3].max(q[1])];
            }
        });
        b
    }

    #[test]
    fn capitals_are_the_text_size() {
        // "H" stands on the baseline and reaches the cap height.
        let b = bounds("H", &placement(2.0, HAlign::Left, VAlign::Bottom));
        assert!(b[1].abs() < 1e-3 && (b[3] - 2.0).abs() < 1e-2, "{b:?}");
        // Descenders hang below it.
        let g = bounds("g", &placement(2.0, HAlign::Left, VAlign::Bottom));
        assert!(g[1] < -0.3, "{g:?}");
    }

    #[test]
    fn glyphs_have_holes_and_both_weights_share_metrics() {
        let o = glyph('o', false);
        assert_eq!(o.shapes.len(), 1);
        assert_eq!(o.shapes[0].len(), 2, "outline and hole");
        assert!(!o.triangles.is_empty());
        for c in ['i', 'M', '0', ' ', '$'] {
            assert_eq!(glyph(c, false).advance, glyph(c, true).advance, "{c:?}");
        }
        // A character the font lacks gets the missing-glyph box.
        assert!(!glyph('\u{4e2d}', false).triangles.is_empty());
        // Bold strokes are wider: more area.
        let area = |g: &Glyph| -> f32 {
            g.triangles
                .iter()
                .map(|([a, b, c], _)| ((b[0] - a[0]) * (c[1] - a[1]) - (c[0] - a[0]) * (b[1] - a[1])).abs() * 0.5)
                .sum()
        };
        assert!(area(&glyph('l', true)) > area(&glyph('l', false)) * 1.2);
    }

    #[test]
    fn lines_stack_downwards() {
        let p = placement(1.0, HAlign::Left, VAlign::Top);
        let b = bounds("H\nH", &p);
        assert!(b[3].abs() < 1e-2, "the first line hangs from the anchor: {b:?}");
        assert!((b[1] + 1.0 + line_spacing()).abs() < 1e-2, "{b:?}");
        assert_eq!(text_width("II\nIIII", 6.0), text_width("IIII", 6.0));
        let boxed = text_box("H\nHg", 1.0, HAlign::Left, VAlign::Top);
        assert!(boxed[1] < b[1] && boxed[3] >= b[3] - 1e-3);
    }

    #[test]
    fn centered_text_is_symmetric_and_mirroring_flips_it() {
        let p = placement(1.0, HAlign::Center, VAlign::Center);
        let b = bounds("HH", &p);
        assert!((b[0] + b[2]).abs() < 1e-3, "{b:?}");
        let l = bounds("L", &placement(1.0, HAlign::Left, VAlign::Bottom));
        let m = bounds("L", &Placement { mirror: true, ..placement(1.0, HAlign::Left, VAlign::Bottom) });
        assert!((l[0] + m[2]).abs() < 1e-4 && (l[2] + m[0]).abs() < 1e-4);
    }
}
