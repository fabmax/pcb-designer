//! The board as Gerber files, one per layer: copper, solder mask, paste, silkscreen and
//! the board outline.

use editor_core::copper::BoardConnectivity;
use editor_core::fab::{PadOutline, pad_copper_layers, pad_mask_layers, pad_outline, pad_paste_layer};
use editor_core::query::footprint_of_instance;
use model::*;
use render::font;

use crate::bridge::bridge_holes;
use crate::writer::{Aperture, GerberWriter};

/// Lines without a width (outline drawings) are written this wide.
pub const DEFAULT_LINE_WIDTH: Coord = 100_000;

/// What a file holds: a board layer or drills.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FileKind {
    Layer(LayerId),
    PlatedDrill,
    NonPlatedDrill,
}

/// The file name suffix (KiCad's layer names) and the X2 file attributes of a layer.
pub fn layer_naming(board: &Board, layer: LayerId) -> Option<(String, Vec<String>)> {
    let mut copper: Vec<LayerId> = board.copper_layers().map(|l| l.id).collect();
    copper.sort();
    let side = |top: bool| if top { "Top" } else { "Bot" };
    Some(match layer {
        l if l.is_copper() => {
            let n = copper.iter().position(|&c| c == l)? + 1;
            let (suffix, pos) = match l {
                LayerId::TOP_COPPER => ("F_Cu".to_string(), "Top"),
                LayerId::BOTTOM_COPPER => ("B_Cu".to_string(), "Bot"),
                _ => (format!("In{}_Cu", n - 1), "Inr"),
            };
            (suffix, vec![format!(".FileFunction,Copper,L{n},{pos}"), ".FilePolarity,Positive".into()])
        }
        LayerId::TOP_MASK | LayerId::BOTTOM_MASK => {
            let top = layer == LayerId::TOP_MASK;
            (
                format!("{}_Mask", if top { "F" } else { "B" }),
                vec![format!(".FileFunction,Soldermask,{}", side(top)), ".FilePolarity,Negative".into()],
            )
        }
        LayerId::TOP_PASTE | LayerId::BOTTOM_PASTE => {
            let top = layer == LayerId::TOP_PASTE;
            (
                format!("{}_Paste", if top { "F" } else { "B" }),
                vec![format!(".FileFunction,Paste,{}", side(top)), ".FilePolarity,Positive".into()],
            )
        }
        LayerId::TOP_SILK | LayerId::BOTTOM_SILK => {
            let top = layer == LayerId::TOP_SILK;
            (
                format!("{}_Silkscreen", if top { "F" } else { "B" }),
                vec![format!(".FileFunction,Legend,{}", side(top)), ".FilePolarity,Positive".into()],
            )
        }
        LayerId::OUTLINE => {
            ("Edge_Cuts".into(), vec![".FileFunction,Profile,NP".into(), ".FilePolarity,Positive".into()])
        }
        _ => return None,
    })
}

/// The exported layers in file order: copper top to bottom, then mask, paste, silkscreen
/// (top, bottom) and the outline.
pub fn exported_layers(board: &Board) -> Vec<LayerId> {
    let mut copper: Vec<LayerId> = board.copper_layers().map(|l| l.id).collect();
    copper.sort();
    copper.extend([
        LayerId::TOP_MASK,
        LayerId::BOTTOM_MASK,
        LayerId::TOP_PASTE,
        LayerId::BOTTOM_PASTE,
        LayerId::TOP_SILK,
        LayerId::BOTTOM_SILK,
        LayerId::OUTLINE,
    ]);
    copper
}

fn is_bottom(layer: LayerId) -> bool {
    matches!(layer, LayerId::BOTTOM_COPPER | LayerId::BOTTOM_MASK | LayerId::BOTTOM_PASTE | LayerId::BOTTOM_SILK)
}

/// The Gerber file of one layer. `copper` provides the zone fills.
pub fn layer_file(project: &Project, copper: &BoardConnectivity, layer: LayerId) -> String {
    let board = &project.board;
    let (_, attributes) = layer_naming(board, layer).expect("an exported layer");
    let mut w = GerberWriter::new(&attributes);
    let is_copper = layer.is_copper();
    let mut copper_ids: Vec<LayerId> = board.copper_layers().map(|l| l.id).collect();
    copper_ids.sort();

    // Zone fills first (copper only; holes bridged so nothing else gets erased).
    if is_copper {
        for fill in copper.fills.iter().filter(|f| f.layer == layer) {
            for shape in &fill.shapes {
                let (outer, holes) = shape.split_first().expect("shapes have an outline");
                w.region(&bridge_holes(outer, holes), Some("Conductor"));
            }
        }
        for t in board.tracks.iter().filter(|t| t.layer == layer) {
            w.aperture(Aperture::Circle(t.width), Some("Conductor"));
            w.line(t.a, t.b);
        }
        for v in board.vias.iter().filter(|v| v.from.0 <= layer.0 && layer.0 <= v.to.0) {
            w.aperture(Aperture::Circle(v.diameter), Some("ViaPad"));
            w.flash(v.pos);
        }
    }

    let rules = &board.rules;
    for inst in board.footprints.iter() {
        let Some(fp) = footprint_of_instance(project, inst) else { continue };
        let t = inst.transform();
        for pad in fp.pads.iter() {
            let (margin, function) = if is_copper {
                if !pad_copper_layers(&pad.kind, inst.side, &copper_ids).contains(&layer) {
                    continue;
                }
                (0, Some(if matches!(pad.kind, PadKind::Smd) { "SMDPad,CuDef" } else { "ComponentPad" }))
            } else if matches!(layer, LayerId::TOP_MASK | LayerId::BOTTOM_MASK) {
                if !pad_mask_layers(&pad.kind, inst.side).contains(&layer) {
                    continue;
                }
                (rules.mask_expansion, None)
            } else if matches!(layer, LayerId::TOP_PASTE | LayerId::BOTTOM_PASTE) {
                if pad_paste_layer(&pad.kind, inst.side) != Some(layer) {
                    continue;
                }
                (rules.paste_margin, None)
            } else {
                continue;
            };
            if let Some(outline) = pad_outline(pad, &t, margin) {
                write_pad(&mut w, &outline, function);
            }
        }
        let component = project.schematic.components.get(inst.component);
        let (refdes, value) = component.map_or(("?", ""), |c| (c.refdes.as_str(), c.value.as_str()));
        for g in fp.graphics.iter().filter(|g| LayerId::for_footprint(g.layer, inst.side) == layer) {
            write_shape(&mut w, &g.shape.transformed(&t), graphic_function(layer));
        }
        for ft in fp.texts.iter().filter(|ft| LayerId::for_footprint(ft.layer, inst.side) == layer) {
            let text = ft.text.text.replace("${REF}", refdes).replace("${VALUE}", value);
            write_text(&mut w, &ft.text.transformed(&t), &text, is_bottom(layer), text_function(layer));
        }
    }

    for g in board.graphics.iter().filter(|g| g.layer == layer) {
        write_shape(&mut w, &g.shape, graphic_function(layer));
    }
    for bt in board.texts.iter().filter(|t| t.layer == layer) {
        write_text(&mut w, &bt.text, &bt.text.text, is_bottom(layer), text_function(layer));
    }
    w.finish()
}

fn graphic_function(layer: LayerId) -> Option<&'static str> {
    if layer == LayerId::OUTLINE {
        Some("Profile")
    } else if layer.is_copper() {
        Some("Conductor")
    } else {
        None
    }
}

fn text_function(layer: LayerId) -> Option<&'static str> {
    if layer.is_copper() { Some("NonConductor") } else { graphic_function(layer) }
}

fn write_pad(w: &mut GerberWriter, outline: &PadOutline, function: Option<&str>) {
    // Rectangles and ovals turned by quarter turns swap their sides; any other angle
    // goes through the RoundRect macro.
    let turned = |size: Size, rotation: Angle| match rotation.quarter_turns() {
        Some(q) if q % 2 == 0 => Some((size.w, size.h)),
        Some(_) => Some((size.h, size.w)),
        None => None,
    };
    match outline {
        PadOutline::Circle { center, diameter } => {
            w.aperture(Aperture::Circle(*diameter), function);
            w.flash(*center);
        }
        PadOutline::Rect { center, size, rotation } => {
            let ap = match turned(*size, *rotation) {
                Some((a, b)) => Aperture::Rect(a, b),
                None => round_rect(*size, 0, *rotation),
            };
            w.aperture(ap, function);
            w.flash(*center);
        }
        PadOutline::Oval { center, size, rotation } => {
            let ap = match turned(*size, *rotation) {
                Some((a, b)) => Aperture::Oval(a, b),
                None => round_rect(*size, size.w.min(size.h) / 2, *rotation),
            };
            w.aperture(ap, function);
            w.flash(*center);
        }
        PadOutline::RoundRect { center, size, radius, rotation } => {
            w.aperture(round_rect(*size, *radius, *rotation), function);
            w.flash(*center);
        }
        PadOutline::Polygon(shapes) => {
            for shape in shapes {
                let (outer, holes) = shape.split_first().expect("shapes have an outline");
                w.region(&bridge_holes(outer, holes), function);
            }
        }
    }
}

/// The `RoundRect` aperture of a rectangle with corner radius `r`, turned by `rotation`.
fn round_rect(size: Size, r: Coord, rotation: Angle) -> Aperture {
    let (x, y) = (size.w / 2 - r, size.h / 2 - r);
    let corners =
        [Point::new(-x, -y), Point::new(x, -y), Point::new(x, y), Point::new(-x, y)].map(|c| c.rotated(rotation));
    Aperture::RoundRect { r, corners }
}

fn stroke_width(width: Coord) -> Coord {
    if width > 0 { width } else { DEFAULT_LINE_WIDTH }
}

fn closed(w: &mut GerberWriter, points: &[Point]) {
    for i in 0..points.len() {
        w.line(points[i], points[(i + 1) % points.len()]);
    }
}

fn write_shape(w: &mut GerberWriter, shape: &Shape, function: Option<&str>) {
    match shape {
        Shape::Line { a, b, width } => {
            w.aperture(Aperture::Circle(stroke_width(*width)), function);
            w.line(*a, *b);
        }
        Shape::Arc { center, radius, start, sweep, width } => {
            w.aperture(Aperture::Circle(stroke_width(*width)), function);
            let at = |deg: f64| {
                let r = deg.to_radians();
                *center
                    + Point::new(
                        (*radius as f64 * r.cos()).round() as Coord,
                        (*radius as f64 * r.sin()).round() as Coord,
                    )
            };
            let a = at(start.degrees());
            if sweep.abs() >= 360.0 {
                w.arc(a, a, *center, true);
            } else {
                w.arc(a, at(start.degrees() + sweep), *center, *sweep > 0.0);
            }
        }
        Shape::Circle { center, radius, width, fill } => {
            if *fill {
                w.aperture(Aperture::Circle(2 * radius + width.max(&0)), function);
                w.flash(*center);
            } else {
                w.aperture(Aperture::Circle(stroke_width(*width)), function);
                let a = *center + Point::new(*radius, 0);
                w.arc(a, a, *center, true);
            }
        }
        Shape::Rect { a, b, width, fill } => {
            let pts = [*a, Point::new(b.x, a.y), *b, Point::new(a.x, b.y)];
            polygon(w, &pts, *width, *fill, function);
        }
        Shape::Polygon { points, width, fill } => polygon(w, points, *width, *fill, function),
    }
}

fn polygon(w: &mut GerberWriter, points: &[Point], width: Coord, fill: bool, function: Option<&str>) {
    if fill {
        w.region(points, function);
        if width <= 0 {
            return;
        }
    }
    w.aperture(Aperture::Circle(stroke_width(width)), function);
    closed(w, points);
}

/// Stroked text; on bottom layers mirrored around its anchor so it reads from below.
fn write_text(w: &mut GerberWriter, t: &Text, s: &str, mirror: bool, function: Option<&str>) {
    if s.is_empty() {
        return;
    }
    let h = match t.halign {
        HAlign::Left => font::HAlign::Left,
        HAlign::Center => font::HAlign::Center,
        HAlign::Right => font::HAlign::Right,
    };
    let v = match t.valign {
        VAlign::Bottom => font::VAlign::Bottom,
        VAlign::Center => font::VAlign::Center,
        VAlign::Top => font::VAlign::Top,
    };
    let (sin, cos) = t.rotation.radians().sin_cos();
    let place = |p: [f32; 2]| {
        let (x, y) = (if mirror { -p[0] as f64 } else { p[0] as f64 }, p[1] as f64);
        t.pos + Point::new(((x * cos - y * sin) * 1e6).round() as Coord, ((x * sin + y * cos) * 1e6).round() as Coord)
    };
    let mut segments = Vec::new();
    font::text_segments(s, [0.0, 0.0], (t.size as f64 * 1e-6) as f32, 0.0, h, v, |a, b| segments.push((a, b)));
    w.aperture(Aperture::Circle(t.width.max(1)), function);
    for (a, b) in segments {
        w.line(place(a), place(b));
    }
}
