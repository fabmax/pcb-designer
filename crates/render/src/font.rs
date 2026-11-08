//! Minimal stroke (vector) font. Text turns into line segments that are drawn as capsules,
//! so it uses the regular primitive pipeline and scales without artifacts.
//!
//! Glyphs live on a 4×6 unit grid (cap height 6, baseline at y = 0). Each glyph is a list
//! of polylines separated by `|`; every point is two digits `xy`.
//! This is a placeholder font for milestone 0. It will be replaced by a complete font.

pub const CAP_UNITS: f32 = 6.0;
pub const ADVANCE_UNITS: f32 = 5.0;

fn glyph(c: char) -> Option<&'static str> {
    Some(match c.to_ascii_uppercase() {
        '0' => "10 30 41 45 36 16 05 01 10",
        '1' => "15 26 20 | 10 30",
        '2' => "05 16 36 45 44 00 40",
        '3' => "05 16 36 45 44 33 42 41 30 10 01 | 13 33",
        '4' => "30 36 02 42",
        '5' => "46 06 04 34 43 41 30 10 01",
        '6' => "36 16 05 01 10 30 41 42 33 03",
        '7' => "06 46 10",
        '8' => "13 04 05 16 36 45 44 33 13 02 01 10 30 41 42 33",
        '9' => "43 13 04 05 16 36 45 41 30 10 01",
        'A' => "00 04 26 44 40 | 02 42",
        'B' => "00 06 36 45 44 33 03 | 33 42 41 30 00",
        'C' => "45 36 16 05 01 10 30 41",
        'D' => "00 06 36 45 41 30 00",
        'E' => "46 06 00 40 | 03 33",
        'F' => "46 06 00 | 03 33",
        'G' => "45 36 16 05 01 10 30 41 43 23",
        'H' => "00 06 | 40 46 | 03 43",
        'I' => "10 30 | 20 26 | 16 36",
        'J' => "46 41 30 10 01 02",
        'K' => "00 06 | 46 02 | 13 40",
        'L' => "06 00 40",
        'M' => "00 06 23 46 40",
        'N' => "00 06 40 46",
        'O' => "10 30 41 45 36 16 05 01 10",
        'P' => "00 06 36 45 44 33 03",
        'Q' => "10 30 41 45 36 16 05 01 10 | 22 40",
        'R' => "00 06 36 45 44 33 03 | 23 40",
        'S' => "45 36 16 05 04 13 33 42 41 30 10 01",
        'T' => "06 46 | 26 20",
        'U' => "06 01 10 30 41 46",
        'V' => "06 20 46",
        'W' => "06 10 23 30 46",
        'X' => "00 46 | 06 40",
        'Y' => "06 23 46 | 23 20",
        'Z' => "06 46 00 40",
        '-' => "13 33",
        '+' => "03 43 | 21 25",
        '=' => "02 42 | 04 44",
        '.' => "20 20",
        ',' => "21 10",
        ':' => "22 22 | 24 24",
        '/' => "00 46",
        '_' => "00 40",
        '(' => "36 25 21 30",
        ')' => "16 25 21 10",
        '!' => "26 22 | 20 20",
        '?' => "05 16 36 45 44 23 22 | 20 20",
        ' ' => "",
        _ => return None,
    })
}

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

/// Width of `text` in world units for the given cap height.
pub fn text_width(text: &str, height: f32) -> f32 {
    let n = text.chars().count() as f32;
    if n == 0.0 {
        0.0
    } else {
        (n * ADVANCE_UNITS - 1.0) * height / CAP_UNITS
    }
}

/// Line segments for `text` with cap `height`, anchored at `origin`, rotated by
/// `rotation` radians around the anchor.
pub fn text_segments(
    text: &str,
    origin: [f32; 2],
    height: f32,
    rotation: f32,
    halign: HAlign,
    valign: VAlign,
    mut out: impl FnMut([f32; 2], [f32; 2]),
) {
    let unit = height / CAP_UNITS;
    let width = text_width(text, height);
    let dx = match halign {
        HAlign::Left => 0.0,
        HAlign::Center => -width * 0.5,
        HAlign::Right => -width,
    };
    let dy = match valign {
        VAlign::Bottom => 0.0,
        VAlign::Center => -height * 0.5,
        VAlign::Top => -height,
    };
    let (s, c) = rotation.sin_cos();
    let xf = |x: f32, y: f32| {
        let (x, y) = (x + dx, y + dy);
        [origin[0] + x * c - y * s, origin[1] + x * s + y * c]
    };

    for (i, ch) in text.chars().enumerate() {
        let Some(g) = glyph(ch).or_else(|| glyph('?')) else {
            continue;
        };
        let x0 = i as f32 * ADVANCE_UNITS * unit;
        for stroke in g.split('|') {
            let mut prev: Option<[f32; 2]> = None;
            for pt in stroke.split_whitespace() {
                let b = pt.as_bytes();
                let p = xf(x0 + (b[0] - b'0') as f32 * unit, (b[1] - b'0') as f32 * unit);
                match prev {
                    Some(q) => out(q, p),
                    None if stroke.split_whitespace().count() == 1 => out(p, p),
                    None => {}
                }
                prev = Some(p);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_glyphs_parse() {
        for c in (b' '..=b'~').map(char::from) {
            if let Some(g) = glyph(c) {
                for stroke in g.split('|') {
                    for pt in stroke.split_whitespace() {
                        let b = pt.as_bytes();
                        assert_eq!(b.len(), 2, "bad point {pt:?} in {c:?}");
                        assert!(b[0] <= b'4' && b[1] <= b'6', "out of grid {pt:?} in {c:?}");
                    }
                }
            }
        }
    }

    #[test]
    fn centered_text_is_symmetric() {
        let mut xs = vec![];
        text_segments("HH", [0.0, 0.0], 6.0, 0.0, HAlign::Center, VAlign::Center, |a, b| {
            xs.push(a[0]);
            xs.push(b[0]);
        });
        let min = xs.iter().cloned().fold(f32::MAX, f32::min);
        let max = xs.iter().cloned().fold(f32::MIN, f32::max);
        assert!((min + max).abs() < 1e-5);
    }
}
