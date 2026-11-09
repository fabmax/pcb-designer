//! Tool icons, drawn as vectors (crisp at any scale, follow the theme colours).
//! Coordinates are in a unit square (0..1, y down) mapped onto the icon rectangle.

use egui::{Color32, Painter, Pos2, Rect, Shape, Stroke, vec2};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Icon {
    Select,
    Wire,
    Label,
    Power,
    Junction,
    Place,
    Route,
    Via,
    Outline,
    Zone,
    Pin,
    Pad,
    Line,
    Rect,
    Circle,
    Text,
    Layers,
    Grid,
    Fit,
}

/// Paint `icon` into `rect` (square) in `color`.
pub fn paint(painter: &Painter, rect: Rect, icon: Icon, color: Color32) {
    let s = rect.width().min(rect.height());
    let o = rect.center() - vec2(s, s) / 2.0;
    let p = |x: f32, y: f32| -> Pos2 { o + vec2(x * s, y * s) };
    let w = (s / 10.0).max(1.4);
    let line = Stroke::new(w, color);
    let thick = Stroke::new(w * 1.8, color);
    let faint = color.gamma_multiply(0.35);
    let seg = |a: Pos2, b: Pos2, st: Stroke| painter.line_segment([a, b], st);
    let poly = |pts: Vec<Pos2>, fill: Color32, st: Stroke| {
        painter.add(Shape::Path(egui::epaint::PathShape { points: pts, closed: true, fill, stroke: st.into() }));
    };
    let open = |pts: Vec<Pos2>, st: Stroke| {
        painter.add(Shape::line(pts, st));
    };
    let close = |mut pts: Vec<Pos2>| {
        pts.push(pts[0]);
        pts
    };
    let dot = |c: Pos2, r: f32| painter.circle_filled(c, r * s, color);
    let ring = |c: Pos2, r: f32, st: Stroke| painter.circle_stroke(c, r * s, st);
    let rect_of = |x0: f32, y0: f32, x1: f32, y1: f32| Rect::from_min_max(p(x0, y0), p(x1, y1));

    match icon {
        Icon::Select => {
            // Mouse pointer.
            let pts = vec![
                p(0.24, 0.08),
                p(0.24, 0.84),
                p(0.42, 0.67),
                p(0.55, 0.94),
                p(0.67, 0.88),
                p(0.54, 0.62),
                p(0.78, 0.62),
            ];
            poly(pts, faint, line);
        }
        Icon::Wire => {
            open(vec![p(0.1, 0.8), p(0.5, 0.8), p(0.5, 0.2), p(0.9, 0.2)], line);
            dot(p(0.1, 0.8), 0.08);
            dot(p(0.9, 0.2), 0.08);
        }
        Icon::Label => {
            let pts = vec![p(0.08, 0.3), p(0.62, 0.3), p(0.92, 0.5), p(0.62, 0.7), p(0.08, 0.7)];
            poly(pts, faint, line);
            ring(p(0.24, 0.5), 0.06, line);
        }
        Icon::Power => {
            // Supply symbol: bar on a stem, an arrow pointing up.
            seg(p(0.22, 0.25), p(0.78, 0.25), thick);
            seg(p(0.5, 0.25), p(0.5, 0.88), line);
            open(vec![p(0.36, 0.58), p(0.5, 0.44), p(0.64, 0.58)], line);
        }
        Icon::Junction => {
            seg(p(0.08, 0.5), p(0.92, 0.5), line);
            seg(p(0.5, 0.5), p(0.5, 0.92), line);
            seg(p(0.5, 0.08), p(0.5, 0.5), Stroke::new(w, faint));
            dot(p(0.5, 0.5), 0.13);
        }
        Icon::Place => {
            // A resistor with its leads and a "+".
            seg(p(0.04, 0.62), p(0.22, 0.62), line);
            painter.rect_stroke(rect_of(0.22, 0.48, 0.74, 0.76), 1.0, line, egui::StrokeKind::Middle);
            seg(p(0.74, 0.62), p(0.96, 0.62), line);
            seg(p(0.78, 0.08), p(0.78, 0.36), line);
            seg(p(0.64, 0.22), p(0.92, 0.22), line);
        }
        Icon::Route => {
            open(vec![p(0.16, 0.82), p(0.46, 0.82), p(0.84, 0.44), p(0.84, 0.18)], thick);
            dot(p(0.16, 0.82), 0.12);
            dot(p(0.84, 0.18), 0.12);
        }
        Icon::Via => {
            ring(p(0.5, 0.5), 0.27, Stroke::new(w * 2.6, color));
            seg(p(0.04, 0.5), p(0.2, 0.5), Stroke::new(w, faint));
            seg(p(0.8, 0.5), p(0.96, 0.5), Stroke::new(w, faint));
        }
        Icon::Outline => {
            painter.rect(rect_of(0.1, 0.18, 0.9, 0.82), 0.08 * s, faint, line, egui::StrokeKind::Middle);
            for (x, y) in [(0.24, 0.32), (0.76, 0.32), (0.24, 0.68), (0.76, 0.68)] {
                ring(p(x, y), 0.05, Stroke::new(w * 0.8, color));
            }
        }
        Icon::Zone => {
            let pts = vec![p(0.12, 0.16), p(0.88, 0.16), p(0.88, 0.56), p(0.56, 0.88), p(0.12, 0.88)];
            poly(pts, color.gamma_multiply(0.45), line);
            // A pad with its clearance in the fill.
            ring(p(0.42, 0.5), 0.17, Stroke::new(w * 1.6, Color32::from_black_alpha(90)));
            dot(p(0.42, 0.5), 0.1);
        }
        Icon::Pin => {
            seg(p(0.12, 0.2), p(0.12, 0.8), thick);
            seg(p(0.12, 0.5), p(0.72, 0.5), line);
            ring(p(0.8, 0.5), 0.08, line);
        }
        Icon::Pad => {
            painter.rect_filled(rect_of(0.14, 0.26, 0.86, 0.74), 0.1 * s, color);
        }
        Icon::Line => {
            seg(p(0.18, 0.82), p(0.82, 0.18), line);
            for (x, y) in [(0.18, 0.82), (0.82, 0.18)] {
                painter.rect_filled(Rect::from_center_size(p(x, y), vec2(0.16 * s, 0.16 * s)), 1.0, color);
            }
        }
        Icon::Rect => {
            painter.rect_stroke(rect_of(0.14, 0.24, 0.86, 0.76), 1.0, line, egui::StrokeKind::Middle);
        }
        Icon::Circle => {
            ring(p(0.5, 0.5), 0.34, line);
            dot(p(0.5, 0.5), 0.05);
        }
        Icon::Layers => {
            // Three stacked sheets, the top one filled.
            let sheet = |y: f32| vec![p(0.5, y - 0.17), p(0.9, y), p(0.5, y + 0.17), p(0.1, y)];
            open(close(sheet(0.7)), line);
            open(close(sheet(0.5)), line);
            poly(sheet(0.3), color, line);
        }
        Icon::Grid => {
            let thin = Stroke::new(w * 0.8, color);
            for t in [0.15, 0.38, 0.62, 0.85] {
                seg(p(t, 0.1), p(t, 0.9), thin);
                seg(p(0.1, t), p(0.9, t), thin);
            }
        }
        Icon::Fit => {
            // Corner brackets around a square.
            for (x, y, dx, dy) in
                [(0.1, 0.1, 1.0, 1.0), (0.9, 0.1, -1.0, 1.0), (0.1, 0.9, 1.0, -1.0), (0.9, 0.9, -1.0, -1.0)]
            {
                open(vec![p(x, y + dy * 0.24), p(x, y), p(x + dx * 0.24, y)], line);
            }
            painter.rect_filled(rect_of(0.34, 0.34, 0.66, 0.66), 0.04 * s, faint);
            painter.rect_stroke(rect_of(0.34, 0.34, 0.66, 0.66), 0.04 * s, line, egui::StrokeKind::Middle);
        }
        Icon::Text => {
            seg(p(0.18, 0.18), p(0.82, 0.18), thick);
            seg(p(0.5, 0.18), p(0.5, 0.86), thick);
            seg(p(0.36, 0.86), p(0.64, 0.86), line);
        }
    }
}
