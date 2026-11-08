//! Import of KiCad symbol and footprint libraries.
//!
//! Supports the S-expression formats of KiCad 6 to 10: `.kicad_sym` symbol libraries,
//! `.kicad_symdir` directories (one file per symbol, KiCad 10) and `.pretty` footprint
//! directories of `.kicad_mod` files.

pub mod catalog;
pub mod common;
pub mod footprint;
pub mod import;
pub mod sexpr;
pub mod symbol;

pub use catalog::{Catalog, Error, default_paths};
pub use import::{Imported, Request, import};
