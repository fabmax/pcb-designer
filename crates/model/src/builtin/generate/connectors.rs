//! Generic connectors `Conn_01xNN` (1 to 40 pins) and `Conn_02xNN` (2 to 40 pins, odd
//! pins in the left column): pin headers and sockets at 2.54 mm and, on single rows, JST
//! PH / XH / SH and screw terminals where that pin count exists.

use super::fp::{Fp, row};
use super::{Builder, Map, PartSpec, Variant, sym, variant};
use crate::geom::{Point, Shape};
use crate::units::mm;

const P: f64 = 2.54;

#[derive(Clone, Copy, PartialEq)]
enum Style {
    Header,
    RightAngle,
    Socket,
    Smd,
}

/// Pad (name, x, y) of a `rows` × `cols` pin grid, `pitch` apart, centred: pin 1 top left,
/// numbered across the columns.
fn grid(rows: usize, cols: usize, x_pitch: f64) -> Vec<(String, f64, f64)> {
    let xs: Vec<f64> = row(cols, x_pitch).collect();
    let ys: Vec<f64> = row(rows, P).collect();
    let mut out = Vec::new();
    for (r, y) in ys.iter().enumerate() {
        for (c, &x) in xs.iter().enumerate() {
            out.push(((r * cols + c + 1).to_string(), x, -y));
        }
    }
    out
}

fn pin_row(b: &mut Builder, rows: usize, cols: usize, style: Style) -> String {
    let (kind, suffix) = match style {
        Style::Header => ("PinHeader", "Vertical"),
        Style::RightAngle => ("PinHeader", "Horizontal"),
        Style::Socket => ("PinSocket", "Vertical"),
        Style::Smd => ("PinHeader", if cols == 1 { "Vertical_SMD_Pin1Left" } else { "Vertical_SMD" }),
    };
    let name = format!("{kind}_{cols}x{rows:02}_P2.54mm_{suffix}");
    let what = match style {
        Style::Header => "pin header",
        Style::RightAngle => "right-angle pin header",
        Style::Socket => "pin socket",
        Style::Smd => "SMD pin header",
    };
    let n = rows * cols;
    let mut f = Fp::new(&name, format!("{cols}×{rows} {what}, 2.54 mm pitch, {n} pins"));
    let half_w = cols as f64 * P / 2.0;
    let half_h = rows as f64 * P / 2.0;
    let pads = grid(rows, cols, P);
    if style == Style::Smd {
        // Single row: pins alternate left and right; two rows: each column's pads outward.
        let (x, w) = if cols == 1 { (1.655, 2.51) } else { (2.525, 3.15) };
        for (i, (n, _, y)) in pads.iter().enumerate() {
            f.smd(n, if i % 2 == 0 { -x } else { x }, *y, w, 1.0);
        }
    } else {
        for (n, x, y) in &pads {
            f.tht(n, *x, *y, 1.7, 1.7, 1.0);
        }
    }
    let y1 = pads[0].2;
    if style == Style::RightAngle {
        // The plastic beside the pads, the pins out of it to the right.
        let x0 = half_w - P / 2.0 + 1.5;
        let x1 = x0 + P;
        f.outline([x0, -half_h, x1, half_h], 0.06);
        for (_, _, y) in pads.iter().filter(|p| p.1 == pads[cols - 1].1) {
            f.fab(Shape::line(Point::mm(x1, *y), Point::mm(x1 + 6.0, *y), mm(0.64)));
            f.keep([x1, y - 0.32, x1 + 6.0, y + 0.32]);
        }
        f.keep([-half_w, -half_h, x0, half_h]);
    } else {
        f.outline([-half_w, -half_h, half_w, half_h], 0.06);
    }
    // Pin 1 mark left of the outline.
    let x = -half_w - 0.4 - if style == Style::Smd { 1.4 + 0.25 * cols as f64 } else { 0.0 };
    f.silk(Shape::line(Point::mm(x, y1 - 0.6), Point::mm(x, y1 + 0.6), mm(0.12)));
    f.keep([x - 0.1, y1 - 0.6, x, y1 + 0.6]);
    b.footprint(f.finish());
    name
}

/// JST PH / XH (through-hole, vertical) and SH (SMD, side entry) of `n` pins.
fn jst(b: &mut Builder, series: &str, n: usize) -> String {
    let mut f;
    let name;
    match series {
        "PH" | "XH" => {
            let (pitch, pad, drill, ext, back, front, part) = if series == "PH" {
                (2.0, (1.2, 1.75), 0.75, 1.95, -2.8, 1.7, format!("B{n}B-PH-K"))
            } else {
                (2.5, (1.7, 2.0), 1.0, 2.45, -3.4, 2.35, format!("B{n}B-XH-A"))
            };
            name = format!("JST_{series}_{part}_1x{n:02}_P{pitch:.2}mm_Vertical");
            f = Fp::new(&name, format!("JST {series} connector {part}, {n} pins, {pitch} mm pitch, vertical"));
            for (i, x) in row(n, pitch).enumerate() {
                f.tht(&(i + 1).to_string(), x, 0.0, pad.0, pad.1, drill);
            }
            let half = (n - 1) as f64 * pitch / 2.0 + ext;
            f.outline([-half, back, half, front], 0.11);
            f.silk_dot(-half - 0.45, 0.0);
            f.keep([-half - 0.65, -0.2, -half, 0.2]);
        }
        _ => {
            let part = format!("SM{n:02}B-SRSS-TB");
            name = format!("JST_SH_{part}_1x{n:02}-1MP_P1.00mm_Horizontal");
            f = Fp::new(&name, format!("JST SH connector {part}, {n} pins, 1.00 mm pitch, side entry"));
            for (i, x) in row(n, 1.0).enumerate() {
                f.smd(&(i + 1).to_string(), x, 2.0, 0.6, 1.55);
            }
            let mp = (n - 1) as f64 / 2.0 + 1.3;
            f.smd("MP", -mp, -1.875, 1.2, 1.8);
            f.smd("MP", mp, -1.875, 1.2, 1.8);
            let half = (n - 1) as f64 / 2.0 + 1.5;
            f.outline([-half, -2.4, half, 1.2], 0.11);
            let x1 = -(n as f64 - 1.0) / 2.0;
            f.silk_dot(x1 - 0.65, 2.6);
        }
    }
    b.footprint(f.finish());
    name
}

/// Screw terminal block of `n` positions, `pitch` 5.00 or 3.50 mm.
fn terminal(b: &mut Builder, n: usize, pitch: f64) -> String {
    let name = format!("TerminalBlock_1x{n:02}_P{pitch:.2}mm");
    let mut f = Fp::new(
        &name,
        format!("Screw terminal block, {n} positions, {pitch} mm pitch; generic, check the terminal's datasheet"),
    );
    let (pad, drill, back, front) = if pitch == 5.0 { (2.6, 1.3, -4.0, 4.5) } else { (2.2, 1.2, -3.4, 3.9) };
    for (i, x) in row(n, pitch).enumerate() {
        f.tht(&(i + 1).to_string(), x, 0.0, pad, pad, drill);
    }
    let half = n as f64 * pitch / 2.0;
    f.outline([-half, back, half, front], 0.11);
    b.footprint(f.finish());
    name
}

pub(super) fn add(b: &mut Builder) {
    for n in 1..=40 {
        let name = format!("Conn_01x{n:02}");
        b.symbol(sym::connector(&name, n, 1));
        let mut variants: Vec<Variant> = vec![
            variant("Pin header", pin_row(b, n, 1, Style::Header), Map::ByNumber),
            variant("Pin header, right angle", pin_row(b, n, 1, Style::RightAngle), Map::ByNumber),
            variant("Pin socket", pin_row(b, n, 1, Style::Socket), Map::ByNumber),
        ];
        if n >= 2 {
            variants.push(variant("Pin header SMD", pin_row(b, n, 1, Style::Smd), Map::ByNumber));
        }
        if (2..=8).contains(&n) {
            variants.push(variant("JST PH", jst(b, "PH", n), Map::ByNumber));
            variants.push(variant("JST XH", jst(b, "XH", n), Map::ByNumber));
        }
        if (2..=6).contains(&n) {
            variants.push(variant("JST SH", jst(b, "SH", n), Map::ByNumber));
        }
        if (2..=4).contains(&n) {
            variants.push(variant("Screw terminal 5.00 mm", terminal(b, n, 5.0), Map::ByNumber));
            variants.push(variant("Screw terminal 3.50 mm", terminal(b, n, 3.5), Map::ByNumber));
        }
        let pins = if n == 1 { "1 pin".to_string() } else { format!("{n} pins") };
        b.part(PartSpec {
            name: name.clone(),
            description: format!("Generic connector, 1 row, {pins}: pin header or socket, JST, screw terminal"),
            prefix: "J",
            value: name.clone(),
            symbol: Some(&name),
            variants,
        });
    }
    for rows in 1..=20 {
        let name = format!("Conn_02x{rows:02}");
        b.symbol(sym::connector(&name, rows, 2));
        let variants = vec![
            variant("Pin header", pin_row(b, rows, 2, Style::Header), Map::ByNumber),
            variant("Pin header, right angle", pin_row(b, rows, 2, Style::RightAngle), Map::ByNumber),
            variant("Pin socket", pin_row(b, rows, 2, Style::Socket), Map::ByNumber),
            variant("Pin header SMD", pin_row(b, rows, 2, Style::Smd), Map::ByNumber),
        ];
        b.part(PartSpec {
            name: name.clone(),
            description: format!(
                "Generic connector, 2 rows, {} pins (odd left, even right): pin header or socket",
                2 * rows
            ),
            prefix: "J",
            value: name.clone(),
            symbol: Some(&name),
            variants,
        });
    }
}
