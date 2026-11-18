//! The crate's integration tests, as one binary (each test binary costs link time and a
//! first-launch check on macOS). Add new test files as modules here.

mod board;
mod board_parts;
mod buses;
mod cutouts_clearance;
mod drag;
mod drc;
mod editing;
mod erc;
mod fields;
mod footprintless;
mod library_update;
mod net_classes;
mod netlist;
mod no_connect;
mod outline;
mod power_styles;
mod project_libraries;
mod routing;
mod sheets;
mod sheets_clip;
