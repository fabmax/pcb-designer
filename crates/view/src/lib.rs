//! Everything between the document model and the GUI toolkit:
//!
//! * [`View`]: draws the schematic or the board into a [`render::Scene`] and keeps it up
//!   to date incrementally from [`editor_core::ChangeSet`]s; spatial index for hit testing.
//! * [`Document`]: an [`editor_core::Editor`] plus both views, kept in sync.
//! * [`Interaction`]: selection, hover, box selection, dragging with grid snapping,
//!   rotate/mirror/flip/delete. Independent of egui, so it is unit-testable.

pub mod demo;
pub mod document;
mod draw;
pub mod index;
pub mod interaction;
pub mod style;
pub mod view;

pub use document::Document;
pub use interaction::{Interaction, KeyAction, PointerInput};
pub use view::{View, ViewKind};
