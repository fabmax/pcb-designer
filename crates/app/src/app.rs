//! Main window: menu, side panels, docked schematic / PCB / library editors, status bar.
//!
//! Focus is either a project view (schematic or PCB) or a library tab; the side panels,
//! keyboard shortcuts and Save follow the focus.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use editor_core::Command;
use editor_core::clip::Clip;
use editor_core::query::{PartState, part_statuses};
use egui::{RichText, WidgetText};
use egui_tiles::{Behavior, Container, SimplificationOptions, Tile, TileId, Tiles, Tree, UiResponse};
use model::*;
use view::item::Item;
use view::pcb_tools::{PcbPlacer, PcbSettings, PcbTool};
use view::sch_tools::{SchPlacer, SchSettings, SchTool};

use crate::name_prompt::NamePrompt;
use crate::sheet_picker::SheetPicker;
use view::project_libs::{self, MissingLibrary};
use view::tools::Tool;
use view::{Document, Interaction, KeyAction, LibraryDoc, ViewId, ViewKind};

use crate::agent::{Agent, Job};
use crate::canvas::{self, Canvas, CanvasResources};
use crate::clipboard;
use crate::dock::{Dock, DropZone, Group as DockGroup, Panel, Side, Unseen};
use crate::icons::{self, Icon};
use crate::item_tabs;
use crate::keymap::{Action, Context, Keymap, Shortcut};
use crate::kicad_import::{ImportAction, KicadImport};
use crate::libraries::Libraries;
use crate::library::{ItemEditor, ItemTab, LibraryEditor, Open, PROJECT_LIB};
use crate::menu::{self, Cmd, Menu, MenuState};
use crate::messages;
use crate::nets_ui::{NetsAct, NetsPanel};
use crate::outputs::{self, GerberAct, GerberPane, Outputs};
use crate::pcb_ui;
use crate::plot_ui::PlotAct;
use crate::project_panel::{self, Group, NodeId, Owner, ProjectPanel, TreeNode};
use crate::properties::Properties;
use crate::sch_ui;
use crate::script::Script;
use crate::settings::Settings;
use crate::settings_ui::{AgentCmd, AgentStatus, Page as SettingsPage, SettingsWindow};
use crate::sim_ui::{SimAct, SimPanel};
use crate::theme;
#[cfg(target_os = "macos")]
use crate::title_bar;
use crate::toolbar;
use crate::units::{self, GridKind};
use crate::workspace::{self, ItemRef, ItemState, LibRef, TabRef, ViewState, Workspace};
use view::sim_overlay::{self, ProbeTarget};

pub enum Startup {
    Open(PathBuf),
    /// Import a KiCad project (`.kicad_pro`) as a new, unsaved project.
    ImportKicad(PathBuf),
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

#[derive(Clone)]
enum Naming {
    /// A library item, by library key.
    Item(u64, ObjectId),
    /// A category of a library's group (parts, symbols or footprints), by library key.
    Category(u64, project_panel::Group, String),
    Sheet(Id),
    /// A new sheet below this one.
    NewSheet(Id),
}

/// Content of a dock tab.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Tab {
    /// A schematic sheet or the board.
    View(ViewId),
    /// A library item, by [`ItemTab::id`].
    Item(u64),
    /// The Gerber export, read back from disk (read-only).
    Gerber,
    /// The simulation plot of the analysis the Simulation panel shows.
    Simulation,
}

/// One editor pane: camera + interaction state for a view of the document.
pub(crate) struct Pane {
    pub view: ViewId,
    pub canvas: Canvas,
    pub it: Interaction<ObjectId>,
    /// A press landed on the text being edited: drags select in it.
    pub text_drag: bool,
    /// The corner handle under the pointer (board: a selected zone, keepout or drawing).
    pub grip_hover: Option<(ObjectId, view::handles::Grip)>,
    /// A corner handle being dragged.
    pub grip_drag: Option<GripDrag>,
}

/// A corner handle drag: the object as it was, the handle, its offset from the pointer,
/// the undo group, the snap points (built once at the start) and whether it moved yet.
pub(crate) struct GripDrag {
    original: Object,
    grip: view::handles::Grip,
    offset: Point,
    group: u64,
    snap: view::measure::BoardSnap,
    moved: bool,
    start: egui::Pos2,
}

impl Pane {
    fn new(view: ViewId) -> Self {
        let grid = match view.kind() {
            ViewKind::Schematic => 1.27,
            ViewKind::Board => 0.5,
        };
        let canvas = Canvas::new(view::style::background(grid as f32));
        Pane { view, canvas, it: Interaction::new(mm(grid)), text_drag: false, grip_hover: None, grip_drag: None }
    }
}

/// The panes of the board and of the schematic sheets (made when first needed). Sheet
/// panes share their grid settings.
pub(crate) struct Panes {
    list: Vec<Pane>,
}

impl Panes {
    fn new() -> Self {
        Panes { list: vec![Pane::new(ViewId::Board)] }
    }

    /// The pane of `view`, made if there is none yet.
    pub fn get(&mut self, view: ViewId) -> &mut Pane {
        let i = match self.list.iter().position(|p| p.view == view) {
            Some(i) => i,
            None => {
                let mut pane = Pane::new(view);
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

/// An item tab's item in the workspace.
fn item_ref(open: Open) -> ItemRef {
    match open {
        Open::Item(Item::Symbol(id)) => ItemRef::Symbol(id),
        Open::Item(Item::Footprint(id)) => ItemRef::Footprint(id),
        Open::Part(id) => ItemRef::Part(id),
    }
}

fn open_of_ref(item: ItemRef) -> Open {
    match item {
        ItemRef::Symbol(id) => Open::Item(Item::Symbol(id)),
        ItemRef::Footprint(id) => Open::Item(Item::Footprint(id)),
        ItemRef::Part(id) => Open::Part(id),
    }
}

/// Work the tab tree asks for while it is drawn (when it can't borrow all of
/// `PcbApp`), done right after.
enum Deferred {
    /// A tab's close button was clicked.
    Close(Tab),
    /// A sheet symbol was double-clicked: open its sheet.
    EnterSheet(Id),
    Gerber(GerberAct),
    Plot(PlotAct),
    /// Picked in a canvas's context menu.
    Action(Action),
    /// An item tab asked to open another item ("Edit footprint" of a part).
    Navigate(u64, Open),
    /// "Tune R1" in a schematic's context menu: a tuner row for the component.
    Tune(Id),
}

/// Which editor takes keyboard shortcuts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Focus {
    /// The project pane [`PcbApp::active`].
    Project,
    /// A library item tab, by [`ItemTab::id`] (the Project Library's too).
    Item(u64),
    /// The Gerber tab: it takes the measure keys; other view keys do nothing.
    Gerber,
    /// The Simulation tab: fit and Esc; other view keys do nothing.
    Simulation,
}

impl Focus {
    fn item(self) -> Option<u64> {
        if let Focus::Item(tab) = self { Some(tab) } else { None }
    }
}

pub struct PcbApp {
    tree: Tree<Tab>,
    /// The project: its document, edited as the Project Library in its tab (key
    /// [`PROJECT_LIB`]) and as schematic and board in the panes.
    project: LibraryEditor,
    panes: Panes,
    /// The project pane worked on: it takes keyboard shortcuts while `focus` is
    /// [`Focus::Project`], and stays the target of project actions while it isn't.
    active: ViewId,
    focus: Focus,
    /// The sheet shown last (where new sheets and placed parts go when the board is active).
    last_sheet: Option<Id>,
    /// Text a keyboard paste brought this frame.
    paste_text: Option<String>,
    /// Clip text to put on the system clipboard at the end of the frame.
    copy_out: Option<String>,
    /// The clip being carried and the view it was pasted into; it moves to the sheet
    /// under the pointer.
    carry_clip: Option<(ViewId, Clip)>,
    hovered: Option<ViewId>,
    properties: Properties,
    libraries: Libraries,
    /// The tab group a closed view was in, to reopen it there.
    view_groups: Vec<(ViewId, TileId)>,
    status: Option<(String, Instant)>,
    title: String,
    revision: u64,
    screenshot: Option<(String, u32)>,
    allow_close: bool,
    keymap: Keymap,
    settings: Settings,
    settings_win: SettingsWindow,
    /// The port `--mcp` / `--mcp-port` gave (over the settings' one, for this run).
    mcp_port: Option<u16>,
    /// The Nets panel's state.
    nets: NetsPanel,
    sim: SimPanel,
    parts: crate::parts_ui::PartsPanel,
    /// A panel button being dragged to another place in the bars.
    dock_drag: Option<Panel>,
    /// Where the bars' groups were drawn this frame (drop targets).
    drop_zones: Vec<DropZone>,
    /// The tool settings changed (saved a second later).
    tools_changed: Option<Instant>,
    /// The main window as of the last frame (saved on quit).
    window_state: Option<crate::settings::WindowState>,
    /// The report of the last KiCad project import, shown until dismissed.
    import_report: Option<ImportReport>,
    agent: Agent,
    kicad: KicadImport,
    /// Schematic creation tools.
    sch: SchPlacer,
    /// Asking for the name of a new bus or of a signal leaving one.
    name_prompt: Option<NamePrompt>,
    /// "Move to sheet…" asking for the sheet.
    sheet_picker: Option<SheetPicker>,
    /// Array mode opened the Tool settings panel: the item tab and the panel it
    /// replaced, shown again when the mode ends.
    array_dock: Option<(u64, Option<Panel>)>,
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
    /// The Gerber export folder (Project panel: Outputs).
    outputs: Outputs,
    /// The project folder `outputs` was scanned for, and whether the window had focus.
    outputs_scanned: Option<Option<PathBuf>>,
    window_focused: bool,
    /// The export is older than the design (as of this frame).
    outputs_outdated: bool,
    gerber: GerberPane,
    /// What the tab tree asked for while it was drawn.
    deferred: Vec<Deferred>,
    /// Counts item tab uses ([`ItemTab::used`]).
    tab_clock: u64,
    /// The Messages panel's tab, and whether the panel was open last frame.
    message_tab: messages::Category,
    messages_open: bool,
    #[cfg(target_os = "macos")]
    native_menu: crate::native_menu::NativeMenu,
    pointer_filter: crate::pointer_filter::PointerFilter,
    #[cfg(target_os = "macos")]
    title_bar: title_bar::TitleBar,
}

impl PcbApp {
    pub fn new(cc: &eframe::CreationContext<'_>, opts: Options) -> Self {
        let mut settings = Settings::load();
        settings.dock.normalize();
        view::style::set_theme(settings.theme);
        view::style::set_layer_colors(settings.layer_colors());
        units::set_unit(settings.unit);
        // ⌘+ / ⌘− zoom the canvases, not the UI (Settings → Appearance → UI scale).
        cc.egui_ctx.options_mut(|o| o.zoom_with_keyboard = false);
        cc.egui_ctx.set_zoom_factor(settings.ui_scale());
        theme::install_fonts(&cc.egui_ctx);
        theme::apply(&cc.egui_ctx);
        theme::apply_to_window(&cc.egui_ctx);
        let keymap = opts.keymap.unwrap_or_else(|| settings.keymap());
        let rs = cc.wgpu_render_state.as_ref().expect("wgpu renderer");
        CanvasResources::install(rs);

        let mut status = None;
        let mut import_report = None;
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
            Startup::ImportKicad(path) => match kicad::import_project(&path) {
                Ok(import) => {
                    import_report = Some(ImportReport::new(&path, import.report));
                    Document::new(import.project)
                }
                Err(e) => {
                    status = Some((format!("KiCad import failed: {e}"), Instant::now()));
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
        let opened = doc.path.is_some();
        let mcp_port = opts.mcp.unwrap_or(settings.mcp.port);
        let start_mcp = opts.mcp.is_some() || (settings.mcp.start && opts.screenshot.is_none());
        let kicad_root = settings.kicad_dir();
        let mut sch = SchPlacer::default();
        sch.settings = view::tool_memory::recall(settings.tools.get("schematic"));
        let mut pcb = PcbPlacer::default();
        pcb.settings = view::tool_memory::recall(settings.tools.get("board"));
        let tree = if opts.side_by_side { side_by_side_tree(root) } else { default_tree(root) };
        let mut app = Self {
            tree,
            project: LibraryEditor::new(PROJECT_LIB, LibraryDoc { doc, path: None }),
            panes: Panes::new(),
            active: root,
            last_sheet: root.sheet(),
            focus: Focus::Project,
            deferred: Vec::new(),
            tab_clock: 0,
            message_tab: messages::Category::default(),
            messages_open: false,
            paste_text: None,
            copy_out: None,
            carry_clip: None,
            hovered: None,
            properties: Properties::default(),
            libraries: Libraries::default(),
            outputs: Outputs::default(),
            outputs_scanned: None,
            window_focused: true,
            outputs_outdated: false,
            gerber: GerberPane::default(),
            view_groups: Vec::new(),
            status,
            title: String::new(),
            revision: u64::MAX,
            screenshot: opts.screenshot.map(|p| (p, opts.screenshot_frames)),
            allow_close: false,
            keymap,
            settings,
            settings_win: SettingsWindow::default(),
            mcp_port: opts.mcp,
            tools_changed: None,
            nets: NetsPanel::default(),
            sim: SimPanel::default(),
            parts: Default::default(),
            dock_drag: None,
            drop_zones: Vec::new(),
            window_state: None,
            import_report,
            agent: Agent::new(mcp_port),
            kicad: KicadImport::new(opts.kicad_paths.clone(), kicad_root),
            sch,
            name_prompt: None,
            sheet_picker: None,
            array_dock: None,
            saved_dock: Dock::default(),
            project_tree: ProjectPanel::default(),
            rename: None,
            unseen: Unseen::default(),
            pcb,
            probe_key: None,
            script: None,
            menu_cmds: Vec::new(),
            #[cfg(target_os = "macos")]
            native_menu: crate::native_menu::NativeMenu::new(&cc.egui_ctx),
            pointer_filter: Default::default(),
            #[cfg(target_os = "macos")]
            title_bar: title_bar::TitleBar::default(),
        };
        match opts.script.as_deref().map(Script::parse) {
            Some(Ok(mut s)) => {
                if !opts.show.is_empty() {
                    s.keep_tab();
                }
                app.script = Some(s);
            }
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
            if name == "gerber" {
                app.refresh_outputs();
                app.open_gerber(None);
            } else if name == "simulation" {
                app.open_sim_tab(true);
            } else if let Some(page) = name.strip_prefix("settings") {
                let page = page.strip_prefix(':').and_then(SettingsPage::parse);
                app.settings_win.show(page, &mut app.settings);
            } else {
                app.show_by_name(name);
            }
        }
        for name in &opts.panels {
            match Panel::parse(name) {
                Some(p) => app.settings.dock.show(p),
                None => eprintln!("unknown panel {name:?}"),
            }
        }
        app.saved_dock = app.settings.dock.clone();
        if start_mcp {
            app.agent.start(&cc.egui_ctx);
        }
        // The project's saved tabs and cameras (the command line's layout wins).
        if opened && !opts.side_by_side {
            app.restore_workspace();
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
        let doc = &self.doc();
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
        if self.name_prompt.is_none() && self.sheet_picker.is_none() {
            let grid = self.panes.get(self.sheet_view()).it.grid;
            self.name_prompt = self.sch.take_prompt(&mut self.project.lib.doc, grid).map(NamePrompt::new);
        }
        let Some(dialog) = &mut self.name_prompt else { return };
        let Some(answer) = dialog.ui(ctx) else { return };
        let command = answer.map(|name| dialog.prompt.label(&name));
        self.name_prompt = None;
        match command {
            Some(Ok(cmd)) => {
                if let Err(e) = self.doc_mut().execute(cmd) {
                    self.set_status(e.to_string());
                }
            }
            Some(Err(e)) => self.set_status(e),
            None => {}
        }
    }

    /// "Move to sheet…": ask where the active sheet's selection goes.
    /// The selected objects of the sheet being edited, which "Move to sheet…" takes.
    fn movable_to_sheet(&self) -> Vec<ObjectId> {
        let view = self.sheet_view();
        let Some(from) = view.sheet() else { return Vec::new() };
        movable_ids(self.doc().project(), &self.panes.selection(view), from)
    }

    fn move_to_sheet(&mut self) {
        let Some(from) = self.sheet_view().sheet() else { return };
        let ids = self.movable_to_sheet();
        if ids.is_empty() {
            return self.set_status("Select schematic objects to move to another sheet");
        }
        match SheetPicker::new(self.doc().project(), ids, from) {
            Some(p) => self.sheet_picker = Some(p),
            None => self.set_status("There is no other sheet to move them to"),
        }
    }

    /// The sheet picker's dialog; once a sheet is chosen, move the objects there and let
    /// them follow the pointer on it (Esc undoes the move).
    fn sheet_picker_ui(&mut self, ctx: &egui::Context) {
        let Some(picker) = &mut self.sheet_picker else { return };
        let Some(answer) = picker.ui(ctx) else { return };
        let ids = std::mem::take(&mut picker.ids);
        self.sheet_picker = None;
        let Some(target) = answer else { return };
        let from = self.sheet_view();
        let group = view::interaction::new_group();
        let renamed = match self.move_to_sheet_cmd(ids.clone(), target, group) {
            Ok(n) => n,
            Err(e) => return self.set_status(e),
        };
        let doc = &mut self.project.lib.doc;
        let pane = self.panes.get(from);
        pane.it.set_selection(&mut doc.target(from), Vec::new());
        let to = ViewId::Sheet(target);
        self.show_view(to);
        let doc = &mut self.project.lib.doc;
        let carried: Vec<ObjectId> = ids.into_iter().filter(|id| !matches!(id, ObjectId::SymbolField(_))).collect();
        let anchor = carried.iter().find_map(|&id| view::interaction::reference_point(doc.project(), id));
        let anchor = anchor.unwrap_or_default();
        self.panes.get(to).it.carry(&mut doc.target(to), carried, anchor, anchor, group);
        let path = doc.project().schematic.display_path(target);
        self.set_status(format!("Moved to {path}{}. Click to place them, Esc undoes the move", renamed_note(renamed)));
    }

    /// Run [`Command::MoveToSheet`] as part of undo `group`. Returns how many nets lost
    /// their name by it (the move doesn't fix connections, it tells about them).
    fn move_to_sheet_cmd(&mut self, ids: Vec<ObjectId>, sheet: Id, group: u64) -> Result<usize, String> {
        let doc = &mut self.project.lib.doc;
        let nets = |doc: &mut Document| -> std::collections::HashSet<String> {
            doc.current_connectivity().nets.iter().map(|n| n.name.clone()).filter(|n| !n.is_empty()).collect()
        };
        let before = nets(doc);
        doc.execute_grouped(Command::MoveToSheet { ids, sheet }, Some(group)).map_err(|e| e.to_string())?;
        Ok(before.difference(&nets(doc)).count())
    }

    fn set_status(&mut self, msg: impl Into<String>) {
        self.status = Some((msg.into(), Instant::now()));
    }

    fn busy(&self) -> bool {
        self.panes.iter().any(|p| !p.it.is_idle()) || self.all_libs().any(|l| l.tabs.iter().any(|t| !t.it.is_idle()))
    }

    /// A library editor by key, including the Project Library's.
    /// The project document (also edited as the Project Library in its tab).
    fn doc(&self) -> &Document {
        &self.project.lib.doc
    }

    fn doc_mut(&mut self) -> &mut Document {
        &mut self.project.lib.doc
    }

    /// The document with a text being edited in place: the project's or a library's.
    fn editing_doc(&mut self) -> Option<&mut Document> {
        if self.project.lib.doc.text_edit().is_some() {
            return Some(&mut self.project.lib.doc);
        }
        self.libraries.iter_mut().map(|l| &mut l.lib.doc).find(|d| d.text_edit().is_some())
    }

    fn lib_ref(&self, key: u64) -> Option<&LibraryEditor> {
        if key == PROJECT_LIB {
            return Some(&self.project);
        }
        self.libraries.by_key(key)
    }

    fn lib(&mut self, key: u64) -> Option<&mut LibraryEditor> {
        if key == PROJECT_LIB {
            return Some(&mut self.project);
        }
        self.libraries.by_key_mut(key)
    }

    /// The Project Library and the loaded libraries.
    fn all_libs(&self) -> impl Iterator<Item = &LibraryEditor> {
        std::iter::once(&self.project).chain(self.libraries.iter())
    }

    fn all_libs_mut(&mut self) -> impl Iterator<Item = &mut LibraryEditor> {
        std::iter::once(&mut self.project).chain(self.libraries.iter_mut())
    }

    /// The library an item tab belongs to.
    fn lib_of_tab(&self, tab: u64) -> Option<&LibraryEditor> {
        self.all_libs().find(|l| l.tab(tab).is_some())
    }

    /// An item tab with its library, for editing.
    fn editor(&mut self, tab: u64) -> Option<ItemEditor<'_>> {
        self.all_libs_mut().find(|l| l.tab(tab).is_some())?.editor(tab)
    }

    /// The library of the focused item tab.
    fn focused_lib_key(&self) -> Option<u64> {
        self.lib_of_tab(self.focus.item()?).map(|l| l.key)
    }

    /// The Gerber tab is open and has focus.
    fn gerber_focused(&self) -> bool {
        self.focus == Focus::Gerber && self.tree.tiles.find_pane(&Tab::Gerber).is_some()
    }

    /// The library of the focused item tab (the Project Library's too).
    fn focused_lib_ref(&self) -> Option<&LibraryEditor> {
        self.lib_of_tab(self.focus.item()?)
    }

    /// The focused item tab.
    fn focused_tab(&self) -> Option<&ItemTab> {
        let tab = self.focus.item()?;
        self.lib_of_tab(tab)?.tab(tab)
    }

    // ------------------------------------------------------------------------------------
    // Workspace (tabs, cameras, hidden layers; saved in the project folder)

    /// How the workspace names a library (`None`: unsaved, not kept).
    fn lib_ref_of(&self, l: &LibraryEditor) -> Option<LibRef> {
        Some(match &l.lib.path {
            _ if l.is_project() => LibRef::Project,
            Some(p) => LibRef::File(project_libs::absolute(p)),
            None if l.is_builtin() => LibRef::Builtin,
            None => return None,
        })
    }

    /// The loaded library a workspace reference names.
    fn lib_key_of(&self, r: &LibRef) -> Option<u64> {
        Some(match r {
            LibRef::Project => PROJECT_LIB,
            LibRef::Builtin => self.libraries.iter().find(|l| l.lib.is_builtin())?.key,
            LibRef::File(path) => {
                self.libraries.iter().find(|l| l.lib.path.as_deref().is_some_and(|p| same_file(p, path)))?.key
            }
        })
    }

    /// A tab's reference in the workspace.
    fn tab_ref(&self, tab: Tab) -> Option<TabRef> {
        Some(match tab {
            Tab::View(v) => workspace::view_ref(v),
            Tab::Gerber => TabRef::Gerber,
            Tab::Simulation => TabRef::Simulation,
            Tab::Item(id) => {
                let l = self.lib_of_tab(id)?;
                let t = l.tab(id)?;
                TabRef::Library { lib: self.lib_ref_of(l)?, item: item_ref(t.open), preview: t.preview }
            }
        })
    }

    fn workspace(&self) -> Workspace {
        let doc = self.doc();
        let views = self
            .panes
            .iter()
            .map(|p| ViewState {
                view: workspace::view_ref(p.view),
                center: p.canvas.camera.center,
                zoom: p.canvas.camera.zoom,
                hidden: doc.view(p.view).hidden_layers(),
            })
            .collect();
        let mut items: Vec<(u64, ItemState)> = self
            .all_libs()
            .filter_map(|l| Some((l, self.lib_ref_of(l)?)))
            .flat_map(|(l, lib)| {
                l.tabs.iter().map(move |t| {
                    let camera = &t.canvas.camera;
                    let state = ItemState {
                        lib: lib.clone(),
                        item: item_ref(t.open),
                        center: camera.center,
                        zoom: camera.zoom,
                    };
                    (t.used, state)
                })
            })
            .collect();
        items.sort_by_key(|(used, _)| *used);
        let active = match self.focus {
            Focus::Project => Some(Tab::View(self.active)),
            Focus::Item(id) => Some(Tab::Item(id)),
            Focus::Gerber => Some(Tab::Gerber),
            Focus::Simulation => Some(Tab::Simulation),
        };
        Workspace {
            layout: workspace::layout_of(&self.tree, &|t| self.tab_ref(*t)),
            active: active.and_then(|t| self.tab_ref(t)),
            views,
            items: items.into_iter().map(|(_, s)| s).collect(),
            layer: Some(self.pcb.settings.layer),
        }
    }

    /// Save the workspace into the project folder (saved projects only; not in scripted
    /// runs).
    fn save_workspace(&mut self) {
        let Some(dir) = self.doc().path.clone() else { return };
        if self.screenshot.is_some() || self.script.is_some() {
            return;
        }
        if let Err(e) = self.workspace().save(&dir) {
            log::warn!("workspace not saved: {e}");
        }
    }

    /// The tab a saved reference names, if it still exists (item tabs only of loaded
    /// libraries' items; made if there is none).
    fn open_tab_ref(&mut self, r: &TabRef) -> Option<Tab> {
        match r {
            TabRef::Sheet(s) => {
                self.doc().project().schematic.sheets.contains(*s).then_some(Tab::View(ViewId::Sheet(*s)))
            }
            TabRef::Board => Some(Tab::View(ViewId::Board)),
            TabRef::Gerber => self.outputs.dir.is_some().then_some(Tab::Gerber),
            TabRef::Simulation => Some(Tab::Simulation),
            TabRef::Library { lib, item, preview } => {
                let key = self.lib_key_of(lib)?;
                let open = open_of_ref(*item);
                let l = self.lib(key)?;
                if !l.exists(open) {
                    return None;
                }
                let id = match l.tab_of(open) {
                    Some(t) => t.id,
                    None => l.add_tab(open, *preview),
                };
                Some(Tab::Item(id))
            }
        }
    }

    /// Restore the saved workspace of the open project, if it has one.
    fn restore_workspace(&mut self) {
        let Some(dir) = self.doc().path.clone() else { return };
        let Some(ws) = Workspace::load(&dir) else { return };
        self.refresh_outputs();
        if let Some(layout) = &ws.layout
            && let Some(tree) = workspace::tree_of(layout, egui::Id::unique("main_tree"), &mut |r| self.open_tab_ref(r))
        {
            self.tree = tree;
            self.view_groups.clear();
            if self.tree.tiles.find_pane(&Tab::Gerber).is_some()
                && let Some(d) = self.outputs.dir.clone()
            {
                self.gerber.load(&d, None);
            }
        }
        self.drop_orphan_tabs();
        for v in &ws.views {
            let Some(view) = workspace::view_of(&v.view) else { continue };
            if view.sheet().is_some_and(|s| !self.doc().project().schematic.sheets.contains(s)) {
                continue;
            }
            if v.zoom.is_finite() && v.zoom > 0.0 {
                let camera = render::Camera { center: v.center, zoom: v.zoom };
                self.panes.get(view).canvas.set_camera(camera);
            }
            self.doc_mut().view_mut(view).set_hidden_layers(&v.hidden);
        }
        for s in &ws.items {
            let Some(key) = self.lib_key_of(&s.lib) else { continue };
            let used = self.tick();
            let Some(t) = self.lib(key).and_then(|l| l.tabs.iter_mut().find(|t| t.open == open_of_ref(s.item))) else {
                continue;
            };
            t.used = used;
            if s.zoom.is_finite() && s.zoom > 0.0 {
                t.canvas.set_camera(render::Camera { center: s.center, zoom: s.zoom });
            }
        }
        if let Some(layer) = ws.layer.filter(|l| self.doc().project().board.layers.iter().any(|x| x.id == *l)) {
            self.pcb.settings.layer = layer;
        }
        let active = ws.active.as_ref().and_then(|r| self.open_tab_ref(r));
        if let Some(tab) = active.filter(|t| self.tree.tiles.find_pane(t).is_some()) {
            let tile = self.tree.tiles.find_pane(&tab);
            self.tree.make_active(|id, _| Some(id) == tile);
            match tab {
                Tab::View(v) => {
                    self.active = v;
                    self.focus = Focus::Project;
                    if let Some(s) = v.sheet() {
                        self.last_sheet = Some(s);
                    }
                }
                Tab::Item(id) => self.focus = Focus::Item(id),
                Tab::Gerber => self.focus = Focus::Gerber,
                Tab::Simulation => self.focus = Focus::Simulation,
            }
        }
        self.drop_orphan_tabs();
        self.follow_visible_tabs();
    }

    // ------------------------------------------------------------------------------------
    // Project files

    fn replace_document(&mut self, doc: Document) {
        self.save_workspace();
        let old_root = self.doc().root();
        self.project = LibraryEditor::new(PROJECT_LIB, LibraryDoc { doc, path: None });
        self.sync_libraries();
        // The new root sheet takes the old one's tab; tabs of other sheets close.
        let root = self.doc().root();
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
        self.sim.clear();
        self.sch.set_tool(SchTool::Select);
        let settings = self.pcb.settings.clone();
        self.pcb = PcbPlacer::default();
        self.pcb.settings = PcbSettings { layer: LayerId::TOP_COPPER, ..settings };
        self.restore_workspace();
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
            self.active = self.doc().root();
        }
        self.follow_visible_tabs();
    }

    /// Ask to save unsaved changes. Returns false if the user cancelled.
    fn confirm_discard(&mut self) -> bool {
        if !self.doc().is_modified() {
            return true;
        }
        match ask_save(&format!("Save changes to \"{}\"?", self.doc().project().name)) {
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

    /// File → Import KiCad project: the result opens as a new, unsaved project.
    fn import_kicad_project(&mut self) {
        if !self.confirm_discard() {
            return;
        }
        let Some(path) = rfd::FileDialog::new()
            .set_title("Import KiCad project")
            .add_filter("KiCad project", &["kicad_pro"])
            .pick_file()
        else {
            return;
        };
        match kicad::import_project(&path) {
            Ok(import) => self.open_import(&path, import),
            Err(e) => self.set_status(format!("KiCad import failed: {e}")),
        }
    }

    /// Open an imported project (after unsaved changes were dealt with) and show its report.
    fn open_import(&mut self, path: &std::path::Path, import: kicad::ProjectImport) {
        self.replace_document(Document::new(import.project));
        self.set_status(format!("Imported {} (not saved yet)", path.display()));
        self.import_report = Some(ImportReport::new(path, import.report));
    }

    /// An agent's project import: replaces the project unless the user cancels.
    fn agent_replace_project(&mut self, import: kicad::ProjectImport) -> Result<serde_json::Value, String> {
        if !self.confirm_discard() {
            return Err("the user kept the open project (it has unsaved changes)".into());
        }
        let reply = mcp::host::project_import_reply(&import.report);
        let name = import.project.name.clone();
        self.open_import(std::path::Path::new(&name), import);
        Ok(reply)
    }

    /// The report of the last KiCad project or parts import.
    fn import_report_ui(&mut self, ctx: &egui::Context) {
        let Some(r) = &self.import_report else { return };
        let mut close = false;
        let modal = theme::dialog(ctx, egui::Id::unique("kicad_import_report"), &r.title, 520.0, |ui| {
            if let Some(summary) = &r.summary {
                let color = if r.errors.is_empty() { theme::success() } else { theme::warn() };
                ui.label(RichText::new(summary).color(color));
                ui.add_space(8.0);
            }
            // Long reports scroll inside a framed area, at most half the window tall.
            let max_height = (ctx.content_rect().height() * 0.5).max(120.0);
            egui::Frame::NONE.fill(theme::panel_dark()).corner_radius(6).inner_margin(egui::Margin::same(8)).show(
                ui,
                |ui| {
                    egui::ScrollArea::vertical().max_height(max_height).auto_shrink([false, true]).show(ui, |ui| {
                        let report = &r.report;
                        if report.is_empty() && r.errors.is_empty() {
                            ui.label("Everything was imported.");
                        }
                        let section =
                            |ui: &mut egui::Ui, title: &str, lines: &[String], color: Option<egui::Color32>| {
                                if lines.is_empty() {
                                    return;
                                }
                                if ui.min_rect().height() > 0.0 {
                                    ui.add_space(6.0);
                                }
                                ui.strong(title);
                                for l in lines {
                                    let text = RichText::new(format!("• {l}"));
                                    ui.label(if let Some(c) = color { text.color(c) } else { text });
                                }
                            };
                        section(ui, "Failed", &r.errors, Some(theme::warn()));
                        section(ui, "Warnings", &report.warnings, Some(theme::warn()));
                        section(ui, "Notes", &report.notes, None);
                        let skipped: Vec<String> =
                            report.skipped.iter().map(|(what, n)| format!("{what}: {n}")).collect();
                        section(ui, "Not imported", &skipped, Some(theme::text_dim()));
                    });
                },
            );
            ui.add_space(12.0);
            if ui.button("OK").clicked() {
                close = true;
            }
        });
        if close || modal.should_close() {
            self.import_report = None;
        }
    }

    fn remember_project(&mut self, dir: &std::path::Path) {
        self.settings.add_recent(dir);
        self.save_settings();
    }

    fn save_settings(&mut self) {
        // Scripted and screenshot runs leave the settings alone (`--panel` is for one run).
        if self.screenshot.is_some() || self.script.is_some() {
            return;
        }
        if let Err(e) = self.settings.save() {
            self.set_status(format!("Settings not saved: {e}"));
        }
    }

    /// Switch the theme of the chrome and every canvas.
    fn set_theme(&mut self, ctx: &egui::Context, t: view::style::Theme) {
        view::style::set_theme(t);
        self.restyle(ctx);
        self.settings.theme = t;
        self.save_settings();
    }

    /// Redraw everything in the current colours.
    fn restyle(&mut self, ctx: &egui::Context) {
        theme::apply(ctx);
        theme::apply_to_window(ctx);
        for pane in self.panes.iter_mut() {
            pane.canvas.background = view::style::background(pane.canvas.background.grid);
        }
        self.doc_mut().restyle();
        for l in self.libraries.iter_mut() {
            l.lib.doc.restyle();
        }
        for t in self.all_libs_mut().flat_map(|l| l.tabs.iter_mut()) {
            t.canvas.background = view::style::background(t.canvas.background.grid);
            t.view.restyle();
        }
        self.gerber.restyle();
        self.parts.restyle();
        self.probe_key = None;
    }

    /// Save (or Save As when `choose` or the document has no folder yet).
    fn save(&mut self, choose: bool) -> bool {
        let path = match (&self.doc().path, choose) {
            (Some(p), false) => p.clone(),
            _ => {
                let name = format!("{}.pcbproj", self.doc().project().name);
                match rfd::FileDialog::new().set_title("Save project folder").set_file_name(name).save_file() {
                    Some(p) => p,
                    None => return false,
                }
            }
        };
        match self.doc_mut().save_as(&path) {
            Ok(()) => {
                self.set_status(format!("Saved {}", path.display()));
                self.remember_project(&path);
                self.save_workspace();
                true
            }
            Err(e) => {
                self.set_status(format!("Save failed: {e}"));
                false
            }
        }
    }

    /// Export the Gerber and drill files into the project folder (saving a new project
    /// first). Design rule violations are reported in Messages but don't stop it.
    fn export_gerber(&mut self) {
        if self.doc().path.is_none() && !self.save(false) {
            return;
        }
        match self.doc_mut().export_gerber() {
            Ok(export) => {
                self.refresh_outputs();
                let files = export.manifest.files.len() + export.manifest.assembly.len();
                let dir = export.dir.parent().unwrap_or(&export.dir).display();
                self.set_status(match export.warnings.len() {
                    0 => format!("Exported {files} Gerber, drill and assembly files to {dir}"),
                    n => format!(
                        "Exported {files} Gerber, drill and assembly files to {dir}, with {n} design rule violation(s) (see Messages)"
                    ),
                });
            }
            Err(e) => self.set_status(format!("Gerber export failed: {e}")),
        }
    }

    /// Scan the export folder again, and reload the Gerber tab if it is open.
    fn refresh_outputs(&mut self) {
        let dir = self.doc().path.clone();
        self.outputs.scan(dir.as_deref());
        self.outputs_scanned = Some(dir);
        if self.tree.tiles.find_pane(&Tab::Gerber).is_some()
            && let Some(d) = self.outputs.dir.clone()
        {
            self.gerber.load(&d, None);
        }
    }

    /// Show the Gerber tab (next to the PCB), with only `file` if given.
    fn open_gerber(&mut self, file: Option<PathBuf>) {
        let Some(dir) = self.outputs.dir.clone() else { return };
        let only = file.as_deref().and_then(|f| f.file_name()).map(|n| n.to_string_lossy().into_owned());
        self.gerber.load(&dir, only.as_deref());
        let tile = match self.tree.tiles.find_pane(&Tab::Gerber) {
            Some(tile) => tile,
            None => insert_tab(&mut self.tree, Tab::Gerber, None, &[Tab::View(ViewId::Board)]),
        };
        self.tree.make_active(|id, _| id == tile);
        self.focus = Focus::Gerber;
    }

    /// Show the Simulation tab. New: right of the schematic when its tab is alone in its
    /// group, else a tab of the schematic's group. `front`: bring it to the front (and
    /// give it the keys) when it was open already.
    fn open_sim_tab(&mut self, front: bool) {
        self.sim.plot_closed = false;
        let tile = match self.tree.tiles.find_pane(&Tab::Simulation) {
            Some(_) if !front => return,
            Some(tile) => tile,
            None => {
                let sch = Tab::View(self.sheet_view());
                let group = self.tree.tiles.find_pane(&sch).and_then(|t| Some((t, self.tree.tiles.parent_of(t)?)));
                match group {
                    Some((sch_tile, g)) if tabs_count(&self.tree, g) == 1 => {
                        split_right(&mut self.tree, g, sch_tile, Tab::Simulation)
                    }
                    _ => insert_tab(&mut self.tree, Tab::Simulation, None, &[sch, Tab::View(ViewId::Board)]),
                }
            }
        };
        self.tree.make_active(|id, _| id == tile);
        if front {
            self.focus = Focus::Simulation;
        }
    }

    /// Delete the export folders and the zip, after asking.
    fn delete_outputs(&mut self) {
        let Some(dir) = self.outputs.dir.clone() else { return };
        let dirs = self.outputs.dirs();
        let names: Vec<String> = dirs.iter().map(|d| d.display().to_string()).collect();
        let answer = rfd::MessageDialog::new()
            .set_title("Delete outputs")
            .set_description(format!("Delete {} and the Gerber zip?", names.join(" and ")))
            .set_buttons(rfd::MessageButtons::OkCancelCustom("Delete".into(), "Cancel".into()))
            .show();
        if !matches!(answer, rfd::MessageDialogResult::Custom(ref b) if b == "Delete") {
            return;
        }
        if let Some(zip) = self.outputs.manifest.as_ref().and_then(|m| dir.parent().map(|p| p.join(&m.zip))) {
            let _ = std::fs::remove_file(zip);
        }
        for d in dirs {
            if let Err(e) = std::fs::remove_dir_all(&d) {
                self.set_status(format!("Could not delete {}: {e}", d.display()));
            }
        }
        if let Some(tile) = self.tree.tiles.find_pane(&Tab::Gerber) {
            self.tree.remove_recursively(tile);
        }
        self.gerber.view = None;
        self.refresh_outputs();
    }

    // ------------------------------------------------------------------------------------
    // Libraries

    /// Keep a library loaded, without a tab. Returns its key.
    fn load_library(&mut self, lib: LibraryDoc) -> u64 {
        self.libraries.add(lib)
    }

    /// Load a library; `show`: show it in the Project panel.
    fn add_library(&mut self, lib: LibraryDoc, show: bool) -> u64 {
        let key = self.load_library(lib);
        if show {
            self.show_library_node(key);
        }
        key
    }

    /// Show a library's node in the Project panel.
    fn show_library_node(&mut self, key: u64) {
        let owner = if key == PROJECT_LIB { Owner::Project } else { Owner::Lib(key) };
        self.project_tree.reveal(NodeId::Library(owner));
        self.settings.dock.show(Panel::Project);
    }

    /// The next value of the tab clock (when item tabs were used).
    fn tick(&mut self) -> u64 {
        self.tab_clock += 1;
        self.tab_clock
    }

    /// The preview tab, if one is open.
    fn preview_tab(&self) -> Option<u64> {
        self.all_libs().flat_map(|l| &l.tabs).find(|t| t.preview).map(|t| t.id)
    }

    /// Show a library item: in its tab if it has one (a normal open keeps a preview open),
    /// else in the preview tab (`preview`: the item replaces the preview's) or a new tab
    /// next to the item tab used last. The tab comes to the front; `focus`: it takes the
    /// keyboard. Returns the tab.
    fn open_item(&mut self, key: u64, open: Open, preview: bool, focus: bool) -> Option<u64> {
        self.commit_text_edit();
        let old_preview = self.preview_tab();
        let last_used = self.all_libs().flat_map(|l| &l.tabs).max_by_key(|t| t.used).map(|t| t.id);
        let l = self.lib(key)?;
        if !l.exists(open) {
            return None;
        }
        let (id, tile) = match l.tabs.iter_mut().find(|t| t.open == open) {
            Some(t) => {
                t.preview &= preview;
                let id = t.id;
                (id, self.tree.tiles.find_pane(&Tab::Item(id)))
            }
            None => {
                let id = l.add_tab(open, preview);
                let replaced = old_preview.filter(|_| preview).and_then(|old| {
                    let tile = self.tree.tiles.find_pane(&Tab::Item(old))?;
                    self.tree.tiles.insert(tile, Tile::Pane(Tab::Item(id)));
                    self.remove_item_tab(old);
                    Some(tile)
                });
                (id, replaced)
            }
        };
        let tile = match tile {
            Some(tile) => tile,
            None => self.insert_item_tab(id, last_used),
        };
        self.tree.make_active(|t, _| t == tile);
        // A tab it covers hands it the focus (browsing in the Project panel goes on).
        let covered = self.focus.item().is_some_and(|f| !self.is_tab_visible(Tab::Item(f)));
        let used = self.tick();
        if let Some(t) = self.lib(key).and_then(|l| l.tab_mut(id)) {
            t.used = used;
        }
        if focus || covered {
            self.focus = Focus::Item(id);
        }
        self.limit_item_tabs(&[id]);
        Some(id)
    }

    /// Add item tab `id` to the tree: right after tab `after`, else next to the schematic.
    fn insert_item_tab(&mut self, id: u64, after: Option<u64>) -> TileId {
        let tab = Tab::Item(id);
        let tiles = &self.tree.tiles;
        let at = after.and_then(|a| tiles.find_pane(&Tab::Item(a))).and_then(|a| Some((tiles.parent_of(a)?, a)));
        if let Some((group, after)) = at {
            let tile = self.tree.tiles.insert_pane(tab);
            if let Some(Tile::Container(Container::Tabs(tabs))) = self.tree.tiles.get_mut(group) {
                let i = tabs.children.iter().position(|&c| c == after).map_or(tabs.children.len(), |i| i + 1);
                tabs.children.insert(i, tile);
                return tile;
            }
            self.tree.tiles.remove(tile);
        }
        let near = [Tab::View(self.sheet_view()), Tab::View(ViewId::Board)];
        insert_tab(&mut self.tree, tab, None, &near)
    }

    /// The revision of an item tab's library document.
    fn tab_revision(&self, id: u64) -> Option<u64> {
        Some(self.lib_of_tab(id)?.lib.doc.revision)
    }

    /// An edit made in item tab `id` (its library's revision was `before`): a preview tab
    /// becomes a normal one.
    fn keep_if_edited(&mut self, id: u64, before: Option<u64>) {
        if self.tab_revision(id) != before
            && let Some(t) = self.all_libs_mut().find_map(|l| l.tab_mut(id))
        {
            t.preview = false;
        }
    }

    /// After an undo or redo in library `key`: bring the open tab of an item it changed
    /// to the front, with focus.
    fn front_tab_of(&mut self, key: u64, changed: &[ObjectId]) {
        let Some(l) = self.lib_ref(key) else { return };
        let Some(id) = l.tabs.iter().find(|t| changed.contains(&t.open.object_id())).map(|t| t.id) else { return };
        if let Some(tile) = self.tree.tiles.find_pane(&Tab::Item(id)) {
            self.tree.make_active(|t, _| t == tile);
            self.focus = Focus::Item(id);
        }
    }

    /// Close item tabs over the maximum (least recently used first), never `keep` nor
    /// the focused tab.
    fn limit_item_tabs(&mut self, keep: &[u64]) {
        let visible: Vec<Tab> =
            self.tree.active_tiles().into_iter().filter_map(|t| self.tree.tiles.get_pane(&t).copied()).collect();
        let tabs: Vec<item_tabs::TabInfo> = self
            .all_libs()
            .flat_map(|l| &l.tabs)
            .map(|t| item_tabs::TabInfo {
                id: t.id,
                used: t.used,
                preview: t.preview,
                visible: visible.contains(&Tab::Item(t.id)),
            })
            .collect();
        let mut keep = keep.to_vec();
        keep.extend(self.focus.item());
        for id in item_tabs::to_close(&tabs, self.settings.max_library_tabs(), &keep) {
            self.close_item_tab(id);
        }
    }

    /// Close an item tab. Nothing is lost: its edits are the library's.
    fn close_item_tab(&mut self, id: u64) {
        if let Some(ed) = self.editor(id) {
            if let Err(e) = ed.lib.doc.commit_text_edit() {
                *ed.message = Some(e);
            }
            ed.lib.doc.close_group();
        }
        if let Some(tile) = self.tree.tiles.find_pane(&Tab::Item(id)) {
            self.tree.remove_recursively(tile);
        }
        self.remove_item_tab(id);
    }

    /// Drop an item tab's state (its tile is gone or replaced).
    fn remove_item_tab(&mut self, id: u64) {
        for l in self.all_libs_mut() {
            l.remove_tab(id);
        }
        if self.focus.item() == Some(id) {
            self.focus = Focus::Project;
        }
    }

    /// Close the item tabs of a library.
    fn close_lib_tabs(&mut self, key: u64) {
        let ids: Vec<u64> = self.lib_ref(key).map_or(Vec::new(), |l| l.tabs.iter().map(|t| t.id).collect());
        for id in ids {
            self.close_item_tab(id);
        }
    }

    /// Per frame: tabs of deleted items close, and the focused tab is the most recently
    /// used.
    fn maintain_item_tabs(&mut self) {
        self.drop_orphan_tabs();
        let Some(id) = self.focus.item() else { return };
        let clock = self.tab_clock;
        if self.all_libs().flat_map(|l| &l.tabs).any(|t| t.id == id && t.used != clock) {
            let used = self.tick();
            if let Some(t) = self.all_libs_mut().find_map(|l| l.tab_mut(id)) {
                t.used = used;
            }
        }
    }

    /// Tabs whose item is gone close; item tab states without a tile (a restored layout
    /// left them out) and tiles without a state are dropped.
    fn drop_orphan_tabs(&mut self) {
        let gone: Vec<u64> =
            self.all_libs().flat_map(|l| l.tabs.iter().filter(move |t| !l.exists(t.open)).map(|t| t.id)).collect();
        for id in gone {
            self.close_item_tab(id);
        }
        let tree = &self.tree;
        let orphans: Vec<u64> = self
            .all_libs()
            .flat_map(|l| &l.tabs)
            .map(|t| t.id)
            .filter(|&id| tree.tiles.find_pane(&Tab::Item(id)).is_none())
            .collect();
        for id in orphans {
            self.remove_item_tab(id);
        }
        // And tiles without a state (the Project Library of a replaced project).
        let stale: Vec<TileId> = (self.tree.tiles.iter())
            .filter_map(|(&tile, t)| match t {
                Tile::Pane(Tab::Item(id)) if self.lib_of_tab(*id).is_none() => Some(tile),
                _ => None,
            })
            .collect();
        for tile in stale {
            self.tree.remove_recursively(tile);
        }
    }

    /// Unload a library and close its tabs (unsaved changes are lost).
    fn unload_library(&mut self, key: u64) {
        self.close_lib_tabs(key);
        self.libraries.remove(key);
    }

    /// Bring the loaded libraries in line with the project's library files (opening a
    /// project, adding or removing a library, undo and redo of those).
    fn sync_libraries(&mut self) {
        for l in self.libraries.sync(&self.project.lib.doc.project().libraries) {
            for t in &l.tabs {
                if let Some(tile) = self.tree.tiles.find_pane(&Tab::Item(t.id)) {
                    self.tree.remove_recursively(tile);
                }
            }
        }
        self.drop_orphan_tabs();
    }

    /// Whether the project lists the library's file.
    fn is_listed(&self, lib: &LibraryDoc) -> bool {
        lib.path.as_deref().is_some_and(|p| self.doc().project().libraries.contains(&project_libs::absolute(p)))
    }

    /// Add a library (saved to a file) to the project; `show`: show it in the Project panel.
    fn add_to_project(&mut self, lib: LibraryDoc, show: bool) -> Option<u64> {
        let loaded: Vec<&LibraryDoc> = self.libraries.iter().map(|l| &l.lib).collect();
        match project_libs::add_to_project(&mut self.project.lib.doc, &loaded, &lib) {
            Ok(()) => {
                let name = lib.name().to_string();
                let key = self.add_library(lib, show);
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
    fn add_library_file(&mut self, path: &std::path::Path, show: bool) {
        if let Some(l) = self.libraries.iter().find(|l| l.lib.path.as_deref().is_some_and(|p| same_file(p, path))) {
            let key = l.key;
            if !self.is_listed(&self.libraries.by_key(key).unwrap().lib) {
                // Dropped from the project while it had unsaved changes: add it again.
                let command = Command::AddLibrary { path: project_libs::absolute(path) };
                if let Err(e) = self.doc_mut().execute(command) {
                    return self.set_status(e.to_string());
                }
            }
            if show {
                self.show_library_node(key);
            }
            return;
        }
        match LibraryDoc::open(path) {
            Ok(lib) => {
                self.add_to_project(lib, show);
            }
            Err(e) => self.set_status(format!("Could not open library: {e}")),
        }
    }

    /// A file dialog for library files, starting in the project folder.
    fn library_dialog(&self, title: &str) -> rfd::FileDialog {
        let dialog = rfd::FileDialog::new().set_title(title).add_filter("PCB library", &["pcblib"]);
        match &self.doc().path {
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
                self.add_to_project(lib, true);
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
        let Some(l) = self.libraries.by_key(key) else { return false };
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
        if let Some(command) = project_libs::after_save(&self.doc().project().libraries, old.as_deref(), &path)
            && let Err(e) = self.doc_mut().execute(command)
        {
            self.set_status(e.to_string());
        }
        self.libraries.resync();
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
        self.add_to_project(copy, true).is_some()
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

    /// Close a library outside the project (Project panel), asking about its changes:
    /// its tabs close and it is unloaded.
    fn close_library(&mut self, key: u64) {
        if key == PROJECT_LIB || !self.confirm_discard_library(key) {
            return;
        }
        self.close_lib_tabs(key);
        let Some(l) = self.libraries.by_key(key) else { return };
        let (builtin, listed, discarded) = (l.is_builtin(), self.is_listed(&l.lib), l.lib.is_modified());
        if !builtin && (discarded || !listed) {
            self.libraries.remove(key);
            self.libraries.resync();
        }
    }

    /// Remove a library from the project (asking about unsaved changes first).
    fn remove_library(&mut self, key: u64) {
        if !self.confirm_discard_library(key) {
            return;
        }
        let Some(l) = self.libraries.by_key(key) else { return };
        let path = l.lib.path.as_deref().map(project_libs::absolute);
        let name = l.lib.name().to_string();
        if let Some(path) = path.filter(|p| self.doc().project().libraries.contains(p))
            && let Err(e) = self.doc_mut().execute(Command::RemoveLibrary { path })
        {
            return self.set_status(e.to_string());
        }
        self.unload_library(key);
        self.set_status(format!("Removed library {name} from the project"));
    }

    /// Remove a library file that could not be loaded from the project.
    fn remove_missing_library(&mut self, path: PathBuf) {
        if let Err(e) = self.doc_mut().execute(Command::RemoveLibrary { path }) {
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
        let others: Vec<PathBuf> = self.doc().project().libraries.iter().filter(|p| **p != from).cloned().collect();
        let loaded: Vec<&LibraryDoc> = self.libraries.iter().map(|l| &l.lib).collect();
        if let Err(e) = project_libs::check_new(&others, &loaded, &lib) {
            return self.set_status(e);
        }
        match self.doc_mut().execute(Command::RelinkLibrary { from, to }) {
            Ok(_) => {
                self.load_library(lib);
                self.sync_libraries();
            }
            Err(e) => self.set_status(e.to_string()),
        }
    }

    /// Open a part, symbol or footprint by name, searching the libraries from the last
    /// loaded one (development aid).
    fn show_by_name(&mut self, name: &str) {
        // "symbol:NAME" / "footprint:NAME" / "part:NAME", or the first match of NAME.
        let (kind, name) = name.split_once(':').unwrap_or(("", name));
        let find = |lib: &Library| {
            let part = lib.parts.iter().find(|p| p.name == name).filter(|_| matches!(kind, "" | "part"));
            let symbol = lib.symbols.iter().find(|s| s.name == name).filter(|_| matches!(kind, "" | "symbol"));
            let footprint = lib.footprints.iter().find(|f| f.name == name).filter(|_| matches!(kind, "" | "footprint"));
            if let Some(p) = part {
                Some(Open::Part(p.id))
            } else if let Some(s) = symbol {
                Some(Open::Item(Item::Symbol(s.id)))
            } else {
                footprint.map(|f| Open::Item(Item::Footprint(f.id)))
            }
        };
        let Some((key, open)) = self.libraries.iter().rev().find_map(|l| find(l.lib.library()).map(|o| (l.key, o)))
        else {
            return;
        };
        self.open_item(key, open, false, true);
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
        let l = self.lib(key).ok_or("library closed")?;
        let (added, replaced) =
            l.lib.upsert(imported.symbols.clone(), imported.footprints.clone(), imported.parts.clone())?;
        Ok((key, imported, added, replaced))
    }

    /// Import from the KiCad window.
    fn run_kicad_import(&mut self, target: Option<u64>, req: kicad::Request) {
        match self.kicad_import_into(target, "KiCad import", &req) {
            Ok((key, imported, added, replaced)) => {
                let name = self.lib(key).map(|l| l.lib.name().to_string()).unwrap_or_default();
                self.kicad.target = Some(key);
                self.kicad.finish();
                self.import_report = Some(ImportReport::parts(&imported, &name, added, replaced));
                self.set_status(format!("Imported from KiCad into {name}"));
                let first = (imported.parts.first().map(|p| Open::Part(p.id)))
                    .or(imported.symbols.first().map(|s| Open::Item(Item::Symbol(s.id))))
                    .or(imported.footprints.first().map(|f| Open::Item(Item::Footprint(f.id))));
                match first {
                    Some(open) => _ = self.open_item(key, open, false, true),
                    None => self.show_library_node(key),
                }
            }
            Err(e) => self.import_report = Some(ImportReport::failed(e)),
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
            let command = Command::update_parts(self.libraries[lib].lib.library(), &parts);
            match command.and_then(|c| self.doc_mut().execute(c)) {
                Ok(_) => updated += parts.len(),
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
        match self.doc().editor().history().undo_label().map(str::to_string) {
            Some(label) => {
                self.doc_mut().undo();
                self.set_status(format!("Undo: {label}"));
            }
            None => self.set_status("Nothing to undo"),
        }
    }

    fn redo(&mut self) {
        if self.busy() {
            return;
        }
        match self.doc().editor().history().redo_label().map(str::to_string) {
            Some(label) => {
                self.doc_mut().redo();
                self.set_status(format!("Redo: {label}"));
            }
            None => self.set_status("Nothing to redo"),
        }
    }

    /// Start placing a part (it follows the cursor until clicked): in the schematic, or
    /// on the board for a board part. Imports it from library `lib_key` if the project
    /// doesn't have it yet.
    fn place_part(&mut self, lib_key: Option<u64>, part: Id) {
        if !self.doc().project().library.parts.contains(part) {
            let Some(l) = lib_key.and_then(|k| self.libraries.by_key(k)) else { return };
            let issues = l.lib.library().validate();
            if !issues.is_empty() {
                return self.set_status(format!("Library has problems: {}", issues[0]));
            }
            let command = Command::import_parts(l.lib.library(), &[part]);
            if let Err(e) = command.and_then(|c| self.doc_mut().execute(c)) {
                return self.set_status(format!("Import failed: {e}"));
            }
        }
        if self.doc().project().library.parts.get(part).is_some_and(Part::is_board_part) {
            self.show_view(ViewId::Board);
            self.set_pcb_tool(PcbTool::Place);
            self.pcb.place(part);
            return;
        }
        self.focus_schematic();
        self.set_sch_tool(SchTool::Place);
        self.sch.place(part);
    }

    /// After tab changes (clicks, drag and drop, closing): keyboard focus goes to a view
    /// or item tab that is visible.
    fn follow_visible_tabs(&mut self) {
        let visible: Vec<Tab> =
            self.tree.active_tiles().into_iter().filter_map(|t| self.tree.tiles.get_pane(&t).copied()).collect();
        match self.focus {
            Focus::Item(id) if !visible.contains(&Tab::Item(id)) => self.focus = Focus::Project,
            Focus::Gerber if !visible.contains(&Tab::Gerber) => self.focus = Focus::Project,
            Focus::Simulation if !visible.contains(&Tab::Simulation) => self.focus = Focus::Project,
            _ => {}
        }
        if self.focus.item().is_none() && !visible.contains(&Tab::View(self.active)) {
            match visible.iter().find_map(|t| if let Tab::View(k) = t { Some(*k) } else { None }) {
                Some(kind) => self.active = kind,
                None if self.focus == Focus::Project => {
                    if let Some(id) = visible.iter().find_map(|t| if let Tab::Item(k) = t { Some(*k) } else { None }) {
                        self.focus = Focus::Item(id);
                    }
                }
                None => {}
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

    /// Is the tab in front of its group?
    fn is_tab_visible(&self, tab: Tab) -> bool {
        self.tree.active_tiles().into_iter().any(|t| self.tree.tiles.get_pane(&t) == Some(&tab))
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
        let root = self.doc().root();
        let sheets = self.doc().sheet_views().map(|v| Tab::View(v.id)).filter(|t| *t != Tab::View(root));
        let items: Vec<Tab> = self.all_libs().flat_map(|l| &l.tabs).map(|t| Tab::Item(t.id)).collect();
        let others: Vec<Tab> = sheets.chain(items).filter(|t| self.tree.tiles.find_pane(t).is_some()).collect();
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
    /// closed: in the tab group it was in, else next to the schematic, the item tab used
    /// last or the board.
    fn show_view(&mut self, view: ViewId) {
        let sheet = self.sheet_view();
        self.active = view;
        self.focus = Focus::Project;
        if let Some(s) = view.sheet() {
            self.last_sheet = Some(s);
        }
        let tile = match self.tree.tiles.find_pane(&Tab::View(view)) {
            Some(tile) => tile,
            None => {
                let root = self.doc().root();
                let item = self.all_libs().flat_map(|l| &l.tabs).max_by_key(|t| t.used).map(|t| Tab::Item(t.id));
                let mut near = vec![Tab::View(sheet), Tab::View(root)];
                near.extend(item);
                if view != ViewId::Board {
                    near.push(Tab::View(ViewId::Board));
                }
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
        self.commit_text_edit();
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
        let key = Some((from, selection.clone(), self.doc().revision, routing.clone()));
        if key == self.probe_key {
            return;
        }
        self.probe_key = key;
        let doc = self.doc_mut();
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
        match (self.focus.item(), self.active.kind()) {
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
            Action::SchText => SchTool::Text,
            Action::SchPower => SchTool::Power,
            Action::SchJunction => SchTool::Junction,
            Action::SchNoConnect => SchTool::NoConnect,
            Action::SchPowerFlag => SchTool::PowerFlag,
            Action::SchProbe => SchTool::Probe,
            Action::ToolSelect => SchTool::Select,
            Action::Posture => {
                self.sch.toggle_posture();
                return true;
            }
            Action::NextCorners => {
                if let Some(name) = self.sch.next_corners() {
                    self.set_status(format!("Corners: {name}"));
                }
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
        self.commit_text_edit();
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
            Action::PcbKeepout => PcbTool::Keepout,
            Action::PcbCutout => PcbTool::Cutout,
            Action::PcbText => PcbTool::Text,
            Action::MeasureClearance => {
                if self.pcb.tool() != PcbTool::Measure {
                    self.set_pcb_tool(PcbTool::Measure);
                }
                let mode = self.pcb.toggle_clearance();
                self.set_status(mode);
                return true;
            }
            Action::ToolMeasure => PcbTool::Measure,
            Action::ToolSelect => PcbTool::Select,
            Action::Posture => {
                self.pcb.toggle_posture();
                return true;
            }
            Action::NextCorners => {
                if let Some(name) = self.pcb.next_corners() {
                    self.set_status(format!("Corners: {name}"));
                }
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
            Action::Rotate if matches!(self.pcb.tool(), PcbTool::Place | PcbTool::Text) => {
                self.pcb.rotate();
                return true;
            }
            Action::Flip if matches!(self.pcb.tool(), PcbTool::Place | PcbTool::Text) => {
                self.pcb.flip();
                return true;
            }
            _ => return false,
        };
        self.set_pcb_tool(tool);
        true
    }

    // ------------------------------------------------------------------------------------
    // Clipboard

    /// Cut, copy, paste and duplicate in the focused library item or the active view.
    fn clipboard_action(&mut self, action: Action) {
        if action == Action::Copy
            && let Some(text) = self.measure_reading()
        {
            self.copy_out = Some(text);
            return self.set_status("Copied the measurement");
        }
        if self.gerber_focused() || self.focus == Focus::Simulation {
            return;
        }
        let result = if let Some(id) = self.focus.item() {
            let before = self.tab_revision(id);
            let r = self.lib_clipboard(id, action);
            self.keep_if_edited(id, before);
            r
        } else if self.is_open(self.active) {
            self.view_clipboard(action)
        } else {
            Ok(None)
        };
        if let Ok(Some(m)) | Err(m) = result {
            self.set_status(m);
        }
    }

    /// The measurement of the Measure tool in the focused editor, if it shows one.
    fn measure_reading(&mut self) -> Option<String> {
        let text = |r: view::measure::Reading| r.text(units::fmt);
        if self.gerber_focused() {
            return self.gerber.measure.reading().filter(|_| self.gerber.measuring).map(text);
        }
        if self.focus.item().is_some() {
            let t = self.focused_tab()?;
            return t.placer.measure.reading().filter(|_| t.placer.tool == Tool::Measure).map(text);
        }
        if self.active != ViewId::Board || self.pcb.tool() != PcbTool::Measure {
            return None;
        }
        if self.pcb.settings.clearance {
            return self.pcb.gap.as_ref().map(|g| view::measure::gap_text(g, units::fmt));
        }
        self.pcb.measure.reading().map(text)
    }

    /// The clip on the system clipboard.
    fn clip_from_clipboard(&mut self) -> Result<Clip, String> {
        let text = self.paste_text.take().or_else(clipboard::read).unwrap_or_default();
        Clip::from_text(&text).unwrap_or_else(|| Err("The clipboard holds no objects to paste".into()))
    }

    fn lib_clipboard(&mut self, tab: u64, action: Action) -> Result<Option<String>, String> {
        let pasted = if action == Action::Paste { Some(self.clip_from_clipboard()?) } else { None };
        let mut l = self.editor(tab).ok_or("The tab is gone")?;
        match action {
            Action::Paste => l.paste(&pasted.expect("read above"))?,
            Action::Duplicate => {
                let clip = l.copy(false)?;
                l.paste(&clip)?;
            }
            _ => {
                let clip = l.copy(action == Action::Cut)?;
                let n = clip.content.len();
                self.copy_out = Some(clip.to_text());
                return Ok(Some(copied(action, n)));
            }
        }
        Ok(None)
    }

    fn view_clipboard(&mut self, action: Action) -> Result<Option<String>, String> {
        if action == Action::Paste {
            let clip = self.clip_from_clipboard()?;
            self.paste_into_view(clip)?;
            return Ok(None);
        }
        let view = self.active;
        let it = &self.panes.get(view).it;
        let (grid, snap) = (it.grid, it.snap);
        let selection = it.selection.clone();
        let mut snapper = Interaction::<ObjectId>::new(grid);
        snapper.snap = snap;
        let (clip, note) = view::clipboard::copy(self.doc().project(), &selection, |p| snapper.snap(p))?;
        if action == Action::Duplicate {
            self.paste_into_view(clip)?;
            return Ok(note);
        }
        if action == Action::Cut {
            let doc = &mut self.project.lib.doc;
            self.panes.get(view).it.key(&mut doc.target(view), KeyAction::Delete);
        }
        let n = clip.content.len();
        self.copy_out = Some(clip.to_text());
        Ok(Some(note.unwrap_or_else(|| copied(action, n))))
    }

    /// Paste into the view under the pointer (else the active one); the objects follow
    /// the pointer until a click.
    fn paste_into_view(&mut self, clip: Clip) -> Result<(), String> {
        if self.busy() {
            return Ok(());
        }
        let view = self.hovered.filter(|v| self.is_open(*v)).unwrap_or(self.active);
        if view.kind() == ViewKind::Schematic {
            self.sch.set_tool(SchTool::Select);
        } else {
            self.pcb.set_tool(PcbTool::Select);
        }
        self.carry_clip = Some((view, clip.clone()));
        let pointer = self.panes.get(view).canvas.cursor_world.map(|[x, y]| Point::mm(x, y));
        paste_and_carry(&mut self.project.lib.doc, self.panes.get(view), view, clip, pointer)?;
        self.active = view;
        self.focus = Focus::Project;
        Ok(())
    }

    /// Keys of the Gerber tab: its tools and fitting. Returns true if handled.
    fn gerber_action(&mut self, action: Action) -> bool {
        match action {
            Action::ToolMeasure => self.gerber.set_measuring(true),
            Action::ToolSelect => self.gerber.set_measuring(false),
            Action::Cancel => self.gerber.escape(),
            Action::FitView => self.gerber.canvas.request_fit(),
            _ => return false,
        }
        true
    }

    /// Round the corners between the selected tracks with the route tool's radius.
    /// Delete a corner of a zone, keepout or polygon drawing (from its handle).
    fn delete_corner(&mut self, id: ObjectId, grip: view::handles::Grip) {
        let Some(o) = self.doc().project().get(id) else { return };
        let result = view::handles::without_corner(&o, grip)
            .and_then(|o| self.doc_mut().execute(Command::Update { objects: vec![o] }).map_err(|e| e.to_string()));
        match result {
            Ok(_) => self.panes.get(ViewId::Board).grip_hover = None,
            Err(e) => self.set_status(e),
        }
    }

    /// The selected rectangle drawing (the board outline) as a polygon, whose corners can
    /// then be added and deleted.
    fn convert_to_polygon(&mut self) {
        let sel = self.panes.selection(ViewId::Board);
        let converted: Vec<Object> = sel
            .iter()
            .filter_map(|&id| self.doc().project().get(id))
            .filter_map(|o| view::handles::rect_to_polygon(&o))
            .collect();
        if converted.is_empty() {
            return self.set_status("Select a rectangle on the board to convert");
        }
        if let Err(e) = self.doc_mut().execute(Command::Update { objects: converted }) {
            self.set_status(e.to_string());
        }
    }

    fn round_corners(&mut self) {
        let tracks: Vec<Id> = self
            .panes
            .selection(ViewId::Board)
            .iter()
            .filter_map(|id| if let ObjectId::Track(t) = id { Some(*t) } else { None })
            .collect();
        if tracks.is_empty() {
            return self.set_status("Select the tracks whose corners to round (on the board)".to_string());
        }
        let total = editor_core::fillet::corners(self.doc().project(), &tracks).len();
        let radius = self.pcb.settings.corner_radius;
        let arcs = |p: &Project| p.board.tracks.iter().filter(|t| t.mid.is_some()).count();
        let before = arcs(self.doc().project());
        match self.project.lib.doc.execute(Command::RoundCorners { tracks, radius }) {
            Ok(_) => {
                let n = arcs(self.doc().project()) - before;
                self.set_status(format!("Rounded {n} of {total} corners ({} mm radius)", radius as f64 / 1e6));
            }
            Err(e) => self.set_status(e.to_string()),
        }
    }

    /// Keys, typing and the clipboard go to the text being edited in place.
    fn text_edit_input(&mut self, ctx: &egui::Context) {
        use view::text_edit::{EditKey, Mods};
        enum Input {
            Key(EditKey, Mods),
            Text(String),
            Copy,
            Cut,
        }
        let mut out: Vec<Input> = Vec::new();
        ctx.input_mut(|i| {
            i.events.retain(|e| {
                match e {
                    egui::Event::Key { key, pressed, modifiers, .. } => {
                        let m = Mods { shift: modifiers.shift, word: modifiers.alt, line: modifiers.command };
                        let k = match key {
                            egui::Key::ArrowLeft => Some(EditKey::Left),
                            egui::Key::ArrowRight => Some(EditKey::Right),
                            egui::Key::ArrowUp => Some(EditKey::Up),
                            egui::Key::ArrowDown => Some(EditKey::Down),
                            egui::Key::Home => Some(EditKey::Home),
                            egui::Key::End => Some(EditKey::End),
                            egui::Key::Backspace => Some(EditKey::Backspace),
                            egui::Key::Delete => Some(EditKey::Delete),
                            egui::Key::Enter => Some(EditKey::Enter),
                            egui::Key::Escape => Some(EditKey::Escape),
                            egui::Key::A if modifiers.command => Some(EditKey::SelectAll),
                            _ => None,
                        };
                        if let (true, Some(k)) = (*pressed, k) {
                            out.push(Input::Key(k, m));
                        }
                        // Other keys with ⌘ (save, undo, …) stay shortcuts; plain ones are typing.
                        let edit_key =
                            matches!(key, egui::Key::Z | egui::Key::Y | egui::Key::C | egui::Key::X | egui::Key::V);
                        !(k.is_some() || !modifiers.command || edit_key)
                    }
                    egui::Event::Text(t) | egui::Event::Paste(t) => {
                        out.push(Input::Text(t.clone()));
                        false
                    }
                    egui::Event::Copy => {
                        out.push(Input::Copy);
                        false
                    }
                    egui::Event::Cut => {
                        out.push(Input::Cut);
                        false
                    }
                    _ => true,
                }
            });
        });
        for input in out {
            let Some(doc) = self.editing_doc() else { break };
            let r = match input {
                Input::Key(k, m) => doc.text_edit_key(k, m),
                Input::Text(t) => {
                    doc.with_text_edit(|e| e.insert(&t));
                    Ok(())
                }
                Input::Copy => {
                    self.copy_out = doc.text_edit().map(|(_, e)| e.selected_text()).filter(|s| !s.is_empty());
                    Ok(())
                }
                Input::Cut => {
                    self.copy_out = doc.with_text_edit(|e| e.cut()).flatten();
                    Ok(())
                }
            };
            if let Err(e) = r {
                self.set_status(e);
            }
        }
    }

    /// Finish the text being edited (before other actions and tool changes).
    fn commit_text_edit(&mut self) {
        if let Some(doc) = self.editing_doc()
            && let Err(e) = doc.commit_text_edit()
        {
            self.set_status(e);
        }
    }

    fn run_action(&mut self, action: Action) {
        self.commit_text_edit();
        if let Some(panel) = Panel::of_action(action) {
            return self.settings.dock.toggle(panel);
        }
        match action {
            Action::RunSimulation => return self.run_simulation(None),
            Action::StopSimulation => return self.sim.session.stop(),
            Action::ShowSimNetlist => return self.sim.show_window(false),
            Action::ShowSimPlot => return self.open_sim_tab(true),
            Action::ToggleSimValues => return self.sim.hide_values = !self.sim.hide_values,
            _ => {}
        }
        if matches!(action, Action::SchFindPart | Action::PcbFindPart) {
            self.settings.dock.show(Panel::Parts);
            return self.parts.focus_search();
        }
        if matches!(action, Action::Cut | Action::Copy | Action::Paste | Action::Duplicate) {
            return self.clipboard_action(action);
        }
        if let Some(id) = self.focus.item() {
            let before = self.tab_revision(id);
            let handled = self.lib_action(id, action);
            if !matches!(action, Action::Undo | Action::Redo) {
                self.keep_if_edited(id, before);
            }
            if handled {
                return;
            }
        }
        let gerber = self.gerber_focused();
        if gerber && self.gerber_action(action) {
            return;
        }
        let plot = self.focus == Focus::Simulation && self.tree.tiles.find_pane(&Tab::Simulation).is_some();
        if plot {
            let setup = &self.project.lib.doc.project().schematic.simulation;
            if let Some(a) = self.sim.current(setup).map(|a| a.id) {
                let state = self.sim.plots.entry(a).or_default();
                match action {
                    Action::FitView => return state.fit(),
                    Action::Cancel => return state.clear_cursors(),
                    _ => {}
                }
            }
        }
        // A closed view takes no keys (only the project-level actions below), nor does
        // one without focus while the Gerber tab has it.
        let view_keys = !gerber && !plot && (self.focus.item().is_some() || self.is_open(self.active));
        if view_keys && self.context() == Context::Schematic && self.sch_action(action) {
            return;
        }
        if view_keys && self.context() == Context::Board && self.pcb_action(action) {
            return;
        }
        if view_keys
            && action == Action::Delete
            && self.active == ViewId::Board
            && let Some((id, grip)) = self.panes.get(ViewId::Board).grip_hover
            && matches!(grip, view::handles::Grip::Corner { .. })
        {
            return self.delete_corner(id, grip);
        }
        let key_action = match action {
            Action::New => return self.new_document(),
            Action::Settings => {
                self.settings_win.show(None, &mut self.settings);
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
            Action::ExportGerber => return self.export_gerber(),
            Action::RoundCorners => return self.round_corners(),
            Action::ConvertToPolygon => return self.convert_to_polygon(),
            Action::MoveToSheet => return self.move_to_sheet(),
            Action::ToggleErcMarkers => {
                let on = !self.doc().erc_markers();
                self.doc_mut().set_erc_markers(on);
                return self.set_status(if on { "ERC markers shown" } else { "ERC markers hidden" });
            }
            Action::CreateArray => {
                return self.set_status("Select one pad or pin in a footprint or symbol to make an array of");
            }
            Action::Cut | Action::Copy | Action::Paste | Action::Duplicate => return,
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
            | Action::ToolArc
            | Action::ToolPolygon
            | Action::ToolText
            | Action::ToolMeasure
            | Action::SchWire
            | Action::SchBus
            | Action::SchBusEntry
            | Action::SchLabel
            | Action::SchText
            | Action::SchPower
            | Action::SchJunction
            | Action::SchNoConnect
            | Action::SchPowerFlag
            | Action::SchProbe
            | Action::Posture
            | Action::NextCorners
            | Action::PcbRoute
            | Action::PcbVia
            | Action::PcbOutline
            | Action::PcbZone
            | Action::PcbKeepout
            | Action::PcbCutout
            | Action::MeasureClearance
            | Action::PcbText
            | Action::PcbNextLayer
            | Action::PanelProject
            | Action::PanelMessages
            | Action::PanelProperties
            | Action::PanelToolSettings
            | Action::PanelBoardSetup
            | Action::PanelSchematicSetup
            | Action::PanelOverview
            | Action::PanelNets
            | Action::PanelParts
            | Action::SchFindPart
            | Action::PcbFindPart
            | Action::PanelActivity
            | Action::PanelSimulation
            | Action::RunSimulation
            | Action::StopSimulation
            | Action::ShowSimNetlist
            | Action::ShowSimPlot
            | Action::ToggleSimValues => return,
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
        let s = &self.doc().project().schematic;
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

    /// Handle an action for the focused item tab `tab`. Returns false for project-level
    /// actions.
    fn lib_action(&mut self, tab: u64, action: Action) -> bool {
        let Some(key) = self.lib_of_tab(tab).map(|l| l.key) else { return false };
        match action {
            Action::New | Action::Open | Action::Settings | Action::ExportGerber => return false,
            Action::Save | Action::SaveAs => {
                self.save_library(key, action == Action::SaveAs);
                return true;
            }
            _ => {}
        }
        if let Some(ed) = self.editor(tab)
            && matches!(action, Action::Undo | Action::Redo)
        {
            let mut ed = ed;
            let done = if action == Action::Undo { ed.undo() } else { ed.redo() };
            if let Some((msg, changed)) = done {
                self.set_status(msg);
                self.front_tab_of(key, &changed);
            }
            return true;
        }
        let Some(mut l) = self.editor(tab) else { return false };
        let msg = match action {
            Action::Undo | Action::Redo => None,
            Action::FitView => {
                l.tab.canvas.request_fit();
                None
            }
            Action::SelectAll => key_msg(&mut l, KeyAction::SelectAll),
            Action::Rotate => key_msg(&mut l, KeyAction::Rotate),
            Action::Mirror | Action::Flip => key_msg(&mut l, KeyAction::Mirror),
            Action::Delete => match l.delete_hovered_corner() {
                Some(r) => r.err(),
                None => key_msg(&mut l, KeyAction::Delete),
            },
            Action::ConvertToPolygon => l.convert_to_polygon().err(),
            Action::CreateArray => {
                if l.start_array() {
                    let group = self.settings.dock.group_of(Panel::ToolSettings);
                    if !self.settings.dock.is_open(Panel::ToolSettings) {
                        self.array_dock = Some((tab, self.settings.dock.open(group)));
                    }
                    self.settings.dock.show(Panel::ToolSettings);
                    None
                } else {
                    l.message.take()
                }
            }
            Action::Cancel => key_msg(&mut l, KeyAction::Escape),
            Action::ToolSelect => tool_msg(&mut l, Tool::Select),
            Action::ToolPin => tool_msg(&mut l, Tool::Pin),
            Action::ToolLine => tool_msg(&mut l, Tool::Line),
            Action::ToolRect => tool_msg(&mut l, Tool::Rect),
            Action::ToolCircle => tool_msg(&mut l, Tool::Circle),
            Action::ToolArc => tool_msg(&mut l, Tool::Arc),
            Action::ToolPolygon => tool_msg(&mut l, Tool::Polygon),
            Action::Posture => {
                l.tab.placer.flip_arc();
                None
            }
            Action::ToolText => tool_msg(&mut l, Tool::Text),
            Action::ToolMeasure => tool_msg(&mut l, Tool::Measure),
            Action::Cut | Action::Copy | Action::Paste | Action::Duplicate => None,
            Action::New | Action::Open | Action::Save | Action::SaveAs | Action::Settings | Action::ExportGerber => {
                None
            }
            Action::SchWire
            | Action::SchBus
            | Action::SchBusEntry
            | Action::SchLabel
            | Action::SchText
            | Action::SchPower
            | Action::SchJunction
            | Action::SchNoConnect
            | Action::SchPowerFlag
            | Action::SchProbe
            | Action::ToggleErcMarkers
            | Action::NextCorners
            | Action::RoundCorners
            | Action::MoveToSheet => None,
            Action::PcbRoute
            | Action::PcbVia
            | Action::PcbOutline
            | Action::PcbZone
            | Action::PcbKeepout
            | Action::PcbCutout
            | Action::MeasureClearance
            | Action::PcbText
            | Action::PcbNextLayer => None,
            Action::PanelProject
            | Action::PanelMessages
            | Action::PanelProperties
            | Action::PanelToolSettings
            | Action::PanelBoardSetup
            | Action::PanelSchematicSetup
            | Action::PanelOverview
            | Action::PanelNets
            | Action::PanelParts
            | Action::SchFindPart
            | Action::PcbFindPart
            | Action::PanelActivity
            | Action::PanelSimulation
            | Action::RunSimulation
            | Action::StopSimulation
            | Action::ShowSimNetlist
            | Action::ShowSimPlot
            | Action::ToggleSimValues => None,
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
                    let edited = self.doc().is_modified();
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
        let h = match self.focused_lib_ref() {
            Some(l) => l.lib.doc.editor().history(),
            None => self.doc().editor().history(),
        };
        menu::menus(&MenuState {
            native,
            keymap: &self.keymap,
            library_focus: self.focused_lib_key().is_some_and(|k| k != PROJECT_LIB),
            recent: &self.settings.recent,
            undo: h.undo_label().map(|l| format!("Undo {l}")),
            redo: h.redo_label().map(|l| format!("Redo {l}")),
            side_by_side: self.is_side_by_side(),
            theme: view::style::theme(),
            agent_running: self.agent.is_running(),
            agent_error: self.agent.error.clone(),
            agent_url: self.agent.url(),
            agent_port: self.agent.port,
            headless_command: self.headless_command(),
            move_to_sheet: !self.movable_to_sheet().is_empty(),
            erc_markers: self.doc().erc_markers(),
            sim_values: !self.sim.hide_values,
            sim_running: self.sim.session.running().is_some(),
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
            Cmd::ImportKicadProject => self.import_kicad_project(),
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
                let n = self.doc_mut().run_drc().violations.len();
                self.set_status(match n {
                    0 => "DRC: no violations".to_string(),
                    n => format!("DRC: {n} violation(s), listed in the board properties"),
                });
            }
            Cmd::ToggleAgent => {
                if self.agent.is_running() {
                    self.agent.stop();
                } else {
                    self.agent.port = self.mcp_port.unwrap_or(self.settings.mcp.port);
                    self.agent.start(ctx);
                }
            }
            Cmd::Copy(text) => ctx.copy_text(text),
            Cmd::ResetPanels => {
                let d = &self.settings.dock;
                let sizes = (d.left_width, d.right_width, d.bottom_height);
                self.settings.dock = Dock::default();
                (self.settings.dock.left_width, self.settings.dock.right_width, self.settings.dock.bottom_height) =
                    sizes;
            }
            Cmd::ShowShortcuts => self.settings_win.show(Some(SettingsPage::Keymap), &mut self.settings),
        }
    }

    fn headless_command(&self) -> String {
        let exe = std::env::current_exe().map_or("pcb-designer".into(), |p| p.display().to_string());
        let project = self.doc().path.as_ref().map_or(String::new(), |p| format!(" \"{}\"", p.display()));
        format!("claude mcp add pcb-designer -- \"{exe}\" --mcp-stdio{project}")
    }

    /// The Settings window (saved in the settings file).
    fn settings_window(&mut self, ctx: &egui::Context) {
        if !self.settings_win.open {
            return;
        }
        let status = AgentStatus {
            url: self.agent.url(),
            error: self.agent.error.clone(),
            headless_command: self.headless_command(),
        };
        let c = self.settings_win.ui(ctx, &mut self.settings, &self.keymap, &status);
        if let Some(t) = c.theme {
            self.set_theme(ctx, t);
        }
        if c.ui_scale {
            ctx.set_zoom_factor(self.settings.ui_scale());
        }
        if c.layer_colors {
            view::style::set_layer_colors(self.settings.layer_colors());
            self.restyle(ctx);
        }
        if c.keymap {
            self.keymap = self.settings.keymap();
        }
        match c.agent {
            Some(AgentCmd::Start) => {
                self.agent.port = self.settings.mcp.port;
                self.agent.start(ctx);
            }
            Some(AgentCmd::Stop) => self.agent.stop(),
            Some(AgentCmd::Restart) => {
                self.agent.stop();
                self.agent.port = self.settings.mcp.port;
                self.agent.start(ctx);
            }
            None => {}
        }
        if c.forget_tools {
            self.settings.tools.clear();
            self.sch.settings = SchSettings::default();
            self.pcb.settings = PcbSettings { layer: self.pcb.settings.layer, ..Default::default() };
            for t in self.all_libs_mut().flat_map(|l| l.tabs.iter_mut()) {
                t.forget_tools();
            }
        }
        if c.library_tabs {
            self.limit_item_tabs(&[]);
        }
        if c.kicad_dir {
            self.kicad.set_root(self.settings.kicad_dir());
        }
        if c.ngspice {
            self.sim.forget_ngspice();
        }
        if c.save {
            self.save_settings();
        }
    }

    /// The main window's size and position (kept while maximized or fullscreen).
    fn track_window(&mut self, ctx: &egui::Context) {
        let info = ctx.input(|i| i.viewport().clone());
        let (Some(outer), Some(inner)) = (info.outer_rect, info.inner_rect) else { return };
        let maximized = info.maximized.unwrap_or(false);
        let fullscreen = info.fullscreen.unwrap_or(false);
        let mut state = self.window_state.or(self.settings.window).unwrap_or(crate::settings::WindowState {
            pos: None,
            size: [inner.width(), inner.height()],
            maximized: false,
            fullscreen: false,
        });
        if !maximized && !fullscreen {
            state.pos = Some([outer.min.x, outer.min.y]);
            state.size = [inner.width(), inner.height()];
        }
        state.maximized = maximized;
        state.fullscreen = fullscreen;
        self.window_state = Some(state);
    }

    /// Remember the tools' style choices (saved a second after the last change), and hand
    /// a library editor's changes to the other library editors.
    fn sync_tool_memory(&mut self) {
        use view::tool_memory::remember;
        // (family, value, default)
        let mut current = vec![
            ("schematic", remember(&self.sch.settings), remember(&SchSettings::default())),
            ("board", remember(&self.pcb.settings), remember(&PcbSettings::default())),
        ];
        if let Some(ed) = self.focused_tab() {
            let defaults = ItemTab::default_remembered();
            current.extend(ed.remembered().into_iter().zip(defaults).map(|((k, v), (_, d))| (k, v, d)));
        }
        let tools = &mut self.settings.tools;
        let mut changed = false;
        for (key, v, default) in current {
            let unchanged = match tools.get(key) {
                Some(old) => *old == v,
                None => v == default,
            };
            if !unchanged {
                tools.insert(key.to_string(), v);
                changed = true;
            }
        }
        // Editors opened since, and the others after a change, take the remembered ones.
        let tools = &self.settings.tools;
        let tabs = std::iter::once(&mut self.project).chain(self.libraries.iter_mut()).flat_map(|l| l.tabs.iter_mut());
        for t in tabs {
            t.recall(tools);
        }
        if changed {
            self.tools_changed = Some(Instant::now());
        }
        if self.tools_changed.is_some_and(|t| t.elapsed() > Duration::from_secs(1)) {
            self.tools_changed = None;
            self.save_settings();
        }
    }

    /// While a shortcut is being recorded, the next key press (with modifiers) becomes it.
    /// Returns true if input was consumed.
    fn record_shortcut(&mut self, ctx: &egui::Context) -> bool {
        let Some(action) = self.settings_win.recording else { return false };
        let pressed = ctx.input(|i| {
            i.events.iter().find_map(|e| match e {
                egui::Event::Key { key, pressed: true, modifiers, .. } => Some((*key, *modifiers)),
                _ => None,
            })
        });
        let Some((key, m)) = pressed else { return true };
        self.settings_win.recording = None;
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
        let p = self.doc().project();
        let refs: Vec<&Library> = self.libraries.iter().map(|l| l.lib.library()).collect();
        let statuses = part_statuses(p, &refs);
        let changed = statuses.iter().filter(|s| s.state == PartState::LibraryChanged).count();

        let project_info = Info {
            strong: true,
            detail: self.doc().path.as_deref().map_or("not saved".into(), project_panel::short_path),
            path: self.doc().path.clone(),
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
            vec![
                schematic,
                Some(TreeNode::new(NodeId::Board, "PCB", Info::icon(Icon::Board))),
                Some(project_lib),
                Some(self.outputs.node(self.outputs_outdated)),
            ]
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
            } else if let Some(m) = self.libraries.missing.iter().find(|m| m.path == *path) {
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
        for l in self.libraries.iter() {
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
        match self.focused_lib_ref().zip(self.focused_tab()) {
            Some((l, t)) => {
                let owner = if l.is_project() { Owner::Project } else { Owner::Lib(l.key) };
                Some(NodeId::Item(owner, t.open.object_id()))
            }
            None if !self.is_open(self.active) => None,
            None => Some(match self.active {
                ViewId::Sheet(s) => NodeId::Sheet(s),
                ViewId::Board => NodeId::Board,
            }),
        }
    }

    fn project_panel_ui(&mut self, ui: &mut egui::Ui, roots: &[TreeNode]) {
        self.project_tree.expand_defaults(self.doc().root().sheet());
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
                if let Some(sh) = self.doc().project().schematic.sheets.get(sheet) {
                    self.rename = Some(Rename { what: Naming::Sheet(sheet), text: sh.name.clone(), focused: false });
                }
            }
            Act::MoveSheet { sheet, to } => {
                let group = view::interaction::new_group();
                match self.move_to_sheet_cmd(vec![ObjectId::Sheet(sheet)], to, group) {
                    Ok(renamed) => {
                        self.doc_mut().close_group();
                        let sch = &self.doc().project().schematic;
                        let msg = format!("Moved to {}{}", sch.display_path(sheet), renamed_note(renamed));
                        self.set_status(msg);
                    }
                    Err(e) => self.set_status(e),
                }
            }
            Act::DeleteSheet(sheet) => {
                let ids = vec![ObjectId::Sheet(sheet)];
                if self.confirm_sheet_delete(&ids)
                    && let Err(e) = self.doc_mut().execute(Command::Delete { ids })
                {
                    self.set_status(e.to_string());
                }
            }
            Act::SaveProject => {
                self.save(false);
            }
            Act::ExportGerber => self.export_gerber(),
            Act::OpenGerber(file) => self.open_gerber(file),
            Act::DeleteOutputs => self.delete_outputs(),
            Act::SaveProjectAs => {
                self.save(true);
            }
            Act::OpenFile(path) => {
                if let Err(e) = project_panel::open_file(&path) {
                    self.set_status(format!("Could not open {}: {e}", path.display()));
                }
            }
            Act::Reveal(path) => {
                if let Err(e) = project_panel::reveal(&path) {
                    self.set_status(format!("Could not show {}: {e}", path.display()));
                }
            }
            Act::NewLibrary => self.new_library(),
            Act::AddLibrary => self.open_library(),
            Act::ImportKicad => self.kicad.open = true,
            Act::OpenItem(key, open) => _ = self.open_item(key, open, false, true),
            Act::PreviewItem(key, open) => _ = self.open_item(key, open, true, false),
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
            Act::RenameCategory(key, group, name) => {
                let text = name.clone();
                self.rename = Some(Rename { what: Naming::Category(key, group, name), text, focused: false });
            }
            Act::Delete(key, item) => {
                let Some(l) = self.lib(key) else { return };
                if let Err(e) = l.lib.delete(item) {
                    self.set_status(e);
                }
            }
            Act::Place(Owner::Project, part) => self.place_part(None, part),
            Act::Place(Owner::Lib(key), part) => self.place_part(Some(key), part),
            Act::CopyToProject(key, part) => {
                let Some(l) = self.libraries.by_key(key) else { return };
                let issues = l.lib.library().validate();
                if let Some(first) = issues.first() {
                    return self.set_status(format!("Library has problems: {first}"));
                }
                let command = Command::import_parts(l.lib.library(), &[part]);
                match command.and_then(|c| self.doc_mut().execute(c)) {
                    Ok(_) => self.set_status("Copied to the Project Library"),
                    Err(e) => self.set_status(format!("Copy failed: {e}")),
                }
            }
            Act::Update(part) => {
                let refs: Vec<&Library> = self.libraries.iter().map(|l| l.lib.library()).collect();
                let statuses = part_statuses(self.doc().project(), &refs);
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
                let ids = editor_core::query::unused_library_objects(self.doc().project());
                let n = ids.iter().filter(|id| matches!(id, ObjectId::LibPart(_))).count();
                match self.doc_mut().execute(Command::Delete { ids }) {
                    Ok(_) => self.set_status(format!(
                        "Removed {} from the Project Library",
                        project_panel::count(n, "unused part")
                    )),
                    Err(e) => self.set_status(e.to_string()),
                }
            }
        }
    }

    /// Show a newly created library item in a tab of its own.
    fn show_created(&mut self, key: u64, created: Result<Open, String>) {
        match created {
            Ok(open) => _ = self.open_item(key, open, false, true),
            Err(e) => self.set_status(e),
        }
    }

    /// The rename dialog (from the Project panel).
    /// "Sheet", "Sheet 2", …: the first name not taken below `parent`.
    fn new_sheet_name(&self, parent: Id) -> String {
        let s = &self.doc().project().schematic;
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
            Naming::Category(..) => ("Rename category", "Rename"),
        };
        // Sheet names are checked as they are typed.
        let schematic = &self.project.lib.doc.project().schematic;
        let issue = match r.what {
            Naming::NewSheet(parent) => schematic.sheet_name_issue(Some(parent), &r.text, None),
            Naming::Sheet(s) => {
                schematic.sheet_name_issue(schematic.sheets.get(s).and_then(|s| s.parent), &r.text, Some(s))
            }
            Naming::Item(..) | Naming::Category(..) => None,
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
                if let Err(e) = l.lib.rename(item, &text) {
                    self.set_status(e);
                }
            }
            Naming::Category(key, group, from) => {
                let Some(l) = self.lib(key) else { return };
                let kind = match group {
                    project_panel::Group::Parts => model::ObjectKind::LibPart,
                    project_panel::Group::Symbols => model::ObjectKind::LibSymbol,
                    project_panel::Group::Footprints => model::ObjectKind::LibFootprint,
                };
                if let Err(e) = l.lib.rename_category(kind, &from, &text) {
                    self.set_status(e);
                }
            }
            Naming::Sheet(sheet) => {
                let Some(mut sh) = self.doc().project().schematic.sheets.get(sheet).cloned() else { return };
                sh.name = text;
                let objects = vec![Object::Sheet(sh)];
                if let Err(e) = self.doc_mut().execute(Command::Update { objects }) {
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

    /// A window edge's bar of panel buttons: its top and bottom groups; the left one
    /// also holds the bottom area's group at its foot. Buttons drag to other places.
    fn dock_bar(&mut self, ui: &mut egui::Ui, side: Side) {
        let frame = egui::Frame::NONE.fill(theme::window_bg()).inner_margin(egui::Margin::symmetric(0, 2));
        let panel = match side {
            Side::Left => egui::Panel::left("dock_bar_left"),
            _ => egui::Panel::right("dock_bar_right"),
        };
        let mut clicked = None;
        let mut zones = Vec::new();
        let dragging = self.dock_drag;
        panel.exact_size(DOCK_BAR).resizable(false).show_separator_line(false).frame(frame).show(ui, |ui| {
            ui.vertical_centered(|ui| {
                ui.spacing_mut().item_spacing.y = 4.0;
                let mut group_ui = |ui: &mut egui::Ui, group: DockGroup, zones: &mut Vec<DropZone>| {
                    let mut buttons = Vec::new();
                    for &panel in self.settings.dock.panels(group) {
                        let badge = match panel {
                            Panel::Messages if !self.settings.dock.is_open(panel) => {
                                Some(Badge::Problems { new: self.unseen.new, total: self.unseen.total })
                            }
                            Panel::Activity => self.agent.url().map(Badge::Live),
                            _ => None,
                        };
                        let open = self.settings.dock.is_open(panel);
                        let r = dock_button(ui, panel, open, badge, &self.keymap, dragging == Some(panel));
                        if r.clicked() {
                            clicked = Some(panel);
                        }
                        if r.drag_started() {
                            self.dock_drag = Some(panel);
                        }
                        buttons.push(r.rect);
                    }
                    // An empty group is still a place to drop on while dragging.
                    let rect = match (buttons.first(), buttons.last()) {
                        (Some(a), Some(b)) => a.union(*b),
                        _ => {
                            let h = if dragging.is_some() { DOCK_BUTTON } else { 4.0 };
                            ui.allocate_exact_size(egui::vec2(DOCK_BUTTON, h), egui::Sense::hover()).0
                        }
                    };
                    zones.push(DropZone { group, rect, buttons });
                };
                let divider = |ui: &mut egui::Ui| {
                    let (r, _) = ui.allocate_exact_size(egui::vec2(20.0, 9.0), egui::Sense::hover());
                    ui.painter().hline(r.x_range(), r.center().y, egui::Stroke::new(1.0, theme::border()));
                };
                let [top, bottom] = match side {
                    Side::Left => [DockGroup::LeftTop, DockGroup::LeftBottom],
                    _ => [DockGroup::RightTop, DockGroup::RightBottom],
                };
                group_ui(ui, top, &mut zones);
                divider(ui);
                group_ui(ui, bottom, &mut zones);
                if side == Side::Left {
                    // The bottom area's panels sit at the bar's foot.
                    ui.with_layout(egui::Layout::bottom_up(egui::Align::Center), |ui| {
                        let start = ui.cursor().bottom();
                        let n = self.settings.dock.panels(DockGroup::Bottom).len().max(1) as f32;
                        let height = n * DOCK_BUTTON + (n - 1.0) * 4.0;
                        let (r, _) = ui.allocate_exact_size(egui::vec2(DOCK_BUTTON, height), egui::Sense::hover());
                        let mut child = ui.new_child(
                            egui::UiBuilder::new()
                                .max_rect(egui::Rect::from_min_max(egui::pos2(r.left(), start - height), r.max))
                                .layout(egui::Layout::top_down(egui::Align::Center)),
                        );
                        child.spacing_mut().item_spacing.y = 4.0;
                        group_ui(&mut child, DockGroup::Bottom, &mut zones);
                        divider(ui);
                    });
                }
            });
        });
        self.drop_zones.extend(zones);
        if let Some(p) = clicked {
            self.settings.dock.toggle(p);
        }
    }

    /// While a panel button is dragged: the button under the pointer and where it would go;
    /// on release, move it there. Call after both bars are drawn.
    fn dock_drag_ui(&mut self, ctx: &egui::Context) {
        let zones = std::mem::take(&mut self.drop_zones);
        let Some(panel) = self.dock_drag else { return };
        let (pos, down) = ctx.input(|i| (i.pointer.interact_pos(), i.pointer.primary_down()));
        let Some(pos) = pos else { return };
        let target = crate::dock::drop_target(&zones, pos);
        if !down {
            self.dock_drag = None;
            if let Some((group, index)) = target {
                self.settings.dock.move_panel(panel, group, index);
            }
            return;
        }
        let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Tooltip, egui::Id::unique("dock_drag")));
        if let Some((group, index)) = target
            && let Some(z) = zones.iter().find(|z| z.group == group)
        {
            let y = match (z.buttons.get(index), z.buttons.last()) {
                (Some(b), _) => b.top() - 2.0,
                (None, Some(b)) => b.bottom() + 2.0,
                (None, None) => z.rect.center().y,
            };
            painter.hline(z.rect.x_range().expand(2.0), y, egui::Stroke::new(2.0, theme::accent()));
        }
        let rect = egui::Rect::from_center_size(pos, egui::vec2(DOCK_BUTTON, DOCK_BUTTON));
        painter.rect_filled(rect, 6.0, theme::accent().gamma_multiply(0.35));
        icons::paint(&painter, rect.shrink(DOCK_ICON_INSET), panel.icon(), theme::accent());
        ctx.set_cursor_icon(egui::CursorIcon::Grabbing);
    }

    /// The panels shown on `side` (two stack with a draggable split), each with a title bar
    /// to hide it.
    fn dock_panel(&mut self, ui: &mut egui::Ui, side: Side) {
        let shown = self.settings.dock.shown(side);
        if shown.is_empty() {
            return;
        }
        let gap = theme::GAP;
        let container = match side {
            Side::Left => egui::Panel::left("dock_left"),
            Side::Right => egui::Panel::right("dock_right"),
            Side::Bottom => egui::Panel::bottom("dock_bottom"),
        };
        let margin = match side {
            Side::Bottom => egui::Margin { left: gap, right: gap, top: gap, bottom: 0 },
            _ => egui::Margin::default(),
        };
        let mut hide = None;
        let mut split = None;
        let r = container
            .frame(egui::Frame::NONE.outer_margin(margin))
            .show_separator_line(false)
            .resizable(true)
            .default_size(self.settings.dock.size(side))
            .size_range(if side == Side::Bottom { 80.0..=800.0 } else { 160.0..=700.0 })
            .show(ui, |ui| {
                // The panel's size is the user's: overflowing content is clipped.
                crate::dock::fixed_size(ui, |ui| {
                    let all = ui.available_rect_before_wrap();
                    let rects = match shown.len() {
                        1 => vec![all],
                        _ => {
                            let g = f32::from(gap);
                            let top_h = ((all.height() - g) * self.settings.dock.split(side)).round();
                            let top = egui::Rect::from_min_size(all.min, egui::vec2(all.width(), top_h));
                            let handle = egui::Rect::from_min_max(
                                egui::pos2(all.left(), top.bottom()),
                                egui::pos2(all.right(), top.bottom() + g),
                            );
                            let bottom = egui::Rect::from_min_max(egui::pos2(all.left(), handle.bottom()), all.max);
                            let id = ui.make_persistent_id(("dock_split", side as u8));
                            let r = ui.interact(handle, id, egui::Sense::drag());
                            if r.hovered() || r.dragged() {
                                ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeVertical);
                            }
                            if r.dragged()
                                && let Some(p) = r.interact_pointer_pos()
                            {
                                split = Some((p.y - all.top()) / (all.height() - g).max(1.0));
                            }
                            vec![top, bottom]
                        }
                    };
                    for ((group, panel), rect) in shown.iter().copied().zip(rects) {
                        ui.painter().rect_filled(rect, theme::RADIUS, theme::panel());
                        // The title row's shade, sized once the row is laid out.
                        let header = ui.painter().add(egui::Shape::Noop);
                        let mut child = ui.new_child(
                            egui::UiBuilder::new()
                                .max_rect(rect.shrink(8.0))
                                .layout(egui::Layout::top_down(egui::Align::Min)),
                        );
                        child.set_clip_rect(rect.shrink(4.0));
                        let (hidden, title_bottom) = self.panel_frame_ui(&mut child, group, panel);
                        if hidden {
                            hide = Some(group);
                        }
                        // As tall as the tab bars, or more with the Messages tabs below the title.
                        let bottom = (title_bottom + 5.0).max(rect.top() + theme::TITLE_ROW_HEIGHT);
                        let band = egui::Rect::from_min_max(rect.min, egui::pos2(rect.right(), bottom));
                        let r = theme::RADIUS;
                        let corners = egui::CornerRadius { nw: r, ne: r, sw: 0, se: 0 };
                        ui.painter().set(header, egui::Shape::rect_filled(band, corners, theme::panel_header()));
                    }
                    ui.advance_cursor_after_rect(all);
                })
            });
        if let Some(g) = hide {
            self.settings.dock.hide(g);
        }
        if let Some(s) = split {
            self.settings.dock.set_split(side, s);
        }
        let rect = r.response.rect;
        let size = if side == Side::Bottom { rect.height() } else { rect.width() };
        self.settings.dock.set_size(side, size);
    }

    /// A panel's title bar and contents. Returns whether its hide button was clicked and
    /// the bottom of the title row.
    fn panel_frame_ui(&mut self, ui: &mut egui::Ui, group: DockGroup, panel: Panel) -> (bool, f32) {
        let mut hide = false;
        let mut header_act = None;
        // The Messages tabs sit in the title bar, or below it in a narrow panel.
        let message_tabs = (panel == Panel::Messages)
            .then(|| messages::status(&self.project.lib.doc, &mut self.libraries, self.sim.session.attempt()))
            .map(|status| (status, ui.available_width() >= messages::TABS_WIDTH + 120.0));
        let title = ui.horizontal(|ui| {
            theme::bold_label(ui, panel.title(), theme::TITLE_SIZE, ui.visuals().strong_text_color());
            if let Some((status, true)) = &message_tabs {
                ui.add_space(8.0);
                messages::tabs_ui(ui, &mut self.message_tab, status);
            }
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
        let mut title_bottom = title.response.rect.bottom();
        if let Some((status, false)) = &message_tabs {
            ui.add_space(2.0);
            title_bottom =
                ui.horizontal(|ui| messages::tabs_ui(ui, &mut self.message_tab, status)).response.rect.bottom();
        }
        ui.add_space(8.0);
        // These scroll themselves.
        // One label column per panel (`fields::form_scope`).
        if matches!(panel, Panel::Messages | Panel::Nets | Panel::Activity) {
            crate::fields::form_scope(ui, panel, |ui| self.panel_ui(ui, panel));
        } else {
            egui::ScrollArea::vertical()
                .id_salt(("dock", group))
                .auto_shrink([false, false])
                .show(ui, |ui| crate::fields::form_scope(ui, panel, |ui| self.panel_ui(ui, panel)));
        }
        (hide, title_bottom)
    }

    /// The Parts panel, over the project's library, the loaded libraries (Built-in last)
    /// and for the editor last active.
    fn parts_ui(&mut self, ui: &mut egui::Ui) -> Option<crate::parts_ui::PartsAct> {
        let kind = match self.active.kind() {
            ViewKind::Schematic => crate::parts_ui::Kind::Schematic,
            ViewKind::Board => crate::parts_ui::Kind::Board,
        };
        // Board parts are made from footprints, schematic parts from symbols.
        let board = kind == crate::parts_ui::Kind::Board;
        let kicad = self.kicad.match_count(ui.ctx(), self.parts.query(), board);
        let issues: Vec<Option<String>> =
            self.all_libs_mut().map(|l| l.issues().first().map(|(_, m)| m.clone())).collect();
        // The fields, not `all_libs`: the panel is borrowed mutably alongside.
        let mut sources: Vec<crate::parts_ui::Source> = std::iter::once(&self.project)
            .chain(self.libraries.iter())
            .zip(issues)
            .map(|(l, issue)| crate::parts_ui::Source {
                key: l.key,
                title: if l.is_project() {
                    "Project Library".into()
                } else if l.is_builtin() {
                    "Built-in Library".into()
                } else {
                    l.lib.name().to_string()
                },
                doc: &l.lib,
                issue,
            })
            .collect();
        // Project Library first, Built-in last.
        sources.sort_by_key(|s| (s.key != PROJECT_LIB, s.doc.is_builtin()));
        self.parts.ui(ui, kind, &sources, kicad)
    }

    fn panel_ui(&mut self, ui: &mut egui::Ui, panel: Panel) {
        let mut message = None;
        match panel {
            Panel::Project => {
                let roots = self.tree_nodes();
                self.project_panel_ui(ui, &roots);
            }
            Panel::Properties => {
                if let Some(id) = self.focus.item() {
                    let before = self.tab_revision(id);
                    if let Some(mut ed) = self.editor(id) {
                        ed.properties_ui(ui);
                    }
                    return self.keep_if_edited(id, before);
                }
                let view = self.active;
                let mut selection = if self.is_open(view) { self.panes.selection(view) } else { Vec::new() };
                // Nothing selected on a sheet: the sheet itself (its page and the title block).
                if let (true, Some(s)) = (selection.is_empty() && self.is_open(view), view.sheet())
                    && self.doc().project().schematic.sheets.contains(s)
                {
                    selection = vec![ObjectId::Sheet(s)];
                }
                self.properties.ui(ui, &mut self.project.lib.doc, &selection, &mut message);
            }
            Panel::ToolSettings => {
                if let Some(id) = self.focus.item() {
                    let before = self.tab_revision(id);
                    if let Some(mut ed) = self.editor(id) {
                        ed.tool_settings_ui(ui);
                    }
                    return self.keep_if_edited(id, before);
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
            Panel::BoardSetup => pcb_ui::board_setup(ui, self.doc_mut(), &mut message),
            Panel::SchematicSetup => crate::checks_ui::schematic_setup(ui, self.doc_mut(), &mut message),
            Panel::Overview => match self.focused_lib_key().and_then(|k| self.lib(k)) {
                Some(l) => l.overview_ui(ui),
                None => crate::properties::project_summary(ui, self.doc()),
            },
            Panel::Messages => {
                let tab = self.message_tab;
                let attempt = self.sim.session.attempt();
                if let Some(show) = messages::ui(ui, tab, &mut self.project.lib.doc, &mut self.libraries, attempt) {
                    self.show_message(show);
                }
            }
            Panel::Parts => match self.parts_ui(ui) {
                Some(crate::parts_ui::PartsAct::Tree(act)) => self.tree_act(act),
                Some(crate::parts_ui::PartsAct::Kicad) => {
                    let board = self.active.kind() == ViewKind::Board;
                    let query = self.parts.query().to_string();
                    self.kicad.open_with(&query, board);
                }
                None => {}
            },
            Panel::Nets => match self.nets.ui(ui, &self.project.lib.doc) {
                Some(NetsAct::Select(net)) => self.select_net(&net, false),
                Some(NetsAct::Zoom(net)) => self.select_net(&net, true),
                Some(NetsAct::SetClass(net, class)) => {
                    let mut rules = self.doc().project().board.rules.clone();
                    rules.assign_net(&net, class.as_deref());
                    if let Err(e) = self.doc_mut().execute(Command::SetDesignRules { rules }) {
                        self.set_status(e.to_string());
                    }
                }
                None => {}
            },
            Panel::Simulation => {
                // Look for ngspice once the panel shows (its hint shows when missing).
                _ = self.sim.engine(&self.settings.engine());
                match self.sim.ui(ui, &mut self.project.lib.doc, &mut message) {
                    Some(SimAct::Run(id)) => self.run_simulation(Some(id)),
                    Some(SimAct::Stop) => self.sim.session.stop(),
                    Some(SimAct::ShowNetlist) => self.sim.show_window(false),
                    Some(SimAct::ShowLog) => self.sim.show_window(true),
                    Some(SimAct::OpenSettings) => {
                        self.settings_win.show(Some(SettingsPage::Simulation), &mut self.settings)
                    }
                    Some(SimAct::ShowMessages) => {
                        self.message_tab = messages::Category::Simulation;
                        self.messages_open = true;
                        self.settings.dock.show(Panel::Messages);
                    }
                    Some(SimAct::SelectNet(net)) => self.select_net(&net, false),
                    Some(SimAct::ShowPlot) => self.open_sim_tab(true),
                    None => {}
                }
            }
            Panel::Activity => {
                match crate::activity_ui::ui(
                    ui,
                    &self.agent.activity,
                    self.agent.is_running(),
                    self.agent.error.as_deref(),
                ) {
                    Some(crate::activity_ui::Act::Clear) => self.agent.activity.clear(),
                    Some(crate::activity_ui::Act::Start) => {
                        self.agent.port = self.mcp_port.unwrap_or(self.settings.mcp.port);
                        self.agent.start(ui.ctx());
                    }
                    None => {}
                }
                ui.ctx().request_repaint_after(Duration::from_secs(1));
            }
        }
        if let Some(m) = message {
            self.status = Some((m, Instant::now()));
        }
    }

    /// Runs an analysis (`None`: the one the Simulation panel shows; an operating point
    /// is added when there is none) and shows the panel.
    /// Replaces the tuner's rows (one undo step).
    fn set_tune_rows(&mut self, rows: Vec<TuneRow>) {
        let mut setup = self.doc().project().schematic.simulation.clone();
        setup.tune = rows;
        if let Err(e) = self.doc_mut().execute(Command::SetSimulation { simulation: setup }) {
            self.set_status(e.to_string());
        }
    }

    /// Writes tuned values into the schematic (one parameter's, else all), one undo step.
    fn apply_tune(&mut self, param: Option<TuneParam>) {
        let project = self.doc().project();
        let tuned: Vec<(TuneParam, f64)> =
            self.sim.tuned(project).into_iter().filter(|(p, _)| param.as_ref().is_none_or(|q| q == p)).collect();
        let overrides = sim::tune::Overrides::new(project, &tuned);
        let objects: Vec<Object> = overrides.components().cloned().map(Object::Component).collect();
        if objects.is_empty() {
            return;
        }
        match self.doc_mut().execute(Command::Update { objects }) {
            Ok(_) => {
                for (p, _) in &tuned {
                    self.sim.tuner.forget(p);
                }
            }
            Err(e) => self.set_status(e.to_string()),
        }
    }

    /// "Tune R1": adds the component's value (a source: its first field) to the tuner
    /// and shows the Simulation tab.
    fn tune_component(&mut self, component: Id) {
        let project = self.doc().project();
        let setup = &project.schematic.simulation;
        let Some(c) = project.schematic.components.get(component) else { return };
        let mut params = sim::tune::tunable_of(project, c);
        // A source's level first: its DC value, amplitude, high level.
        let rank = |p: &TuneParam| match p {
            TuneParam::Source { field, .. } => {
                ["value", "amplitude", "high", "v2"].iter().position(|f| f == field).unwrap_or(9)
            }
            TuneParam::Value { .. } => 0,
        };
        params.sort_by_key(rank);
        if !params.iter().any(|p| setup.tune.iter().any(|r| &r.param == p))
            && let Some(row) = params.first().and_then(|p| sim::tune::default_row(project, p))
        {
            let mut rows = setup.tune.clone();
            rows.push(row);
            self.set_tune_rows(rows);
        }
        self.open_sim_tab(true);
    }

    fn run_simulation(&mut self, analysis: Option<Id>) {
        self.commit_text_edit();
        self.settings.dock.show(Panel::Simulation);
        let setup = &self.doc().project().schematic.simulation;
        let id = match analysis.or(self.sim.current(setup).map(|a| a.id)) {
            Some(id) => id,
            None => {
                let mut setup = setup.clone();
                let a = Analysis::new("Operating point", AnalysisKind::Op);
                let id = a.id;
                setup.analyses.push(a);
                if let Err(e) = self.doc_mut().execute(Command::SetSimulation { simulation: setup }) {
                    return self.set_status(e.to_string());
                }
                id
            }
        };
        let engine = self.settings.engine();
        if let Err(e) = self.sim.run(&mut self.project.lib.doc, id, &engine) {
            self.set_status(e);
        }
    }

    /// Select a net (Nets panel): its wires, labels and junctions on the sheet worked on
    /// (or the first sheet it is on; its parts if it has no wires), which highlights its
    /// copper on the board. `zoom`: also show all of it in the active view.
    fn select_net(&mut self, name: &str, zoom: bool) {
        let doc = &self.project.lib.doc;
        let Some(net) = doc.connectivity().net(name) else { return };
        let sch = &doc.project().schematic;
        let mut objects: Vec<(Id, ObjectId, Vec<Point>)> = Vec::new();
        for &w in &net.wires {
            if let Some(w) = sch.wires.get(w) {
                objects.push((w.sheet, ObjectId::Wire(w.id), vec![w.a, w.b]));
            }
        }
        for &l in &net.labels {
            if let Some(l) = sch.labels.get(l) {
                objects.push((l.sheet, ObjectId::Label(l.id), vec![l.pos]));
            }
        }
        for &j in &net.junctions {
            if let Some(j) = sch.junctions.get(j) {
                objects.push((j.sheet, ObjectId::Junction(j.id), vec![j.pos]));
            }
        }
        if objects.is_empty() {
            for p in &net.pins {
                if let Some(s) = sch.symbols.get(p.symbol) {
                    objects.push((s.sheet, ObjectId::Symbol(s.id), vec![s.pos]));
                }
            }
        }
        let here = self.sheet_view().sheet();
        let Some(sheet) = here.filter(|s| objects.iter().any(|o| o.0 == *s)).or(objects.first().map(|o| o.0)) else {
            return;
        };
        let view = ViewId::Sheet(sheet);
        let on_sheet: Vec<&(Id, ObjectId, Vec<Point>)> = objects.iter().filter(|o| o.0 == sheet).collect();
        let mut ids: Vec<ObjectId> = on_sheet.iter().map(|o| o.1).collect();
        ids.dedup();
        let points: Vec<Point> = on_sheet.iter().flat_map(|o| o.2.iter().copied()).collect();
        let board_bounds = zoom
            .then(|| {
                let copper = self.doc().copper();
                let i = copper.net_index(name)?;
                copper.items_of_net(i).map(|c| c.geo.bounds()).reduce(|a, b| a.union(b))
            })
            .flatten();
        if self.active == ViewId::Board {
            // The board shows the net through the schematic's selection.
            let mut doc = self.project.lib.doc.target(ViewId::Board);
            self.panes.get(ViewId::Board).it.set_selection(&mut doc, Vec::new());
        } else {
            self.show_view(view);
        }
        let pane = self.panes.get(view);
        pane.it.set_selection(&mut self.project.lib.doc.target(view), ids);
        if !zoom {
            return;
        }
        let fit = |pane: &mut Pane, r: Rect| {
            let b = [to_mm(r.min.x), to_mm(r.min.y), to_mm(r.max.x), to_mm(r.max.y)];
            let pad = 5.0;
            let vp = pane.canvas.viewport();
            pane.canvas.camera.fit([b[0] - pad, b[1] - pad, b[2] + pad, b[3] + pad], vp, 32.0);
        };
        if self.active == ViewId::Board {
            if let Some(r) = board_bounds {
                fit(self.panes.get(ViewId::Board), r);
            }
        } else if let Some(r) = points.iter().map(|&p| Rect::from_points(p, p)).reduce(|a, b| a.union(b)) {
            fit(self.panes.get(view), r);
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
            messages::Show::LibraryPart(key, part) => _ = self.open_item(key, Open::Part(part), false, true),
        }
    }

    /// Per frame: the Messages badge, saving the dock.
    fn update_dock(&mut self, ctx: &egui::Context) {
        if let Some((tab, before)) = self.array_dock
            && !self.lib_of_tab(tab).and_then(|l| l.tab(tab)).is_some_and(|t| t.in_array())
        {
            self.array_dock = None;
            if self.settings.dock.is_open(Panel::ToolSettings) {
                match before {
                    Some(p) => self.settings.dock.show(p),
                    None => self.settings.dock.hide(self.settings.dock.group_of(Panel::ToolSettings)),
                }
            }
        }
        // Opening the Messages panel on a fine category shows the first one with problems.
        let open = self.settings.dock.is_open(Panel::Messages);
        if open && !self.messages_open {
            let status = messages::status(&self.project.lib.doc, &mut self.libraries, self.sim.session.attempt());
            self.message_tab = messages::tab_to_open(self.message_tab, &status);
        }
        self.messages_open = open;
        let keys = messages::keys(&self.project.lib.doc, &mut self.libraries, self.sim.session.attempt());
        self.unseen.update(keys, self.settings.dock.is_open(Panel::Messages));
        // Save once a resize is done.
        if self.settings.dock != self.saved_dock && !ctx.input(|i| i.pointer.any_down()) {
            self.saved_dock = self.settings.dock.clone();
            self.save_settings();
        }
    }

    fn status_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            let lib = self.focused_lib_ref().zip(self.focused_tab());
            // Nothing about an editor when none is showing (all view tabs closed).
            let editor = match lib {
                Some((l, t)) => Some({
                    let what = match t.open {
                        Open::Item(Item::Symbol(_)) => "Symbol editor",
                        Open::Item(Item::Footprint(_)) => "Footprint editor",
                        Open::Part(_) => "Part",
                    };
                    (
                        format!("{what} · {}", l.lib.name()),
                        t.canvas.cursor_world,
                        (t.it.grid, t.it.snap),
                        t.it.selection.len(),
                    )
                }),
                None if !self.is_open(self.active) => None,
                None => {
                    let view = self.hovered.unwrap_or(self.active);
                    let title = match view {
                        ViewId::Board => "PCB".to_string(),
                        ViewId::Sheet(s) => match self.doc().project().schematic.sheet_path(s) {
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
                if let Some(text) = self.sim.running_text() {
                    let r = ui
                        .add(egui::Label::new(RichText::new(text).color(theme::accent())).sense(egui::Sense::click()))
                        .on_hover_text("Show the Simulation panel (⌘. stops it)");
                    ui.spinner();
                    if r.clicked() {
                        self.settings.dock.show(Panel::Simulation);
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
        match &self.doc().path {
            Some(p) => p.file_name().map_or(String::new(), |n| n.to_string_lossy().into_owned()),
            None => self.doc().project().name.clone(),
        }
    }

    fn update_title(&mut self, ctx: &egui::Context) {
        let name = self.project_name();
        let title = format!("{}{} — PCB Designer", name, if self.doc().is_modified() { " •" } else { "" });
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
        if !self.doc().is_modified() && modified_libs.is_empty() {
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

/// Paste `clip` into `view` under `pointer` (else where it was copied from) and let it
/// follow the pointer.
fn paste_and_carry(
    doc: &mut Document,
    pane: &mut Pane,
    view: ViewId,
    clip: Clip,
    pointer: Option<Point>,
) -> Result<(), String> {
    let at = pointer.map_or(clip.anchor, |p| pane.it.snap(p));
    let group = view::interaction::new_group();
    let placed = view::clipboard::paste(doc, view, clip, at, Some(group))?;
    pane.it.carry(&mut doc.target(view), placed, at, pointer.unwrap_or(at), group);
    Ok(())
}

fn copied(action: Action, n: usize) -> String {
    let verb = if action == Action::Cut { "Cut" } else { "Copied" };
    if n == 1 { format!("{verb} 1 object") } else { format!("{verb} {n} objects") }
}

fn key_msg(l: &mut ItemEditor, action: KeyAction) -> Option<String> {
    l.key(action);
    None
}

fn tool_msg(l: &mut ItemEditor, tool: Tool) -> Option<String> {
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
const DOCK_BAR: f32 = 40.0;
/// The size of a tool window button in the dock bars: a 24 pt icon, [`DOCK_ICON_INSET`]
/// around it.
const DOCK_BUTTON: f32 = 32.0;
/// Space between a dock button's edge and its icon.
const DOCK_ICON_INSET: f32 = 4.0;

/// A mark on a panel's bar button.
enum Badge {
    /// Problems: a count bubble for new ones, a dot if only old ones remain.
    Problems { new: usize, total: usize },
    /// A server is on (the MCP server, its URL): a green dot.
    Live(String),
}

/// A panel's bar button: its icon, accent-tinted while the panel is open, and its badge.
fn dock_button(
    ui: &mut egui::Ui,
    panel: Panel,
    open: bool,
    badge: Option<Badge>,
    keymap: &Keymap,
    dragged: bool,
) -> egui::Response {
    let (rect, r) = ui.allocate_exact_size(egui::vec2(DOCK_BUTTON, DOCK_BUTTON), egui::Sense::click_and_drag());
    let painter = ui.painter();
    if dragged {
        painter.rect_stroke(rect, 6.0, egui::Stroke::new(1.0, theme::border()), egui::StrokeKind::Inside);
        return r;
    }
    if open {
        painter.rect_filled(rect, 6.0, theme::accent().gamma_multiply(0.28));
    } else if r.hovered() {
        painter.rect_filled(rect, 6.0, ui.visuals().widgets.hovered.weak_bg_fill);
    }
    let color = if open { theme::accent() } else { ui.visuals().text_color() };
    icons::paint(painter, rect.shrink(DOCK_ICON_INSET), panel.icon(), color);
    let mut tip = panel.title().to_string();
    match &badge {
        Some(Badge::Problems { new, .. }) if *new > 0 => {
            let new = *new;
            let text = if new > 99 { "99+".to_string() } else { new.to_string() };
            let font = egui::FontId::proportional(9.5);
            let galley = painter.layout_no_wrap(text, font, egui::Color32::BLACK);
            let w = (galley.size().x + 6.0).max(14.0);
            let center = rect.right_top() + egui::vec2(-4.0, 5.0);
            let bubble = egui::Rect::from_center_size(center, egui::vec2(w, 14.0));
            painter.rect_filled(bubble, 7.0, theme::warn());
            painter.galley(bubble.center() - galley.size() / 2.0, galley, egui::Color32::BLACK);
        }
        Some(Badge::Problems { total, .. }) if *total > 0 => {
            painter.circle_filled(rect.right_top() + egui::vec2(-5.0, 5.0), 3.5, theme::warn());
        }
        Some(Badge::Live(url)) => {
            painter.circle_filled(rect.right_top() + egui::vec2(-5.0, 5.0), 3.5, theme::success());
            tip = format!("{tip} (MCP server on: {url})");
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
    if !keymap.shortcuts(panel.action()).is_empty() {
        tip = format!("{tip}  {shortcut}");
    }
    r.on_hover_text(tip)
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
        self.0.libraries.missing.clone()
    }

    fn push(&mut self, lib: LibraryDoc) -> usize {
        self.0.add_library(lib, false);
        self.0.libraries.len() - 1
    }

    fn remove(&mut self, index: usize) {
        let key = self.0.libraries[index].key;
        self.0.unload_library(key);
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
    focus: &'a mut Focus,
    carry_clip: &'a mut Option<(ViewId, Clip)>,
    hovered: &'a mut Option<ViewId>,
    deferred: &'a mut Vec<Deferred>,
    sch: &'a mut SchPlacer,
    pcb: &'a mut PcbPlacer,
    status: &'a mut Option<(String, Instant)>,
    keymap: &'a Keymap,
    gerber: &'a mut GerberPane,
    gerber_outdated: bool,
    sim: &'a mut SimPanel,
    /// The Simulation panel or tab is open.
    sim_shown: bool,
}

/// The operating point of a part with more than two pins, as a tooltip at the pointer:
/// each pin's voltage and current (into the pin) and the power.
fn device_tip(ui: &egui::Ui, d: &sim::report::Device, stale: bool) {
    egui::Tooltip::always_open(
        ui.ctx().clone(),
        ui.layer_id(),
        egui::Id::unique("device_tip"),
        egui::PopupAnchor::Pointer,
    )
    .gap(14.0)
    .show(|ui| {
        ui.label(egui::RichText::new(&d.refdes).strong());
        let f = |v: Option<f64>, unit| v.map_or("–".to_string(), |v| model::si::format(v, unit));
        egui::Grid::new("device_tip_grid").spacing([12.0, 2.0]).show(ui, |ui| {
            for t in ["Pin", "Voltage", "Current in"] {
                ui.label(egui::RichText::new(t).color(theme::text_dim()).small());
            }
            ui.end_row();
            for p in &d.pins {
                ui.label(&p.key);
                ui.label(egui::RichText::new(f(p.voltage, "V")).monospace());
                ui.label(egui::RichText::new(f(p.current, "A")).monospace());
                ui.end_row();
            }
        });
        ui.label(format!("Power {}", f(d.power, "W")));
        if stale {
            ui.label(egui::RichText::new("Changed since the run").color(theme::warn()).small());
        }
    });
}

/// Make `sheet` the one the schematic tools work on, clearing their preview on the sheet
/// they worked on before.
fn switch_sheet(sch: &mut SchPlacer, doc: &mut Document, sheet: Id) {
    if let Some(old) = sch.set_sheet(sheet) {
        let (project, view) = doc.project_and_view(ViewId::Sheet(old));
        view.set_preview(project, &[]);
    }
}

/// The Probe tool on a sheet: the target under the pointer is highlighted with a tip; a
/// click adds or removes its probe.
#[allow(clippy::too_many_arguments)]
fn probe_ui(
    ui: &egui::Ui,
    doc: &mut Document,
    pane: &Pane,
    sim: &mut SimPanel,
    status: &mut Option<(String, Instant)>,
    view: ViewId,
    response: &egui::Response,
    on_edit: bool,
) {
    let tol = (6.0 / pane.canvas.camera.zoom * 1e6) as Coord;
    let power = ui.input(|i| i.modifiers.alt);
    let under = response.hover_pos().and_then(|p| {
        let [x, y] = pane.canvas.screen_to_world(p);
        sim_overlay::probe_target(doc.project(), doc.connectivity(), doc.view(view), Point::mm(x, y), tol, power)
    });
    if response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
    }
    let (pressed, down, released) =
        ui.input(|i| (i.pointer.primary_pressed(), i.pointer.primary_down(), i.pointer.primary_released()));
    if pressed && response.hovered() && !on_edit {
        sim.probe_drag = under.clone().zip(response.hover_pos());
    }
    // A drag from a net to another: the first net's voltage to the second.
    let target = match (&sim.probe_drag, &under) {
        (Some((ProbeTarget::Net { net, at }, _)), Some(ProbeTarget::Net { net: other, at: other_at }))
            if net != other =>
        {
            Some(ProbeTarget::Diff { net: net.clone(), at: *at, reference: other.clone(), ref_at: *other_at })
        }
        (Some((start, _)), _) => Some(start.clone()),
        (None, _) => under,
    };
    if let Some((ProbeTarget::Net { .. }, from)) = &sim.probe_drag
        && down
        && let Some(to) = ui.input(|i| i.pointer.latest_pos())
    {
        let color = theme::accent();
        ui.painter().line_segment([*from, to], egui::Stroke::new(1.5, color));
        ui.painter().circle_filled(*from, 3.0, color);
    }
    if released && let Some(t) = sim.probe_drag.take().and(target.clone()) {
        if let Err(e) = sim.toggle_probe(doc, t.probe()) {
            *status = Some((e, Instant::now()));
        }
        return;
    }
    let Some(target) = target else { return };
    let probed = sim.is_probed(doc, &target);
    let dragging_net = matches!(sim.probe_drag, Some((ProbeTarget::Net { .. }, from))
        if ui.input(|i| i.pointer.latest_pos()).is_some_and(|p| p.distance(from) > 4.0));
    let hint = if dragging_net && matches!(target, ProbeTarget::Net { .. }) {
        " · drop on another net for the voltage between them"
    } else if probed {
        " · click removes the probe"
    } else if matches!(target, ProbeTarget::Net { .. }) && sim.probe_drag.is_none() {
        " · drag to another net for the voltage between them"
    } else {
        ""
    };
    let tip = format!("{}{hint}", target.describe(doc.project()));
    egui::Tooltip::always_open(
        ui.ctx().clone(),
        ui.layer_id(),
        egui::Id::unique("probe_tip"),
        egui::PopupAnchor::Pointer,
    )
    .gap(14.0)
    .show(|ui| ui.label(tip));
    sim.hover = Some(target);
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
        let picked = view.sheet().and_then(|_| sch_ui::toolbar(ui, rect, self.sch, self.keymap, self.sim_shown));
        if picked == Some(sch_ui::Pick::Run) {
            self.deferred.push(Deferred::Action(Action::RunSimulation));
        }
        if let Some(sheet) = view.sheet()
            && let Some(sch_ui::Pick::Tool(tool)) = picked
        {
            let pane = self.panes.get(view);
            if tool != SchTool::Select {
                pane.it.reset();
                pane.it.refresh(&mut doc.target(view));
            }
            switch_sheet(self.sch, doc, sheet);
            self.sch.set_tool(tool);
            *self.active = view;
            *self.focus = Focus::Project;
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
            *self.focus = Focus::Project;
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
        if response.hovered() {
            *self.hovered = Some(view);
        }
        if response.hovered() && ui.input(|i| i.pointer.any_pressed()) {
            *self.active = view;
            *self.focus = Focus::Project;
        }
        // The schematic tools work on the sheet the pointer is in.
        let sheet = view.sheet();
        if let Some(s) = sheet
            && self.sch.is_placing()
            && response.hovered()
        {
            switch_sheet(self.sch, doc, s);
        }
        let pane = self.panes.get(view);
        // The text being edited takes presses on it; a press elsewhere commits it.
        let pointer = response.hover_pos().map(|p| {
            let [x, y] = pane.canvas.screen_to_world(p);
            Point::mm(x, y)
        });
        let editing_here = doc.text_edit().is_some_and(|(v, _)| v == view::text_edit::EditSite::View(view));
        let (pressed, down, shift) =
            ui.input(|i| (i.pointer.primary_pressed(), i.pointer.primary_down(), i.modifiers.shift));
        let mut on_edit = false;
        if editing_here && let Some(p) = pointer {
            let inside = doc.text_edit().is_some_and(|(_, e)| e.contains(p));
            if pressed && response.hovered() {
                if inside {
                    doc.with_text_edit(|e| e.click(p, shift));
                    pane.text_drag = true;
                } else if let Err(e) = doc.commit_text_edit() {
                    *self.status = Some((e, Instant::now()));
                }
            } else if pane.text_drag && down {
                doc.with_text_edit(|e| e.drag(p));
            }
            if response.double_clicked() && inside {
                doc.with_text_edit(|e| e.select_word(p));
            }
            on_edit = inside || pane.text_drag;
        }
        if !down {
            pane.text_drag = false;
        }
        let tools_here = sheet.is_some() && self.sch.sheet == sheet;
        if sheet.is_some()
            && let Some(p) = response.hover_pos()
            && !doc.sim_overlay().voltages.is_empty()
        {
            let [x, y] = pane.canvas.screen_to_world(p);
            let tol = (4.0 / pane.canvas.camera.zoom * 1e6) as Coord;
            if let Some(ObjectId::Symbol(s)) = doc.view(view).hit(doc.project(), Point::mm(x, y), tol)
                && let Some(c) = doc.project().schematic.symbols.get(s).map(|s| s.component)
                && let Some(d) = self.sim.device(c).filter(|d| d.pins.len() > 2)
            {
                device_tip(ui, d, doc.sim_overlay().dim);
            }
        }
        if sheet.is_some() && self.sch.tool() == SchTool::Probe {
            if tools_here {
                probe_ui(ui, doc, pane, self.sim, self.status, view, &response, on_edit);
            }
        } else if sheet.is_some() && self.sch.is_placing() {
            if tools_here {
                let pixel = 1e6 / pane.canvas.camera.zoom;
                let world = response.hover_pos().map(|p| {
                    let [x, y] = pane.canvas.screen_to_world(p);
                    self.sch.steer(Point::mm(x, y), pixel);
                    pane.it.snap(Point::mm(x, y))
                });
                self.sch.hover(doc, world);
                if let Some(p) = world
                    && response.hovered()
                    && !on_edit
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
            if let Some((raw, _)) = world {
                self.pcb.steer(raw, 1e6 / pane.canvas.camera.zoom);
            }
            (self.pcb.free, self.pcb.force) = ui.input(|i| (i.modifiers.alt, i.modifiers.shift));
            self.pcb.hover(doc, world, tol);
            if world.is_some()
                && response.hovered()
                && !on_edit
                && ui.input(|i| i.pointer.primary_pressed())
                && let Err(e) = self.pcb.click(doc)
            {
                *self.status = Some((e, Instant::now()));
            }
            if response.hovered() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
            }
        } else {
            // A paste carried on another sheet moves here when the pointer does.
            if response.hovered()
                && kind == ViewKind::Schematic
                && let Some((from, clip)) = self.carry_clip.clone()
                && from != view
                && from.kind() == ViewKind::Schematic
                && self.panes.get(from).it.is_carrying()
            {
                self.panes.get(from).it.key(&mut doc.target(from), KeyAction::Escape);
                let pane = self.panes.get(view);
                let pointer = pane.canvas.cursor_world.map(|[x, y]| Point::mm(x, y));
                match paste_and_carry(doc, pane, view, clip.clone(), pointer) {
                    Ok(()) => {
                        *self.carry_clip = Some((view, clip));
                        *self.active = view;
                    }
                    Err(e) => *self.status = Some((e, Instant::now())),
                }
            }
            let pane = self.panes.get(view);
            let mut on_grip = false;
            if kind == ViewKind::Board && !on_edit {
                let (consumed, err) = corner_handles(ui, &response, pane, doc);
                on_grip = consumed;
                if let Some(e) = err {
                    *self.status = Some((e, Instant::now()));
                }
            }
            let engaged = !pane.it.is_idle();
            let carried_off = pane.it.is_carrying() && !response.hovered();
            if let Some(input) = canvas::pointer_input(ui, &response, &pane.canvas, engaged, pane.it.hover.is_some())
                && !carried_off
                && !on_edit
                && !on_grip
            {
                pane.it.pointer(&mut doc.target(view), &input);
            }
            // Double-clicking a text edits it; a sheet symbol opens the sheet; on a wire
            // or track it selects the whole chain.
            if response.double_clicked() && !on_edit && !on_grip {
                let edits = match (pane.it.hover, pointer) {
                    (Some(h), Some(p)) => doc.begin_text_edit(view, h, p),
                    _ => false,
                };
                if edits {
                    pane.it.key(&mut doc.target(view), KeyAction::Escape);
                } else if let Some(ObjectId::Sheet(s)) = pane.it.hover {
                    self.deferred.push(Deferred::EnterSheet(s));
                } else {
                    let add = ui.input(|i| i.modifiers.shift || i.modifiers.command);
                    pane.it.double_click(&mut doc.target(view), add);
                }
            }
            // A right click selects what is under the pointer (unless it is selected
            // already) and opens the context menu.
            if response.secondary_clicked() && pane.it.is_idle() {
                if let Some(h) = pane.it.hover
                    && !pane.it.selection.contains(&h)
                {
                    pane.it.set_selection(&mut doc.target(view), vec![h]);
                } else if pane.it.hover.is_none() {
                    pane.it.set_selection(&mut doc.target(view), Vec::new());
                }
            }
            let selected = !pane.it.selection.is_empty();
            let movable = sheet.is_some_and(|s| !movable_ids(doc.project(), &pane.it.selection, s).is_empty());
            let rect = kind == ViewKind::Board
                && pane
                    .it
                    .selection
                    .iter()
                    .any(|&id| doc.project().get(id).is_some_and(|o| view::handles::rect_to_polygon(&o).is_some()));
            let kind = if sheet.is_some() { menu::CanvasKind::Schematic } else { menu::CanvasKind::Board };
            let entries = menu::canvas_menu(self.keymap, kind, selected, movable, rect, false);
            // One part (or its text) selected on a sheet whose values can be tuned:
            // "Tune R1".
            let tune = match pane.it.selection.as_slice() {
                [id @ (ObjectId::Symbol(_) | ObjectId::SymbolField(_))] if sheet.is_some() => {
                    let project = doc.project();
                    let sch = &project.schematic;
                    let symbol = match *id {
                        ObjectId::SymbolField(f) => sch.fields.get(f).map(|f| f.symbol),
                        ObjectId::Symbol(s) => Some(s),
                        _ => None,
                    };
                    symbol
                        .and_then(|s| sch.symbols.get(s))
                        .and_then(|s| sch.components.get(s.component))
                        .filter(|c| !sim::tune::tunable_of(project, c).is_empty())
                        .map(|c| (c.id, c.refdes.clone()))
                }
                _ => None,
            };
            response.context_menu(|ui| {
                if let Some(menu::Cmd::Action(a)) = menu::entries_menu_ui(ui, &entries) {
                    self.deferred.push(Deferred::Action(a));
                }
                if let Some((c, refdes)) = &tune {
                    ui.separator();
                    if ui
                        .button(format!("Tune {refdes}"))
                        .on_hover_text("Try its values in the Simulation tab's tuner")
                        .clicked()
                    {
                        self.deferred.push(Deferred::Tune(*c));
                        ui.close();
                    }
                }
            });
            if pane.it.is_dragging() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
            } else if pane.it.hover.is_some() && response.hovered() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            }
            if let Some((r, crossing)) = pane.it.box_rect() {
                canvas::draw_box(ui, &pane.canvas, response.rect, r, crossing);
            }
        }
        if on_edit && response.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Text);
        }
        if tools_here && let Some(ghosts) = self.sch.take_preview() {
            let (project, v) = doc.project_and_view(view);
            v.set_preview(project, &ghosts);
        }
        if kind == ViewKind::Board && self.pcb.tool() == PcbTool::Measure {
            let canvas = &self.panes.get(view).canvas;
            if self.pcb.settings.clearance {
                let (project, copper) = (doc.project(), doc.copper());
                let shapes = |f| editor_core::clearance::geometry(project, copper, f);
                let picked: Vec<_> = self.pcb.clearance.picked().flat_map(shapes).collect();
                let hover: Vec<_> = self.pcb.clearance.hover.into_iter().flat_map(shapes).collect();
                canvas::draw_clearance(ui, canvas, response.rect, &picked, &hover, self.pcb.gap.as_ref());
            } else {
                canvas::draw_measure(ui, canvas, response.rect, &self.pcb.measure);
            }
        }
        if kind == ViewKind::Board
            && let Some(ghosts) = self.pcb.take_preview(doc)
        {
            let (project, v) = doc.project_and_view(ViewId::Board);
            v.set_board_preview(project, &ghosts);
        }
        if !self.panes.get(view).it.is_idle() {
            ui.ctx().request_repaint();
        }
    }

    fn tab_of(tiles: &Tiles<Tab>, tile: TileId) -> Option<Tab> {
        match tiles.get(tile) {
            Some(Tile::Pane(tab)) => Some(*tab),
            _ => None,
        }
    }

    /// The library of an item tab.
    fn lib_of(&self, tab: u64) -> Option<&LibraryEditor> {
        std::iter::once(&*self.project).chain(self.libraries.iter()).find(|l| l.tab(tab).is_some())
    }

    fn lib_of_mut(&mut self, tab: u64) -> Option<&mut LibraryEditor> {
        std::iter::once(&mut *self.project).chain(self.libraries.iter_mut()).find(|l| l.tab(tab).is_some())
    }

    /// An item tab's icon: what it shows.
    fn item_icon(&self, tab: u64) -> Icon {
        let Some(l) = self.lib_of(tab) else { return Icon::Library };
        match l.tab(tab).map(|t| t.open) {
            Some(Open::Item(Item::Symbol(_))) => Icon::Symbol,
            Some(Open::Item(Item::Footprint(_))) => Icon::Footprint,
            Some(Open::Part(p)) if l.lib.library().parts.get(p).is_some_and(Part::is_board_part) => Icon::BoardPart,
            _ => Icon::Part,
        }
    }
}

impl Behavior<Tab> for TreeBehavior<'_> {
    fn tab_title_for_pane(&mut self, tab: &Tab) -> WidgetText {
        match tab {
            Tab::View(view) => PcbApp::view_title(self.project.lib.doc.project(), *view).into(),
            Tab::Item(id) => {
                let Some((l, t)) = self.lib_of(*id).and_then(|l| Some((l, l.tab(*id)?))) else {
                    return "".into();
                };
                let name = l.item_name(t.open).unwrap_or_default();
                let dot = if l.lib.is_modified() && !l.is_project() { " •" } else { "" };
                let text = RichText::new(format!("{name}{dot}"));
                if t.preview { text.italics().into() } else { text.into() }
            }
            Tab::Gerber => "Gerber".into(),
            Tab::Simulation => "Simulation".into(),
        }
    }

    fn pane_ui(&mut self, ui: &mut egui::Ui, _tile_id: TileId, tab: &mut Tab) -> UiResponse {
        match *tab {
            Tab::View(view) => self.view_ui(ui, view),
            Tab::Item(id) => {
                let keymap = self.keymap;
                let Some(mut ed) = self.lib_of_mut(id).and_then(|l| l.editor(id)) else { return UiResponse::None };
                let before = ed.lib.doc.revision;
                let pressed = ed.center_ui(ui, keymap);
                // An edit made in a preview tab keeps it.
                if ed.lib.doc.revision != before {
                    ed.tab.preview = false;
                }
                let (picked, navigate) = (ed.tab.picked.take(), ed.tab.navigate.take());
                if pressed || picked.is_some() {
                    *self.focus = Focus::Item(id);
                }
                if let Some(a) = picked {
                    self.deferred.push(Deferred::Action(a));
                }
                if let Some(open) = navigate {
                    self.deferred.push(Deferred::Navigate(id, open));
                }
            }
            Tab::Gerber => {
                if let Some(act) = outputs::gerber_ui(ui, self.gerber, self.gerber_outdated, self.keymap) {
                    self.deferred.push(Deferred::Gerber(act));
                }
            }
            Tab::Simulation => {
                if let Some(act) = crate::plot_ui::ui(ui, self.sim, &self.project.lib.doc) {
                    self.deferred.push(Deferred::Plot(act));
                }
            }
        }
        UiResponse::None
    }

    fn on_tab_button(&mut self, tiles: &mut Tiles<Tab>, tile_id: TileId, response: egui::Response) -> egui::Response {
        if response.clicked() || response.drag_started() || response.double_clicked() {
            match Self::tab_of(tiles, tile_id) {
                Some(Tab::Item(id)) => {
                    *self.focus = Focus::Item(id);
                    // A double click keeps a preview tab.
                    if response.double_clicked()
                        && let Some(t) = self.lib_of_mut(id).and_then(|l| l.tab_mut(id))
                    {
                        t.preview = false;
                    }
                }
                Some(Tab::View(view)) => {
                    *self.active = view;
                    *self.focus = Focus::Project;
                }
                Some(Tab::Gerber) => {
                    *self.focus = Focus::Gerber;
                }
                Some(Tab::Simulation) => {
                    *self.focus = Focus::Simulation;
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
            self.deferred.push(Deferred::Close(tab));
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
        theme::TITLE_ROW_HEIGHT
    }

    fn tab_bar_color(&self, _visuals: &egui::Visuals) -> egui::Color32 {
        theme::panel_header()
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
        let font_id = egui::FontId::proportional(theme::TITLE_SIZE);
        let galley = text.into_galley(ui, Some(egui::TextWrapMode::Extend), f32::INFINITY, font_id);
        let margin = 10.0;
        let close = 14.0;
        // An icon in the colour of what the tab shows: schematic wires, copper, libraries.
        let icon = match Self::tab_of(tiles, tile_id) {
            Some(Tab::View(ViewId::Sheet(_))) => Icon::Schematic,
            Some(Tab::View(ViewId::Board)) => Icon::Board,
            Some(Tab::Gerber) => Icon::Layers,
            Some(Tab::Simulation) => Icon::Simulation,
            Some(Tab::Item(id)) => self.item_icon(id),
            None => Icon::Library,
        };
        let icon_color = match icon {
            Icon::Layers => project_panel::icon_color(Icon::Board),
            i => project_panel::icon_color(i),
        };
        let icon_size = 15.0;
        let icon_gap = 6.0;
        let width = icon_size
            + icon_gap
            + galley.size().x
            + theme::BOLD
            + 2.0 * margin
            + if state.closable { close + 4.0 } else { 0.0 };
        let (_, rect) = ui.allocate_space(egui::vec2(width + 4.0, ui.available_height()));
        let pill = egui::Rect::from_min_max(
            egui::pos2(rect.left() + 4.0, rect.top() + 4.0),
            egui::pos2(rect.right(), rect.bottom() - 4.0),
        );
        let mut response =
            ui.interact(pill, id, egui::Sense::click_and_drag()).on_hover_cursor(egui::CursorIcon::Default);
        if let Some(Tab::Item(tab)) = Self::tab_of(tiles, tile_id)
            && let Some(l) = self.lib_of(tab)
        {
            let preview = l.tab(tab).is_some_and(|t| t.preview);
            let tip = if preview { format!("{}\nPreview — double-click to keep open", l.title()) } else { l.title() };
            response = response.on_hover_text(tip);
        }
        if ui.is_rect_visible(rect) && !state.is_being_dragged {
            let v = ui.visuals();
            // The active tab in the content's colour, on the bar's title-row shade.
            let fill = if state.active {
                theme::panel()
            } else if response.hovered() {
                v.widgets.inactive.weak_bg_fill.gamma_multiply(0.5)
            } else {
                egui::Color32::TRANSPARENT
            };
            ui.painter().rect_filled(pill, 6, fill);
            let color = if state.active { v.strong_text_color() } else { theme::text_dim() };
            let icon_color = if state.active { icon_color } else { icon_color.gamma_multiply(0.6) };
            let icon_rect = egui::Rect::from_center_size(
                egui::pos2(pill.left() + margin + icon_size / 2.0, pill.center().y),
                egui::vec2(icon_size, icon_size),
            );
            icons::paint(ui.painter(), icon_rect, icon, icon_color);
            let pos = egui::pos2(icon_rect.right() + icon_gap, pill.center().y - galley.size().y / 2.0);
            // The active tab's title bold (faked like `theme::bold_label`).
            let smear: &[f32] = if state.active { &[0.0, theme::BOLD / 2.0, theme::BOLD] } else { &[0.0] };
            for &dx in smear {
                ui.painter().galley(pos + egui::vec2(dx, 0.0), galley.clone(), color);
            }
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
                crate::native_menu::Picked::Key(s) => raw.events.push(s.event()),
                crate::native_menu::Picked::Cmd(cmd) => self.menu_cmds.push(cmd),
            }
        }
        let sheet = self.sheet_view();
        // A `run` step waits for the simulation.
        let simulating = self.sim.session.running().is_some();
        if let Some(script) = &mut self.script
            && !(simulating && script.waits_for_run())
        {
            let panes = &self.panes;
            let view = if script.view() == ViewKind::Board { ViewId::Board } else { sheet };
            raw.events.extend(
                script.events(|_, w| panes.find(view).map(|p| p.canvas.world_to_screen(w)).unwrap_or_default()),
            );
        }
        self.pointer_filter.filter(raw);
    }

    fn on_exit(&mut self) {
        if self.screenshot.is_some() || self.script.is_some() {
            return;
        }
        self.save_workspace();
        if let Some(w) = self.window_state {
            self.settings.window = Some(w);
        }
        self.save_settings();
    }

    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.track_window(&ctx);
        self.sync_tool_memory();
        self.sync_libraries();
        self.handle_close(&ctx);
        // The name dialog takes the keys while it is open.
        self.paste_text = ctx.input(|i| {
            i.events.iter().find_map(|e| if let egui::Event::Paste(t) = e { Some(t.clone()) } else { None })
        });
        let modal = self.name_prompt.is_some() || self.sheet_picker.is_some();
        if !modal && !ctx.egui_wants_keyboard_input() && self.editing_doc().is_some() {
            self.text_edit_input(&ctx);
        }
        if !self.record_shortcut(&ctx) && !modal {
            for action in self.keymap.triggered(&ctx, self.context()) {
                self.run_action(action);
            }
        }
        for cmd in std::mem::take(&mut self.menu_cmds) {
            self.run_menu_cmd(&ctx, cmd);
        }
        if let Some(text) = self.copy_out.take() {
            ctx.copy_text(text);
        }
        // The export folder: scanned when the project folder changes and when the window
        // gets focus back (files may have changed outside).
        let focused = ctx.input(|i| i.focused);
        if self.outputs_scanned.as_ref() != Some(&self.doc().path) || (focused && !self.window_focused) {
            self.refresh_outputs();
        }
        self.window_focused = focused;
        self.outputs_outdated = self.outputs.outdated(&self.project.lib.doc, self.busy());

        let gap = theme::GAP;
        let bar = |m: egui::Margin| egui::Frame::NONE.fill(theme::window_bg()).inner_margin(m);
        self.main_menu(ui, frame, gap);
        egui::Panel::bottom("status")
            .frame(bar(egui::Margin { left: 10, right: 10, top: 4, bottom: 4 }))
            .show_separator_line(false)
            .show(ui, |ui| self.status_bar(ui));
        self.dock_bar(ui, Side::Left);
        self.dock_bar(ui, Side::Right);
        self.dock_drag_ui(&ctx);
        self.dock_panel(ui, Side::Left);
        self.dock_panel(ui, Side::Right);
        self.dock_panel(ui, Side::Bottom);

        self.hovered = None;
        // Set again by whatever hovers a probe target this frame (the Probe tool, a table).
        self.sim.hover = None;
        let sim_shown =
            self.settings.dock.is_open(Panel::Simulation) || self.tree.tiles.find_pane(&Tab::Simulation).is_some();
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
                focus: &mut self.focus,
                carry_clip: &mut self.carry_clip,
                hovered: &mut self.hovered,
                deferred: &mut self.deferred,
                sch: &mut self.sch,
                pcb: &mut self.pcb,
                status: &mut self.status,
                keymap: &self.keymap,
                gerber: &mut self.gerber,
                gerber_outdated: self.outputs_outdated,
                sim: &mut self.sim,
                sim_shown,
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
            let view = if kind == ViewKind::Board { ViewId::Board } else { self.doc().root() };
            self.show_view(view);
        }
        for d in std::mem::take(&mut self.deferred) {
            match d {
                Deferred::EnterSheet(sheet) => self.show_view(ViewId::Sheet(sheet)),
                Deferred::Close(Tab::Item(id)) => self.close_item_tab(id),
                Deferred::Navigate(from, open) => {
                    if let Some(key) = self.lib_of_tab(from).map(|l| l.key) {
                        self.open_item(key, open, false, true);
                    }
                }
                Deferred::Close(Tab::View(view)) => self.close_view(view),
                Deferred::Close(Tab::Gerber) => {
                    if let Some(tile) = self.tree.tiles.find_pane(&Tab::Gerber) {
                        self.tree.remove_recursively(tile);
                    }
                }
                Deferred::Gerber(GerberAct::Reexport) => self.export_gerber(),
                Deferred::Gerber(GerberAct::Focus) => self.focus = Focus::Gerber,
                Deferred::Close(Tab::Simulation) => {
                    if let Some(tile) = self.tree.tiles.find_pane(&Tab::Simulation) {
                        self.tree.remove_recursively(tile);
                    }
                    self.sim.plot_closed = true;
                    self.follow_visible_tabs();
                }
                Deferred::Plot(PlotAct::Focus) => self.focus = Focus::Simulation,
                Deferred::Plot(PlotAct::Run(id)) => self.run_simulation(Some(id)),
                Deferred::Plot(PlotAct::RemoveProbe(analysis, i)) => {
                    if let Err(e) = SimPanel::remove_probe(self.doc_mut(), analysis, i) {
                        self.set_status(e);
                    }
                }
                Deferred::Plot(PlotAct::SetTune(rows)) => self.set_tune_rows(rows),
                Deferred::Plot(PlotAct::TuneRun) => {
                    let engine = self.settings.engine();
                    self.sim.tune_run(&mut self.project.lib.doc, &engine);
                }
                Deferred::Plot(PlotAct::ApplyTune(param)) => self.apply_tune(param),
                Deferred::Tune(component) => self.tune_component(component),
                Deferred::Action(a) => self.run_action(a),
            }
        }
        let overlay = self.sim.overlay(&self.project.lib.doc, sim_shown);
        self.project.lib.doc.set_sim_overlay(overlay);
        self.maintain_item_tabs();
        if let ViewId::Sheet(s) = self.active {
            self.last_sheet = Some(s);
        }
        let message = self.all_libs_mut().find_map(|l| l.message.take());
        if let Some(m) = message {
            self.set_status(m);
        }
        let save_copy = self.libraries.iter_mut().find_map(|l| std::mem::take(&mut l.save_copy).then_some(l.key));
        if let Some(key) = save_copy {
            self.save_library(key, true);
        }

        // Agent requests are answered between user interactions.
        if !self.busy() {
            let libraries: Vec<&Library> = self.libraries.iter().map(|l| l.lib.library()).collect();
            let jobs = self.agent.process(
                &ctx,
                &mut self.project.lib.doc,
                &mut self.panes,
                &libraries,
                &mut self.kicad,
                (&mut self.sim, self.settings.engine()),
            );
            for job in jobs {
                match job {
                    Job::Kicad(job, reply) => {
                        let result = self.agent_kicad_import(job).map(mcp::Reply::Json);
                        let _ = reply.send(result);
                    }
                    Job::ReplaceProject(import, reply) => {
                        let result = self.agent_replace_project(*import).map(mcp::Reply::Json);
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
        let busy = self.panes.iter().any(|p| !p.it.is_idle()) || self.project.tabs.iter().any(ItemTab::busy);
        if self.project.lib.doc.update_connectivity(busy) {
            self.probe_key = None;
            ctx.request_repaint();
        }

        self.update_highlights();

        // Model changed (any source): drop stale selections, refresh highlights.
        if self.doc().revision != self.revision {
            self.revision = self.doc().revision;
            self.close_missing_sheets();
            for pane in self.panes.iter_mut() {
                pane.it.refresh(&mut self.project.lib.doc.target(pane.view));
            }
        }
        if let Some(name) = self.script.as_mut().and_then(Script::take_run) {
            let setup = &self.doc().project().schematic.simulation;
            let id = setup.analyses.iter().find(|a| a.name == name).map(|a| a.id);
            if id.is_none() && !name.is_empty() {
                eprintln!("script: no analysis {name:?}");
            }
            self.run_simulation(id);
        }
        match self.sim.poll(&self.project.lib.doc) {
            Some(Err(m)) => self.set_status(m),
            // A sweep's first plot opens the tab (unless the user closed it).
            Some(Ok(true)) if !self.sim.plot_closed && self.tree.tiles.find_pane(&Tab::Simulation).is_none() => {
                self.open_sim_tab(false);
            }
            _ => {}
        }
        // A tuned value changed while the last run went: run it with the newest.
        if self.sim.tune_pending && self.sim.session.running().is_none() {
            let engine = self.settings.engine();
            self.sim.tune_run(&mut self.project.lib.doc, &engine);
        }
        if self.sim.session.running().is_some() {
            ctx.request_repaint_after(Duration::from_millis(100));
        }
        self.sim.window_ui(&ctx, &self.project.lib.doc);
        self.update_dock(&ctx);
        self.name_prompt_ui(&ctx);
        self.sheet_picker_ui(&ctx);
        self.rename_ui(&ctx);
        self.settings_window(&ctx);
        self.import_report_ui(&ctx);
        let libs: Vec<(u64, String)> = self.libraries.iter().map(|l| (l.key, l.lib.name().to_string())).collect();
        self.kicad.poll();
        if let Some(ImportAction::Import(target, req)) = self.kicad.ui(&ctx, &libs) {
            self.run_kicad_import(target, req);
        }
        self.update_title(&ctx);
        self.screenshot_tick(&ctx);
    }
}

/// What a KiCad project import reported, for its window.
/// Result of a KiCad project or parts import, shown in a dialog.
struct ImportReport {
    title: String,
    /// One line on what was imported (parts import).
    summary: Option<String>,
    /// Requested items that failed.
    errors: Vec<String>,
    report: kicad::Report,
}

impl ImportReport {
    fn new(path: &std::path::Path, report: kicad::Report) -> Self {
        let name = path.file_name().map_or(String::new(), |n| n.to_string_lossy().into_owned());
        ImportReport { title: format!("Imported {name}"), summary: None, errors: Vec::new(), report }
    }

    /// The result of importing parts into library `target`.
    fn parts(imported: &kicad::Imported, target: &str, added: usize, replaced: usize) -> Self {
        let summary = format!(
            "{} part(s), {} symbol(s), {} footprint(s) into {target}: {added} added, {replaced} updated.",
            imported.parts.len(),
            imported.symbols.len(),
            imported.footprints.len()
        );
        ImportReport {
            title: "Imported KiCad parts".into(),
            summary: Some(summary),
            errors: imported.errors.clone(),
            report: kicad::Report { notes: imported.warnings.clone(), ..Default::default() },
        }
    }

    fn failed(error: String) -> Self {
        ImportReport {
            title: "KiCad import failed".into(),
            summary: None,
            errors: vec![error],
            report: kicad::Report::default(),
        }
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
    children.sort_by(|a, b| model::natural_cmp(&a.name, &b.name));
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

/// How many tabs a tab group has (0 if it isn't one).
fn tabs_count(tree: &Tree<Tab>, group: TileId) -> usize {
    match tree.tiles.get(group) {
        Some(Tile::Container(Container::Tabs(t))) => t.children.len(),
        _ => 0,
    }
}

/// Splits tab group `group` (showing `front`): it becomes a horizontal split of its
/// tabs, left, and a new group with `tab`, right. Returns the new tab's tile.
fn split_right(tree: &mut Tree<Tab>, group: TileId, front: TileId, tab: Tab) -> TileId {
    let children = match tree.tiles.get(group) {
        Some(Tile::Container(Container::Tabs(t))) => t.children.clone(),
        _ => Vec::new(),
    };
    let mut left = egui_tiles::Tabs::new(children);
    left.set_active(front);
    let left = tree.tiles.insert_new(Tile::Container(Container::Tabs(left)));
    let pane = tree.tiles.insert_pane(tab);
    let right = tree.tiles.insert_tab_tile(vec![pane]);
    tree.tiles
        .insert(group, Tile::Container(Container::new_linear(egui_tiles::LinearDir::Horizontal, vec![left, right])));
    pane
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

/// The corner handles of the one selected zone, keepout or board drawing: drawn, and
/// dragged (one undo step per drag; Alt: no snapping, Shift: 0°/45°/90° to the next
/// corners). A double click on a corner deletes it. Returns whether the pointer input
/// went to a handle (the selection interaction skips it) and an error to show.
fn corner_handles(
    ui: &egui::Ui,
    response: &egui::Response,
    pane: &mut Pane,
    doc: &mut Document,
) -> (bool, Option<String>) {
    use view::handles;
    let nm_per_pt = 1e6 / pane.canvas.camera.zoom;
    let tol = (6.0 * nm_per_pt) as Coord;
    let (pos, pressed, down, alt, shift) = ui.input(|i| {
        (
            i.pointer.latest_pos(),
            i.pointer.primary_pressed(),
            i.pointer.primary_down(),
            i.modifiers.alt,
            i.modifiers.shift,
        )
    });
    let world = |p: egui::Pos2| {
        let [x, y] = pane.canvas.screen_to_world(p);
        Point::mm(x, y)
    };
    if let Some(d) = &mut pane.grip_drag {
        if !down {
            if d.moved {
                doc.close_group();
            }
            pane.grip_drag = None;
            return (true, None);
        }
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
        ui.ctx().request_repaint();
        let Some(pos) = pos else { return (true, None) };
        if !d.moved && pos.distance(d.start) < 4.0 {
            return (true, None);
        }
        d.moved = true;
        let raw = world(pos) + d.offset;
        let mut p = if alt { raw } else { d.snap.snap(raw, pane.it.snap(raw), tol, false).0 };
        if shift {
            p = handles::constrain(p, &handles::neighbours(&d.original, d.grip));
        }
        let Some(o) = handles::moved(&d.original, d.grip, p) else { return (true, None) };
        let err = doc.execute_grouped(Command::Update { objects: vec![o] }, Some(d.group)).err().map(|e| e.to_string());
        return (true, err);
    }
    let target = match pane.it.selection.as_slice() {
        [id] if pane.it.is_idle() => doc.project().get(*id).filter(handles::has_handles).map(|o| (*id, o)),
        _ => None,
    };
    let Some((id, obj)) = target else {
        pane.grip_hover = None;
        return (false, None);
    };
    let hs = handles::handles(&obj);
    let at = pos.filter(|_| response.hovered()).map(world);
    let hit = at.and_then(|p| handles::hit(&hs, p, tol));
    pane.grip_hover = hit.map(|h| (id, h.grip));
    canvas::draw_handles(ui, &pane.canvas, response.rect, &hs, hit.map(|h| h.grip));
    let Some(h) = hit else { return (false, None) };
    ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
    if response.double_clicked() {
        if !h.is_corner() {
            return (true, None);
        }
        let err = handles::without_corner(&obj, h.grip)
            .and_then(|o| doc.execute(Command::Update { objects: vec![o] }).map(|_| ()).map_err(|e| e.to_string()))
            .err();
        pane.grip_hover = None;
        return (true, err);
    }
    if pressed && let (Some(p), Some(start)) = (at, pos) {
        let snap = view::measure::BoardSnap::build(doc.project(), doc.revision);
        let group = view::interaction::new_group();
        pane.grip_drag =
            Some(GripDrag { original: obj, grip: h.grip, offset: h.pos - p, group, snap, moved: false, start });
        return (true, None);
    }
    (false, None)
}

/// "; N nets were renamed" for the status after a move to another sheet.
fn renamed_note(renamed: usize) -> String {
    match renamed {
        0 => String::new(),
        1 => "; 1 net was renamed".into(),
        n => format!("; {n} nets were renamed"),
    }
}

/// The objects of `selection` that lie on sheet `from` ("Move to sheet…" moves them).
/// A selected reference or value text stands for its symbol (fields go with their symbol).
fn movable_ids(project: &Project, selection: &[ObjectId], from: Id) -> Vec<ObjectId> {
    let sch = &project.schematic;
    let mut ids: Vec<ObjectId> = selection
        .iter()
        .map(|&id| match id {
            ObjectId::SymbolField(f) => sch.fields.get(f).map_or(id, |f| ObjectId::Symbol(f.symbol)),
            _ => id,
        })
        .filter(|id| sch.sheet_of(*id) == Some(from))
        .collect();
    ids.sort();
    ids.dedup();
    ids
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
