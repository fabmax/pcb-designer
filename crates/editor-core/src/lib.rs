//! Editing core: serializable [`Command`]s executed as atomic transactions on a
//! [`model::Project`], with undo/redo and automatic schematic → board sync.
//!
//! Every command records object-level [`Change`]s (before/after snapshots). Undo and redo
//! replay them, and the same change sets tell views what to update.

pub mod bus;
pub mod clip;
pub mod command;
pub mod connectivity;
pub mod copper;
pub mod drc;
pub mod editor;
pub mod fab;
pub mod history;
pub mod query;
mod rubber_band;
mod sync;
pub mod transaction;
pub mod zones;

pub use command::Command;
pub use editor::Editor;
pub use transaction::{Change, ChangeSet};

use model::ObjectId;

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum CommandError {
    #[error("object not found: {0:?}")]
    NotFound(ObjectId),
    #[error("object already exists: {0:?}")]
    Duplicate(ObjectId),
    #[error("invalid command: {0}")]
    Invalid(String),
    #[error("not allowed: {0}")]
    Forbidden(String),
}
