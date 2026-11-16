//! An open project: editor (model + history), file location and both views.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use editor_core::connectivity::Connectivity;
use editor_core::copper::BoardConnectivity;
use editor_core::drc::{self, Violation};
use editor_core::route_check;
use editor_core::{ChangeSet, Command, CommandError, Editor};
use model::io::{FileError, load_project, save_project};
use model::{Component, Coord, Id, LayerId, ObjectId, ObjectKind, Point, Project, Rect, Text};

use crate::interaction::{Edit, Target, reference_point};
use crate::text_edit::{EditGeometry, EditKey, EditSite, Mods, Outcome, TextEdit, TextTarget};
use crate::view::{View, ViewId, ViewKind};

pub struct Document {
    editor: Editor,
    /// Project folder; `None` until saved.
    pub path: Option<PathBuf>,
    /// One view per schematic sheet (in the project's sheet order).
    sheets: Vec<View>,
    pub board: View,
    /// Stands in for a sheet that doesn't exist (any more): draws nothing.
    no_sheet: View,
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
    /// Library objects whose placed symbols and footprints still show the old version (see
    /// [`update_connectivity`](Self::update_connectivity)).
    stale_library: Vec<ObjectId>,
    /// How long redrawing the users of a library edit took last time.
    library_time: Duration,
    /// An interaction is in progress, as of the last `update_connectivity`.
    busy: bool,
    /// Updates that took longer than this wait while dragging (4 ms).
    pub slow: Duration,
    /// Why edits are refused (e.g. the built-in library); `None`: editable.
    pub read_only: Option<String>,
    /// A text being edited in place, in a view, and the colour it's drawn in.
    text_edit: Option<(EditSite, TextEdit, render::Color)>,
}

/// Result of the last design rule check.
#[derive(Clone, Debug)]
pub struct DrcReport {
    pub violations: Vec<Violation>,
    /// [`Document::revision`] it was run at.
    pub revision: u64,
    pub time: Duration,
}

/// Result of [`Document::export_gerber`].
#[derive(Clone, Debug)]
pub struct GerberExport {
    /// The folder the files went into.
    pub dir: PathBuf,
    pub manifest: gerber::Manifest,
    /// Design rule violations at the time of the export.
    pub warnings: Vec<Violation>,
}

impl Document {
    pub fn new(project: Project) -> Self {
        let sheets = project.schematic.sheets.ids().map(|s| View::new(ViewId::Sheet(s), &project)).collect();
        let board = View::new(ViewId::Board, &project);
        let no_sheet = View::new(ViewId::Sheet(Id::from_u128(0)), &project);
        let mut doc = Document {
            editor: Editor::new(project),
            path: None,
            sheets,
            board,
            no_sheet,
            revision: 0,
            connectivity: Connectivity::default(),
            connectivity_stale: true,
            connectivity_time: Duration::ZERO,
            copper: BoardConnectivity::default(),
            copper_stale: true,
            copper_time: Duration::ZERO,
            drc: None,
            stale_library: Vec::new(),
            library_time: Duration::ZERO,
            busy: false,
            slow: Duration::from_millis(4),
            read_only: None,
            text_edit: None,
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

    /// The text being edited in place, and where.
    pub fn text_edit(&self) -> Option<(EditSite, &TextEdit)> {
        self.text_edit.as_ref().map(|(v, e, _)| (*v, e))
    }

    /// The text being edited in the editor of `item`, to draw.
    pub fn item_text_edit(&self, item: crate::item::Item) -> Option<(EditGeometry, render::Color)> {
        let (site, e, color) = self.text_edit.as_ref()?;
        (*site == EditSite::Item(item)).then(|| (e.geometry(), *color))
    }

    /// Start editing text `index` of a symbol or footprint (library editors). Returns
    /// false if there is no such text.
    pub fn begin_item_text_edit(&mut self, item: crate::item::Item, index: usize) -> bool {
        use crate::item::Content;
        self.commit_text_edit().ok();
        let (text, color) = match Content::get(self.editor.project(), item) {
            Some(Content::Symbol(s)) => match s.texts.get(index) {
                Some(t) => (t.clone(), crate::style::pal().sch_text),
                None => return false,
            },
            Some(Content::Footprint(f)) => match f.texts.get(index) {
                Some(t) => (t.text.clone(), crate::style::board_layer_color(t.board_layer(model::Side::Top))),
                None => return false,
            },
            None => return false,
        };
        let edit = TextEdit::new(TextTarget::ItemText { item, index }, text.text.clone(), text);
        self.text_edit = Some((EditSite::Item(item), edit, color));
        true
    }

    /// Start editing the text of `id` in `view`, double-clicked at `at` (a sheet's name
    /// only when `at` is on it). Returns false if it has no editable text.
    pub fn begin_text_edit(&mut self, view: ViewId, id: ObjectId, at: Point) -> bool {
        self.commit_text_edit().ok();
        let project = self.editor.project();
        let Some((shape, color)) = self.view(view).object_text(project, id) else { return false };
        let s = &project.schematic;
        let (target, text) = match id {
            ObjectId::SchText(i) => (TextTarget::SchText(i), s.texts.get(i).map(|t| t.text.text.clone())),
            ObjectId::BoardText(i) => {
                (TextTarget::BoardText(i), project.board.texts.get(i).map(|t| t.text.text.clone()))
            }
            ObjectId::Label(i) => (TextTarget::Label(i), s.labels.get(i).map(|l| l.name.clone())),
            ObjectId::Sheet(i) => (TextTarget::Sheet(i), s.sheets.get(i).map(|x| x.name.clone())),
            ObjectId::SymbolField(i) => {
                let f = s.fields.get(i);
                let c = f.and_then(|f| s.symbols.get(f.symbol)).and_then(|x| s.components.get(x.component));
                field_target(f.map(|f| f.text.text.as_str()), c)
            }
            ObjectId::FootprintField(i) => {
                let f = project.board.fields.get(i);
                let c = f
                    .and_then(|f| project.board.footprints.get(f.footprint))
                    .and_then(|x| s.components.get(x.component));
                field_target(f.map(|f| f.text.text.as_str()), c)
            }
            _ => return false,
        };
        let Some(text) = text else { return false };
        let edit = TextEdit::new(target, text, shape);
        if matches!(id, ObjectId::Sheet(_)) && !edit.contains(at) {
            return false;
        }
        self.text_edit = Some((EditSite::View(view), edit, color));
        self.show_text_edit();
        true
    }

    /// Start editing a new text (placed by a text tool), drawn like `shape`.
    pub fn begin_new_text(&mut self, view: ViewId, target: TextTarget, shape: Text, color: render::Color) {
        self.commit_text_edit().ok();
        self.text_edit = Some((EditSite::View(view), TextEdit::new(target, String::new(), shape), color));
        self.show_text_edit();
    }

    /// Change the text being edited (keys, typing, pointer); it is redrawn.
    pub fn with_text_edit<R>(&mut self, f: impl FnOnce(&mut TextEdit) -> R) -> Option<R> {
        let r = f(&mut self.text_edit.as_mut()?.1);
        self.show_text_edit();
        Some(r)
    }

    /// A key for the text being edited: Enter commits, Esc cancels.
    pub fn text_edit_key(&mut self, key: EditKey, mods: Mods) -> Result<(), String> {
        match self.with_text_edit(|e| e.key(key, mods)) {
            Some(Outcome::Commit) => self.commit_text_edit(),
            Some(Outcome::Cancel) => {
                self.cancel_text_edit();
                Ok(())
            }
            _ => Ok(()),
        }
    }

    /// Apply the text being edited (one undo step) and stop editing. On an error the
    /// edit ends too, with the old text kept.
    pub fn commit_text_edit(&mut self) -> Result<(), String> {
        let Some((site, edit, _)) = self.text_edit.take() else { return Ok(()) };
        if let EditSite::View(view) = site {
            self.view_mut(view).set_text_edit(None);
        }
        let command = edit.commit(self.editor.project())?;
        if let Some(c) = command {
            self.execute(c).map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    pub fn cancel_text_edit(&mut self) {
        if let Some((EditSite::View(view), _, _)) = self.text_edit.take() {
            self.view_mut(view).set_text_edit(None);
        }
    }

    fn show_text_edit(&mut self) {
        let Some((EditSite::View(view), edit, color)) = &self.text_edit else { return };
        let (view, g, color) = (*view, edit.geometry(), *color);
        self.view_mut(view).set_text_edit(Some((&g, color)));
    }

    /// The rule copper about to be added breaks (see [`route_check::check`]).
    pub fn route_clash(
        &mut self,
        probes: &[route_check::Probe],
        net: Option<&str>,
        ends: &[(LayerId, Point)],
    ) -> Option<route_check::Clash> {
        self.update_connectivity(false);
        route_check::check(self.editor.project(), &self.copper, probes, net, ends)
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

    /// Export the Gerber and drill files into the project folder (see [`gerber::export`]),
    /// with zones refilled and the design rules checked first. Violations don't stop the
    /// export: they come back as warnings. A missing outline or an unsaved project do.
    pub fn export_gerber(&mut self) -> Result<GerberExport, gerber::ExportError> {
        let dir = self.path.clone().ok_or(gerber::ExportError::NotSaved)?;
        let warnings = self.run_drc().violations.clone();
        let manifest = gerber::export(self.editor.project(), &self.copper, &dir)?;
        Ok(GerberExport { dir: gerber::gerber_dir(&dir), manifest, warnings })
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
        for view in self.sheets.iter_mut().chain([&mut self.board]) {
            let probed = view.probed().to_vec();
            view.rebuild(project);
            view.set_probe(project, &probed);
        }
        self.stale_library.clear();
        for view in &mut self.sheets {
            view.set_connectivity(&self.connectivity);
        }
        self.board.set_copper(project, &self.copper);
    }

    /// Highlight the copper of these nets on the board (empty clears).
    pub fn highlight_nets(&mut self, nets: &[String]) {
        self.board.set_nets(&self.copper, nets);
    }

    /// Recompute schematic and board connectivity where they changed, and show junction
    /// dots, unconnected pins and the ratsnest. While `busy` (dragging), large projects
    /// whose connectivity takes long are only updated afterwards. The same goes for
    /// redrawing the placed parts of an edited library object: a drag in the Project
    /// Library's symbol editor redraws every placed symbol, and with thousands of them the
    /// views catch up when the drag ends. Call it every frame. Returns true if anything was
    /// recomputed or redrawn.
    pub fn update_connectivity(&mut self, busy: bool) -> bool {
        self.busy = busy;
        let limit = self.slow;
        let slow = |t: Duration| busy && t > limit;
        let mut done = false;
        if !self.stale_library.is_empty() && !slow(self.library_time) {
            let stale = std::mem::take(&mut self.stale_library);
            self.redraw_library_users(&stale);
            done = true;
        }
        if self.connectivity_stale && !slow(self.connectivity_time) {
            let t = Instant::now();
            self.connectivity = Connectivity::build(self.editor.project());
            self.connectivity_time = t.elapsed();
            self.connectivity_stale = false;
            self.copper_stale = true;
            for view in &mut self.sheets {
                view.set_connectivity(&self.connectivity);
            }
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

    /// The view of the root sheet.
    pub fn root(&self) -> ViewId {
        ViewId::root(self.project())
    }

    /// The root sheet's view.
    pub fn schematic(&self) -> &View {
        self.view(self.root())
    }

    /// The views of all sheets.
    pub fn sheet_views(&self) -> impl Iterator<Item = &View> {
        self.sheets.iter()
    }

    /// A sheet's view or the board. A sheet that doesn't exist gets an empty view.
    pub fn view(&self, id: ViewId) -> &View {
        match id {
            ViewId::Board => &self.board,
            ViewId::Sheet(_) => self.sheets.iter().find(|v| v.id == id).unwrap_or(&self.no_sheet),
        }
    }

    pub fn view_mut(&mut self, id: ViewId) -> &mut View {
        match id {
            ViewId::Board => &mut self.board,
            ViewId::Sheet(_) => self.sheets.iter_mut().find(|v| v.id == id).unwrap_or(&mut self.no_sheet),
        }
    }

    /// The project and one view, borrowed together.
    pub fn project_and_view(&mut self, id: ViewId) -> (&Project, &mut View) {
        let view = match id {
            ViewId::Board => &mut self.board,
            ViewId::Sheet(_) => self.sheets.iter_mut().find(|v| v.id == id).unwrap_or(&mut self.no_sheet),
        };
        (self.editor.project(), view)
    }

    /// Every view (all sheets, then the board) with the project.
    pub fn project_and_views(&mut self) -> (&Project, impl Iterator<Item = &mut View>) {
        (self.editor.project(), self.sheets.iter_mut().chain([&mut self.board]))
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

    /// See [`Editor::retract`].
    pub fn retract(&mut self, group: u64) -> Option<ChangeSet> {
        let changes = self.editor.retract(group)?;
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
        if changes.changes.iter().any(|c| c.affects_connectivity()) {
            self.connectivity_stale = true;
        }
        if changes.touches_board() || self.connectivity_stale {
            self.copper_stale = true;
        }
        // Sheets that came or went (also by undo) get or lose their view.
        let project = self.editor.project();
        for c in changes.changes.iter().filter(|c| c.id.kind() == ObjectKind::Sheet) {
            let id = ViewId::Sheet(c.id.raw());
            let exists = project.schematic.sheets.contains(c.id.raw());
            let has_view = self.sheets.iter().any(|v| v.id == id);
            if exists && !has_view {
                let mut view = View::new(id, project);
                view.set_connectivity(&self.connectivity);
                self.sheets.push(view);
            } else if !exists {
                self.sheets.retain(|v| v.id != id);
            }
        }
        for view in self.sheets.iter_mut().chain([&mut self.board]) {
            view.apply_objects(project, changes);
        }
        let library = crate::view::drawn_library_changes(changes);
        if self.busy && self.library_time > self.slow {
            for id in library {
                if !self.stale_library.contains(&id) {
                    self.stale_library.push(id);
                }
            }
        } else if !library.is_empty() {
            self.redraw_library_users(&library);
        }
    }

    fn redraw_library_users(&mut self, library: &[ObjectId]) {
        let t = Instant::now();
        let project = self.editor.project();
        for view in self.sheets.iter_mut().chain([&mut self.board]) {
            view.redraw_users(project, library);
        }
        self.library_time = t.elapsed();
    }
}

/// A schematic or board view of a document, as an interaction [`Target`].
pub struct ProjectTarget<'a> {
    pub doc: &'a mut Document,
    pub view: ViewId,
}

impl Document {
    pub fn target(&mut self, view: ViewId) -> ProjectTarget<'_> {
        ProjectTarget { doc: self, view }
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
        self.doc.view(self.view).hit(self.doc.project(), p, tol)
    }

    fn in_rect(&self, r: Rect, crossing: bool) -> Vec<ObjectId> {
        self.doc.view(self.view).select_rect(r, crossing)
    }

    fn all(&self) -> Vec<ObjectId> {
        self.doc.view(self.view).index.ids()
    }

    fn bounds(&self, id: ObjectId) -> Option<Rect> {
        self.doc.view(self.view).index.bounds(id)
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
        let (edit, carry) = match edit {
            Edit::Carry(e) => (*e, true),
            e => (e, false),
        };
        // What is attached stays attached, except to a carried paste.
        let keep = !carry;
        match edit {
            Edit::Move { ids, delta } => self.run(Command::Move { ids, delta, keep_connected: keep }, group)?,
            Edit::Rotate { ids, center, angle } => {
                self.run(Command::Rotate { ids, center, angle, keep_connected: keep }, group)?
            }
            Edit::Mirror { ids, axis_x } | Edit::Flip { ids, axis_x } => match self.view.kind() {
                ViewKind::Schematic => self.run(Command::Mirror { ids, axis_x, keep_connected: keep }, group)?,
                ViewKind::Board => {
                    let ids: Vec<ObjectId> = ids.into_iter().filter(is_fp).collect();
                    if ids.is_empty() {
                        return Err("Only footprints can be flipped".into());
                    }
                    self.run(Command::FlipSide { ids, axis_x, keep_connected: keep }, group)?
                }
            },
            Edit::Delete { ids } => {
                // Footprint texts can only be hidden (in Properties).
                let (fields, ids): (Vec<ObjectId>, Vec<ObjectId>) = ids
                    .into_iter()
                    .partition(|id| matches!(id, ObjectId::FootprintField(_) | ObjectId::SymbolField(_)));
                if !fields.is_empty() && ids.is_empty() {
                    return Ok(Some("Reference and value texts can't be deleted: hide them in Properties".into()));
                }
                // Board parts live on the board and are deleted there; other footprints
                // follow the schematic.
                let project = self.doc.project();
                let board_part = |id: &ObjectId| match id {
                    ObjectId::Footprint(f) => project
                        .board
                        .footprints
                        .get(*f)
                        .and_then(|f| project.schematic.components.get(f.component))
                        .and_then(|c| project.library.parts.get(c.part))
                        .is_some_and(model::Part::is_board_part),
                    _ => true,
                };
                let (ids, footprints): (Vec<ObjectId>, Vec<ObjectId>) = ids.into_iter().partition(board_part);
                if !ids.is_empty() {
                    self.run(Command::Delete { ids }, group)?;
                }
                if !footprints.is_empty() {
                    return Ok(Some("Footprints follow the schematic: delete the component there".into()));
                }
            }
            Edit::Carry(e) => return self.edit(*e, group),
        }
        Ok(None)
    }

    fn close_group(&mut self) {
        self.doc.close_group();
    }

    fn undo(&mut self) {
        self.doc.undo();
    }

    fn retract(&mut self, group: u64) {
        self.doc.retract(group);
    }

    fn chain(&self, id: ObjectId) -> Vec<ObjectId> {
        let p = self.doc.project();
        match id {
            ObjectId::Wire(_) | ObjectId::BusWire(_) => editor_core::query::wire_chain(p, id),
            ObjectId::Track(t) => editor_core::query::track_chain(p, t).into_iter().map(ObjectId::Track).collect(),
            _ => vec![id],
        }
    }

    fn drag_axis(&self, ids: &[ObjectId]) -> Option<Point> {
        let p = self.doc.project();
        let (a, b) = match ids {
            [ObjectId::Wire(i)] => p.schematic.wires.get(*i).map(|w| (w.a, w.b))?,
            [ObjectId::BusWire(i)] => p.schematic.bus_wires.get(*i).map(|w| (w.a, w.b))?,
            [ObjectId::Track(i)] => p.board.tracks.get(*i).filter(|t| t.mid.is_none()).map(|t| (t.a, t.b))?,
            _ => return None,
        };
        let d = b - a;
        // Wires: only horizontal and vertical ones; tracks: any straight one.
        let orthogonal = d.x == 0 || d.y == 0;
        if d == Point::ZERO || (!orthogonal && !matches!(ids, [ObjectId::Track(_)])) {
            return None;
        }
        // 45° tracks as a unit diagonal, so dragging can snap to whole grid steps.
        if d.x.abs() == d.y.abs() {
            return Some(Point::new(d.x.signum(), d.y.signum()));
        }
        Some(d)
    }

    fn highlight(&mut self, selection: &[ObjectId], hover: Option<ObjectId>) {
        let (project, view) = self.doc.project_and_view(self.view);
        view.set_highlight(project, selection, hover);
    }
}

/// The target and text of a reference or value field (`template` `${REF}` or `${VALUE}`).
fn field_target(template: Option<&str>, c: Option<&Component>) -> (TextTarget, Option<String>) {
    match (template, c) {
        (Some("${VALUE}"), Some(c)) => (TextTarget::Value(c.id), Some(c.value.clone())),
        (_, Some(c)) => (TextTarget::Reference(c.id), Some(c.refdes.clone())),
        _ => (TextTarget::Reference(Id::from_u128(0)), None),
    }
}
