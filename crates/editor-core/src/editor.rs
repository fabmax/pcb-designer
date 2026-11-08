//! [`Editor`]: a project plus its undo history. All modifications go through
//! [`Editor::execute`], [`Editor::undo`] and [`Editor::redo`], which return the effective
//! changes so views and derived data (connectivity, render scenes) can update
//! incrementally.

use model::Project;

use crate::CommandError;
use crate::command::Command;
use crate::history::History;
use crate::sync::sync_board;
use crate::transaction::{ChangeSet, Transaction, apply_forward, apply_reverse};

pub struct Editor {
    project: Project,
    history: History,
}

impl Editor {
    pub fn new(project: Project) -> Self {
        Self { project, history: History::default() }
    }

    pub fn project(&self) -> &Project {
        &self.project
    }

    pub fn into_project(self) -> Project {
        self.project
    }

    pub fn history(&self) -> &History {
        &self.history
    }

    /// Run a command atomically: on error nothing changes.
    pub fn execute(&mut self, command: Command) -> Result<ChangeSet, CommandError> {
        self.execute_grouped(command, None)
    }

    /// Like [`Editor::execute`], but consecutive commands with the same `group` (e.g. the
    /// per-frame moves of one drag) become a single undo step. Call
    /// [`Editor::close_group`] when the interaction ends.
    pub fn execute_grouped(&mut self, command: Command, group: Option<u64>) -> Result<ChangeSet, CommandError> {
        let label = command.label();
        let mut tx = Transaction::new(&mut self.project);
        let result = command.apply(&mut tx).and_then(|()| sync_board(&mut tx));
        match result {
            Ok(()) => {
                let changes = tx.finish();
                if !changes.is_empty() {
                    self.history.push_grouped(label, changes.clone(), group);
                }
                Ok(changes)
            }
            Err(e) => {
                tx.rollback();
                Err(e)
            }
        }
    }

    /// Parse a JSON command (as sent by scripts or MCP clients) and execute it.
    pub fn execute_json(&mut self, json: &str) -> Result<ChangeSet, CommandError> {
        let command: Command = serde_json::from_str(json).map_err(|e| CommandError::Invalid(e.to_string()))?;
        self.execute(command)
    }

    pub fn undo(&mut self) -> Option<ChangeSet> {
        let entry = self.history.pop_undo()?;
        let effective = apply_reverse(&mut self.project, &entry.changes);
        self.history.push_redo(entry);
        Some(effective)
    }

    pub fn redo(&mut self) -> Option<ChangeSet> {
        let entry = self.history.pop_redo()?;
        let effective = apply_forward(&mut self.project, &entry.changes);
        self.history.push_undo(entry);
        Some(effective)
    }

    pub fn close_group(&mut self) {
        self.history.close_group();
    }

    /// Rename the project's library. Metadata, not an undoable edit (library files take the
    /// name of the file they are saved to).
    pub fn set_library_name(&mut self, name: String) {
        self.project.library.name = name;
    }

    pub fn mark_saved(&mut self) {
        self.history.mark_saved();
    }

    pub fn is_modified(&self) -> bool {
        self.history.is_modified()
    }
}
