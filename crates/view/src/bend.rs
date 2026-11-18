//! How wires, buses and tracks bend while drawn: corner styles, the path between two
//! clicks, and the posture (which leg comes first), chosen automatically from the way the
//! cursor leaves the start point.

use model::*;

/// Corner style of drawn paths.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Corners {
    /// Horizontal and vertical legs.
    Deg90,
    /// A horizontal or vertical leg and a 45° one.
    Deg45,
    /// One straight segment at any angle.
    Free,
    /// A 45° path with the corner rounded by the largest arc the legs allow (tracks).
    Arc45,
    /// A 90° path with the corner rounded by the largest arc the legs allow (tracks).
    Arc90,
}

impl Corners {
    /// Styles of schematic wires and buses, in cycling order.
    pub const WIRE: &[Corners] = &[Corners::Deg90, Corners::Deg45, Corners::Free];
    /// Styles of tracks, in cycling order.
    pub const TRACK: &[Corners] = &[Corners::Deg45, Corners::Deg90, Corners::Arc45, Corners::Arc90];

    pub fn label(self) -> &'static str {
        match self {
            Corners::Deg90 => "90°",
            Corners::Deg45 => "45°",
            Corners::Free => "Free angle",
            Corners::Arc45 => "Arc 45°",
            Corners::Arc90 => "Arc 90°",
        }
    }

    /// Names of the two postures by their first leg: straight (horizontal), or not.
    pub fn postures(self) -> (&'static str, &'static str) {
        match self {
            Corners::Deg90 | Corners::Arc90 => ("Horizontal", "Vertical"),
            Corners::Deg45 | Corners::Arc45 => ("Straight", "Diagonal"),
            Corners::Free => ("", ""),
        }
    }

    /// The style after `self` in `styles` (wrapping around).
    pub fn next_in(self, styles: &[Corners]) -> Corners {
        let i = styles.iter().position(|&c| c == self).map_or(0, |i| i + 1);
        styles[i % styles.len()]
    }
}

/// Which leg comes first: chosen automatically, or fixed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Bend {
    /// From the way the cursor leaves the start point ([`Posture`]).
    #[default]
    Auto,
    /// Horizontal first (90°) or straight first (45°).
    First,
    /// Vertical first (90°) or diagonal first (45°).
    Second,
}

/// Corner points of a path from `a` to `b`: 90° and 45° paths have at most one corner;
/// `second` picks the second posture (vertical first, or diagonal first).
pub fn path(a: Point, b: Point, corners: Corners, second: bool) -> Vec<Point> {
    let (dx, dy) = (b.x - a.x, b.y - a.y);
    let diagonal = matches!(corners, Corners::Deg45 | Corners::Arc45);
    if corners == Corners::Free || dx == 0 || dy == 0 || (diagonal && dx.abs() == dy.abs()) {
        return vec![a, b];
    }
    let corner = match corners {
        Corners::Deg90 | Corners::Arc90 if second => Point::new(a.x, b.y),
        Corners::Deg90 | Corners::Arc90 => Point::new(b.x, a.y),
        _ => {
            let m = dx.abs().min(dy.abs());
            let diag = Point::new(dx.signum() * m, dy.signum() * m);
            if second { a + diag } else { b - diag }
        }
    };
    vec![a, corner, b]
}

/// Segments of a path from `a` to `b` as (start, end, point on the arc): arc styles round
/// the corner of [`path`].
pub fn segments(a: Point, b: Point, corners: Corners, second: bool) -> Vec<(Point, Point, Option<Point>)> {
    let pts = path(a, b, corners, second);
    if let ([a, c, b], Corners::Arc45 | Corners::Arc90) = (&pts[..], corners)
        && let Some((p1, mid, p2)) = editor_core::fillet::fillet(*a, *c, *b, None)
    {
        return [(*a, p1, None), (p1, p2, Some(mid)), (p2, *b, None)].into_iter().filter(|s| s.0 != s.1).collect();
    }
    pts.windows(2).map(|w| (w[0], w[1], None)).collect()
}

/// The automatic posture of the leg being drawn. It is decided when the cursor first
/// leaves the start point, by the direction it goes, and stays until the cursor comes
/// back to the start point. [`Posture::flip`] overrides it until the next leg.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Posture {
    /// The second posture is chosen.
    second: bool,
    /// The next move away from the start decides.
    armed: bool,
    /// Flipped by hand: kept until the next leg.
    pinned: bool,
}

impl Default for Posture {
    fn default() -> Self {
        Posture { second: false, armed: true, pinned: false }
    }
}

impl Posture {
    /// A new leg starts: the next move decides again.
    pub fn start(&mut self) {
        self.armed = true;
        self.pinned = false;
    }

    /// The cursor moved to `cursor` while drawing a leg from `start`.
    pub fn track(&mut self, start: Point, cursor: Point, corners: Corners) {
        if cursor == start {
            self.armed = true;
            return;
        }
        if !self.armed || self.pinned {
            return;
        }
        let (dx, dy) = ((cursor.x - start.x).abs() as f64, (cursor.y - start.y).abs() as f64);
        self.second = match corners {
            Corners::Deg90 | Corners::Arc90 => dy > dx,
            // Leaving at a steep angle (more than 22.5° off the axis) starts diagonally.
            Corners::Deg45 | Corners::Arc45 => dx.min(dy) > dx.max(dy) * std::f64::consts::FRAC_PI_8.tan(),
            Corners::Free => self.second,
        };
        self.armed = false;
    }

    /// Flip the posture by hand (`/`); kept until the next leg.
    pub fn flip(&mut self) {
        self.second = !self.second;
        self.pinned = true;
    }

    /// Whether the second posture applies under `bend`.
    pub fn second(&self, bend: Bend) -> bool {
        match bend {
            Bend::Auto => self.second,
            Bend::First => false,
            Bend::Second => true,
        }
    }
}

/// `/`: flip the automatic posture, or swap a fixed one.
pub fn flip(bend: &mut Bend, posture: &mut Posture) {
    match bend {
        Bend::Auto => posture.flip(),
        Bend::First => *bend = Bend::Second,
        Bend::Second => *bend = Bend::First,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths() {
        let (a, b) = (Point::mm(0.0, 0.0), Point::mm(5.08, 2.54));
        assert_eq!(path(a, b, Corners::Deg90, false), vec![a, Point::mm(5.08, 0.0), b]);
        assert_eq!(path(a, b, Corners::Deg90, true), vec![a, Point::mm(0.0, 2.54), b]);
        assert_eq!(path(a, b, Corners::Deg45, false), vec![a, Point::mm(2.54, 0.0), b]);
        assert_eq!(path(a, b, Corners::Deg45, true), vec![a, Point::mm(2.54, 2.54), b]);
        assert_eq!(path(a, b, Corners::Free, false), vec![a, b]);
        assert_eq!(path(a, Point::mm(0.0, 5.0), Corners::Deg90, false).len(), 2);
    }

    #[test]
    fn posture_follows_the_first_move_until_back_at_the_start() {
        let s = Point::ZERO;
        let mut p = Posture::default();
        p.track(s, Point::mm(0.0, 1.27), Corners::Deg90);
        assert!(p.second(Bend::Auto), "left upwards: vertical first");
        // Later moves don't change it...
        p.track(s, Point::mm(10.0, 2.0), Corners::Deg90);
        assert!(p.second(Bend::Auto));
        // ...until the cursor comes back to the start.
        p.track(s, s, Corners::Deg90);
        p.track(s, Point::mm(1.27, 0.0), Corners::Deg90);
        assert!(!p.second(Bend::Auto));
        // A flip holds until the next leg.
        p.flip();
        p.track(s, s, Corners::Deg90);
        p.track(s, Point::mm(1.27, 0.0), Corners::Deg90);
        assert!(p.second(Bend::Auto));
        p.start();
        p.track(s, Point::mm(1.27, 0.0), Corners::Deg90);
        assert!(!p.second(Bend::Auto));
        assert!(p.second(Bend::Second) && !p.second(Bend::First));
    }

    #[test]
    fn diagonal_first_when_leaving_steeply() {
        let s = Point::ZERO;
        let mut p = Posture::default();
        p.track(s, Point::mm(1.0, 0.2), Corners::Deg45);
        assert!(!p.second(Bend::Auto));
        let mut p = Posture::default();
        p.track(s, Point::mm(1.0, 0.9), Corners::Deg45);
        assert!(p.second(Bend::Auto));
    }

    #[test]
    fn arc_styles_round_the_corner() {
        let (a, b) = (Point::mm(0.0, 0.0), Point::mm(10.0, 4.0));
        let s = segments(a, b, Corners::Arc90, false);
        assert_eq!(s.len(), 2, "the shorter leg is used up: {s:?}");
        assert_eq!((s[0].0, s[0].1, s[0].2), (a, Point::mm(6.0, 0.0), None));
        assert!(s[1].2.is_some() && s[1].1 == b);
        let s = segments(a, b, Corners::Arc45, false);
        assert_eq!(s.len(), 2);
        assert!(s[1].2.is_some() && s[1].1 == b);
        assert_eq!(segments(a, b, Corners::Deg90, false).len(), 2);
    }

    #[test]
    fn styles_cycle() {
        assert_eq!(Corners::Free.next_in(Corners::WIRE), Corners::Deg90);
        assert_eq!(Corners::Deg45.next_in(Corners::TRACK), Corners::Deg90);
    }
}
