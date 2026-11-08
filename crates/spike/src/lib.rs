//! Milestone-0 spike: a provisional model, a synthetic project generator and the live
//! schematic → board sync chain (connectivity → footprints → ratsnest → render scenes).
//! Validates performance before the real `model`/`netlist` crates are designed.

pub mod connectivity;
pub mod draw;
pub mod library;
pub mod model;
pub mod project;
pub mod ratsnest;

pub use project::{Edit, GenStats, Project, SyncReport};
