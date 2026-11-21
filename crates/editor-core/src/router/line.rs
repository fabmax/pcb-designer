//! Lines: chains of straight tracks of one width on one layer between joints, the unit
//! the router pushes. A joint is where the chain can't go on: a track end with no or
//! several other track ends on it, a pad or via, an arc, or a change of width. A line's
//! ends stay where they are when it's pushed; only its inner corners move.

use model::*;

use crate::copper::{BoardConnectivity, CopperItem, layer_bit};

#[derive(Clone, Debug, PartialEq)]
pub struct Line {
    /// The tracks in order along the line.
    pub tracks: Vec<Id>,
    pub layer: LayerId,
    pub width: Coord,
    /// Net index (into [`BoardConnectivity::nets`]).
    pub net: Option<u32>,
    /// Its corners, from end to end (one more than tracks).
    pub path: Vec<Point>,
    /// It may not be pushed: a track is locked or an arc.
    pub fixed: bool,
}

/// The line `track` belongs to.
pub fn line_of(project: &Project, copper: &BoardConnectivity, track: Id) -> Option<Line> {
    let board = &project.board;
    let t = board.tracks.get(track)?;
    let net = copper.get(CopperItem::Track(track)).and_then(|c| c.net);
    if t.mid.is_some() {
        return Some(Line {
            tracks: vec![track],
            layer: t.layer,
            width: t.width,
            net,
            path: vec![t.a, t.b],
            fixed: true,
        });
    }
    // Walk from `t` through `from`, the end where the chain goes on.
    let extend = |from: Point| -> Vec<(Id, Point)> {
        let mut out = Vec::new();
        let (mut prev, mut at) = (track, from);
        while let Some((next, far)) = continuation(project, copper, t, prev, at) {
            if next == track || out.iter().any(|&(id, _)| id == next) {
                break; // a loop
            }
            out.push((next, far));
            (prev, at) = (next, far);
        }
        out
    };
    // back: tracks going on from `a`, each with its far end; ahead: the same from `b`.
    let back = extend(t.a);
    let ahead = extend(t.b);
    let tracks: Vec<Id> =
        back.iter().rev().map(|&(id, _)| id).chain([track]).chain(ahead.iter().map(|&(id, _)| id)).collect();
    let path: Vec<Point> =
        back.iter().rev().map(|&(_, p)| p).chain([t.a, t.b]).chain(ahead.iter().map(|&(_, p)| p)).collect();
    let fixed = tracks.iter().any(|id| board.tracks.get(*id).is_some_and(|t| t.locked));
    Some(Line { tracks, layer: t.layer, width: t.width, net, path, fixed })
}

/// The track continuing the chain past `prev` at `at` and its other end, if `at` is no
/// joint.
fn continuation(
    project: &Project,
    copper: &BoardConnectivity,
    like: &Track,
    prev: Id,
    at: Point,
) -> Option<(Id, Point)> {
    let bit = layer_bit(like.layer);
    let mut next = None;
    for c in copper.query(Rect::around(at, 0)) {
        if c.layers & bit == 0 {
            continue;
        }
        match c.item {
            CopperItem::Pad { .. } | CopperItem::Via(_) if c.geo.contains(at) => return None,
            CopperItem::Track(id) if id != prev => {
                let Some(t) = project.board.tracks.get(id) else { continue };
                let far = if t.a == at {
                    t.b
                } else if t.b == at {
                    t.a
                } else {
                    continue;
                };
                if next.is_some() || t.mid.is_some() || t.width != like.width {
                    return None;
                }
                next = Some((id, far));
            }
            _ => {}
        }
    }
    next
}
