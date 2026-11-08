//! KiCad footprints (`.kicad_mod`) → [`Footprint`].
//!
//! KiCad footprint coordinates are y-down; they are flipped to our y-up system. Angles
//! keep their value: KiCad angles are counterclockwise as seen on screen, and so are ours
//! after the flip. Only front-side content is imported (footprints are defined on the top
//! side; bottom-side items are reported as warnings).

use model::*;

use crate::common::*;
use crate::sexpr::{self, Node};

/// A converted KiCad footprint.
#[derive(Clone, Debug)]
pub struct KiFootprint {
    pub lib: String,
    pub footprint: Footprint,
    pub tags: String,
    pub warnings: Vec<String>,
}

impl KiFootprint {
    pub fn full_name(&self) -> String {
        format!("{}:{}", self.lib, self.footprint.name)
    }
}

/// Parse and convert a `.kicad_mod` file of library `lib`.
pub fn parse(lib: &str, text: &str) -> Result<KiFootprint, sexpr::ParseError> {
    let root = sexpr::parse(text)?;
    if !matches!(root.head(), "footprint" | "module") {
        return Err(sexpr::ParseError { line: 1, message: "not a KiCad footprint".into() });
    }
    Ok(convert(lib, &root))
}

fn layer(name: &str) -> Option<FootprintLayer> {
    Some(match name {
        "F.Cu" => FootprintLayer::Copper,
        "F.SilkS" | "F.Silkscreen" => FootprintLayer::Silkscreen,
        "F.Fab" => FootprintLayer::Fabrication,
        "F.CrtYd" | "F.Courtyard" => FootprintLayer::Courtyard,
        "F.Mask" => FootprintLayer::SolderMask,
        "F.Paste" => FootprintLayer::SolderPaste,
        _ => return None,
    })
}

/// Layers that are silently ignored (documentation and board outline layers).
fn ignored_layer(name: &str) -> bool {
    name.ends_with(".User") || name.starts_with("User.") || name == "Edge.Cuts" || name == "Margin"
}

fn is_filled(n: &Node) -> bool {
    match n.child("fill") {
        Some(f) => match f.arg(0) {
            Some(a) => matches!(a, "yes" | "solid"),
            None => f.child("type").and_then(|t| t.arg(0)).is_some_and(|t| t != "none"),
        },
        None => false,
    }
}

fn width_of(n: &Node) -> Coord {
    nm(stroke_width(n).unwrap_or(0.0).max(0.0))
}

struct Ctx<'a> {
    lib: &'a str,
    name: &'a str,
    warnings: Vec<String>,
    /// SMD pads with copper only on the back side.
    bottom_pads: Vec<Pad>,
}

impl Ctx<'_> {
    fn warn(&mut self, w: impl Into<String>) {
        self.warnings.push(w.into());
    }

    /// Footprint layer of an item, or `None` (with a warning for unsupported layers).
    fn layer_of(&mut self, n: &Node, what: &str) -> Option<FootprintLayer> {
        let name = n.child("layer").and_then(|l| l.arg(0)).unwrap_or("");
        let l = layer(name);
        if l.is_none() && !ignored_layer(name) {
            if name.starts_with("B.") {
                self.warn("back-side graphics and texts skipped");
            } else if name.starts_with("In") && name.ends_with(".Cu") {
                self.warn("inner-layer graphics and texts skipped");
            } else {
                self.warn(format!("{what} on layer {name} skipped"));
            }
        }
        l
    }
}

pub fn convert(lib: &str, root: &Node) -> KiFootprint {
    let name = root.arg(0).unwrap_or("").to_string();
    // Names may carry a library prefix in old files ("Lib:Name").
    let name = name.rsplit(':').next().unwrap_or("").to_string();
    let mut cx = Ctx { lib, name: &name, warnings: Vec::new(), bottom_pads: Vec::new() };
    let mut fp = Footprint {
        id: kid(&["kicad-footprint", lib, &name]),
        name: name.clone(),
        description: root.child("descr").and_then(|d| d.arg(0)).unwrap_or("").to_string(),
        pads: Store::default(),
        graphics: Vec::new(),
        texts: Vec::new(),
    };
    let mut seen_pads: std::collections::HashMap<String, usize> = Default::default();

    for item in root.lists() {
        match item.head() {
            "fp_line" | "fp_rect" | "fp_circle" | "fp_arc" | "fp_poly" | "fp_curve" => {
                if let Some(layer) = cx.layer_of(item, "graphic") {
                    for shape in graphic(item, Point::ZERO, Angle::ZERO) {
                        fp.graphics.push(FootprintGraphic { layer, shape });
                    }
                }
            }
            "property" => {
                let key = item.arg(0).unwrap_or("");
                let content = match key {
                    "Reference" => "${REF}".to_string(),
                    "Value" => "${VALUE}".to_string(),
                    _ => continue, // other properties are hidden metadata
                };
                if let Some(t) = fp_text(&mut cx, item, content) {
                    fp.texts.push(t);
                }
            }
            "fp_text" => {
                let content = match item.arg(0) {
                    Some("reference") => "${REF}".to_string(),
                    Some("value") => "${VALUE}".to_string(),
                    _ => substitute(&plain(item.arg(1).unwrap_or(""))),
                };
                if let Some(t) = fp_text(&mut cx, item, content) {
                    fp.texts.push(t);
                }
            }
            "fp_text_box" => cx.warn("text box skipped"),
            "pad" => pad(&mut cx, item, &mut fp, &mut seen_pads),
            "zone" => cx.warn("zone/keepout skipped"),
            _ => {}
        }
    }
    // Back-side copies of front pads (exposed pads with thermal vias) are redundant;
    // pads that exist only on the back are moved to the front so that no pin is lost.
    for pad in std::mem::take(&mut cx.bottom_pads) {
        if !fp.pads.iter().any(|p| p.name == pad.name) {
            cx.warn("bottom-side pad imported on the top side");
            fp.pads.insert(pad);
        }
    }
    let mut warnings = std::mem::take(&mut cx.warnings);
    warnings.sort();
    warnings.dedup();
    KiFootprint {
        lib: lib.to_string(),
        footprint: fp,
        tags: root.child("tags").and_then(|d| d.arg(0)).unwrap_or("").to_string(),
        warnings,
    }
}

/// KiCad text variables → ours.
fn substitute(s: &str) -> String {
    s.replace("${REFERENCE}", "${REF}").replace("%R", "${REF}").replace("%V", "${VALUE}")
}

fn fp_text(cx: &mut Ctx, n: &Node, content: String) -> Option<FootprintText> {
    if n.flag("hide") {
        return None;
    }
    let layer = cx.layer_of(n, "text")?;
    let t = text(content, n.child("at")?, n.child("effects"), true, 1.0)?;
    Some(FootprintText { layer, text: t })
}

/// A graphic item (also pad primitives: `gr_*` relative to the pad at `origin`/`rot`).
fn graphic(n: &Node, origin: Point, rot: Angle) -> Vec<Shape> {
    let tp = |p: Point| origin + p.rotated(rot);
    let at = |head: &str| child_point(n, head, true).map(tp);
    let width = width_of(n);
    let fill = is_filled(n);
    let kind = n.head().trim_start_matches("fp_").trim_start_matches("gr_");
    match kind {
        "line" => match (at("start"), at("end")) {
            (Some(a), Some(b)) => vec![Shape::line(a, b, width)],
            _ => Vec::new(),
        },
        "rect" => match (at("start"), at("end")) {
            (Some(a), Some(b)) if rot.quarter_turns().is_some() => vec![Shape::Rect { a, b, width, fill }],
            (Some(_), Some(_)) => {
                // Rotated rectangle (pad primitive): as a polygon.
                let (a, b) = (child_point(n, "start", true).unwrap(), child_point(n, "end", true).unwrap());
                let pts = [a, Point::new(b.x, a.y), b, Point::new(a.x, b.y)].map(tp).to_vec();
                crate::symbol::polyline(close(pts), width, fill)
            }
            _ => Vec::new(),
        },
        "circle" => match (at("center"), at("end")) {
            (Some(c), Some(e)) => {
                vec![Shape::Circle { center: c, radius: (e - c).length().round() as Coord, width, fill }]
            }
            _ => Vec::new(),
        },
        "arc" => {
            let (Some(s), Some(e)) = (at("start"), at("end")) else { return Vec::new() };
            let shape = if let Some(m) = at("mid") {
                arc3(s, m, e, width)
            } else if let Some(angle) = n.child("angle").and_then(|a| a.num(0)) {
                // KiCad 5: start = center, end = arc start, angle clockwise on screen.
                let (c, p0) = (s, e);
                let r = p0 - c;
                let mid = c + r.rotated(Angle::deg(-angle / 2.0));
                let end = c + r.rotated(Angle::deg(-angle));
                arc3(p0, mid, end, width)
            } else {
                None
            };
            shape.map_or_else(|| vec![Shape::line(s, e, width)], |a| vec![a])
        }
        "poly" => {
            let mut points = Vec::new();
            if let Some(list) = n.child("pts") {
                for p in list.lists() {
                    match p.head() {
                        "xy" => points.extend(p.num(0).zip(p.num(1)).map(|(x, y)| tp(point(x, y, true)))),
                        "arc" => {
                            for h in ["start", "mid", "end"] {
                                points.extend(child_point(p, h, true).map(tp));
                            }
                        }
                        _ => {}
                    }
                }
            }
            // Footprint polygons are always closed.
            crate::symbol::polyline(close(points), width, fill)
        }
        "curve" | "bezier" => {
            let pts: Vec<Point> = pts(n, true).into_iter().map(tp).collect();
            crate::symbol::polyline(bezier(&pts), width, false)
        }
        _ => Vec::new(),
    }
}

/// Make a point list closed (first == last) so `polyline` treats it as a polygon.
fn close(mut p: Vec<Point>) -> Vec<Point> {
    if p.len() >= 3 && p.first() != p.last() {
        p.push(p[0]);
    }
    p
}

fn numbers(n: &Node) -> Vec<f64> {
    n.items().iter().skip(1).filter_map(|i| i.as_atom()?.parse().ok()).collect()
}

/// Rectangle outline (y-up, CCW from bottom left) with optional chamfered corners.
fn rect_outline(w: Coord, h: Coord, chamfer: Coord, corners: [bool; 4]) -> Vec<Point> {
    let (hx, hy) = (w / 2, h / 2);
    let pts = [Point::new(-hx, -hy), Point::new(hx, -hy), Point::new(hx, hy), Point::new(-hx, hy)];
    let mut out = Vec::new();
    for i in 0..4 {
        let p = pts[i];
        if corners[i] && chamfer > 0 {
            let prev = pts[(i + 3) % 4];
            let next = pts[(i + 1) % 4];
            let toward = |q: Point| {
                let d = q - p;
                let l = d.length().max(1.0);
                p + Point::new((d.x as f64 / l * chamfer as f64) as Coord, (d.y as f64 / l * chamfer as f64) as Coord)
            };
            out.push(toward(prev));
            out.push(toward(next));
        } else {
            out.push(p);
        }
    }
    out
}

fn pad(cx: &mut Ctx, n: &Node, fp: &mut Footprint, seen: &mut std::collections::HashMap<String, usize>) {
    let name = n.arg(0).unwrap_or("").to_string();
    let typ = n.arg(1).unwrap_or("");
    let shape_name = n.arg(2).unwrap_or("");
    let Some(at) = n.child("at") else { return };
    let (Some(x), Some(y)) = (at.num(0), at.num(1)) else { return };
    let pos = point(x, y, true);
    let rotation = Angle::deg(at.num(2).unwrap_or(0.0));
    let (w, h) = n.child("size").map_or((0.0, 0.0), |s| (s.num(0).unwrap_or(0.0), s.num(1).unwrap_or(0.0)));
    let (w, h) = (nm(w), nm(h));
    let layers: Vec<&str> =
        n.child("layers").map(|l| l.items().iter().skip(1).filter_map(Node::as_atom).collect()).unwrap_or_default();
    let front_copper = layers.iter().any(|l| matches!(*l, "F.Cu" | "*.Cu" | "F&B.Cu"));
    let any_copper = front_copper || layers.iter().any(|l| l.ends_with(".Cu"));

    // Drill.
    let mut drill = 0;
    if let Some(d) = n.child("drill") {
        let v = numbers(d);
        let oval = d.items().iter().skip(1).any(|i| i.as_atom() == Some("oval"));
        if oval && v.len() >= 2 && v[0] != v[1] {
            cx.warn("oval drill imported as round hole");
            drill = nm(v[0].min(v[1]));
        } else if let Some(&dv) = v.first() {
            drill = nm(dv);
        }
        if d.child("offset").and_then(|o| o.num(0).zip(o.num(1))).is_some_and(|(a, b)| a != 0.0 || b != 0.0) {
            cx.warn("drill offset ignored");
        }
    }

    // Shape.
    let chamfer = || {
        let ratio = n.child("chamfer_ratio").and_then(|c| c.num(0)).unwrap_or(0.0);
        let corners = n.child("chamfer").map_or([false; 4], |c| {
            let has = |s: &str| c.items().iter().skip(1).any(|i| i.as_atom() == Some(s));
            [has("bottom_left"), has("bottom_right"), has("top_right"), has("top_left")]
        });
        ((ratio * w.min(h) as f64) as Coord, corners)
    };
    let mut extra: Vec<Shape> = Vec::new();
    let shape = match shape_name {
        "circle" => PadShape::Circle,
        "rect" => PadShape::Rect,
        "oval" => PadShape::Oval,
        "roundrect" => {
            let (c, corners) = chamfer();
            if c > 0 && corners.iter().any(|&b| b) {
                PadShape::Polygon { points: rect_outline(w, h, c, corners) }
            } else {
                PadShape::RoundRect { ratio: n.child("roundrect_rratio").and_then(|r| r.num(0)).unwrap_or(0.25) }
            }
        }
        "trapezoid" => {
            let (dx, dy) =
                n.child("rect_delta").map_or((0.0, 0.0), |d| (d.num(0).unwrap_or(0.0), d.num(1).unwrap_or(0.0)));
            let (hx, hy, dx, dy) = (w as f64 / 2.0, h as f64 / 2.0, nm(dx) as f64 / 2.0, nm(dy) as f64 / 2.0);
            // KiCad's corner formula, in y-down coordinates, then flipped.
            let pts = [(-hx - dy, hy + dx), (hx + dy, hy - dx), (hx - dy, -hy + dx), (-hx + dy, -hy - dx)];
            PadShape::Polygon { points: pts.iter().map(|&(x, y)| Point::new(x as Coord, -y as Coord)).collect() }
        }
        "custom" => {
            let anchor_circle =
                n.child("options").and_then(|o| o.child("anchor")).and_then(|a| a.arg(0)) == Some("circle");
            let anchor = if anchor_circle { PadShape::Circle } else { PadShape::Rect };
            let prims: Vec<&Node> = n.child("primitives").map(|p| p.lists().collect()).unwrap_or_default();
            let mut shapes: Vec<Shape> = prims.iter().flat_map(|p| graphic(p, Point::ZERO, Angle::ZERO)).collect();
            // A single filled polygon that contains the anchor becomes the pad outline.
            let single = match shapes.as_slice() {
                [Shape::Polygon { points, .. }] => {
                    let b = Rect::union_all(points.iter().map(|&p| Rect::around(p, 0)));
                    b.is_some_and(|b| b.contains(Point::new(-w / 2, -h / 2)) && b.contains(Point::new(w / 2, h / 2)))
                }
                _ => false,
            };
            if single && let Some(Shape::Polygon { points, .. }) = shapes.pop() {
                PadShape::Polygon { points }
            } else {
                // Anchor as the pad, the primitives as copper graphics around it.
                extra = shapes.into_iter().map(|s| s.transformed(&Transform::new(pos, rotation, false))).collect();
                anchor
            }
        }
        other => {
            cx.warn(format!("pad shape {other} imported as rectangle"));
            PadShape::Rect
        }
    };
    // Polygon pads: size = symmetric extent of the outline (used for bounds).
    let size = match &shape {
        PadShape::Polygon { points } => {
            let ex = points.iter().map(|p| p.x.abs()).max().unwrap_or(0);
            let ey = points.iter().map(|p| p.y.abs()).max().unwrap_or(0);
            Size::new(2 * ex, 2 * ey)
        }
        _ => Size::new(w, h),
    };

    let kind = match typ {
        "np_thru_hole" => PadKind::NonPlatedHole { drill: drill.max(1) },
        "thru_hole" => PadKind::ThroughHole { drill: drill.max(1) },
        _ => {
            if !any_copper {
                // Aperture-only pad (paste or mask opening): a filled graphic.
                let layer = if layers.contains(&"F.Paste") {
                    Some(FootprintLayer::SolderPaste)
                } else if layers.contains(&"F.Mask") {
                    Some(FootprintLayer::SolderMask)
                } else {
                    None
                };
                if let Some(layer) = layer {
                    let t = Transform::new(pos, rotation, false);
                    let outline = match &shape {
                        PadShape::Circle => Shape::Circle { center: pos, radius: w / 2, width: 0, fill: true },
                        PadShape::Polygon { points } => Shape::Polygon {
                            points: points.iter().map(|&p| t.point(p)).collect(),
                            width: 0,
                            fill: true,
                        },
                        _ => Shape::Polygon {
                            points: rect_outline(w, h, 0, [false; 4]).into_iter().map(|p| t.point(p)).collect(),
                            width: 0,
                            fill: true,
                        },
                    };
                    fp.graphics.push(FootprintGraphic { layer, shape: outline });
                }
                return;
            }
            PadKind::Smd
        }
    };
    for shape in extra {
        fp.graphics.push(FootprintGraphic { layer: FootprintLayer::Copper, shape });
    }
    let occurrence = seen.entry(name.clone()).or_default();
    *occurrence += 1;
    let id = kid(&["kicad-pad", cx.lib, cx.name, &name, &occurrence.to_string()]);
    let pad = Pad { id, name, pos, rotation, size, shape, kind };
    if !front_copper && pad.kind == PadKind::Smd {
        cx.bottom_pads.push(pad);
    } else {
        fp.pads.insert(pad);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const R0603: &str = r#"(footprint "R_0603_1608Metric" (version 20240108) (generator "pcbnew")
  (layer "F.Cu")
  (descr "Resistor SMD 0603")
  (tags "resistor")
  (property "Reference" "REF**" (at 0 -1.43 0) (layer "F.SilkS") (effects (font (size 1 1) (thickness 0.15))))
  (property "Value" "R_0603_1608Metric" (at 0 1.43 0) (layer "F.Fab") (effects (font (size 1 1) (thickness 0.15))))
  (property "Datasheet" "" (at 0 0 0) (layer "F.Fab") (hide yes) (effects (font (size 1.27 1.27))))
  (attr smd)
  (fp_line (start -0.237258 -0.5225) (end 0.237258 -0.5225) (stroke (width 0.12) (type solid)) (layer "F.SilkS"))
  (fp_rect (start -1.48 -0.73) (end 1.48 0.73) (stroke (width 0.05) (type solid)) (fill none) (layer "F.CrtYd"))
  (fp_line (start -1 1) (end 1 1) (stroke (width 0.1) (type solid)) (layer "B.SilkS"))
  (fp_text user "${REFERENCE}" (at 0 0 0) (layer "F.Fab") (effects (font (size 0.4 0.4) (thickness 0.06))))
  (pad "1" smd roundrect (at -0.825 0) (size 0.8 0.95) (layers "F.Cu" "F.Paste" "F.Mask") (roundrect_rratio 0.25))
  (pad "2" smd roundrect (at 0.825 0 90) (size 0.8 0.95) (layers "F.Cu" "F.Paste" "F.Mask") (roundrect_rratio 0.25))
  (pad "" smd rect (at 0 0.5) (size 0.2 0.2) (layers "F.Paste"))
  (pad "" np_thru_hole circle (at 0 -2) (size 1 1) (drill 1) (layers "*.Cu" "*.Mask"))
  (pad "3" thru_hole oval (at 2 0) (size 1.5 2) (drill oval 0.8 1.2) (layers "*.Cu" "*.Mask"))
)"#;

    #[test]
    fn converts_basic_footprint() {
        let k = parse("Resistor_SMD", R0603).unwrap();
        let fp = &k.footprint;
        assert_eq!(fp.name, "R_0603_1608Metric");
        assert_eq!(fp.description, "Resistor SMD 0603");
        assert_eq!(k.tags, "resistor");
        assert_eq!(fp.pads.len(), 4);
        let p1 = fp.pads.iter().find(|p| p.name == "1").unwrap();
        assert_eq!(p1.pos, Point::new(-825_000, 0));
        assert_eq!(p1.size, Size::new(800_000, 950_000));
        assert_eq!(p1.shape, PadShape::RoundRect { ratio: 0.25 });
        let p2 = fp.pads.iter().find(|p| p.name == "2").unwrap();
        assert_eq!(p2.rotation.degrees(), 90.0);
        let p3 = fp.pads.iter().find(|p| p.name == "3").unwrap();
        assert_eq!(p3.kind, PadKind::ThroughHole { drill: 800_000 });
        assert!(fp.pads.iter().any(|p| p.kind == PadKind::NonPlatedHole { drill: 1_000_000 }));
        // Reference flipped to y-up: -1.43 → +1.43.
        let r = fp.texts.iter().find(|t| t.text.text == "${REF}" && t.layer == FootprintLayer::Silkscreen).unwrap();
        assert_eq!(r.text.pos, Point::new(0, 1_430_000));
        assert!(fp.texts.iter().any(|t| t.text.text == "${REF}" && t.layer == FootprintLayer::Fabrication));
        assert!(fp.texts.iter().any(|t| t.text.text == "${VALUE}"));
        assert_eq!(fp.texts.len(), 3);
        // Silk line, courtyard rect, paste aperture; the B.SilkS line is skipped.
        assert_eq!(fp.graphics.len(), 3);
        assert!(fp.graphics.iter().any(|g| g.layer == FootprintLayer::SolderPaste));
        assert!(k.warnings.iter().any(|w| w.contains("back-side")));
        assert!(k.warnings.iter().any(|w| w.contains("oval drill")));
        // Stable ids.
        assert_eq!(fp.id, parse("Resistor_SMD", R0603).unwrap().footprint.id);
    }

    #[test]
    fn custom_pad_becomes_polygon() {
        let src = r#"(footprint "X" (layer "F.Cu")
  (pad "1" smd custom (at 1 1 90) (size 0.5 0.5) (layers "F.Cu" "F.Mask")
    (options (clearance outline) (anchor rect))
    (primitives (gr_poly (pts (xy -1 -0.5) (xy 1 -0.5) (xy 1 2) (xy -1 2)) (width 0) (fill yes))))
  (pad "2" smd custom (at 0 0) (size 0.5 0.5) (layers "F.Cu" "F.Mask")
    (options (clearance outline) (anchor circle))
    (primitives (gr_line (start 0 0) (end 2 0) (width 0.3))))
  (fp_poly (pts (xy 0 0) (xy 1 0) (xy 0 1)) (stroke (width 0)) (fill solid) (layer "F.SilkS"))
)"#;
        let fp = parse("L", src).unwrap().footprint;
        let p1 = fp.pads.iter().find(|p| p.name == "1").unwrap();
        let PadShape::Polygon { points } = &p1.shape else { panic!("{:?}", p1.shape) };
        // y flipped: (1, 2) → (1, -2).
        assert!(points.contains(&Point::new(1_000_000, -2_000_000)));
        assert_eq!(p1.size, Size::new(2_000_000, 4_000_000));
        let p2 = fp.pads.iter().find(|p| p.name == "2").unwrap();
        assert_eq!(p2.shape, PadShape::Circle);
        assert!(fp.graphics.iter().any(|g| g.layer == FootprintLayer::Copper));
        assert!(fp.graphics.iter().any(|g| matches!(g.shape, Shape::Polygon { fill: true, .. })));
    }
}
