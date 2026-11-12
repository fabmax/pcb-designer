//! The measure tool of the layout editors (PCB, footprint, Gerber): click a start point,
//! the ruler follows the pointer, a second click pins it, the next click starts anew.
//! Ends snap to points on objects (pad centres, track ends, vertices…) within a few
//! pixels, else to the grid; the app draws it as an overlay.

use editor_core::query::footprint_of_instance;
use model::*;

/// The usage hint of the tool.
pub const HINT: &str = "Click two points to measure · ends snap to pads and corners (Alt: free) · Esc clears";

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Measure {
    start: Option<Point>,
    end: Option<Point>,
    /// The second click happened: the ruler stays put.
    pinned: bool,
    /// The object point the pointer snaps to (drawn as a ring).
    pub target: Option<Point>,
}

/// Length, offsets and direction of a measurement.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Reading {
    /// nm
    pub length: f64,
    pub dx: Coord,
    pub dy: Coord,
    /// Counterclockwise from +X, in degrees (-180, 180].
    pub angle: f64,
}

impl Measure {
    /// The pointer moved to the (snapped) point `p` (`None`: it left the canvas);
    /// `on_object`: `p` is a point on an object.
    pub fn hover(&mut self, p: Option<Point>, on_object: bool) {
        self.target = p.filter(|_| on_object);
        if let Some(p) = p
            && self.start.is_some()
            && !self.pinned
        {
            self.end = Some(p);
        }
    }

    /// A click at `p`: start a measurement, or pin the one in progress.
    pub fn click(&mut self, p: Point) {
        if self.start.is_none() || self.pinned {
            self.start = Some(p);
            self.end = Some(p);
            self.pinned = false;
        } else {
            self.end = Some(p);
            self.pinned = true;
        }
    }

    /// Esc: clear the measurement. Returns false if there was none (leave the tool).
    pub fn escape(&mut self) -> bool {
        let had = self.start.is_some();
        self.clear();
        had
    }

    pub fn clear(&mut self) {
        *self = Measure::default();
    }

    pub fn is_pinned(&self) -> bool {
        self.pinned
    }

    /// The ruler: start and end.
    pub fn segment(&self) -> Option<(Point, Point)> {
        Some((self.start?, self.end?))
    }

    pub fn reading(&self) -> Option<Reading> {
        let (a, b) = self.segment()?;
        let (dx, dy) = (b.x - a.x, b.y - a.y);
        let angle = if dx == 0 && dy == 0 { 0.0 } else { (dy as f64).atan2(dx as f64).to_degrees() };
        Some(Reading { length: (dx as f64).hypot(dy as f64), dx, dy, angle })
    }
}

/// Where the pointer at `raw` snaps to: the nearest of `points` within `tol`, else
/// `grid` (the grid-snapped point). `free` (Alt held) takes `raw` as it is. Returns the
/// point and whether it is on an object.
pub fn snap(raw: Point, grid: Point, points: impl IntoIterator<Item = Point>, tol: Coord, free: bool) -> (Point, bool) {
    if free {
        return (raw, false);
    }
    let nearest = points
        .into_iter()
        .map(|q| ((q - raw).length(), q))
        .filter(|(d, _)| *d <= tol as f64)
        .min_by(|a, b| a.0.total_cmp(&b.0));
    match nearest {
        Some((_, q)) => (q, true),
        None => (grid, false),
    }
}

/// Points of a shape worth measuring from: ends, corners, vertices, centres.
pub fn shape_points(shape: &Shape) -> Vec<Point> {
    match shape {
        Shape::Line { a, b, .. } => vec![*a, *b],
        Shape::Arc { center, radius, start, sweep, .. } => {
            let at = |deg: f64| {
                let r = deg.to_radians();
                *center
                    + Point::new(
                        (*radius as f64 * r.cos()).round() as Coord,
                        (*radius as f64 * r.sin()).round() as Coord,
                    )
            };
            vec![*center, at(start.degrees()), at(start.degrees() + sweep)]
        }
        Shape::Circle { center, .. } => vec![*center],
        Shape::Rect { a, b, .. } => vec![*a, Point::new(b.x, a.y), *b, Point::new(a.x, b.y)],
        Shape::Polygon { points, .. } => points.clone(),
    }
}

/// Snap points of the board: pad centres (holes included), via centres, track ends and
/// the vertices of board drawings (the outline).
pub fn board_points(project: &Project) -> impl Iterator<Item = Point> + '_ {
    let board = &project.board;
    let pads = board.footprints.iter().flat_map(move |inst| {
        let t = inst.transform();
        footprint_of_instance(project, inst).into_iter().flat_map(move |fp| fp.pads.iter().map(move |p| t.point(p.pos)))
    });
    let vias = board.vias.iter().map(|v| v.pos);
    let tracks = board.tracks.iter().flat_map(|t| [t.a, t.b]);
    let graphics = board.graphics.iter().flat_map(|g| shape_points(&g.shape));
    pads.chain(vias).chain(tracks).chain(graphics)
}

/// Snap points of a footprint (in its own coordinates): pad centres and graphic vertices.
pub fn footprint_points(fp: &Footprint) -> impl Iterator<Item = Point> + '_ {
    fp.pads.iter().map(|p| p.pos).chain(fp.graphics.iter().flat_map(|g| shape_points(&g.shape)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn click_click_and_start_again() {
        let mut m = Measure::default();
        assert_eq!(m.reading(), None);
        m.click(Point::mm(1.0, 1.0));
        m.hover(Some(Point::mm(4.0, 5.0)), false);
        let r = m.reading().unwrap();
        assert_eq!((r.length, r.dx, r.dy), (5e6, mm(3.0), mm(4.0)));
        m.click(Point::mm(4.0, 5.0));
        assert!(m.is_pinned());
        // Pinned: hovering doesn't move it.
        m.hover(Some(Point::mm(9.0, 9.0)), true);
        assert_eq!(m.segment().unwrap().1, Point::mm(4.0, 5.0));
        assert_eq!(m.target, Some(Point::mm(9.0, 9.0)));
        // The next click starts a new measurement.
        m.click(Point::mm(9.0, 9.0));
        assert!(!m.is_pinned());
        assert_eq!(m.reading().unwrap().length, 0.0);
        assert!(m.escape());
        assert!(!m.escape(), "nothing left: the tool is left");
    }

    #[test]
    fn angles() {
        let mut m = Measure::default();
        m.click(Point::ZERO);
        m.hover(Some(Point::mm(-1.0, 0.0)), false);
        assert_eq!(m.reading().unwrap().angle, 180.0);
        m.hover(Some(Point::mm(0.0, -2.0)), false);
        assert_eq!(m.reading().unwrap().angle, -90.0);
    }

    #[test]
    fn snapping() {
        let pts = [Point::mm(1.0, 0.0), Point::mm(1.2, 0.0)];
        let raw = Point::mm(1.15, 0.05);
        let grid = Point::mm(1.0, 0.0);
        assert_eq!(snap(raw, grid, pts, mm(0.2), false), (Point::mm(1.2, 0.0), true));
        assert_eq!(snap(raw, grid, pts, mm(0.01), false), (grid, false), "too far: the grid");
        assert_eq!(snap(raw, grid, pts, mm(0.2), true), (raw, false), "Alt: free");
    }

    #[test]
    fn board_points_cover_pads_tracks_and_outline() {
        let p = crate::demo::demo_project();
        let points: Vec<Point> = board_points(&p).collect();
        let t = p.board.tracks.iter().next().unwrap();
        assert!(points.contains(&t.a) && points.contains(&t.b));
        let inst = p.board.footprints.iter().next().unwrap();
        let pad = footprint_of_instance(&p, inst).unwrap().pads.iter().next().unwrap();
        assert!(points.contains(&inst.transform().point(pad.pos)));
        let outline = p.board.graphics.iter().find(|g| g.layer == LayerId::OUTLINE).unwrap();
        assert!(shape_points(&outline.shape).iter().all(|q| points.contains(q)));
    }
}
