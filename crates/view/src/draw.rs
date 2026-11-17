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
    /// Texts drawn for hit testing (an object's own text: a label's name, a field), with
    /// variables substituted, and their colour: what in-place editing edits.
    pub texts: Vec<(Text, Color)>,
}

impl Painter {
    pub fn clear(&mut self) {
        self.out.clear();
        self.bounds = None;
        self.texts.clear();
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
        let p = font::Placement {
            origin: mmf(t.pos),
            height: len(t.size),
            rotation: t.rotation.radians() as f32,
            halign: h,
            valign: v,
            bold: t.bold,
            mirror: false,
        };
        font::text_triangles(&s, &p, |([a, b, c], edges)| self.push(layer, Instance::triangle(a, b, c, edges, color)));
        if hit {
            self.texts.push((Text { text: s.clone(), ..t.clone() }, color));
        }
        if hit {
            // The box of advances, descender to cap top, rotated: the hit area.
            let [x0, y0, x1, y1] = font::text_box(&s, p.height, h, v);
            let (sin, cos) = p.rotation.sin_cos();
            let corner = |x: f32, y: f32| {
                Point::mm((p.origin[0] + x * cos - y * sin) as f64, (p.origin[1] + x * sin + y * cos) as f64)
            };
            let c = [corner(x0, y0), corner(x1, y0), corner(x1, y1), corner(x0, y1)];
            self.grow(Rect::from_points(c[0], c[2]).union(Rect::from_points(c[1], c[3])));
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
