//! Helpers shared by the symbol and footprint converters.

use model::*;

use crate::sexpr::Node;

/// Deterministic id from a path of names, so that importing the same KiCad object again
/// produces the same ids (and "update from library" works for projects using it).
pub fn kid(parts: &[&str]) -> Id {
    // Two independent 64-bit FNV-1a hashes.
    let mut a: u64 = 0xcbf2_9ce4_8422_2325;
    let mut b: u64 = 0x8422_2325_cbf2_9ce4;
    for p in parts {
        for byte in p.bytes().chain([0xff]) {
            a = (a ^ byte as u64).wrapping_mul(0x0000_0100_0000_01b3);
            b = (b ^ byte as u64).wrapping_mul(0x0000_0100_0000_01b3).rotate_left(5);
        }
    }
    // Shape it like a UUID v8 (custom) so tools that inspect ids don't choke.
    let mut v = ((a as u128) << 64) | b as u128;
    v = (v & !(0xf << 76)) | (0x8 << 76);
    v = (v & !(0x3 << 62)) | (0x2 << 62);
    Id::from_u128(v)
}

/// mm → nm.
pub fn nm(v: f64) -> Coord {
    (v * 1e6).round() as Coord
}

/// A point in KiCad coordinates. Symbols use y up like this application; footprints use
/// y down and are flipped (`flip_y`).
pub fn point(x: f64, y: f64, flip_y: bool) -> Point {
    Point::new(nm(x), nm(if flip_y { -y } else { y }))
}

pub fn child_point(n: &Node, head: &str, flip_y: bool) -> Option<Point> {
    let (x, y) = n.xy(head)?;
    Some(point(x, y, flip_y))
}

pub fn pts(n: &Node, flip_y: bool) -> Vec<Point> {
    n.child("pts")
        .map(|p| p.children("xy").filter_map(|xy| Some(point(xy.num(0)?, xy.num(1)?, flip_y))).collect())
        .unwrap_or_default()
}

/// Stroke width: `(stroke (width w))` (KiCad 6+) or `(width w)` (older).
pub fn stroke_width(n: &Node) -> Option<f64> {
    n.child("stroke").and_then(|s| s.child("width")).or_else(|| n.child("width")).and_then(|w| w.num(0))
}

/// Arc through three points. `None` if they are (nearly) collinear.
pub fn arc3(start: Point, mid: Point, end: Point, width: Coord) -> Option<Shape> {
    let (ax, ay) = (start.x as f64, start.y as f64);
    let (bx, by) = (mid.x as f64, mid.y as f64);
    let (cx, cy) = (end.x as f64, end.y as f64);
    let d = 2.0 * (ax * (by - cy) + bx * (cy - ay) + cx * (ay - by));
    if d.abs() < 1e-3 {
        return None;
    }
    let ux = ((ax * ax + ay * ay) * (by - cy) + (bx * bx + by * by) * (cy - ay) + (cx * cx + cy * cy) * (ay - by)) / d;
    let uy = ((ax * ax + ay * ay) * (cx - bx) + (bx * bx + by * by) * (ax - cx) + (cx * cx + cy * cy) * (bx - ax)) / d;
    let radius = ((ax - ux).powi(2) + (ay - uy).powi(2)).sqrt();
    let ang = |x: f64, y: f64| (y - uy).atan2(x - ux).to_degrees().rem_euclid(360.0);
    let (s, m, e) = (ang(ax, ay), ang(bx, by), ang(cx, cy));
    let ccw = (e - s).rem_euclid(360.0);
    let ccw = if ccw == 0.0 { 360.0 } else { ccw };
    let sweep = if (m - s).rem_euclid(360.0) <= ccw { ccw } else { ccw - 360.0 };
    Some(Shape::Arc {
        center: Point::new(ux.round() as Coord, uy.round() as Coord),
        radius: radius.round() as Coord,
        start: Angle::deg(s),
        sweep,
        width,
    })
}

/// Cubic Bézier as a polyline.
pub fn bezier(p: &[Point]) -> Vec<Point> {
    if p.len() != 4 {
        return p.to_vec();
    }
    (0..=16)
        .map(|i| {
            let t = i as f64 / 16.0;
            let u = 1.0 - t;
            let w = [u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t];
            let x = (0..4).map(|k| w[k] * p[k].x as f64).sum::<f64>();
            let y = (0..4).map(|k| w[k] * p[k].y as f64).sum::<f64>();
            Point::new(x.round() as Coord, y.round() as Coord)
        })
        .collect()
}

/// Text with KiCad `(effects (font (size h w) (thickness t)) (justify …) (hide))`.
/// Returns `None` for hidden text.
pub fn text(content: String, at: &Node, effects: Option<&Node>, flip_y: bool, angle_scale: f64) -> Option<Text> {
    let pos = point(at.num(0)?, at.num(1)?, flip_y);
    let angle = at.num(2).unwrap_or(0.0) * angle_scale;
    let font = effects.and_then(|e| e.child("font"));
    let size = font.and_then(|f| f.child("size")).and_then(|s| s.num(0)).unwrap_or(1.27);
    let thickness = font.and_then(|f| f.child("thickness")).and_then(|t| t.num(0));
    if effects.is_some_and(|e| e.flag("hide")) {
        return None;
    }
    let mut t = Text::new(content, pos, nm(size));
    if let Some(th) = thickness
        && th > 0.0
    {
        t.width = nm(th);
    }
    t.rotation = Angle::deg(angle);
    t.halign = HAlign::Center;
    t.valign = VAlign::Center;
    if let Some(j) = effects.and_then(|e| e.child("justify")) {
        for a in j.items().iter().skip(1).filter_map(Node::as_atom) {
            match a {
                "left" => t.halign = HAlign::Left,
                "right" => t.halign = HAlign::Right,
                // In y-down footprint coordinates "top" is still visually on top.
                "top" => t.valign = VAlign::Top,
                "bottom" => t.valign = VAlign::Bottom,
                _ => {}
            }
        }
    }
    Some(t)
}

/// KiCad text markup: `~{RESET}` (overbar), `_{x}`, `^{x}` → plain text.
pub fn plain(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    // For each open brace: whether it belongs to markup (and its `}` is dropped).
    let mut open: Vec<bool> = Vec::new();
    while let Some(c) = chars.next() {
        if matches!(c, '~' | '_' | '^') && chars.peek() == Some(&'{') {
            chars.next();
            open.push(true);
            continue;
        }
        match c {
            '{' => open.push(false),
            '}' if open.pop() == Some(true) => continue,
            _ => {}
        }
        out.push(c);
    }
    out
}

/// KiCad wildcard match (`*` and `?`), ASCII case-insensitive. Allocation free: it runs
/// for every footprint name when looking for footprint candidates.
pub fn glob(pattern: &str, name: &str) -> bool {
    let (p, n) = (pattern.as_bytes(), name.as_bytes());
    let (mut pi, mut ni, mut star, mut mark) = (0, 0, None, 0);
    while ni < n.len() {
        if pi < p.len() && p[pi] != b'*' && (p[pi] == b'?' || p[pi].eq_ignore_ascii_case(&n[ni])) {
            pi += 1;
            ni += 1;
        } else if pi < p.len() && p[pi] == b'*' {
            star = Some(pi);
            mark = ni;
            pi += 1;
        } else if let Some(s) = star {
            pi = s + 1;
            mark += 1;
            ni = mark;
        } else {
            return false;
        }
    }
    p[pi..].iter().all(|&c| c == b'*')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_stable_and_distinct() {
        assert_eq!(kid(&["sym", "Device", "R"]), kid(&["sym", "Device", "R"]));
        assert_ne!(kid(&["sym", "Device", "R"]), kid(&["sym", "DeviceR", ""]));
    }

    #[test]
    fn arc_through_three_points() {
        // Quarter circle, counterclockwise from (1,0) to (0,1).
        let s = std::f64::consts::FRAC_1_SQRT_2;
        let Some(Shape::Arc { center, radius, start, sweep, .. }) =
            arc3(point(1.0, 0.0, false), point(s, s, false), point(0.0, 1.0, false), 0)
        else {
            panic!()
        };
        assert!(center.length() < 2.0 && (radius - 1_000_000).abs() < 2);
        let s0 = start.degrees().rem_euclid(360.0);
        assert!(s0.min(360.0 - s0) < 1e-3 && (sweep - 90.0).abs() < 1e-3, "{start:?} {sweep}");
        // The same arc given the other way round sweeps clockwise.
        let Some(Shape::Arc { sweep, .. }) =
            arc3(point(0.0, 1.0, false), point(s, s, false), point(1.0, 0.0, false), 0)
        else {
            panic!()
        };
        assert!((sweep + 90.0).abs() < 1e-3, "{sweep}");
        assert!(arc3(point(0.0, 0.0, false), point(1.0, 0.0, false), point(2.0, 0.0, false), 0).is_none());
    }

    #[test]
    fn markup_and_globs() {
        assert_eq!(plain("~{RESET}"), "RESET");
        assert_eq!(plain("V_{CC}"), "VCC");
        assert_eq!(plain("${REFERENCE}"), "${REFERENCE}");
        assert_eq!(plain("~{CS}_{${X}}"), "CS${X}");
        assert!(glob("SOIC*3.9x4.9mm*P1.27mm*", "SOIC-8_3.9x4.9mm_P1.27mm"));
        assert!(glob("R_*", "r_0603_1608Metric"));
        assert!(!glob("DIP*W7.62mm*", "SOIC-8"));
        assert!(glob("TO?92*", "TO-92_Inline"));
    }
}
