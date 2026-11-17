//! The measure tool of the layout editors (PCB, footprint, Gerber): click a start point,
//! the ruler follows the pointer, a second click pins it, the next click starts anew.
//! Ends snap to points on objects (pad centres, track ends, vertices…) within a few
//! pixels, else to the grid; the app draws it as an overlay.

use editor_core::clearance::{Feature, Gap};
use editor_core::copper::{Geo, pad_geo, via_geo};
use editor_core::query::footprint_of_instance;
use model::*;
use rstar::{AABB, RTree, RTreeObject};

/// The usage hint of the tool.
pub const HINT: &str = if cfg!(target_os = "macos") {
    "Click two points to measure · ends snap to pads and corners (Alt: free) · ⌘C copies · Esc clears"
} else {
    "Click two points to measure · ends snap to pads and corners (Alt: free) · Ctrl+C copies · Esc clears"
};

/// The usage hint of the board's Clearance mode.
pub const CLEARANCE_HINT: &str =
    "Click two objects: copper, a drill hole or the board edge · their gap and the rule show · E points · Esc clears";

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

impl Reading {
    /// As text, lengths through `fmt` (the current unit):
    /// `12.700 mm (ΔX 12.700 mm, ΔY 0.000 mm, 0.0°)`.
    pub fn text(&self, fmt: impl Fn(Coord) -> String) -> String {
        format!("{} (ΔX {}, ΔY {}, {:.1}°)", fmt(self.length.round() as Coord), fmt(self.dx), fmt(self.dy), self.angle)
    }
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

/// Where the pointer at `raw` snaps to: the nearest of `points` within `tol`, else the
/// centre of the pad (via, flash) under the pointer (`inside`), else `grid` (the
/// grid-snapped point). `free` (Alt held) takes `raw` as it is. Returns the point and
/// whether it is on an object.
pub fn snap(
    raw: Point,
    grid: Point,
    points: impl IntoIterator<Item = Point>,
    inside: Option<Point>,
    tol: Coord,
    free: bool,
) -> (Point, bool) {
    if free {
        return (raw, false);
    }
    let nearest = points
        .into_iter()
        .map(|q| ((q - raw).length(), q))
        .filter(|(d, _)| *d <= tol as f64)
        .min_by(|a, b| a.0.total_cmp(&b.0));
    match nearest.map(|n| n.1).or(inside) {
        Some(q) => (q, true),
        None => (grid, false),
    }
}

/// A pad or via outline and its centre, in the snap index.
#[derive(Clone, Debug)]
struct Centred {
    geo: Geo,
    centre: Point,
    env: AABB<[i64; 2]>,
}

impl RTreeObject for Centred {
    type Envelope = AABB<[i64; 2]>;
    fn envelope(&self) -> Self::Envelope {
        self.env
    }
}

fn env(r: Rect) -> AABB<[i64; 2]> {
    AABB::from_corners([r.min.x, r.min.y], [r.max.x, r.max.y])
}

/// The board's snap points ([`board_points`]) and pad and via outlines in R-trees, built
/// once per document revision (hovering doesn't scan the board).
#[derive(Clone, Debug)]
pub struct BoardSnap {
    pub revision: u64,
    points: RTree<[i64; 2]>,
    centred: RTree<Centred>,
}

impl BoardSnap {
    pub fn build(project: &Project, revision: u64) -> BoardSnap {
        let points = RTree::bulk_load(board_points(project).map(|p| [p.x, p.y]).collect());
        let board = &project.board;
        let mut centred = Vec::new();
        for inst in board.footprints.iter() {
            let Some(fp) = footprint_of_instance(project, inst) else { continue };
            let t = inst.transform();
            for pad in fp.pads.iter() {
                let geo = pad_geo(pad, &t);
                centred.push(Centred { env: env(geo.bounds()), centre: t.point(pad.pos), geo });
            }
        }
        for v in board.vias.iter() {
            let geo = via_geo(v);
            centred.push(Centred { env: env(geo.bounds()), centre: v.pos, geo });
        }
        BoardSnap { revision, points, centred: RTree::bulk_load(centred) }
    }

    /// [`snap`] against the board.
    pub fn snap(&self, raw: Point, grid: Point, tol: Coord, free: bool) -> (Point, bool) {
        let t = tol;
        let area = AABB::from_corners([raw.x - t, raw.y - t], [raw.x + t, raw.y + t]);
        let near = self.points.locate_in_envelope(area).map(|p| Point::new(p[0], p[1]));
        let inside = self
            .centred
            .locate_in_envelope_intersecting(AABB::from_point([raw.x, raw.y]))
            .filter(|c| c.geo.contains(raw))
            .min_by(|a, b| (a.centre - raw).length().total_cmp(&(b.centre - raw).length()))
            .map(|c| c.centre);
        snap(raw, grid, near, inside, tol, free)
    }
}

/// The centre of the footprint pad (in footprint coordinates) under `p`.
pub fn footprint_pad_centre(fp: &Footprint, p: Point) -> Option<Point> {
    fp.pads.iter().find(|pad| pad_geo(pad, &Transform::IDENTITY).contains(p)).map(|pad| pad.pos)
}

/// The Measure tool's Clearance mode on the board: two features are picked (a click on
/// each), and the gap between them shows; before the second click, the gap to the
/// feature under the pointer.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Clearance {
    first: Option<Feature>,
    second: Option<Feature>,
    /// The feature under the pointer.
    pub hover: Option<Feature>,
}

impl Clearance {
    pub fn hover(&mut self, f: Option<Feature>) {
        self.hover = f;
    }

    /// A click on the hovered feature: pick the first one, then the second; a click
    /// after both starts anew. Nothing hovered: nothing happens.
    pub fn click(&mut self) {
        let Some(f) = self.hover else { return };
        if self.first.is_none() || self.second.is_some() {
            self.first = Some(f);
            self.second = None;
        } else {
            self.second = Some(f);
        }
    }

    /// Esc: clear. Returns false if there was nothing (leave the tool).
    pub fn escape(&mut self) -> bool {
        let had = self.first.is_some();
        self.clear();
        had
    }

    pub fn clear(&mut self) {
        *self = Clearance::default();
    }

    /// The features measured between: both picked, or the first and the hovered one.
    pub fn pair(&self) -> Option<(Feature, Feature)> {
        Some((self.first?, self.second.or(self.hover)?))
    }

    pub fn picked(&self) -> impl Iterator<Item = Feature> + '_ {
        self.first.into_iter().chain(self.second)
    }

    pub fn is_pinned(&self) -> bool {
        self.second.is_some()
    }
}

/// The reading of a clearance as text: `0.700 mm (rule 0.200 mm)`, `overlap`.
pub fn gap_text(gap: &Gap, fmt: impl Fn(Coord) -> String) -> String {
    let d = if gap.distance <= 0.0 { "touching".to_string() } else { fmt(gap.distance.round() as Coord) };
    match gap.rule {
        Some(r) if gap.violates() => format!("{d} (below the clearance of {})", fmt(r)),
        Some(r) => format!("{d} (clearance {})", fmt(r)),
        None => d,
    }
}

/// Outlines of a shape as polylines (closed ones repeat their first point), to draw it
/// highlighted.
pub fn geo_outlines(g: &Geo) -> Vec<Vec<Point>> {
    let close = |mut v: Vec<Point>| {
        if let Some(&f) = v.first() {
            v.push(f);
        }
        v
    };
    match g {
        Geo::Capsule { a, b, r } => vec![capsule_outline(*a, *b, *r)],
        Geo::Path { points, r: 0 } => vec![points.clone()],
        Geo::Path { points, r } => points.windows(2).map(|w| capsule_outline(w[0], w[1], *r)).collect(),
        Geo::Polygon(p) => vec![close(p.clone())],
        Geo::Region { outer, holes } => std::iter::once(outer).chain(holes).map(|c| close(c.clone())).collect(),
    }
}

/// A stadium around segment `ab` of radius `r` (a circle when `a == b`), closed.
fn capsule_outline(a: Point, b: Point, r: Coord) -> Vec<Point> {
    let d = b - a;
    let base = if d == Point::ZERO { 0.0 } else { (d.y as f64).atan2(d.x as f64) };
    let n = 16;
    let arc = |c: Point, from: f64| {
        (0..=n).map(move |i| {
            let t = from + std::f64::consts::PI * i as f64 / n as f64;
            c + Point::new((r as f64 * t.cos()).round() as Coord, (r as f64 * t.sin()).round() as Coord)
        })
    };
    let half = std::f64::consts::FRAC_PI_2;
    let mut out: Vec<Point> = arc(b, base - half).chain(arc(a, base + half)).collect();
    out.push(out[0]);
    out
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
    fn reading_as_text() {
        let mut m = Measure::default();
        m.click(Point::ZERO);
        m.click(Point::mm(3.0, 4.0));
        let text = m.reading().unwrap().text(|c| format!("{:.3} mm", c as f64 * 1e-6));
        assert_eq!(text, "5.000 mm (ΔX 3.000 mm, ΔY 4.000 mm, 53.1°)");
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
        assert_eq!(snap(raw, grid, pts, None, mm(0.2), false), (Point::mm(1.2, 0.0), true));
        assert_eq!(snap(raw, grid, pts, None, mm(0.01), false), (grid, false), "too far: the grid");
        assert_eq!(snap(raw, grid, pts, None, mm(0.2), true), (raw, false), "Alt: free");
        let centre = Some(Point::mm(3.0, 3.0));
        assert_eq!(snap(raw, grid, pts, centre, mm(0.01), false), (Point::mm(3.0, 3.0), true), "inside a pad");
        assert_eq!(snap(raw, grid, pts, centre, mm(0.2), false).0, Point::mm(1.2, 0.0), "near points win");
    }

    #[test]
    fn pads_snap_to_their_centre_from_inside() {
        let p = crate::demo::demo_project();
        let snap = BoardSnap::build(&p, 0);
        let inst = p.board.footprints.iter().next().unwrap();
        let pad = footprint_of_instance(&p, inst).unwrap().pads.iter().next().unwrap();
        let centre = inst.transform().point(pad.pos);
        // A bit off the centre, beyond the tolerance, still inside the pad.
        let off = centre + Point::new(pad.size.w.min(pad.size.h) / 4, 0);
        assert_eq!(snap.snap(off, Point::ZERO, 1_000, false), (centre, true));
        assert_eq!(snap.snap(off, Point::ZERO, 1_000, true), (off, false), "Alt: free");
    }

    #[test]
    fn clearance_picks_two_features() {
        let mut c = Clearance::default();
        c.click();
        assert_eq!(c.pair(), None, "nothing hovered");
        c.hover(Some(Feature::Edge));
        c.click();
        let v = Feature::Via { id: Id::new() };
        c.hover(Some(v));
        assert_eq!(c.pair(), Some((Feature::Edge, v)), "live to the hovered one");
        c.click();
        c.hover(None);
        assert!(c.is_pinned());
        assert_eq!(c.pair(), Some((Feature::Edge, v)));
        assert!(c.escape());
        assert!(!c.escape());
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
