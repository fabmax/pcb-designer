//! The crate's integration tests, as one binary (each test binary costs link time and a
//! first-launch check on macOS). Add new test files as modules here.

mod arc_tracks;
mod clipboard;
mod drag;
mod footprint_fields;
mod frames;
mod gerber;
mod interaction;
mod keepouts;
mod layers;
mod object_json;
mod part_editor;
mod part_preview;
mod part_tools;
mod pcb_tools;
mod print;
mod probe;
mod sch_tools;
mod sheets;
mod sim_overlay;
mod text_edit;
