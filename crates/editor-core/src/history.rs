//! Undo/redo stacks.

use crate::transaction::ChangeSet;

#[derive(Clone, Debug)]
pub struct Entry {
    pub label: String,
    pub changes: ChangeSet,
    serial: u64,
    /// Consecutive commands with the same group merge into one undo step.
    group: Option<u64>,
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
        self.push_grouped(label, changes, None);
    }

    /// Push, or merge into the top entry if it belongs to the same `group`.
    pub fn push_grouped(&mut self, label: String, changes: ChangeSet, group: Option<u64>) {
        if group.is_some()
            && self.redo.is_empty()
            && let Some(top) = self.undo.last_mut()
            && top.group == group
            && self.saved_at != Some(top.serial)
        {
            top.changes = std::mem::take(&mut top.changes).then(changes);
            return;
        }
        self.redo.clear();
        self.undo.push(Entry { label, changes, serial: self.next_serial, group });
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
        let mut e = self.undo.pop()?;
        // An undone group is closed: a later command with the same group starts fresh.
        e.group = None;
        Some(e)
    }

    /// Stop merging into the current top entry.
    pub fn close_group(&mut self) {
        if let Some(top) = self.undo.last_mut() {
            top.group = None;
        }
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
