//! Shoving: lines in the way of the head are walked round it, on whichever side is
//! shorter and clear of fixed copper, and push the lines in their own way in turn. Pads,
//! vias, locked tracks, arcs, keepouts and the board edge don't move: hitting them fails
//! the shove. Everything is computed from the board as it is (a fresh start every time,
//! so pushed tracks spring back when the head retreats).

use std::collections::HashMap;

use model::*;

use super::SLACK;
use super::hull::Hull;
use super::line::{Line, line_of};
use super::walk;
use crate::copper::{BoardConnectivity, Clearances, CopperItem, Geo, LayerMask, layer_bit};
use crate::route_check::{self, Clash, Probe};

/// Most pushes in one shove.
const MAX_PUSHES: usize = 1000;

/// How much closer than the clearance copper must be to count as a collision (nm): a
/// walked line sits at the clearance plus `SLACK`, give or take rounding.
const TOLERANCE: f64 = (SLACK / 2) as f64;

/// A line pushed aside: its tracks and new corners.
#[derive(Clone, Debug, PartialEq)]
pub struct Pushed {
    pub line: Line,
    pub path: Vec<Point>,
}

/// What a pusher ran into that doesn't move.
#[derive(Clone, Debug, PartialEq)]
pub struct Blocked(pub Clash);

/// The head of a route: the track being added.
pub struct Head<'a> {
    pub layer: LayerId,
    pub width: Coord,
    pub net: Option<&'a str>,
    pub path: &'a [Point],
    /// Give up (fail) after this.
    pub deadline: std::time::Instant,
}

/// Push the lines in the head's way. `Err`: something that can't move is in the way (of
/// the head or of a pushed line).
pub fn shove(project: &Project, copper: &BoardConnectivity, head: &Head) -> Result<Vec<Pushed>, Blocked> {
    let mut e = Engine::new(project, copper, head);
    e.run()?;
    let mut out: Vec<Pushed> = e
        .moved
        .into_iter()
        .map(|(i, path)| Pushed { line: e.lines[i].clone(), path })
        .filter(|p| p.path != p.line.path)
        .collect();
    out.sort_by_key(|p| p.line.tracks[0]);
    Ok(out)
}

/// Something pushing: the head or a pushed line.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Pusher {
    Head,
    Line(usize),
}

struct Engine<'a> {
    project: &'a Project,
    copper: &'a BoardConnectivity,
    bit: LayerMask,
    clearances: Clearances,
    head: Vec<Point>,
    head_half: Coord,
    head_net: Option<u32>,
    /// The clearance of the head's net class (known by name before the net has copper).
    head_clearance: Coord,
    /// Islands the head connects to (its own copper while the net isn't known).
    head_islands: Vec<u32>,
    lines: Vec<Line>,
    line_of: HashMap<Id, usize>,
    /// Lines pushed so far: their current corners.
    moved: HashMap<usize, Vec<Point>>,
    deadline: std::time::Instant,
    /// Bounding boxes of the pushed lines' corners.
    moved_bounds: HashMap<usize, Rect>,
    /// What last pushed each line (a line never pushes its own pushers back).
    pushed_by: HashMap<usize, Pusher>,
}

impl<'a> Engine<'a> {
    fn new(project: &'a Project, copper: &'a BoardConnectivity, head: &Head) -> Self {
        let rules = &project.board.rules;
        let head_net = head.net.and_then(|n| copper.net_index(n));
        let ends = [head.path.first(), head.path.last()];
        let head_islands = ends
            .iter()
            .enumerate()
            .filter(|(i, _)| *i == 0 || head_net.is_none())
            .filter_map(|(_, p)| copper.at(*(*p)?, head.layer))
            .filter(|c| !matches!(c.item, CopperItem::Zone { .. }))
            .map(|c| c.island)
            .collect();
        Engine {
            project,
            copper,
            bit: layer_bit(head.layer),
            clearances: copper.clearances(rules),
            head: head.path.to_vec(),
            head_half: head.width / 2,
            head_net,
            head_clearance: rules.for_net(head.net).clearance,
            head_islands,
            lines: Vec::new(),
            line_of: HashMap::new(),
            moved: HashMap::new(),
            pushed_by: HashMap::new(),
            moved_bounds: HashMap::new(),
            deadline: head.deadline,
        }
    }

    fn run(&mut self) -> Result<(), Blocked> {
        let mut stack = vec![Pusher::Head];
        let mut pushes = 0;
        while let Some(p) = stack.pop() {
            pushes += 1;
            if std::time::Instant::now() > self.deadline {
                let at = *self.head.last().expect("a head");
                return Err(Blocked(Clash { at, message: "too much to push aside (time limit)".into() }));
            }
            if pushes > MAX_PUSHES {
                let at = *self.head.last().expect("a head");
                return Err(Blocked(Clash { at, message: "too much to push aside".into() }));
            }
            // The nearest line in the way first; its own pushes settle before the pusher
            // is looked at again.
            let (path, half, clearance, net) = self.geometry(p);
            let Some(&l) = self.hits(&path, half, clearance, net, p)?.first() else { continue };
            if self.pushes_back(p, l) {
                let at = self.lines[l].path[0];
                return Err(Blocked(Clash { at, message: "tracks pushed into each other".into() }));
            }
            let new = self.push(l, &path, half, clearance)?;
            let bounds = new.iter().fold(Rect::around(new[0], 0), |r, &q| r.union(Rect::around(q, 0)));
            self.moved_bounds.insert(l, bounds.grow(self.lines[l].width / 2));
            self.moved.insert(l, new);
            self.pushed_by.insert(l, p);
            stack.push(p);
            stack.push(Pusher::Line(l));
        }
        Ok(())
    }

    /// Would `p` push `l`, one of the lines that pushed it (directly or not)?
    fn pushes_back(&self, p: Pusher, l: usize) -> bool {
        let mut at = p;
        for _ in 0..=self.lines.len() {
            match at {
                Pusher::Head => return false,
                Pusher::Line(i) if i == l => return true,
                Pusher::Line(i) => match self.pushed_by.get(&i) {
                    Some(&q) => at = q,
                    None => return false,
                },
            }
        }
        true
    }

    /// Corners, half width, own clearance and net of a pusher.
    fn geometry(&self, p: Pusher) -> (Vec<Point>, Coord, Coord, Option<u32>) {
        match p {
            Pusher::Head => (self.head.clone(), self.head_half, self.head_clearance, self.head_net),
            Pusher::Line(i) => {
                let l = &self.lines[i];
                let path = self.moved.get(&i).cloned().unwrap_or_else(|| l.path.clone());
                (path, l.width / 2, self.clearances.of(l.net), l.net)
            }
        }
    }

    /// The line a track belongs to (assembled on first use).
    fn line(&mut self, track: Id) -> Option<usize> {
        if let Some(&i) = self.line_of.get(&track) {
            return Some(i);
        }
        let l = line_of(self.project, self.copper, track)?;
        let i = self.lines.len();
        for &t in &l.tracks {
            self.line_of.insert(t, i);
        }
        self.lines.push(l);
        Some(i)
    }

    /// Is copper of `other` the same net as a pusher of `net` (never in the way)?
    fn same_net(&self, net: Option<u32>, pusher: Pusher, other_net: Option<u32>, island: u32) -> bool {
        (net.is_some() && other_net == net) || (pusher == Pusher::Head && self.head_islands.contains(&island))
    }

    /// The movable lines the track along `path` (of a pusher) comes too close to, nearest
    /// first, or the first fixed thing it does.
    fn hits(
        &mut self,
        path: &[Point],
        half: Coord,
        clearance: Coord,
        net: Option<u32>,
        pusher: Pusher,
    ) -> Result<Vec<usize>, Blocked> {
        let me = match pusher {
            Pusher::Line(i) => Some(i),
            Pusher::Head => None,
        };
        let mut out: Vec<(f64, usize)> = Vec::new();
        let hit = |out: &mut Vec<(f64, usize)>, d: f64, l: usize| match out.iter_mut().find(|h| h.1 == l) {
            Some(h) => h.0 = h.0.min(d),
            None => out.push((d, l)),
        };
        let reach = clearance.max(self.clearances.max) + half;
        for w in path.windows(2) {
            let seg = Geo::Capsule { a: w[0], b: w[1], r: half };
            let area = Rect::from_points(w[0], w[1]).grow(reach + self.clearances.max);
            let near: Vec<_> = self
                .copper
                .query(area)
                .filter(|c| c.layers & self.bit != 0 && !matches!(c.item, CopperItem::Zone { .. }))
                .filter(|c| !self.same_net(net, pusher, c.net, c.island))
                .map(|c| (c.item, c.net, c.geo.clone()))
                .collect();
            for (item, other_net, geo) in near {
                let need = clearance.max(self.clearances.of(other_net));
                if let CopperItem::Track(id) = item {
                    let Some(l) = self.line(id) else { continue };
                    if Some(l) == me || self.moved.contains_key(&l) {
                        continue; // checked at its new place below
                    }
                    let d = geo.distance(&seg);
                    if d < need as f64 - TOLERANCE {
                        if self.lines[l].fixed {
                            return Err(self.blocked(&seg, &geo, "a locked track or an arc"));
                        }
                        hit(&mut out, d - need as f64, l);
                    }
                } else if geo.distance(&seg) < need as f64 - TOLERANCE {
                    let what = if matches!(item, CopperItem::Via(_)) { "a via" } else { "a pad" };
                    return Err(self.blocked(&seg, &geo, what));
                }
            }
            // Lines pushed already, where they are now.
            for (&l, moved) in &self.moved {
                if !self.moved_bounds[&l].intersects(&area) {
                    continue;
                }
                let line = &self.lines[l];
                if Some(l) == me || (net.is_some() && line.net == net) {
                    continue;
                }
                let need = clearance.max(self.clearances.of(line.net));
                let r = line.width / 2;
                let d = moved
                    .windows(2)
                    .map(|m| Geo::Capsule { a: m[0], b: m[1], r }.distance(&seg))
                    .fold(f64::MAX, f64::min);
                if d < need as f64 - TOLERANCE {
                    hit(&mut out, d - need as f64, l);
                }
            }
            // A pushed line never goes over the head.
            if pusher != Pusher::Head && !(net.is_some() && net == self.head_net) {
                let need = clearance.max(self.head_clearance);
                for h in self.head.windows(2) {
                    let geo = Geo::Capsule { a: h[0], b: h[1], r: self.head_half };
                    if geo.distance(&seg) < need as f64 - TOLERANCE {
                        return Err(self.blocked(&seg, &geo, "the new track"));
                    }
                }
            }
        }
        out.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
        Ok(out.into_iter().map(|h| h.1).collect())
    }

    fn blocked(&self, seg: &Geo, other: &Geo, what: &str) -> Blocked {
        let at = match other {
            Geo::Capsule { a, b, .. } => Point::new((a.x + b.x) / 2, (a.y + b.y) / 2),
            _ => other.bounds().center(),
        };
        let at = if seg.distance(other) <= 0.0 { at } else { other.bounds().center() };
        Blocked(Clash { at, message: format!("{what} is in the way") })
    }

    /// Line `l` walked round the pusher along `path` (`half` wide, `clearance` its own),
    /// on the shorter side that is clear of fixed copper, keepouts and the board edge.
    fn push(&mut self, l: usize, path: &[Point], half: Coord, clearance: Coord) -> Result<Vec<Point>, Blocked> {
        let line = self.lines[l].clone();
        let current = self.moved.get(&l).cloned().unwrap_or_else(|| line.path.clone());
        let lhalf = line.width / 2;
        let grow = clearance.max(self.clearances.of(line.net)) + lhalf + SLACK;
        let hulls: Vec<Hull> =
            path.windows(2).flat_map(|w| Hull::around(&Geo::Capsule { a: w[0], b: w[1], r: half }, grow)).collect();
        let mut ways: Vec<Vec<Point>> = [true, false]
            .into_iter()
            .map(|ccw| walk::walk(&current, &hulls, ccw))
            .filter(|w| w.reached)
            .map(|w| w.path)
            .collect();
        ways.sort_by(|a, b| walk::length(a).total_cmp(&walk::length(b)));
        let mut why = None;
        for way in ways {
            let clear = self.hits(&way, lhalf, self.clearances.of(line.net), line.net, Pusher::Line(l)).map(drop);
            let clear = clear.and_then(|_| self.board_limits(&line, &way));
            match clear {
                Ok(()) => return Ok(way),
                Err(b) => why = why.or(Some(b)),
            }
        }
        let at = line.path[line.path.len() / 2];
        Err(why.unwrap_or(Blocked(Clash { at, message: "no room to push a track aside".into() })))
    }

    /// Keepouts and the board edge, for a pushed line along `path`.
    fn board_limits(&self, line: &Line, path: &[Point]) -> Result<(), Blocked> {
        let probes: Vec<Probe> = path
            .windows(2)
            .map(|w| Probe::Track { layer: line.layer, a: w[0], b: w[1], mid: None, width: line.width })
            .collect();
        match route_check::check_with(self.project, self.copper, &probes, None, &[], &|_| true) {
            Some(c) => Err(Blocked(c)),
            None => Ok(()),
        }
    }
}
