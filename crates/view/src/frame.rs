//! The drawing frame of a schematic sheet: the paper's edge, a border with a zone grid
//! (numbers along the top and bottom, letters down the sides) and the title block in the
//! lower right corner. Drawn as one scene item; not selectable.

use model::{Coord, HAlign, Id, Point, Project, Rect, Text, VAlign};
use render::{Color, LayerId};

use crate::draw::Painter;
use crate::style::pal;

const fn mm(v: f64) -> Coord {
    (v * 1e6) as Coord
}

/// Paper edge to border.
pub const MARGIN: Coord = mm(10.0);
/// Width of the zone label band inside the border.
pub const BAND: Coord = mm(2.0);
/// Nominal zone size (the zones divide the border evenly).
const ZONE: Coord = mm(50.0);
/// Width of the title block.
pub const TITLE_WIDTH: Coord = mm(110.0);
const LINE: Coord = mm(0.2);
const SMALL: Coord = mm(1.5);
const BIG: Coord = mm(2.0);
/// Height of the title block (KiCad's default is about as tall, so text placed on top of
/// it in KiCad lands in its comment area here too).
pub const TITLE_HEIGHT: Coord = mm(32.0);

/// Draw the frame of `sheet` (nothing for a sheet without page).
pub(crate) fn draw(project: &Project, sheet: Id, layer: LayerId, p: &mut Painter) {
    let s = &project.schematic;
    let Some(page) = s.sheets.get(sheet).and_then(|sh| sh.page.rect()) else { return };
    let (frame, ink) = (pal().sch_frame, pal().sch_page);
    rect(p, layer, page, LINE, ink);
    let outer = shrink(page, MARGIN);
    if outer.width() <= 2 * BAND || outer.height() <= 2 * BAND {
        return; // too small for a frame
    }
    let inner = shrink(outer, BAND);
    rect(p, layer, outer, LINE, frame);
    rect(p, layer, inner, LINE, frame);

    // Zone grid: numbers left to right, letters top to bottom.
    let cols = zones(inner.width());
    let rows = zones(inner.height());
    for i in 0..cols {
        let x0 = inner.min.x + inner.width() * i as Coord / cols as Coord;
        let x1 = inner.min.x + inner.width() * (i + 1) as Coord / cols as Coord;
        let label = (i + 1).to_string();
        for (y_band, y_edge) in [(inner.max.y, outer.max.y), (outer.min.y, inner.min.y)] {
            if i > 0 {
                p.line(layer, Point::new(x0, y_band), Point::new(x0, y_edge), LINE, frame);
            }
            let mid = Point::new((x0 + x1) / 2, (y_band + y_edge) / 2);
            text(p, layer, &label, mid, SMALL, HAlign::Center, VAlign::Center, frame);
        }
    }
    for i in 0..rows {
        let y0 = inner.max.y - inner.height() * i as Coord / rows as Coord;
        let y1 = inner.max.y - inner.height() * (i + 1) as Coord / rows as Coord;
        let label = zone_letter(i);
        for (x_band, x_edge) in [(outer.min.x, inner.min.x), (inner.max.x, outer.max.x)] {
            if i > 0 {
                p.line(layer, Point::new(x_band, y0), Point::new(x_edge, y0), LINE, frame);
            }
            let mid = Point::new((x_band + x_edge) / 2, (y0 + y1) / 2);
            text(p, layer, &label, mid, SMALL, HAlign::Center, VAlign::Center, frame);
        }
    }

    title_block(project, sheet, inner, layer, p, frame);
}

/// The title block in the lower right corner of `inner`, rows from the bottom up.
fn title_block(project: &Project, sheet: Id, inner: Rect, layer: LayerId, p: &mut Painter, c: Color) {
    let s = &project.schematic;
    let tb = s.title_block();
    let x1 = inner.max.x;
    let x0 = (x1 - TITLE_WIDTH).max(inner.min.x);
    let pad = mm(1.0);
    let row = mm(3.5);
    let mut y = inner.min.y;
    let hline = |p: &mut Painter, y: Coord| p.line(layer, Point::new(x0, y), Point::new(x1, y), LINE, c);
    let left = |y: Coord, h: Coord| Point::new(x0 + pad, y + h / 2);
    let mid_v = VAlign::Center;

    // Generator | page n / m
    let split = x1 - mm(30.0);
    let pages = s.page_order();
    let n = pages.iter().position(|&i| i == sheet).map_or(1, |i| i + 1);
    text(p, layer, "PCB Designer", left(y, row), SMALL, HAlign::Left, mid_v, c);
    let page = format!("Page {n} / {}", pages.len().max(1));
    text(p, layer, &page, Point::new(split + pad, y + row / 2), SMALL, HAlign::Left, mid_v, c);
    p.line(layer, Point::new(split, y), Point::new(split, y + row), LINE, c);
    y += row;
    hline(p, y);

    // Size | date | revision
    let size = s.sheets.get(sheet).map(|sh| {
        let page = sh.page;
        let name = page.size.label();
        match page.dimensions() {
            Some(d) if matches!(page.size, model::PageSize::Custom { .. }) => {
                format!("{name} {:.0} × {:.0} mm", d.w as f64 * 1e-6, d.h as f64 * 1e-6)
            }
            _ if page.portrait => format!("{name} portrait"),
            _ => name.to_string(),
        }
    });
    let date_x = x0 + mm(32.0);
    text(p, layer, &format!("Size: {}", size.unwrap_or_default()), left(y, row), SMALL, HAlign::Left, mid_v, c);
    text(p, layer, &format!("Date: {}", tb.date), Point::new(date_x + pad, y + row / 2), SMALL, HAlign::Left, mid_v, c);
    text(
        p,
        layer,
        &format!("Rev: {}", tb.revision),
        Point::new(split + pad, y + row / 2),
        SMALL,
        HAlign::Left,
        mid_v,
        c,
    );
    for x in [date_x, split] {
        p.line(layer, Point::new(x, y), Point::new(x, y + row), LINE, c);
    }
    y += row;
    hline(p, y);

    // Title
    let title_h = mm(5.0);
    text(p, layer, "Title:", Point::new(x0 + pad, y + title_h - pad), SMALL, HAlign::Left, VAlign::Top, c);
    text(p, layer, &tb.title, Point::new(x0 + mm(12.0), y + title_h / 2), BIG, HAlign::Left, mid_v, c);
    y += title_h;
    hline(p, y);

    // Sheet name and path
    let path = s.sheet_path(sheet);
    let name = s.sheets.get(sheet).map_or("", |sh| sh.name.as_str());
    let sheet_text = if path.is_empty() { format!("Sheet: {name}") } else { format!("Sheet: {path}") };
    text(p, layer, &sheet_text, left(y, row), SMALL, HAlign::Left, mid_v, c);
    y += row;
    hline(p, y);

    // Company and comments, top down: comments 1… then the company.
    let line_h = mm(3.2);
    let mut lines: Vec<&str> = if tb.comment.is_empty() { Vec::new() } else { tb.comment.lines().collect() };
    lines.push(&tb.company);
    let top = (inner.min.y + TITLE_HEIGHT).max(y + line_h * lines.len() as Coord + pad);
    for (i, l) in lines.iter().enumerate() {
        let baseline = top - pad - line_h * (i as Coord + 1) + (line_h - SMALL) / 2;
        text(p, layer, l, Point::new(x0 + pad, baseline), SMALL, HAlign::Left, VAlign::Bottom, c);
    }
    // The block's outline (its left and top edges; the border is its right and bottom).
    p.line(layer, Point::new(x0, inner.min.y), Point::new(x0, top), LINE, c);
    p.line(layer, Point::new(x0, top), Point::new(x1, top), LINE, c);
}

/// Number of zones along a border side of length `len`.
fn zones(len: Coord) -> i64 {
    ((len as f64 / ZONE as f64).round() as i64).max(1)
}

/// Zone letters: A … Z, then AA, AB, …
fn zone_letter(i: i64) -> String {
    let letter = |n: i64| char::from(b'A' + n as u8);
    if i < 26 { letter(i).to_string() } else { format!("{}{}", letter(i / 26 - 1), letter(i % 26)) }
}

fn shrink(r: Rect, d: Coord) -> Rect {
    Rect::from_points(Point::new(r.min.x + d, r.min.y + d), Point::new(r.max.x - d, r.max.y - d))
}

fn rect(p: &mut Painter, layer: LayerId, r: Rect, w: Coord, c: Color) {
    let corners = [r.min, Point::new(r.max.x, r.min.y), r.max, Point::new(r.min.x, r.max.y), r.min];
    for e in corners.windows(2) {
        p.line(layer, e[0], e[1], w, c);
    }
}

#[allow(clippy::too_many_arguments)]
fn text(p: &mut Painter, layer: LayerId, s: &str, pos: Point, size: Coord, h: HAlign, v: VAlign, c: Color) {
    if s.is_empty() {
        return;
    }
    let t = Text { halign: h, valign: v, ..Text::new(s, pos, size) };
    p.text(layer, &t, c, &|s| s.to_string(), false);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zones_divide_the_border() {
        assert_eq!(zones(mm(273.0)), 5); // A4 landscape inside the border
        assert_eq!(zones(mm(10.0)), 1);
        assert_eq!(zone_letter(0), "A");
        assert_eq!(zone_letter(25), "Z");
        assert_eq!(zone_letter(26), "AA");
    }
}
