//! Undo/redo stacks.

use crate::transaction::ChangeSet;

#[derive(Clone, Debug)]
pub struct Entry {
    pub label: String,
    pub changes: ChangeSet,
    serial: u64,
}

#[derive(Debug)]
pub struct History {
    undo: Vec<Entry>,
    redo: Vec<Entry>,
    limit: usize,
    next_serial: u64,
    /// Serial of the top undo entry when the project was last saved (0 = empty history).
    saved_at: Option<u64>,
}

impl Default for History {
    fn default() -> Self {
        Self { undo: Vec::new(), redo: Vec::new(), limit: 1000, next_serial: 1, saved_at: Some(0) }
    }
}

impl History {
    pub fn push(&mut self, label: String, changes: ChangeSet) {
        self.redo.clear();
        self.undo.push(Entry { label, changes, serial: self.next_serial });
        self.next_serial += 1;
        if self.undo.len() > self.limit {
            self.undo.remove(0);
            // The saved state may no longer be reachable by undo.
            if let Some(s) = self.saved_at
                && !self.undo.iter().any(|e| e.serial == s)
            {
                self.saved_at = None;
            }
        }
    }

    pub(crate) fn pop_undo(&mut self) -> Option<Entry> {
        self.undo.pop()
    }

    pub(crate) fn push_redo(&mut self, e: Entry) {
        self.redo.push(e);
    }

    pub(crate) fn pop_redo(&mut self) -> Option<Entry> {
        self.redo.pop()
    }

    pub(crate) fn push_undo(&mut self, e: Entry) {
        self.undo.push(e);
    }

    pub fn undo_label(&self) -> Option<&str> {
        self.undo.last().map(|e| e.label.as_str())
    }

    pub fn redo_label(&self) -> Option<&str> {
        self.redo.last().map(|e| e.label.as_str())
    }

    fn current(&self) -> u64 {
        self.undo.last().map_or(0, |e| e.serial)
    }

    pub fn mark_saved(&mut self) {
        self.saved_at = Some(self.current());
    }

    pub fn is_modified(&self) -> bool {
        self.saved_at != Some(self.current())
    }
}
