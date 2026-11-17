//! Pad and pin arrays: copies of a selected pad (pin) laid out in a row, two rows (SOIC,
//! DIP), a grid (BGA) or around four sides (QFP, QFN), numbered on from it.

use model::*;

use crate::document::Document;
use crate::item::{Content, Elem, Item, update_item};
use crate::tools::increment_name;

/// The direction a row runs in.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Dir {
    Right,
    #[default]
    Down,
    Left,
    Up,
}

impl Dir {
    pub const ALL: [Dir; 4] = [Dir::Right, Dir::Down, Dir::Left, Dir::Up];

    pub fn label(self) -> &'static str {
        match self {
            Dir::Right => "Right",
            Dir::Down => "Down",
            Dir::Left => "Left",
            Dir::Up => "Up",
        }
    }

    fn unit(self) -> (i64, i64) {
        match self {
            Dir::Right => (1, 0),
            Dir::Down => (0, -1),
            Dir::Left => (-1, 0),
            Dir::Up => (0, 1),
        }
    }
}

/// How the copies are laid out. The selected pad is the first; lengths in nm.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Layout {
    Linear {
        count: usize,
        pitch: Coord,
        dir: Dir,
    },
    /// Two rows `span` apart (centre to centre); the second lies 90° counterclockwise from
    /// the first and runs back, so the numbers go round counterclockwise.
    DualRow {
        per_row: usize,
        pitch: Coord,
        span: Coord,
        dir: Dir,
    },
    /// Rows A, B, … (↓) of numbered columns (→); the selected pad is the top left one.
    Grid {
        rows: usize,
        cols: usize,
        pitch_x: Coord,
        pitch_y: Coord,
    },
    /// Four sides, numbered counterclockwise: `side_count` pads on the first and third
    /// side (`span` apart), `end_count` on the second and fourth (`end_span` apart).
    Quad {
        side_count: usize,
        end_count: usize,
        pitch: Coord,
        span: Coord,
        end_span: Coord,
        dir: Dir,
    },
}

impl Layout {
    pub fn count(&self) -> usize {
        match *self {
            Layout::Linear { count, .. } => count,
            Layout::DualRow { per_row, .. } => 2 * per_row,
            Layout::Grid { rows, cols, .. } => rows * cols,
            Layout::Quad { side_count, end_count, .. } => 2 * (side_count + end_count),
        }
    }
}

/// One element of an array: where, whether it is turned by 90° from the first, its name.
#[derive(Clone, Debug, PartialEq)]
pub struct Slot {
    pub pos: Point,
    pub turned: bool,
    pub name: String,
}

/// More than this is refused (a typo, not a part).
const MAX: usize = 10_000;

/// The slots of `layout` starting at the selected element (`pos`, `name`).
pub fn slots(layout: &Layout, pos: Point, name: &str) -> Result<Vec<Slot>, String> {
    let n = layout.count();
    if n == 0 {
        return Err("the array needs at least one pad".into());
    }
    if n > MAX {
        return Err(format!("at most {MAX} pads"));
    }
    let positive = |v: Coord, what: &str| if v > 0 { Ok(()) } else { Err(format!("the {what} must be positive")) };
    let at = |u: (i64, i64), v: (i64, i64), a: f64, b: f64| {
        pos + Point::new(
            (u.0 as f64 * a + v.0 as f64 * b).round() as Coord,
            (u.1 as f64 * a + v.1 as f64 * b).round() as Coord,
        )
    };
    let mut names = vec![name.to_string()];
    while names.len() < n {
        names.push(increment_name(names.last().expect("one at least")));
    }
    let slot = |pos, turned, i: usize| Slot { pos, turned, name: names[i].clone() };
    Ok(match *layout {
        Layout::Linear { count, pitch, dir } => {
            positive(pitch, "pitch")?;
            let u = dir.unit();
            (0..count).map(|i| slot(at(u, (0, 0), (i as Coord * pitch) as f64, 0.0), false, i)).collect()
        }
        Layout::DualRow { per_row, pitch, span, dir } => {
            positive(pitch, "pitch")?;
            positive(span, "row spacing")?;
            let (u, v) = (dir.unit(), ccw(dir.unit()));
            let p = pitch as f64;
            let first = (0..per_row).map(|i| at(u, v, i as f64 * p, 0.0));
            let second = (0..per_row).rev().map(|i| at(u, v, i as f64 * p, span as f64));
            first.chain(second).enumerate().map(|(i, q)| slot(q, false, i)).collect()
        }
        Layout::Quad { side_count, end_count, pitch, span, end_span, dir } => {
            positive(pitch, "pitch")?;
            positive(span, "span")?;
            positive(end_span, "end span")?;
            let (u, v) = (dir.unit(), ccw(dir.unit()));
            let p = pitch as f64;
            // Centre of the body in (u, v): halfway along the first side, half the span in.
            let cu = (side_count.max(1) - 1) as f64 * p / 2.0;
            let cv = span as f64 / 2.0;
            let half_end = (end_count.max(1) - 1) as f64 * p / 2.0;
            let mut out: Vec<(Point, bool)> = Vec::with_capacity(n);
            // 1: along u at v = 0. 2: along v at u = cu + end_span/2. 3: back along u at
            // v = span. 4: back along v at u = cu - end_span/2.
            out.extend((0..side_count).map(|i| (at(u, v, i as f64 * p, 0.0), false)));
            out.extend(
                (0..end_count).map(|i| (at(u, v, cu + end_span as f64 / 2.0, cv - half_end + i as f64 * p), true)),
            );
            out.extend((0..side_count).rev().map(|i| (at(u, v, i as f64 * p, span as f64), false)));
            out.extend(
                (0..end_count)
                    .rev()
                    .map(|i| (at(u, v, cu - end_span as f64 / 2.0, cv - half_end + i as f64 * p), true)),
            );
            out.into_iter().enumerate().map(|(i, (q, t))| slot(q, t, i)).collect()
        }
        Layout::Grid { rows, cols, pitch_x, pitch_y } => {
            positive(pitch_x, "column pitch")?;
            positive(pitch_y, "row pitch")?;
            let (row0, col0) = parse_ball(name).unwrap_or((0, 1));
            let mut out = Vec::with_capacity(n);
            for r in 0..rows {
                for c in 0..cols {
                    let q = pos + Point::new(c as Coord * pitch_x, -(r as Coord) * pitch_y);
                    out.push(Slot { pos: q, turned: false, name: format!("{}{}", row_name(row0 + r), col0 + c) });
                }
            }
            out
        }
    })
}

/// 90° counterclockwise.
fn ccw((x, y): (i64, i64)) -> (i64, i64) {
    (-y, x)
}

/// BGA row letters (JEDEC: no I, O, Q, S, X, Z).
const ROWS: &[u8] = b"ABCDEFGHJKLMNPRTUVWY";

/// Row `i`: A … Y, then AA, AB, …
pub fn row_name(i: usize) -> String {
    let n = ROWS.len();
    if i < n {
        (ROWS[i] as char).to_string()
    } else {
        format!("{}{}", ROWS[(i / n - 1) % n] as char, ROWS[i % n] as char)
    }
}

/// "C12" → (row 2, column 12). `None` for other names.
fn parse_ball(name: &str) -> Option<(usize, usize)> {
    let letters: String = name.chars().take_while(|c| c.is_ascii_uppercase()).collect();
    let number: usize = name[letters.len()..].parse().ok()?;
    let row = (0..ROWS.len() * (ROWS.len() + 1)).find(|&i| row_name(i) == letters)?;
    Some((row, number))
}

/// Names the array would give that other pads (pins) already have.
pub fn clashes(content: &Content, base: Elem, slots: &[Slot]) -> Vec<String> {
    let others: Vec<&str> = match (content, base) {
        (Content::Footprint(f), Elem::Pad(id)) => {
            f.pads.iter().filter(|p| p.id != id).map(|p| p.name.as_str()).collect()
        }
        (Content::Symbol(s), Elem::Pin(id)) => s.pins.iter().filter(|p| p.id != id).map(|p| p.name.as_str()).collect(),
        _ => Vec::new(),
    };
    let mut out: Vec<String> =
        slots.iter().filter(|s| others.contains(&s.name.as_str())).map(|s| s.name.clone()).collect();
    out.dedup();
    out
}

/// The pads of the array: the selected one first (renamed if the layout says so), then
/// copies with new ids (turned ones rotated by 90°).
pub fn pads(base: &Pad, slots: &[Slot]) -> Vec<Pad> {
    slots
        .iter()
        .enumerate()
        .map(|(i, s)| Pad {
            id: if i == 0 { base.id } else { Id::new() },
            name: s.name.clone(),
            pos: s.pos,
            rotation: if s.turned { base.rotation.rotated_by(Angle::DEG90) } else { base.rotation },
            ..base.clone()
        })
        .collect()
}

/// The pins of a (linear) array, as [`pads`].
pub fn pins(base: &Pin, slots: &[Slot]) -> Vec<Pin> {
    slots
        .iter()
        .enumerate()
        .map(|(i, s)| Pin {
            id: if i == 0 { base.id } else { Id::new() },
            name: s.name.clone(),
            pos: s.pos,
            ..base.clone()
        })
        .collect()
}

/// Make the array of `base` in `item` (one undo step). Returns the elements, the
/// selected one first.
pub fn apply(doc: &mut Document, item: Item, base: Elem, layout: &Layout) -> Result<Vec<Elem>, String> {
    update_item(doc, item, None, |c| match (c, base) {
        (Content::Footprint(f), Elem::Pad(id)) => {
            let pad = f.pads.get(id).ok_or("the pad no longer exists")?.clone();
            let all = pads(&pad, &slots(layout, pad.pos, &pad.name)?);
            let elems = all.iter().map(|p| Elem::Pad(p.id)).collect();
            for p in all {
                f.pads.insert(p);
            }
            Ok(elems)
        }
        (Content::Symbol(s), Elem::Pin(id)) => {
            if !matches!(layout, Layout::Linear { .. }) {
                return Err("pins only make linear arrays".into());
            }
            let pin = s.pins.get(id).ok_or("the pin no longer exists")?.clone();
            let all = pins(&pin, &slots(layout, pin.pos, &pin.name)?);
            let elems = all.iter().map(|p| Elem::Pin(p.id)).collect();
            for p in all {
                s.pins.insert(p);
            }
            Ok(elems)
        }
        _ => Err("select one pad or pin".into()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(s: &[Slot]) -> Vec<&str> {
        s.iter().map(|s| s.name.as_str()).collect()
    }

    #[test]
    fn linear_rows() {
        let s = slots(&Layout::Linear { count: 3, pitch: mm(1.0), dir: Dir::Right }, Point::ZERO, "4").unwrap();
        assert_eq!(names(&s), ["4", "5", "6"]);
        assert_eq!(s[2].pos, Point::mm(2.0, 0.0));
        assert!(slots(&Layout::Linear { count: 3, pitch: 0, dir: Dir::Right }, Point::ZERO, "1").is_err());
    }

    #[test]
    fn dual_rows_go_round_counterclockwise() {
        // SOIC-8: pin 1 top left, rows going down, the second row 5 mm to the right.
        let l = Layout::DualRow { per_row: 4, pitch: mm(1.27), span: mm(5.0), dir: Dir::Down };
        let s = slots(&l, Point::ZERO, "1").unwrap();
        assert_eq!(names(&s), ["1", "2", "3", "4", "5", "6", "7", "8"]);
        assert_eq!(s[3].pos, Point::mm(0.0, -3.81));
        assert_eq!(s[4].pos, Point::mm(5.0, -3.81), "pin 5 opposite pin 4");
        assert_eq!(s[7].pos, Point::mm(5.0, 0.0), "pin 8 opposite pin 1");
    }

    #[test]
    fn quads_have_four_sides() {
        // 2 pads per side, pitch 1, spans 4: centre at (2, -0.5).
        let l = Layout::Quad {
            side_count: 2,
            end_count: 2,
            pitch: mm(1.0),
            span: mm(4.0),
            end_span: mm(4.0),
            dir: Dir::Down,
        };
        let s = slots(&l, Point::ZERO, "1").unwrap();
        assert_eq!(s.len(), 8);
        let pos: Vec<Point> = s.iter().map(|s| s.pos).collect();
        assert_eq!(&pos[..2], [Point::ZERO, Point::mm(0.0, -1.0)], "left side, down");
        assert_eq!(&pos[2..4], [Point::mm(1.5, -2.5), Point::mm(2.5, -2.5)], "bottom, to the right");
        assert_eq!(&pos[4..6], [Point::mm(4.0, -1.0), Point::mm(4.0, 0.0)], "right side, up");
        assert_eq!(&pos[6..], [Point::mm(2.5, 1.5), Point::mm(1.5, 1.5)], "top, to the left");
        assert!(s[2].turned && !s[0].turned && !s[4].turned && s[6].turned);
    }

    #[test]
    fn grids_name_rows_with_letters() {
        assert_eq!(row_name(7), "H");
        assert_eq!(row_name(8), "J", "no I");
        assert_eq!(row_name(20), "AA");
        let l = Layout::Grid { rows: 2, cols: 2, pitch_x: mm(0.8), pitch_y: mm(0.8) };
        let s = slots(&l, Point::ZERO, "1").unwrap();
        assert_eq!(names(&s), ["A1", "A2", "B1", "B2"]);
        assert_eq!(s[3].pos, Point::mm(0.8, -0.8));
        let s = slots(&l, Point::ZERO, "H3").unwrap();
        assert_eq!(names(&s), ["H3", "H4", "J3", "J4"]);
    }

    #[test]
    fn pads_copy_the_selected_one() {
        let base = Pad {
            id: Id::new(),
            name: "1".into(),
            pos: Point::ZERO,
            rotation: Angle::ZERO,
            size: Size::mm(1.0, 0.5),
            shape: PadShape::Rect,
            kind: PadKind::Smd,
            back: false,
            mask: true,
            paste: true,
            mask_margin: Some(1000),
            paste_margin: None,
        };
        let l = Layout::Quad {
            side_count: 1,
            end_count: 1,
            pitch: mm(1.0),
            span: mm(2.0),
            end_span: mm(2.0),
            dir: Dir::Down,
        };
        let p = pads(&base, &slots(&l, Point::ZERO, "1").unwrap());
        assert_eq!(p[0].id, base.id);
        assert!(p[1].id != base.id && p[1].rotation == Angle::DEG90 && p[1].mask_margin == Some(1000));
    }
}
