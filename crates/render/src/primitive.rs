//! Instanced primitive types shared between CPU scene and GPU shader.

use bytemuck::{Pod, Zeroable};

/// RGBA color, 8 bit per channel, non-premultiplied, sRGB encoded.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Pod, Zeroable)]
pub struct Color(pub [u8; 4]);

impl Color {
    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self([r, g, b, 255])
    }

    pub const fn rgba(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self([r, g, b, a])
    }

    pub const fn with_alpha(self, a: u8) -> Self {
        Self([self.0[0], self.0[1], self.0[2], a])
    }

    pub fn to_f32(self) -> [f32; 4] {
        self.0.map(|c| c as f32 / 255.0)
    }
}

/// Shape kinds understood by the primitive shader. Must match `shader.wgsl`.
pub mod kind {
    /// Line segment with round caps: `p0`..`p1`, half width `r`.
    pub const CAPSULE: u32 = 0;
    /// Disc (`r2 == 0`) or ring (`r2` = inner radius) centered at `p0` with radius `r`.
    pub const CIRCLE: u32 = 1;
    /// Rotated rounded rectangle centered at `p0`, half size `p1`, corner radius `r`,
    /// rotation `r2` (radians).
    pub const RECT: u32 = 2;
    /// Arc around center `p0`, `p1 = (start angle, sweep)` in radians, half width `r`,
    /// radius `r2`.
    pub const ARC: u32 = 3;
    /// Placeholder for freed slots. Produces no fragments.
    pub const EMPTY: u32 = 255;
}

/// One GPU instance (32 bytes). Coordinates are world units (millimeters).
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
pub struct Instance {
    pub p0: [f32; 2],
    pub p1: [f32; 2],
    pub r: f32,
    pub r2: f32,
    pub kind: u32,
    pub color: Color,
}

impl Instance {
    pub const EMPTY: Instance =
        Instance { p0: [0.0; 2], p1: [0.0; 2], r: 0.0, r2: 0.0, kind: kind::EMPTY, color: Color([0; 4]) };

    /// Line from `a` to `b` with total `width`.
    pub fn line(a: [f32; 2], b: [f32; 2], width: f32, color: Color) -> Self {
        Self { p0: a, p1: b, r: width * 0.5, r2: 0.0, kind: kind::CAPSULE, color }
    }

    pub fn circle(center: [f32; 2], radius: f32, color: Color) -> Self {
        Self { p0: center, p1: [0.0; 2], r: radius, r2: 0.0, kind: kind::CIRCLE, color }
    }

    /// Annulus with outer and inner radius.
    pub fn ring(center: [f32; 2], outer: f32, inner: f32, color: Color) -> Self {
        Self { p0: center, p1: [0.0; 2], r: outer, r2: inner, kind: kind::CIRCLE, color }
    }

    /// Circle outline with stroke `width` centered on `radius`.
    pub fn circle_outline(center: [f32; 2], radius: f32, width: f32, color: Color) -> Self {
        Self::ring(center, radius + width * 0.5, (radius - width * 0.5).max(0.0), color)
    }

    /// Rounded rectangle centered at `center` with full `size`.
    pub fn rect(center: [f32; 2], size: [f32; 2], corner: f32, rotation: f32, color: Color) -> Self {
        Self { p0: center, p1: [size[0] * 0.5, size[1] * 0.5], r: corner, r2: rotation, kind: kind::RECT, color }
    }

    /// Arc stroke around `center`. Angles in radians, counterclockwise.
    pub fn arc(center: [f32; 2], radius: f32, start: f32, sweep: f32, width: f32, color: Color) -> Self {
        Self { p0: center, p1: [start, sweep], r: width * 0.5, r2: radius, kind: kind::ARC, color }
    }

    /// Axis aligned bounding box `[min_x, min_y, max_x, max_y]`, or `None` for empty slots.
    pub fn bounds(&self) -> Option<[f32; 4]> {
        let [x0, y0] = self.p0;
        match self.kind {
            kind::CAPSULE => {
                let [x1, y1] = self.p1;
                Some([x0.min(x1) - self.r, y0.min(y1) - self.r, x0.max(x1) + self.r, y0.max(y1) + self.r])
            }
            kind::CIRCLE => Some([x0 - self.r, y0 - self.r, x0 + self.r, y0 + self.r]),
            kind::RECT => {
                let (s, c) = self.r2.sin_cos();
                let [hx, hy] = self.p1;
                let ex = (hx * c).abs() + (hy * s).abs();
                let ey = (hx * s).abs() + (hy * c).abs();
                Some([x0 - ex, y0 - ey, x0 + ex, y0 + ey])
            }
            kind::ARC => {
                let e = self.r2 + self.r;
                Some([x0 - e, y0 - e, x0 + e, y0 + e])
            }
            _ => None,
        }
    }
}
