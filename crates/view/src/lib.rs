//! Everything between the document model and the GUI toolkit:
//!
//! * [`View`]: draws the schematic or the board into a [`render::Scene`] and keeps it up
//!   to date incrementally from [`editor_core::ChangeSet`]s; spatial index for hit testing.
//! * [`Document`]: an [`editor_core::Editor`] plus both views, kept in sync.
//! * [`Interaction`]: selection, hover, box selection, dragging with grid snapping,
//!   rotate/mirror/flip/delete. Independent of egui, so it is unit-testable.

pub mod clipboard;
pub mod demo;
pub mod document;
mod draw;
pub mod gerber_view;
pub mod index;
pub mod interaction;
pub mod item;
pub mod library_doc;
pub mod measure;
pub mod pcb_tools;
pub mod probe;
pub mod project_libs;
pub mod sch_tools;
pub mod style;
pub mod tools;
pub mod tree;
pub mod view;

pub use document::{Document, DrcReport, GerberExport, ProjectTarget};
pub use gerber;
pub use gerber_view::GerberView;
pub use interaction::{Edit, Interaction, KeyAction, PointerInput, Target};
pub use item::{Elem, Item, ItemTarget, ItemView};
pub use library_doc::LibraryDoc;
pub use view::{LayerEntry, View, ViewId, ViewKind};
