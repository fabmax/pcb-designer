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
        '[' => "36 26 20 30",
        ']' => "16 26 20 10",
        '{' => "36 26 24 13 22 20 30",
        '}' => "16 26 24 33 22 20 10",
        '<' => "45 03 41",
        '>' => "05 43 01",
        '~' => "04 15 34 45",
        '#' => "10 16 | 30 36 | 02 42 | 04 44",
        '*' => "13 33 | 22 24 | 12 34 | 14 32",
        '\'' => "25 24",
        '"' => "15 14 | 35 34",
        '&' => "40 05 16 26 35 34 02 01 10 20 42",
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

/// Distance between the baselines of two lines, in cap heights.
pub const LINE_SPACING: f32 = 1.6;

/// Width of `text` in world units for the given cap height: its widest line.
pub fn text_width(text: &str, height: f32) -> f32 {
    text.split('\n').map(|l| line_width(l, height)).fold(0.0, f32::max)
}

/// Advance of a character, in grid units.
fn advance(_c: char) -> f32 {
    ADVANCE_UNITS
}

fn line_width(line: &str, height: f32) -> f32 {
    let last = line.chars().last();
    let w: f32 = line.chars().map(advance).sum();
    // The last glyph's spacing doesn't count.
    match last {
        None => 0.0,
        Some(_) => (w - 1.0) * height / CAP_UNITS,
    }
}

/// Caret positions of a line: the x of every character boundary (`len + 1` values, the
/// first 0), in world units for cap `height`, before alignment.
pub fn caret_positions(line: &str, height: f32) -> Vec<f32> {
    let unit = height / CAP_UNITS;
    let mut x = 0.0;
    let mut out = vec![0.0];
    for c in line.chars() {
        x += advance(c) * unit;
        out.push(x);
    }
    // The caret after the last character sits at the glyph's end, not its spacing.
    if out.len() > 1
        && let Some(last) = out.last_mut()
    {
        *last -= unit;
    }
    out
}

/// Where each line of `text` starts: its left end on the baseline, relative to the anchor
/// (before rotation). Lines stack downwards (−y), aligned by `halign`; `valign` places the
/// block (`Top`: the first line's top, `Bottom`: the last line's baseline).
pub fn line_origins(text: &str, height: f32, halign: HAlign, valign: VAlign) -> Vec<[f32; 2]> {
    let lines: Vec<&str> = text.split('\n').collect();
    let pitch = height * LINE_SPACING;
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

/// Line segments for `text` with cap `height`, anchored at `origin`, rotated by
/// `rotation` radians around the anchor, laid out by [`line_origins`].
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
    let (s, c) = rotation.sin_cos();
    let origins = line_origins(text, height, halign, valign);
    for (line, [dx, dy]) in text.split('\n').zip(origins) {
        let xf = |x: f32, y: f32| {
            let (x, y) = (x + dx, y + dy);
            [origin[0] + x * c - y * s, origin[1] + x * s + y * c]
        };
        let carets = caret_positions(line, height);
        for (i, ch) in line.chars().enumerate() {
            let Some(g) = glyph(ch).or_else(|| glyph('?')) else {
                continue;
            };
            let x0 = carets[i];
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
    fn lines_stack_downwards() {
        let mut ys = vec![];
        text_segments("I\nI", [0.0, 0.0], 6.0, 0.0, HAlign::Left, VAlign::Top, |a, b| ys.extend([a[1], b[1]]));
        let top = ys.iter().copied().fold(f32::MIN, f32::max);
        let bottom = ys.iter().copied().fold(f32::MAX, f32::min);
        assert!(top.abs() < 1e-4, "the first line hangs from the anchor: {top}");
        assert!((bottom + 6.0 + 6.0 * LINE_SPACING).abs() < 1e-4, "{bottom}");
        assert_eq!(text_width("II\nIIII", 6.0), text_width("IIII", 6.0));
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
