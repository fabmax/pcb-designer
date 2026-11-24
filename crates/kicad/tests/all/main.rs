//! The crate's integration tests, as one binary (each test binary costs link time and a
//! first-launch check on macOS). Add new test files as modules here.

mod corpus;
mod download;
mod library_import;
mod project_export;
mod project_import;
mod sim_import;
