//! An open project: editor (model + history), file location and both views.

use std::path::{Path, PathBuf};

use editor_core::{ChangeSet, Command, CommandError, Editor};
use model::Project;
use model::io::{FileError, load_project, save_project};

use crate::view::{View, ViewKind};

pub struct Document {
    editor: Editor,
    /// Project folder; `None` until saved.
    pub path: Option<PathBuf>,
    pub schematic: View,
    pub board: View,
    /// Bumped on every model change, so UI caches can tell when to refresh.
    pub revision: u64,
}

impl Document {
    pub fn new(project: Project) -> Self {
        let schematic = View::new(ViewKind::Schematic, &project);
        let board = View::new(ViewKind::Board, &project);
        Document { editor: Editor::new(project), path: None, schematic, board, revision: 0 }
    }

    pub fn open(dir: impl AsRef<Path>) -> Result<Self, FileError> {
        let project = load_project(dir.as_ref())?;
        let mut doc = Document::new(project);
        doc.path = Some(dir.as_ref().to_path_buf());
        Ok(doc)
    }

    /// Save to the current path. Fails if the document has none yet.
    pub fn save(&mut self) -> Result<(), FileError> {
        let path = self.path.clone().ok_or_else(|| FileError::Io {
            path: PathBuf::new(),
            source: std::io::Error::new(std::io::ErrorKind::NotFound, "document has no path"),
        })?;
        self.save_as(path)
    }

    pub fn save_as(&mut self, dir: impl AsRef<Path>) -> Result<(), FileError> {
        save_project(self.editor.project(), dir.as_ref())?;
        self.path = Some(dir.as_ref().to_path_buf());
        self.editor.mark_saved();
        Ok(())
    }

    pub fn project(&self) -> &Project {
        self.editor.project()
    }

    pub fn editor(&self) -> &Editor {
        &self.editor
    }

    pub fn view(&self, kind: ViewKind) -> &View {
        match kind {
            ViewKind::Schematic => &self.schematic,
            ViewKind::Board => &self.board,
        }
    }

    pub fn view_mut(&mut self, kind: ViewKind) -> &mut View {
        match kind {
            ViewKind::Schematic => &mut self.schematic,
            ViewKind::Board => &mut self.board,
        }
    }

    /// The project and one view, borrowed together.
    pub fn project_and_view(&mut self, kind: ViewKind) -> (&Project, &mut View) {
        let view = match kind {
            ViewKind::Schematic => &mut self.schematic,
            ViewKind::Board => &mut self.board,
        };
        (self.editor.project(), view)
    }

    pub fn is_modified(&self) -> bool {
        self.editor.is_modified()
    }

    pub fn title(&self) -> String {
        let name = &self.project().name;
        if self.is_modified() { format!("{name} •") } else { name.clone() }
    }

    pub fn execute(&mut self, command: Command) -> Result<ChangeSet, CommandError> {
        self.execute_grouped(command, None)
    }

    pub fn execute_grouped(&mut self, command: Command, group: Option<u64>) -> Result<ChangeSet, CommandError> {
        let changes = self.editor.execute_grouped(command, group)?;
        self.changed(&changes);
        Ok(changes)
    }

    pub fn close_group(&mut self) {
        self.editor.close_group();
    }

    pub fn undo(&mut self) -> Option<ChangeSet> {
        let changes = self.editor.undo()?;
        self.changed(&changes);
        Some(changes)
    }

    pub fn redo(&mut self) -> Option<ChangeSet> {
        let changes = self.editor.redo()?;
        self.changed(&changes);
        Some(changes)
    }

    fn changed(&mut self, changes: &ChangeSet) {
        if changes.is_empty() {
            return;
        }
        self.revision += 1;
        let project = self.editor.project();
        self.schematic.apply(project, changes);
        self.board.apply(project, changes);
    }
}
