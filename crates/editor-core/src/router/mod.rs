//! The interactive router (see routing-plan.md): obstacle hulls around the copper of
//! other nets and keepouts, and a track leg that walks round them. Pure: it reads the
//! project and its board connectivity and says where the leg goes; the Route tool adds it.

pub mod hull;
pub mod line;
pub mod shove;
pub mod smooth;
pub mod walk;

use model::*;

use crate::copper::{BoardConnectivity, Copper, CopperItem, layer_bit};
use crate::route_check::{self, Clash, Probe};
use hull::Hull;

/// Extra room the router keeps beyond the clearance (nm), so rounding to whole nm never
/// takes a path below it.
const SLACK: Coord = 10;

/// How the Route tool deals with copper in the way.
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum RouteMode {
    /// Show the clash and refuse the click.
    Highlight,
    /// Push other nets' tracks aside (going round what can't move).
    #[default]
    Shove,
    /// Go round obstacles.
    Walkaround,
}

impl RouteMode {
    pub const ALL: &[RouteMode] = &[RouteMode::Highlight, RouteMode::Shove, RouteMode::Walkaround];

    pub fn label(self) -> &'static str {
        match self {
            RouteMode::Highlight => "Highlight collisions",
            RouteMode::Shove => "Shove",
            RouteMode::Walkaround => "Walk around",
        }
    }

    pub fn next(self) -> RouteMode {
        let i = Self::ALL.iter().position(|&m| m == self).unwrap_or(0);
        Self::ALL[(i + 1) % Self::ALL.len()]
    }
}

/// A leg as the router lays it.
#[derive(Clone, Debug, PartialEq)]
pub struct Routed {
    /// The centre line, from the leg's start; a single point when nothing fits.
    pub points: Vec<Point>,
    /// It gets to the end asked for (else it stops short).
    pub reached: bool,
    /// What the straight leg runs into, when it stops short.
    pub blocked: Option<Clash>,
    /// Lines and vias pushed aside (Shove).
    pub shoved: shove::Shoved,
}

/// The leg along `points` laid in `mode` (Shove or Walkaround; Highlight takes it as it
/// is): see [`walkaround`] and [`shove`].
pub fn route(
    project: &Project,
    copper: &BoardConnectivity,
    mode: RouteMode,
    layer: LayerId,
    width: Coord,
    net: Option<&str>,
    points: &[Point],
) -> Routed {
    match mode {
        RouteMode::Highlight => {
            Routed { points: points.to_vec(), reached: true, blocked: None, shoved: Default::default() }
        }
        RouteMode::Walkaround => walkaround(project, copper, layer, width, net, points),
        RouteMode::Shove => shove(project, copper, layer, width, net, points),
    }
}

/// The leg along `points` walked round what can't be pushed (pads, vias, locked tracks,
/// arcs, keepouts), pushing the tracks in its way aside. Where they can't give way, the
/// leg stops at the furthest point that works.
pub fn shove(
    project: &Project,
    copper: &BoardConnectivity,
    layer: LayerId,
    width: Coord,
    net: Option<&str>,
    points: &[Point],
) -> Routed {
    let walked = lay(project, copper, layer, width, net, points, true);
    // A frame or two at most, the search for where to stop included.
    let deadline = std::time::Instant::now() + SHOVE_TIME;
    let attempt =
        |path: &[Point]| shove::shove(project, copper, &shove::Head { layer, width, net, path, via: None, deadline });
    let blocked = match attempt(&walked.points) {
        Ok(shoved) => return Routed { shoved, ..walked },
        Err(shove::Blocked(c)) => c,
    };
    // The longest start of the leg that pushes clear, to 20 µm.
    let (mut lo, mut hi) = (0.0, walk::length(&walked.points));
    let mut best = shove::Shoved::default();
    while hi - lo > 20_000.0 && std::time::Instant::now() < deadline {
        let mid = (lo + hi) / 2.0;
        match attempt(&prefix(&walked.points, mid)) {
            Ok(pushed) => {
                lo = mid;
                best = pushed;
            }
            Err(_) => hi = mid,
        }
    }
    let points = prefix(&walked.points, lo);
    let shoved = if points.len() < 2 { Default::default() } else { best };
    Routed { points, reached: false, blocked: Some(blocked), shoved }
}

/// What a via of `diameter` on `layers` at `pos` (the end of the track being routed, of
/// `net`, on `layer`) pushes aside; `Err`: what it can't.
pub fn shove_via(
    project: &Project,
    copper: &BoardConnectivity,
    layer: LayerId,
    net: Option<&str>,
    pos: Point,
    diameter: Coord,
    layers: crate::copper::LayerMask,
) -> Result<shove::Shoved, Clash> {
    // Fixed copper, keepouts and the board edge; what can move is pushed aside below.
    let probe = Probe::Via { pos, diameter, layers };
    if let Some(c) = route_check::check_with(project, copper, &[probe], net, &[(layer, pos)], &|c| movable(project, c))
    {
        return Err(c);
    }
    let deadline = std::time::Instant::now() + SHOVE_TIME;
    let path = [pos];
    let head = shove::Head { layer, width: 0, net, path: &path, via: Some((diameter, layers)), deadline };
    shove::shove(project, copper, &head).map_err(|b| b.0)
}

/// What tracks and vias just moved (by a drag, `moved`) push aside. `copper` is the board
/// from before the move (their old copper is no obstacle); `Err`: what they can't push.
pub fn shove_moved(project: &Project, copper: &BoardConnectivity, moved: &[ObjectId]) -> Result<shove::Shoved, Clash> {
    let board = &project.board;
    let mut shapes = Vec::new();
    let (mut islands, mut net) = (Vec::new(), None);
    for &id in moved {
        let (item, geo, layers) = match id {
            ObjectId::Track(t) => {
                let Some(t) = board.tracks.get(t) else { continue };
                (CopperItem::Track(t.id), crate::copper::track_geo(t), layer_bit(t.layer))
            }
            ObjectId::Via(v) => {
                let Some(v) = board.vias.get(v) else { continue };
                (CopperItem::Via(v.id), crate::copper::via_geo(v), crate::copper::via_layers(board, v))
            }
            _ => continue,
        };
        shapes.push((geo, layers));
        if let Some(c) = copper.get(item) {
            islands.push(c.island);
            net = net.or(copper.net_of(item));
        }
    }
    let deadline = std::time::Instant::now() + SHOVE_TIME;
    shove::shove_shapes(project, copper, shapes, net, islands, deadline).map_err(|b| b.0)
}

/// The leg along `points` (its first point is where it starts, the last the pointer) of
/// a track `width` wide on `layer`, of `net` (`None`: not known yet), walked round the
/// copper of other nets and the keepouts that forbid tracks, and cut short where it would
/// still break a rule (the board edge; anything round which no way was found).
pub fn walkaround(
    project: &Project,
    copper: &BoardConnectivity,
    layer: LayerId,
    width: Coord,
    net: Option<&str>,
    points: &[Point],
) -> Routed {
    lay(project, copper, layer, width, net, points, false)
}

/// Can the router push this copper aside (a straight, unlocked track; an unlocked via)?
fn movable(project: &Project, c: &Copper) -> bool {
    match c.item {
        CopperItem::Track(id) => project.board.tracks.get(id).is_some_and(|t| !t.locked && t.mid.is_none()),
        CopperItem::Via(id) => project.board.vias.get(id).is_some_and(|v| !v.locked),
        _ => false,
    }
}

/// [`walkaround`], round everything or (`fixed_only`) only what can't be pushed aside.
fn lay(
    project: &Project,
    copper: &BoardConnectivity,
    layer: LayerId,
    width: Coord,
    net: Option<&str>,
    points: &[Point],
    fixed_only: bool,
) -> Routed {
    let (Some(&start), Some(&target)) = (points.first(), points.last()) else {
        return Routed { points: Vec::new(), reached: false, blocked: None, shoved: Default::default() };
    };
    let ends = [(layer, start), (layer, target)];
    let skip = |c: &Copper| fixed_only && movable(project, c);
    let obstacles = Obstacles::new(project, copper, layer, width, net, &ends, &skip);

    // Hulls of what's near the leg; more if the walk goes further.
    let mut region = bounds(points).grow(DETOUR);
    let mut walks = Vec::new();
    for round in 0..3 {
        let hulls = obstacles.hulls(region);
        walks = [true, false].map(|ccw| walk::walk(points, &hulls, ccw)).to_vec();
        let reach = walks.iter().map(|w| bounds(&w.path)).fold(bounds(points), Rect::union);
        let needed = reach.grow(obstacles.reach);
        if round == 2 || (region.contains(needed.min) && region.contains(needed.max)) {
            break;
        }
        region = region.union(needed.grow(DETOUR));
    }

    // Each way, checked exactly (hulls are a little larger than the copper, and the board
    // edge isn't one); the one getting furthest wins, then the shorter.
    let mut best: Option<(Vec<Point>, bool)> = None;
    for w in walks {
        // Without the outline's steps where nothing needs them.
        let clear = &mut |way: &[Point]| check(project, copper, layer, width, net, start, way, &skip).is_none();
        let path = smooth::smooth(w.path, clear);
        let (path, ok) = checked(project, copper, layer, width, net, start, path, &skip);
        let reached = w.reached && ok;
        let better = match &best {
            None => true,
            Some((b, b_reached)) => {
                let gap = |p: &[Point]| (*p.last().expect("a start") - target).length();
                match (reached, *b_reached) {
                    (true, false) => true,
                    (false, true) => false,
                    (true, true) => walk::length(&path) < walk::length(b),
                    (false, false) => gap(&path) < gap(b),
                }
            }
        };
        if better {
            best = Some((path, reached));
        }
    }
    let (points_out, reached) = best.expect("two walks");
    let blocked = (!reached).then(|| check(project, copper, layer, width, net, start, points, &skip)).flatten();
    Routed { points: points_out, reached, blocked, shoved: Default::default() }
}

/// The longest a shove may take, stopping short included.
const SHOVE_TIME: std::time::Duration = std::time::Duration::from_millis(40);

/// How far round an obstacle a walk is expected to go (nm): hulls are gathered this far
/// around the leg at first.
const DETOUR: Coord = 5_000_000;

/// The copper (and keepouts) a leg must keep clear of.
struct Obstacles<'a> {
    skip: &'a dyn Fn(&Copper) -> bool,
    project: &'a Project,
    copper: &'a BoardConnectivity,
    layer: LayerId,
    half: Coord,
    net: Option<u32>,
    net_clearance: Coord,
    clearances: crate::copper::Clearances,
    /// Islands the leg connects to.
    islands: Vec<u32>,
    /// The furthest a hull reaches beyond its copper.
    reach: Coord,
}

impl<'a> Obstacles<'a> {
    fn new(
        project: &'a Project,
        copper: &'a BoardConnectivity,
        layer: LayerId,
        width: Coord,
        net: Option<&str>,
        ends: &[(LayerId, Point)],
        skip: &'a dyn Fn(&Copper) -> bool,
    ) -> Self {
        let rules = &project.board.rules;
        let clearances = copper.clearances(rules);
        let net_clearance = rules.for_net(net).clearance;
        let net = net.and_then(|n| copper.net_index(n));
        // As in `route_check`: the island at the start, and at the end while the net
        // isn't known (the end decides it).
        let islands = ends
            .iter()
            .enumerate()
            .filter(|(i, _)| *i == 0 || net.is_none())
            .filter_map(|(_, &(l, p))| copper.at(p, l))
            .filter(|c| !matches!(c.item, CopperItem::Zone { .. }))
            .map(|c| c.island)
            .collect();
        let half = width / 2;
        let reach = clearances.max.max(net_clearance) + half + SLACK;
        Obstacles { skip, project, copper, layer, half, net, net_clearance, clearances, islands, reach }
    }

    /// Hulls of everything whose copper is within `region`.
    fn hulls(&self, region: Rect) -> Vec<Hull> {
        let bit = layer_bit(self.layer);
        let mut out = Vec::new();
        for c in self.copper.query(region.grow(self.reach)) {
            if c.layers & bit == 0
                || matches!(c.item, CopperItem::Zone { .. })
                || self.islands.contains(&c.island)
                || (self.net.is_some() && c.net == self.net)
                || (self.skip)(c)
            {
                continue;
            }
            let clearance = self.net_clearance.max(self.clearances.of(c.net));
            out.extend(Hull::around(&c.geo, clearance + self.half + SLACK));
        }
        let board = &self.project.board;
        for k in board.keepouts.iter() {
            if k.no_tracks && k.outline.len() >= 3 && k.on_layer(self.layer) {
                let b = bounds(&k.outline);
                if b.intersects(&region.grow(self.reach)) {
                    out.push(Hull::grown(&k.outline, self.half + SLACK));
                }
            }
        }
        out
    }
}

/// The first rule the track along `path` breaks (`skip`: copper left out).
#[allow(clippy::too_many_arguments)]
fn check(
    project: &Project,
    copper: &BoardConnectivity,
    layer: LayerId,
    width: Coord,
    net: Option<&str>,
    start: Point,
    path: &[Point],
    skip: &dyn Fn(&Copper) -> bool,
) -> Option<Clash> {
    let probes: Vec<Probe> =
        path.windows(2).map(|w| Probe::Track { layer, a: w[0], b: w[1], mid: None, width }).collect();
    let end = *path.last()?;
    route_check::check_with(project, copper, &probes, net, &[(layer, start), (layer, end)], skip)
}

/// `path`, or the longest start of it that breaks no rule (and whether it's all of it).
#[allow(clippy::too_many_arguments)]
fn checked(
    project: &Project,
    copper: &BoardConnectivity,
    layer: LayerId,
    width: Coord,
    net: Option<&str>,
    start: Point,
    path: Vec<Point>,
    skip: &dyn Fn(&Copper) -> bool,
) -> (Vec<Point>, bool) {
    if check(project, copper, layer, width, net, start, &path, skip).is_none() {
        return (path, true);
    }
    // Binary search on the length, to a micrometre.
    let (mut lo, mut hi) = (0.0, walk::length(&path));
    while hi - lo > 1000.0 {
        let mid = (lo + hi) / 2.0;
        if check(project, copper, layer, width, net, start, &prefix(&path, mid), skip).is_none() {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    (prefix(&path, lo), false)
}

/// The first `len` nm of `path`.
fn prefix(path: &[Point], len: f64) -> Vec<Point> {
    let mut out = vec![path[0]];
    let mut left = len;
    for w in path.windows(2) {
        let l = (w[1] - w[0]).length();
        if l >= left {
            if l > 0.0 && left > 0.0 {
                out.push(Hull::at(w[0], w[1], left / l));
            }
            break;
        }
        left -= l;
        out.push(w[1]);
    }
    walk::simplify(out)
}

fn bounds(points: &[Point]) -> Rect {
    let first = points.first().copied().unwrap_or(Point::ZERO);
    points.iter().fold(Rect::around(first, 0), |r, &p| r.union(Rect::around(p, 0)))
}
