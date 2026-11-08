//! Main window: menu, library and layer panel, docked schematic and PCB editors,
//! properties panel, status bar.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use editor_core::Command;
use egui::{RichText, WidgetText};
use egui_tiles::{Behavior, SimplificationOptions, TileId, Tiles, Tree, UiResponse};
use model::*;
use render::gpu::Background;
use view::{Document, Interaction, KeyAction, PointerInput, ViewKind};

use crate::agent::{self, Agent};
use crate::canvas::{Canvas, CanvasResources};
use crate::keymap::{Action, Keymap};
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
}

const SCH_GRIDS: &[(f64, &str)] = &[(2.54, "2.54 mm"), (1.27, "1.27 mm"), (0.635, "0.635 mm")];
const BOARD_GRIDS: &[(f64, &str)] =
    &[(1.0, "1 mm"), (0.5, "0.5 mm"), (0.25, "0.25 mm"), (0.1, "0.1 mm"), (0.05, "0.05 mm"), (0.635, "25 mil")];

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
    tree: Tree<ViewKind>,
    doc: Document,
    panes: [Pane; 2],
    /// Pane receiving keyboard shortcuts and shown in the side panels.
    active: ViewKind,
    hovered: Option<ViewKind>,
    properties: Properties,
    starter: Library,
    status: Option<(String, Instant)>,
    title: String,
    revision: u64,
    screenshot: Option<(String, u32)>,
    allow_close: bool,
    keymap: Keymap,
    show_keymap: bool,
    agent: Agent,
}

impl PcbApp {
    pub fn new(cc: &eframe::CreationContext<'_>, opts: Options) -> Self {
        theme::apply(&cc.egui_ctx);
        let rs = cc.wgpu_render_state.as_ref().expect("wgpu renderer");
        CanvasResources::install(rs);

        let mut tiles = Tiles::default();
        let sch = tiles.insert_pane(ViewKind::Schematic);
        let pcb = tiles.insert_pane(ViewKind::Board);
        let root = tiles.insert_horizontal_tile(vec![sch, pcb]);
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
            hovered: None,
            properties: Properties::default(),
            starter: builtin::starter_library(),
            status,
            title: String::new(),
            revision: u64::MAX,
            screenshot: opts.screenshot.map(|p| (p, opts.screenshot_frames)),
            allow_close: false,
            keymap: opts.keymap,
            show_keymap: false,
            agent: Agent::new(opts.mcp.unwrap_or(agent::DEFAULT_PORT)),
        };
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
        self.panes.iter().any(|p| !p.it.is_idle())
    }

    // ------------------------------------------------------------------------------------
    // Files

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
        let answer = rfd::MessageDialog::new()
            .set_level(rfd::MessageLevel::Warning)
            .set_title("Unsaved changes")
            .set_description(format!("Save changes to \"{}\"?", self.doc.project().name))
            .set_buttons(rfd::MessageButtons::YesNoCancel)
            .show();
        match answer {
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

    /// Place a part from the starter library at the center of the schematic view.
    fn place_part(&mut self, part: Id) {
        if !self.doc.project().library.parts.contains(part) {
            let cmd = Command::ImportParts { library: self.starter.clone(), parts: vec![part] };
            if let Err(e) = self.doc.execute(cmd) {
                self.set_status(format!("Import failed: {e}"));
                return;
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
                let pane = &mut self.panes[0];
                pane.it.set_selection(&mut self.doc.target(ViewKind::Schematic), symbols);
            }
            Err(e) => self.set_status(format!("Place failed: {e}")),
        }
    }

    fn run_action(&mut self, action: Action) {
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
        };
        let kind = self.active;
        let pane = &mut self.panes[kind as usize];
        pane.it.key(&mut self.doc.target(kind), key_action);
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
                self.menu_item(ui, "New", Action::New, true);
                self.menu_item(ui, "Open…", Action::Open, true);
                if ui.button("Open demo project").clicked() && self.confirm_discard() {
                    self.replace_document(Document::new(view::demo::demo_project()));
                }
                ui.separator();
                self.menu_item(ui, "Save", Action::Save, true);
                self.menu_item(ui, "Save As…", Action::SaveAs, true);
                ui.separator();
                if ui.button("Quit").clicked() {
                    ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                }
            });
            ui.menu_button("Edit", |ui| {
                let h = self.doc.editor().history();
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
        ui.add_space(6.0);
        section(ui, "Library", |ui| {
            ui.label(RichText::new(&self.starter.name).color(theme::TEXT_DIM));
            let parts: Vec<(Id, String, String)> =
                self.starter.parts.iter().map(|p| (p.id, p.name.clone(), p.description.clone())).collect();
            for (id, name, description) in parts {
                ui.horizontal(|ui| {
                    let r = ui.button("Place");
                    ui.label(RichText::new(&name).strong()).on_hover_text(&description);
                    if r.clicked() {
                        self.place_part(id);
                    }
                });
            }
        });

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

    fn status_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            let pane = &self.panes[self.hovered.unwrap_or(self.active) as usize];
            ui.label(RichText::new(pane.title()).color(theme::ACCENT));
            match pane.canvas.cursor_world {
                Some([x, y]) => ui.monospace(format!("X {x:>9.3}  Y {y:>9.3} mm")),
                None => ui.monospace(" ".repeat(26)),
            };
            ui.separator();
            ui.monospace(format!("grid {} mm", to_mm(pane.it.grid)));
            ui.separator();
            let sel = &self.panes[self.active as usize].it.selection;
            ui.label(format!("{} selected", sel.len()));
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
        if ctx.input(|i| i.viewport().close_requested()) && !self.allow_close && self.doc.is_modified() {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            if self.confirm_discard() {
                self.allow_close = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }
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

fn section(ui: &mut egui::Ui, title: &str, add: impl FnOnce(&mut egui::Ui)) {
    egui::CollapsingHeader::new(RichText::new(title).strong()).default_open(true).show(ui, add);
    ui.add_space(4.0);
}

struct TreeBehavior<'a> {
    doc: &'a mut Document,
    panes: &'a mut [Pane; 2],
    active: &'a mut ViewKind,
    hovered: &'a mut Option<ViewKind>,
}

impl TreeBehavior<'_> {
    fn pointer_input(ui: &egui::Ui, response: &egui::Response, pane: &Pane) -> Option<PointerInput> {
        let engaged = !pane.it.is_idle();
        if !(response.hovered() || engaged || pane.it.hover.is_some()) {
            return None;
        }
        let px_to_nm = |px: f64| (px / pane.canvas.camera.zoom * 1e6) as Coord;
        Some(ui.input(|i| {
            let pos = i.pointer.latest_pos().filter(|_| response.hovered() || engaged);
            PointerInput {
                world: pos.map(|p| {
                    let [x, y] = pane.canvas.screen_to_world(p);
                    Point::mm(x, y)
                }),
                pressed: response.hovered() && i.pointer.primary_pressed(),
                released: i.pointer.primary_released(),
                down: i.pointer.primary_down(),
                shift: i.modifiers.shift,
                command: i.modifiers.command,
                tolerance: px_to_nm(5.0),
                drag_threshold: px_to_nm(4.0),
            }
        }))
    }
}

impl Behavior<ViewKind> for TreeBehavior<'_> {
    fn tab_title_for_pane(&mut self, kind: &ViewKind) -> WidgetText {
        self.panes[*kind as usize].title().into()
    }

    fn pane_ui(&mut self, ui: &mut egui::Ui, _tile_id: TileId, kind: &mut ViewKind) -> UiResponse {
        let kind = *kind;
        let pane = &mut self.panes[kind as usize];
        let response = pane.canvas.ui(ui, &mut self.doc.view_mut(kind).scene);
        if response.hovered() {
            *self.hovered = Some(kind);
        }
        if response.hovered() && ui.input(|i| i.pointer.any_pressed()) {
            *self.active = kind;
        }
        if let Some(input) = Self::pointer_input(ui, &response, pane) {
            pane.it.pointer(&mut self.doc.target(kind), &input);
        }
        if pane.it.is_dragging() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
        } else if pane.it.hover.is_some() && response.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }

        // Box selection: solid blue for "inside", dashed green for "crossing".
        if let Some((r, crossing)) = pane.it.box_rect() {
            let a = pane.canvas.world_to_screen(Point::new(r.min.x, r.min.y).to_mm());
            let b = pane.canvas.world_to_screen(Point::new(r.max.x, r.max.y).to_mm());
            let rect = egui::Rect::from_two_pos(a, b);
            let color = if crossing { theme::SUCCESS } else { theme::ACCENT };
            let painter = ui.painter_at(response.rect);
            painter.rect_filled(rect, 0.0, color.gamma_multiply(0.12));
            if crossing {
                let pts = [rect.left_top(), rect.right_top(), rect.right_bottom(), rect.left_bottom(), rect.left_top()];
                painter.extend(egui::Shape::dashed_line(&pts, egui::Stroke::new(1.0, color), 6.0, 4.0));
            } else {
                painter.rect_stroke(rect, 0.0, egui::Stroke::new(1.0, color), egui::StrokeKind::Inside);
            }
        }
        if !pane.it.is_idle() {
            ui.ctx().request_repaint();
        }
        UiResponse::None
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
        egui::Panel::left("library").resizable(true).default_size(230.0).show(ui, |ui| {
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| self.left_panel(ui));
        });
        egui::Panel::right("properties").resizable(true).default_size(280.0).show(ui, |ui| {
            ui.add_space(6.0);
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
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
                active: &mut self.active,
                hovered: &mut self.hovered,
            };
            self.tree.ui(&mut behavior, ui);
        });

        // Agent requests are answered between user interactions.
        if !self.busy() {
            self.agent.process(&ctx, &mut self.doc, &mut self.panes, &self.starter);
        }

        // Model changed (any source): drop stale selections, refresh highlights.
        if self.doc.revision != self.revision {
            self.revision = self.doc.revision;
            for pane in &mut self.panes {
                pane.it.refresh(&mut self.doc.target(pane.kind));
            }
        }
        self.keymap_window(&ctx);
        self.update_title(&ctx);
        self.screenshot_tick(&ctx);
    }
}
