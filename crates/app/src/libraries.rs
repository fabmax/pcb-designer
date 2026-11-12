//! The loaded libraries: the built-in one, the project's library files and unsaved
//! ones. Only some have a tab. The project's own library is not in here.

use std::path::PathBuf;

use view::LibraryDoc;
use view::project_libs::{self, MissingLibrary};

use crate::library::LibraryEditor;

/// Loaded libraries, kept in line with the project's library list by [`Libraries::sync`].
/// Derefs to the editors, in load order.
#[derive(Default)]
pub struct Libraries {
    list: Vec<LibraryEditor>,
    /// The project's library files that could not be loaded.
    pub missing: Vec<MissingLibrary>,
    /// The project library list `list` was last synced with (`None`: sync again).
    synced: Option<Vec<PathBuf>>,
    next_key: u64,
}

impl Libraries {
    pub fn by_key(&self, key: u64) -> Option<&LibraryEditor> {
        self.list.iter().find(|l| l.key == key)
    }

    pub fn by_key_mut(&mut self, key: u64) -> Option<&mut LibraryEditor> {
        self.list.iter_mut().find(|l| l.key == key)
    }

    /// Keep a library loaded. Returns its key.
    pub fn add(&mut self, lib: LibraryDoc) -> u64 {
        let key = self.next_key;
        self.next_key += 1;
        self.list.push(LibraryEditor::new(key, lib));
        key
    }

    /// Unload a library (unsaved changes are lost).
    pub fn remove(&mut self, key: u64) {
        self.list.retain(|l| l.key != key);
    }

    /// Sync again on the next [`Libraries::sync`] (a library file was saved or dropped).
    pub fn resync(&mut self) {
        self.synced = None;
    }

    /// Load and unload libraries to match the project's library files `wanted`. Returns
    /// the libraries it unloaded (their tabs should close).
    pub fn sync(&mut self, wanted: &[PathBuf]) -> Vec<LibraryEditor> {
        if self.synced.as_deref() == Some(wanted) {
            return Vec::new();
        }
        let mut next = self.next_key;
        let dropped = project_libs::sync(wanted, &mut self.list, &mut self.missing, |doc| {
            next += 1;
            LibraryEditor::new(next - 1, doc)
        });
        self.next_key = next;
        self.synced = Some(wanted.to_vec());
        dropped
    }
}

impl std::ops::Deref for Libraries {
    type Target = [LibraryEditor];

    fn deref(&self) -> &[LibraryEditor] {
        &self.list
    }
}

impl std::ops::DerefMut for Libraries {
    fn deref_mut(&mut self) -> &mut [LibraryEditor] {
        &mut self.list
    }
}
