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
use view::item::Item;
use view::pcb_tools::{PcbPlacer, PcbTool};
use view::sch_tools::{SchPlacer, SchTool};

use crate::name_prompt::NamePrompt;
use view::project_libs::{self, MissingLibrary};
use view::tools::Tool;
use view::{Document, Interaction, KeyAction, LibraryDoc, ViewId, ViewKind};

use crate::agent::{self, Agent, Job};
use crate::canvas::{self, Canvas, CanvasResources};
use crate::dock::{Dock, Panel, Side, Unseen};
use crate::icons::{self, Icon};
use crate::keymap::{Action, Context, Keymap, Shortcut};
use crate::kicad_import::{ImportAction, KicadImport};
use crate::library::{LibraryEditor, Open, PROJECT_LIB};
use crate::menu::{self, Cmd, Menu, MenuState};
use crate::messages;
use crate::pcb_ui;
use crate::project_panel::{self, Group, NodeId, Owner, ProjectPanel, TreeNode};
use crate::properties::Properties;
use crate::sch_ui;
use crate::script::Script;
use crate::settings::Settings;
use crate::theme;
#[cfg(target_os = "macos")]
use crate::title_bar;
use crate::toolbar;
use crate::units::{self, GridKind};

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
    /// Key bindings from `--keymap` (instead of the saved ones).
    pub keymap: Option<Keymap>,
    /// Start the MCP server on this port.
    pub mcp: Option<u16>,
    /// Library files to add to the project (and show in tabs).
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
    /// Start with schematic and PCB side by side instead of as tabs.
    pub side_by_side: bool,
    /// Panels to open (`messages`, `board_setup`, …; development aid for screenshots).
    pub panels: Vec<String>,
    /// Scripted input, see [`crate::script`].
    pub script: Option<String>,
}

/// The name dialog's state: renaming a library item or a sheet, or naming a new sheet
/// (all from the Project panel).
struct Rename {
    what: Naming,
    text: String,
    focused: bool,
}

#[derive(Clone, Copy)]
enum Naming {
    /// A library item, by library key.
    Item(u64, ObjectId),
    Sheet(Id),
    /// A new sheet below this one.
    NewSheet(Id),
}

/// Content of a dock tab.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Tab {
    /// A schematic sheet or the board.
    View(ViewId),
    /// Library editor, by [`LibraryEditor::key`].
    Library(u64),
}

/// One editor pane: camera + interaction state for a view of the document.
pub(crate) struct Pane {
    pub view: ViewId,
    pub canvas: Canvas,
    pub it: Interaction<ObjectId>,
}

impl Pane {
    fn new(view: ViewId, canvas_id: u64) -> Self {
        let grid = match view.kind() {
            ViewKind::Schematic => 1.27,
            ViewKind::Board => 0.5,
        };
        let canvas = Canvas::new(canvas_id, view::style::background(grid as f32));
        Pane { view, canvas, it: Interaction::new(mm(grid)) }
    }
}

/// The panes of the board and of the schematic sheets (made when first needed). Sheet
/// panes share their grid settings.
pub(crate) struct Panes {
    list: Vec<Pane>,
    next_canvas: u64,
}

impl Panes {
    /// Canvas ids of sheet panes (library canvases use small ones).
    const FIRST_SHEET_CANVAS: u64 = 1 << 32;

    fn new() -> Self {
        Panes { list: vec![Pane::new(ViewId::Board, 2)], next_canvas: Self::FIRST_SHEET_CANVAS }
    }

    /// The pane of `view`, made if there is none yet.
    pub fn get(&mut self, view: ViewId) -> &mut Pane {
        let i = match self.list.iter().position(|p| p.view == view) {
            Some(i) => i,
            None => {
                let mut pane = Pane::new(view, self.next_canvas);
                self.next_canvas += 1;
                if let Some(other) = self.list.iter().find(|p| p.view.kind() == view.kind()) {
                    pane.it.grid = other.it.grid;
                    pane.it.snap = other.it.snap;
                    pane.canvas.background.grid = other.canvas.background.grid;
                }
                self.list.push(pane);
                self.list.len() - 1
            }
        };
        &mut self.list[i]
    }

    pub fn find(&self, view: ViewId) -> Option<&Pane> {
        self.list.iter().find(|p| p.view == view)
    }

    /// The selection in `view`.
    pub fn selection(&self, view: ViewId) -> Vec<ObjectId> {
        self.find(view).map(|p| p.it.selection.clone()).unwrap_or_default()
    }

    pub fn iter(&self) -> impl Iterator<Item = &Pane> {
        self.list.iter()
    }

    pub fn iter_mut(&mut self) -> impl Iterator<Item = &mut Pane> {
        self.list.iter_mut()
    }

    /// Grid settings changed in one schematic pane apply to all.
    fn share_grid(&mut self, from: ViewId) {
        let Some(p) = self.find(from) else { return };
        let (grid, snap, bg) = (p.it.grid, p.it.snap, p.canvas.background.grid);
        for q in self.list.iter_mut().filter(|q| q.view.kind() == from.kind()) {
            q.it.grid = grid;
            q.it.snap = snap;
            q.canvas.background.grid = bg;
        }
    }

    /// Drop the panes of sheets that no longer exist.
    fn retain_sheets(&mut self, project: &Project) {
        self.list.retain(|p| p.view.sheet().is_none_or(|s| project.schematic.sheets.contains(s)));
    }
}

pub struct PcbApp {
    tree: Tree<Tab>,
    /// The project: its document, edited as the Project Library in its tab (key
    /// [`PROJECT_LIB`]) and as schematic and board in the panes.
    project: LibraryEditor,
    panes: Panes,
    /// Project pane receiving keyboard shortcuts when no library has focus.
    active: ViewId,
    /// The sheet shown last (where new sheets and placed parts go when the board is active).
    last_sheet: Option<Id>,
    /// Library tab with focus (`None`: the project has focus).
    focus_lib: Option<u64>,
    hovered: Option<ViewId>,
    properties: Properties,
    /// Loaded libraries: the built-in one, the project's library files (see
    /// [`PcbApp::sync_libraries`]) and unsaved ones. Only some have a tab.
    libraries: Vec<LibraryEditor>,
    /// The project's library files that could not be loaded.
    missing_libraries: Vec<MissingLibrary>,
    /// The project's library list `libraries` was last synced with (`None`: sync again).
    synced_libraries: Option<Vec<PathBuf>>,
    next_key: u64,
    /// Tabs whose close button was clicked (closed after the tree is drawn).
    close_requests: Vec<Tab>,
    /// The tab group a closed view was in, to reopen it there.
    view_groups: Vec<(ViewId, TileId)>,
    /// A sheet symbol was double-clicked: open its sheet.
    enter_sheet: Option<Id>,
    status: Option<(String, Instant)>,
    title: String,
    revision: u64,
    screenshot: Option<(String, u32)>,
    allow_close: bool,
    keymap: Keymap,
    settings: Settings,
    show_settings: bool,
    /// Action whose new shortcut is being recorded in the settings window.
    recording: Option<Action>,
    agent: Agent,
    kicad: KicadImport,
    /// Schematic creation tools.
    sch: SchPlacer,
    /// Asking for the name of a new bus or of a signal leaving one.
    name_prompt: Option<NamePrompt>,
    /// The dock as last saved (the live one is in `settings`).
    saved_dock: Dock,
    /// The Project panel's tree.
    project_tree: ProjectPanel,
    /// Renaming a library item (from the Project panel).
    rename: Option<Rename>,
    /// Problems not seen in the Messages panel yet.
    unseen: Unseen,
    /// Board creation tools.
    pcb: PcbPlacer,
    /// Inputs of the current cross-probe highlights (to skip unchanged frames).
    probe_key: Option<(ViewId, Vec<ObjectId>, u64, Option<String>)>,
    script: Option<Script>,
    /// Commands picked in the native menu, run in the next `ui`.
    menu_cmds: Vec<Cmd>,
    #[cfg(target_os = "macos")]
    native_menu: crate::native_menu::NativeMenu,
    #[cfg(target_os = "macos")]
    title_bar: title_bar::TitleBar,
}

impl PcbApp {
    pub fn new(cc: &eframe::CreationContext<'_>, opts: Options) -> Self {
        let mut settings = Settings::load();
        view::style::set_theme(settings.theme);
        units::set_unit(settings.unit);
        theme::apply(&cc.egui_ctx);
        theme::apply_to_window(&cc.egui_ctx);
        let keymap = opts.keymap.unwrap_or_else(|| settings.keymap());
        let rs = cc.wgpu_render_state.as_ref().expect("wgpu renderer");
        CanvasResources::install(rs);

        let mut status = None;
        let doc = match opts.startup {
            Startup::Open(path) => match Document::open(&path) {
                Ok(doc) => {
                    settings.add_recent(&path);
                    let _ = settings.save();
                    doc
                }
                Err(e) => {
                    status = Some((format!("Could not open {}: {e}", path.display()), Instant::now()));
                    Document::new(Project::new("Untitled"))
                }
            },
            Startup::Demo => demo_document(),
            Startup::Empty => Document::new(Project::new("Untitled")),
            Startup::Stress(n) => {
                let t = Instant::now();
                let doc = Document::new(view::demo::stress_project(n));
                log::info!("generated {n} components in {:?}", t.elapsed());
                doc
            }
        };
        let root = doc.root();
        let tree = if opts.side_by_side { side_by_side_tree(root) } else { default_tree(root) };
        let mut app = Self {
            tree,
            project: LibraryEditor::new(PROJECT_LIB, LibraryDoc { doc, path: None }),
            panes: Panes::new(),
            active: root,
            last_sheet: root.sheet(),
            focus_lib: None,
            hovered: None,
            properties: Properties::default(),
            libraries: Vec::new(),
            missing_libraries: Vec::new(),
            synced_libraries: None,
            next_key: 0,
            close_requests: Vec::new(),
            view_groups: Vec::new(),
            enter_sheet: None,
            status,
            title: String::new(),
            revision: u64::MAX,
            screenshot: opts.screenshot.map(|p| (p, opts.screenshot_frames)),
            allow_close: false,
            keymap,
            settings,
            show_settings: false,
            recording: None,
            agent: Agent::new(opts.mcp.unwrap_or(agent::DEFAULT_PORT)),
            kicad: KicadImport::new(opts.kicad_paths.clone(), crate::settings::kicad_dir()),
            sch: SchPlacer::default(),
            name_prompt: None,
            saved_dock: Dock::default(),
            project_tree: ProjectPanel::default(),
            rename: None,
            unseen: Unseen::default(),
            pcb: PcbPlacer::default(),
            probe_key: None,
            script: None,
            menu_cmds: Vec::new(),
            #[cfg(target_os = "macos")]
            native_menu: crate::native_menu::NativeMenu::new(&cc.egui_ctx),
            #[cfg(target_os = "macos")]
            title_bar: title_bar::TitleBar::default(),
        };
        match opts.script.as_deref().map(Script::parse) {
            Some(Ok(s)) => app.script = Some(s),
            Some(Err(e)) => eprintln!("script: {e}"),
            None => {}
        }
        // The built-in library is always available (built in, saved only with "Save as").
        app.load_library(LibraryDoc::new(builtin::builtin_library()));
        app.sync_libraries();
        for path in &opts.library {
            app.add_library_file(&project_libs::absolute(path), false);
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
        for name in &opts.panels {
            match Panel::parse(name) {
                Some(p) => app.settings.dock.show(p),
                None => eprintln!("unknown panel {name:?}"),
            }
        }
        app.saved_dock = app.settings.dock.clone();
        if opts.mcp.is_some() {
            app.agent.start(&cc.egui_ctx);
        }
        // Development aid for screenshots: "all" or a reference designator.
        match opts.select.as_deref() {
            Some("all") => app.panes.get(root).it.key(&mut app.project.lib.doc.target(root), KeyAction::SelectAll),
            Some(refdes) => {
                let p = app.project.lib.doc.project();
                let symbols: Vec<&SymbolInstance> = p
                    .schematic
                    .component_by_refdes(refdes)
                    .map(|c| p.schematic.symbols_of(c.id).collect())
                    .unwrap_or_default();
                let view = symbols.first().map_or(root, |s| ViewId::Sheet(s.sheet));
                let ids = symbols.iter().filter(|s| ViewId::Sheet(s.sheet) == view).map(|s| ObjectId::Symbol(s.id));
                let ids = ids.collect();
                app.show_view(view);
                app.panes.get(view).it.set_selection(&mut app.project.lib.doc.target(view), ids);
            }
            None => {}
        }
        app
    }

    /// The schematic sheet being worked on: the active view's, else the one shown last.
    fn sheet_view(&self) -> ViewId {
        let doc = &self.project.lib.doc;
        match self.active {
            ViewId::Sheet(_) => self.active,
            ViewId::Board => match self.last_sheet {
                Some(s) if doc.project().schematic.sheets.contains(s) => ViewId::Sheet(s),
                _ => doc.root(),
            },
        }
    }

    /// Title of a view's tab: the sheet's name ("Schematic" for the root sheet), "PCB".
    fn view_title(project: &Project, view: ViewId) -> String {
        match view {
            ViewId::Board => "PCB".into(),
            ViewId::Sheet(s) => match project.schematic.sheets.get(s) {
                Some(sheet) if !sheet.is_root() => sheet.name.clone(),
                _ => "Schematic".into(),
            },
        }
    }

    /// After drawing a bus or a wire to a bus entry, ask for the missing name and add
    /// the label.
    fn name_prompt_ui(&mut self, ctx: &egui::Context) {
        if self.name_prompt.is_none() {
            let grid = self.panes.get(self.sheet_view()).it.grid;
            self.name_prompt = self.sch.take_prompt(&mut self.project.lib.doc, grid).map(NamePrompt::new);
        }
        let Some(dialog) = &mut self.name_prompt else { return };
        let Some(answer) = dialog.ui(ctx) else { return };
        let command = answer.map(|name| dialog.prompt.label(&name));
        self.name_prompt = None;
        match command {
            Some(Ok(cmd)) => {
                if let Err(e) = self.project.lib.doc.execute(cmd) {
                    self.set_status(e.to_string());
                }
            }
            Some(Err(e)) => self.set_status(e),
            None => {}
        }
    }

    fn set_status(&mut self, msg: impl Into<String>) {
        self.status = Some((msg.into(), Instant::now()));
    }

    fn busy(&self) -> bool {
        self.panes.iter().any(|p| !p.it.is_idle())
            || !self.project.it.is_idle()
            || self.libraries.iter().any(|l| !l.it.is_idle())
    }

    /// A library editor by key, including the Project Library's.
    fn lib(&mut self, key: u64) -> Option<&mut LibraryEditor> {
        if key == PROJECT_LIB {
            return Some(&mut self.project);
        }
        self.libraries.iter_mut().find(|l| l.key == key)
    }

    /// The focused library editor (the Project Library's too).
    fn focused_lib_ref(&self) -> Option<&LibraryEditor> {
        match self.focus_lib? {
            PROJECT_LIB => Some(&self.project),
            k => self.libraries.iter().find(|l| l.key == k),
        }
    }

    fn focused_lib(&mut self) -> Option<&mut LibraryEditor> {
        let key = self.focus_lib?;
        self.lib(key)
    }

    // ------------------------------------------------------------------------------------
    // Project files

    fn replace_document(&mut self, doc: Document) {
        let old_root = self.project.lib.doc.root();
        self.project = LibraryEditor::new(PROJECT_LIB, LibraryDoc { doc, path: None });
        self.sync_libraries();
        // The new root sheet takes the old one's tab; tabs of other sheets close.
        let root = self.project.lib.doc.root();
        if let Some(tile) = self.tree.tiles.find_pane(&Tab::View(old_root)) {
            self.tree.tiles.insert(tile, Tile::Pane(Tab::View(root)));
        }
        if self.active.sheet().is_some() {
            self.active = root;
        }
        self.last_sheet = root.sheet();
        self.panes.retain_sheets(self.project.lib.doc.project());
        for pane in self.panes.iter_mut() {
            let snap = pane.it.snap;
            pane.it = Interaction::new(pane.it.grid);
            pane.it.snap = snap;
            pane.canvas.request_fit();
        }
        self.close_missing_sheets();
        self.properties = Properties::default();
        self.sch.set_tool(SchTool::Select);
        self.pcb = PcbPlacer::default();
    }

    /// Close the tabs of sheets that no longer exist (deleted, undone, another project).
    fn close_missing_sheets(&mut self) {
        let project = self.project.lib.doc.project();
        let gone: Vec<TileId> = self
            .tree
            .tiles
            .iter()
            .filter_map(|(&id, t)| match t {
                Tile::Pane(Tab::View(ViewId::Sheet(s))) if !project.schematic.sheets.contains(*s) => Some(id),
                _ => None,
            })
            .collect();
        for tile in gone {
            self.tree.remove_recursively(tile);
        }
        self.view_groups.retain(|(v, _)| v.sheet().is_none_or(|s| project.schematic.sheets.contains(s)));
        self.panes.retain_sheets(project);
        let missing = |v: ViewId| v.sheet().is_some_and(|s| !project.schematic.sheets.contains(s));
        if self.hovered.is_some_and(missing) {
            self.hovered = None;
        }
        if missing(self.active) {
            self.active = self.project.lib.doc.root();
        }
        self.follow_visible_tabs();
    }

    /// Ask to save unsaved changes. Returns false if the user cancelled.
    fn confirm_discard(&mut self) -> bool {
        if !self.project.lib.doc.is_modified() {
            return true;
        }
        match ask_save(&format!("Save changes to \"{}\"?", self.project.lib.doc.project().name)) {
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
        self.open_path(&dir);
    }

    /// Open a project folder (after unsaved changes were dealt with).
    fn open_path(&mut self, dir: &std::path::Path) {
        match Document::open(dir) {
            Ok(doc) => {
                self.replace_document(doc);
                self.set_status(format!("Opened {}", dir.display()));
                self.remember_project(dir);
            }
            Err(e) => self.set_status(format!("Open failed: {e}")),
        }
    }

    fn remember_project(&mut self, dir: &std::path::Path) {
        self.settings.add_recent(dir);
        self.save_settings();
    }

    fn save_settings(&mut self) {
        if let Err(e) = self.settings.save() {
            self.set_status(format!("Settings not saved: {e}"));
        }
    }

    /// Switch the theme of the chrome and every canvas.
    fn set_theme(&mut self, ctx: &egui::Context, t: view::style::Theme) {
        view::style::set_theme(t);
        theme::apply(ctx);
        theme::apply_to_window(ctx);
        for pane in self.panes.iter_mut() {
            pane.canvas.background = view::style::background(pane.canvas.background.grid);
        }
        self.project.lib.doc.restyle();
        for l in &mut self.libraries {
            l.canvas.background = view::style::background(l.canvas.background.grid);
            l.lib.doc.restyle();
            l.view.restyle();
        }
        self.project.canvas.background = view::style::background(self.project.canvas.background.grid);
        self.project.view.restyle();
        self.probe_key = None;
        self.settings.theme = t;
        self.save_settings();
    }

    /// Save (or Save As when `choose` or the document has no folder yet).
    fn save(&mut self, choose: bool) -> bool {
        let path = match (&self.project.lib.doc.path, choose) {
            (Some(p), false) => p.clone(),
            _ => {
                let name = format!("{}.pcbproj", self.project.lib.doc.project().name);
                match rfd::FileDialog::new().set_title("Save project folder").set_file_name(name).save_file() {
                    Some(p) => p,
                    None => return false,
                }
            }
        };
        match self.project.lib.doc.save_as(&path) {
            Ok(()) => {
                self.set_status(format!("Saved {}", path.display()));
                self.remember_project(&path);
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

    /// Keep a library loaded, without a tab. Returns its key.
    fn load_library(&mut self, lib: LibraryDoc) -> u64 {
        let key = self.next_key;
        self.next_key += 1;
        self.libraries.push(LibraryEditor::new(key, lib));
        key
    }

    /// Load a library and show it in a tab (next to the schematic), optionally focused.
    fn add_library(&mut self, lib: LibraryDoc, focus: bool) -> u64 {
        let key = self.load_library(lib);
        self.open_library_tab(key, focus);
        key
    }

    /// Show a loaded library's tab, adding it next to the schematic if it has none.
    fn open_library_tab(&mut self, key: u64, focus: bool) {
        let tile = match self.tree.tiles.find_pane(&Tab::Library(key)) {
            Some(tile) => tile,
            None => {
                let near = [Tab::View(self.sheet_view()), Tab::Library(PROJECT_LIB), Tab::View(ViewId::Board)];
                insert_tab(&mut self.tree, Tab::Library(key), None, &near)
            }
        };
        if focus {
            self.tree.make_active(|id, _| id == tile);
            self.focus_lib = Some(key);
        }
    }

    fn focus_library_tab(&mut self, key: u64) {
        self.open_library_tab(key, true);
    }

    fn close_library_tab(&mut self, key: u64) {
        if let Some(tile) = self.tree.tiles.find_pane(&Tab::Library(key)) {
            self.tree.remove_recursively(tile);
        }
        if self.focus_lib == Some(key) {
            self.focus_lib = None;
        }
    }

    /// Unload a library and close its tab (unsaved changes are lost).
    fn unload_library(&mut self, key: u64) {
        self.close_library_tab(key);
        self.libraries.retain(|l| l.key != key);
    }

    /// Bring the loaded libraries in line with the project's library files (opening a
    /// project, adding or removing a library, undo and redo of those).
    fn sync_libraries(&mut self) {
        if self.synced_libraries.as_ref() == Some(&self.project.lib.doc.project().libraries) {
            return;
        }
        let wanted = self.project.lib.doc.project().libraries.clone();
        let mut next = self.next_key;
        let dropped = project_libs::sync(&wanted, &mut self.libraries, &mut self.missing_libraries, |doc| {
            next += 1;
            LibraryEditor::new(next - 1, doc)
        });
        self.next_key = next;
        for l in dropped {
            self.close_library_tab(l.key);
        }
        self.synced_libraries = Some(wanted);
    }

    /// Whether the project lists the library's file.
    fn is_listed(&self, lib: &LibraryDoc) -> bool {
        lib.path
            .as_deref()
            .is_some_and(|p| self.project.lib.doc.project().libraries.contains(&project_libs::absolute(p)))
    }

    /// Add a library (saved to a file) to the project and show it in a focused tab.
    fn add_to_project(&mut self, lib: LibraryDoc) -> Option<u64> {
        let loaded: Vec<&LibraryDoc> = self.libraries.iter().map(|l| &l.lib).collect();
        match project_libs::add_to_project(&mut self.project.lib.doc, &loaded, &lib) {
            Ok(()) => {
                let name = lib.name().to_string();
                let key = self.add_library(lib, true);
                self.set_status(format!("Added library {name} to the project"));
                Some(key)
            }
            Err(e) => {
                self.set_status(e);
                None
            }
        }
    }

    /// Add a library file to the project (or show it, if it is already there).
    fn add_library_file(&mut self, path: &std::path::Path, focus: bool) {
        if let Some(l) = self.libraries.iter().find(|l| l.lib.path.as_deref().is_some_and(|p| same_file(p, path))) {
            let key = l.key;
            if !self.is_listed(&self.libraries.iter().find(|l| l.key == key).unwrap().lib) {
                // Dropped from the project while it had unsaved changes: add it again.
                let command = Command::AddLibrary { path: project_libs::absolute(path) };
                if let Err(e) = self.project.lib.doc.execute(command) {
                    return self.set_status(e.to_string());
                }
            }
            return self.open_library_tab(key, focus);
        }
        match LibraryDoc::open(path) {
            Ok(lib) => {
                if let Some(key) = self.add_to_project(lib)
                    && !focus
                {
                    self.focus_lib = None;
                    self.open_library_tab(key, false);
                }
            }
            Err(e) => self.set_status(format!("Could not open library: {e}")),
        }
    }

    /// A file dialog for library files, starting in the project folder.
    fn library_dialog(&self, title: &str) -> rfd::FileDialog {
        let dialog = rfd::FileDialog::new().set_title(title).add_filter("PCB library", &["pcblib"]);
        match &self.project.lib.doc.path {
            Some(dir) => dialog.set_directory(dir),
            None => dialog,
        }
    }

    /// New library: choose its file right away, then it is added to the project.
    fn new_library(&mut self) {
        let Some(path) = self.library_dialog("New library").set_file_name("New library.pcblib").save_file() else {
            return;
        };
        let name = path.file_stem().map_or("New library".into(), |s| s.to_string_lossy().into_owned());
        let mut lib = LibraryDoc::new(Library::new(name));
        match lib.save_as(&path) {
            Ok(()) => {
                self.add_to_project(lib);
            }
            Err(e) => self.set_status(format!("Could not create library: {e}")),
        }
    }

    /// Add an existing library file to the project.
    fn open_library(&mut self) {
        let Some(path) = self.library_dialog("Add library").pick_file() else { return };
        self.add_library_file(&project_libs::absolute(&path), true);
    }

    fn save_library(&mut self, key: u64, choose: bool) -> bool {
        if key == PROJECT_LIB {
            return self.save(choose);
        }
        let Some(l) = self.libraries.iter().find(|l| l.key == key) else { return false };
        if l.is_builtin() {
            return self.save_builtin_copy(key);
        }
        let old = l.lib.path.clone();
        let path = match (old.clone(), choose) {
            (Some(p), false) => p,
            _ => {
                let name = format!("{}.pcblib", l.lib.name());
                match self.library_dialog("Save library").set_file_name(name).save_file() {
                    Some(p) => p,
                    None => return false,
                }
            }
        };
        let Some(l) = self.lib(key) else { return false };
        if let Err(e) = l.lib.save_as(&path) {
            self.set_status(format!("Save failed: {e}"));
            return false;
        }
        self.set_status(format!("Saved library {}", path.display()));
        // The project follows the file (a new file, or a library that wasn't in it yet).
        if let Some(command) =
            project_libs::after_save(&self.project.lib.doc.project().libraries, old.as_deref(), &path)
            && let Err(e) = self.project.lib.doc.execute(command)
        {
            self.set_status(e.to_string());
        }
        self.synced_libraries = None;
        true
    }

    /// The built-in library is read-only; "save" makes a copy of it (a library of its
    /// own) and adds that to the project.
    fn save_builtin_copy(&mut self, key: u64) -> bool {
        let Some(path) =
            self.library_dialog("Save a copy of the built-in library").set_file_name("Parts.pcblib").save_file()
        else {
            return false;
        };
        let Some(l) = self.lib(key) else { return false };
        let mut copy = project_libs::copy_for(l.lib.library(), &path);
        if let Err(e) = copy.save_as(&path) {
            self.set_status(format!("Save failed: {e}"));
            return false;
        }
        self.add_to_project(copy).is_some()
    }

    fn confirm_discard_library(&mut self, key: u64) -> bool {
        if key == PROJECT_LIB {
            return true;
        }
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

    /// Close a library's tab. Project libraries stay loaded; discarded changes are
    /// dropped (the file is loaded again). Libraries outside the project are unloaded.
    fn close_library(&mut self, key: u64) {
        if key == PROJECT_LIB {
            return self.close_library_tab(key);
        }
        if !self.confirm_discard_library(key) {
            return;
        }
        self.close_library_tab(key);
        let Some(l) = self.libraries.iter().find(|l| l.key == key) else { return };
        let (builtin, listed, discarded) = (l.is_builtin(), self.is_listed(&l.lib), l.lib.is_modified());
        if !builtin && (discarded || !listed) {
            self.libraries.retain(|l| l.key != key);
            self.synced_libraries = None;
        }
    }

    /// Remove a library from the project (asking about unsaved changes first).
    fn remove_library(&mut self, key: u64) {
        if !self.confirm_discard_library(key) {
            return;
        }
        let Some(l) = self.libraries.iter().find(|l| l.key == key) else { return };
        let path = l.lib.path.as_deref().map(project_libs::absolute);
        let name = l.lib.name().to_string();
        if let Some(path) = path.filter(|p| self.project.lib.doc.project().libraries.contains(p))
            && let Err(e) = self.project.lib.doc.execute(Command::RemoveLibrary { path })
        {
            return self.set_status(e.to_string());
        }
        self.unload_library(key);
        self.set_status(format!("Removed library {name} from the project"));
    }

    /// Remove a library file that could not be loaded from the project.
    fn remove_missing_library(&mut self, path: PathBuf) {
        if let Err(e) = self.project.lib.doc.execute(Command::RemoveLibrary { path }) {
            self.set_status(e.to_string());
        }
    }

    /// Point a library file that could not be loaded at another file.
    fn locate_missing_library(&mut self, from: PathBuf) {
        let title = format!("Locate {}", from.file_name().map_or(String::new(), |n| n.to_string_lossy().into_owned()));
        let Some(to) = self.library_dialog(&title).pick_file() else { return };
        let to = project_libs::absolute(&to);
        let lib = match LibraryDoc::open(&to) {
            Ok(lib) => lib,
            Err(e) => return self.set_status(format!("Could not open library: {e}")),
        };
        let others: Vec<PathBuf> =
            self.project.lib.doc.project().libraries.iter().filter(|p| **p != from).cloned().collect();
        let loaded: Vec<&LibraryDoc> = self.libraries.iter().map(|l| &l.lib).collect();
        if let Err(e) = project_libs::check_new(&others, &loaded, &lib) {
            return self.set_status(e);
        }
        match self.project.lib.doc.execute(Command::RelinkLibrary { from, to }) {
            Ok(_) => {
                self.load_library(lib);
                self.sync_libraries();
            }
            Err(e) => self.set_status(e.to_string()),
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
        let req = mcp::host::kicad_request(&job, |symbol| self.kicad.suggested(symbol))?;
        let target =
            job.library.as_deref().and_then(|n| self.libraries.iter().find(|l| l.lib.name() == n)).map(|l| l.key);
        let target = target.or_else(|| self.libraries.iter().find(|l| l.lib.name() == "KiCad import").map(|l| l.key));
        let new_name = job.library.as_deref().unwrap_or("KiCad import");
        let (key, imported, added, replaced) = self.kicad_import_into(target, new_name, &req)?;
        let library = self.lib(key).map(|l| l.lib.name().to_string()).unwrap_or_default();
        Ok(mcp::host::kicad_import_reply(&library, added, replaced, &imported, &req))
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
            match self.project.lib.doc.execute(Command::UpdateParts { library, parts }) {
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
        match self.project.lib.doc.editor().history().undo_label().map(str::to_string) {
            Some(label) => {
                self.project.lib.doc.undo();
                self.set_status(format!("Undo: {label}"));
            }
            None => self.set_status("Nothing to undo"),
        }
    }

    fn redo(&mut self) {
        if self.busy() {
            return;
        }
        match self.project.lib.doc.editor().history().redo_label().map(str::to_string) {
            Some(label) => {
                self.project.lib.doc.redo();
                self.set_status(format!("Redo: {label}"));
            }
            None => self.set_status("Nothing to redo"),
        }
    }

    /// Start placing a part in the schematic (it follows the cursor until clicked),
    /// importing it from library `lib_key` if the project doesn't have it yet.
    fn place_part(&mut self, lib_key: Option<u64>, part: Id) {
        if !self.project.lib.doc.project().library.parts.contains(part) {
            let Some(l) = lib_key.and_then(|k| self.libraries.iter().find(|l| l.key == k)) else { return };
            let issues = l.lib.library().validate();
            if !issues.is_empty() {
                return self.set_status(format!("Library has problems: {}", issues[0]));
            }
            let library = l.lib.library().clone();
            if let Err(e) = self.project.lib.doc.execute(Command::ImportParts { library, parts: vec![part] }) {
                return self.set_status(format!("Import failed: {e}"));
            }
        }
        self.focus_schematic();
        self.set_sch_tool(SchTool::Place);
        self.sch.place(part);
    }

    /// After tab changes (clicks, drag and drop, closing): keyboard focus goes to a view
    /// or library that is visible.
    fn follow_visible_tabs(&mut self) {
        let visible: Vec<Tab> =
            self.tree.active_tiles().into_iter().filter_map(|t| self.tree.tiles.get_pane(&t).copied()).collect();
        if let Some(key) = self.focus_lib
            && !visible.contains(&Tab::Library(key))
        {
            self.focus_lib = None;
        }
        if self.focus_lib.is_none() && !visible.contains(&Tab::View(self.active)) {
            match visible.iter().find_map(|t| if let Tab::View(k) = t { Some(*k) } else { None }) {
                Some(kind) => self.active = kind,
                None => {
                    self.focus_lib = visible.iter().find_map(|t| if let Tab::Library(k) = t { Some(*k) } else { None })
                }
            }
        }
    }

    /// Schematic and PCB are open in different tab groups (visible at the same time).
    fn is_side_by_side(&self) -> bool {
        let parent = |k| self.tree.tiles.find_pane(&Tab::View(k)).and_then(|t| self.tree.tiles.parent_of(t));
        match (parent(self.sheet_view()), parent(ViewId::Board)) {
            (Some(a), Some(b)) => a != b,
            _ => false,
        }
    }

    /// Does the view have a tab (it may be behind another one)?
    fn is_open(&self, view: ViewId) -> bool {
        self.tree.tiles.find_pane(&Tab::View(view)).is_some()
    }

    /// Close a view's tab. Its camera and selection stay for when it is shown again; a
    /// tool in progress is cancelled.
    fn close_view(&mut self, view: ViewId) {
        let Some(tile) = self.tree.tiles.find_pane(&Tab::View(view)) else { return };
        self.view_groups.retain(|(v, _)| *v != view);
        if let Some(group) = self.tree.tiles.parent_of(tile) {
            self.view_groups.push((view, group));
        }
        self.tree.remove_recursively(tile);
        match view.kind() {
            ViewKind::Schematic => self.set_sch_tool(SchTool::Select),
            ViewKind::Board => self.set_pcb_tool(PcbTool::Select),
        }
        let pane = self.panes.get(view);
        pane.it.reset();
        pane.it.refresh(&mut self.project.lib.doc.target(view));
        if self.hovered == Some(view) {
            self.hovered = None;
        }
        self.follow_visible_tabs();
    }

    /// Rearrange: everything as tabs of one area, or schematic (sheets and libraries)
    /// left and the PCB right.
    fn set_layout(&mut self, side_by_side: bool) {
        let root = self.project.lib.doc.root();
        let sheets = self.project.lib.doc.sheet_views().map(|v| Tab::View(v.id)).filter(|t| *t != Tab::View(root));
        let libs = std::iter::once(PROJECT_LIB).chain(self.libraries.iter().map(|l| l.key)).map(Tab::Library);
        let others: Vec<Tab> = sheets.chain(libs).filter(|t| self.tree.tiles.find_pane(t).is_some()).collect();
        let mut tree = if side_by_side { side_by_side_tree(root) } else { default_tree(root) };
        let group =
            tree.tiles.find_pane(&Tab::View(root)).and_then(|t| tree.tiles.parent_of(t)).expect("schematic tab");
        for tab in others {
            let tile = tree.tiles.insert_pane(tab);
            if let Some(Tile::Container(c)) = tree.tiles.get_mut(group) {
                c.add_child(tile);
            }
        }
        self.tree = tree;
        // Tile ids are sequential: the old tree's groups must not be looked up in the new one.
        self.view_groups.clear();
        self.show_view(self.active);
        for pane in self.panes.iter_mut() {
            pane.canvas.request_fit();
        }
    }

    /// Bring a view's tab to the front and give it keyboard focus, opening it if it was
    /// closed: in the tab group it was in, else next to the schematic, the libraries or
    /// the board.
    fn show_view(&mut self, view: ViewId) {
        let sheet = self.sheet_view();
        self.active = view;
        self.focus_lib = None;
        if let Some(s) = view.sheet() {
            self.last_sheet = Some(s);
        }
        let tile = match self.tree.tiles.find_pane(&Tab::View(view)) {
            Some(tile) => tile,
            None => {
                let root = self.project.lib.doc.root();
                let near = match view {
                    ViewId::Board => vec![Tab::View(sheet), Tab::View(root), Tab::Library(PROJECT_LIB)],
                    _ => vec![Tab::View(sheet), Tab::View(root), Tab::Library(PROJECT_LIB), Tab::View(ViewId::Board)],
                };
                let group = self.view_groups.iter().find(|(v, _)| *v == view).map(|(_, g)| *g);
                let tile = insert_tab(&mut self.tree, Tab::View(view), group, &near);
                self.panes.get(view).canvas.request_fit();
                tile
            }
        };
        self.tree.make_active(|id, _| id == tile);
    }

    /// Give the schematic tab (of the sheet worked on) keyboard focus and bring it to the
    /// front.
    fn focus_schematic(&mut self) {
        self.show_view(self.sheet_view());
    }

    fn set_sch_tool(&mut self, tool: SchTool) {
        let view = self.sheet_view();
        let pane = self.panes.get(view);
        if tool != SchTool::Select {
            pane.it.reset();
            pane.it.refresh(&mut self.project.lib.doc.target(view));
        }
        self.sch.set_tool(tool);
    }

    /// Cross-probing: the active view's selection highlights the matching objects in the
    /// views of the other kind (all sheets for the board), and nets on the board. The net
    /// being routed is highlighted too. With nothing selected in the active view, the
    /// other's selection probes (select in the schematic, switch to the board tab: the
    /// footprints are highlighted); for the board that is the sheet shown last.
    fn update_highlights(&mut self) {
        let (here, there) = match self.active {
            ViewId::Board => (ViewId::Board, self.sheet_view()),
            sheet => (sheet, ViewId::Board),
        };
        let from =
            if self.panes.selection(here).is_empty() && !self.panes.selection(there).is_empty() { there } else { here };
        let selection = self.panes.selection(from);
        let routing = self.pcb.routing_net().or(self.pcb.zone_net()).map(str::to_string);
        let key = Some((from, selection.clone(), self.project.lib.doc.revision, routing.clone()));
        if key == self.probe_key {
            return;
        }
        self.probe_key = key;
        let doc = &mut self.project.lib.doc;
        let probe = view::probe::cross_probe(doc.project(), doc.connectivity(), doc.copper(), from.kind(), &selection);
        let (project, views) = doc.project_and_views();
        for view in views {
            let objects: &[ObjectId] = if view.kind == from.kind() { &[] } else { &probe.objects };
            view.set_probe(project, objects);
        }
        let nets = routing.map_or(probe.nets, |n| vec![n]);
        doc.highlight_nets(&nets);
    }

    /// Where keyboard shortcuts go.
    fn context(&self) -> Context {
        match (self.focus_lib, self.active.kind()) {
            (Some(_), _) => Context::Library,
            (None, ViewKind::Schematic) => Context::Schematic,
            (None, ViewKind::Board) => Context::Board,
        }
    }

    /// Schematic tool shortcuts. Returns true if handled.
    fn sch_action(&mut self, action: Action) -> bool {
        let tool = match action {
            Action::SchWire => SchTool::Wire,
            Action::SchBus => SchTool::Bus,
            Action::SchBusEntry => SchTool::BusEntry,
            Action::SchLabel => SchTool::Label,
            Action::SchPower => SchTool::Power,
            Action::SchJunction => SchTool::Junction,
            Action::ToolSelect => SchTool::Select,
            Action::Posture => {
                self.sch.toggle_posture();
                return true;
            }
            _ if !self.sch.is_placing() => return false,
            Action::Rotate => {
                self.sch.rotate();
                return true;
            }
            Action::Mirror => {
                self.sch.mirror();
                return true;
            }
            Action::Cancel => {
                self.sch.escape();
                return true;
            }
            _ => return false,
        };
        self.set_sch_tool(tool);
        true
    }

    fn set_pcb_tool(&mut self, tool: PcbTool) {
        let pane = self.panes.get(ViewId::Board);
        if tool != PcbTool::Select {
            pane.it.reset();
            pane.it.refresh(&mut self.project.lib.doc.target(ViewId::Board));
        }
        self.pcb.set_tool(tool);
    }

    /// Board tool shortcuts. Returns true if handled.
    fn pcb_action(&mut self, action: Action) -> bool {
        let tool = match action {
            Action::PcbRoute => PcbTool::Route,
            Action::PcbVia => PcbTool::Via,
            Action::PcbOutline => PcbTool::Outline,
            Action::PcbZone => PcbTool::Zone,
            Action::ToolSelect => PcbTool::Select,
            Action::Posture => {
                self.pcb.toggle_posture();
                return true;
            }
            Action::PcbNextLayer => {
                if let Err(e) = self.pcb.next_layer(&mut self.project.lib.doc) {
                    self.set_status(e);
                }
                return true;
            }
            Action::Cancel if self.pcb.is_placing() => {
                self.pcb.escape();
                return true;
            }
            _ => return false,
        };
        self.set_pcb_tool(tool);
        true
    }

    fn run_action(&mut self, action: Action) {
        if let Some(panel) = Panel::of_action(action) {
            return self.settings.dock.toggle(panel);
        }
        if let Some(key) = self.focus_lib
            && self.lib_action(key, action)
        {
            return;
        }
        // A closed view takes no keys (only the project-level actions below).
        let view_keys = self.focus_lib.is_some() || self.is_open(self.active);
        if view_keys && self.context() == Context::Schematic && self.sch_action(action) {
            return;
        }
        if view_keys && self.context() == Context::Board && self.pcb_action(action) {
            return;
        }
        let key_action = match action {
            Action::New => return self.new_document(),
            Action::Settings => {
                self.show_settings = true;
                return;
            }
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
            _ if !view_keys => return,
            Action::FitView => return self.panes.get(self.active).canvas.request_fit(),
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
            | Action::SchWire
            | Action::SchBus
            | Action::SchBusEntry
            | Action::SchLabel
            | Action::SchPower
            | Action::SchJunction
            | Action::Posture
            | Action::PcbRoute
            | Action::PcbVia
            | Action::PcbOutline
            | Action::PcbZone
            | Action::PcbNextLayer
            | Action::PanelProject
            | Action::PanelMessages
            | Action::PanelProperties
            | Action::PanelToolSettings
            | Action::PanelBoardSetup
            | Action::PanelOverview => return,
        };
        let view = self.active;
        if key_action == KeyAction::Delete && !self.confirm_sheet_delete(&self.panes.selection(view)) {
            return;
        }
        let pane = self.panes.get(view);
        pane.it.key(&mut self.project.lib.doc.target(view), key_action);
    }

    /// Ask before deleting sheets that have contents (they go along). True: go ahead.
    fn confirm_sheet_delete(&self, ids: &[ObjectId]) -> bool {
        let s = &self.project.lib.doc.project().schematic;
        let full: Vec<&str> = ids
            .iter()
            .filter_map(|&id| if let ObjectId::Sheet(sh) = id { s.sheets.get(sh) } else { None })
            .filter(|sh| !s.objects_on(sh.id).is_empty())
            .map(|sh| sh.name.as_str())
            .collect();
        if full.is_empty() {
            return true;
        }
        let what = match &full[..] {
            [one] => format!("Sheet \"{one}\" is not empty."),
            many => format!(
                "Sheets {} are not empty.",
                many.iter().map(|n| format!("\"{n}\"")).collect::<Vec<_>>().join(", ")
            ),
        };
        let answer = rfd::MessageDialog::new()
            .set_title("Delete sheet")
            .set_description(format!(
                "{what} Deleting removes everything on it and its sub-sheets (undo brings it back)."
            ))
            .set_buttons(rfd::MessageButtons::OkCancelCustom("Delete".into(), "Cancel".into()))
            .show();
        matches!(answer, rfd::MessageDialogResult::Custom(ref b) if b == "Delete")
    }

    /// Handle an action for a focused library. Returns false for project-level actions.
    fn lib_action(&mut self, key: u64, action: Action) -> bool {
        match action {
            Action::New | Action::Open | Action::Settings => return false,
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
            Action::New | Action::Open | Action::Save | Action::SaveAs | Action::Settings => None,
            Action::SchWire
            | Action::SchBus
            | Action::SchBusEntry
            | Action::SchLabel
            | Action::SchPower
            | Action::SchJunction
            | Action::Posture => None,
            Action::PcbRoute | Action::PcbVia | Action::PcbOutline | Action::PcbZone | Action::PcbNextLayer => None,
            Action::PanelProject
            | Action::PanelMessages
            | Action::PanelProperties
            | Action::PanelToolSettings
            | Action::PanelBoardSetup
            | Action::PanelOverview => None,
        };
        if let Some(m) = msg {
            self.set_status(m);
        }
        true
    }

    // ------------------------------------------------------------------------------------
    // Panels

    /// The main menu: in the system menu bar on macOS, else a bar atop the window.
    fn main_menu(&mut self, ui: &mut egui::Ui, frame: &eframe::Frame, gap: i8) {
        let margin = |top| {
            egui::Frame::NONE.fill(theme::window_bg()).inner_margin(egui::Margin {
                left: 8,
                right: 8,
                top,
                bottom: gap,
            })
        };
        let panel = egui::Panel::top("menu").show_separator_line(false);
        if cfg!(target_os = "macos") {
            #[cfg(target_os = "macos")]
            {
                self.native_menu.sync(self.menus(true));
                // Our own title strip, unless full screen hides the title bar.
                let chrome = title_bar::chrome(frame).unwrap_or(title_bar::Chrome::DEFAULT);
                let fullscreen = ui.ctx().input(|i| i.viewport().fullscreen.unwrap_or(false));
                if !fullscreen && chrome.height > 0.0 {
                    let edited = self.project.lib.doc.is_modified();
                    title_bar::set_document_edited(frame, edited);
                    let name = self.project_name();
                    self.title_bar.show(ui, frame, chrome, &name, edited);
                    return;
                }
            }
            let _ = frame;
            panel.frame(margin(0)).exact_size(f32::from(gap)).show(ui, |_| {});
        } else {
            let menus = self.menus(false);
            if let Some(cmd) = panel.frame(margin(2)).show(ui, |ui| menu::bar_ui(ui, &menus)).inner {
                self.run_menu_cmd(ui.ctx(), cmd);
            }
        }
    }

    fn menus(&self, native: bool) -> Vec<Menu> {
        let h = match self.focus_lib.and_then(|k| self.libraries.iter().find(|l| l.key == k)) {
            Some(l) => l.lib.doc.editor().history(),
            None => self.project.lib.doc.editor().history(),
        };
        let exe = std::env::current_exe().map_or("pcb-designer".into(), |p| p.display().to_string());
        let project = self.project.lib.doc.path.as_ref().map_or(String::new(), |p| format!(" \"{}\"", p.display()));
        menu::menus(&MenuState {
            native,
            keymap: &self.keymap,
            library_focus: self.focus_lib.is_some_and(|k| k != PROJECT_LIB),
            recent: &self.settings.recent,
            undo: h.undo_label().map(|l| format!("Undo {l}")),
            redo: h.redo_label().map(|l| format!("Redo {l}")),
            side_by_side: self.is_side_by_side(),
            theme: view::style::theme(),
            agent_running: self.agent.is_running(),
            agent_error: self.agent.error.clone(),
            agent_url: self.agent.url(),
            agent_port: self.agent.port,
            headless_command: format!("claude mcp add pcb-designer -- \"{exe}\" --mcp-stdio{project}"),
        })
    }

    fn run_menu_cmd(&mut self, ctx: &egui::Context, cmd: Cmd) {
        match cmd {
            Cmd::Action(a) => self.run_action(a),
            Cmd::OpenRecent(p) => {
                if self.confirm_discard() {
                    self.open_path(&p);
                }
            }
            Cmd::ClearRecent => {
                self.settings.recent.clear();
                self.save_settings();
            }
            Cmd::OpenDemo => {
                if self.confirm_discard() {
                    self.replace_document(demo_document());
                }
            }
            Cmd::NewLibrary => self.new_library(),
            Cmd::AddLibrary => self.open_library(),
            Cmd::ImportKicad => self.kicad.open = true,
            Cmd::Quit => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
            Cmd::Layout { side_by_side } => {
                if side_by_side != self.is_side_by_side() {
                    self.set_layout(side_by_side);
                }
            }
            Cmd::Theme(t) => {
                if t != view::style::theme() {
                    self.set_theme(ctx, t);
                }
            }
            Cmd::Drc => {
                let n = self.project.lib.doc.run_drc().violations.len();
                self.set_status(match n {
                    0 => "DRC: no violations".to_string(),
                    n => format!("DRC: {n} violation(s), listed in the board properties"),
                });
            }
            Cmd::ToggleAgent => {
                if self.agent.is_running() {
                    self.agent.stop();
                } else {
                    self.agent.start(ctx);
                }
            }
            Cmd::Copy(text) => ctx.copy_text(text),
            Cmd::ShowShortcuts => self.show_settings = true,
        }
    }

    /// Theme and key bindings (saved in the settings file).
    fn settings_window(&mut self, ctx: &egui::Context) {
        let mut open = self.show_settings;
        let mut theme_choice = None;
        let mut changed = false;
        egui::Window::new("Settings").open(&mut open).resizable(true).default_height(560.0).collapsible(false).show(
            ctx,
            |ui| {
                ui.heading("Appearance");
                ui.horizontal(|ui| {
                    ui.label("Theme");
                    let current = view::style::theme();
                    for &t in view::style::Theme::ALL {
                        if ui.selectable_label(current == t, t.label()).clicked() && current != t {
                            theme_choice = Some(t);
                        }
                    }
                });
                ui.add_space(8.0);
                ui.heading("Keyboard shortcuts");
                let conflicts = self.keymap.conflicts();
                if !conflicts.is_empty() {
                    let text: Vec<String> = conflicts
                        .iter()
                        .map(|(s, a)| {
                            let names: Vec<&str> = a.iter().map(|a| a.label()).collect();
                            format!("{} is bound to {}", ctx.format_shortcut(&s.to_egui()), names.join(" and "))
                        })
                        .collect();
                    ui.label(RichText::new(text.join("\n")).color(theme::warn()));
                }
                egui::ScrollArea::vertical().max_height(420.0).show(ui, |ui| {
                    egui::Grid::new("keymap").num_columns(2).striped(true).spacing([24.0, 4.0]).show(ui, |ui| {
                        for &action in Action::ALL {
                            ui.label(action.label());
                            ui.horizontal(|ui| {
                                let keys = self.keymap.shortcuts(action).to_vec();
                                for (i, k) in keys.iter().enumerate() {
                                    let text = format!("{}  ×", ctx.format_shortcut(&k.to_egui()));
                                    if ui.small_button(text).on_hover_text("Remove").clicked() {
                                        let mut keys = keys.clone();
                                        keys.remove(i);
                                        self.keymap.set(action, keys);
                                        changed = true;
                                    }
                                }
                                if self.recording == Some(action) {
                                    ui.label(RichText::new("Press a key… (Esc cancels)").color(theme::accent()));
                                } else if ui.small_button("+").on_hover_text("Add a shortcut").clicked() {
                                    self.recording = Some(action);
                                }
                            });
                            ui.end_row();
                        }
                    });
                });
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    if ui.button("Reset to defaults").clicked() {
                        self.keymap = Keymap::default();
                        changed = true;
                    }
                    if ui.button("Copy as JSON").clicked() {
                        ui.ctx().copy_text(self.keymap.to_json());
                    }
                });
                if let Some(path) = Settings::path() {
                    ui.label(RichText::new(format!("Saved in {}", path.display())).color(theme::text_dim()));
                }
            },
        );
        if !open {
            self.recording = None;
        }
        self.show_settings = open;
        if changed {
            self.settings.set_keymap(&self.keymap);
            self.save_settings();
        }
        if let Some(t) = theme_choice {
            self.set_theme(ctx, t);
        }
    }

    /// While a shortcut is being recorded, the next key press (with modifiers) becomes it.
    /// Returns true if input was consumed.
    fn record_shortcut(&mut self, ctx: &egui::Context) -> bool {
        let Some(action) = self.recording else { return false };
        let pressed = ctx.input(|i| {
            i.events.iter().find_map(|e| match e {
                egui::Event::Key { key, pressed: true, modifiers, .. } => Some((*key, *modifiers)),
                _ => None,
            })
        });
        let Some((key, m)) = pressed else { return true };
        self.recording = None;
        if key == egui::Key::Escape {
            return true;
        }
        let shortcut = Shortcut { command: m.command, shift: m.shift, alt: m.alt, key };
        let mut keys = self.keymap.shortcuts(action).to_vec();
        if !keys.contains(&shortcut) {
            keys.push(shortcut);
            self.keymap.set(action, keys);
            self.settings.set_keymap(&self.keymap);
            self.save_settings();
        }
        true
    }

    // ------------------------------------------------------------------------------------
    // Project panel

    /// The Project panel's tree, built from the project and the loaded libraries.
    fn tree_nodes(&self) -> Vec<TreeNode> {
        use project_panel::Info;
        let p = self.project.lib.doc.project();
        let refs: Vec<&Library> = self.libraries.iter().map(|l| l.lib.library()).collect();
        let statuses = part_statuses(p, &refs);
        let changed = statuses.iter().filter(|s| s.state == PartState::LibraryChanged).count();

        let project_info = Info {
            strong: true,
            detail: self.project.lib.doc.path.as_deref().map_or("not saved".into(), project_panel::short_path),
            path: self.project.lib.doc.path.clone(),
            ..Info::icon(Icon::Project)
        };
        let used = |part: Id| p.schematic.components.iter().any(|c| c.part == part);
        let unused = p.library.parts.iter().filter(|part| !used(part.id)).count();
        let lib_info = Info {
            detail: project_panel::count(p.library.parts.len(), "part"),
            count: changed,
            unused,
            editable: true,
            tip: "The parts this design uses (copies, edited here for this project only)".into(),
            ..Info::icon(Icon::Library)
        };
        let project_lib = TreeNode::new(NodeId::Library(Owner::Project), "Project Library", lib_info).with_children(
            project_panel::library_children(Owner::Project, &p.library, true, |id| {
                let s = statuses.iter().find(|s| s.part == id)?;
                let (text, warn) = match s.state {
                    PartState::UpToDate => ("", false),
                    PartState::LibraryChanged => ("library changed", true),
                    PartState::ModifiedLocally => ("modified", false),
                    PartState::BothChanged => ("modified · library changed", true),
                    PartState::LibraryMissing => ("library not loaded", false),
                    PartState::RemovedFromLibrary => ("removed from library", true),
                    PartState::Local => ("local", false),
                };
                let mut text = text.to_string();
                if !used(id) {
                    text = if text.is_empty() { "unused".into() } else { format!("unused · {text}") };
                }
                let tip = s.library_name.as_deref().map_or(String::new(), |n| format!("From {n}"));
                let update = matches!(s.state, PartState::LibraryChanged | PartState::BothChanged);
                Some(project_panel::PartDetail { text, warn, update, dim: !used(id), tip })
            }),
        );
        let schematic = p.schematic.root_sheet().map(|root| sheet_node(p, root));
        let project = TreeNode::new(NodeId::Project, p.name.clone(), project_info).with_children(
            vec![schematic, Some(TreeNode::new(NodeId::Board, "PCB", Info::icon(Icon::Board))), Some(project_lib)]
                .into_iter()
                .flatten()
                .collect(),
        );

        let listed = &p.libraries;
        let library = |l: &LibraryEditor, label: String| {
            let path = l.lib.path.clone();
            let is_listed = path.as_deref().is_some_and(|p| listed.contains(&project_libs::absolute(p)));
            let mut detail = match &path {
                None if l.is_builtin() => "read-only".to_string(),
                None => "not saved".to_string(),
                Some(p) if is_listed => p.parent().map_or(String::new(), project_panel::short_path),
                Some(_) => "not in the project".to_string(),
            };
            if l.lib.is_modified() {
                detail = format!("modified · {detail}");
            }
            let info = Info {
                detail,
                warn: l.lib.is_modified(),
                editable: !l.is_builtin(),
                flag: is_listed,
                path,
                ..Info::icon(Icon::Library)
            };
            TreeNode::new(NodeId::Library(Owner::Lib(l.key)), label, info).with_children(
                project_panel::library_children(Owner::Lib(l.key), l.lib.library(), !l.is_builtin(), |_| None),
            )
        };
        let builtin = self.libraries.iter().find(|l| l.is_builtin()).map(|l| library(l, "Built-in Library".into()));

        // External libraries in the project's order, then unsaved / unlisted ones.
        let mut external: Vec<TreeNode> = Vec::new();
        for path in listed {
            let loaded = self
                .libraries
                .iter()
                .find(|l| l.lib.path.as_deref().is_some_and(|p| project_libs::absolute(p) == *path));
            if let Some(l) = loaded {
                external.push(library(l, l.lib.name().to_string()));
            } else if let Some(m) = self.missing_libraries.iter().find(|m| m.path == *path) {
                let name = m.path.file_name().map_or(String::new(), |n| n.to_string_lossy().into_owned());
                let info = Info {
                    detail: "missing".into(),
                    warn: true,
                    dim: true,
                    tip: format!("{}\n{}", m.path.display(), m.error),
                    path: Some(m.path.clone()),
                    ..Info::icon(Icon::Missing)
                };
                external.push(TreeNode::new(NodeId::Missing(project_panel::path_hash(&m.path)), name, info));
            }
        }
        for l in &self.libraries {
            let is_listed = l.lib.path.as_deref().is_some_and(|p| listed.contains(&project_libs::absolute(p)));
            if !l.is_builtin() && !is_listed {
                external.push(library(l, l.lib.name().to_string()));
            }
        }
        let external_info = Info {
            detail: if external.is_empty() { String::new() } else { external.len().to_string() },
            ..Info::icon(Icon::Folder)
        };
        let external =
            TreeNode::new(NodeId::ExternalLibraries, "External Libraries", external_info).with_children(external);
        std::iter::once(project).chain(builtin).chain(std::iter::once(external)).collect()
    }

    /// The tree node of the focused tab (the item open in a library tab).
    fn focus_node(&self) -> Option<NodeId> {
        match self.focused_lib_ref() {
            Some(l) => {
                let owner = if l.is_project() { Owner::Project } else { Owner::Lib(l.key) };
                Some(match l.open {
                    Open::Part(p) => NodeId::Item(owner, ObjectId::LibPart(p)),
                    Open::Item(Item::Symbol(s)) => NodeId::Item(owner, ObjectId::LibSymbol(s)),
                    Open::Item(Item::Footprint(f)) => NodeId::Item(owner, ObjectId::LibFootprint(f)),
                    Open::None => NodeId::Library(owner),
                })
            }
            None if !self.is_open(self.active) => None,
            None => Some(match self.active {
                ViewId::Sheet(s) => NodeId::Sheet(s),
                ViewId::Board => NodeId::Board,
            }),
        }
    }

    fn project_panel_ui(&mut self, ui: &mut egui::Ui, roots: &[TreeNode]) {
        self.project_tree.expand_defaults(self.project.lib.doc.root().sheet());
        self.project_tree.follow(roots, self.focus_node());
        if let Some(act) = self.project_tree.ui(ui, roots) {
            self.tree_act(act);
        }
    }

    /// Carry out something asked for in the Project panel.
    fn tree_act(&mut self, act: project_panel::Act) {
        use project_panel::Act;
        match act {
            Act::ShowView(view) => self.show_view(view),
            Act::NewSheet(parent) => {
                let parent = parent.or(self.sheet_view().sheet());
                if let Some(parent) = parent {
                    let text = self.new_sheet_name(parent);
                    self.rename = Some(Rename { what: Naming::NewSheet(parent), text, focused: false });
                }
            }
            Act::RenameSheet(sheet) => {
                if let Some(sh) = self.project.lib.doc.project().schematic.sheets.get(sheet) {
                    self.rename = Some(Rename { what: Naming::Sheet(sheet), text: sh.name.clone(), focused: false });
                }
            }
            Act::DeleteSheet(sheet) => {
                let ids = vec![ObjectId::Sheet(sheet)];
                if self.confirm_sheet_delete(&ids)
                    && let Err(e) = self.project.lib.doc.execute(Command::Delete { ids })
                {
                    self.set_status(e.to_string());
                }
            }
            Act::SaveProject => {
                self.save(false);
            }
            Act::SaveProjectAs => {
                self.save(true);
            }
            Act::Reveal(path) => {
                if let Err(e) = project_panel::reveal(&path) {
                    self.set_status(format!("Could not show {}: {e}", path.display()));
                }
            }
            Act::NewLibrary => self.new_library(),
            Act::AddLibrary => self.open_library(),
            Act::ImportKicad => self.kicad.open = true,
            Act::OpenLibrary(key) => self.focus_library_tab(key),
            Act::OpenItem(key, open) => {
                if let Some(l) = self.lib(key) {
                    l.show(open);
                }
                self.focus_library_tab(key);
            }
            Act::SaveLibrary(key) => {
                self.save_library(key, false);
            }
            Act::SaveLibraryAs(key) => {
                self.save_library(key, true);
            }
            Act::CloseLibrary(key) => self.close_library(key),
            Act::RemoveLibrary(key) => self.remove_library(key),
            Act::Locate(path) => self.locate_missing_library(path),
            Act::Forget(path) => self.remove_missing_library(path),
            Act::NewItem(key, group) => {
                let Some(l) = self.lib(key) else { return };
                let created = match group {
                    Group::Parts => l.lib.new_part("New part", None, None).map(Open::Part),
                    Group::Symbols => l.lib.new_symbol("New symbol").map(|s| Open::Item(Item::Symbol(s))),
                    Group::Footprints => l.lib.new_footprint("New footprint").map(|f| Open::Item(Item::Footprint(f))),
                };
                self.show_created(key, created);
            }
            Act::PartFrom(key, item) => {
                let Some(l) = self.lib(key) else { return };
                let lib = l.lib.library();
                let (symbol, footprint, name) = match item {
                    ObjectId::LibSymbol(s) => (Some(s), None, lib.symbols.get(s).map(|s| s.name.clone())),
                    ObjectId::LibFootprint(f) => (None, Some(f), lib.footprints.get(f).map(|f| f.name.clone())),
                    _ => return,
                };
                let created = l.lib.new_part(&name.unwrap_or("New part".into()), symbol, footprint).map(Open::Part);
                self.show_created(key, created);
            }
            Act::Duplicate(key, item) => {
                let Some(l) = self.lib(key) else { return };
                let created = l.lib.duplicate(item).map(project_panel::open_of);
                self.show_created(key, created);
            }
            Act::Rename(key, item) => {
                let lib = self.lib(key).map(|l| l.lib.library());
                let name = lib.and_then(|lib| match item {
                    ObjectId::LibPart(p) => lib.parts.get(p).map(|p| p.name.clone()),
                    ObjectId::LibSymbol(s) => lib.symbols.get(s).map(|s| s.name.clone()),
                    ObjectId::LibFootprint(f) => lib.footprints.get(f).map(|f| f.name.clone()),
                    _ => None,
                });
                if let Some(name) = name {
                    self.rename = Some(Rename { what: Naming::Item(key, item), text: name, focused: false });
                }
            }
            Act::Delete(key, item) => {
                let Some(l) = self.lib(key) else { return };
                let r = l.lib.delete(item);
                l.refresh();
                if let Err(e) = r {
                    self.set_status(e);
                }
            }
            Act::Place(Owner::Project, part) => self.place_part(None, part),
            Act::Place(Owner::Lib(key), part) => self.place_part(Some(key), part),
            Act::CopyToProject(key, part) => {
                let Some(l) = self.libraries.iter().find(|l| l.key == key) else { return };
                let issues = l.lib.library().validate();
                if let Some(first) = issues.first() {
                    return self.set_status(format!("Library has problems: {first}"));
                }
                let library = l.lib.library().clone();
                match self.project.lib.doc.execute(Command::ImportParts { library, parts: vec![part] }) {
                    Ok(_) => self.set_status("Copied to the Project Library"),
                    Err(e) => self.set_status(format!("Copy failed: {e}")),
                }
            }
            Act::Update(part) => {
                let refs: Vec<&Library> = self.libraries.iter().map(|l| l.lib.library()).collect();
                let statuses = part_statuses(self.project.lib.doc.project(), &refs);
                // All: the parts only the library changed. One part: also one edited here,
                // after asking.
                let wanted = |s: &editor_core::query::PartStatus| match part {
                    None => s.state == PartState::LibraryChanged,
                    Some(p) => s.part == p && matches!(s.state, PartState::LibraryChanged | PartState::BothChanged),
                };
                let chosen: Vec<_> = statuses.into_iter().filter(wanted).collect();
                if chosen.iter().any(|s| s.state == PartState::BothChanged) {
                    let answer = rfd::MessageDialog::new()
                        .set_title("Update from library")
                        .set_description(format!(
                            "\"{}\" was also changed in this project. Updating replaces those changes with the library's version.",
                            chosen[0].name
                        ))
                        .set_buttons(rfd::MessageButtons::OkCancelCustom("Update".into(), "Cancel".into()))
                        .show();
                    if !matches!(answer, rfd::MessageDialogResult::Custom(ref b) if b == "Update") {
                        return;
                    }
                }
                let parts: Vec<(usize, Id)> = chosen.iter().filter_map(|s| Some((s.library?, s.part))).collect();
                self.update_parts(&parts);
            }
            Act::RemoveUnused => {
                let ids = editor_core::query::unused_library_objects(self.project.lib.doc.project());
                let n = ids.iter().filter(|id| matches!(id, ObjectId::LibPart(_))).count();
                match self.project.lib.doc.execute(Command::Delete { ids }) {
                    Ok(_) => self.set_status(format!(
                        "Removed {} from the Project Library",
                        project_panel::count(n, "unused part")
                    )),
                    Err(e) => self.set_status(e.to_string()),
                }
                self.project.refresh();
            }
        }
    }

    /// Show a newly created library item in its tab.
    fn show_created(&mut self, key: u64, created: Result<Open, String>) {
        match created {
            Ok(open) => {
                if let Some(l) = self.lib(key) {
                    l.refresh();
                    l.show(open);
                }
                self.focus_library_tab(key);
            }
            Err(e) => self.set_status(e),
        }
    }

    /// The rename dialog (from the Project panel).
    /// "Sheet", "Sheet 2", …: the first name not taken below `parent`.
    fn new_sheet_name(&self, parent: Id) -> String {
        let s = &self.project.lib.doc.project().schematic;
        (1..)
            .map(|i| if i == 1 { "Sheet".to_string() } else { format!("Sheet {i}") })
            .find(|n| s.sheet_name_issue(Some(parent), n, None).is_none())
            .unwrap_or_default()
    }

    fn rename_ui(&mut self, ctx: &egui::Context) {
        let Some(r) = &mut self.rename else { return };
        let (title, button) = match r.what {
            Naming::NewSheet(_) => ("New sheet", "Place"),
            Naming::Sheet(_) => ("Rename sheet", "Rename"),
            Naming::Item(..) => ("Rename", "Rename"),
        };
        // Sheet names are checked as they are typed.
        let schematic = &self.project.lib.doc.project().schematic;
        let issue = match r.what {
            Naming::NewSheet(parent) => schematic.sheet_name_issue(Some(parent), &r.text, None),
            Naming::Sheet(s) => {
                schematic.sheet_name_issue(schematic.sheets.get(s).and_then(|s| s.parent), &r.text, Some(s))
            }
            Naming::Item(..) => None,
        };
        let mut done = None;
        let modal = theme::dialog(ctx, egui::Id::unique("rename_item"), title, 320.0, |ui| {
            let edit = ui.add(egui::TextEdit::singleline(&mut r.text).desired_width(f32::INFINITY));
            if !r.focused {
                edit.request_focus();
                r.focused = true;
            }
            if let Some(issue) = &issue {
                ui.add_space(4.0);
                ui.label(RichText::new(issue).color(theme::warn()).small());
            }
            if edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) && issue.is_none() {
                done = Some(true);
            }
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                if ui.add_enabled(issue.is_none(), egui::Button::new(button)).clicked() {
                    done = Some(true);
                }
                if ui.button("Cancel").clicked() {
                    done = Some(false);
                }
            });
        });
        if modal.should_close() && done.is_none() {
            done = Some(false);
        }
        let Some(ok) = done else { return };
        let Rename { what, text, .. } = self.rename.take().unwrap();
        if !ok {
            return;
        }
        match what {
            Naming::Item(key, item) => {
                let Some(l) = self.lib(key) else { return };
                let r = l.lib.rename(item, &text);
                l.refresh();
                if let Err(e) = r {
                    self.set_status(e);
                }
            }
            Naming::Sheet(sheet) => {
                let Some(mut sh) = self.project.lib.doc.project().schematic.sheets.get(sheet).cloned() else { return };
                sh.name = text;
                let objects = vec![Object::Sheet(sh)];
                if let Err(e) = self.project.lib.doc.execute(Command::Update { objects }) {
                    self.set_status(e.to_string());
                }
            }
            Naming::NewSheet(parent) => {
                // The symbol follows the cursor on the parent sheet; a click adds the sheet.
                self.show_view(ViewId::Sheet(parent));
                self.set_sch_tool(SchTool::Select);
                self.sch.place_sheet(text);
            }
        }
    }

    /// A window edge's bar of panel buttons (the left one also holds the bottom panels).
    fn dock_bar(&mut self, ui: &mut egui::Ui, side: Side) {
        let frame = egui::Frame::NONE.fill(theme::window_bg()).inner_margin(egui::Margin::symmetric(0, 2));
        let panel = match side {
            Side::Left => egui::Panel::left("dock_bar_left"),
            _ => egui::Panel::right("dock_bar_right"),
        };
        let mut clicked = None;
        panel.exact_size(DOCK_BAR).resizable(false).show_separator_line(false).frame(frame).show(ui, |ui| {
            ui.vertical_centered(|ui| {
                ui.spacing_mut().item_spacing.y = 4.0;
                let groups: &[Side] = if side == Side::Left { &[Side::Left, Side::Bottom] } else { &[Side::Right] };
                for (g, &group) in groups.iter().enumerate() {
                    if g > 0 {
                        let (r, _) = ui.allocate_exact_size(egui::vec2(20.0, 9.0), egui::Sense::hover());
                        ui.painter().hline(r.x_range(), r.center().y, egui::Stroke::new(1.0, theme::border()));
                    }
                    for panel in Panel::on(group) {
                        let badge = (panel == Panel::Messages && !self.settings.dock.is_open(panel))
                            .then_some((self.unseen.new, self.unseen.total));
                        let open = self.settings.dock.is_open(panel);
                        if dock_button(ui, panel, open, badge, &self.keymap) {
                            clicked = Some(panel);
                        }
                    }
                }
            });
        });
        if let Some(p) = clicked {
            self.settings.dock.toggle(p);
        }
    }

    /// The panel open on `side`, with a title bar to hide it.
    fn dock_panel(&mut self, ui: &mut egui::Ui, side: Side) {
        let Some(panel) = self.settings.dock.open(side) else { return };
        let gap = theme::GAP;
        let (container, margin) = match side {
            Side::Left => (egui::Panel::left("dock_left"), egui::Margin::default()),
            Side::Right => (egui::Panel::right("dock_right"), egui::Margin::default()),
            Side::Bottom => {
                (egui::Panel::bottom("dock_bottom"), egui::Margin { left: gap, right: gap, top: gap, bottom: 0 })
            }
        };
        let mut hide = false;
        let r = container
            .frame(theme::panel_frame(margin))
            .show_separator_line(false)
            .resizable(true)
            .default_size(self.settings.dock.size(side))
            .size_range(if side == Side::Bottom { 80.0..=800.0 } else { 160.0..=700.0 })
            .show(ui, |ui| {
                let mut header_act = None;
                ui.horizontal(|ui| {
                    ui.label(RichText::new(panel.title()).strong());
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let (rect, r) = ui.allocate_exact_size(egui::vec2(20.0, 20.0), egui::Sense::click());
                        if r.hovered() {
                            ui.painter().rect_filled(rect, 4.0, ui.visuals().widgets.hovered.weak_bg_fill);
                        }
                        icons::paint(ui.painter(), rect.shrink(4.0), Icon::Minus, theme::text_dim());
                        hide = r.on_hover_text("Hide").clicked();
                        if panel == Panel::Project {
                            ui.add_space(6.0);
                            let roots = self.tree_nodes();
                            header_act = self.project_tree.header_ui(ui, &roots);
                        }
                    });
                });
                if let Some(act) = header_act {
                    self.tree_act(act);
                }
                ui.add_space(4.0);
                if panel == Panel::Messages {
                    self.panel_ui(ui, panel);
                } else {
                    egui::ScrollArea::vertical()
                        .id_salt(("dock", side as u8))
                        .auto_shrink([false, false])
                        .show(ui, |ui| self.panel_ui(ui, panel));
                }
            });
        if hide {
            self.settings.dock.hide(side);
        }
        let rect = r.response.rect;
        let size = if side == Side::Bottom { rect.height() } else { rect.width() };
        self.settings.dock.set_size(side, size);
    }

    fn panel_ui(&mut self, ui: &mut egui::Ui, panel: Panel) {
        let mut message = None;
        match panel {
            Panel::Project => {
                let roots = self.tree_nodes();
                self.project_panel_ui(ui, &roots);
            }
            Panel::Properties => {
                if let Some(l) = self.focused_lib() {
                    return l.properties_ui(ui);
                }
                let view = self.active;
                let mut selection = if self.is_open(view) { self.panes.selection(view) } else { Vec::new() };
                // Nothing selected on a sub-sheet: the sheet itself.
                if let (true, Some(s)) = (selection.is_empty() && self.is_open(view), view.sheet())
                    && self.project.lib.doc.project().schematic.sheets.get(s).is_some_and(|s| !s.is_root())
                {
                    selection = vec![ObjectId::Sheet(s)];
                }
                self.properties.ui(ui, &mut self.project.lib.doc, &selection, &mut message);
            }
            Panel::ToolSettings => {
                if let Some(l) = self.focused_lib() {
                    return l.tool_settings_ui(ui);
                }
                match self.active.kind() {
                    ViewKind::Schematic if self.sch.is_placing() => {
                        sch_ui::settings(ui, &mut self.sch, &self.project.lib.doc)
                    }
                    ViewKind::Board if self.pcb.is_placing() => {
                        pcb_ui::settings(ui, &mut self.pcb, &self.project.lib.doc)
                    }
                    _ => crate::fields::hint(ui, "The select tool has no settings. Pick a tool from the tool bar."),
                }
            }
            Panel::BoardSetup => pcb_ui::board_setup(ui, &mut self.project.lib.doc, &mut message),
            Panel::Overview => match self.focused_lib() {
                Some(l) => l.overview_ui(ui),
                None => crate::properties::project_summary(ui, &self.project.lib.doc),
            },
            Panel::Messages => {
                if let Some(show) = messages::ui(ui, &mut self.project.lib.doc, &mut self.libraries) {
                    self.show_message(show);
                }
            }
        }
        if let Some(m) = message {
            self.status = Some((m, Instant::now()));
        }
    }

    /// Bring what a clicked message is about into view.
    fn show_message(&mut self, show: messages::Show) {
        match show {
            messages::Show::Objects(view, ids, pos) => {
                self.show_view(view);
                let pane = self.panes.get(view);
                if !ids.is_empty() {
                    pane.it.set_selection(&mut self.project.lib.doc.target(view), ids);
                }
                if let Some(pos) = pos {
                    let cam = &mut pane.canvas.camera;
                    cam.center = [to_mm(pos.x), to_mm(pos.y)];
                    if view == ViewId::Board {
                        cam.zoom = cam.zoom.max(25.0);
                    }
                }
            }
            messages::Show::LibraryPart(key, part) => {
                self.focus_library_tab(key);
                if let Some(l) = self.lib(key) {
                    l.show(Open::Part(part));
                }
            }
        }
    }

    /// Per frame: the Messages badge, saving the dock.
    fn update_dock(&mut self, ctx: &egui::Context) {
        let keys = messages::keys(&self.project.lib.doc, &mut self.libraries);
        self.unseen.update(keys, self.settings.dock.is_open(Panel::Messages));
        // Save once a resize is done.
        if self.settings.dock != self.saved_dock && !ctx.input(|i| i.pointer.any_down()) {
            self.saved_dock = self.settings.dock.clone();
            self.save_settings();
        }
    }

    fn status_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            let lib = self.focused_lib_ref();
            // Nothing about an editor when none is showing (all view tabs closed).
            let editor = match lib {
                Some(l) => Some({
                    let what = match l.open {
                        Open::Item(Item::Symbol(_)) => "Symbol editor",
                        Open::Item(Item::Footprint(_)) => "Footprint editor",
                        Open::Part(_) => "Part",
                        Open::None => "Library",
                    };
                    (
                        format!("{what} · {}", l.lib.name()),
                        l.canvas.cursor_world,
                        (l.it.grid, l.it.snap),
                        l.it.selection.len(),
                    )
                }),
                None if !self.is_open(self.active) => None,
                None => {
                    let view = self.hovered.unwrap_or(self.active);
                    let title = match view {
                        ViewId::Board => "PCB".to_string(),
                        ViewId::Sheet(s) => match self.project.lib.doc.project().schematic.sheet_path(s) {
                            path if path.is_empty() => "Schematic".into(),
                            path => format!("Schematic · {path}"),
                        },
                    };
                    let sel = self.panes.selection(self.active).len();
                    self.panes
                        .find(view)
                        .map(|pane| (title, pane.canvas.cursor_world, (pane.it.grid, pane.it.snap), sel))
                }
            };
            let has_editor = editor.is_some();
            if let Some((title, cursor, (grid, snap), selected)) = editor {
                ui.label(RichText::new(title).color(theme::accent()));
                match cursor {
                    Some([x, y]) => {
                        let u = units::unit();
                        let (x, y) = (x * 1e6 / u.nm(), y * 1e6 / u.nm());
                        let d = u.decimals().min(3);
                        ui.monospace(format!("X {x:>10.d$}  Y {y:>10.d$} {:<3}", u.label()))
                    }
                    None => ui.monospace(" ".repeat(29)),
                };
                ui.separator();
                ui.monospace(format!("grid {}", units::fmt(grid)));
                if !snap {
                    ui.label(RichText::new("no snap").color(theme::warn()))
                        .on_hover_text("Snapping to the grid is off (grid menu at the canvas' bottom left)");
                }
                ui.separator();
                ui.label(format!("{selected} selected"));
            }
            let msg = self.panes.iter().find_map(|p| p.it.message.clone());
            let status = self.status.as_ref().filter(|(_, t)| t.elapsed() < Duration::from_secs(6));
            if let Some(m) = msg.as_deref().or(status.map(|s| s.0.as_str())) {
                if has_editor {
                    ui.separator();
                }
                ui.label(RichText::new(m).color(theme::warn()));
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if let Some(url) = self.agent.url() {
                    let recent = self.agent.last.as_ref().filter(|(_, t)| t.elapsed() < Duration::from_secs(4));
                    let text = match recent {
                        Some((what, _)) => format!("Agent: {what}"),
                        None => "MCP".to_string(),
                    };
                    let tip = format!("{url}\n{} requests", self.agent.calls);
                    ui.label(RichText::new(text).color(theme::success())).on_hover_text(&tip);
                    // Right-to-left layout: the dot ends up left of the text.
                    let (dot, _) = ui.allocate_exact_size(egui::vec2(8.0, 8.0), egui::Sense::hover());
                    ui.painter().circle_filled(dot.center(), 3.5, theme::success());
                    if recent.is_some() {
                        ui.ctx().request_repaint_after(Duration::from_millis(500));
                    }
                    ui.separator();
                }
                if has_editor {
                    ui.label(
                        RichText::new("Drag: move · Box: select · Middle/right drag: pan · Wheel: zoom · V: fit")
                            .color(theme::text_dim()),
                    );
                }
            });
        });
    }

    /// The project's folder name, or its name while unsaved.
    fn project_name(&self) -> String {
        match &self.project.lib.doc.path {
            Some(p) => p.file_name().map_or(String::new(), |n| n.to_string_lossy().into_owned()),
            None => self.project.lib.doc.project().name.clone(),
        }
    }

    fn update_title(&mut self, ctx: &egui::Context) {
        let name = self.project_name();
        let title = format!("{}{} — PCB Designer", name, if self.project.lib.doc.is_modified() { " •" } else { "" });
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
        if !self.project.lib.doc.is_modified() && modified_libs.is_empty() {
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
        if self.script.is_some() {
            ctx.request_repaint();
        }
        if self.script.as_ref().is_some_and(|s| !s.is_done()) {
            return;
        }
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

/// Width of the panel button bars (points).
const DOCK_BAR: f32 = 36.0;

/// A panel's bar button: its icon, accent-tinted while the panel is open. `badge` is
/// (new, total) problems: a count bubble for new ones, a dot if only old ones remain.
fn dock_button(ui: &mut egui::Ui, panel: Panel, open: bool, badge: Option<(usize, usize)>, keymap: &Keymap) -> bool {
    let (rect, r) = ui.allocate_exact_size(egui::vec2(28.0, 28.0), egui::Sense::click());
    let painter = ui.painter();
    if open {
        painter.rect_filled(rect, 6.0, theme::accent().gamma_multiply(0.28));
    } else if r.hovered() {
        painter.rect_filled(rect, 6.0, ui.visuals().widgets.hovered.weak_bg_fill);
    }
    let color = if open { theme::accent() } else { ui.visuals().text_color() };
    icons::paint(painter, rect.shrink(6.0), panel.icon(), color);
    match badge {
        Some((new, _)) if new > 0 => {
            let text = if new > 99 { "99+".to_string() } else { new.to_string() };
            let font = egui::FontId::proportional(9.5);
            let galley = painter.layout_no_wrap(text, font, egui::Color32::BLACK);
            let w = (galley.size().x + 6.0).max(14.0);
            let center = rect.right_top() + egui::vec2(-4.0, 5.0);
            let bubble = egui::Rect::from_center_size(center, egui::vec2(w, 14.0));
            painter.rect_filled(bubble, 7.0, theme::warn());
            painter.galley(bubble.center() - galley.size() / 2.0, galley, egui::Color32::BLACK);
        }
        Some((_, total)) if total > 0 => {
            painter.circle_filled(rect.right_top() + egui::vec2(-5.0, 5.0), 3.5, theme::warn());
        }
        _ => {}
    }
    let shortcut = ui.ctx().format_shortcut(
        &keymap
            .shortcuts(panel.action())
            .first()
            .map(|s| s.to_egui())
            .unwrap_or(egui::KeyboardShortcut::new(egui::Modifiers::NONE, egui::Key::F35)),
    );
    let tip = if keymap.shortcuts(panel.action()).is_empty() {
        panel.title().to_string()
    } else {
        format!("{}  {shortcut}", panel.title())
    };
    r.on_hover_text(tip).clicked()
}

/// The open library tabs, as the agent's library tools see them.
struct AgentLibraries<'a>(&'a mut PcbApp);

impl mcp::library::Libraries for AgentLibraries<'_> {
    fn count(&self) -> usize {
        self.0.libraries.len()
    }

    fn get(&self, index: usize) -> &LibraryDoc {
        &self.0.libraries[index].lib
    }

    fn get_mut(&mut self, index: usize) -> &mut LibraryDoc {
        &mut self.0.libraries[index].lib
    }

    fn project(&self) -> &Document {
        &self.0.project.lib.doc
    }

    fn project_mut(&mut self) -> &mut Document {
        &mut self.0.project.lib.doc
    }

    fn missing(&self) -> Vec<MissingLibrary> {
        self.0.missing_libraries.clone()
    }

    fn push(&mut self, lib: LibraryDoc) -> usize {
        self.0.add_library(lib, false);
        self.0.libraries.len() - 1
    }

    fn remove(&mut self, index: usize) {
        let key = self.0.libraries[index].key;
        self.0.unload_library(key);
    }

    /// Show what the agent changed in its library tab (without switching tabs).
    fn changed(&mut self, index: usize, show: Option<ObjectId>) {
        let l = &mut self.0.libraries[index];
        l.refresh();
        let open = match show {
            Some(ObjectId::LibPart(id)) => Open::Part(id),
            Some(ObjectId::LibSymbol(id)) => Open::Item(Item::Symbol(id)),
            Some(ObjectId::LibFootprint(id)) => Open::Item(Item::Footprint(id)),
            _ => return,
        };
        l.show(open);
    }

    fn offscreen(&mut self) -> &mut Option<render::offscreen::Offscreen> {
        &mut self.0.agent.offscreen
    }
}

struct TreeBehavior<'a> {
    /// Tab containers: drawn as rounded panels.
    tab_groups: Vec<TileId>,
    project: &'a mut LibraryEditor,
    panes: &'a mut Panes,
    libraries: &'a mut [LibraryEditor],
    active: &'a mut ViewId,
    focus_lib: &'a mut Option<u64>,
    hovered: &'a mut Option<ViewId>,
    close_requests: &'a mut Vec<Tab>,
    /// A sheet symbol was double-clicked.
    enter_sheet: &'a mut Option<Id>,
    sch: &'a mut SchPlacer,
    pcb: &'a mut PcbPlacer,
    status: &'a mut Option<(String, Instant)>,
    keymap: &'a Keymap,
}

/// Make `sheet` the one the schematic tools work on, clearing their preview on the sheet
/// they worked on before.
fn switch_sheet(sch: &mut SchPlacer, doc: &mut Document, sheet: Id) {
    if let Some(old) = sch.set_sheet(sheet) {
        let (project, view) = doc.project_and_view(ViewId::Sheet(old));
        view.set_preview(project, &[]);
    }
}

impl TreeBehavior<'_> {
    fn view_ui(&mut self, ui: &mut egui::Ui, view: ViewId) {
        let kind = view.kind();
        let doc = &mut self.project.lib.doc;
        // The canvas fills the tab; the tool bars float over it.
        let rect = ui.max_rect();
        let pane = self.panes.get(view);
        let scene = &mut doc.view_mut(view).scene;
        let canvas = &mut pane.canvas;
        let response = ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| canvas.ui(ui, scene)).inner;
        if let Some(sheet) = view.sheet()
            && let Some(tool) = sch_ui::toolbar(ui, rect, self.sch, self.keymap)
        {
            let pane = self.panes.get(view);
            if tool != SchTool::Select {
                pane.it.reset();
                pane.it.refresh(&mut doc.target(view));
            }
            switch_sheet(self.sch, doc, sheet);
            self.sch.set_tool(tool);
            *self.active = view;
            *self.focus_lib = None;
        }
        if kind == ViewKind::Board
            && let Some(tool) = pcb_ui::toolbar(ui, rect, self.pcb, doc, self.keymap)
        {
            let pane = self.panes.get(view);
            if tool != PcbTool::Select {
                pane.it.reset();
                pane.it.refresh(&mut doc.target(view));
            }
            self.pcb.set_tool(tool);
            *self.active = view;
            *self.focus_lib = None;
        }
        let pane = self.panes.get(view);
        let entries = doc.view(view).layer_menu();
        let scene = &mut doc.view_mut(view).scene;
        let grid_kind = if kind == ViewKind::Schematic { GridKind::Schematic } else { GridKind::Board };
        let (grid, snap) = (pane.it.grid, pane.it.snap);
        let fit = toolbar::view_buttons(
            ui,
            rect,
            egui::Id::unique(("view_buttons", view)),
            self.keymap,
            |ui| toolbar::layer_menu(ui, scene, &entries),
            |ui| toolbar::grid_menu(ui, &mut pane.it.grid, &mut pane.it.snap, &mut pane.canvas.background, grid_kind),
        );
        if fit {
            pane.canvas.request_fit();
        }
        if (pane.it.grid, pane.it.snap) != (grid, snap) {
            self.panes.share_grid(view);
        }
        let pane = self.panes.get(view);
        if response.hovered() {
            *self.hovered = Some(view);
        }
        if response.hovered() && ui.input(|i| i.pointer.any_pressed()) {
            *self.active = view;
            *self.focus_lib = None;
        }
        // The schematic tools work on the sheet the pointer is in.
        let sheet = view.sheet();
        if let Some(s) = sheet
            && self.sch.is_placing()
            && response.hovered()
        {
            switch_sheet(self.sch, doc, s);
        }
        let tools_here = sheet.is_some() && self.sch.sheet == sheet;
        if sheet.is_some() && self.sch.is_placing() {
            if tools_here {
                let world = response.hover_pos().map(|p| {
                    let [x, y] = pane.canvas.screen_to_world(p);
                    pane.it.snap(Point::mm(x, y))
                });
                self.sch.hover(doc, world);
                if let Some(p) = world
                    && response.hovered()
                    && ui.input(|i| i.pointer.primary_pressed())
                    && let Err(e) = self.sch.click(doc, p)
                {
                    *self.status = Some((e, Instant::now()));
                }
            }
            if response.hovered() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
            }
        } else if kind == ViewKind::Board && self.pcb.is_placing() {
            let world = response.hover_pos().map(|p| {
                let [x, y] = pane.canvas.screen_to_world(p);
                let raw = Point::mm(x, y);
                (raw, pane.it.snap(raw))
            });
            let tol = (8.0 / pane.canvas.camera.zoom * 1e6) as Coord;
            self.pcb.hover(doc, world, tol);
            if world.is_some()
                && response.hovered()
                && ui.input(|i| i.pointer.primary_pressed())
                && let Err(e) = self.pcb.click(doc)
            {
                *self.status = Some((e, Instant::now()));
            }
            if response.hovered() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
            }
        } else {
            let engaged = !pane.it.is_idle();
            if let Some(input) = canvas::pointer_input(ui, &response, &pane.canvas, engaged, pane.it.hover.is_some()) {
                pane.it.pointer(&mut doc.target(view), &input);
            }
            // Double-clicking a sheet symbol opens the sheet.
            if response.double_clicked()
                && let Some(ObjectId::Sheet(s)) = pane.it.hover
            {
                *self.enter_sheet = Some(s);
            }
            if pane.it.is_dragging() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
            } else if pane.it.hover.is_some() && response.hovered() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            }
            if let Some((r, crossing)) = pane.it.box_rect() {
                canvas::draw_box(ui, &pane.canvas, response.rect, r, crossing);
            }
        }
        if tools_here && let Some(ghosts) = self.sch.take_preview() {
            let (project, v) = doc.project_and_view(view);
            v.set_preview(project, &ghosts);
        }
        if kind == ViewKind::Board
            && let Some(ghosts) = self.pcb.take_preview(doc)
        {
            doc.board.set_board_preview(&ghosts);
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
            Tab::View(view) => PcbApp::view_title(self.project.lib.doc.project(), *view).into(),
            Tab::Library(key) => {
                let l = if *key == PROJECT_LIB {
                    Some(&*self.project)
                } else {
                    self.libraries.iter().find(|l| l.key == *key)
                };
                l.map_or("Library".into(), |l| l.title()).into()
            }
        }
    }

    fn pane_ui(&mut self, ui: &mut egui::Ui, _tile_id: TileId, tab: &mut Tab) -> UiResponse {
        match *tab {
            Tab::View(view) => self.view_ui(ui, view),
            Tab::Library(key) => {
                let l = if key == PROJECT_LIB {
                    Some(&mut *self.project)
                } else {
                    self.libraries.iter_mut().find(|l| l.key == key)
                };
                if let Some(l) = l
                    && l.center_ui(ui, self.keymap)
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
                Some(Tab::View(view)) => {
                    *self.active = view;
                    *self.focus_lib = None;
                }
                None => {}
            }
        }
        response
    }

    fn is_tab_closable(&self, _tiles: &Tiles<Tab>, _tile_id: TileId) -> bool {
        true
    }

    fn on_tab_close(&mut self, tiles: &mut Tiles<Tab>, tile_id: TileId) -> bool {
        // Closed by the app (libraries after asking about unsaved changes).
        if let Some(tab) = Self::tab_of(tiles, tile_id) {
            self.close_requests.push(tab);
        }
        false
    }

    fn simplification_options(&self) -> SimplificationOptions {
        SimplificationOptions { all_panes_must_have_tabs: true, ..Default::default() }
    }

    fn gap_width(&self, _style: &egui::Style) -> f32 {
        f32::from(theme::GAP)
    }

    fn resize_stroke(&self, style: &egui::Style, state: egui_tiles::ResizeState) -> egui::Stroke {
        match state {
            // The gap shows the window background.
            egui_tiles::ResizeState::Idle => egui::Stroke::NONE,
            egui_tiles::ResizeState::Hovering => egui::Stroke::new(2.0, style.visuals.widgets.hovered.bg_fill),
            egui_tiles::ResizeState::Dragging => egui::Stroke::new(2.0, theme::accent()),
        }
    }

    fn tab_bar_height(&self, _style: &egui::Style) -> f32 {
        32.0
    }

    fn tab_bar_color(&self, _visuals: &egui::Visuals) -> egui::Color32 {
        theme::panel()
    }

    fn tab_bar_hline_stroke(&self, _visuals: &egui::Visuals) -> egui::Stroke {
        egui::Stroke::new(1.0, theme::border())
    }

    fn paint_on_top_of_tile(&self, painter: &egui::Painter, _style: &egui::Style, tile_id: TileId, rect: egui::Rect) {
        if self.tab_groups.contains(&tile_id) {
            theme::round_corners(painter, rect);
        }
    }

    /// Tabs as rounded pills (the active one filled, others on hover).
    fn tab_ui(
        &mut self,
        tiles: &mut Tiles<Tab>,
        ui: &mut egui::Ui,
        id: egui::Id,
        tile_id: TileId,
        state: &egui_tiles::TabState,
    ) -> egui::Response {
        let text = self.tab_title_for_tile(tiles, tile_id);
        let font_id = egui::TextStyle::Button.resolve(ui.style());
        let galley = text.into_galley(ui, Some(egui::TextWrapMode::Extend), f32::INFINITY, font_id);
        let margin = 10.0;
        let close = 14.0;
        // An icon in the colour of what the tab shows: schematic wires, copper, libraries.
        let icon = match Self::tab_of(tiles, tile_id) {
            Some(Tab::View(ViewId::Sheet(_))) => (Icon::Schematic, view::style::pal().sch_wire),
            Some(Tab::View(ViewId::Board)) => (Icon::Board, view::style::pal().top_copper),
            _ => (Icon::Library, view::style::pal().sch_label),
        };
        let icon_size = 15.0;
        let icon_gap = 6.0;
        let width =
            icon_size + icon_gap + galley.size().x + 2.0 * margin + if state.closable { close + 4.0 } else { 0.0 };
        let (_, rect) = ui.allocate_space(egui::vec2(width + 4.0, ui.available_height()));
        let pill = egui::Rect::from_min_max(
            egui::pos2(rect.left() + 4.0, rect.top() + 4.0),
            egui::pos2(rect.right(), rect.bottom() - 4.0),
        );
        let response = ui.interact(pill, id, egui::Sense::click_and_drag()).on_hover_cursor(egui::CursorIcon::Default);
        if ui.is_rect_visible(rect) && !state.is_being_dragged {
            let v = ui.visuals();
            let fill = if state.active {
                v.widgets.inactive.weak_bg_fill
            } else if response.hovered() {
                v.widgets.inactive.weak_bg_fill.gamma_multiply(0.5)
            } else {
                egui::Color32::TRANSPARENT
            };
            ui.painter().rect_filled(pill, 6, fill);
            let color = if state.active { v.strong_text_color() } else { theme::text_dim() };
            let [r, g, b, _] = icon.1.0;
            let icon_color = egui::Color32::from_rgb(r, g, b);
            let icon_color = if state.active { icon_color } else { icon_color.gamma_multiply(0.6) };
            let icon_rect = egui::Rect::from_center_size(
                egui::pos2(pill.left() + margin + icon_size / 2.0, pill.center().y),
                egui::vec2(icon_size, icon_size),
            );
            icons::paint(ui.painter(), icon_rect, icon.0, icon_color);
            let pos = egui::pos2(icon_rect.right() + icon_gap, pill.center().y - galley.size().y / 2.0);
            ui.painter().galley(pos, galley, color);
            if state.closable {
                let r = egui::Rect::from_center_size(
                    egui::pos2(pill.right() - margin - close / 2.0 + 2.0, pill.center().y),
                    egui::vec2(close, close),
                );
                let close_id = ui.auto_id_with(("tab_close", tile_id));
                let c = ui.interact(r, close_id, egui::Sense::click()).on_hover_cursor(egui::CursorIcon::Default);
                if c.hovered() {
                    ui.painter().rect_filled(r, 4, v.widgets.hovered.weak_bg_fill);
                }
                let x = r.shrink(4.0);
                let stroke =
                    egui::Stroke::new(1.2, if c.hovered() { v.strong_text_color() } else { theme::text_dim() });
                ui.painter().line_segment([x.left_top(), x.right_bottom()], stroke);
                ui.painter().line_segment([x.right_top(), x.left_bottom()], stroke);
                if (c.clicked() || response.clicked_by(egui::PointerButton::Middle))
                    && self.on_tab_close(tiles, tile_id)
                {
                    tiles.remove(tile_id);
                }
            }
        }
        self.on_tab_button(tiles, tile_id, response)
    }
}

impl eframe::App for PcbApp {
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        theme::window_bg().to_normalized_gamma_f32()
    }

    fn raw_input_hook(&mut self, _ctx: &egui::Context, raw: &mut egui::RawInput) {
        #[cfg(target_os = "macos")]
        for picked in self.native_menu.picked() {
            match picked {
                crate::native_menu::Picked::Key(s) => raw.events.push(crate::native_menu::key_event(s)),
                crate::native_menu::Picked::Cmd(cmd) => self.menu_cmds.push(cmd),
            }
        }
        let sheet = self.sheet_view();
        if let Some(script) = &mut self.script {
            let panes = &self.panes;
            let view = if script.view() == ViewKind::Board { ViewId::Board } else { sheet };
            raw.events.extend(
                script.events(|_, w| panes.find(view).map(|p| p.canvas.world_to_screen(w)).unwrap_or_default()),
            );
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.sync_libraries();
        self.handle_close(&ctx);
        // The name dialog takes the keys while it is open.
        if !self.record_shortcut(&ctx) && self.name_prompt.is_none() {
            for action in self.keymap.triggered(&ctx, self.context()) {
                self.run_action(action);
            }
        }
        for cmd in std::mem::take(&mut self.menu_cmds) {
            self.run_menu_cmd(&ctx, cmd);
        }

        let gap = theme::GAP;
        let bar = |m: egui::Margin| egui::Frame::NONE.fill(theme::window_bg()).inner_margin(m);
        self.main_menu(ui, frame, gap);
        egui::Panel::bottom("status")
            .frame(bar(egui::Margin { left: 10, right: 10, top: 4, bottom: 4 }))
            .show_separator_line(false)
            .show(ui, |ui| self.status_bar(ui));
        self.dock_bar(ui, Side::Left);
        self.dock_bar(ui, Side::Right);
        self.dock_panel(ui, Side::Left);
        self.dock_panel(ui, Side::Right);
        self.dock_panel(ui, Side::Bottom);

        self.hovered = None;
        let central = egui::Frame::NONE.fill(theme::window_bg()).inner_margin(egui::Margin {
            left: gap,
            right: gap,
            ..Default::default()
        });
        let tab_groups: Vec<TileId> = self
            .tree
            .tiles
            .iter()
            .filter(|(_, t)| matches!(t, Tile::Container(Container::Tabs(_))))
            .map(|(&id, _)| id)
            .collect();
        egui::CentralPanel::default().frame(central).show(ui, |ui| {
            let mut behavior = TreeBehavior {
                tab_groups,
                project: &mut self.project,
                panes: &mut self.panes,
                libraries: &mut self.libraries,
                active: &mut self.active,
                focus_lib: &mut self.focus_lib,
                hovered: &mut self.hovered,
                close_requests: &mut self.close_requests,
                enter_sheet: &mut self.enter_sheet,
                sch: &mut self.sch,
                pcb: &mut self.pcb,
                status: &mut self.status,
                keymap: &self.keymap,
            };
            if self.tree.root.is_none() {
                let shortcut = self.keymap.menu_text(ui.ctx(), Action::PanelProject);
                ui.centered_and_justified(|ui| {
                    ui.colored_label(
                        theme::text_dim(),
                        format!("Double-click a sheet or the PCB in the Project panel ({shortcut}) to open it"),
                    );
                });
            } else {
                self.tree.ui(&mut behavior, ui);
            }
        });
        self.follow_visible_tabs();
        // The grid menus change the display unit.
        if units::unit() != self.settings.unit {
            self.settings.unit = units::unit();
            self.save_settings();
        }
        if let Some(kind) = self.script.as_mut().and_then(Script::take_switch) {
            let view = if kind == ViewKind::Board { ViewId::Board } else { self.project.lib.doc.root() };
            self.show_view(view);
        }
        if let Some(sheet) = self.enter_sheet.take() {
            self.show_view(ViewId::Sheet(sheet));
        }
        if let ViewId::Sheet(s) = self.active {
            self.last_sheet = Some(s);
        }

        for tab in std::mem::take(&mut self.close_requests) {
            match tab {
                Tab::Library(key) => self.close_library(key),
                Tab::View(view) => self.close_view(view),
            }
        }
        if let Some(m) = self.libraries.iter_mut().find_map(|l| l.message.take()) {
            self.set_status(m);
        }
        let save_copy = self.libraries.iter_mut().find_map(|l| std::mem::take(&mut l.save_copy).then_some(l.key));
        if let Some(key) = save_copy {
            self.save_library(key, true);
        }

        // Agent requests are answered between user interactions.
        if !self.busy() {
            let libraries: Vec<&Library> = self.libraries.iter().map(|l| l.lib.library()).collect();
            let jobs =
                self.agent.process(&ctx, &mut self.project.lib.doc, &mut self.panes, &libraries, &mut self.kicad);
            for job in jobs {
                match job {
                    Job::Kicad(job, reply) => {
                        let result = self.agent_kicad_import(job).map(mcp::Reply::Json);
                        let _ = reply.send(result);
                    }
                    Job::Library(request, reply) => {
                        self.sync_libraries();
                        let _ = reply.send(mcp::library::respond(request, &mut AgentLibraries(self)));
                    }
                }
            }
            self.sync_libraries();
        }

        // Junction dots and unconnected pins follow the schematic.
        // Drags in the Project Library's editors count: they change the project.
        let busy = self.panes.iter().any(|p| !p.it.is_idle()) || self.project.busy();
        if self.project.lib.doc.update_connectivity(busy) {
            self.probe_key = None;
            ctx.request_repaint();
        }

        self.update_highlights();

        // Model changed (any source): drop stale selections, refresh highlights.
        if self.project.lib.doc.revision != self.revision {
            self.revision = self.project.lib.doc.revision;
            self.close_missing_sheets();
            for pane in self.panes.iter_mut() {
                pane.it.refresh(&mut self.project.lib.doc.target(pane.view));
            }
        }
        self.update_dock(&ctx);
        self.name_prompt_ui(&ctx);
        self.rename_ui(&ctx);
        self.settings_window(&ctx);
        let libs: Vec<(u64, String)> = self.libraries.iter().map(|l| (l.key, l.lib.name().to_string())).collect();
        self.kicad.poll();
        if let Some(ImportAction::Import(target, req)) = self.kicad.ui(&ctx, &libs) {
            self.run_kicad_import(target, req);
        }
        self.update_title(&ctx);
        self.screenshot_tick(&ctx);
    }
}

/// A sheet's node in the Project panel, with its sub-sheets (by name) below it.
fn sheet_node(p: &Project, id: Id) -> TreeNode {
    use project_panel::Info;
    let s = &p.schematic;
    let root = s.sheets.get(id).is_none_or(|sh| sh.is_root());
    let mut components: Vec<Id> = s.symbols.iter().filter(|y| y.sheet == id).map(|y| y.component).collect();
    components.sort();
    components.dedup();
    let info = Info {
        detail: if components.is_empty() { String::new() } else { project_panel::count(components.len(), "component") },
        tip: if root { "The root sheet".into() } else { s.display_path(id) },
        editable: !root,
        ..Info::icon(Icon::Schematic)
    };
    let mut children: Vec<&model::Sheet> = s.children(id).collect();
    children.sort_by(|a, b| a.name.cmp(&b.name));
    let label =
        if root { "Schematic".to_string() } else { s.sheets.get(id).map_or(String::new(), |sh| sh.name.clone()) };
    TreeNode::new(NodeId::Sheet(id), label, info)
        .with_children(children.into_iter().map(|c| sheet_node(p, c.id)).collect())
}

/// The demo project, with a demo library file (written to the temp folder) in its
/// External Libraries.
fn demo_document() -> Document {
    let mut project = view::demo::demo_project();
    let path = std::env::temp_dir().join("pcb-designer-demo").join("Demo parts.pcblib");
    let written = std::fs::create_dir_all(path.parent().unwrap())
        .map_err(|e| e.to_string())
        .and_then(|()| model::io::save_library(&view::demo::demo_library(), &path).map_err(|e| e.to_string()));
    match written {
        Ok(()) => project.libraries.push(project_libs::absolute(&path)),
        Err(e) => log::warn!("demo library not written: {e}"),
    }
    Document::new(project)
}

fn same_file(a: &std::path::Path, b: &std::path::Path) -> bool {
    a == b || matches!((a.canonicalize(), b.canonicalize()), (Ok(x), Ok(y)) if x == y)
}

/// Schematic and board as tabs of one full-size area (schematic in front). Tabs can be
/// dragged apart to work side by side.
/// Add `tab` to a tab group: `group` if it is still in the tree, else the group of the
/// first open tab of `near`, else any tab group, else a new one (all tabs were closed).
fn insert_tab(tree: &mut Tree<Tab>, tab: Tab, group: Option<TileId>, near: &[Tab]) -> TileId {
    let tiles = &tree.tiles;
    let is_tabs = |id: TileId| matches!(tiles.get(id), Some(Tile::Container(Container::Tabs(_))));
    let in_tree = |id: TileId| tree.root == Some(id) || tiles.parent_of(id).is_some();
    let group = group
        .filter(|&g| is_tabs(g) && in_tree(g))
        .or_else(|| near.iter().find_map(|t| tiles.find_pane(t)).and_then(|t| tiles.parent_of(t)))
        .or_else(|| tiles.iter().find(|(id, _)| is_tabs(**id) && in_tree(**id)).map(|(id, _)| *id));
    let tile = tree.tiles.insert_pane(tab);
    match group {
        Some(g) => {
            if let Some(Tile::Container(c)) = tree.tiles.get_mut(g) {
                c.add_child(tile);
            }
        }
        None => {
            let g = tree.tiles.insert_tab_tile(vec![tile]);
            match tree.root.and_then(|r| tree.tiles.get_mut(r)) {
                Some(Tile::Container(c)) => c.add_child(g),
                _ => tree.root = Some(g),
            }
        }
    }
    tile
}

/// The root sheet and the PCB as tabs of one group.
fn default_tree(root: ViewId) -> Tree<Tab> {
    let mut tiles = Tiles::default();
    let sch = tiles.insert_pane(Tab::View(root));
    let pcb = tiles.insert_pane(Tab::View(ViewId::Board));
    let root = tiles.insert_tab_tile(vec![sch, pcb]);
    let mut tree = Tree::new(egui::Id::unique("main_tree"), root, tiles);
    tree.make_active(|id, _| id == sch);
    tree
}

/// Schematic left, PCB right (each in its own tab group).
fn side_by_side_tree(root: ViewId) -> Tree<Tab> {
    let mut tiles = Tiles::default();
    let sch = tiles.insert_pane(Tab::View(root));
    let pcb = tiles.insert_pane(Tab::View(ViewId::Board));
    let left = tiles.insert_tab_tile(vec![sch]);
    let right = tiles.insert_tab_tile(vec![pcb]);
    let root = tiles.insert_horizontal_tile(vec![left, right]);
    Tree::new(egui::Id::unique("main_tree"), root, tiles)
}

#[cfg(test)]
mod tests {
    use super::*;

    const ROOT: ViewId = ViewId::Sheet(Id::from_u128(1));

    fn visible(tree: &Tree<Tab>) -> Vec<Tab> {
        tree.active_tiles().into_iter().filter_map(|t| tree.tiles.get_pane(&t).copied()).collect()
    }

    #[test]
    fn default_layout_is_tabs_with_the_schematic_in_front() {
        let mut tree = default_tree(ROOT);
        let root = tree.root.unwrap();
        assert!(matches!(tree.tiles.get(root), Some(Tile::Container(Container::Tabs(t))) if t.children.len() == 2));
        assert_eq!(visible(&tree), vec![Tab::View(ROOT)]);
        let pcb = tree.tiles.find_pane(&Tab::View(ViewId::Board)).unwrap();
        tree.make_active(|id, _| id == pcb);
        assert_eq!(visible(&tree), vec![Tab::View(ViewId::Board)]);
        assert_eq!(visible(&side_by_side_tree(ROOT)).len(), 2, "both views visible side by side");
    }

    /// Close a tab as the app does, then let the tree tidy up as it does when drawn.
    fn close(tree: &mut Tree<Tab>, tab: Tab) -> Option<TileId> {
        let tile = tree.tiles.find_pane(&tab).unwrap();
        let group = tree.tiles.parent_of(tile);
        tree.remove_recursively(tile);
        tree.simplify(&SimplificationOptions { all_panes_must_have_tabs: true, ..Default::default() });
        group
    }

    fn group_of(tree: &Tree<Tab>, tab: Tab) -> Option<TileId> {
        tree.tiles.find_pane(&tab).and_then(|t| tree.tiles.parent_of(t))
    }

    #[test]
    fn closed_views_reopen_where_they_were() {
        let (sch, pcb) = (Tab::View(ROOT), Tab::View(ViewId::Board));

        // Tabs: back into the same group.
        let mut tree = default_tree(ROOT);
        let group = close(&mut tree, pcb);
        insert_tab(&mut tree, pcb, group, &[sch]);
        assert_eq!(group_of(&tree, pcb), group);

        // Side by side: the PCB's group went away with it, so it joins the schematic.
        let mut tree = side_by_side_tree(ROOT);
        let group = close(&mut tree, pcb);
        insert_tab(&mut tree, pcb, group, &[sch]);
        assert_eq!(group_of(&tree, pcb), group_of(&tree, sch));

        // Everything closed: a new tab group becomes the root.
        let mut tree = default_tree(ROOT);
        close(&mut tree, pcb);
        let group = close(&mut tree, sch);
        assert_eq!(tree.root, None);
        let tile = insert_tab(&mut tree, sch, group, &[pcb]);
        tree.make_active(|id, _| id == tile);
        assert_eq!(visible(&tree), vec![sch]);
    }
}
