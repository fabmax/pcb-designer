//! Keeping a host's loaded libraries in line with the project's list of library files
//! (`Project::libraries`, the External Libraries).
//!
//! Hosts keep their libraries in a `Vec<T>` (the app wraps each [`LibraryDoc`] in an
//! editor, the headless host keeps the docs). After every project change (including undo
//! and redo of `AddLibrary` / `RemoveLibrary`) they call [`sync`], which
//!
//! * loads listed files that aren't loaded yet; files that fail to load are reported as
//!   [`MissingLibrary`] (the project still opens);
//! * drops loaded files that are no longer listed, unless they have unsaved changes (those
//!   stay until the user saves or closes them, so an undo never loses library edits);
//! * keeps libraries without a file (the built-in library).

use std::path::{Path, PathBuf};

use editor_core::Command;
use model::Library;

use crate::document::Document;
use crate::library_doc::LibraryDoc;

/// A listed library file that could not be loaded.
#[derive(Clone, Debug, PartialEq)]
pub struct MissingLibrary {
    pub path: PathBuf,
    pub error: String,
}

/// Access to the [`LibraryDoc`] inside a host's library entry.
pub trait HasLibrary {
    fn library_doc(&self) -> &LibraryDoc;
}

impl HasLibrary for LibraryDoc {
    fn library_doc(&self) -> &LibraryDoc {
        self
    }
}

/// An absolute path without `.` / `..`, as the project keeps library paths.
pub fn absolute(path: &Path) -> PathBuf {
    model::io::normalize(&std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf()))
}

/// Bring `loaded` in line with `wanted` (see the module docs). `make` wraps a newly
/// loaded library. Returns the dropped entries (the app closes their tabs).
pub fn sync<T: HasLibrary>(
    wanted: &[PathBuf],
    loaded: &mut Vec<T>,
    missing: &mut Vec<MissingLibrary>,
    mut make: impl FnMut(LibraryDoc) -> T,
) -> Vec<T> {
    let mut dropped = Vec::new();
    let mut i = 0;
    while i < loaded.len() {
        let doc = loaded[i].library_doc();
        let keep = match &doc.path {
            Some(p) => wanted.contains(&absolute(p)) || doc.is_modified(),
            None => true,
        };
        if keep {
            i += 1;
        } else {
            dropped.push(loaded.remove(i));
        }
    }
    missing.retain(|m| wanted.contains(&m.path));
    for path in wanted {
        let is_loaded = loaded.iter().any(|l| l.library_doc().path.as_deref().is_some_and(|p| absolute(p) == *path));
        if is_loaded || missing.iter().any(|m| m.path == *path) {
            continue;
        }
        match LibraryDoc::open(path) {
            Ok(doc) => loaded.push(make(doc)),
            Err(e) => missing.push(MissingLibrary { path: path.clone(), error: e.to_string() }),
        }
    }
    dropped
}

/// Check that `lib` (opened or created, and saved) may be added to a project that lists
/// `listed` and has `loaded` libraries: it has a file, isn't listed yet, and no loaded
/// library has its id (a copied library file keeps the id). Returns the path to add.
pub fn check_new(listed: &[PathBuf], loaded: &[&LibraryDoc], lib: &LibraryDoc) -> Result<PathBuf, String> {
    let path = lib.path.as_ref().ok_or("a library must be saved to a file before it can be added")?;
    let path = absolute(path);
    if listed.contains(&path) {
        return Err(format!("{} is already in the project", path.display()));
    }
    let id = lib.library().id;
    if let Some(other) = loaded.iter().find(|l| id.is_some() && l.library().id == id && l.path.as_ref() != Some(&path))
    {
        let r#where = other.path.as_ref().map_or("not saved".to_string(), |p| p.display().to_string());
        return Err(format!(
            "library \"{}\" is already in the project ({where}); a copy of a library file keeps its id",
            other.name()
        ));
    }
    Ok(path)
}

/// [`check_new`], then add the library with `AddLibrary`. The caller keeps `lib` in its
/// list (so [`sync`] doesn't load the file a second time).
pub fn add_to_project(doc: &mut Document, loaded: &[&LibraryDoc], lib: &LibraryDoc) -> Result<(), String> {
    let path = check_new(&doc.project().libraries, loaded, lib)?;
    doc.execute(Command::AddLibrary { path }).map(drop).map_err(|e| e.to_string())
}

/// The project command that follows saving a library that was at `old` to `new`: a listed
/// library saved under a new name is relinked; a library that wasn't listed (never saved
/// before) is added. `None` if the project already lists `new`.
pub fn after_save(listed: &[PathBuf], old: Option<&Path>, new: &Path) -> Option<Command> {
    let new = absolute(new);
    let old = old.map(absolute).filter(|o| listed.contains(o));
    match old {
        Some(from) if from != new => Some(Command::RelinkLibrary { from, to: new }),
        Some(_) => None,
        None if listed.contains(&new) => None,
        None => Some(Command::AddLibrary { path: new }),
    }
}

/// A copy of `lib` to save as a new library file: a new id (so it is a library of its own,
/// e.g. made from the built-in one), named after the file.
pub fn copy_for(lib: &Library, path: &Path) -> LibraryDoc {
    let mut copy = lib.clone();
    copy.id = Some(model::Id::new());
    if let Some(stem) = path.file_stem() {
        copy.name = stem.to_string_lossy().into_owned();
    }
    LibraryDoc::new(copy)
}

#[cfg(test)]
mod tests {
    use super::*;
    use model::Project;
    use model::io::save_library;

    fn file(dir: &Path, name: &str) -> PathBuf {
        let path = dir.join(format!("{name}.pcblib"));
        save_library(&Library::new(name), &path).unwrap();
        path
    }

    #[test]
    fn sync_loads_drops_and_reports_missing() {
        let tmp = tempfile::tempdir().unwrap();
        let (a, b) = (file(tmp.path(), "a"), file(tmp.path(), "b"));
        let gone = tmp.path().join("gone.pcblib");
        let mut loaded = vec![LibraryDoc::new(model::builtin::starter_library())];
        let mut missing = Vec::new();

        let dropped = sync(&[a.clone(), gone.clone(), b.clone()], &mut loaded, &mut missing, |d| d);
        assert!(dropped.is_empty());
        let names: Vec<&str> = loaded.iter().map(|l| l.name()).collect();
        assert_eq!(names, ["Starter", "a", "b"]);
        assert_eq!(missing.len(), 1);
        assert_eq!(missing[0].path, gone);

        // Nothing changes when nothing changed (missing files aren't retried every frame).
        sync(&[a.clone(), gone.clone(), b.clone()], &mut loaded, &mut missing, |_| unreachable!());

        // Unlisted: dropped, unless modified. The built-in library stays.
        loaded[2].new_symbol("edit").unwrap();
        let dropped = sync(&[], &mut loaded, &mut missing, |d| d);
        assert_eq!(dropped.iter().map(|l| l.name()).collect::<Vec<_>>(), ["a"]);
        assert_eq!(loaded.iter().map(|l| l.name()).collect::<Vec<_>>(), ["Starter", "b"]);
        assert!(missing.is_empty());
    }

    #[test]
    fn adding_refuses_copies_of_a_loaded_library() {
        let tmp = tempfile::tempdir().unwrap();
        let a = file(tmp.path(), "a");
        let copy = tmp.path().join("copy.pcblib");
        std::fs::copy(&a, &copy).unwrap();
        let mut doc = Document::new(Project::new("p"));
        let mut loaded = vec![LibraryDoc::new(model::builtin::starter_library())];

        let lib = LibraryDoc::open(&a).unwrap();
        add_to_project(&mut doc, &loaded.iter().collect::<Vec<_>>(), &lib).unwrap();
        loaded.push(lib);
        assert_eq!(doc.project().libraries, [absolute(&a)]);

        let refs: Vec<&LibraryDoc> = loaded.iter().collect();
        let err = add_to_project(&mut doc, &refs, &LibraryDoc::open(&copy).unwrap()).unwrap_err();
        assert!(err.contains("already in the project"), "{err}");
        let err = add_to_project(&mut doc, &refs, &LibraryDoc::open(&a).unwrap()).unwrap_err();
        assert!(err.contains("already in the project"), "{err}");
        let builtin = LibraryDoc::new(model::builtin::starter_library());
        assert!(add_to_project(&mut doc, &refs, &builtin).is_err(), "the built-in library has no file");
        assert_eq!(doc.project().libraries.len(), 1);

        // A copy of the built-in library is a library of its own.
        let fork = tmp.path().join("Mine.pcblib");
        let mut mine = copy_for(refs[0].library(), &fork);
        mine.save_as(&fork).unwrap();
        add_to_project(&mut doc, &refs, &mine).unwrap();
        assert_eq!(mine.name(), "Mine");
    }

    #[test]
    fn saving_relinks_or_adds() {
        let a = absolute(Path::new("/libs/a.pcblib"));
        let b = absolute(Path::new("/libs/b.pcblib"));
        let listed = [a.clone()];
        assert_eq!(after_save(&listed, Some(&a), &a), None);
        assert_eq!(after_save(&listed, Some(&a), &b), Some(Command::RelinkLibrary { from: a.clone(), to: b.clone() }));
        assert_eq!(after_save(&listed, None, &b), Some(Command::AddLibrary { path: b.clone() }));
        assert_eq!(after_save(&listed, None, &a), None);
        // A library that was dropped from the project (unsaved edits kept it open) is added again.
        assert_eq!(after_save(&[], Some(&a), &a), Some(Command::AddLibrary { path: a }));
    }
}
