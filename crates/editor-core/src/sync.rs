//! Schematic → board sync, run inside every command's transaction so undo reverts both.
//!
//! The board only stores *placements* of components. Which footprint a placement shows
//! is derived from the component's part variant, so changing the variant needs no board
//! edit. This module only creates and removes placements: components with a variant get
//! one, components without (no footprint) and deleted ones lose theirs. Copper at their
//! pads stays.

use model::*;

use crate::CommandError;
use crate::transaction::Transaction;

/// Gap between the board and the staging area for new footprints.
const STAGING_GAP: Coord = 10 * MM;
const STAGING_PITCH: Coord = 5 * MM;

pub(crate) fn sync_board(tx: &mut Transaction) -> Result<(), CommandError> {
    let touched: Vec<Id> =
        tx.changes().iter().filter_map(|c| if let ObjectId::Component(id) = c.id { Some(id) } else { None }).collect();
    if touched.is_empty() {
        return Ok(());
    }
    let mut new_components = Vec::new();
    for id in touched {
        // On the board: components with a footprint variant.
        let on_board = tx.project().schematic.components.get(id).is_some_and(|c| c.variant.is_some());
        let placements: Vec<Id> =
            tx.project().board.footprints.iter().filter(|f| f.component == id).map(|f| f.id).collect();
        if on_board && placements.is_empty() && !new_components.contains(&id) {
            new_components.push(id);
        }
        if !on_board {
            for f in placements {
                tx.remove(ObjectId::Footprint(f))?;
            }
        }
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
