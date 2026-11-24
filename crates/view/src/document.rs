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
use crate::sim_overlay::SimOverlay;
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
    /// The DRC settings changed: the last check is redone with them.
    drc_settings_changed: bool,
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
    /// What the sheets show of a simulation.
    sim: SimOverlay,
    /// Tracks the router pushed aside for the leg being routed.
    route_preview: Option<RoutePreview>,
    /// Dragging tracks and vias on the board pushes aside what's in the way (the Route
    /// tool in Shove mode; the app sets it).
    pub drag_shove: bool,
    /// The copper before the drag (its undo group) that shoves.
    drag_base: Option<(u64, BoardConnectivity)>,
}

/// A route leg's pushed tracks, shown as real edits in an undo group of their own: retracted
/// before the next step (the router works on the board as it was) and kept by a click.
struct RoutePreview {
    group: u64,
    /// The copper before the edits (what the router works with).
    base: BoardConnectivity,
    /// The revision `base` was taken at.
    base_revision: u64,
    /// The revision after our last own step: anything else changing the document since
    /// makes this state stale.
    revision: u64,
    /// The edits are in the project now.
    applied: bool,
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
            drc_settings_changed: false,
            library_time: Duration::ZERO,
            busy: false,
            slow: Duration::from_millis(4),
            read_only: None,
            text_edit: None,
            sim: SimOverlay::default(),
            route_preview: None,
            drag_shove: false,
            drag_base: None,
        };
        doc.update_connectivity(false);
        doc
    }

    /// Schematic connectivity as of the last [`update_connectivity`](Self::update_connectivity).
    pub fn connectivity(&self) -> &Connectivity {
        &self.connectivity
    }

    /// Whether [`connectivity`](Self::connectivity) is up to date with the schematic.
    pub fn is_connectivity_current(&self) -> bool {
        !self.connectivity_stale
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
                Some(t) => (
                    t.clone(),
                    crate::style::with_editor(crate::style::Editor::Schematic, || crate::style::pal().sch_text),
                ),
                None => return false,
            },
            Some(Content::Footprint(f)) => match f.texts.get(index) {
                Some(t) => {
                    let layer = t.board_layer(model::Side::Top);
                    let color =
                        crate::style::with_editor(crate::style::Editor::Pcb, || crate::style::board_layer_color(layer));
                    (t.text.clone(), color)
                }
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

    /// The route preview, if nothing but its own steps changed the document since.
    fn live_preview(&mut self) -> Option<&mut RoutePreview> {
        let revision = self.revision;
        if self.route_preview.as_ref().is_some_and(|p| p.revision != revision) {
            self.route_preview = None;
        }
        self.route_preview.as_mut()
    }

    /// The board connectivity the router works with: before a route preview's edits.
    pub fn route_copper(&mut self) -> &BoardConnectivity {
        if self.live_preview().is_some() {
            return &self.route_preview.as_ref().expect("live").base;
        }
        self.current_copper()
    }

    /// The project and the router's board connectivity (call
    /// [`route_copper`](Self::route_copper) first to bring it up to date).
    pub fn route_base(&self) -> (&Project, &BoardConnectivity) {
        let copper = self.route_preview.as_ref().map_or(&self.copper, |p| &p.base);
        (self.editor.project(), copper)
    }

    /// The revision the router's board is at (a route preview's own steps don't count).
    pub fn route_revision(&mut self) -> u64 {
        match self.live_preview() {
            Some(p) => p.base_revision,
            None => self.revision,
        }
    }

    /// Take a route preview's edits out of the project (the router works on the board as
    /// it was); the preview's base stays for the next step.
    pub fn clear_route_preview(&mut self) {
        let Some(p) = self.live_preview() else { return };
        if !p.applied {
            return;
        }
        p.applied = false;
        let group = p.group;
        if self.editor.history().open_group() == Some(group) {
            self.retract(group);
        }
        if let Some(p) = self.route_preview.as_mut() {
            p.revision = self.revision;
        }
    }

    /// Show `commands` (the router's pushes) as a route preview, replacing the last one.
    pub fn set_route_preview(&mut self, commands: Vec<Command>) -> Result<(), String> {
        self.clear_route_preview();
        if self.live_preview().is_none() {
            let base = self.current_copper().clone();
            let revision = self.revision;
            self.route_preview = Some(RoutePreview {
                group: crate::interaction::new_group(),
                base,
                base_revision: revision,
                revision,
                applied: false,
            });
        }
        let group = self.route_preview.as_ref().expect("made above").group;
        for (i, c) in commands.into_iter().enumerate() {
            if let Err(e) = self.execute_grouped(c, Some(group)) {
                if i > 0 {
                    self.retract(group);
                }
                if let Some(p) = self.route_preview.as_mut() {
                    p.revision = self.revision;
                }
                return Err(e.to_string());
            }
            if let Some(p) = self.route_preview.as_mut() {
                p.applied = true;
            }
        }
        if let Some(p) = self.route_preview.as_mut() {
            p.revision = self.revision;
        }
        Ok(())
    }

    /// Run `commands` (the leg, a via at its end) with the route preview's edits as one
    /// undo step, and end the preview.
    pub fn commit_route(&mut self, commands: Vec<Command>) -> Result<(), CommandError> {
        let group = match self.live_preview().filter(|p| p.applied).map(|p| p.group) {
            Some(g) => g,
            None => crate::interaction::new_group(),
        };
        let mut result = Ok(());
        for c in commands {
            if let Err(e) = self.execute_grouped(c, Some(group)) {
                result = Err(e);
                break;
            }
        }
        if result.is_err() && self.editor.history().open_group() == Some(group) {
            // Nothing of it stays, the preview's pushes included.
            self.retract(group);
        }
        self.close_group();
        self.route_preview = None;
        result
    }

    /// Move board `ids` by `delta` as a step of the drag with undo group `group`, pushing
    /// aside the copper in the way of the tracks and vias moved. `Err`: something can't
    /// give way (the step is taken back).
    pub fn move_shoving(&mut self, ids: Vec<ObjectId>, delta: Point, group: u64) -> Result<(), String> {
        // Each step starts from the board before the drag (the last step is retracted).
        if self.drag_base.as_ref().is_none_or(|b| b.0 != group) {
            let base = self.current_copper().clone();
            self.drag_base = Some((group, base));
        }
        if self.route_preview.is_some() {
            self.drop_route_preview();
        }
        if let Some(why) = &self.read_only {
            return Err(why.clone());
        }
        // The move and its pushes are one transaction: wires and tracks are tidied after
        // the pushes (tidied before, a track dragged onto another would merge with it).
        let base = &self.drag_base.as_ref().expect("set above").1;
        let result = self.editor.execute_then(
            Command::Move { ids, delta, keep_connected: true },
            Some(group),
            |project, moved| match editor_core::router::shove_moved(project, base, moved) {
                Ok(s) => Ok(s.commands(project)),
                Err(c) => Err(CommandError::Invalid(format!("Can't push aside: {}", c.message))),
            },
        );
        match result {
            Ok(changes) => {
                self.changed(&changes);
                Ok(())
            }
            Err(CommandError::Invalid(m)) => Err(m),
            Err(e) => Err(e.to_string()),
        }
    }

    /// End a route preview, taking its edits out.
    pub fn drop_route_preview(&mut self) {
        self.clear_route_preview();
        self.route_preview = None;
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

    /// Write the schematic as a PDF (a page per sheet, see [`crate::print`]) to
    /// `<project>/output/<name>-schematic.pdf`. Returns its path; an unsaved project has
    /// no folder for it.
    pub fn export_schematic_pdf(&mut self) -> Result<std::path::PathBuf, String> {
        let dir = self.path.clone().ok_or("save the project first: the PDF goes into its output folder")?;
        self.update_connectivity(false);
        let pdf = crate::print::schematic_pdf(self.editor.project(), &self.connectivity);
        let out = dir.join("output");
        std::fs::create_dir_all(&out).map_err(|e| e.to_string())?;
        let file = out.join(format!("{}-schematic.pdf", gerber::stem(self.editor.project())));
        std::fs::write(&file, pdf).map_err(|e| e.to_string())?;
        Ok(file)
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

    /// Redraw both views and their overlays in their editors' palettes (after
    /// [`crate::style::set_theme`]).
    pub fn restyle(&mut self) {
        self.restyle_editor(crate::style::Editor::Schematic);
        self.restyle_editor(crate::style::Editor::Pcb);
        // Every view is redrawn: library changes waiting for theirs are in.
        self.stale_library.clear();
    }

    /// Redraw the views of one editor (the sheets, or the board) with their overlays
    /// (after [`crate::style::set_editor_theme`]).
    pub fn restyle_editor(&mut self, editor: crate::style::Editor) {
        let project = self.editor.project();
        for view in self.sheets.iter_mut().chain([&mut self.board]).filter(|v| v.editor() == editor) {
            let probed = view.probed().to_vec();
            view.rebuild(project);
            view.set_probe(project, &probed);
        }
        match editor {
            crate::style::Editor::Schematic => {
                for view in &mut self.sheets {
                    view.set_connectivity(&self.connectivity);
                }
                self.draw_sim();
            }
            crate::style::Editor::Pcb => self.board.set_copper(project, &self.copper),
            crate::style::Editor::Simulation => {}
        }
    }

    /// Show a simulation's probes and values on the sheets (redrawn when it changes and
    /// with the connectivity).
    pub fn set_sim_overlay(&mut self, o: SimOverlay) {
        if o == self.sim {
            return;
        }
        self.sim = o;
        self.draw_sim();
    }

    pub fn sim_overlay(&self) -> &SimOverlay {
        &self.sim
    }

    fn draw_sim(&mut self) {
        let project = self.editor.project();
        for view in &mut self.sheets {
            view.set_sim(project, &self.connectivity, &self.sim);
        }
    }

    /// Net names on the board near `visible` (the viewport) at `zoom` (pixels per mm);
    /// see [`View::update_net_labels`]. Returns true if they were redrawn.
    pub fn update_net_labels(&mut self, visible: Rect, zoom: f64) -> bool {
        self.board.update_net_labels(self.editor.project(), &self.copper, visible, zoom)
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
            if !self.sim.is_empty() {
                self.draw_sim();
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
        if self.drc_settings_changed && !self.copper_stale {
            self.drc_settings_changed = false;
            if self.drc.is_some() {
                self.run_drc();
                done = true;
            }
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
        self.drop_route_preview();
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
        if self.route_preview.as_ref().is_some_and(|p| Some(p.group) != group) {
            self.drop_route_preview();
        }
        if let Some(why) = &self.read_only {
            return Err(CommandError::Forbidden(why.clone()));
        }
        let command = self.with_class_sizes(command);
        let changes = self.editor.execute_grouped(command, group)?;
        self.changed(&changes);
        Ok(changes)
    }

    /// A track or via added without sizes gets those of the net class of the copper it
    /// starts on (the command alone would take the board's: it doesn't know the nets).
    fn with_class_sizes(&mut self, mut command: Command) -> Command {
        if self.editor.project().board.rules.net_classes.is_empty() {
            return command;
        }
        match &mut command {
            Command::AddTrack { layer, points, width: width @ None, .. } => {
                let net = self
                    .net_at(&points.first().into_iter().chain(points.last()).map(|&p| (*layer, p)).collect::<Vec<_>>());
                *width = net.map(|n| self.editor.project().board.rules.for_net(Some(&n)).track_width);
            }
            Command::AddVia { pos, diameter, drill, span } if diameter.is_none() || drill.is_none() => {
                let [from, to] = span.unwrap_or([LayerId::TOP_COPPER, LayerId::BOTTOM_COPPER]);
                let (lo, hi) = (from.0.min(to.0), from.0.max(to.0));
                let layers: Vec<(LayerId, Point)> = self
                    .editor
                    .project()
                    .board
                    .copper_layers()
                    .filter(|l| (lo..=hi).contains(&l.id.0))
                    .map(|l| (l.id, *pos))
                    .collect();
                if let Some(n) = self.net_at(&layers) {
                    let r = self.editor.project().board.rules.for_net(Some(&n));
                    diameter.get_or_insert(r.via_diameter);
                    drill.get_or_insert(r.via_drill);
                }
            }
            _ => {}
        }
        command
    }

    /// The net of the first copper with a net at one of `points`.
    fn net_at(&mut self, points: &[(LayerId, Point)]) -> Option<String> {
        self.update_connectivity(false);
        points.iter().find_map(|&(l, p)| {
            let c = self.copper.at(p, l)?;
            self.copper.net_of(c.item).map(str::to_string)
        })
    }

    /// See [`Editor::set_library_name`].
    pub fn set_library_name(&mut self, name: String) {
        self.editor.set_library_name(name);
    }

    /// Mark the current state as saved (for documents saved by other means).
    pub fn mark_saved(&mut self) {
        self.editor.mark_saved();
    }

    /// Unsaved changes from the start (a project restored from a recovery copy).
    pub fn mark_unsaved(&mut self) {
        self.editor.mark_unsaved();
    }

    pub fn close_group(&mut self) {
        self.editor.close_group();
    }

    pub fn undo(&mut self) -> Option<ChangeSet> {
        self.drop_route_preview();
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
        self.drop_route_preview();
        let changes = self.editor.redo()?;
        self.changed(&changes);
        Some(changes)
    }

    fn changed(&mut self, changes: &ChangeSet) {
        if changes.is_empty() {
            return;
        }
        self.revision += 1;
        if changes.changes.iter().any(|c| c.affects_connectivity()) || changes.erc_changed() {
            self.connectivity_stale = true;
        }
        if changes.drc_changed() {
            self.drc_settings_changed = true;
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
                view.set_sim(project, &self.connectivity, &self.sim);
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

/// Does dragging `ids` push copper aside (tracks and vias only)?
fn shoves(ids: &[ObjectId]) -> bool {
    !ids.is_empty() && ids.iter().all(|id| matches!(id, ObjectId::Track(_) | ObjectId::Via(_)))
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
        self.doc.view(self.view).select_rect(self.doc.project(), r, crossing)
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
            Edit::Move { ids, delta } => match group {
                Some(g) if keep && self.view.kind() == ViewKind::Board && self.doc.drag_shove && shoves(&ids) => {
                    self.doc.move_shoving(ids, delta, g)?
                }
                _ => self.run(Command::Move { ids, delta, keep_connected: keep }, group)?,
            },
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
