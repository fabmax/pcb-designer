//! Import of KiCad symbol and footprint libraries, and of KiCad projects.
//!
//! Supports the S-expression formats of KiCad 6 to 10: `.kicad_sym` symbol libraries,
//! `.kicad_symdir` directories (one file per symbol, KiCad 10) and `.pretty` footprint
//! directories of `.kicad_mod` files.

pub mod catalog;
pub mod common;
pub mod download;
pub mod footprint;
pub mod import;
pub mod pcb;
pub mod preview;
pub mod project;
pub mod sch;
pub mod sexpr;
pub mod symbol;

pub use catalog::{Catalog, Error, default_paths};
pub use import::{Imported, Request, import};
pub use project::{ProjectImport, Report, import_project};
