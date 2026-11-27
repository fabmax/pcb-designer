//! The file system the app reads and writes. On the desktop these are `std::fs`'s
//! functions (and `Path::exists` & co.). In the browser, which has no file system, they
//! work on an in-memory one, [`MemFs`], which the app keeps in the browser's storage.
//!
//! Code that touches files goes through here, `Path::exists` included (it would ask the
//! real file system, which the browser doesn't have).

use std::collections::{BTreeMap, BTreeSet};
use std::io;
use std::path::{Component, Path, PathBuf};

#[cfg(not(target_arch = "wasm32"))]
pub use native::*;
#[cfg(target_arch = "wasm32")]
pub use web::*;

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use std::io;
    use std::path::{Path, PathBuf};

    pub use std::fs::{
        canonicalize, copy, create_dir_all, read, read_to_string, remove_dir_all, remove_file, rename, write,
    };

    pub fn exists(path: impl AsRef<Path>) -> bool {
        path.as_ref().exists()
    }

    pub fn is_file(path: impl AsRef<Path>) -> bool {
        path.as_ref().is_file()
    }

    pub fn is_dir(path: impl AsRef<Path>) -> bool {
        path.as_ref().is_dir()
    }

    /// The paths of a folder's entries (in no particular order).
    pub fn list_dir(path: impl AsRef<Path>) -> io::Result<Vec<PathBuf>> {
        std::fs::read_dir(path)?.map(|e| e.map(|e| e.path())).collect()
    }

    /// When a file was last written, in milliseconds since 1970.
    pub fn modified(path: impl AsRef<Path>) -> Option<u64> {
        let t = std::fs::metadata(path).and_then(|m| m.modified()).ok()?;
        t.duration_since(std::time::UNIX_EPOCH).ok().map(|d| d.as_millis() as u64)
    }

    pub fn temp_dir() -> PathBuf {
        std::env::temp_dir()
    }
}

#[cfg(target_arch = "wasm32")]
mod web {
    use std::cell::RefCell;
    use std::io;
    use std::path::{Path, PathBuf};

    use super::MemFs;

    thread_local! {
        static FS: RefCell<MemFs> = RefCell::new(MemFs::default());
    }

    /// The browser's file system (to load it from storage and save its changes).
    pub fn with_mem<R>(f: impl FnOnce(&mut MemFs) -> R) -> R {
        FS.with(|fs| f(&mut fs.borrow_mut()))
    }

    pub fn read(path: impl AsRef<Path>) -> io::Result<Vec<u8>> {
        with_mem(|fs| fs.read(path.as_ref()))
    }

    pub fn read_to_string(path: impl AsRef<Path>) -> io::Result<String> {
        String::from_utf8(read(path)?).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
    }

    pub fn write(path: impl AsRef<Path>, data: impl AsRef<[u8]>) -> io::Result<()> {
        with_mem(|fs| fs.write(path.as_ref(), data.as_ref().to_vec()))
    }

    pub fn create_dir_all(path: impl AsRef<Path>) -> io::Result<()> {
        with_mem(|fs| fs.create_dir_all(path.as_ref()))
    }

    pub fn remove_file(path: impl AsRef<Path>) -> io::Result<()> {
        with_mem(|fs| fs.remove_file(path.as_ref()))
    }

    pub fn remove_dir_all(path: impl AsRef<Path>) -> io::Result<()> {
        with_mem(|fs| fs.remove_dir_all(path.as_ref()))
    }

    pub fn rename(from: impl AsRef<Path>, to: impl AsRef<Path>) -> io::Result<()> {
        with_mem(|fs| fs.rename(from.as_ref(), to.as_ref()))
    }

    pub fn copy(from: impl AsRef<Path>, to: impl AsRef<Path>) -> io::Result<u64> {
        with_mem(|fs| fs.copy(from.as_ref(), to.as_ref()))
    }

    pub fn canonicalize(path: impl AsRef<Path>) -> io::Result<PathBuf> {
        with_mem(|fs| fs.canonicalize(path.as_ref()))
    }

    pub fn exists(path: impl AsRef<Path>) -> bool {
        with_mem(|fs| fs.exists(path.as_ref()))
    }

    pub fn is_file(path: impl AsRef<Path>) -> bool {
        with_mem(|fs| fs.is_file(path.as_ref()))
    }

    pub fn is_dir(path: impl AsRef<Path>) -> bool {
        with_mem(|fs| fs.is_dir(path.as_ref()))
    }

    pub fn list_dir(path: impl AsRef<Path>) -> io::Result<Vec<PathBuf>> {
        with_mem(|fs| fs.list_dir(path.as_ref()))
    }

    pub fn modified(path: impl AsRef<Path>) -> Option<u64> {
        with_mem(|fs| fs.modified(path.as_ref()))
    }

    pub fn temp_dir() -> PathBuf {
        PathBuf::from("/tmp")
    }
}

/// A file or a folder of a [`MemFs`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Entry {
    Dir,
    /// Its content, and when it was written (milliseconds since 1970).
    File(Vec<u8>, u64),
}

/// A file system in memory, rooted at `/` (the browser's). It behaves like `std::fs`
/// where the app relies on it: writing needs the parent folder, removing a file isn't
/// removing a folder. It remembers which paths changed, for saving them elsewhere.
#[derive(Debug, Default)]
pub struct MemFs {
    entries: BTreeMap<PathBuf, Entry>,
    changed: BTreeSet<PathBuf>,
}

fn not_found(path: &Path) -> io::Error {
    io::Error::new(io::ErrorKind::NotFound, format!("{} not found", path.display()))
}

fn error(kind: io::ErrorKind, path: &Path, what: &str) -> io::Error {
    io::Error::new(kind, format!("{}: {what}", path.display()))
}

/// `path` as a key: absolute (relative to `/`), without `.` and `..`.
fn key(path: &Path) -> PathBuf {
    let mut out = PathBuf::from("/");
    for c in path.components() {
        match c {
            Component::Normal(n) => out.push(n),
            Component::ParentDir => {
                out.pop();
            }
            Component::RootDir | Component::CurDir | Component::Prefix(_) => {}
        }
    }
    out
}

fn now() -> u64 {
    web_time::SystemTime::now().duration_since(web_time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64)
}

impl MemFs {
    fn is_root(path: &Path) -> bool {
        path.parent().is_none()
    }

    fn get(&self, path: &Path) -> Option<&Entry> {
        if Self::is_root(path) { Some(&Entry::Dir) } else { self.entries.get(path) }
    }

    fn set(&mut self, path: PathBuf, entry: Option<Entry>) {
        match entry {
            Some(e) => self.entries.insert(path.clone(), e),
            None => self.entries.remove(&path),
        };
        self.changed.insert(path);
    }

    /// Paths below `dir` (not `dir` itself), deepest last.
    fn below(&self, dir: &Path) -> Vec<PathBuf> {
        self.entries
            .range(dir.to_path_buf()..)
            .map(|(p, _)| p)
            .take_while(|p| p.starts_with(dir))
            .filter(|p| *p != dir)
            .cloned()
            .collect()
    }

    fn need_parent(&self, path: &Path) -> io::Result<()> {
        match path.parent().map(|p| self.get(p)) {
            Some(Some(Entry::Dir)) => Ok(()),
            Some(Some(Entry::File(..))) => Err(error(io::ErrorKind::NotADirectory, path, "its parent is a file")),
            _ => Err(not_found(path.parent().unwrap_or(path))),
        }
    }

    pub fn read(&self, path: &Path) -> io::Result<Vec<u8>> {
        let path = key(path);
        match self.get(&path) {
            Some(Entry::File(data, _)) => Ok(data.clone()),
            Some(Entry::Dir) => Err(error(io::ErrorKind::IsADirectory, &path, "is a folder")),
            None => Err(not_found(&path)),
        }
    }

    pub fn write(&mut self, path: &Path, data: Vec<u8>) -> io::Result<()> {
        let path = key(path);
        self.need_parent(&path)?;
        match self.get(&path) {
            Some(Entry::Dir) => return Err(error(io::ErrorKind::IsADirectory, &path, "is a folder")),
            // Unchanged: not written again (nor stored again), unlike `std::fs::write`.
            Some(Entry::File(old, _)) if *old == data => return Ok(()),
            _ => {}
        }
        self.set(path, Some(Entry::File(data, now())));
        Ok(())
    }

    pub fn create_dir_all(&mut self, path: &Path) -> io::Result<()> {
        let path = key(path);
        let mut at = PathBuf::from("/");
        for c in path.components().skip(1) {
            at.push(c);
            match self.get(&at) {
                Some(Entry::Dir) => {}
                Some(Entry::File(..)) => return Err(error(io::ErrorKind::AlreadyExists, &at, "is a file")),
                None => self.set(at.clone(), Some(Entry::Dir)),
            }
        }
        Ok(())
    }

    pub fn remove_file(&mut self, path: &Path) -> io::Result<()> {
        let path = key(path);
        match self.get(&path) {
            Some(Entry::File(..)) => {
                self.set(path, None);
                Ok(())
            }
            Some(Entry::Dir) => Err(error(io::ErrorKind::IsADirectory, &path, "is a folder")),
            None => Err(not_found(&path)),
        }
    }

    pub fn remove_dir_all(&mut self, path: &Path) -> io::Result<()> {
        let path = key(path);
        match self.get(&path) {
            Some(Entry::Dir) if !Self::is_root(&path) => {
                for p in self.below(&path) {
                    self.set(p, None);
                }
                self.set(path, None);
                Ok(())
            }
            Some(Entry::Dir) => Err(error(io::ErrorKind::PermissionDenied, &path, "is the root")),
            Some(Entry::File(..)) => Err(error(io::ErrorKind::NotADirectory, &path, "is a file")),
            None => Err(not_found(&path)),
        }
    }

    /// Moves a file (replacing a file at `to`) or a folder (to a path that doesn't
    /// exist, or an empty folder), with what's in it.
    pub fn rename(&mut self, from: &Path, to: &Path) -> io::Result<()> {
        let (from, to) = (key(from), key(to));
        let entry = self.get(&from).cloned().ok_or_else(|| not_found(&from))?;
        if from == to {
            return Ok(());
        }
        self.need_parent(&to)?;
        match (&entry, self.get(&to)) {
            (Entry::File(..), Some(Entry::Dir)) => return Err(error(io::ErrorKind::IsADirectory, &to, "is a folder")),
            (Entry::Dir, Some(Entry::File(..))) => return Err(error(io::ErrorKind::NotADirectory, &to, "is a file")),
            (Entry::Dir, Some(Entry::Dir)) if !self.below(&to).is_empty() => {
                return Err(error(io::ErrorKind::DirectoryNotEmpty, &to, "isn't empty"));
            }
            (Entry::Dir, _) if to.starts_with(&from) => {
                return Err(error(io::ErrorKind::InvalidInput, &to, "is inside the folder moved"));
            }
            _ => {}
        }
        for p in self.below(&from) {
            let moved = to.join(p.strip_prefix(&from).expect("below"));
            let e = self.entries.get(&p).cloned();
            self.set(p, None);
            self.set(moved, e);
        }
        self.set(from, None);
        self.set(to, Some(entry));
        Ok(())
    }

    pub fn copy(&mut self, from: &Path, to: &Path) -> io::Result<u64> {
        let data = self.read(from)?;
        let n = data.len() as u64;
        self.write(to, data)?;
        Ok(n)
    }

    pub fn canonicalize(&self, path: &Path) -> io::Result<PathBuf> {
        let path = key(path);
        self.get(&path).map(|_| path.clone()).ok_or_else(|| not_found(&path))
    }

    pub fn exists(&self, path: &Path) -> bool {
        self.get(&key(path)).is_some()
    }

    pub fn is_file(&self, path: &Path) -> bool {
        matches!(self.get(&key(path)), Some(Entry::File(..)))
    }

    pub fn is_dir(&self, path: &Path) -> bool {
        matches!(self.get(&key(path)), Some(Entry::Dir))
    }

    pub fn list_dir(&self, path: &Path) -> io::Result<Vec<PathBuf>> {
        let path = key(path);
        match self.get(&path) {
            Some(Entry::Dir) => Ok(self.below(&path).into_iter().filter(|p| p.parent() == Some(&path)).collect()),
            Some(Entry::File(..)) => Err(error(io::ErrorKind::NotADirectory, &path, "is a file")),
            None => Err(not_found(&path)),
        }
    }

    pub fn modified(&self, path: &Path) -> Option<u64> {
        match self.get(&key(path))? {
            Entry::File(_, t) => Some(*t),
            Entry::Dir => None,
        }
    }

    /// The paths changed since the last call, each with what's there now (`None`:
    /// removed).
    pub fn take_changes(&mut self) -> Vec<(PathBuf, Option<Entry>)> {
        std::mem::take(&mut self.changed).into_iter().map(|p| (p.clone(), self.entries.get(&p).cloned())).collect()
    }

    /// Puts an entry back as it was saved (not a change).
    pub fn restore(&mut self, path: &Path, entry: Entry) {
        let path = key(path);
        if !Self::is_root(&path) {
            self.entries.insert(path, entry);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(s: &str) -> &Path {
        Path::new(s)
    }

    #[test]
    fn files_are_written_into_existing_folders() {
        let mut fs = MemFs::default();
        assert_eq!(fs.write(p("/a/b.txt"), b"x".to_vec()).unwrap_err().kind(), io::ErrorKind::NotFound);
        fs.create_dir_all(p("/a/c")).unwrap();
        fs.write(p("/a/b.txt"), b"x".to_vec()).unwrap();
        assert_eq!(fs.read(p("/a/./c/../b.txt")).unwrap(), b"x");
        assert!(fs.is_file(p("/a/b.txt")) && fs.is_dir(p("/a")) && fs.is_dir(p("/")) && !fs.exists(p("/b")));
        let mut listed = fs.list_dir(p("/a")).unwrap();
        listed.sort();
        assert_eq!(listed, [PathBuf::from("/a/b.txt"), PathBuf::from("/a/c")]);
        assert_eq!(fs.list_dir(p("/")).unwrap(), [PathBuf::from("/a")]);
        assert!(fs.modified(p("/a/b.txt")).is_some());
        assert_eq!(fs.create_dir_all(p("/a/b.txt/d")).unwrap_err().kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(fs.write(p("/a/c"), Vec::new()).unwrap_err().kind(), io::ErrorKind::IsADirectory);
    }

    #[test]
    fn removing_tells_files_from_folders() {
        let mut fs = MemFs::default();
        fs.create_dir_all(p("/a/b")).unwrap();
        fs.write(p("/a/b/f"), Vec::new()).unwrap();
        fs.write(p("/ab"), Vec::new()).unwrap();
        assert!(fs.remove_file(p("/a")).is_err());
        assert!(fs.remove_dir_all(p("/ab")).is_err());
        fs.remove_dir_all(p("/a")).unwrap();
        assert!(!fs.exists(p("/a/b/f")) && !fs.exists(p("/a")));
        assert!(fs.exists(p("/ab")), "a sibling with the same prefix stays");
        fs.remove_file(p("/ab")).unwrap();
        assert!(fs.remove_file(p("/ab")).is_err());
    }

    #[test]
    fn renaming_moves_folders_with_their_files() {
        let mut fs = MemFs::default();
        fs.create_dir_all(p("/tmp/x/sub")).unwrap();
        fs.write(p("/tmp/x/sub/f"), b"1".to_vec()).unwrap();
        fs.create_dir_all(p("/projects")).unwrap();
        fs.rename(p("/tmp/x"), p("/projects/y")).unwrap();
        assert_eq!(fs.read(p("/projects/y/sub/f")).unwrap(), b"1");
        assert!(!fs.exists(p("/tmp/x")));
        // A file replaces a file (how saving writes `x.tmp`, then renames it).
        fs.write(p("/projects/a.tmp"), b"new".to_vec()).unwrap();
        fs.write(p("/projects/a"), b"old".to_vec()).unwrap();
        fs.rename(p("/projects/a.tmp"), p("/projects/a")).unwrap();
        assert_eq!(fs.read(p("/projects/a")).unwrap(), b"new");
        assert!(fs.rename(p("/projects"), p("/projects/y/in")).is_err(), "into itself");
        assert!(fs.rename(p("/projects/a"), p("/projects/y")).is_err(), "a file onto a folder");
    }

    #[test]
    fn changes_are_handed_out_once() {
        let mut fs = MemFs::default();
        fs.create_dir_all(p("/d")).unwrap();
        fs.write(p("/d/f"), b"1".to_vec()).unwrap();
        let changes = fs.take_changes();
        assert_eq!(changes.len(), 2);
        assert!(matches!(&changes[1], (path, Some(Entry::File(data, _))) if path == p("/d/f") && data == b"1"));
        assert!(fs.take_changes().is_empty());
        fs.write(p("/d/f"), b"1".to_vec()).unwrap();
        assert!(fs.take_changes().is_empty(), "the same content again");
        fs.remove_dir_all(p("/d")).unwrap();
        assert_eq!(fs.take_changes(), vec![(PathBuf::from("/d"), None), (PathBuf::from("/d/f"), None)]);

        let mut loaded = MemFs::default();
        loaded.restore(p("/d"), Entry::Dir);
        loaded.restore(p("/d/f"), Entry::File(b"2".to_vec(), 7));
        assert_eq!(loaded.read(p("/d/f")).unwrap(), b"2");
        assert!(loaded.take_changes().is_empty(), "restoring isn't a change");
    }
}
