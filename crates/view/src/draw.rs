//! Painter: converts model shapes and text into render instances and tracks bounds.

use model::{Coord, HAlign, Point, Rect, Shape, Text, VAlign};
use render::font::{self, HAlign as FH, VAlign as FV};
use render::{Color, Instance, LayerId};

pub(crate) fn mmf(p: Point) -> [f32; 2] {
    [(p.x as f64 * 1e-6) as f32, (p.y as f64 * 1e-6) as f32]
}

fn len(c: Coord) -> f32 {
    (c as f64 * 1e-6) as f32
}

#[derive(Default)]
pub(crate) struct Painter {
    pub out: Vec<(LayerId, Instance)>,
    /// Bounds used for hit testing (text is excluded unless added explicitly).
    pub bounds: Option<Rect>,
}

impl Painter {
    pub fn clear(&mut self) {
        self.out.clear();
        self.bounds = None;
    }

    pub fn grow(&mut self, r: Rect) {
        self.bounds = Some(match self.bounds {
            Some(b) => b.union(r),
            None => r,
        });
    }

    pub fn push(&mut self, layer: LayerId, inst: Instance) {
        self.out.push((layer, inst));
    }

    pub fn line(&mut self, layer: LayerId, a: Point, b: Point, width: Coord, color: Color) {
        self.push(layer, Instance::line(mmf(a), mmf(b), len(width), color));
        self.grow(Shape::line(a, b, width).bounds());
    }

    pub fn circle(&mut self, layer: LayerId, c: Point, r: Coord, color: Color) {
        self.push(layer, Instance::circle(mmf(c), len(r), color));
        self.grow(Rect::around(c, r));
    }

    pub fn shape(&mut self, layer: LayerId, shape: &Shape, color: Color, fill_layer: Option<(LayerId, Color)>) {
        match shape {
            Shape::Line { a, b, width } => self.push(layer, Instance::line(mmf(*a), mmf(*b), len(*width), color)),
            Shape::Arc { center, radius, start, sweep, width } => self.push(
                layer,
                Instance::arc(
                    mmf(*center),
                    len(*radius),
                    start.radians() as f32,
                    sweep.to_radians() as f32,
                    len(*width),
                    color,
                ),
            ),
            Shape::Circle { center, radius, width, fill } => {
                if *fill {
                    let (l, c) = fill_layer.unwrap_or((layer, color));
                    self.push(l, Instance::circle(mmf(*center), len(radius + width / 2), c));
                }
                if !*fill || fill_layer.is_some() {
                    self.push(layer, Instance::circle_outline(mmf(*center), len(*radius), len(*width), color));
                }
            }
            Shape::Rect { a, b, width, fill } => {
                let r = Rect::from_points(*a, *b);
                if *fill {
                    let (l, c) = fill_layer.unwrap_or((layer, color));
                    let size = [len(r.width() + width), len(r.height() + width)];
                    self.push(l, Instance::rect(mmf(r.center()), size, 0.0, 0.0, c));
                }
                if !*fill || fill_layer.is_some() {
                    let pts = [r.min, Point::new(r.max.x, r.min.y), r.max, Point::new(r.min.x, r.max.y)];
                    for i in 0..4 {
                        self.push(layer, Instance::line(mmf(pts[i]), mmf(pts[(i + 1) % 4]), len(*width), color));
                    }
                }
            }
            Shape::Polygon { points, width, fill } => {
                if *fill {
                    let (l, c) = fill_layer.unwrap_or((layer, color));
                    let pts: Vec<[f32; 2]> = points.iter().map(|&q| mmf(q)).collect();
                    for inst in render::tess::fill_polygon(&pts, c) {
                        self.push(l, inst);
                    }
                }
                for i in 0..points.len() {
                    let (a, b) = (points[i], points[(i + 1) % points.len()]);
                    self.push(layer, Instance::line(mmf(a), mmf(b), len(*width), color));
                }
            }
        }
        self.grow(shape.bounds());
    }

    /// Filled polygon without outline.
    pub fn fill(&mut self, layer: LayerId, points: impl IntoIterator<Item = Point>, color: Color) {
        let pts: Vec<[f32; 2]> = points.into_iter().map(mmf).collect();
        for inst in render::tess::fill_polygon(&pts, color) {
            self.push(layer, inst);
        }
    }

    /// Draw text. `subst` replaces `${REF}`-style variables.
    pub fn text(&mut self, layer: LayerId, t: &Text, color: Color, subst: &dyn Fn(&str) -> String, hit: bool) {
        let s = subst(&t.text);
        if s.is_empty() {
            return;
        }
        let h = match t.halign {
            HAlign::Left => FH::Left,
            HAlign::Center => FH::Center,
            HAlign::Right => FH::Right,
        };
        let v = match t.valign {
            VAlign::Bottom => FV::Bottom,
            VAlign::Center => FV::Center,
            VAlign::Top => FV::Top,
        };
        let width = len(t.width.max(1));
        let mut min = [f32::MAX; 2];
        let mut max = [f32::MIN; 2];
        let mut segs = Vec::new();
        font::text_segments(&s, mmf(t.pos), len(t.size), t.rotation.radians() as f32, h, v, |a, b| {
            for p in [a, b] {
                min = [min[0].min(p[0]), min[1].min(p[1])];
                max = [max[0].max(p[0]), max[1].max(p[1])];
            }
            segs.push((a, b));
        });
        for (a, b) in segs {
            self.push(layer, Instance::line(a, b, width, color));
        }
        if hit && min[0] <= max[0] {
            self.grow(Rect::from_points(
                Point::mm(min[0] as f64, min[1] as f64),
                Point::mm(max[0] as f64, max[1] as f64),
            ));
        }
    }
}

/// Replace `${REF}` and `${VALUE}`.
pub(crate) fn substitute(text: &str, refdes: &str, value: &str) -> String {
    if !text.contains("${") {
        return text.to_string();
    }
    text.replace("${REF}", refdes).replace("${VALUE}", value)
}
