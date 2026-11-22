//! Net names on the board at high zoom: below the number on pads with a net, and along
//! straight tracks wide enough for the text on screen. Only what's near the viewport is
//! drawn, so a large board doesn't turn into millions of text triangles; the view
//! rebuilds them as the viewport leaves the area drawn, the zoom steps or the board
//! changes ([`crate::view::View::update_net_labels`]).

use editor_core::copper::Geo;
use model::{Coord, Footprint, FootprintInstance, Point, Rect, Track};
use render::font::{self, HAlign, Placement, VAlign};
use render::{Color, Instance};

/// Pixels per mm from which the board view draws labels at all (tracks: when their text
/// is large enough too).
pub const LABEL_ZOOM: f64 = 25.0;

/// Pixels per mm from which pads show their net (and the labels draw pad numbers, the
/// static ones hidden).
pub const PAD_LABEL_ZOOM: f64 = 50.0;

/// Smallest label on screen, in pixels (points): smaller ones aren't drawn.
const MIN_PIXELS: f64 = 8.0;

/// A track's label height as part of its width.
const TRACK_TEXT: f64 = 0.6;

/// A net name's height on a pad as part of the pad number's.
const PAD_NAME: f64 = 0.6;

pub fn label_color() -> Color {
    Color::rgba(0xff, 0xff, 0xff, 0xc0)
}

fn mm(c: Coord) -> f64 {
    c as f64 * 1e-6
}

fn text(out: &mut Vec<Instance>, s: &str, at: [f64; 2], height: f64, rotation: f64, color: Color) {
    let p = Placement {
        origin: [at[0] as f32, at[1] as f32],
        height: height as f32,
        rotation: rotation as f32,
        halign: HAlign::Center,
        valign: VAlign::Center,
        bold: false,
        mirror: false,
    };
    font::text_triangles(s, &p, |([a, b, c], edges)| out.push(Instance::triangle(a, b, c, edges, color)));
}

/// Width of `s` at height 1.
fn width(s: &str) -> f64 {
    font::text_width(s, 1.0) as f64
}

/// The labels of the pads of footprint `fp` placed as `inst`: the number, and below it
/// the net (`net(pad id)`) where the pad has one and it's readable at `zoom`. Pad numbers
/// are no larger than `max_number` (as the static ones).
pub fn pad_labels<'a>(
    fp: &Footprint,
    inst: &FootprintInstance,
    net: impl Fn(model::Id) -> Option<&'a str>,
    max_number: Coord,
    zoom: f64,
    out: &mut Vec<Instance>,
) {
    let t = inst.transform();
    let color = label_color();
    for pad in fp.pads.iter() {
        let center = t.point(pad.pos);
        let c = [mm(center.x), mm(center.y)];
        let number = (mm(pad.size.w.min(pad.size.h)) * 0.45).min(mm(max_number));
        let name = net(pad.id).filter(|n| !n.is_empty());
        // The pad's extent along and across its text, on screen axes: text runs along
        // the longer side (turned 90° on a tall pad).
        let turn = (pad.rotation.degrees() + t.rotation.degrees()).rem_euclid(180.0);
        let square = turn.rem_euclid(90.0).min(90.0 - turn.rem_euclid(90.0)) < 1.0;
        let (w, h) = (mm(pad.size.w), mm(pad.size.h));
        let (sx, sy) = match (square, (turn - 90.0).abs() < 1.0) {
            (true, false) => (w, h),
            (true, true) => (h, w),
            (false, _) => (w.min(h), w.min(h)),
        };
        let tall = sy > sx * 1.2;
        let (along, across, rotation) = if tall { (sy, sx, std::f64::consts::FRAC_PI_2) } else { (sx, sy, 0.0) };
        let stacked = name.and_then(|name| stack(&pad.name, name, number, along, across, zoom));
        match stacked {
            Some((n, s, gap)) => {
                // Number above, name below, the pair centred on the pad ("up" is
                // -x for turned text).
                let total = n + gap + s;
                let up = if tall { [-1.0, 0.0] } else { [0.0, 1.0] };
                let at = |d: f64| [c[0] + up[0] * d, c[1] + up[1] * d];
                if !pad.name.is_empty() {
                    text(out, &pad.name, at(total / 2.0 - n / 2.0), n, rotation, color);
                }
                text(out, name.unwrap_or_default(), at(-(total / 2.0 - s / 2.0)), s, rotation, color);
            }
            None if !pad.name.is_empty() => text(out, &pad.name, c, number, 0.0, color),
            None => {}
        }
    }
}

/// Heights of the number and the net name stacked on a pad, and the gap between them:
/// the name at most [`PAD_NAME`] of the number, both made smaller to fit the pad. None
/// if the name would be too small to read at `zoom`.
fn stack(number: &str, name: &str, size: f64, along: f64, across: f64, zoom: f64) -> Option<(f64, f64, f64)> {
    let fit = |s: &str, h: f64| if s.is_empty() { h } else { h.min(along * 0.9 / width(s)) };
    let n = fit(number, size);
    let s = fit(name, size * PAD_NAME);
    let gap = s * 0.3;
    let scale = (across * 0.85 / (n + gap + s)).min(1.0);
    let (n, s, gap) = (n * scale, s * scale, gap * scale);
    (s * zoom >= MIN_PIXELS).then_some((n, s, gap))
}

/// Labels of straight track `tr` (net `name`) inside `area`: along the track, upright,
/// where the text fits between its ends, repeated on a long track at fixed places (so
/// they don't move as the view pans), and not where `blocked(the text's box)` (pads).
/// None while the text would be too small at `zoom`.
pub fn track_labels(
    tr: &Track,
    name: &str,
    area: Rect,
    zoom: f64,
    blocked: &dyn Fn(&Geo) -> bool,
    out: &mut Vec<Instance>,
) {
    if tr.mid.is_some() || name.is_empty() {
        return;
    }
    let h = mm(tr.width) * TRACK_TEXT;
    if h * zoom < MIN_PIXELS {
        return;
    }
    let (a, b) = ([mm(tr.a.x), mm(tr.a.y)], [mm(tr.b.x), mm(tr.b.y)]);
    let d = [b[0] - a[0], b[1] - a[1]];
    let len = d[0].hypot(d[1]);
    let text_len = width(name) * h;
    let margin = mm(tr.width);
    let room = len - 2.0 * margin;
    if text_len > room {
        return;
    }
    let mut angle = d[1].atan2(d[0]);
    // Upright: reading left to right, or bottom to top.
    if angle > std::f64::consts::FRAC_PI_2 + 1e-9 || angle <= -std::f64::consts::FRAC_PI_2 + 1e-9 {
        angle += std::f64::consts::PI;
    }
    // Places at the middle and every `period` either side, inside the track's ends.
    let period = text_len + 30.0 * h;
    let reach = ((room - text_len) / 2.0 / period).floor() as i64;
    let area = area.grow((text_len / 2.0 * 1e6) as Coord);
    let color = label_color();
    for k in -reach..=reach {
        let s = len / 2.0 + k as f64 * period;
        let at = [a[0] + d[0] * s / len, a[1] + d[1] * s / len];
        if !area.contains(Point::mm(at[0], at[1])) {
            continue;
        }
        let half = text_len / 2.0 / len;
        let end = |f: f64| Point::mm(at[0] + d[0] * f, at[1] + d[1] * f);
        if blocked(&Geo::Capsule { a: end(-half), b: end(half), r: (h / 2.0 * 1e6) as Coord }) {
            continue;
        }
        text(out, name, at, h, angle, color);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_name_too_small_to_read_is_left_out() {
        // A 1 mm pad: number 0.45 mm, name 0.27 mm before fitting.
        assert!(stack("1", "GND", 0.45, 1.0, 1.0, 10.0).is_none());
        let (n, s, gap) = stack("1", "GND", 0.45, 1.0, 1.0, 100.0).unwrap();
        assert!(n + gap + s <= 0.85 + 1e-9, "{n} {s} {gap}");
        assert!(s < n);
        // A long name shrinks to the pad's length.
        let (_, s, _) = stack("1", "A_VERY_LONG_NET_NAME", 0.45, 1.0, 1.0, 1000.0).unwrap();
        assert!(width("A_VERY_LONG_NET_NAME") * s <= 0.9 + 1e-9);
    }

    #[test]
    fn track_labels_repeat_upright_and_keep_off_blocked_places() {
        use model::{Id, LayerId, mm};
        // 100 mm, 1 mm wide, drawn right to left.
        let tr = Track {
            id: Id::new(),
            layer: LayerId::TOP_COPPER,
            a: Point::new(mm(100.0), 0),
            b: Point::ZERO,
            width: mm(1.0),
            mid: None,
            locked: false,
        };
        let all = Rect::around(Point::ZERO, mm(1000.0));
        let count = |area: Rect, zoom: f64, blocked: &dyn Fn(&Geo) -> bool| {
            let mut out = Vec::new();
            track_labels(&tr, "GND", area, zoom, blocked, &mut out);
            out
        };
        let free = |_: &Geo| false;
        assert!(count(all, 5.0, &free).is_empty(), "too small");
        let one = count(Rect::around(Point::new(mm(50.0), 0), mm(1.0)), 50.0, &free);
        let many = count(all, 50.0, &free);
        assert!(!one.is_empty() && many.len() >= 3 * one.len(), "{} {}", one.len(), many.len());
        assert!(count(all, 50.0, &|_| true).is_empty());
        // Upright: read left to right although the track runs right to left.
        let xs: Vec<f32> = one.iter().map(|i| i.p0[0]).collect();
        let (lo, hi) = xs.iter().fold((f32::MAX, f32::MIN), |(a, b), &x| (a.min(x), b.max(x)));
        assert!(hi - lo > 0.5, "spans along x");
        let first = one.first().unwrap().p0[0];
        assert!(first < 50.0, "the first glyph is on the left: {first}");
    }
}
