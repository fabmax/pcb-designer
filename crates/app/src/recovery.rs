//! Recovery copies of unsaved changes.
//!
//! The project itself is only written when the user saves. Besides that, every running
//! app keeps a recovery copy of its unsaved changes in its own *slot*: a folder in the
//! settings folder's `recovery/`, written [`INTERVAL`] after the first change not in it.
//! The slot is emptied when the project is saved (or its changes are undone or
//! discarded) and removed when the app quits.
//!
//! The app holds a lock on its slot's `lock` file while it runs, and the OS lets go of
//! it when the app crashes. So a slot nobody holds is one a crashed app left: at the
//! next start (or when its project is opened) the app offers to restore it.

use std::fs::File;
use std::path::{Path, PathBuf};
// File times are std's; there's no recovery in the browser (no config folder).
use std::time::{SystemTime, UNIX_EPOCH};
use web_time::{Duration, Instant};

use model::Project;
use serde::{Deserialize, Serialize};

use crate::background::{self, Job};

/// How long changes wait before they are written to the recovery copy.
pub const INTERVAL: Duration = Duration::from_secs(5 * 60);

const META: &str = "recovery.json";
const LOCK: &str = "lock";
const PROJECT: &str = "project";
const NEXT: &str = "project.next";

/// The recovery folder in the settings folder.
pub fn root() -> Option<PathBuf> {
    crate::settings::config_dir().map(|d| d.join("recovery"))
}

/// What a recovery copy is of.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Meta {
    /// The project's name.
    pub name: String,
    /// Its folder (`None`: never saved).
    pub project: Option<PathBuf>,
    /// When the copy was written (seconds since 1970).
    pub written: u64,
}

/// A recovery copy left by an app that didn't quit cleanly.
#[derive(Clone, Debug)]
pub struct Entry {
    pub dir: PathBuf,
    pub meta: Meta,
}

impl Entry {
    pub fn load(&self) -> Result<Project, model::io::FileError> {
        model::io::load_project(self.dir.join(PROJECT))
    }

    /// The project folder was saved after the copy was written (restoring it would go
    /// back to an older state).
    pub fn saved_since(&self) -> bool {
        let Some(dir) = &self.meta.project else { return false };
        let saved = std::fs::metadata(dir.join(model::io::PROJECT_FILE)).and_then(|m| m.modified());
        saved.is_ok_and(|t| secs(t) > self.meta.written)
    }

    pub fn discard(&self) {
        if let Err(e) = std::fs::remove_dir_all(&self.dir) {
            log::warn!("can't remove {}: {e}", self.dir.display());
        }
    }

    /// How long ago the copy was written, in words.
    pub fn age(&self) -> String {
        let s = now().saturating_sub(self.meta.written);
        match s {
            0..60 => "less than a minute ago".into(),
            60..120 => "a minute ago".into(),
            120..3600 => format!("{} minutes ago", s / 60),
            3600..7200 => "an hour ago".into(),
            7200..172_800 => format!("{} hours ago", s / 3600),
            _ => format!("{} days ago", s / 86_400),
        }
    }
}

/// The recovery copies in `root` that no running app holds, newest first. Slots
/// without a copy are removed.
pub fn left_over(root: &Path) -> Vec<Entry> {
    let Ok(dirs) = std::fs::read_dir(root) else { return Vec::new() };
    let mut out: Vec<Entry> = dirs
        .flatten()
        .map(|d| d.path())
        .filter(|dir| dir.is_dir() && !held(dir))
        .filter_map(|dir| {
            let meta = std::fs::read(dir.join(META)).ok().and_then(|b| serde_json::from_slice(&b).ok());
            match meta {
                Some(meta) if dir.join(PROJECT).is_dir() => Some(Entry { dir, meta }),
                _ => {
                    let _ = std::fs::remove_dir_all(&dir);
                    None
                }
            }
        })
        .collect();
    out.sort_by_key(|e| std::cmp::Reverse(e.meta.written));
    out
}

/// A running app holds the slot's lock.
fn held(dir: &Path) -> bool {
    let Ok(file) = File::options().read(true).write(true).open(dir.join(LOCK)) else { return false };
    file.try_lock().is_err()
}

/// This app's slot.
pub struct Slot {
    dir: PathBuf,
    _lock: File,
    /// The document revision in the copy (`None`: no copy).
    written: Option<u64>,
    /// Since when there are changes the copy doesn't have.
    dirty_since: Option<Instant>,
    job: Option<Job<()>>,
}

impl Slot {
    /// Make and lock a slot in `root`.
    pub fn new(root: &Path) -> std::io::Result<Self> {
        let dir = root.join(format!("{}-{}", now(), std::process::id()));
        std::fs::create_dir_all(&dir)?;
        let lock = File::create(dir.join(LOCK))?;
        lock.try_lock().map_err(std::io::Error::other)?;
        Ok(Self { dir, _lock: lock, written: None, dirty_since: None, job: None })
    }

    /// Call every frame with the document's state; `copy` gives the project and its
    /// folder when a copy is written. Returns how long until the next write is due.
    pub fn tick(
        &mut self,
        now: Instant,
        modified: bool,
        revision: u64,
        copy: impl FnOnce() -> (Project, Option<PathBuf>),
    ) -> Option<Duration> {
        if !modified {
            if self.written.is_some() {
                self.clear();
            }
            self.dirty_since = None;
            return None;
        }
        if self.written == Some(revision) {
            self.dirty_since = None;
            return None;
        }
        let due = *self.dirty_since.get_or_insert(now) + INTERVAL;
        if now < due {
            return Some(due - now);
        }
        if self.job.as_ref().is_some_and(|j| !j.is_finished()) {
            return Some(Duration::from_secs(1));
        }
        self.wait();
        let (project, folder) = copy();
        let dir = self.dir.clone();
        self.job = Some(background::spawn(move || {
            if let Err(e) = write(&dir, &project, folder) {
                log::warn!("recovery copy not written: {e}");
            }
        }));
        self.written = Some(revision);
        self.dirty_since = None;
        None
    }

    /// Write the copy at the next tick (a restored copy: it is this slot's now).
    pub fn write_soon(&mut self) {
        self.written = None;
        self.dirty_since = Some(Instant::now() - INTERVAL);
    }

    /// Wait for a copy being written.
    pub fn wait(&mut self) {
        if let Some(job) = self.job.take() {
            let _ = job.join();
        }
    }

    /// Remove the copy (saved, discarded, or another document).
    pub fn clear(&mut self) {
        self.wait();
        let _ = std::fs::remove_file(self.dir.join(META));
        let _ = std::fs::remove_dir_all(self.dir.join(PROJECT));
        self.written = None;
        self.dirty_since = None;
    }

    /// Quit cleanly: the slot goes.
    pub fn close(mut self) {
        self.wait();
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// Write the copy beside the old one, then swap them, so a crash while writing leaves
/// the old copy whole. The meta file goes last.
fn write(dir: &Path, project: &Project, folder: Option<PathBuf>) -> Result<(), String> {
    let next = dir.join(NEXT);
    let _ = std::fs::remove_dir_all(&next);
    model::io::save_project(project, &next).map_err(|e| e.to_string())?;
    let _ = std::fs::remove_file(dir.join(META));
    let _ = std::fs::remove_dir_all(dir.join(PROJECT));
    std::fs::rename(&next, dir.join(PROJECT)).map_err(|e| e.to_string())?;
    let meta = Meta { name: project.name.clone(), project: folder, written: now() };
    let json = serde_json::to_vec_pretty(&meta).map_err(|e| e.to_string())?;
    std::fs::write(dir.join(META), json).map_err(|e| e.to_string())
}

fn secs(t: SystemTime) -> u64 {
    t.duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

fn now() -> u64 {
    secs(SystemTime::now())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn copy(name: &str) -> impl FnOnce() -> (Project, Option<PathBuf>) {
        let project = Project::new(name);
        move || (project, Some(PathBuf::from("/somewhere/x")))
    }

    #[test]
    fn a_copy_is_written_after_the_interval_and_held_while_running() {
        let root = tempfile::tempdir().unwrap();
        let mut slot = Slot::new(root.path()).unwrap();
        let t0 = Instant::now();
        assert_eq!(slot.tick(t0, false, 0, copy("A")), None, "nothing to keep");
        assert_eq!(slot.tick(t0, true, 1, copy("A")), Some(INTERVAL));
        let later = t0 + Duration::from_secs(60);
        assert_eq!(
            slot.tick(later, true, 2, copy("A")),
            Some(INTERVAL - Duration::from_secs(60)),
            "from the first change"
        );
        assert_eq!(slot.tick(t0 + INTERVAL, true, 2, copy("A")), None);
        slot.wait();
        assert!(left_over(root.path()).is_empty(), "this app holds it");
        assert_eq!(slot.tick(t0 + INTERVAL * 3, true, 2, copy("A")), None, "unchanged: not again");

        // The app crashes: its lock goes.
        drop(slot);
        let entries = left_over(root.path());
        assert_eq!(entries.len(), 1);
        let e = &entries[0];
        assert_eq!(e.meta.name, "A");
        assert_eq!(e.meta.project, Some(PathBuf::from("/somewhere/x")));
        assert_eq!(e.load().unwrap().name, "A");
        assert!(!e.saved_since());
        e.discard();
        assert!(left_over(root.path()).is_empty());
    }

    #[test]
    fn saving_or_quitting_leaves_nothing() {
        let root = tempfile::tempdir().unwrap();
        let mut slot = Slot::new(root.path()).unwrap();
        let t0 = Instant::now();
        slot.tick(t0, true, 1, copy("A"));
        slot.tick(t0 + INTERVAL, true, 1, copy("A"));
        slot.tick(t0 + INTERVAL, false, 2, copy("A")); // saved
        drop(slot);
        assert!(left_over(root.path()).is_empty(), "an empty slot is no copy");
        assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 0, "and is removed");

        let mut slot = Slot::new(root.path()).unwrap();
        slot.tick(t0, true, 1, copy("B"));
        slot.tick(t0 + INTERVAL, true, 1, copy("B"));
        slot.close();
        assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 0);
    }

    #[test]
    fn a_restored_copy_is_written_again_right_away() {
        let root = tempfile::tempdir().unwrap();
        let mut slot = Slot::new(root.path()).unwrap();
        slot.write_soon();
        assert_eq!(slot.tick(Instant::now(), true, 0, copy("R")), None);
        slot.wait();
        drop(slot);
        assert_eq!(left_over(root.path())[0].meta.name, "R");
    }
}
