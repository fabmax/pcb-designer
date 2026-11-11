//! Geometry primitives: points, angles, transforms, shapes and text.

use std::ops::{Add, Neg, Sub};

use serde::{Deserialize, Serialize};

use crate::units::{Coord, mm};

/// A point (or vector) in nanometers. World Y points up. Serialized as `[x, y]`.
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(from = "[Coord; 2]", into = "[Coord; 2]")]
pub struct Point {
    pub x: Coord,
    pub y: Coord,
}

impl From<[Coord; 2]> for Point {
    fn from([x, y]: [Coord; 2]) -> Self {
        Point { x, y }
    }
}

impl From<Point> for [Coord; 2] {
    fn from(p: Point) -> Self {
        [p.x, p.y]
    }
}

impl Point {
    pub const ZERO: Point = Point { x: 0, y: 0 };

    pub const fn new(x: Coord, y: Coord) -> Self {
        Point { x, y }
    }

    /// Point from millimeters.
    pub fn mm(x: f64, y: f64) -> Self {
        Point { x: mm(x), y: mm(y) }
    }

    pub fn to_mm(self) -> [f64; 2] {
        [crate::units::to_mm(self.x), crate::units::to_mm(self.y)]
    }

    pub fn length(self) -> f64 {
        (self.x as f64).hypot(self.y as f64)
    }

    /// Rotate around the origin. Multiples of 90° are exact.
    pub fn rotated(self, a: Angle) -> Point {
        match a.quarter_turns() {
            Some(0) => self,
            Some(1) => Point::new(-self.y, self.x),
            Some(2) => Point::new(-self.x, -self.y),
            Some(3) => Point::new(self.y, -self.x),
            _ => {
                let (s, c) = a.radians().sin_cos();
                let (x, y) = (self.x as f64, self.y as f64);
                Point::new((x * c - y * s).round() as Coord, (x * s + y * c).round() as Coord)
            }
        }
    }
}

impl Add for Point {
    type Output = Point;
    fn add(self, o: Point) -> Point {
        Point::new(self.x + o.x, self.y + o.y)
    }
}

impl Sub for Point {
    type Output = Point;
    fn sub(self, o: Point) -> Point {
        Point::new(self.x - o.x, self.y - o.y)
    }
}

impl Neg for Point {
    type Output = Point;
    fn neg(self) -> Point {
        Point::new(-self.x, -self.y)
    }
}

/// Width/height in nanometers. Serialized as `[w, h]`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(from = "[Coord; 2]", into = "[Coord; 2]")]
pub struct Size {
    pub w: Coord,
    pub h: Coord,
}

impl From<[Coord; 2]> for Size {
    fn from([w, h]: [Coord; 2]) -> Self {
        Size { w, h }
    }
}

impl From<Size> for [Coord; 2] {
    fn from(s: Size) -> Self {
        [s.w, s.h]
    }
}

impl Size {
    pub const fn new(w: Coord, h: Coord) -> Self {
        Size { w, h }
    }

    pub fn mm(w: f64, h: f64) -> Self {
        Size { w: mm(w), h: mm(h) }
    }
}

/// Angle in degrees, counterclockwise, normalized to `[0, 360)`.
#[derive(Clone, Copy, Debug, Default, PartialEq, PartialOrd, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(transparent)]
pub struct Angle(f64);

impl Angle {
    pub const ZERO: Angle = Angle(0.0);
    pub const DEG90: Angle = Angle(90.0);
    pub const DEG180: Angle = Angle(180.0);
    pub const DEG270: Angle = Angle(270.0);

    pub fn deg(v: f64) -> Self {
        let mut a = v % 360.0;
        if a < 0.0 {
            a += 360.0;
        }
        // Avoid 360.0 from rounding and -0.0.
        if a >= 360.0 || a == 0.0 {
            a = 0.0;
        }
        Angle(a)
    }

    pub fn degrees(self) -> f64 {
        self.0
    }

    pub fn radians(self) -> f64 {
        self.0.to_radians()
    }

    /// `Some(n)` if the angle is exactly `n * 90°`.
    pub fn quarter_turns(self) -> Option<u8> {
        let q = self.0 / 90.0;
        (q.fract() == 0.0).then_some(q as u8 % 4)
    }

    pub fn rotated_by(self, o: Angle) -> Angle {
        Angle::deg(self.0 + o.0)
    }

    /// Angle mirrored across the Y axis (x → -x).
    pub fn mirrored(self) -> Angle {
        Angle::deg(180.0 - self.0)
    }
}

/// Mirror (across the Y axis), then rotate, then translate.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Transform {
    pub offset: Point,
    pub rotation: Angle,
    pub mirror: bool,
}

impl Transform {
    pub const IDENTITY: Transform = Transform { offset: Point::ZERO, rotation: Angle::ZERO, mirror: false };

    pub fn new(offset: Point, rotation: Angle, mirror: bool) -> Self {
        Self { offset, rotation, mirror }
    }

    pub fn point(&self, p: Point) -> Point {
        let p = if self.mirror { Point::new(-p.x, p.y) } else { p };
        p.rotated(self.rotation) + self.offset
    }

    /// Transform a vector (no translation).
    pub fn vector(&self, v: Point) -> Point {
        let v = if self.mirror { Point::new(-v.x, v.y) } else { v };
        v.rotated(self.rotation)
    }

    pub fn angle(&self, a: Angle) -> Angle {
        let a = if self.mirror { a.mirrored() } else { a };
        a.rotated_by(self.rotation)
    }

    /// Sweep direction flips under mirroring.
    pub fn sweep(&self, sweep: f64) -> f64 {
        if self.mirror { -sweep } else { sweep }
    }
}

/// Axis aligned rectangle.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Rect {
    pub min: Point,
    pub max: Point,
}

impl Rect {
    pub fn from_points(a: Point, b: Point) -> Self {
        Rect { min: Point::new(a.x.min(b.x), a.y.min(b.y)), max: Point::new(a.x.max(b.x), a.y.max(b.y)) }
    }

    pub fn around(p: Point, r: Coord) -> Self {
        Rect { min: Point::new(p.x - r, p.y - r), max: Point::new(p.x + r, p.y + r) }
    }

    pub fn union(self, o: Rect) -> Rect {
        Rect {
            min: Point::new(self.min.x.min(o.min.x), self.min.y.min(o.min.y)),
            max: Point::new(self.max.x.max(o.max.x), self.max.y.max(o.max.y)),
        }
    }

    /// Grown by `d` on every side.
    pub fn grow(self, d: Coord) -> Rect {
        Rect { min: Point::new(self.min.x - d, self.min.y - d), max: Point::new(self.max.x + d, self.max.y + d) }
    }

    /// Do the rectangles overlap (touching counts)?
    pub fn intersects(&self, o: &Rect) -> bool {
        self.min.x <= o.max.x && o.min.x <= self.max.x && self.min.y <= o.max.y && o.min.y <= self.max.y
    }

    pub fn contains(&self, p: Point) -> bool {
        p.x >= self.min.x && p.x <= self.max.x && p.y >= self.min.y && p.y <= self.max.y
    }

    pub fn width(&self) -> Coord {
        self.max.x - self.min.x
    }

    pub fn height(&self) -> Coord {
        self.max.y - self.min.y
    }

    pub fn center(&self) -> Point {
        Point::new((self.min.x + self.max.x) / 2, (self.min.y + self.max.y) / 2)
    }

    /// Union of all rectangles, `None` for an empty iterator.
    pub fn union_all(rects: impl IntoIterator<Item = Rect>) -> Option<Rect> {
        rects.into_iter().reduce(Rect::union)
    }
}

/// Stroked or filled geometric shape. `width` is the stroke width (0 = hairline/filled).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Shape {
    Line {
        a: Point,
        b: Point,
        width: Coord,
    },
    /// Arc from `start` sweeping `sweep` degrees (positive = counterclockwise).
    Arc {
        center: Point,
        radius: Coord,
        start: Angle,
        sweep: f64,
        width: Coord,
    },
    Circle {
        center: Point,
        radius: Coord,
        width: Coord,
        #[serde(default)]
        fill: bool,
    },
    Rect {
        a: Point,
        b: Point,
        width: Coord,
        #[serde(default)]
        fill: bool,
    },
    Polygon {
        points: Vec<Point>,
        width: Coord,
        #[serde(default)]
        fill: bool,
    },
}

impl Shape {
    pub fn line(a: Point, b: Point, width: Coord) -> Shape {
        Shape::Line { a, b, width }
    }

    pub fn rect(a: Point, b: Point, width: Coord) -> Shape {
        Shape::Rect { a, b, width, fill: false }
    }

    pub fn transformed(&self, t: &Transform) -> Shape {
        match self {
            Shape::Line { a, b, width } => Shape::Line { a: t.point(*a), b: t.point(*b), width: *width },
            Shape::Arc { center, radius, start, sweep, width } => Shape::Arc {
                center: t.point(*center),
                radius: *radius,
                start: t.angle(*start),
                sweep: t.sweep(*sweep),
                width: *width,
            },
            Shape::Circle { center, radius, width, fill } => {
                Shape::Circle { center: t.point(*center), radius: *radius, width: *width, fill: *fill }
            }
            Shape::Rect { a, b, width, fill } => {
                if t.rotation.quarter_turns().is_some() {
                    Shape::Rect { a: t.point(*a), b: t.point(*b), width: *width, fill: *fill }
                } else {
                    let pts = [*a, Point::new(b.x, a.y), *b, Point::new(a.x, b.y)];
                    Shape::Polygon { points: pts.iter().map(|p| t.point(*p)).collect(), width: *width, fill: *fill }
                }
            }
            Shape::Polygon { points, width, fill } => {
                Shape::Polygon { points: points.iter().map(|p| t.point(*p)).collect(), width: *width, fill: *fill }
            }
        }
    }

    /// Bounding box including stroke width.
    pub fn bounds(&self) -> Rect {
        match self {
            Shape::Line { a, b, width } => grow(Rect::from_points(*a, *b), width / 2),
            Shape::Arc { center, radius, width, .. } => Rect::around(*center, radius + width / 2),
            Shape::Circle { center, radius, width, .. } => Rect::around(*center, radius + width / 2),
            Shape::Rect { a, b, width, .. } => grow(Rect::from_points(*a, *b), width / 2),
            Shape::Polygon { points, width, .. } => grow(
                Rect::union_all(points.iter().map(|p| Rect::from_points(*p, *p)))
                    .unwrap_or(Rect::around(Point::ZERO, 0)),
                width / 2,
            ),
        }
    }
}

fn grow(r: Rect, d: Coord) -> Rect {
    Rect { min: Point::new(r.min.x - d, r.min.y - d), max: Point::new(r.max.x + d, r.max.y + d) }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum HAlign {
    #[default]
    Left,
    Center,
    Right,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum VAlign {
    #[default]
    Bottom,
    Center,
    Top,
}

/// Stroke-font text.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Text {
    pub text: String,
    pub pos: Point,
    /// Cap height.
    pub size: Coord,
    /// Stroke width.
    pub width: Coord,
    #[serde(default)]
    pub rotation: Angle,
    #[serde(default)]
    pub halign: HAlign,
    #[serde(default)]
    pub valign: VAlign,
}

impl Text {
    pub fn new(text: impl Into<String>, pos: Point, size: Coord) -> Self {
        Text {
            text: text.into(),
            pos,
            size,
            width: size / 7,
            rotation: Angle::ZERO,
            halign: HAlign::Left,
            valign: VAlign::Bottom,
        }
    }

    pub fn aligned(mut self, h: HAlign, v: VAlign) -> Self {
        self.halign = h;
        self.valign = v;
        self
    }

    pub fn transformed(&self, t: &Transform) -> Text {
        // Text stays readable: it is moved and rotated but never mirrored.
        Text { pos: t.point(self.pos), rotation: self.rotation.rotated_by(t.rotation), ..self.clone() }
    }

    /// The same text, never upside down or reading downwards: turned by 180° with its
    /// alignment swapped when it would read right-to-left or top-to-bottom, so it covers
    /// the same area (how KiCad draws rotated labels).
    pub fn upright(self) -> Text {
        let d = self.rotation.degrees();
        if d <= 90.0 || d > 270.0 {
            return self;
        }
        let halign = match self.halign {
            HAlign::Left => HAlign::Right,
            HAlign::Center => HAlign::Center,
            HAlign::Right => HAlign::Left,
        };
        let valign = match self.valign {
            VAlign::Bottom => VAlign::Top,
            VAlign::Center => VAlign::Center,
            VAlign::Top => VAlign::Bottom,
        };
        Text { rotation: self.rotation.rotated_by(Angle::DEG180), halign, valign, ..self }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn point_serializes_as_array() {
        assert_eq!(serde_json::to_string(&Point::new(1, -2)).unwrap(), "[1,-2]");
        assert_eq!(serde_json::from_str::<Point>("[3,4]").unwrap(), Point::new(3, 4));
    }

    #[test]
    fn upright_text_turns_around() {
        let t = Text::new("A", Point::ZERO, 1).aligned(HAlign::Left, VAlign::Bottom);
        assert_eq!(t.clone().upright(), t);
        let turned = Text { rotation: Angle::DEG180, ..t.clone() }.upright();
        assert_eq!((turned.rotation, turned.halign, turned.valign), (Angle::ZERO, HAlign::Right, VAlign::Top));
        let down = Text { rotation: Angle::DEG270, ..t }.upright();
        assert_eq!(down.rotation, Angle::DEG90);
    }

    #[test]
    fn angle_normalization() {
        assert_eq!(Angle::deg(-90.0), Angle::DEG270);
        assert_eq!(Angle::deg(720.0), Angle::ZERO);
        assert_eq!(Angle::deg(450.0).quarter_turns(), Some(1));
        assert_eq!(Angle::deg(45.0).quarter_turns(), None);
    }

    #[test]
    fn quarter_rotations_are_exact() {
        let p = Point::new(1_234_567, 89);
        assert_eq!(p.rotated(Angle::DEG90), Point::new(-89, 1_234_567));
        let mut q = p;
        for _ in 0..4 {
            q = q.rotated(Angle::DEG90);
        }
        assert_eq!(q, p);
    }

    #[test]
    fn transform_mirror_then_rotate_then_translate() {
        let t = Transform::new(Point::new(100, 0), Angle::DEG90, true);
        // (10, 0) → mirror (-10, 0) → rotate 90° (0, -10) → translate (100, -10)
        assert_eq!(t.point(Point::new(10, 0)), Point::new(100, -10));
        // Angle 0° → mirrored 180° → +90° = 270°
        assert_eq!(t.angle(Angle::ZERO), Angle::DEG270);
    }
}
