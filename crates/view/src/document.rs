//! An open project: editor (model + history), file location and both views.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use editor_core::connectivity::Connectivity;
use editor_core::copper::BoardConnectivity;
use editor_core::drc::{self, Violation};
use editor_core::{ChangeSet, Command, CommandError, Editor};
use model::io::{FileError, load_project, save_project};
use model::{Coord, ObjectId, Point, Project, Rect};

use crate::interaction::{Edit, Target, reference_point};
use crate::view::{View, ViewKind};

pub struct Document {
    editor: Editor,
    /// Project folder; `None` until saved.
    pub path: Option<PathBuf>,
    pub schematic: View,
    pub board: View,
    /// Bumped on every model change, so UI caches can tell when to refresh.
    pub revision: u64,
    connectivity: Connectivity,
    /// The schematic changed since `connectivity` was computed.
    connectivity_stale: bool,
    connectivity_time: Duration,
    copper: BoardConnectivity,
    /// The board (or the schematic) changed since `copper` was computed.
    copper_stale: bool,
    copper_time: Duration,
    drc: Option<DrcReport>,
    /// Why edits are refused (e.g. the built-in library); `None`: editable.
    pub read_only: Option<String>,
}

/// Result of the last design rule check.
#[derive(Clone, Debug)]
pub struct DrcReport {
    pub violations: Vec<Violation>,
    /// [`Document::revision`] it was run at.
    pub revision: u64,
    pub time: Duration,
}

impl Document {
    pub fn new(project: Project) -> Self {
        let schematic = View::new(ViewKind::Schematic, &project);
        let board = View::new(ViewKind::Board, &project);
        let mut doc = Document {
            editor: Editor::new(project),
            path: None,
            schematic,
            board,
            revision: 0,
            connectivity: Connectivity::default(),
            connectivity_stale: true,
            connectivity_time: Duration::ZERO,
            copper: BoardConnectivity::default(),
            copper_stale: true,
            copper_time: Duration::ZERO,
            drc: None,
            read_only: None,
        };
        doc.update_connectivity(false);
        doc
    }

    /// Schematic connectivity as of the last [`update_connectivity`](Self::update_connectivity).
    pub fn connectivity(&self) -> &Connectivity {
        &self.connectivity
    }

    /// Up-to-date connectivity (recomputed if the schematic changed).
    pub fn current_connectivity(&mut self) -> &Connectivity {
        self.update_connectivity(false);
        &self.connectivity
    }

    /// Board connectivity (islands, nets of copper, ratsnest) as of the last
    /// [`update_connectivity`](Self::update_connectivity).
    pub fn copper(&self) -> &BoardConnectivity {
        &self.copper
    }

    /// Up-to-date board connectivity.
    pub fn current_copper(&mut self) -> &BoardConnectivity {
        self.update_connectivity(false);
        &self.copper
    }

    /// Run the design rule check on the current board and mark the violations.
    pub fn run_drc(&mut self) -> &DrcReport {
        self.update_connectivity(false);
        let t = Instant::now();
        let violations = drc::run(self.editor.project(), &self.copper);
        let report = DrcReport { violations, revision: self.revision, time: t.elapsed() };
        self.board.set_drc(&report.violations);
        self.drc.insert(report)
    }

    /// The last design rule check (it may be outdated: compare the revision).
    pub fn drc(&self) -> Option<&DrcReport> {
        self.drc.as_ref()
    }

    /// Forget the design rule check and its markers.
    pub fn clear_drc(&mut self) {
        self.drc = None;
        self.board.set_drc(&[]);
    }

    /// Redraw both views and their overlays in the current palette (after
    /// [`crate::style::set_theme`]).
    pub fn restyle(&mut self) {
        let project = self.editor.project();
        for view in [&mut self.schematic, &mut self.board] {
            let probed = view.probed().to_vec();
            view.rebuild(project);
            view.set_probe(project, &probed);
        }
        self.schematic.set_connectivity(&self.connectivity);
        self.board.set_copper(project, &self.copper);
    }

    /// Highlight the copper of these nets on the board (empty clears).
    pub fn highlight_nets(&mut self, nets: &[String]) {
        self.board.set_nets(&self.copper, nets);
    }

    /// Recompute schematic and board connectivity where they changed, and show junction
    /// dots, unconnected pins and the ratsnest. While `busy` (dragging), large projects
    /// whose connectivity takes long are only updated afterwards. Returns true if
    /// anything was recomputed.
    pub fn update_connectivity(&mut self, busy: bool) -> bool {
        let slow = |t: Duration| busy && t > Duration::from_millis(4);
        let mut done = false;
        if self.connectivity_stale && !slow(self.connectivity_time) {
            let t = Instant::now();
            self.connectivity = Connectivity::build(self.editor.project());
            self.connectivity_time = t.elapsed();
            self.connectivity_stale = false;
            self.copper_stale = true;
            self.schematic.set_connectivity(&self.connectivity);
            done = true;
        }
        if self.copper_stale && !self.connectivity_stale && !slow(self.copper_time) {
            let t = Instant::now();
            self.copper = BoardConnectivity::build(self.editor.project(), &self.connectivity);
            self.copper_time = t.elapsed();
            self.copper_stale = false;
            self.board.set_copper(self.editor.project(), &self.copper);
            done = true;
        }
        done
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
        if let Some(why) = &self.read_only {
            return Err(CommandError::Forbidden(why.clone()));
        }
        let changes = self.editor.execute_grouped(command, group)?;
        self.changed(&changes);
        Ok(changes)
    }

    /// See [`Editor::set_library_name`].
    pub fn set_library_name(&mut self, name: String) {
        self.editor.set_library_name(name);
    }

    /// Mark the current state as saved (for documents saved by other means).
    pub fn mark_saved(&mut self) {
        self.editor.mark_saved();
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
        if changes.touches_schematic() || changes.ids().any(|id| id.kind().is_library()) {
            self.connectivity_stale = true;
        }
        if changes.touches_board() || self.connectivity_stale {
            self.copper_stale = true;
        }
        let project = self.editor.project();
        self.schematic.apply(project, changes);
        self.board.apply(project, changes);
    }
}

/// A schematic or board view of a document, as an interaction [`Target`].
pub struct ProjectTarget<'a> {
    pub doc: &'a mut Document,
    pub kind: ViewKind,
}

impl Document {
    pub fn target(&mut self, kind: ViewKind) -> ProjectTarget<'_> {
        ProjectTarget { doc: self, kind }
    }
}

impl ProjectTarget<'_> {
    fn run(&mut self, cmd: Command, group: Option<u64>) -> Result<(), String> {
        self.doc.execute_grouped(cmd, group).map(drop).map_err(|e| e.to_string())
    }
}

impl Target for ProjectTarget<'_> {
    type Id = ObjectId;

    fn hit(&self, p: Point, tol: Coord) -> Option<ObjectId> {
        self.doc.view(self.kind).hit(self.doc.project(), p, tol)
    }

    fn in_rect(&self, r: Rect, crossing: bool) -> Vec<ObjectId> {
        self.doc.view(self.kind).select_rect(r, crossing)
    }

    fn all(&self) -> Vec<ObjectId> {
        self.doc.view(self.kind).index.ids()
    }

    fn bounds(&self, id: ObjectId) -> Option<Rect> {
        self.doc.view(self.kind).index.bounds(id)
    }

    fn exists(&self, id: ObjectId) -> bool {
        self.doc.project().contains(id)
    }

    fn anchor(&self, id: ObjectId) -> Option<Point> {
        let project = self.doc.project();
        match id {
            ObjectId::Footprint(f) if project.board.footprints.get(f).is_some_and(|f| f.locked) => None,
            _ => reference_point(project, id),
        }
    }

    fn edit(&mut self, edit: Edit<ObjectId>, group: Option<u64>) -> Result<Option<String>, String> {
        let is_fp = |id: &ObjectId| matches!(id, ObjectId::Footprint(_));
        // Wires stay attached to what moves in the schematic.
        let sch = self.kind == ViewKind::Schematic;
        match edit {
            Edit::Move { ids, delta } => self.run(Command::Move { ids, delta, keep_connected: sch }, group)?,
            Edit::Rotate { ids, center, angle } => {
                self.run(Command::Rotate { ids, center, angle, keep_connected: sch }, group)?
            }
            Edit::Mirror { ids, axis_x } | Edit::Flip { ids, axis_x } => match self.kind {
                ViewKind::Schematic => self.run(Command::Mirror { ids, axis_x, keep_connected: true }, group)?,
                ViewKind::Board => {
                    let ids: Vec<ObjectId> = ids.into_iter().filter(is_fp).collect();
                    if ids.is_empty() {
                        return Err("Only footprints can be flipped".into());
                    }
                    self.run(Command::FlipSide { ids, axis_x }, group)?
                }
            },
            Edit::Delete { ids } => {
                let (footprints, ids): (Vec<ObjectId>, Vec<ObjectId>) = ids.into_iter().partition(is_fp);
                if !ids.is_empty() {
                    self.run(Command::Delete { ids }, group)?;
                }
                if !footprints.is_empty() {
                    return Ok(Some("Footprints follow the schematic: delete the component there".into()));
                }
            }
        }
        Ok(None)
    }

    fn close_group(&mut self) {
        self.doc.close_group();
    }

    fn undo(&mut self) {
        self.doc.undo();
    }

    fn highlight(&mut self, selection: &[ObjectId], hover: Option<ObjectId>) {
        let (project, view) = self.doc.project_and_view(self.kind);
        view.set_highlight(project, selection, hover);
    }
}
