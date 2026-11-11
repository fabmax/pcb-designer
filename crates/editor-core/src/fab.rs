//! Fabrication geometry shared by the PCB view and the Gerber export: which layers a pad
//! puts copper, solder mask openings and paste on, and its placed outline grown or shrunk
//! by a margin (`DesignRules::mask_expansion`, `paste_margin`).

use i_overlay::mesh::float::outline::offset::OutlineOffset;
use i_overlay::mesh::float::style::{LineJoin, OutlineStyle};
use model::*;

/// Outline of a placed pad in board coordinates. `rotation` is counterclockwise and
/// already includes the footprint's placement (mirroring included).
#[derive(Clone, Debug, PartialEq)]
pub enum PadOutline {
    Circle {
        center: Point,
        diameter: Coord,
    },
    Rect {
        center: Point,
        size: Size,
        rotation: Angle,
    },
    /// Rectangle with semicircular short ends.
    Oval {
        center: Point,
        size: Size,
        rotation: Angle,
    },
    RoundRect {
        center: Point,
        size: Size,
        radius: Coord,
        rotation: Angle,
    },
    /// Shapes of `[outline, holes…]`, outlines counterclockwise (a shrunk polygon may
    /// split into several).
    Polygon(Vec<Vec<Vec<Point>>>),
}

/// Copper layers of a pad: SMD pads on their side, through-hole pads on all of
/// `copper`, non-plated holes on none.
pub fn pad_copper_layers(kind: &PadKind, side: Side, copper: &[LayerId]) -> Vec<LayerId> {
    match kind {
        PadKind::Smd => vec![LayerId::for_footprint(FootprintLayer::Copper, side)],
        PadKind::ThroughHole { .. } => copper.to_vec(),
        PadKind::NonPlatedHole { .. } => vec![],
    }
}

/// Solder mask layers a pad opens: SMD pads on their side, holes on both.
pub fn pad_mask_layers(kind: &PadKind, side: Side) -> Vec<LayerId> {
    match kind {
        PadKind::Smd => vec![LayerId::for_footprint(FootprintLayer::SolderMask, side)],
        PadKind::ThroughHole { .. } | PadKind::NonPlatedHole { .. } => vec![LayerId::TOP_MASK, LayerId::BOTTOM_MASK],
    }
}

/// Paste layer of a pad: SMD pads only.
pub fn pad_paste_layer(kind: &PadKind, side: Side) -> Option<LayerId> {
    matches!(kind, PadKind::Smd).then(|| LayerId::for_footprint(FootprintLayer::SolderPaste, side))
}

/// The pad placed by `t`, grown by `margin` (negative shrinks). `None` when shrinking
/// leaves nothing. Corners stay sharp when growing a rectangle (as in KiCad); polygons
/// grow with round corners.
pub fn pad_outline(pad: &Pad, t: &Transform, margin: Coord) -> Option<PadOutline> {
    let center = t.point(pad.pos);
    let local = if t.mirror { Angle::deg(-pad.rotation.degrees()) } else { pad.rotation };
    let rotation = local.rotated_by(t.rotation);
    let size = Size::new(pad.size.w + 2 * margin, pad.size.h + 2 * margin);
    let short = size.w.min(size.h);
    if !matches!(pad.shape, PadShape::Polygon { .. }) && short <= 0 {
        return None;
    }
    Some(match &pad.shape {
        PadShape::Circle => PadOutline::Circle { center, diameter: short },
        PadShape::Rect => PadOutline::Rect { center, size, rotation },
        PadShape::Oval if size.w == size.h => PadOutline::Circle { center, diameter: short },
        PadShape::Oval => PadOutline::Oval { center, size, rotation },
        PadShape::RoundRect { ratio } => {
            let radius =
                ((ratio.clamp(0.0, 0.5) * pad.size.w.min(pad.size.h) as f64) as Coord + margin).clamp(0, short / 2);
            if radius == 0 {
                PadOutline::Rect { center, size, rotation }
            } else {
                PadOutline::RoundRect { center, size, radius, rotation }
            }
        }
        PadShape::Polygon { points } => {
            let placed: Vec<Point> = points.iter().map(|&q| t.point(pad.pos + q.rotated(pad.rotation))).collect();
            if placed.len() < 3 {
                return None;
            }
            let contours = offset_polygon(&placed, margin);
            if contours.is_empty() {
                return None;
            }
            PadOutline::Polygon(contours)
        }
    })
}

/// Polygon grown by `d` (negative shrinks) with round joins, as shapes of
/// `[outline, holes…]`.
fn offset_polygon(points: &[Point], d: Coord) -> Vec<Vec<Vec<Point>>> {
    let mut path: Vec<[f64; 2]> = points.iter().map(|p| [p.x as f64, p.y as f64]).collect();
    if signed_area(&path) < 0.0 {
        path.reverse();
    }
    if d == 0 {
        return vec![vec![path.iter().map(|p| Point::new(p[0] as Coord, p[1] as Coord)).collect()]];
    }
    // About 2° per segment of a round join.
    let style = OutlineStyle::new(d as f64).line_join(LineJoin::Round(0.035));
    let point = |p: [f64; 2]| Point::new(p[0].round() as Coord, p[1].round() as Coord);
    path.outline_as::<i64>(&style)
        .into_iter()
        .map(|shape| shape.into_iter().map(|c| c.into_iter().map(point).collect()).collect())
        .collect()
}

fn signed_area(path: &[[f64; 2]]) -> f64 {
    let n = path.len();
    (0..n).map(|i| path[i][0] * path[(i + 1) % n][1] - path[(i + 1) % n][0] * path[i][1]).sum::<f64>() / 2.0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pad(shape: PadShape, kind: PadKind) -> Pad {
        Pad {
            id: Id::new(),
            name: "1".into(),
            pos: Point::mm(1.0, 0.0),
            rotation: Angle::DEG90,
            size: Size::mm(2.0, 1.0),
            shape,
            kind,
        }
    }

    #[test]
    fn layers_by_kind() {
        let copper = [LayerId::TOP_COPPER, LayerId::BOTTOM_COPPER];
        assert_eq!(pad_mask_layers(&PadKind::Smd, Side::Bottom), vec![LayerId::BOTTOM_MASK]);
        assert_eq!(pad_paste_layer(&PadKind::Smd, Side::Top), Some(LayerId::TOP_PASTE));
        let th = PadKind::ThroughHole { drill: mm(0.8) };
        assert_eq!(pad_mask_layers(&th, Side::Top), vec![LayerId::TOP_MASK, LayerId::BOTTOM_MASK]);
        assert_eq!(pad_paste_layer(&th, Side::Top), None);
        assert_eq!(pad_copper_layers(&th, Side::Top, &copper), copper.to_vec());
        assert!(pad_copper_layers(&PadKind::NonPlatedHole { drill: mm(3.0) }, Side::Top, &copper).is_empty());
    }

    #[test]
    fn grown_and_placed() {
        let p = pad(PadShape::RoundRect { ratio: 0.25 }, PadKind::Smd);
        let t = Transform::new(Point::mm(10.0, 0.0), Angle::DEG90, false);
        let Some(PadOutline::RoundRect { center, size, radius, rotation }) = pad_outline(&p, &t, mm(0.05)) else {
            panic!()
        };
        assert_eq!(center, Point::mm(10.0, 1.0));
        assert_eq!(size, Size::mm(2.1, 1.1));
        assert_eq!(radius, mm(0.3));
        assert_eq!(rotation, Angle::DEG180);
        // Mirrored (bottom side): the pad's own rotation flips.
        let t = Transform::new(Point::ZERO, Angle::ZERO, true);
        let r = pad(PadShape::Rect, PadKind::Smd);
        assert!(matches!(pad_outline(&r, &t, 0), Some(PadOutline::Rect { rotation, .. }) if rotation == Angle::DEG270));
        // Shrunk to nothing.
        assert_eq!(pad_outline(&r, &t, -mm(0.5)), None);
    }

    #[test]
    fn polygon_offsets() {
        let square = vec![Point::mm(-1.0, -1.0), Point::mm(1.0, -1.0), Point::mm(1.0, 1.0), Point::mm(-1.0, 1.0)];
        let mut p = pad(PadShape::Polygon { points: square }, PadKind::Smd);
        p.rotation = Angle::ZERO;
        p.pos = Point::ZERO;
        let bounds = |o: PadOutline| {
            let PadOutline::Polygon(c) = o else { panic!() };
            Rect::union_all(c.iter().flatten().flatten().map(|&q| Rect::from_points(q, q))).unwrap()
        };
        let grown = bounds(pad_outline(&p, &Transform::IDENTITY, mm(0.1)).unwrap());
        assert!((grown.max.x - mm(1.1)).abs() < 1000, "{grown:?}");
        let shrunk = bounds(pad_outline(&p, &Transform::IDENTITY, -mm(0.1)).unwrap());
        assert!((shrunk.max.x - mm(0.9)).abs() < 1000, "{shrunk:?}");
        assert_eq!(pad_outline(&p, &Transform::IDENTITY, -mm(1.5)), None);
    }
}
