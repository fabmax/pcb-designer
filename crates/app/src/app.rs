//! Main window: menu, side panels, docked schematic / PCB / library editors, status bar.
//!
//! Focus is either a project view (schematic or PCB) or a library tab; the side panels,
//! keyboard shortcuts and Save follow the focus.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use editor_core::Command;
use editor_core::query::{PartState, part_statuses};
use egui::{RichText, WidgetText};
use egui_tiles::{Behavior, Container, SimplificationOptions, Tile, TileId, Tiles, Tree, UiResponse};
use model::*;
use render::gpu::Background;
use view::item::Item;
use view::tools::Tool;
use view::{Document, Interaction, KeyAction, LibraryDoc, ViewKind};

use crate::agent::{self, Agent};
use crate::canvas::{self, Canvas, CanvasResources};
use crate::keymap::{Action, Keymap};
use crate::kicad_import::{ImportAction, KicadImport};
use crate::library::{LibEvent, LibraryEditor, Open};
use crate::properties::Properties;
use crate::theme;

pub enum Startup {
    Open(PathBuf),
    Demo,
    Empty,
    Stress(usize),
}

pub struct Options {
    pub startup: Startup,
    /// Save a screenshot of the whole window to this file and exit (development aid).
    pub screenshot: Option<String>,
    /// Frames to render before taking the screenshot.
    pub screenshot_frames: u32,
    /// Select everything in the schematic before the screenshot (shows highlights).
    pub select: Option<String>,
    pub keymap: Keymap,
    /// Start the MCP server on this port.
    pub mcp: Option<u16>,
    /// Library files to open.
    pub library: Vec<PathBuf>,
    /// Open this part, symbol or footprint (by name) of the last library.
    pub show: Vec<String>,
    /// KiCad library folders (default: the KiCad installation).
    pub kicad_paths: Vec<PathBuf>,
    /// KiCad symbols (`Lib:Name`) and footprints (`footprint:Lib:Name`) to pick for import.
    pub kicad_pick: Vec<String>,
    /// Import the picks at startup (otherwise the import window opens with them).
    pub kicad_import: bool,
    /// Open the import window with this search.
    pub kicad_query: Option<String>,
}

const SCH_GRIDS: &[(f64, &str)] = &[(2.54, "2.54 mm"), (1.27, "1.27 mm"), (0.635, "0.635 mm")];
const BOARD_GRIDS: &[(f64, &str)] =
    &[(1.0, "1 mm"), (0.5, "0.5 mm"), (0.25, "0.25 mm"), (0.1, "0.1 mm"), (0.05, "0.05 mm"), (0.635, "25 mil")];

/// Content of a dock tab.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Tab {
    View(ViewKind),
    /// Library editor, by [`LibraryEditor::key`].
    Library(u64),
}

/// One editor pane: camera + interaction state for a view of the document.
pub(crate) struct Pane {
    pub kind: ViewKind,
    pub canvas: Canvas,
    pub it: Interaction<ObjectId>,
}

impl Pane {
    fn new(kind: ViewKind) -> Self {
        let (id, grid) = match kind {
            ViewKind::Schematic => (1, 1.27),
            ViewKind::Board => (2, 0.5),
        };
        let canvas = Canvas::new(id, Background { grid: grid as f32, ..Default::default() });
        Pane { kind, canvas, it: Interaction::new(mm(grid)) }
    }

    fn title(&self) -> &'static str {
        match self.kind {
            ViewKind::Schematic => "Schematic",
            ViewKind::Board => "PCB",
        }
    }
}

pub struct PcbApp {
    tree: Tree<Tab>,
    doc: Document,
    panes: [Pane; 2],
    /// Project pane receiving keyboard shortcuts when no library has focus.
    active: ViewKind,
    /// Library tab with focus (`None`: the project has focus).
    focus_lib: Option<u64>,
    hovered: Option<ViewKind>,
    properties: Properties,
    libraries: Vec<LibraryEditor>,
    next_key: u64,
    /// Library tabs whose close button was pressed.
    close_requests: Vec<u64>,
    status: Option<(String, Instant)>,
    title: String,
    revision: u64,
    screenshot: Option<(String, u32)>,
    allow_close: bool,
    keymap: Keymap,
    show_keymap: bool,
    agent: Agent,
    kicad: KicadImport,
}

impl PcbApp {
    pub fn new(cc: &eframe::CreationContext<'_>, opts: Options) -> Self {
        theme::apply(&cc.egui_ctx);
        let rs = cc.wgpu_render_state.as_ref().expect("wgpu renderer");
        CanvasResources::install(rs);

        let mut tiles = Tiles::default();
        let sch = tiles.insert_pane(Tab::View(ViewKind::Schematic));
        let pcb = tiles.insert_pane(Tab::View(ViewKind::Board));
        let left = tiles.insert_tab_tile(vec![sch]);
        let right = tiles.insert_tab_tile(vec![pcb]);
        let root = tiles.insert_horizontal_tile(vec![left, right]);
        let tree = Tree::new(egui::Id::unique("main_tree"), root, tiles);

        let mut status = None;
        let doc = match opts.startup {
            Startup::Open(path) => Document::open(&path).unwrap_or_else(|e| {
                status = Some((format!("Could not open {}: {e}", path.display()), Instant::now()));
                Document::new(Project::new("Untitled"))
            }),
            Startup::Demo => Document::new(view::demo::demo_project()),
            Startup::Empty => Document::new(Project::new("Untitled")),
            Startup::Stress(n) => {
                let t = Instant::now();
                let doc = Document::new(view::demo::stress_project(n));
                log::info!("generated {n} components in {:?}", t.elapsed());
                doc
            }
        };
        let mut app = Self {
            tree,
            doc,
            panes: [Pane::new(ViewKind::Schematic), Pane::new(ViewKind::Board)],
            active: ViewKind::Schematic,
            focus_lib: None,
            hovered: None,
            properties: Properties::default(),
            libraries: Vec::new(),
            next_key: 0,
            close_requests: Vec::new(),
            status,
            title: String::new(),
            revision: u64::MAX,
            screenshot: opts.screenshot.map(|p| (p, opts.screenshot_frames)),
            allow_close: false,
            keymap: opts.keymap,
            show_keymap: false,
            agent: Agent::new(opts.mcp.unwrap_or(agent::DEFAULT_PORT)),
            kicad: KicadImport::new(opts.kicad_paths.clone()),
        };
        // The starter library is always available (built in, saved only with "Save as").
        app.add_library(LibraryDoc::new(builtin::starter_library()), false);
        for path in &opts.library {
            match LibraryDoc::open(path) {
                Ok(lib) => {
                    app.add_library(lib, false);
                }
                Err(e) => app.set_status(format!("Could not open library {}: {e}", path.display())),
            }
        }
        for name in &opts.kicad_pick {
            match name.strip_prefix("footprint:") {
                Some(f) => app.kicad.pick_footprint(f),
                None => app.kicad.pick_symbol(name),
            }
        }
        if let Some(q) = &opts.kicad_query {
            let (fp, q) = q.strip_prefix("footprint:").map_or((false, q.as_str()), |q| (true, q));
            app.kicad.set_query(q, fp);
            app.kicad.open = true;
        }
        if app.kicad.has_picks() {
            if opts.kicad_import {
                let req = app.kicad.pending_request();
                app.run_kicad_import(None, req);
            } else {
                app.kicad.open = true;
            }
        }
        for name in &opts.show {
            app.show_by_name(name);
        }
        if opts.mcp.is_some() {
            app.agent.start(&cc.egui_ctx);
        }
        // Development aid for screenshots: "all" or a reference designator.
        match opts.select.as_deref() {
            Some("all") => app.panes[0].it.key(&mut app.doc.target(ViewKind::Schematic), KeyAction::SelectAll),
            Some(refdes) => {
                let p = app.doc.project();
                let ids: Vec<ObjectId> = p
                    .schematic
                    .component_by_refdes(refdes)
                    .map(|c| p.schematic.symbols_of(c.id).map(|s| ObjectId::Symbol(s.id)).collect())
                    .unwrap_or_default();
                app.panes[0].it.set_selection(&mut app.doc.target(ViewKind::Schematic), ids);
            }
            None => {}
        }
        app
    }

    fn pane(&mut self, kind: ViewKind) -> &mut Pane {
        &mut self.panes[kind as usize]
    }

    fn set_status(&mut self, msg: impl Into<String>) {
        self.status = Some((msg.into(), Instant::now()));
    }

    fn busy(&self) -> bool {
        self.panes.iter().any(|p| !p.it.is_idle()) || self.libraries.iter().any(|l| !l.it.is_idle())
    }

    fn lib(&mut self, key: u64) -> Option<&mut LibraryEditor> {
        self.libraries.iter_mut().find(|l| l.key == key)
    }

    fn focused_lib(&mut self) -> Option<&mut LibraryEditor> {
        let key = self.focus_lib?;
        self.lib(key)
    }

    // ------------------------------------------------------------------------------------
    // Project files

    fn replace_document(&mut self, doc: Document) {
        self.doc = doc;
        for kind in [ViewKind::Schematic, ViewKind::Board] {
            let pane = self.pane(kind);
            pane.it = Interaction::new(pane.it.grid);
            pane.canvas.request_fit();
        }
        self.properties = Properties::default();
    }

    /// Ask to save unsaved changes. Returns false if the user cancelled.
    fn confirm_discard(&mut self) -> bool {
        if !self.doc.is_modified() {
            return true;
        }
        match ask_save(&format!("Save changes to \"{}\"?", self.doc.project().name)) {
            rfd::MessageDialogResult::Yes => self.save(false),
            rfd::MessageDialogResult::No => true,
            _ => false,
        }
    }

    fn new_document(&mut self) {
        if self.confirm_discard() {
            self.replace_document(Document::new(Project::new("Untitled")));
        }
    }

    fn open(&mut self) {
        if !self.confirm_discard() {
            return;
        }
        let Some(dir) = rfd::FileDialog::new().set_title("Open project folder").pick_folder() else { return };
        match Document::open(&dir) {
            Ok(doc) => {
                self.replace_document(doc);
                self.set_status(format!("Opened {}", dir.display()));
            }
            Err(e) => self.set_status(format!("Open failed: {e}")),
        }
    }

    /// Save (or Save As when `choose` or the document has no folder yet).
    fn save(&mut self, choose: bool) -> bool {
        let path = match (&self.doc.path, choose) {
            (Some(p), false) => p.clone(),
            _ => {
                let name = format!("{}.pcbproj", self.doc.project().name);
                match rfd::FileDialog::new().set_title("Save project folder").set_file_name(name).save_file() {
                    Some(p) => p,
                    None => return false,
                }
            }
        };
        match self.doc.save_as(&path) {
            Ok(()) => {
                self.set_status(format!("Saved {}", path.display()));
                true
            }
            Err(e) => {
                self.set_status(format!("Save failed: {e}"));
                false
            }
        }
    }

    // ------------------------------------------------------------------------------------
    // Libraries

    /// Add a library editor tab (next to the schematic) and optionally focus it.
    fn add_library(&mut self, lib: LibraryDoc, focus: bool) -> u64 {
        let key = self.next_key;
        self.next_key += 1;
        self.libraries.push(LibraryEditor::new(key, lib));
        let tile = self.tree.tiles.insert_pane(Tab::Library(key));
        let parent = self
            .tree
            .tiles
            .find_pane(&Tab::View(ViewKind::Schematic))
            .and_then(|sch| self.tree.tiles.parent_of(sch))
            .or(self.tree.root);
        if let Some(Tile::Container(c)) = parent.and_then(|p| self.tree.tiles.get_mut(p)) {
            c.add_child(tile);
            if focus && let Container::Tabs(tabs) = c {
                tabs.set_active(tile);
            }
        }
        if focus {
            self.focus_lib = Some(key);
        }
        key
    }

    fn focus_library_tab(&mut self, key: u64) {
        if let Some(tile) = self.tree.tiles.find_pane(&Tab::Library(key)) {
            self.tree.make_active(|id, _| id == tile);
        }
        self.focus_lib = Some(key);
    }

    fn new_library(&mut self) {
        let mut lib = LibraryDoc::new(Library::new("New library"));
        lib.doc.mark_saved();
        self.add_library(lib, true);
    }

    fn open_library(&mut self) {
        let Some(path) =
            rfd::FileDialog::new().set_title("Open library").add_filter("PCB library", &["pcblib"]).pick_file()
        else {
            return;
        };
        if let Some(l) = self.libraries.iter().find(|l| l.path() == Some(&path)) {
            let key = l.key;
            return self.focus_library_tab(key);
        }
        match LibraryDoc::open(&path) {
            Ok(lib) => {
                self.add_library(lib, true);
                self.set_status(format!("Opened library {}", path.display()));
            }
            Err(e) => self.set_status(format!("Could not open library: {e}")),
        }
    }

    fn save_library(&mut self, key: u64, choose: bool) -> bool {
        let Some(l) = self.lib(key) else { return false };
        let path = match (l.path().cloned(), choose) {
            (Some(p), false) => p,
            _ => {
                let name = format!("{}.pcblib", l.lib.name());
                match rfd::FileDialog::new()
                    .set_title("Save library")
                    .add_filter("PCB library", &["pcblib"])
                    .set_file_name(name)
                    .save_file()
                {
                    Some(p) => p,
                    None => return false,
                }
            }
        };
        let result = l.lib.save_as(&path);
        match result {
            Ok(()) => {
                self.set_status(format!("Saved library {}", path.display()));
                true
            }
            Err(e) => {
                self.set_status(format!("Save failed: {e}"));
                false
            }
        }
    }

    fn confirm_discard_library(&mut self, key: u64) -> bool {
        let Some(l) = self.lib(key) else { return true };
        if !l.lib.is_modified() {
            return true;
        }
        match ask_save(&format!("Save changes to library \"{}\"?", l.lib.name())) {
            rfd::MessageDialogResult::Yes => self.save_library(key, false),
            rfd::MessageDialogResult::No => true,
            _ => false,
        }
    }

    fn close_library(&mut self, key: u64) {
        if !self.confirm_discard_library(key) {
            return;
        }
        if let Some(tile) = self.tree.tiles.find_pane(&Tab::Library(key)) {
            self.tree.remove_recursively(tile);
        }
        self.libraries.retain(|l| l.key != key);
        if self.focus_lib == Some(key) {
            self.focus_lib = None;
        }
    }

    /// Open a part, symbol or footprint by name in the last library (development aid).
    fn show_by_name(&mut self, name: &str) {
        let Some(l) = self.libraries.last_mut() else { return };
        let lib = l.lib.library();
        // "symbol:NAME" / "footprint:NAME" / "part:NAME", or the first match of NAME.
        let (kind, name) = name.split_once(':').unwrap_or(("", name));
        let part = lib.parts.iter().find(|p| p.name == name).filter(|_| matches!(kind, "" | "part"));
        let symbol = lib.symbols.iter().find(|s| s.name == name).filter(|_| matches!(kind, "" | "symbol"));
        let footprint = lib.footprints.iter().find(|f| f.name == name).filter(|_| matches!(kind, "" | "footprint"));
        let open = if let Some(p) = part {
            Open::Part(p.id)
        } else if let Some(s) = symbol {
            Open::Item(Item::Symbol(s.id))
        } else if let Some(f) = footprint {
            Open::Item(Item::Footprint(f.id))
        } else {
            return;
        };
        l.show(open);
        let key = l.key;
        self.focus_library_tab(key);
    }

    /// Import from KiCad into library `target` (or a new library named `new_name`).
    /// Returns the library key, what was imported and the (added, replaced) counts.
    fn kicad_import_into(
        &mut self,
        target: Option<u64>,
        new_name: &str,
        req: &kicad::Request,
    ) -> Result<(u64, kicad::Imported, usize, usize), String> {
        let imported = self.kicad.import(req);
        if imported.symbols.is_empty() && imported.footprints.is_empty() {
            return Err(if imported.errors.is_empty() {
                "nothing to import".into()
            } else {
                imported.errors.join("; ")
            });
        }
        let key = match target.filter(|k| self.libraries.iter().any(|l| l.key == *k)) {
            Some(k) => k,
            None => {
                let taken = |n: &str| self.libraries.iter().any(|l| l.lib.name() == n);
                let name = std::iter::once(new_name.to_string())
                    .chain((2..).map(|i| format!("{new_name} {i}")))
                    .find(|n| !taken(n))
                    .unwrap();
                let mut lib = LibraryDoc::new(Library::new(name));
                lib.doc.mark_saved();
                self.add_library(lib, false)
            }
        };
        let first = if let Some(p) = imported.parts.first() {
            Open::Part(p.id)
        } else if let Some(s) = imported.symbols.first() {
            Open::Item(Item::Symbol(s.id))
        } else if let Some(f) = imported.footprints.first() {
            Open::Item(Item::Footprint(f.id))
        } else {
            Open::None
        };
        let l = self.lib(key).ok_or("library closed")?;
        let (added, replaced) =
            l.lib.upsert(imported.symbols.clone(), imported.footprints.clone(), imported.parts.clone())?;
        l.refresh();
        if first != Open::None {
            l.show(first);
        }
        Ok((key, imported, added, replaced))
    }

    /// Import from the KiCad window.
    fn run_kicad_import(&mut self, target: Option<u64>, req: kicad::Request) {
        match self.kicad_import_into(target, "KiCad import", &req) {
            Ok((key, imported, added, replaced)) => {
                let name = self.lib(key).map(|l| l.lib.name().to_string()).unwrap_or_default();
                self.kicad.target = Some(key);
                self.kicad.finish(&imported, &name, added, replaced);
                self.set_status(format!("Imported from KiCad into {name}"));
                self.focus_library_tab(key);
            }
            Err(e) => self.kicad.fail(e),
        }
    }

    /// Answer an agent's KiCad import.
    fn agent_kicad_import(&mut self, job: mcp::host::KicadImport) -> Result<serde_json::Value, String> {
        let mut req = kicad::Request { symbols: Vec::new(), footprints: job.footprints };
        for (symbol, fps) in job.symbols {
            let fps = match fps {
                Some(f) => f,
                None => self.kicad.suggested(&symbol)?,
            };
            req.symbols.push((symbol, fps));
        }
        let target =
            job.library.as_deref().and_then(|n| self.libraries.iter().find(|l| l.lib.name() == n)).map(|l| l.key);
        let target = target.or_else(|| self.libraries.iter().find(|l| l.lib.name() == "KiCad import").map(|l| l.key));
        let new_name = job.library.as_deref().unwrap_or("KiCad import");
        let (key, imported, added, replaced) = self.kicad_import_into(target, new_name, &req)?;
        let library = self.lib(key).map(|l| l.lib.name().to_string()).unwrap_or_default();
        let parts: Vec<serde_json::Value> = imported
            .parts
            .iter()
            .map(|p| {
                let variants: Vec<serde_json::Value> = p
                    .variants
                    .iter()
                    .map(|v| serde_json::json!({ "id": v.id, "footprint": v.name, "connected_pads": v.connections.len() }))
                    .collect();
                serde_json::json!({ "id": p.id, "name": p.name, "units": p.units.len(), "variants": variants })
            })
            .collect();
        let footprints: Vec<&str> = imported.footprints.iter().map(|f| f.name.as_str()).collect();
        let symbols_only: Vec<&str> = req.symbols.iter().filter(|s| s.1.is_empty()).map(|s| s.0.as_str()).collect();
        Ok(serde_json::json!({
            "library": library,
            "added": added,
            "updated": replaced,
            "parts": parts,
            "footprints": footprints,
            "symbols_without_part": symbols_only,
            "notes": imported.warnings,
            "errors": imported.errors,
        }))
    }

    fn library_refs(&self) -> Vec<&Library> {
        self.libraries.iter().map(|l| l.lib.library()).collect()
    }

    /// Update project parts from their libraries (`(library index, part)`).
    fn update_parts(&mut self, parts: &[(usize, Id)]) {
        let mut by_lib: Vec<(usize, Vec<Id>)> = Vec::new();
        for &(lib, part) in parts {
            match by_lib.iter_mut().find(|(l, _)| *l == lib) {
                Some((_, v)) => v.push(part),
                None => by_lib.push((lib, vec![part])),
            }
        }
        let mut updated = 0;
        for (lib, parts) in by_lib {
            let library = self.libraries[lib].lib.library().clone();
            let n = parts.len();
            match self.doc.execute(Command::UpdateParts { library, parts }) {
                Ok(_) => updated += n,
                Err(e) => return self.set_status(format!("Update failed: {e}")),
            }
        }
        self.set_status(format!("Updated {updated} part(s) from library"));
    }

    // ------------------------------------------------------------------------------------
    // Editing

    fn undo(&mut self) {
        if self.busy() {
            return;
        }
        match self.doc.editor().history().undo_label().map(str::to_string) {
            Some(label) => {
                self.doc.undo();
                self.set_status(format!("Undo: {label}"));
            }
            None => self.set_status("Nothing to undo"),
        }
    }

    fn redo(&mut self) {
        if self.busy() {
            return;
        }
        match self.doc.editor().history().redo_label().map(str::to_string) {
            Some(label) => {
                self.doc.redo();
                self.set_status(format!("Redo: {label}"));
            }
            None => self.set_status("Nothing to redo"),
        }
    }

    /// Place a part at the center of the schematic view, importing it from library
    /// `lib_key` if the project doesn't have it yet.
    fn place_part(&mut self, lib_key: u64, part: Id) {
        if !self.doc.project().library.parts.contains(part) {
            let Some(l) = self.lib(lib_key) else { return };
            let issues = l.lib.library().validate();
            if !issues.is_empty() {
                return self.set_status(format!("Library has problems: {}", issues[0]));
            }
            let library = l.lib.library().clone();
            if let Err(e) = self.doc.execute(Command::ImportParts { library, parts: vec![part] }) {
                return self.set_status(format!("Import failed: {e}"));
            }
        }
        let pane = self.pane(ViewKind::Schematic);
        let c = pane.canvas.camera.center;
        let pos = pane.it.snap(Point::mm(c[0], c[1]));
        let cmd = Command::PlacePart {
            part,
            variant: None,
            sheet: None,
            pos,
            rotation: Angle::ZERO,
            mirror: false,
            refdes: None,
            value: None,
        };
        match self.doc.execute(cmd) {
            Ok(changes) => {
                let symbols: Vec<ObjectId> = changes
                    .changes
                    .iter()
                    .filter(|c| c.is_insert() && matches!(c.id, ObjectId::Symbol(_)))
                    .map(|c| c.id)
                    .collect();
                self.active = ViewKind::Schematic;
                self.focus_lib = None;
                if let Some(tile) = self.tree.tiles.find_pane(&Tab::View(ViewKind::Schematic)) {
                    self.tree.make_active(|id, _| id == tile);
                }
                let pane = &mut self.panes[0];
                pane.it.set_selection(&mut self.doc.target(ViewKind::Schematic), symbols);
            }
            Err(e) => self.set_status(format!("Place failed: {e}")),
        }
    }

    fn run_action(&mut self, action: Action) {
        if let Some(key) = self.focus_lib
            && self.lib_action(key, action)
        {
            return;
        }
        let key_action = match action {
            Action::New => return self.new_document(),
            Action::Open => return self.open(),
            Action::Save => {
                self.save(false);
                return;
            }
            Action::SaveAs => {
                self.save(true);
                return;
            }
            Action::Undo => return self.undo(),
            Action::Redo => return self.redo(),
            Action::FitView => return self.pane(self.active).canvas.request_fit(),
            Action::SelectAll => KeyAction::SelectAll,
            Action::Rotate => KeyAction::Rotate,
            Action::Mirror => KeyAction::Mirror,
            Action::Flip => KeyAction::Flip,
            Action::Delete => KeyAction::Delete,
            Action::Cancel => KeyAction::Escape,
            Action::ToolSelect
            | Action::ToolPin
            | Action::ToolLine
            | Action::ToolRect
            | Action::ToolCircle
            | Action::ToolText
            | Action::NavigateBack
            | Action::NavigateForward => return,
        };
        let kind = self.active;
        let pane = &mut self.panes[kind as usize];
        pane.it.key(&mut self.doc.target(kind), key_action);
    }

    /// Handle an action for a focused library. Returns false for project-level actions.
    fn lib_action(&mut self, key: u64, action: Action) -> bool {
        match action {
            Action::New | Action::Open => return false,
            Action::Save | Action::SaveAs => {
                self.save_library(key, action == Action::SaveAs);
                return true;
            }
            _ => {}
        }
        let Some(l) = self.lib(key) else { return false };
        let msg = match action {
            Action::Undo => l.undo(),
            Action::Redo => l.redo(),
            Action::FitView => {
                l.canvas.request_fit();
                None
            }
            Action::SelectAll => key_msg(l, KeyAction::SelectAll),
            Action::Rotate => key_msg(l, KeyAction::Rotate),
            Action::Mirror | Action::Flip => key_msg(l, KeyAction::Mirror),
            Action::Delete => key_msg(l, KeyAction::Delete),
            Action::Cancel => key_msg(l, KeyAction::Escape),
            Action::ToolSelect => tool_msg(l, Tool::Select),
            Action::ToolPin => tool_msg(l, Tool::Pin),
            Action::ToolLine => tool_msg(l, Tool::Line),
            Action::ToolRect => tool_msg(l, Tool::Rect),
            Action::ToolCircle => tool_msg(l, Tool::Circle),
            Action::ToolText => tool_msg(l, Tool::Text),
            Action::NavigateBack => {
                if !l.busy() {
                    l.go_back();
                }
                None
            }
            Action::NavigateForward => {
                if !l.busy() {
                    l.go_forward();
                }
                None
            }
            Action::New | Action::Open | Action::Save | Action::SaveAs => None,
        };
        if let Some(m) = msg {
            self.set_status(m);
        }
        true
    }

    /// Menu entry for `action`, showing its current shortcut.
    fn menu_item(&mut self, ui: &mut egui::Ui, label: &str, action: Action, enabled: bool) {
        let shortcut = self.keymap.menu_text(ui.ctx(), action);
        if ui.add_enabled(enabled, egui::Button::new(label).shortcut_text(shortcut)).clicked() {
            self.run_action(action);
        }
    }

    // ------------------------------------------------------------------------------------
    // Panels

    fn menu(&mut self, ui: &mut egui::Ui) {
        egui::MenuBar::new().ui(ui, |ui| {
            ui.menu_button("File", |ui| {
                let lib_focus = self.focus_lib.is_some();
                self.menu_item(ui, "New project", Action::New, true);
                self.menu_item(ui, "Open project…", Action::Open, true);
                if ui.button("Open demo project").clicked() && self.confirm_discard() {
                    self.replace_document(Document::new(view::demo::demo_project()));
                }
                ui.separator();
                if ui.button("New library").clicked() {
                    self.new_library();
                }
                if ui.button("Open library…").clicked() {
                    self.open_library();
                }
                let starter_open = self.libraries.iter().any(|l| l.lib.library().id == Some(builtin::LIBRARY_ID));
                if !starter_open && ui.button("Open starter library").clicked() {
                    self.add_library(LibraryDoc::new(builtin::starter_library()), true);
                }
                if ui.button("Import from KiCad…").clicked() {
                    self.kicad.open = true;
                }
                ui.separator();
                let (save, save_as) = if lib_focus {
                    ("Save library", "Save library as…")
                } else {
                    ("Save project", "Save project as…")
                };
                self.menu_item(ui, save, Action::Save, true);
                self.menu_item(ui, save_as, Action::SaveAs, true);
                ui.separator();
                if ui.button("Quit").clicked() {
                    ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                }
            });
            ui.menu_button("Edit", |ui| {
                let h = match self.focus_lib.and_then(|k| self.libraries.iter().find(|l| l.key == k)) {
                    Some(l) => l.lib.doc.editor().history(),
                    None => self.doc.editor().history(),
                };
                let undo = h.undo_label().map(|l| format!("Undo {l}"));
                let redo = h.redo_label().map(|l| format!("Redo {l}"));
                self.menu_item(ui, undo.as_deref().unwrap_or("Undo"), Action::Undo, undo.is_some());
                self.menu_item(ui, redo.as_deref().unwrap_or("Redo"), Action::Redo, redo.is_some());
                ui.separator();
                for action in [Action::SelectAll, Action::Rotate, Action::Mirror, Action::Flip, Action::Delete] {
                    self.menu_item(ui, action.label(), action, true);
                }
            });
            ui.menu_button("View", |ui| {
                self.menu_item(ui, "Fit view", Action::FitView, true);
                let lib = self.focus_lib.and_then(|k| self.libraries.iter().find(|l| l.key == k));
                let (back, forward) = lib.map_or((false, false), |l| (l.can_go_back(), l.can_go_forward()));
                ui.separator();
                self.menu_item(ui, "Back", Action::NavigateBack, back);
                self.menu_item(ui, "Forward", Action::NavigateForward, forward);
            });
            ui.menu_button("Tools", |ui| self.agent_menu(ui));
            ui.menu_button("Help", |ui| {
                if ui.button("Keyboard shortcuts").clicked() {
                    self.show_keymap = true;
                }
            });
        });
    }

    fn agent_menu(&mut self, ui: &mut egui::Ui) {
        let mut on = self.agent.is_running();
        if ui.checkbox(&mut on, "Agent access (MCP server)").changed() {
            if on {
                self.agent.start(ui.ctx());
            } else {
                self.agent.stop();
            }
        }
        if let Some(e) = &self.agent.error {
            ui.label(RichText::new(e).color(theme::WARN));
        }
        if let Some(url) = self.agent.url() {
            ui.label(RichText::new(&url).monospace().color(theme::TEXT_DIM));
            if ui.button("Copy URL").clicked() {
                ui.ctx().copy_text(url.clone());
            }
            if ui.button("Copy Claude Code command").clicked() {
                ui.ctx().copy_text(format!("claude mcp add --transport http pcb-designer {url}"));
            }
            if ui.button("Copy JSON config").clicked() {
                let cfg = format!(
                    "{{\n  \"mcpServers\": {{\n    \"pcb-designer\": {{ \"type\": \"http\", \"url\": \"{url}\" }}\n  }}\n}}"
                );
                ui.ctx().copy_text(cfg);
            }
        } else {
            ui.label(RichText::new(format!("Listens on 127.0.0.1:{} only", self.agent.port)).color(theme::TEXT_DIM));
        }
    }

    /// Read-only list of the current bindings (editing comes with the settings UI).
    fn keymap_window(&mut self, ctx: &egui::Context) {
        let mut open = self.show_keymap;
        egui::Window::new("Keyboard shortcuts").open(&mut open).resizable(false).collapsible(false).show(ctx, |ui| {
            egui::Grid::new("keymap").num_columns(2).striped(true).spacing([24.0, 4.0]).show(ui, |ui| {
                for &action in Action::ALL {
                    ui.label(action.label());
                    let keys: Vec<String> =
                        self.keymap.shortcuts(action).iter().map(|s| ctx.format_shortcut(&s.to_egui())).collect();
                    ui.monospace(keys.join("  "));
                    ui.end_row();
                }
            });
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if ui.button("Copy as JSON").clicked() {
                    ui.ctx().copy_text(self.keymap.to_json());
                }
                ui.label(RichText::new("Edit and load with --keymap FILE.json").color(theme::TEXT_DIM));
            });
        });
        self.show_keymap = open;
    }

    fn left_panel(&mut self, ui: &mut egui::Ui) {
        if let Some(key) = self.focus_lib {
            let event = self.lib(key).and_then(|l| l.browser_ui(ui, true));
            match event {
                Some(LibEvent::Place(part)) => self.place_part(key, part),
                Some(LibEvent::Save) => {
                    self.save_library(key, false);
                }
                Some(LibEvent::SaveAs) => {
                    self.save_library(key, true);
                }
                Some(LibEvent::Close) => self.close_requests.push(key),
                None => {}
            }
            return;
        }
        ui.add_space(6.0);
        self.libraries_section(ui);
        self.project_parts_section(ui);

        let kind = self.active;
        let pane = &mut self.panes[kind as usize];
        section(ui, "Grid", |ui| {
            let grids = if kind == ViewKind::Schematic { SCH_GRIDS } else { BOARD_GRIDS };
            let current = to_mm(pane.it.grid);
            let label =
                grids.iter().find(|g| (g.0 - current).abs() < 1e-9).map_or(format!("{current} mm"), |g| g.1.into());
            egui::ComboBox::from_id_salt("grid").selected_text(label).show_ui(ui, |ui| {
                for &(g, name) in grids {
                    if ui.selectable_label((g - current).abs() < 1e-9, name).clicked() {
                        pane.it.grid = mm(g);
                        pane.canvas.background.grid = g as f32;
                    }
                }
            });
            ui.checkbox(&mut pane.canvas.background.show_grid, "Show grid");
        });

        let title = format!("{} layers", pane.title());
        section(ui, &title, |ui| {
            let range = self.doc.view(kind).user_layers();
            let scene = &mut self.doc.view_mut(kind).scene;
            for id in range {
                let layer = scene.layer_mut(id);
                let name = layer.name.clone();
                ui.checkbox(&mut layer.visible, name);
            }
        });
    }

    /// Parts of all open libraries, ready to place.
    fn libraries_section(&mut self, ui: &mut egui::Ui) {
        enum Act {
            Place(u64, Id),
            Edit(u64),
            Open,
            New,
            Kicad,
        }
        let mut act = None;
        section(ui, "Libraries", |ui| {
            for l in &self.libraries {
                let lib = l.lib.library();
                let mut parts: Vec<&Part> = lib.parts.iter().collect();
                parts.sort_by(|a, b| natural_cmp(&a.name, &b.name));
                let id = ui.make_persistent_id(("lib_parts", l.key));
                egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id, true)
                    .show_header(ui, |ui| {
                        ui.label(RichText::new(l.title()).color(theme::TEXT_DIM));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.small_button("Edit").on_hover_text("Open in the library editor").clicked() {
                                act = Some(Act::Edit(l.key));
                            }
                        });
                    })
                    .body(|ui| {
                        for part in parts {
                            ui.horizontal(|ui| {
                                if ui.small_button("Place").clicked() {
                                    act = Some(Act::Place(l.key, part.id));
                                }
                                let r = ui.label(RichText::new(&part.name).strong());
                                if !part.description.is_empty() {
                                    r.on_hover_text(&part.description);
                                }
                            });
                        }
                    });
            }
            ui.horizontal(|ui| {
                if ui.button("Open library…").clicked() {
                    act = Some(Act::Open);
                }
                if ui.button("New library").clicked() {
                    act = Some(Act::New);
                }
            });
            if ui.button("Import from KiCad…").clicked() {
                act = Some(Act::Kicad);
            }
        });
        match act {
            Some(Act::Place(key, part)) => self.place_part(key, part),
            Some(Act::Edit(key)) => self.focus_library_tab(key),
            Some(Act::Open) => self.open_library(),
            Some(Act::New) => self.new_library(),
            Some(Act::Kicad) => self.kicad.open = true,
            None => {}
        }
    }

    /// Parts embedded in the project and whether their library has a newer version.
    fn project_parts_section(&mut self, ui: &mut egui::Ui) {
        let statuses = part_statuses(self.doc.project(), &self.library_refs());
        if statuses.is_empty() {
            return;
        }
        let changed: Vec<(usize, Id)> = statuses
            .iter()
            .filter(|s| s.state == PartState::LibraryChanged)
            .filter_map(|s| Some((s.library?, s.part)))
            .collect();
        let mut update = None;
        section(ui, &format!("Project parts ({})", statuses.len()), |ui| {
            for s in &statuses {
                ui.horizontal(|ui| {
                    let (text, color, tip) = match s.state {
                        PartState::UpToDate => ("current", theme::TEXT_DIM, "Same as in the library"),
                        PartState::LibraryChanged => ("changed", theme::WARN, "The library has a different version"),
                        PartState::LibraryMissing => ("lib closed", theme::TEXT_DIM, "Its library is not open"),
                        PartState::RemovedFromLibrary => {
                            ("removed", theme::WARN, "The part no longer exists in its library")
                        }
                        PartState::Local => ("local", theme::TEXT_DIM, "Not from a library"),
                    };
                    ui.label(&s.name).on_hover_text(s.library_name.as_deref().unwrap_or(""));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if s.state == PartState::LibraryChanged
                            && let Some(lib) = s.library
                            && ui.small_button("Update").clicked()
                        {
                            update = Some(vec![(lib, s.part)]);
                        }
                        ui.label(RichText::new(text).color(color).small()).on_hover_text(tip);
                    });
                });
            }
            if !changed.is_empty() && ui.button(format!("Update all ({})", changed.len())).clicked() {
                update = Some(changed.clone());
            }
        });
        if let Some(parts) = update {
            self.update_parts(&parts);
        }
    }

    fn status_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            let lib = self.focus_lib.and_then(|k| self.libraries.iter().find(|l| l.key == k));
            let (title, cursor, grid, selected) = match lib {
                Some(l) => {
                    let what = match l.open {
                        Open::Item(Item::Symbol(_)) => "Symbol editor",
                        Open::Item(Item::Footprint(_)) => "Footprint editor",
                        Open::Part(_) => "Part",
                        Open::None => "Library",
                    };
                    (format!("{what} · {}", l.lib.name()), l.canvas.cursor_world, l.it.grid, l.it.selection.len())
                }
                None => {
                    let pane = &self.panes[self.hovered.unwrap_or(self.active) as usize];
                    let sel = self.panes[self.active as usize].it.selection.len();
                    (pane.title().to_string(), pane.canvas.cursor_world, pane.it.grid, sel)
                }
            };
            ui.label(RichText::new(title).color(theme::ACCENT));
            match cursor {
                Some([x, y]) => ui.monospace(format!("X {x:>9.3}  Y {y:>9.3} mm")),
                None => ui.monospace(" ".repeat(26)),
            };
            ui.separator();
            ui.monospace(format!("grid {} mm", to_mm(grid)));
            ui.separator();
            ui.label(format!("{selected} selected"));
            let msg = self.panes.iter().find_map(|p| p.it.message.clone());
            let status = self.status.as_ref().filter(|(_, t)| t.elapsed() < Duration::from_secs(6));
            if let Some(m) = msg.as_deref().or(status.map(|s| s.0.as_str())) {
                ui.separator();
                ui.label(RichText::new(m).color(theme::WARN));
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if let Some(url) = self.agent.url() {
                    let recent = self.agent.last.as_ref().filter(|(_, t)| t.elapsed() < Duration::from_secs(4));
                    let text = match recent {
                        Some((what, _)) => format!("Agent: {what}"),
                        None => "MCP".to_string(),
                    };
                    let tip = format!("{url}\n{} requests", self.agent.calls);
                    ui.label(RichText::new(text).color(theme::SUCCESS)).on_hover_text(&tip);
                    // Right-to-left layout: the dot ends up left of the text.
                    let (dot, _) = ui.allocate_exact_size(egui::vec2(8.0, 8.0), egui::Sense::hover());
                    ui.painter().circle_filled(dot.center(), 3.5, theme::SUCCESS);
                    if recent.is_some() {
                        ui.ctx().request_repaint_after(Duration::from_millis(500));
                    }
                    ui.separator();
                }
                ui.label(
                    RichText::new("Drag: move · Box: select · Middle/right drag: pan · Wheel: zoom · V: fit")
                        .color(theme::TEXT_DIM),
                );
            });
        });
    }

    fn update_title(&mut self, ctx: &egui::Context) {
        let name = match &self.doc.path {
            Some(p) => p.file_name().map_or(String::new(), |n| n.to_string_lossy().into_owned()),
            None => self.doc.project().name.clone(),
        };
        let title = format!("{}{} — PCB Designer", name, if self.doc.is_modified() { " •" } else { "" });
        if title != self.title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.title = title;
        }
    }

    fn handle_close(&mut self, ctx: &egui::Context) {
        if !ctx.input(|i| i.viewport().close_requested()) || self.allow_close {
            return;
        }
        let modified_libs: Vec<u64> = self.libraries.iter().filter(|l| l.lib.is_modified()).map(|l| l.key).collect();
        if !self.doc.is_modified() && modified_libs.is_empty() {
            return;
        }
        ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
        if !self.confirm_discard() {
            return;
        }
        for key in modified_libs {
            if !self.confirm_discard_library(key) {
                return;
            }
        }
        self.allow_close = true;
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
    }

    fn screenshot_tick(&mut self, ctx: &egui::Context) {
        let Some((path, frames)) = &mut self.screenshot else {
            return;
        };
        ctx.request_repaint();
        if *frames > 0 {
            *frames -= 1;
            return;
        }
        let path = path.clone();
        self.screenshot = None;
        ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::ScreenshotCallback::new(move |img| {
            let rgba = img.pixels.iter().flat_map(|c| c.to_srgba_unmultiplied()).collect();
            let image = render::offscreen::Image { width: img.size[0] as u32, height: img.size[1] as u32, rgba };
            match image.save_png(&path) {
                Ok(()) => println!("screenshot saved to {path}"),
                Err(e) => eprintln!("screenshot failed: {e}"),
            }
            std::process::exit(0);
        })));
    }
}

fn key_msg(l: &mut LibraryEditor, action: KeyAction) -> Option<String> {
    l.key(action);
    None
}

fn tool_msg(l: &mut LibraryEditor, tool: Tool) -> Option<String> {
    l.set_tool(tool);
    None
}

fn ask_save(question: &str) -> rfd::MessageDialogResult {
    rfd::MessageDialog::new()
        .set_level(rfd::MessageLevel::Warning)
        .set_title("Unsaved changes")
        .set_description(question)
        .set_buttons(rfd::MessageButtons::YesNoCancel)
        .show()
}

fn section(ui: &mut egui::Ui, title: &str, add: impl FnOnce(&mut egui::Ui)) {
    egui::CollapsingHeader::new(RichText::new(title).strong()).default_open(true).show(ui, add);
    ui.add_space(4.0);
}

struct TreeBehavior<'a> {
    doc: &'a mut Document,
    panes: &'a mut [Pane; 2],
    libraries: &'a mut [LibraryEditor],
    active: &'a mut ViewKind,
    focus_lib: &'a mut Option<u64>,
    hovered: &'a mut Option<ViewKind>,
    close_requests: &'a mut Vec<u64>,
}

impl TreeBehavior<'_> {
    fn view_ui(&mut self, ui: &mut egui::Ui, kind: ViewKind) {
        let pane = &mut self.panes[kind as usize];
        let response = pane.canvas.ui(ui, &mut self.doc.view_mut(kind).scene);
        if response.hovered() {
            *self.hovered = Some(kind);
        }
        if response.hovered() && ui.input(|i| i.pointer.any_pressed()) {
            *self.active = kind;
            *self.focus_lib = None;
        }
        let engaged = !pane.it.is_idle();
        if let Some(input) = canvas::pointer_input(ui, &response, &pane.canvas, engaged, pane.it.hover.is_some()) {
            pane.it.pointer(&mut self.doc.target(kind), &input);
        }
        if pane.it.is_dragging() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
        } else if pane.it.hover.is_some() && response.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        if let Some((r, crossing)) = pane.it.box_rect() {
            canvas::draw_box(ui, &pane.canvas, response.rect, r, crossing);
        }
        if !pane.it.is_idle() {
            ui.ctx().request_repaint();
        }
    }

    fn tab_of(tiles: &Tiles<Tab>, tile: TileId) -> Option<Tab> {
        match tiles.get(tile) {
            Some(Tile::Pane(tab)) => Some(*tab),
            _ => None,
        }
    }
}

impl Behavior<Tab> for TreeBehavior<'_> {
    fn tab_title_for_pane(&mut self, tab: &Tab) -> WidgetText {
        match tab {
            Tab::View(kind) => self.panes[*kind as usize].title().into(),
            Tab::Library(key) => {
                let title = self.libraries.iter().find(|l| l.key == *key).map_or("Library".into(), |l| l.title());
                format!("Library: {title}").into()
            }
        }
    }

    fn pane_ui(&mut self, ui: &mut egui::Ui, _tile_id: TileId, tab: &mut Tab) -> UiResponse {
        match *tab {
            Tab::View(kind) => self.view_ui(ui, kind),
            Tab::Library(key) => {
                if let Some(l) = self.libraries.iter_mut().find(|l| l.key == key)
                    && l.center_ui(ui)
                {
                    *self.focus_lib = Some(key);
                }
            }
        }
        UiResponse::None
    }

    fn on_tab_button(&mut self, tiles: &mut Tiles<Tab>, tile_id: TileId, response: egui::Response) -> egui::Response {
        if response.clicked() || response.drag_started() {
            match Self::tab_of(tiles, tile_id) {
                Some(Tab::Library(key)) => *self.focus_lib = Some(key),
                Some(Tab::View(kind)) => {
                    *self.active = kind;
                    *self.focus_lib = None;
                }
                None => {}
            }
        }
        response
    }

    fn is_tab_closable(&self, tiles: &Tiles<Tab>, tile_id: TileId) -> bool {
        matches!(Self::tab_of(tiles, tile_id), Some(Tab::Library(_)))
    }

    fn on_tab_close(&mut self, tiles: &mut Tiles<Tab>, tile_id: TileId) -> bool {
        if let Some(Tab::Library(key)) = Self::tab_of(tiles, tile_id) {
            // Closed by the app after asking about unsaved changes.
            self.close_requests.push(key);
        }
        false
    }

    fn simplification_options(&self) -> SimplificationOptions {
        SimplificationOptions { all_panes_must_have_tabs: true, ..Default::default() }
    }

    fn gap_width(&self, _style: &egui::Style) -> f32 {
        2.0
    }

    fn tab_bar_color(&self, _visuals: &egui::Visuals) -> egui::Color32 {
        theme::PANEL_DARK
    }
}

impl eframe::App for PcbApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.handle_close(&ctx);
        for action in self.keymap.triggered(&ctx) {
            self.run_action(action);
        }

        egui::Panel::top("menu").show(ui, |ui| self.menu(ui));
        egui::Panel::bottom("status").show(ui, |ui| self.status_bar(ui));
        egui::Panel::left("library").resizable(true).default_size(240.0).show(ui, |ui| {
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| self.left_panel(ui));
        });
        egui::Panel::right("properties").resizable(true).default_size(300.0).show(ui, |ui| {
            ui.add_space(6.0);
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                if let Some(l) = self.focused_lib() {
                    l.inspector_ui(ui);
                    return;
                }
                let kind = self.active;
                let pane = &mut self.panes[kind as usize];
                let selection = pane.it.selection.clone();
                let mut message = None;
                self.properties.ui(ui, &mut self.doc, &selection, &mut message);
                if let Some(m) = message {
                    self.status = Some((m, Instant::now()));
                }
            });
        });

        self.hovered = None;
        egui::CentralPanel::default().frame(egui::Frame::NONE).show(ui, |ui| {
            let mut behavior = TreeBehavior {
                doc: &mut self.doc,
                panes: &mut self.panes,
                libraries: &mut self.libraries,
                active: &mut self.active,
                focus_lib: &mut self.focus_lib,
                hovered: &mut self.hovered,
                close_requests: &mut self.close_requests,
            };
            self.tree.ui(&mut behavior, ui);
        });

        for key in std::mem::take(&mut self.close_requests) {
            self.close_library(key);
        }
        if let Some(m) = self.libraries.iter_mut().find_map(|l| l.message.take()) {
            self.set_status(m);
        }

        // Agent requests are answered between user interactions.
        if !self.busy() {
            let libraries: Vec<&Library> = self.libraries.iter().map(|l| l.lib.library()).collect();
            let jobs = self.agent.process(&ctx, &mut self.doc, &mut self.panes, &libraries, &mut self.kicad);
            for (job, reply) in jobs {
                let result = self.agent_kicad_import(job).map(mcp::Reply::Json);
                let _ = reply.send(result);
            }
        }

        // Model changed (any source): drop stale selections, refresh highlights.
        if self.doc.revision != self.revision {
            self.revision = self.doc.revision;
            for pane in &mut self.panes {
                pane.it.refresh(&mut self.doc.target(pane.kind));
            }
        }
        self.keymap_window(&ctx);
        let libs: Vec<(u64, String)> = self.libraries.iter().map(|l| (l.key, l.lib.name().to_string())).collect();
        if let Some(ImportAction::Import(target, req)) = self.kicad.ui(&ctx, &libs) {
            self.run_kicad_import(target, req);
        }
        self.update_title(&ctx);
        self.screenshot_tick(&ctx);
    }
}
