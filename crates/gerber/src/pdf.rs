//! A minimal PDF writer for drawings made of render primitives (`render::Instance`): each
//! page is a list of instances in millimetres, drawn as vector paths. Strokes keep their
//! round caps, filled triangles (text, polygon fills) of one colour become one path, with
//! one winding, so no seams show between them. Text is outlines: there are no fonts.
//!
//! Content streams are deflated; bookmarks (the outline) name the pages.

use std::fmt::Write as _;
use std::io::Write as _;

use render::{Color, Instance, primitive::kind};

/// Points per millimetre.
const PT: f64 = 72.0 / 25.4;
/// The cubic Bézier handle of a quarter circle, per unit radius.
const KAPPA: f32 = 0.552_284_8;

/// One page: its size (mm), a bookmark title and the instances, drawn in order. The
/// origin is the page's lower left corner, y up, as in a schematic sheet.
pub struct Page {
    pub size: [f32; 2],
    pub title: String,
    pub instances: Vec<Instance>,
}

/// The PDF of `pages`, titled `title`.
pub fn write(title: &str, pages: &[Page]) -> Vec<u8> {
    // Object numbers: 1 catalog, 2 page tree, 3 outline, 4 info, then per page its page,
    // contents and outline item.
    let n = pages.len();
    let page_obj = |i: usize| 5 + 3 * i;
    let mut objects: Vec<Vec<u8>> = Vec::new();
    objects.push(b"<< /Type /Catalog /Pages 2 0 R /Outlines 3 0 R /PageMode /UseOutlines >>".to_vec());
    let kids: Vec<String> = (0..n).map(|i| format!("{} 0 R", page_obj(i))).collect();
    objects.push(format!("<< /Type /Pages /Kids [{}] /Count {n} >>", kids.join(" ")).into_bytes());
    objects.push(if n == 0 {
        b"<< /Type /Outlines /Count 0 >>".to_vec()
    } else {
        format!("<< /Type /Outlines /First {} 0 R /Last {} 0 R /Count {n} >>", page_obj(0) + 2, page_obj(n - 1) + 2)
            .into_bytes()
    });
    objects.push(format!("<< /Title {} /Producer (pcb-designer) >>", text(title)).into_bytes());
    for (i, page) in pages.iter().enumerate() {
        let (content, alphas) = content(page);
        let gs: String =
            alphas.iter().map(|a| format!("/A{a} << /ca {0} /CA {0} >> ", num(*a as f32 / 255.0))).collect();
        let [w, h] = page.size.map(|v| num((v as f64 * PT) as f32));
        objects.push(
            format!(
                "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {w} {h}] /Contents {} 0 R /Resources << /ExtGState << {gs}>> >> >>",
                page_obj(i) + 1
            )
            .into_bytes(),
        );
        let packed = deflate(content.as_bytes());
        let mut stream = format!("<< /Length {} /Filter /FlateDecode >>\nstream\n", packed.len()).into_bytes();
        stream.extend_from_slice(&packed);
        stream.extend_from_slice(b"\nendstream");
        objects.push(stream);
        let mut item = format!("<< /Title {} /Parent 3 0 R /Dest [{} 0 R /Fit]", text(&page.title), page_obj(i));
        if i > 0 {
            let _ = write!(item, " /Prev {} 0 R", page_obj(i - 1) + 2);
        }
        if i + 1 < n {
            let _ = write!(item, " /Next {} 0 R", page_obj(i + 1) + 2);
        }
        item.push_str(" >>");
        objects.push(item.into_bytes());
    }

    let mut out = b"%PDF-1.4\n%\xe2\xe3\xcf\xd3\n".to_vec();
    let mut offsets = Vec::with_capacity(objects.len());
    for (i, o) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n", i + 1).as_bytes());
        out.extend_from_slice(o);
        out.extend_from_slice(b"\nendobj\n");
    }
    let xref = out.len();
    let mut table = format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1);
    for o in offsets {
        let _ = writeln!(table, "{o:010} 00000 n ");
    }
    let _ =
        write!(table, "trailer\n<< /Size {} /Root 1 0 R /Info 4 0 R >>\nstartxref\n{xref}\n%%EOF\n", objects.len() + 1);
    out.extend_from_slice(table.as_bytes());
    out
}

fn deflate(data: &[u8]) -> Vec<u8> {
    let mut e = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    e.write_all(data).expect("in memory");
    e.finish().expect("in memory")
}

/// A PDF text string: literal for ASCII, else UTF-16BE with a byte order mark.
fn text(s: &str) -> String {
    if s.is_ascii() {
        let escaped: String = s
            .chars()
            .flat_map(|c| match c {
                '(' | ')' | '\\' => vec!['\\', c],
                c if c.is_control() => vec![' '],
                c => vec![c],
            })
            .collect();
        format!("({escaped})")
    } else {
        let hex: String = s.encode_utf16().map(|u| format!("{u:04X}")).collect();
        format!("<FEFF{hex}>")
    }
}

/// A number with at most four decimals, no trailing zeros.
fn num(v: f32) -> String {
    let s = format!("{:.4}", if v.abs() < 5e-5 { 0.0 } else { v });
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" { "0".into() } else { s.into() }
}

/// The page's content stream and the alpha values it uses (an ExtGState each).
fn content(page: &Page) -> (String, Vec<u8>) {
    let mut c = Canvas::default();
    // Millimetres from here on.
    let _ = writeln!(c.out, "{0} 0 0 {0} 0 0 cm 1 J 1 j", num(PT as f32));
    let mut i = 0;
    let insts = &page.instances;
    while i < insts.len() {
        let inst = insts[i];
        if inst.color.0[3] == 0 {
            i += 1;
            continue;
        }
        match inst.kind & kind::MASK {
            kind::TRIANGLE => {
                // A run of triangles of one colour: one path, every triangle wound the
                // same way so shared edges cancel.
                let mut end = i;
                while end < insts.len()
                    && insts[end].kind & kind::MASK == kind::TRIANGLE
                    && insts[end].color == inst.color
                {
                    end += 1;
                }
                c.fill_color(inst.color);
                for t in &insts[i..end] {
                    let (a, b, p) = (t.p0, t.p1, [t.r, t.r2]);
                    let ccw = (b[0] - a[0]) * (p[1] - a[1]) - (b[1] - a[1]) * (p[0] - a[0]) >= 0.0;
                    let (b, p) = if ccw { (b, p) } else { (p, b) };
                    let _ = writeln!(
                        c.out,
                        "{} {} m {} {} l {} {} l h",
                        num(a[0]),
                        num(a[1]),
                        num(b[0]),
                        num(b[1]),
                        num(p[0]),
                        num(p[1])
                    );
                }
                c.out.push_str("f\n");
                i = end;
                continue;
            }
            kind::CAPSULE => {
                c.stroke_color(inst.color, inst.r * 2.0);
                let [a, b] = [inst.p0, inst.p1];
                let _ = writeln!(c.out, "{} {} m {} {} l S", num(a[0]), num(a[1]), num(b[0]), num(b[1]));
            }
            kind::CIRCLE if inst.r2 > 0.0 => {
                // A ring: the stroke of the circle between its radii.
                c.stroke_color(inst.color, inst.r - inst.r2);
                circle(&mut c.out, inst.p0, (inst.r + inst.r2) / 2.0);
                c.out.push_str("S\n");
            }
            kind::CIRCLE => {
                c.fill_color(inst.color);
                circle(&mut c.out, inst.p0, inst.r);
                c.out.push_str("f\n");
            }
            kind::RECT => {
                c.fill_color(inst.color);
                let (s, co) = inst.r2.sin_cos();
                let [x, y] = inst.p0;
                let _ = writeln!(c.out, "q {} {} {} {} {} {} cm", num(co), num(s), num(-s), num(co), num(x), num(y));
                rounded_rect(&mut c.out, inst.p1, inst.r);
                c.out.push_str("f Q\n");
            }
            kind::ARC => {
                c.stroke_color(inst.color, inst.r * 2.0);
                arc(&mut c.out, inst.p0, inst.r2, inst.p1[0], inst.p1[1]);
                c.out.push_str("S\n");
            }
            _ => {}
        }
        i += 1;
    }
    let mut alphas: Vec<u8> = c.alphas.into_iter().collect();
    alphas.sort_unstable();
    (c.out, alphas)
}

/// The content stream being written, with its current graphics state.
#[derive(Default)]
struct Canvas {
    out: String,
    fill: Option<[u8; 3]>,
    stroke: Option<([u8; 3], u32)>,
    alpha: Option<u8>,
    alphas: std::collections::BTreeSet<u8>,
}

impl Canvas {
    fn set_alpha(&mut self, a: u8) {
        if self.alpha != Some(a) {
            self.alpha = Some(a);
            self.alphas.insert(a);
            let _ = writeln!(self.out, "/A{a} gs");
        }
    }

    fn fill_color(&mut self, c: Color) {
        self.set_alpha(c.0[3]);
        let rgb = [c.0[0], c.0[1], c.0[2]];
        if self.fill != Some(rgb) {
            self.fill = Some(rgb);
            let _ = writeln!(self.out, "{} rg", rgb_text(rgb));
        }
    }

    fn stroke_color(&mut self, c: Color, width: f32) {
        self.set_alpha(c.0[3]);
        let rgb = [c.0[0], c.0[1], c.0[2]];
        let w = (width * 1e4).round() as u32;
        if self.stroke.map(|s| s.0) != Some(rgb) {
            let _ = writeln!(self.out, "{} RG", rgb_text(rgb));
        }
        if self.stroke.map(|s| s.1) != Some(w) {
            let _ = writeln!(self.out, "{} w", num(width));
        }
        self.stroke = Some((rgb, w));
    }
}

fn rgb_text(c: [u8; 3]) -> String {
    c.map(|v| num(v as f32 / 255.0)).join(" ")
}

fn circle(out: &mut String, c: [f32; 2], r: f32) {
    arc(out, c, r, 0.0, std::f32::consts::TAU);
    out.push_str("h ");
}

/// An arc as Bézier segments of at most a quarter turn, starting with a move.
fn arc(out: &mut String, c: [f32; 2], r: f32, start: f32, sweep: f32) {
    let parts = (sweep.abs() / std::f32::consts::FRAC_PI_2).ceil().max(1.0) as usize;
    let step = sweep / parts as f32;
    let k = 4.0 / 3.0 * (step / 4.0).tan();
    let at = |a: f32| [c[0] + r * a.cos(), c[1] + r * a.sin()];
    let p = at(start);
    let _ = write!(out, "{} {} m ", num(p[0]), num(p[1]));
    for i in 0..parts {
        let (a0, a1) = (start + step * i as f32, start + step * (i + 1) as f32);
        let (p0, p1) = (at(a0), at(a1));
        let c1 = [p0[0] - k * r * a0.sin(), p0[1] + k * r * a0.cos()];
        let c2 = [p1[0] + k * r * a1.sin(), p1[1] - k * r * a1.cos()];
        let _ =
            write!(out, "{} {} {} {} {} {} c ", num(c1[0]), num(c1[1]), num(c2[0]), num(c2[1]), num(p1[0]), num(p1[1]));
    }
    out.push('\n');
}

/// A rectangle of half size `h` around the origin with corners of radius `r`.
fn rounded_rect(out: &mut String, h: [f32; 2], r: f32) {
    let r = r.clamp(0.0, h[0].min(h[1]));
    let [w, t] = h;
    if r <= 0.0 {
        let _ = writeln!(out, "{} {} {} {} re", num(-w), num(-t), num(2.0 * w), num(2.0 * t));
        return;
    }
    let k = KAPPA * r;
    let _ = write!(out, "{} {} m ", num(-w + r), num(-t));
    // Bottom edge, then each corner and edge counterclockwise.
    let corners = [
        ([w - r, -t], [w - r + k, -t], [w, -t + r - k], [w, -t + r]),
        ([w, t - r], [w, t - r + k], [w - r + k, t], [w - r, t]),
        ([-w + r, t], [-w + r - k, t], [-w, t - r + k], [-w, t - r]),
        ([-w, -t + r], [-w, -t + r - k], [-w + r - k, -t], [-w + r, -t]),
    ];
    for (line, c1, c2, end) in corners {
        let _ = write!(
            out,
            "{} {} l {} {} {} {} {} {} c ",
            num(line[0]),
            num(line[1]),
            num(c1[0]),
            num(c1[1]),
            num(c2[0]),
            num(c2[1]),
            num(end[0]),
            num(end[1])
        );
    }
    out.push_str("h\n");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_and_strings() {
        assert_eq!(num(1.0), "1");
        assert_eq!(num(0.12345), "0.1235");
        assert_eq!(num(-0.00001), "0");
        assert_eq!(text("A (b)"), "(A \\(b\\))");
        assert_eq!(text("Ü"), "<FEFF00DC>");
    }

    #[test]
    fn a_page_is_a_well_formed_file() {
        let red = Color::rgb(255, 0, 0);
        let instances = vec![
            Instance::line([10.0, 10.0], [50.0, 10.0], 0.2, red),
            Instance::circle([20.0, 20.0], 1.0, red.with_alpha(128)),
            Instance::circle_outline([30.0, 20.0], 2.0, 0.2, red),
            Instance::rect([40.0, 40.0], [10.0, 5.0], 1.0, 0.3, red),
            Instance::arc([60.0, 40.0], 5.0, 0.0, 3.0, 0.2, red),
            Instance::triangle([0.0, 0.0], [5.0, 0.0], [0.0, 5.0], [true; 3], red),
            Instance::triangle([5.0, 0.0], [0.0, 5.0], [5.0, 5.0], [true; 3], red),
        ];
        let pages = [
            Page { size: [297.0, 210.0], title: "Root".into(), instances },
            Page { size: [210.0, 297.0], title: "/Sub".into(), instances: Vec::new() },
        ];
        let pdf = write("Demo", &pages);
        let s = String::from_utf8_lossy(&pdf);
        assert!(s.starts_with("%PDF-1.4"));
        assert!(s.ends_with("%%EOF\n"));
        assert!(s.contains("/Count 2"));
        assert!(s.contains("/MediaBox [0 0 841.8898 595.2756]"), "A4 landscape in points");
        assert!(s.contains("/A128 << /ca 0.502"), "the half transparent disc's state");
        // The xref offsets (bytes) point at the objects.
        let tail = String::from_utf8(pdf[pdf.len() - 30..].to_vec()).unwrap();
        let at: usize = tail.split("startxref\n").nth(1).unwrap().lines().next().unwrap().parse().unwrap();
        assert!(pdf[at..].starts_with(b"xref"));
        let table = String::from_utf8(pdf[at..].to_vec()).unwrap();
        for (i, line) in table.lines().skip(3).take(5).enumerate() {
            let offset: usize = line[..10].parse().unwrap();
            assert!(pdf[offset..].starts_with(format!("{} 0 obj", i + 1).as_bytes()), "object {}", i + 1);
        }
        // Both triangles in one path, wound alike.
        let content = inflate(&pdf);
        assert_eq!(content.matches(" l h\n").count(), 2);
        assert_eq!(content.lines().filter(|l| *l == "f").count(), 1, "{content}");
        assert!(content.contains("0 0 m 5 0 l 0 5 l h"), "{content}");
        assert!(content.contains("5 0 m 5 5 l 0 5 l h"), "turned counterclockwise: {content}");
    }

    /// The first content stream, inflated.
    fn inflate(pdf: &[u8]) -> String {
        use std::io::Read;
        let start = pdf.windows(7).position(|w| w == b"stream\n").unwrap() + 7;
        let end = start + pdf[start..].windows(10).position(|w| w == b"\nendstream").unwrap();
        let mut s = String::new();
        flate2::read::ZlibDecoder::new(&pdf[start..end]).read_to_string(&mut s).unwrap();
        s
    }
}
