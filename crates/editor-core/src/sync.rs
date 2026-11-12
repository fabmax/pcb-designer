//! Schematic → board sync, run inside every command's transaction so undo reverts both.
//!
//! The board only stores *placements* of components. Which footprint a placement shows
//! is derived from the component's part variant, so changing between variants with a
//! footprint needs no board edit. This module only creates and removes placements: a
//! component is on the board exactly when its variant has a footprint. Changed
//! components and components of changed parts are checked; copper at removed
//! placements stays.

use std::collections::{HashMap, HashSet};

use model::*;

use crate::CommandError;
use crate::transaction::Transaction;

/// Gap between the board and the staging area for new footprints.
const STAGING_GAP: Coord = 10 * MM;
const STAGING_PITCH: Coord = 5 * MM;

pub(crate) fn sync_board(tx: &mut Transaction) -> Result<(), CommandError> {
    let mut touched: Vec<Id> = Vec::new();
    let mut parts: HashSet<Id> = HashSet::new();
    for c in tx.changes() {
        match c.id {
            ObjectId::Component(id) => touched.push(id),
            ObjectId::LibPart(id) => drop(parts.insert(id)),
            _ => {}
        }
    }
    let p = tx.project();
    if !parts.is_empty() {
        touched.extend(p.schematic.components.iter().filter(|c| parts.contains(&c.part)).map(|c| c.id));
    }
    if touched.is_empty() {
        return Ok(());
    }
    let mut placements: HashMap<Id, Vec<Id>> = HashMap::new();
    for f in p.board.footprints.iter() {
        placements.entry(f.component).or_default().push(f.id);
    }
    let mut new_components = Vec::new();
    let mut removed = Vec::new();
    let mut seen = HashSet::new();
    for id in touched {
        if !seen.insert(id) {
            continue;
        }
        let variant = p.schematic.components.get(id).and_then(|c| p.library.parts.get(c.part)?.variant(c.variant));
        let on_board = variant.is_some_and(|v| v.footprint.is_some());
        match placements.remove(&id) {
            None if on_board => new_components.push(id),
            Some(fs) if !on_board => removed.extend(fs),
            _ => {}
        }
    }
    for f in removed {
        tx.remove(ObjectId::Footprint(f))?;
    }
    if new_components.is_empty() {
        return Ok(());
    }
    let (x, mut y) = staging_origin(tx.project());
    for component in new_components {
        let fp = FootprintInstance {
            id: Id::new(),
            component,
            pos: Point::new(x, y),
            rotation: Angle::ZERO,
            side: Side::Top,
            locked: false,
        };
        tx.insert(Object::Footprint(fp))?;
        y -= STAGING_PITCH;
    }
    Ok(())
}

/// New footprints are stacked in a column to the right of the board outline.
fn staging_origin(project: &Project) -> (Coord, Coord) {
    let board = &project.board;
    let outline =
        Rect::union_all(board.graphics.iter().filter(|g| g.layer == LayerId::OUTLINE).map(|g| g.shape.bounds()));
    let (right, top) = outline.map_or((0, 0), |r| (r.max.x, r.max.y));
    let x = right + STAGING_GAP;
    let lowest_staged = board.footprints.iter().filter(|f| f.pos.x >= x).map(|f| f.pos.y).min();
    let y = lowest_staged.map_or(top, |y| y - STAGING_PITCH);
    (x, y)
}
