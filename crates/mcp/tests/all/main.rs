//! The crate's integration tests, as one binary (each test binary costs link time and a
//! first-launch check on macOS). Add new test files as modules here.

mod end_to_end;
mod gerber_export;
mod kicad_headless;
mod kicad_project;
mod library_tools;
mod stdio;
mod texts;
