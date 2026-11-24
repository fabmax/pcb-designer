//! Projects saved by released versions keep loading, and nothing in them is lost: every
//! value of an old file (after its migrations) is still there when the loaded project is
//! saved again. `tests/saved/VERSION/` holds each release's examples, written by
//! `cargo run -p view --example save_examples -- --release VERSION`.
//!
//! A failure here means a format change breaks older files: add a migration
//! (`model::io::MIGRATIONS`, with the kind's `current_version` bumped) instead of
//! changing these files.

use std::path::{Path, PathBuf};

use model::io::{self, FileKind, MIGRATIONS};
use serde_json::Value;

fn saved_projects() -> Vec<PathBuf> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/saved");
    let mut out = Vec::new();
    for version in std::fs::read_dir(&root).unwrap() {
        for project in std::fs::read_dir(version.unwrap().path()).unwrap() {
            out.push(project.unwrap().path());
        }
    }
    out.sort();
    out
}

/// Where `old` has a value `new` lacks or contradicts (objects by key, arrays by index).
fn missing(old: &Value, new: &Value, at: &str) -> Option<String> {
    match (old, new) {
        (Value::Object(o), Value::Object(n)) => o.iter().find_map(|(k, v)| match n.get(k) {
            Some(nv) => missing(v, nv, &format!("{at}.{k}")),
            None => Some(format!("{at}.{k}: {v}")),
        }),
        (Value::Array(o), Value::Array(n)) if o.len() == n.len() => {
            o.iter().zip(n).enumerate().find_map(|(i, (v, nv))| missing(v, nv, &format!("{at}[{i}]")))
        }
        _ if old == new => None,
        _ => Some(format!("{at}: {old} became {new}")),
    }
}

#[test]
fn projects_saved_by_releases_load_without_losing_anything() {
    let projects = saved_projects();
    assert!(!projects.is_empty());
    let out = tempfile::tempdir().unwrap();
    for dir in projects {
        let project = io::load_project(&dir).unwrap_or_else(|e| panic!("{e}"));
        let again = out.path().join(dir.file_name().unwrap());
        io::save_project(&project, &again).unwrap();
        for (file, kind) in [
            (io::PROJECT_FILE, FileKind::Project),
            (io::LIBRARY_FILE, FileKind::Library),
            (io::SCHEMATIC_FILE, FileKind::Schematic),
            (io::BOARD_FILE, FileKind::Board),
        ] {
            let read = |d: &Path| {
                let path = d.join(file);
                io::upgrade(&std::fs::read_to_string(&path).unwrap(), kind, MIGRATIONS, &path).unwrap()
            };
            if let Some(lost) = missing(&read(&dir), &read(&again), "") {
                panic!("{}: {lost}", dir.join(file).display());
            }
        }
    }
}
