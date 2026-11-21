//! Shoving: what's in the way of the head is pushed aside. Lines are walked round the
//! pusher, on whichever side is shorter and clear of fixed copper; vias move by the
//! smallest step that clears it, dragging the tracks that end on them (on every layer).
//! Whatever was pushed pushes in turn. Pads, locked tracks and vias, arcs, keepouts and
//! the board edge don't move: hitting them fails the shove. Everything is computed from
//! the board as it is (a fresh start every time, so pushed copper springs back when the
//! head retreats).

use std::collections::HashMap;

use model::*;

use super::SLACK;
use super::hull::Hull;
use super::line::{Line, line_of};
use super::walk;
use crate::command::{Command, TrackPath};
use crate::copper::{BoardConnectivity, Clearances, CopperItem, Geo, LayerMask, layer_bit};
use crate::route_check::{self, Clash, Probe};

/// Most pushes in one shove.
const MAX_PUSHES: usize = 1000;

/// How much closer than the clearance copper must be to count as a collision (nm): a
/// pushed line or via sits at the clearance plus `SLACK`, give or take rounding.
const TOLERANCE: f64 = (SLACK / 2) as f64;

/// A line pushed aside: its tracks and new corners.
#[derive(Clone, Debug, PartialEq)]
pub struct Pushed {
    pub line: Line,
    pub path: Vec<Point>,
}

/// A via pushed aside.
#[derive(Clone, Debug, PartialEq)]
pub struct MovedVia {
    pub id: Id,
    pub to: Point,
}

/// Everything a shove moves.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Shoved {
    pub lines: Vec<Pushed>,
    pub vias: Vec<MovedVia>,
}

impl Shoved {
    pub fn is_empty(&self) -> bool {
        self.lines.is_empty() && self.vias.is_empty()
    }

    /// The commands that apply it: the lines' new paths and the vias' new places.
    pub fn commands(&self, project: &Project) -> Vec<Command> {
        let mut out = Vec::new();
        let paths: Vec<TrackPath> =
            self.lines.iter().map(|p| TrackPath { tracks: p.line.tracks.clone(), path: p.path.clone() }).collect();
        if !paths.is_empty() {
            out.push(Command::SetTrackPaths { paths });
        }
        let vias: Vec<Object> = self
            .vias
            .iter()
            .filter_map(|m| project.board.vias.get(m.id).map(|v| Object::Via(Via { pos: m.to, ..v.clone() })))
            .collect();
        if !vias.is_empty() {
            out.push(Command::Update { objects: vias });
        }
        out
    }
}

/// What a pusher ran into that doesn't move.
#[derive(Clone, Debug, PartialEq)]
pub struct Blocked(pub Clash);

/// The head of a route: the track being added, and a via at its end.
pub struct Head<'a> {
    pub layer: LayerId,
    pub width: Coord,
    pub net: Option<&'a str>,
    pub path: &'a [Point],
    /// A via at the end of `path` (the L key while routing): its diameter and layers.
    pub via: Option<(Coord, LayerMask)>,
    /// Give up (fail) after this.
    pub deadline: std::time::Instant,
}

/// Push what's in the head's way. `Err`: something that can't move is in the way (of
/// the head or of something pushed).
pub fn shove(project: &Project, copper: &BoardConnectivity, head: &Head) -> Result<Shoved, Blocked> {
    let rules = &project.board.rules;
    let net = head.net.and_then(|n| copper.net_index(n));
    let ends = [head.path.first(), head.path.last()];
    let islands = ends
        .iter()
        .enumerate()
        .filter(|(i, _)| *i == 0 || net.is_none())
        .filter_map(|(_, p)| copper.at(*(*p)?, head.layer))
        .filter(|c| !matches!(c.item, CopperItem::Zone { .. }))
        .map(|c| c.island)
        .collect();
    let (half, bit) = (head.width / 2, layer_bit(head.layer));
    let mut shapes: Shapes = head.path.windows(2).map(|w| (Geo::Capsule { a: w[0], b: w[1], r: half }, bit)).collect();
    if let (Some((d, layers)), Some(&at)) = (head.via, head.path.last()) {
        shapes.push((Geo::Capsule { a: at, b: at, r: d / 2 }, layers));
    }
    let at = head.path.last().copied().unwrap_or(Point::ZERO);
    let pusher = HeadCopper { shapes, net, clearance: rules.for_net(head.net).clearance, islands, at };
    run(project, copper, pusher, head.deadline)
}

/// Push what's in the way of `shapes` (on their layers), the copper of tracks and vias
/// just moved (a drag): of `net`, connected to `islands` (the moved items' islands, so
/// their old copper is no obstacle).
pub fn shove_shapes(
    project: &Project,
    copper: &BoardConnectivity,
    shapes: Vec<(Geo, LayerMask)>,
    net: Option<&str>,
    islands: Vec<u32>,
    deadline: std::time::Instant,
) -> Result<Shoved, Blocked> {
    let at = shapes.first().map_or(Point::ZERO, |s| s.0.bounds().center());
    let clearance = project.board.rules.for_net(net).clearance;
    let net = net.and_then(|n| copper.net_index(n));
    run(project, copper, HeadCopper { shapes, net, clearance, islands, at }, deadline)
}

/// The copper doing the pushing.
struct HeadCopper {
    shapes: Shapes,
    net: Option<u32>,
    /// The clearance of its net class (known by name before the net has copper).
    clearance: Coord,
    /// Islands it connects to (its own copper while the net isn't known).
    islands: Vec<u32>,
    /// Where to report giving up.
    at: Point,
}

fn run(
    project: &Project,
    copper: &BoardConnectivity,
    head: HeadCopper,
    deadline: std::time::Instant,
) -> Result<Shoved, Blocked> {
    let mut e = Engine::new(project, copper, head, deadline);
    e.run()?;
    let mut lines: Vec<Pushed> = e
        .moved
        .iter()
        .map(|(&i, path)| Pushed { line: e.lines[i].clone(), path: path.clone() })
        .filter(|p| p.path != p.line.path)
        .collect();
    lines.sort_by_key(|p| p.line.tracks[0]);
    let mut vias: Vec<MovedVia> = e
        .moved_vias
        .iter()
        .filter(|&(&v, &to)| e.vias[v].pos != to)
        .map(|(&v, &to)| MovedVia { id: e.vias[v].id, to })
        .collect();
    vias.sort_by_key(|m| m.id);
    Ok(Shoved { lines, vias })
}

/// Something pushing or pushed: the head, a line or a via.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Pusher {
    Head,
    Line(usize),
    Via(usize),
}

/// A via as the engine knows it.
#[derive(Clone, Debug)]
struct ViaItem {
    id: Id,
    pos: Point,
    r: Coord,
    layers: LayerMask,
    net: Option<u32>,
    island: u32,
    locked: bool,
}

/// A pusher's copper: shapes with the layers they're on.
type Shapes = Vec<(Geo, LayerMask)>;

struct Engine<'a> {
    project: &'a Project,
    copper: &'a BoardConnectivity,
    clearances: Clearances,
    head: Shapes,
    head_net: Option<u32>,
    head_clearance: Coord,
    head_islands: Vec<u32>,
    head_at: Point,
    lines: Vec<Line>,
    /// Each line's island.
    line_islands: Vec<u32>,
    line_of: HashMap<Id, usize>,
    /// Lines pushed so far: their current corners, and bounding boxes.
    moved: HashMap<usize, Vec<Point>>,
    moved_bounds: HashMap<usize, Rect>,
    vias: Vec<ViaItem>,
    via_of: HashMap<Id, usize>,
    /// Vias pushed so far: where they are now.
    moved_vias: HashMap<usize, Point>,
    /// What last pushed each line or via (nothing pushes its own pushers back).
    pushed_by: HashMap<Pusher, Pusher>,
    deadline: std::time::Instant,
}

impl<'a> Engine<'a> {
    fn new(
        project: &'a Project,
        copper: &'a BoardConnectivity,
        head: HeadCopper,
        deadline: std::time::Instant,
    ) -> Self {
        Engine {
            project,
            copper,
            clearances: copper.clearances(&project.board.rules),
            head: head.shapes,
            head_net: head.net,
            head_clearance: head.clearance,
            head_islands: head.islands,
            head_at: head.at,
            lines: Vec::new(),
            line_islands: Vec::new(),
            line_of: HashMap::new(),
            moved: HashMap::new(),
            moved_bounds: HashMap::new(),
            vias: Vec::new(),
            via_of: HashMap::new(),
            moved_vias: HashMap::new(),
            pushed_by: HashMap::new(),
            deadline,
        }
    }

    fn run(&mut self) -> Result<(), Blocked> {
        let mut stack = vec![Pusher::Head];
        let mut pops = 0;
        while let Some(p) = stack.pop() {
            pops += 1;
            if pops > MAX_PUSHES || std::time::Instant::now() > self.deadline {
                let at = self.head_at;
                return Err(Blocked(Clash { at, message: "too much to push aside".into() }));
            }
            // The nearest thing in the way first; its own pushes settle before the
            // pusher is looked at again.
            let (shapes, clearance, net) = self.shapes(p);
            let Some(&hit) = self.hits(&shapes, clearance, net, p)?.first() else { continue };
            if self.pushes_back(p, hit) {
                let at = shapes.first().map_or(Point::ZERO, |s| s.0.bounds().center());
                return Err(Blocked(Clash { at, message: "copper pushed into itself".into() }));
            }
            stack.push(p);
            match hit {
                Pusher::Line(l) => {
                    let current = self.path(l);
                    let new = self.push_line(l, current, &shapes, clearance)?;
                    self.set_path(l, new);
                    self.pushed_by.insert(hit, p);
                    stack.push(hit);
                }
                Pusher::Via(v) => {
                    let dragged = self.push_via(v, &shapes, clearance)?;
                    self.pushed_by.insert(hit, p);
                    stack.push(hit);
                    for l in dragged {
                        self.pushed_by.insert(Pusher::Line(l), hit);
                        stack.push(Pusher::Line(l));
                    }
                }
                Pusher::Head => unreachable!("the head is never in the way"),
            }
        }
        Ok(())
    }

    /// Does `p` push `target`, one of the things that pushed it (directly or not)?
    fn pushes_back(&self, p: Pusher, target: Pusher) -> bool {
        let mut at = p;
        for _ in 0..=self.lines.len() + self.vias.len() {
            if at == target {
                return true;
            }
            match self.pushed_by.get(&at) {
                Some(&q) => at = q,
                None => return false,
            }
        }
        true
    }

    fn path(&self, l: usize) -> Vec<Point> {
        self.moved.get(&l).cloned().unwrap_or_else(|| self.lines[l].path.clone())
    }

    fn set_path(&mut self, l: usize, path: Vec<Point>) {
        let bounds = path.iter().fold(Rect::around(path[0], 0), |r, &q| r.union(Rect::around(q, 0)));
        self.moved_bounds.insert(l, bounds.grow(self.lines[l].width / 2));
        self.moved.insert(l, path);
    }

    fn via_pos(&self, v: usize) -> Point {
        self.moved_vias.get(&v).copied().unwrap_or(self.vias[v].pos)
    }

    /// A pusher's copper, its own clearance and its net.
    fn shapes(&self, p: Pusher) -> (Shapes, Coord, Option<u32>) {
        match p {
            Pusher::Head => (self.head.clone(), self.head_clearance, self.head_net),
            Pusher::Line(i) => {
                let l = &self.lines[i];
                let (r, bit) = (l.width / 2, layer_bit(l.layer));
                let s = self.path(i).windows(2).map(|w| (Geo::Capsule { a: w[0], b: w[1], r }, bit)).collect();
                (s, self.clearances.of(l.net), l.net)
            }
            Pusher::Via(v) => {
                let via = &self.vias[v];
                let at = self.via_pos(v);
                (vec![(Geo::Capsule { a: at, b: at, r: via.r }, via.layers)], self.clearances.of(via.net), via.net)
            }
        }
    }

    /// Islands a pusher belongs to: copper there is connected to it, never in its way.
    fn islands(&self, p: Pusher) -> Vec<u32> {
        match p {
            Pusher::Head => self.head_islands.clone(),
            Pusher::Line(l) => vec![self.line_islands[l]],
            Pusher::Via(v) => vec![self.vias[v].island],
        }
    }

    /// The line a track belongs to (assembled on first use).
    fn line(&mut self, track: Id) -> Option<usize> {
        if let Some(&i) = self.line_of.get(&track) {
            return Some(i);
        }
        let l = line_of(self.project, self.copper, track)?;
        let island = self.copper.get(CopperItem::Track(track)).map_or(u32::MAX, |c| c.island);
        let i = self.lines.len();
        for &t in &l.tracks {
            self.line_of.insert(t, i);
        }
        self.lines.push(l);
        self.line_islands.push(island);
        Some(i)
    }

    fn via(&mut self, id: Id) -> Option<usize> {
        if let Some(&i) = self.via_of.get(&id) {
            return Some(i);
        }
        let v = self.project.board.vias.get(id)?;
        let c = self.copper.get(CopperItem::Via(id))?;
        let i = self.vias.len();
        self.vias.push(ViaItem {
            id,
            pos: v.pos,
            r: v.diameter / 2,
            layers: c.layers,
            net: c.net,
            island: c.island,
            locked: v.locked,
        });
        self.via_of.insert(id, i);
        Some(i)
    }

    /// What `shapes` (of a pusher of `net`, with `clearance`) come too close to that can
    /// move, nearest first; or the first thing that can't.
    fn hits(
        &mut self,
        shapes: &Shapes,
        clearance: Coord,
        net: Option<u32>,
        pusher: Pusher,
    ) -> Result<Vec<Pusher>, Blocked> {
        let islands = self.islands(pusher);
        let same =
            |other_net: Option<u32>, island: u32| (net.is_some() && other_net == net) || islands.contains(&island);
        let mut out: Vec<(f64, Pusher)> = Vec::new();
        let hit = |out: &mut Vec<(f64, Pusher)>, depth: f64, p: Pusher| match out.iter_mut().find(|h| h.1 == p) {
            Some(h) => h.0 = h.0.min(depth),
            None => out.push((depth, p)),
        };
        let max = self.clearances.max.max(clearance);
        for (seg, bits) in shapes {
            let area = seg.bounds().grow(2 * max + SLACK);
            let near: Vec<_> = self
                .copper
                .query(area)
                .filter(|c| c.layers & bits != 0 && !matches!(c.item, CopperItem::Zone { .. }))
                .filter(|c| !same(c.net, c.island))
                .map(|c| (c.item, c.net, c.geo.clone()))
                .collect();
            for (item, other_net, geo) in near {
                let need = clearance.max(self.clearances.of(other_net)) as f64;
                let d = geo.distance(seg);
                if d >= need - TOLERANCE {
                    continue;
                }
                match item {
                    CopperItem::Track(id) => {
                        let Some(l) = self.line(id) else { continue };
                        if pusher == Pusher::Line(l) || self.moved.contains_key(&l) {
                            continue; // checked where it is now, below
                        }
                        if self.lines[l].fixed {
                            return Err(blocked(seg, &geo, "a locked track or an arc"));
                        }
                        hit(&mut out, d - need, Pusher::Line(l));
                    }
                    CopperItem::Via(id) => {
                        let Some(v) = self.via(id) else { continue };
                        if pusher == Pusher::Via(v) || self.moved_vias.contains_key(&v) {
                            continue;
                        }
                        if self.vias[v].locked {
                            return Err(blocked(seg, &geo, "a locked via"));
                        }
                        hit(&mut out, d - need, Pusher::Via(v));
                    }
                    CopperItem::Pad { .. } => return Err(blocked(seg, &geo, "a pad")),
                    CopperItem::Zone { .. } => {}
                }
            }
            // Lines and vias pushed already, where they are now.
            for (&l, path) in &self.moved {
                let line = &self.lines[l];
                if pusher == Pusher::Line(l)
                    || layer_bit(line.layer) & bits == 0
                    || !self.moved_bounds[&l].intersects(&area)
                    || same(line.net, self.line_islands[l])
                {
                    continue;
                }
                let need = clearance.max(self.clearances.of(line.net)) as f64;
                let r = line.width / 2;
                let d = path
                    .windows(2)
                    .map(|m| Geo::Capsule { a: m[0], b: m[1], r }.distance(seg))
                    .fold(f64::MAX, f64::min);
                if d < need - TOLERANCE {
                    hit(&mut out, d - need, Pusher::Line(l));
                }
            }
            for (&v, &at) in &self.moved_vias {
                let via = &self.vias[v];
                if pusher == Pusher::Via(v) || via.layers & bits == 0 || same(via.net, via.island) {
                    continue;
                }
                let need = clearance.max(self.clearances.of(via.net)) as f64;
                let d = Geo::Capsule { a: at, b: at, r: via.r }.distance(seg);
                if d < need - TOLERANCE {
                    hit(&mut out, d - need, Pusher::Via(v));
                }
            }
            // Nothing pushed goes over the head.
            if pusher != Pusher::Head && !(net.is_some() && net == self.head_net) {
                let need = clearance.max(self.head_clearance) as f64;
                for (h, hbits) in self.shapes(Pusher::Head).0 {
                    if hbits & bits != 0 && h.distance(seg) < need - TOLERANCE {
                        return Err(blocked(seg, &h, "the new track"));
                    }
                }
            }
        }
        out.sort_by(|a, b| a.0.total_cmp(&b.0));
        Ok(out.into_iter().map(|h| h.1).collect())
    }

    /// Line `l` along `current` walked round the pusher's `shapes` on its layer (with the
    /// pusher's own `clearance`), on the shorter side that is clear of fixed copper, the
    /// head, keepouts and the board edge.
    fn push_line(
        &mut self,
        l: usize,
        current: Vec<Point>,
        shapes: &Shapes,
        clearance: Coord,
    ) -> Result<Vec<Point>, Blocked> {
        let line = self.lines[l].clone();
        let (bit, lhalf) = (layer_bit(line.layer), line.width / 2);
        let grow = clearance.max(self.clearances.of(line.net)) + lhalf + SLACK;
        let hulls: Vec<Hull> =
            shapes.iter().filter(|(_, bits)| bits & bit != 0).flat_map(|(g, _)| Hull::around(g, grow)).collect();
        let mut ways: Vec<Vec<Point>> = [true, false]
            .into_iter()
            .map(|ccw| walk::walk(&current, &hulls, ccw))
            .filter(|w| w.reached)
            .map(|w| w.path)
            .collect();
        ways.sort_by(|a, b| walk::length(a).total_cmp(&walk::length(b)));
        ways.dedup();
        let mut why = None;
        for way in ways {
            let shapes: Shapes = way.windows(2).map(|w| (Geo::Capsule { a: w[0], b: w[1], r: lhalf }, bit)).collect();
            let clear = self.hits(&shapes, self.clearances.of(line.net), line.net, Pusher::Line(l)).map(drop);
            let probes: Vec<Probe> = way
                .windows(2)
                .map(|w| Probe::Track { layer: line.layer, a: w[0], b: w[1], mid: None, width: line.width })
                .collect();
            match clear.and_then(|_| self.board_limits(&probes)) {
                Ok(()) => return Ok(way),
                Err(b) => why = why.or(Some(b)),
            }
        }
        let at = line.path[line.path.len() / 2];
        Err(why.unwrap_or(Blocked(Clash { at, message: "no room to push a track aside".into() })))
    }

    /// Via `v` moved the least that clears the pusher's `shapes`, if that's clear of fixed
    /// copper, the head, keepouts and the board edge. The lines ending on it come along
    /// (and go round the pusher); returns them.
    fn push_via(&mut self, v: usize, shapes: &Shapes, clearance: Coord) -> Result<Vec<usize>, Blocked> {
        let via = self.vias[v].clone();
        let old = self.via_pos(v);
        let need = clearance.max(self.clearances.of(via.net)) + SLACK;
        let mut at = old;
        for _ in 0..8 {
            let mut moved = false;
            for (g, _) in shapes.iter().filter(|(_, bits)| bits & via.layers != 0) {
                let Geo::Capsule { a, b, r } = *g else { continue };
                let axis = nearest_on_segment(at, a, b);
                let want = (r + via.r + need) as f64;
                let d = (at - axis).length();
                if d >= want - TOLERANCE {
                    continue;
                }
                // Away from the pusher's axis (across it if the via sits on it).
                let dir = if d > 0.0 {
                    ((at.x - axis.x) as f64 / d, (at.y - axis.y) as f64 / d)
                } else {
                    let (dx, dy) = ((b.x - a.x) as f64, (b.y - a.y) as f64);
                    let len = dx.hypot(dy);
                    if len > 0.0 { (-dy / len, dx / len) } else { (1.0, 0.0) }
                };
                at = Point::new(
                    axis.x + (dir.0 * want).round() as Coord + dir.0.signum() as Coord,
                    axis.y + (dir.1 * want).round() as Coord + dir.1.signum() as Coord,
                );
                moved = true;
            }
            if !moved {
                break;
            }
        }
        let shape = vec![(Geo::Capsule { a: at, b: at, r: via.r }, via.layers)];
        self.hits(&shape, self.clearances.of(via.net), via.net, Pusher::Via(v))?;
        self.board_limits(&[Probe::Via { pos: at, diameter: 2 * via.r, layers: via.layers }])?;
        self.moved_vias.insert(v, at);

        // The tracks ending on it, on all its layers.
        let delta = at - old;
        let mut ends: Vec<Id> = Vec::new();
        for c in self.copper.query(Rect::around(via.pos, via.r)) {
            if let CopperItem::Track(id) = c.item
                && c.layers & via.layers != 0
                && let Some(t) = self.project.board.tracks.get(id)
                && ((t.a - via.pos).length() <= via.r as f64 || (t.b - via.pos).length() <= via.r as f64)
            {
                ends.push(id);
            }
        }
        let mut dragged = Vec::new();
        for id in ends {
            let Some(l) = self.line(id) else { continue };
            if dragged.contains(&l) {
                continue;
            }
            if self.lines[l].fixed {
                let at = self.vias[v].pos;
                return Err(Blocked(Clash { at, message: "a locked track ends on a pushed via".into() }));
            }
            let mut path = self.path(l);
            let last = path.len() - 1;
            for i in [0, last] {
                if (path[i] - old).length() <= via.r as f64 {
                    path[i] = path[i] + delta;
                }
            }
            let path = self.push_line(l, path, shapes, clearance)?;
            self.set_path(l, path);
            dragged.push(l);
        }
        Ok(dragged)
    }

    /// Keepouts and the board edge.
    fn board_limits(&self, probes: &[Probe]) -> Result<(), Blocked> {
        match route_check::check_with(self.project, self.copper, probes, None, &[], &|_| true) {
            Some(c) => Err(Blocked(c)),
            None => Ok(()),
        }
    }
}

fn blocked(seg: &Geo, other: &Geo, what: &str) -> Blocked {
    let at = if seg.distance(other) <= 0.0 { seg.bounds().center() } else { other.bounds().center() };
    Blocked(Clash { at, message: format!("{what} is in the way") })
}

/// The point of segment `a`–`b` nearest to `p`.
fn nearest_on_segment(p: Point, a: Point, b: Point) -> Point {
    let (dx, dy) = ((b.x - a.x) as f64, (b.y - a.y) as f64);
    let len2 = dx * dx + dy * dy;
    if len2 == 0.0 {
        return a;
    }
    let t = (((p.x - a.x) as f64 * dx + (p.y - a.y) as f64 * dy) / len2).clamp(0.0, 1.0);
    Point::new(a.x + (dx * t).round() as Coord, a.y + (dy * t).round() as Coord)
}
