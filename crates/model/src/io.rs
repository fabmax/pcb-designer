//! Project folders and library files.
//!
//! A project is a folder (conventionally `<name>.pcbproj/`) with one JSON file per part of
//! the document:
//!
//! ```text
//! project.json    name, library files (relative to the folder)
//! library.json    embedded copies of used parts, symbols and footprints
//! schematic.json  schematic
//! board.json      board
//! ```
//!
//! A library is a single `*.pcblib` JSON file. Every file starts with
//! `"format": "pcb-designer/<kind>"` and `"version": N`. Older versions can be upgraded on
//! load by the migrations in [`MIGRATIONS`] (none until production use: until then format
//! changes may break older files). Files are written with stable field order,
//! integer coordinates, and short scalar arrays (like points) kept on one line, so they
//! diff well in git. Writes go through a temporary file and a rename.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Map, Value};

use crate::board::Board;
use crate::library::Library;
use crate::project::Project;
use crate::schematic::Schematic;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileKind {
    Project,
    Library,
    Schematic,
    Board,
}

impl FileKind {
    pub fn format(self) -> &'static str {
        match self {
            FileKind::Project => "pcb-designer/project",
            FileKind::Library => "pcb-designer/library",
            FileKind::Schematic => "pcb-designer/schematic",
            FileKind::Board => "pcb-designer/board",
        }
    }

    /// Version written by this build.
    pub fn current_version(self) -> u32 {
        1
    }
}

pub const PROJECT_FILE: &str = "project.json";
pub const LIBRARY_FILE: &str = "library.json";
pub const SCHEMATIC_FILE: &str = "schematic.json";
pub const BOARD_FILE: &str = "board.json";

/// Upgrades a file of `kind` from version `from` to `from + 1`.
pub struct Migration {
    pub kind: FileKind,
    pub from: u32,
    pub apply: fn(&mut Value) -> Result<(), String>,
}

/// All known migrations, oldest first.
pub const MIGRATIONS: &[Migration] = &[];

#[derive(Debug, thiserror::Error)]
pub enum FileError {
    #[error("{path}: {source}")]
    Io { path: PathBuf, source: std::io::Error },
    #[error("{path}: invalid JSON: {source}")]
    Json { path: PathBuf, source: serde_json::Error },
    #[error("{path}: expected format {expected:?}, found {found:?}")]
    Format { path: PathBuf, expected: &'static str, found: String },
    #[error("{path}: version {found} is newer than supported version {supported}")]
    TooNew { path: PathBuf, found: u32, supported: u32 },
    #[error("{path}: cannot upgrade from version {from}: {reason}")]
    Migration { path: PathBuf, from: u32, reason: String },
}

#[derive(Serialize, serde::Deserialize)]
struct ProjectFile {
    name: String,
    /// Library files, relative to the project folder where possible, with `/` separators.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    libraries: Vec<String>,
}

pub fn save_project(project: &Project, dir: impl AsRef<Path>) -> Result<(), FileError> {
    let dir = dir.as_ref();
    std::fs::create_dir_all(dir).map_err(|e| FileError::Io { path: dir.into(), source: e })?;
    let libraries = project.libraries.iter().map(|p| stored_path(p, dir)).collect();
    write_file(&dir.join(PROJECT_FILE), FileKind::Project, &ProjectFile { name: project.name.clone(), libraries })?;
    write_file(&dir.join(LIBRARY_FILE), FileKind::Library, &project.library)?;
    write_file(&dir.join(SCHEMATIC_FILE), FileKind::Schematic, &project.schematic)?;
    write_file(&dir.join(BOARD_FILE), FileKind::Board, &project.board)?;
    Ok(())
}

pub fn load_project(dir: impl AsRef<Path>) -> Result<Project, FileError> {
    let dir = dir.as_ref();
    let meta: ProjectFile = read_file(&dir.join(PROJECT_FILE), FileKind::Project, MIGRATIONS)?;
    Ok(Project {
        name: meta.name,
        library: read_file(&dir.join(LIBRARY_FILE), FileKind::Library, MIGRATIONS)?,
        schematic: read_file::<Schematic>(&dir.join(SCHEMATIC_FILE), FileKind::Schematic, MIGRATIONS)?,
        board: read_file::<Board>(&dir.join(BOARD_FILE), FileKind::Board, MIGRATIONS)?,
        libraries: meta.libraries.iter().map(|p| normalize(&absolute(dir).join(p))).collect(),
    })
}

/// `path` as written to `project.json`: relative to `dir` with `/` separators, or
/// absolute when there is no relative path (another drive on Windows).
pub fn stored_path(path: &Path, dir: &Path) -> String {
    let path = normalize(&absolute(path));
    let dir = normalize(&absolute(dir));
    match relative_path(&path, &dir) {
        Some(rel) => rel.components().map(|c| c.as_os_str().to_string_lossy()).collect::<Vec<_>>().join("/"),
        None => path.to_string_lossy().into_owned(),
    }
}

/// `path` relative to `base` (both absolute and normalized), if they share a root.
fn relative_path(path: &Path, base: &Path) -> Option<PathBuf> {
    use std::path::Component;
    let mut p = path.components().peekable();
    let mut b = base.components().peekable();
    // Same prefix (drive) and root, else no relative path exists.
    while let (Some(x @ (Component::Prefix(_) | Component::RootDir)), Some(y)) = (p.peek(), b.peek()) {
        if x != y {
            return None;
        }
        p.next();
        b.next();
    }
    while let (Some(x), Some(y)) = (p.peek(), b.peek()) {
        if x != y {
            break;
        }
        p.next();
        b.next();
    }
    let mut out: PathBuf = b.map(|_| Component::ParentDir).collect();
    out.extend(p);
    Some(out)
}

fn absolute(path: &Path) -> PathBuf {
    std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf())
}

/// Resolve `.` and `..` without touching the file system (the file may not exist).
pub fn normalize(path: &Path) -> PathBuf {
    use std::path::Component;
    let mut out = PathBuf::new();
    for c in path.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                if !out.pop() {
                    out.push(c);
                }
            }
            c => out.push(c),
        }
    }
    out
}

pub fn save_library(library: &Library, path: impl AsRef<Path>) -> Result<(), FileError> {
    write_file(path.as_ref(), FileKind::Library, library)
}

pub fn load_library(path: impl AsRef<Path>) -> Result<Library, FileError> {
    read_file(path.as_ref(), FileKind::Library, MIGRATIONS)
}

/// Serialize `data` with the file header into the stable text format.
pub fn to_file_string<T: Serialize>(kind: FileKind, data: &T) -> Result<String, serde_json::Error> {
    let mut map = Map::new();
    map.insert("format".into(), Value::from(kind.format()));
    map.insert("version".into(), Value::from(kind.current_version()));
    match serde_json::to_value(data)? {
        Value::Object(fields) => map.extend(fields),
        other => {
            map.insert("data".into(), other);
        }
    }
    let mut out = String::new();
    write_value(&mut out, &Value::Object(map), 0);
    out.push('\n');
    Ok(out)
}

/// Parse a file's text: check header, apply migrations, deserialize.
pub fn from_file_str<T: DeserializeOwned>(
    text: &str,
    kind: FileKind,
    migrations: &[Migration],
    path: &Path,
) -> Result<T, FileError> {
    let json_err = |e| FileError::Json { path: path.into(), source: e };
    let mut value: Value = serde_json::from_str(text).map_err(json_err)?;
    let obj = value.as_object_mut().ok_or_else(|| FileError::Format {
        path: path.into(),
        expected: kind.format(),
        found: "non-object".into(),
    })?;
    let format = obj.remove("format").and_then(|v| v.as_str().map(String::from)).unwrap_or_default();
    if format != kind.format() {
        return Err(FileError::Format { path: path.into(), expected: kind.format(), found: format });
    }
    let mut version = obj.remove("version").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
    let target = kind.current_version();
    if version > target {
        return Err(FileError::TooNew { path: path.into(), found: version, supported: target });
    }
    while version < target {
        let m = migrations.iter().find(|m| m.kind == kind && m.from == version).ok_or_else(|| {
            FileError::Migration { path: path.into(), from: version, reason: "no migration available".into() }
        })?;
        (m.apply)(&mut value).map_err(|reason| FileError::Migration { path: path.into(), from: version, reason })?;
        version += 1;
    }
    serde_json::from_value(value).map_err(json_err)
}

fn write_file<T: Serialize>(path: &Path, kind: FileKind, data: &T) -> Result<(), FileError> {
    let text = to_file_string(kind, data).map_err(|e| FileError::Json { path: path.into(), source: e })?;
    let tmp = path.with_extension("json.tmp");
    let io_err = |e| FileError::Io { path: path.into(), source: e };
    std::fs::write(&tmp, text).map_err(io_err)?;
    std::fs::rename(&tmp, path).map_err(io_err)
}

fn read_file<T: DeserializeOwned>(path: &Path, kind: FileKind, migrations: &[Migration]) -> Result<T, FileError> {
    let text = std::fs::read_to_string(path).map_err(|e| FileError::Io { path: path.into(), source: e })?;
    from_file_str(&text, kind, migrations, path)
}

/// Pretty printer: objects one field per line, arrays of scalars inline.
fn write_value(out: &mut String, v: &Value, indent: usize) {
    const STEP: usize = 2;
    match v {
        Value::Object(map) if map.is_empty() => out.push_str("{}"),
        Value::Object(map) => {
            out.push_str("{\n");
            for (i, (k, val)) in map.iter().enumerate() {
                pad(out, indent + STEP);
                out.push_str(&Value::String(k.clone()).to_string());
                out.push_str(": ");
                write_value(out, val, indent + STEP);
                if i + 1 < map.len() {
                    out.push(',');
                }
                out.push('\n');
            }
            pad(out, indent);
            out.push('}');
        }
        Value::Array(items) if items.is_empty() => out.push_str("[]"),
        Value::Array(items) if items.iter().all(|i| !i.is_object() && !i.is_array()) => {
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                let _ = write!(out, "{item}");
            }
            out.push(']');
        }
        Value::Array(items) => {
            out.push_str("[\n");
            for (i, item) in items.iter().enumerate() {
                pad(out, indent + STEP);
                write_value(out, item, indent + STEP);
                if i + 1 < items.len() {
                    out.push(',');
                }
                out.push('\n');
            }
            pad(out, indent);
            out.push(']');
        }
        scalar => {
            let _ = write!(out, "{scalar}");
        }
    }
}

fn pad(out: &mut String, n: usize) {
    out.extend(std::iter::repeat_n(' ', n));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::builtin;

    #[test]
    fn library_paths_are_stored_relative_to_the_project() {
        let dir = Path::new("/work/boards/amp.pcbproj");
        assert_eq!(stored_path(Path::new("/work/boards/amp.pcbproj/parts.pcblib"), dir), "parts.pcblib");
        assert_eq!(stored_path(Path::new("/work/libs/mine.pcblib"), dir), "../../libs/mine.pcblib");
        assert_eq!(stored_path(Path::new("/work/libs/./x/../mine.pcblib"), dir), "../../libs/mine.pcblib");
        assert_eq!(normalize(&dir.join("../../libs/mine.pcblib")), Path::new("/work/libs/mine.pcblib"));
    }

    #[test]
    fn project_library_list_round_trips() {
        let tmp = std::env::temp_dir().join(format!("pcbio-{}", crate::Id::new()));
        let dir = tmp.join("boards/amp.pcbproj");
        let mut p = Project::new("amp");
        p.libraries = vec![tmp.join("libs/mine.pcblib"), dir.join("local.pcblib")];
        save_project(&p, &dir).unwrap();
        let text = std::fs::read_to_string(dir.join(PROJECT_FILE)).unwrap();
        assert!(text.contains(r#""libraries": ["../../libs/mine.pcblib", "local.pcblib"]"#), "{text}");
        // Moving the project together with its libraries keeps the references valid.
        let moved = std::env::temp_dir().join(format!("pcbio-{}", crate::Id::new()));
        std::fs::rename(&tmp, &moved).unwrap();
        let loaded = load_project(moved.join("boards/amp.pcbproj")).unwrap();
        assert_eq!(
            loaded.libraries,
            vec![moved.join("libs/mine.pcblib"), moved.join("boards/amp.pcbproj/local.pcblib")]
        );
        std::fs::remove_dir_all(&moved).unwrap();
    }

    #[test]
    fn pretty_printer_keeps_points_inline() {
        let v = serde_json::json!({"a": [1, 2], "b": [{"x": [3, 4]}], "c": {}});
        let mut s = String::new();
        write_value(&mut s, &v, 0);
        assert_eq!(s, "{\n  \"a\": [1, 2],\n  \"b\": [\n    {\n      \"x\": [3, 4]\n    }\n  ],\n  \"c\": {}\n}");
        // Output is valid JSON with the same content.
        assert_eq!(serde_json::from_str::<Value>(&s).unwrap(), v);
    }

    #[test]
    fn library_file_roundtrip_is_stable() {
        let lib = builtin::starter_library();
        let text = to_file_string(FileKind::Library, &lib).unwrap();
        let back: Library = from_file_str(&text, FileKind::Library, MIGRATIONS, Path::new("lib")).unwrap();
        assert_eq!(back, lib);
        // Saving again produces identical bytes.
        assert_eq!(to_file_string(FileKind::Library, &back).unwrap(), text);
    }

    #[test]
    fn rejects_wrong_format_and_newer_version() {
        let p = Path::new("x");
        let wrong = r#"{"format": "pcb-designer/board", "version": 1}"#;
        assert!(matches!(
            from_file_str::<Library>(wrong, FileKind::Library, MIGRATIONS, p),
            Err(FileError::Format { .. })
        ));
        let newer = r#"{"format": "pcb-designer/library", "version": 99}"#;
        assert!(matches!(
            from_file_str::<Library>(newer, FileKind::Library, MIGRATIONS, p),
            Err(FileError::TooNew { .. })
        ));
    }

    #[test]
    fn migrations_run_in_order() {
        // A version-0 library used "title" instead of "name".
        fn rename_title(v: &mut Value) -> Result<(), String> {
            let obj = v.as_object_mut().ok_or("not an object")?;
            let title = obj.remove("title").ok_or("missing title")?;
            obj.insert("name".into(), title);
            Ok(())
        }
        let migrations = [Migration { kind: FileKind::Library, from: 0, apply: rename_title }];
        let old = r#"{"format": "pcb-designer/library", "version": 0, "title": "Old lib"}"#;
        let lib: Library = from_file_str(old, FileKind::Library, &migrations, Path::new("old")).unwrap();
        assert_eq!(lib.name, "Old lib");
        // Without the migration the file cannot be loaded.
        assert!(matches!(
            from_file_str::<Library>(old, FileKind::Library, &[], Path::new("old")),
            Err(FileError::Migration { .. })
        ));
    }
}
