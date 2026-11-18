//! Library editing: a loaded library ([`LibraryEditor`]) with its item tabs
//! ([`ItemTab`], one per open part, symbol or footprint), and [`ItemEditor`], a tab with
//! its library: the symbol and footprint editors (canvas + tools + element inspector)
//! and the part form.

use std::sync::atomic::{AtomicU64, Ordering};

use editor_core::clip::Clip;
use egui::RichText;
use model::*;
use view::array::Slot;
use view::item::{Content, Elem, Ghost, Item, ItemTarget, ItemView, update_item};
use view::text_edit::EditSite;
use view::tools::{Placer, Tool};
use view::{Interaction, KeyAction, LibraryDoc};

use crate::array_form::ArrayForm;
use crate::canvas::{self, Canvas};
use crate::fields::{self, Fields, grid, hint, label};
use crate::icons::Icon;
use crate::keymap::{Action, Keymap};
use crate::part_form::PartForm;
use crate::theme;
use crate::toolbar::{self, ToolButton};
use crate::units::GridKind;

/// What an item tab shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Open {
    Item(Item),
    Part(Id),
}

impl Open {
    pub fn of(id: ObjectId) -> Option<Open> {
        match id {
            ObjectId::LibPart(p) => Some(Open::Part(p)),
            ObjectId::LibSymbol(s) => Some(Open::Item(Item::Symbol(s))),
            ObjectId::LibFootprint(f) => Some(Open::Item(Item::Footprint(f))),
            _ => None,
        }
    }

    pub fn object_id(self) -> ObjectId {
        match self {
            Open::Item(item) => item.object_id(),
            Open::Part(p) => ObjectId::LibPart(p),
        }
    }
}

/// Key of the Project Library's editor: its document is the project's own, so its edits
/// are project commands (one undo history, live on the schematic and board).
pub const PROJECT_LIB: u64 = 1 << 32;

const READ_ONLY: &str = "Read-only: the built-in library ships with the app. Save a copy to change its parts.";

/// A new item tab id, unique in the run.
fn next_tab_id() -> u64 {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

/// A loaded library and its open item tabs.
pub struct LibraryEditor {
    pub key: u64,
    pub lib: LibraryDoc,
    pub tabs: Vec<ItemTab>,
    /// Validation issues and the revision they are of.
    issues: (u64, Vec<(Id, String)>),
    pub message: Option<String>,
    /// "Save a copy…" was clicked (read-only built-in library).
    pub save_copy: bool,
}

/// One item tab: the item and its editor state (camera, selection, tool).
pub struct ItemTab {
    pub id: u64,
    pub open: Open,
    /// A preview tab: replaced by the next item previewed, until it's edited or kept.
    pub preview: bool,
    /// When the tab was last focused (larger is later), for closing the oldest first.
    pub used: u64,
    /// Another item to open in a tab of its own ("Edit footprint" of a part); the app
    /// takes it.
    pub navigate: Option<Open>,
    pub view: ItemView,
    pub canvas: Canvas,
    pub it: Interaction<Elem>,
    pub placer: Placer,
    part_form: PartForm,
    fields: Fields,
    revision: u64,
    /// The pointer went down on the text being edited and selects while it is held.
    text_drag: bool,
    /// The handle under the pointer: of the selected graphic (by index).
    pub grip_hover: Option<(usize, view::handles::Grip)>,
    grip_drag: Option<ShapeDrag>,
    /// Picked in the canvas's context menu; the app runs it.
    pub picked: Option<Action>,
    /// Array mode ("Create array…"): the item and the pad (pin) it copies.
    array: Option<(Item, Elem)>,
    /// The array forms of pads and pins (kept for the session).
    array_pads: ArrayForm,
    array_pins: ArrayForm,
    /// A widget had the keyboard focus last frame (its Enter isn't the array's).
    array_focus: bool,
}

/// An item tab with its library, for editing.
pub struct ItemEditor<'a> {
    pub lib: &'a mut LibraryDoc,
    pub message: &'a mut Option<String>,
    save_copy: &'a mut bool,
    pub tab: &'a mut ItemTab,
}

/// A handle drag of a graphic: as it was, the handle, its offset from the pointer, the
/// undo group, the snap points (taken at the start) and whether it moved yet.
struct ShapeDrag {
    index: usize,
    original: Shape,
    grip: view::handles::Grip,
    offset: Point,
    group: u64,
    points: Vec<Point>,
    moved: bool,
    start: egui::Pos2,
}

/// An array's slots (or why it can't be made), the names in use, the copies to preview and
/// what it copies ("pad 1").
type ArrayPreview = (Result<Vec<Slot>, String>, Vec<String>, Vec<Ghost>, String);

fn graphic(c: &Content, i: usize) -> Option<&Shape> {
    match c {
        Content::Symbol(s) => s.graphics.get(i),
        Content::Footprint(f) => f.graphics.get(i).map(|g| &g.shape),
    }
}

fn set_graphic(c: &mut Content, i: usize, shape: Shape) -> Result<(), String> {
    let slot = match c {
        Content::Symbol(s) => s.graphics.get_mut(i),
        Content::Footprint(f) => f.graphics.get_mut(i).map(|g| &mut g.shape),
    };
    *slot.ok_or("the drawing no longer exists")? = shape;
    Ok(())
}

impl view::project_libs::HasLibrary for LibraryEditor {
    fn library_doc(&self) -> &LibraryDoc {
        &self.lib
    }
}

impl LibraryEditor {
    pub fn new(key: u64, lib: LibraryDoc) -> Self {
        LibraryEditor { key, lib, tabs: Vec::new(), issues: (u64::MAX, Vec::new()), message: None, save_copy: false }
    }

    /// The built-in library: no file, and always loaded.
    pub fn is_builtin(&self) -> bool {
        self.lib.is_builtin()
    }

    /// The Project Library (the project's own document).
    pub fn is_project(&self) -> bool {
        self.key == PROJECT_LIB
    }

    /// The library's name as the user sees it.
    pub fn title(&self) -> String {
        if self.is_project() {
            return "Project Library".into();
        }
        if self.is_builtin() {
            return "Built-in Library".into();
        }
        format!("Library: {}", self.lib.name())
    }

    /// The name of an item of this library.
    pub fn item_name(&self, open: Open) -> Option<&str> {
        let lib = self.lib.library();
        match open {
            Open::Item(Item::Symbol(id)) => lib.symbols.get(id).map(|s| s.name.as_str()),
            Open::Item(Item::Footprint(id)) => lib.footprints.get(id).map(|f| f.name.as_str()),
            Open::Part(id) => lib.parts.get(id).map(|p| p.name.as_str()),
        }
    }

    pub fn exists(&self, open: Open) -> bool {
        self.item_name(open).is_some()
    }

    pub fn tab(&self, id: u64) -> Option<&ItemTab> {
        self.tabs.iter().find(|t| t.id == id)
    }

    pub fn tab_mut(&mut self, id: u64) -> Option<&mut ItemTab> {
        self.tabs.iter_mut().find(|t| t.id == id)
    }

    /// The tab showing `open`, if there is one.
    pub fn tab_of(&self, open: Open) -> Option<&ItemTab> {
        self.tabs.iter().find(|t| t.open == open)
    }

    /// A new tab showing `open`. Returns its id.
    pub fn add_tab(&mut self, open: Open, preview: bool) -> u64 {
        let id = next_tab_id();
        self.tabs.push(ItemTab::new(id, open, preview));
        self.editor(id).expect("just added").set_open(open);
        id
    }

    pub fn remove_tab(&mut self, id: u64) {
        self.tabs.retain(|t| t.id != id);
    }

    /// Tab `id` with this library, for editing.
    pub fn editor(&mut self, id: u64) -> Option<ItemEditor<'_>> {
        let tab = self.tabs.iter_mut().find(|t| t.id == id)?;
        Some(ItemEditor { lib: &mut self.lib, message: &mut self.message, save_copy: &mut self.save_copy, tab })
    }

    /// Validation issues of the library (part, message), recomputed when it changed.
    pub fn issues(&mut self) -> &[(Id, String)] {
        if self.issues.0 != self.lib.doc.revision {
            self.issues = (self.lib.doc.revision, self.lib.library().issues());
        }
        &self.issues.1
    }

    /// The library summary (Overview panel).
    pub fn overview_ui(&mut self, ui: &mut egui::Ui) {
        fields::heading(ui, "Library", "");
        let lib = self.lib.library();
        let (np, ns, nf) = (lib.parts.len(), lib.symbols.len(), lib.footprints.len());
        let issues = lib.validate();
        let name = lib.name.clone();
        grid(ui, "lib_summary", |ui| {
            fields::row(ui, "Name", name);
            fields::row(ui, "Parts", np.to_string());
            fields::row(ui, "Symbols", ns.to_string());
            fields::row(ui, "Footprints", nf.to_string());
        });
        hint(ui, "The library is named after its file (Save as… renames it).");
        issues_ui(ui, &issues);
    }
}

impl ItemTab {
    fn new(id: u64, open: Open, preview: bool) -> Self {
        ItemTab {
            id,
            open,
            preview,
            used: 0,
            navigate: None,
            view: ItemView::new(),
            canvas: Canvas::new(view::style::background(1.27)),
            it: Interaction::new(mm(1.27)),
            placer: Placer::default(),
            part_form: PartForm::default(),
            fields: Fields::default(),
            revision: u64::MAX,
            text_drag: false,
            grip_hover: None,
            grip_drag: None,
            picked: None,
            array: None,
            array_pads: ArrayForm::new(false),
            array_pins: ArrayForm::new(true),
            array_focus: false,
        }
    }

    /// This tab's tool settings to remember, by family (see `view::tool_memory`).
    pub fn remembered(&self) -> [(&'static str, serde_json::Value); 3] {
        use view::tool_memory::remember;
        [
            ("library", remember(&self.placer.settings)),
            ("array_pads", remember(&self.array_pads)),
            ("array_pins", remember(&self.array_pins)),
        ]
    }

    /// [`Self::remembered`] of a new tab.
    pub fn default_remembered() -> [(&'static str, serde_json::Value); 3] {
        use view::tool_memory::remember;
        [
            ("library", remember(&view::tools::Settings::default())),
            ("array_pads", remember(&ArrayForm::new(false))),
            ("array_pins", remember(&ArrayForm::new(true))),
        ]
    }

    /// Take the remembered tool settings that differ from this tab's (another tab
    /// changed them, or they were loaded), keeping the rotation, the next name and text.
    pub fn recall(&mut self, tools: &std::collections::BTreeMap<String, serde_json::Value>) {
        use view::tool_memory::{recall, remember};
        if let Some(v) = tools.get("library")
            && *v != remember(&self.placer.settings)
        {
            let old = &self.placer.settings;
            self.placer.settings = view::tools::Settings {
                rotation: old.rotation,
                name: old.name.clone(),
                text: old.text.clone(),
                ..recall(Some(v))
            };
        }
        for (key, form, pins) in
            [("array_pads", &mut self.array_pads, false), ("array_pins", &mut self.array_pins, true)]
        {
            if let Some(v) = tools.get(key)
                && *v != remember(&*form)
            {
                *form = ArrayForm::recall(Some(v), pins);
            }
        }
    }

    /// Back to the default tool settings (the remembered ones were forgotten).
    pub fn forget_tools(&mut self) {
        let old = &self.placer.settings;
        self.placer.settings = view::tools::Settings {
            rotation: old.rotation,
            name: old.name.clone(),
            text: old.text.clone(),
            ..Default::default()
        };
        self.array_pads = ArrayForm::new(false);
        self.array_pins = ArrayForm::new(true);
    }

    /// An interaction (drag, box, half-drawn shape) is in progress.
    pub fn busy(&self) -> bool {
        !self.it.is_idle() || self.placer.in_progress()
    }

    pub fn in_array(&self) -> bool {
        self.array.is_some()
    }
}

impl ItemEditor<'_> {
    pub fn is_builtin(&self) -> bool {
        self.lib.is_builtin()
    }

    /// Show `open` (a new tab, or the next item in a preview tab).
    fn set_open(&mut self, open: Open) {
        if let Err(e) = self.lib.doc.commit_text_edit() {
            *self.message = Some(e);
        }
        self.tab.open = open;
        self.tab.array = None;
        let item = match open {
            Open::Item(item) => Some(item),
            Open::Part(_) => None,
        };
        self.tab.view.show(item);
        self.tab.view.sync(&self.lib.doc);
        self.tab.it.reset();
        self.tab.placer.set_tool(Tool::Select, None);
        self.tab.view.set_preview(&[]);
        self.tab.fields.forget();
        self.tab.revision = self.lib.doc.revision;
        if let Some(item) = item {
            let g = match item {
                Item::Symbol(_) => 1.27,
                Item::Footprint(_) => 0.25,
            };
            self.tab.it.grid = mm(g);
            self.tab.canvas.background.grid = g as f32;
            self.tab.canvas.request_fit();
        }
    }

    pub fn set_tool(&mut self, tool: Tool) {
        let Open::Item(item) = self.tab.open else { return };
        if let Err(e) = self.lib.doc.commit_text_edit() {
            *self.message = Some(e);
        }
        if !Tool::for_item(item).contains(&tool) {
            // Pin and pad share a shortcut.
            let alt = match tool {
                Tool::Pin => Tool::Pad,
                Tool::Pad => Tool::Pin,
                t => t,
            };
            if !Tool::for_item(item).contains(&alt) {
                return;
            }
            return self.set_tool(alt);
        }
        self.cancel_array();
        let mut t = ItemTarget { doc: &mut self.lib.doc, view: &mut self.tab.view };
        self.tab.it.set_selection(&mut t, Vec::new());
        self.tab.placer.set_tool(tool, self.tab.view.content());
        if tool == Tool::Select {
            self.tab.view.set_preview(&[]);
        }
    }

    // ------------------------------------------------------------------------------------
    // Commands

    /// Keep views and selections consistent with the document (after any change).
    pub fn refresh(&mut self) {
        if self.lib.doc.revision == self.tab.revision {
            return;
        }
        self.tab.revision = self.lib.doc.revision;
        // A deleted item's tab is closed by the app.
        if !self.exists() {
            return;
        }
        self.tab.view.sync(&self.lib.doc);
        self.tab.it.refresh(&mut ItemTarget { doc: &mut self.lib.doc, view: &mut self.tab.view });
    }

    pub fn key(&mut self, action: KeyAction) {
        if !matches!(self.tab.open, Open::Item(_)) {
            return;
        }
        if self.tab.array.is_some() && action == KeyAction::Escape {
            return self.cancel_array();
        }
        if self.tab.placer.is_placing() {
            match action {
                KeyAction::Rotate => self.tab.placer.rotate(),
                KeyAction::Escape if self.tab.placer.escape() => self.tab.view.set_preview(&[]),
                _ => {}
            }
            return;
        }
        let mut t = ItemTarget { doc: &mut self.lib.doc, view: &mut self.tab.view };
        self.tab.it.key(&mut t, action);
    }

    /// A clip of the selected elements (removed after copying if `cut`).
    pub fn copy(&mut self, cut: bool) -> Result<Clip, String> {
        let Open::Item(item) = self.tab.open else { return Err("Open a symbol or footprint to copy from it".into()) };
        if cut && let Some(why) = &self.lib.doc.read_only {
            return Err(why.clone());
        }
        let content = Content::get(self.lib.doc.project(), item).ok_or("the item no longer exists")?;
        let anchor = self.tab.it.selection.iter().find_map(|&e| content.anchor(e)).map(|p| self.tab.it.snap(p));
        let clip = anchor
            .and_then(|a| view::item::copy_elems(&content, &self.tab.it.selection, a))
            .ok_or("Nothing selected")?;
        if cut {
            self.key(KeyAction::Delete);
        }
        Ok(clip)
    }

    /// Paste `clip` under the pointer (else where it was copied from); it follows the
    /// pointer until a click.
    pub fn paste(&mut self, clip: &Clip) -> Result<(), String> {
        let Open::Item(item) = self.tab.open else { return Err("Open a symbol or footprint to paste into it".into()) };
        if let Some(why) = &self.lib.doc.read_only {
            return Err(why.clone());
        }
        if self.tab.busy() {
            return Ok(());
        }
        self.tab.placer.set_tool(Tool::Select, None);
        self.tab.view.set_preview(&[]);
        let pointer = self.tab.canvas.cursor_world.map(|[x, y]| Point::mm(x, y));
        let at = pointer.map_or(clip.anchor, |p| self.tab.it.snap(p));
        let group = view::interaction::new_group();
        let added = update_item(&mut self.lib.doc, item, Some(group), |c| view::item::paste_elems(c, clip, at))?;
        self.tab.view.sync(&self.lib.doc);
        let mut t = ItemTarget { doc: &mut self.lib.doc, view: &mut self.tab.view };
        self.tab.it.carry(&mut t, added, at, pointer.unwrap_or(at), group);
        Ok(())
    }

    /// Undo the library's last step. Returns the message and the objects it changed.
    pub fn undo(&mut self) -> Option<(String, Vec<ObjectId>)> {
        if self.tab.busy() {
            return None;
        }
        let label = self.lib.doc.editor().history().undo_label().map(str::to_string);
        let changed = self.lib.doc.undo().map_or(Vec::new(), |c| c.ids().collect());
        self.refresh();
        Some((label.map_or("Nothing to undo".into(), |l| format!("Undo: {l}")), changed))
    }

    /// Redo the library's next step. Returns the message and the objects it changed.
    pub fn redo(&mut self) -> Option<(String, Vec<ObjectId>)> {
        if self.tab.busy() {
            return None;
        }
        let label = self.lib.doc.editor().history().redo_label().map(str::to_string);
        let changed = self.lib.doc.redo().map_or(Vec::new(), |c| c.ids().collect());
        self.refresh();
        Some((label.map_or("Nothing to redo".into(), |l| format!("Redo: {l}")), changed))
    }

    /// The tab's item still exists.
    pub fn exists(&self) -> bool {
        let lib = self.lib.library();
        match self.tab.open {
            Open::Item(Item::Symbol(id)) => lib.symbols.contains(id),
            Open::Item(Item::Footprint(id)) => lib.footprints.contains(id),
            Open::Part(id) => lib.parts.contains(id),
        }
    }

    fn report<T>(&mut self, r: Result<T, String>) -> Option<T> {
        r.map_err(|e| *self.message = Some(e)).ok()
    }

    /// A new part using the open symbol or footprint, opened in a tab of its own.
    fn create_part(&mut self, item: Item) {
        let (s, f) = match item {
            Item::Symbol(s) => (Some(s), None),
            Item::Footprint(f) => (None, Some(f)),
        };
        let lib = self.lib.library();
        let name = match item {
            Item::Symbol(s) => lib.symbols.get(s).map(|s| s.name.clone()),
            Item::Footprint(f) => lib.footprints.get(f).map(|f| f.name.clone()),
        };
        let r = self.lib.new_part(&name.unwrap_or("New part".into()), s, f);
        if let Some(p) = self.report(r) {
            self.tab.navigate = Some(Open::Part(p));
        }
        self.refresh();
    }

    // ------------------------------------------------------------------------------------
    // Center (tab content)

    /// Tab content. Returns true if the pointer was pressed inside (focus).
    pub fn center_ui(&mut self, ui: &mut egui::Ui, keymap: &Keymap) -> bool {
        self.refresh();
        let pressed = ui.rect_contains_pointer(ui.max_rect()) && ui.input(|i| i.pointer.any_pressed());
        match self.tab.open {
            Open::Part(id) => {
                let frame = egui::Frame::NONE.fill(theme::panel()).inner_margin(egui::Margin::same(16));
                frame.show(ui, |ui| {
                    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                        if self.lib.doc.read_only.is_some() {
                            ui.horizontal(|ui| {
                                ui.label(RichText::new(READ_ONLY).color(theme::warn()));
                                if ui.button("Save a copy…").clicked() {
                                    *self.save_copy = true;
                                }
                            });
                            ui.add_space(8.0);
                        }
                        let mut message = None;
                        if let Some(open) = self.tab.part_form.ui(ui, self.lib, id, &mut message) {
                            self.tab.navigate = Some(open);
                        }
                        if message.is_some() {
                            *self.message = message;
                        }
                    });
                });
            }
            Open::Item(item) => self.item_ui(ui, item, keymap),
        }
        pressed
    }

    /// Floating tool strip, grid header and usage hint over the item canvas.
    fn item_toolbar(&mut self, ui: &mut egui::Ui, rect: egui::Rect, item: Item, keymap: &Keymap) {
        let b =
            |tool, icon, about, action| ToolButton { tool, icon, name: Tool::label(tool), about, action: Some(action) };
        let select = [b(
            Tool::Select,
            Icon::Select,
            "Select, move and edit elements. Drag a box to select several.",
            Action::ToolSelect,
        )];
        let connect = match item {
            Item::Symbol(_) => {
                [b(Tool::Pin, Icon::Pin, "Place pins: where wires connect to the symbol.", Action::ToolPin)]
            }
            Item::Footprint(_) => {
                [b(Tool::Pad, Icon::Pad, "Place pads: the copper the part is soldered to.", Action::ToolPin)]
            }
        };
        let draw = [
            b(Tool::Line, Icon::Line, "Draw lines and polylines.", Action::ToolLine),
            b(Tool::Rect, Icon::Rect, "Draw rectangles.", Action::ToolRect),
            b(Tool::Circle, Icon::Circle, "Draw circles.", Action::ToolCircle),
            b(Tool::Arc, Icon::Arc, "Draw arcs: centre, start, end (/ the other way round).", Action::ToolArc),
            b(Tool::Polygon, Icon::Polygon, "Draw polygons, filled or not.", Action::ToolPolygon),
            b(Tool::Text, Icon::Text, "Place text.", Action::ToolText),
        ];
        let measure: &[ToolButton<Tool>] = match item {
            Item::Symbol(_) => &[],
            Item::Footprint(_) => {
                &[b(Tool::Measure, Icon::Measure, "Measure distances between two points.", Action::ToolMeasure)]
            }
        };
        let id = egui::Id::unique(("item_toolbar", self.tab.id));
        let groups: Vec<&[ToolButton<Tool>]> =
            [&select[..], &connect[..], &draw[..], measure].into_iter().filter(|g| !g.is_empty()).collect();
        let (chosen, _) = toolbar::strip(ui, rect, id, &groups, self.tab.placer.tool, keymap);
        if let Some(tool) = chosen {
            self.set_tool(tool);
        }
        let grid_kind = match item {
            Item::Symbol(_) => GridKind::Schematic,
            Item::Footprint(_) => GridKind::Board,
        };
        let entries = self.tab.view.layer_menu();
        let (scene, it, canvas) = (&mut self.tab.view.scene, &mut self.tab.it, &mut self.tab.canvas);
        let fit = toolbar::view_buttons(
            ui,
            rect,
            id.with("view_buttons"),
            keymap,
            |ui| toolbar::layer_menu(ui, scene, &entries),
            |ui| toolbar::grid_menu(ui, &mut it.grid, &mut it.snap, &mut canvas.background, grid_kind),
        );
        if fit {
            self.tab.canvas.request_fit();
        }
        let tip = match self.tab.placer.tool {
            _ if self.lib.doc.read_only.is_some() => READ_ONLY,
            _ if self.tab.array.is_some() => "Array: set it up in Tool settings · Enter creates · Esc cancels",
            Tool::Select => "Click/box to select · drag to move · R rotate · X mirror · Del delete",
            Tool::Line => "Click points · click the last point again or Esc to finish",
            Tool::Rect => "Click two corners",
            Tool::Circle => "Click center, then a point on the circle",
            Tool::Arc => {
                "Click the centre, the start, then the end (the arc follows the pointer) · / other way · Esc cancel"
            }
            Tool::Polygon => "Click the corners · click the first or last corner again to close · Esc cancel",
            Tool::Measure => view::measure::HINT,
            _ => "Click to place · R rotate · Esc done",
        };
        toolbar::hint(ui, rect, tip);
    }

    fn item_ui(&mut self, ui: &mut egui::Ui, item: Item, keymap: &Keymap) {
        // The canvas fills the rest of the tab; the tool bars float over it.
        let rect = ui.available_rect_before_wrap();
        let scene = &mut self.tab.view.scene;
        let canvas = &mut self.tab.canvas;
        let response = ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| canvas.ui(ui, scene)).inner;
        self.item_toolbar(ui, rect, item, keymap);
        let on_edit = self.text_edit_pointer(ui, &response, item);
        if self.tab.placer.tool == Tool::Measure {
            let fp = match item {
                Item::Footprint(f) => self.lib.doc.project().library.footprints.get(f),
                Item::Symbol(_) => None,
            };
            let tol = (8.0 / self.tab.canvas.camera.zoom * 1e6) as model::Coord;
            let free = ui.input(|i| i.modifiers.alt);
            let at = response.hover_pos().map(|p| {
                let [x, y] = self.tab.canvas.screen_to_world(p);
                let raw = Point::mm(x, y);
                let inside = fp.and_then(|fp| view::measure::footprint_pad_centre(fp, raw));
                let points = fp.into_iter().flat_map(view::measure::footprint_points);
                view::measure::snap(raw, self.tab.it.snap(raw), points, inside, tol, free)
            });
            self.tab.placer.measure.hover(at.map(|a| a.0), at.is_some_and(|a| a.1));
            if let Some((p, _)) = at
                && response.hovered()
                && ui.input(|i| i.pointer.primary_pressed())
            {
                self.tab.placer.measure.click(p);
            }
            canvas::draw_measure(ui, &self.tab.canvas, response.rect, &self.tab.placer.measure);
            if response.hovered() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
            }
        } else if self.tab.placer.is_placing() {
            // Drawing tools snap to pins, pads and vertices too (Alt: the grid only).
            let draws =
                matches!(self.tab.placer.tool, Tool::Line | Tool::Rect | Tool::Circle | Tool::Arc | Tool::Polygon);
            let free = ui.input(|i| i.modifiers.alt);
            let points = if draws && !free { self.tab.view.content().map(|c| c.snap_points()) } else { None };
            let tol = (8.0 / self.tab.canvas.camera.zoom * 1e6) as model::Coord;
            let world = response.hover_pos().map(|p| {
                let [x, y] = self.tab.canvas.screen_to_world(p);
                let raw = Point::mm(x, y);
                let grid = self.tab.it.snap(raw);
                match &points {
                    Some(pts) => view::measure::snap(raw, grid, pts.iter().copied(), None, tol, false).0,
                    None => grid,
                }
            });
            self.tab.placer.hover(world);
            if let Some(p) = world
                && response.hovered()
                && !on_edit
                && ui.input(|i| i.pointer.primary_pressed())
            {
                let r = self.tab.placer.click(&mut self.lib.doc, &mut self.tab.view, p);
                self.report(r);
            }
            self.tab.view.set_preview(&self.tab.placer.ghosts(Some(item)));
            if response.hovered() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
            }
        } else {
            let mut on_grip = false;
            if !on_edit && self.lib.doc.read_only.is_none() {
                let (consumed, err) = self.graphic_handles(ui, &response, item);
                on_grip = consumed;
                if err.is_some() {
                    *self.message = err;
                }
            }
            let engaged = !self.tab.it.is_idle();
            let carried_off = self.tab.it.is_carrying() && !response.hovered();
            if let Some(input) =
                canvas::pointer_input(ui, &response, &self.tab.canvas, engaged, self.tab.it.hover.is_some())
                && !carried_off
                && !on_edit
                && !on_grip
            {
                let mut t = ItemTarget { doc: &mut self.lib.doc, view: &mut self.tab.view };
                self.tab.it.pointer(&mut t, &input);
            }
            self.context_menu(&response, keymap);
            // Double-clicking a text edits it in place.
            if response.double_clicked()
                && !on_grip
                && !on_edit
                && !self.is_builtin()
                && let Some(Elem::Text(i)) = self.tab.it.hover
                && self.lib.doc.begin_item_text_edit(item, i)
            {
                let mut t = ItemTarget { doc: &mut self.lib.doc, view: &mut self.tab.view };
                self.tab.it.set_selection(&mut t, Vec::new());
            }
            if self.tab.it.is_dragging() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
            } else if self.tab.it.hover.is_some() && response.hovered() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            }
            if let Some((r, crossing)) = self.tab.it.box_rect() {
                canvas::draw_box(ui, &self.tab.canvas, response.rect, r, crossing);
            }
            if let Some(m) = self.tab.it.message.take() {
                *self.message = Some(m);
            }
        }
        if on_edit && response.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Text);
        }
        self.tab.view.set_text_edit(self.lib.doc.item_text_edit(item));
        self.array_canvas(ui.ctx(), item);
        if !self.tab.it.is_idle() {
            ui.ctx().request_repaint();
        }
    }

    /// "Create array…" on the one selected pad or pin: array mode, its form in the Tool
    /// settings panel. Returns whether it started (a message otherwise).
    pub fn start_array(&mut self) -> bool {
        let Open::Item(item) = self.tab.open else { return false };
        if self.lib.doc.read_only.is_some() {
            *self.message = Some("This library is read-only".into());
            return false;
        }
        if let Err(e) = self.lib.doc.commit_text_edit() {
            *self.message = Some(e);
        }
        match (item, self.tab.it.selection.as_slice()) {
            (Item::Footprint(_), &[e @ Elem::Pad(_)]) | (Item::Symbol(_), &[e @ Elem::Pin(_)]) => {
                self.tab.placer.set_tool(Tool::Select, None);
                self.tab.array = Some((item, e));
                true
            }
            _ => {
                *self.message = Some("Select one pad or pin to make an array of".into());
                false
            }
        }
    }

    pub fn cancel_array(&mut self) {
        if self.tab.array.take().is_some() {
            self.tab.view.set_preview(&[]);
        }
    }

    /// The array's slots, the names in use, the copies to preview and what it copies
    /// ("pad 1"). `None` when the copied element is gone.
    fn array_preview(&self) -> Option<ArrayPreview> {
        let (_, base) = self.tab.array?;
        let content = self.tab.view.content()?;
        let (slots, ghosts, what) = match (content, base) {
            (Content::Footprint(f), Elem::Pad(id)) => {
                let p = f.pads.get(id)?;
                let s = view::array::slots(&self.tab.array_pads.layout(), p.pos, &p.name);
                let g = s
                    .as_ref()
                    .map_or(Vec::new(), |s| view::array::pads(p, s).into_iter().skip(1).map(Ghost::Pad).collect());
                (s, g, format!("pad {}", p.name))
            }
            (Content::Symbol(sym), Elem::Pin(id)) => {
                let p = sym.pins.get(id)?;
                let s = view::array::slots(&self.tab.array_pins.layout(), p.pos, &p.name);
                let g = s
                    .as_ref()
                    .map_or(Vec::new(), |s| view::array::pins(p, s).into_iter().skip(1).map(Ghost::Pin).collect());
                (s, g, format!("pin {}", p.name))
            }
            _ => return None,
        };
        let clashes = slots.as_ref().map_or(Vec::new(), |s| view::array::clashes(content, base, s));
        Some((slots, clashes, ghosts, what))
    }

    /// Array mode on the canvas: the preview, Enter creates; it ends when the selection
    /// changes or the element goes.
    fn array_canvas(&mut self, ctx: &egui::Context, item: Item) {
        let Some((on, base)) = self.tab.array else { return };
        let preview = (on == item && self.tab.it.selection == [base]).then(|| self.array_preview()).flatten();
        let Some((slots, _, ghosts, _)) = preview else { return self.cancel_array() };
        self.tab.view.set_preview(&ghosts);
        // Enter in a field commits the field; only a free Enter creates.
        let focused = ctx.memory(|m| m.focused().is_some());
        let enter = ctx.input(|i| i.key_pressed(egui::Key::Enter));
        if enter && !focused && !self.tab.array_focus && slots.is_ok() {
            self.create_array();
        }
        self.tab.array_focus = focused;
    }

    fn create_array(&mut self) {
        let Some((item, base)) = self.tab.array.take() else { return };
        self.tab.view.set_preview(&[]);
        let layout =
            if matches!(base, Elem::Pin(_)) { self.tab.array_pins.layout() } else { self.tab.array_pads.layout() };
        match view::array::apply(&mut self.lib.doc, item, base, &layout) {
            Ok(elems) => {
                self.tab.view.sync(&self.lib.doc);
                let mut t = ItemTarget { doc: &mut self.lib.doc, view: &mut self.tab.view };
                self.tab.it.set_selection(&mut t, elems);
            }
            Err(e) => *self.message = Some(e),
        }
    }

    /// The array form (Tool settings panel).
    fn array_panel(&mut self, ui: &mut egui::Ui) {
        let Some((base_elem, (slots, clashes, _, what))) = self.tab.array.map(|a| a.1).zip(self.array_preview()) else {
            return;
        };
        let form = if matches!(base_elem, Elem::Pin(_)) { &mut self.tab.array_pins } else { &mut self.tab.array_pads };
        match form.ui(ui, &what, &slots, &clashes) {
            Some(true) => self.create_array(),
            Some(false) => self.cancel_array(),
            None => {}
        }
    }

    /// The right-click menu of the canvas: a right click selects what is under the
    /// pointer first. The pick is run by the app ([`LibraryEditor::picked`]).
    fn context_menu(&mut self, response: &egui::Response, keymap: &Keymap) {
        if response.secondary_clicked() && self.tab.it.is_idle() {
            let sel = match self.tab.it.hover {
                Some(h) if !self.tab.it.selection.contains(&h) => Some(vec![h]),
                None => Some(Vec::new()),
                _ => None,
            };
            if let Some(sel) = sel {
                let mut t = ItemTarget { doc: &mut self.lib.doc, view: &mut self.tab.view };
                self.tab.it.set_selection(&mut t, sel);
            }
        }
        let content = self.tab.view.content();
        let selected = !self.tab.it.selection.is_empty();
        let rect = self.tab.it.selection.iter().any(|e| {
            matches!(e, Elem::Graphic(i) if content.and_then(|c| graphic(c, *i)).is_some_and(|g| matches!(g, Shape::Rect { .. })))
        });
        let array = matches!(self.tab.it.selection.as_slice(), [Elem::Pad(_) | Elem::Pin(_)]);
        let entries = crate::menu::canvas_menu(keymap, crate::menu::CanvasKind::Part, selected, false, rect, array);
        response.context_menu(|ui| {
            if let Some(crate::menu::Cmd::Action(a)) = crate::menu::entries_menu_ui(ui, &entries) {
                self.tab.picked = Some(a);
            }
        });
    }

    /// Handles of the one selected graphic: drawn and dragged (one undo step per drag;
    /// Alt: no snapping, Shift: 0°/45°/90°). A double click on a polygon corner deletes
    /// it. Returns whether the pointer input went to a handle, and an error.
    fn graphic_handles(&mut self, ui: &egui::Ui, response: &egui::Response, item: Item) -> (bool, Option<String>) {
        use view::handles;
        let tol = (6.0 / self.tab.canvas.camera.zoom * 1e6) as Coord;
        let (pos, pressed, down, alt, shift) = ui.input(|i| {
            (
                i.pointer.latest_pos(),
                i.pointer.primary_pressed(),
                i.pointer.primary_down(),
                i.modifiers.alt,
                i.modifiers.shift,
            )
        });
        let canvas = &self.tab.canvas;
        let world = |p: egui::Pos2| {
            let [x, y] = canvas.screen_to_world(p);
            Point::mm(x, y)
        };
        if let Some(d) = &mut self.tab.grip_drag {
            if !down {
                if d.moved {
                    self.lib.doc.close_group();
                }
                self.tab.grip_drag = None;
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
            let mut p = if alt {
                raw
            } else {
                view::measure::snap(raw, self.tab.it.snap(raw), d.points.iter().copied(), None, tol, false).0
            };
            if shift {
                p = handles::constrain(p, &handles::shape_neighbours(&d.original, d.grip));
            }
            let Some(shape) = handles::shape_moved(&d.original, d.grip, p) else { return (true, None) };
            let index = d.index;
            let r = view::item::update_item(&mut self.lib.doc, item, Some(d.group), |c| set_graphic(c, index, shape));
            self.tab.view.sync(&self.lib.doc);
            return (true, r.err());
        }
        let target = match self.tab.it.selection.as_slice() {
            [Elem::Graphic(i)] if self.tab.it.is_idle() => {
                self.tab.view.content().and_then(|c| graphic(c, *i)).cloned().map(|g| (*i, g))
            }
            _ => None,
        };
        let Some((index, shape)) = target else {
            self.tab.grip_hover = None;
            return (false, None);
        };
        let hs = handles::shape_handles(&shape);
        let at = pos.filter(|_| response.hovered()).map(world);
        let hit = at.and_then(|p| handles::hit(&hs, p, tol));
        self.tab.grip_hover = hit.map(|h| (index, h.grip));
        canvas::draw_handles(ui, &self.tab.canvas, response.rect, &hs, hit.map(|h| h.grip));
        let Some(h) = hit else { return (false, None) };
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
        if response.double_clicked() {
            if !h.is_corner() {
                return (true, None);
            }
            self.tab.grip_hover = None;
            return (true, self.delete_corner(item, index, h.grip).err());
        }
        if pressed && let (Some(p), Some(start)) = (at, pos) {
            let points = self.tab.view.content().map(|c| c.snap_points()).unwrap_or_default();
            let group = view::interaction::new_group();
            self.tab.grip_drag = Some(ShapeDrag {
                index,
                original: shape,
                grip: h.grip,
                offset: h.pos - p,
                group,
                points,
                moved: false,
                start,
            });
            return (true, None);
        }
        (false, None)
    }

    fn delete_corner(&mut self, item: Item, index: usize, grip: view::handles::Grip) -> Result<(), String> {
        let shape = self.tab.view.content().and_then(|c| graphic(c, index)).cloned().ok_or("no such drawing")?;
        let shape = view::handles::shape_without_corner(&shape, grip)?;
        view::item::update_item(&mut self.lib.doc, item, None, |c| set_graphic(c, index, shape))?;
        self.tab.view.sync(&self.lib.doc);
        Ok(())
    }

    /// Delete: the polygon corner under the pointer, if any. `None`: no corner there.
    pub fn delete_hovered_corner(&mut self) -> Option<Result<(), String>> {
        let Open::Item(item) = self.tab.open else { return None };
        let (index, grip) = self.tab.grip_hover.filter(|(_, g)| matches!(g, view::handles::Grip::Corner { .. }))?;
        self.tab.grip_hover = None;
        Some(self.delete_corner(item, index, grip))
    }

    /// The selected rectangles as polygons.
    pub fn convert_to_polygon(&mut self) -> Result<(), String> {
        let Open::Item(item) = self.tab.open else { return Err("open a symbol or footprint".into()) };
        let rects: Vec<usize> = self
            .tab
            .it
            .selection
            .iter()
            .filter_map(|e| if let Elem::Graphic(i) = e { Some(*i) } else { None })
            .collect();
        let r = view::item::update_item(&mut self.lib.doc, item, None, |c| {
            let mut n = 0;
            for &i in &rects {
                if let Some(p) = graphic(c, i).and_then(view::handles::rect_shape_to_polygon) {
                    set_graphic(c, i, p)?;
                    n += 1;
                }
            }
            if n == 0 { Err("select a rectangle to convert".to_string()) } else { Ok(()) }
        });
        self.tab.view.sync(&self.lib.doc);
        r
    }

    /// Pointer on the text being edited in place: a press on it places the caret (a drag
    /// selects, a double-click selects a word); a press elsewhere commits it. Returns true
    /// if the pointer is on the edit, so the tools leave it alone.
    fn text_edit_pointer(&mut self, ui: &egui::Ui, response: &egui::Response, item: Item) -> bool {
        let (pressed, down, shift) =
            ui.input(|i| (i.pointer.primary_pressed(), i.pointer.primary_down(), i.modifiers.shift));
        let mut on_edit = false;
        let doc = &mut self.lib.doc;
        let editing = doc.text_edit().is_some_and(|(site, _)| site == EditSite::Item(item));
        if editing && let Some(pos) = response.hover_pos() {
            let [x, y] = self.tab.canvas.screen_to_world(pos);
            let p = Point::mm(x, y);
            let inside = doc.text_edit().is_some_and(|(_, e)| e.contains(p));
            if pressed && response.hovered() {
                if inside {
                    doc.with_text_edit(|e| e.click(p, shift));
                    self.tab.text_drag = true;
                } else if let Err(e) = doc.commit_text_edit() {
                    *self.message = Some(e);
                }
            } else if self.tab.text_drag && down {
                doc.with_text_edit(|e| e.drag(p));
            }
            if response.double_clicked() && inside {
                doc.with_text_edit(|e| e.select_word(p));
            }
            on_edit = inside || self.tab.text_drag;
        }
        if !down {
            self.tab.text_drag = false;
        }
        on_edit
    }

    // ------------------------------------------------------------------------------------
    // Inspector (right panel)

    /// Settings of the active library tool (Tool settings panel).
    pub fn tool_settings_ui(&mut self, ui: &mut egui::Ui) {
        self.tab.fields.begin_frame();
        match self.tab.open {
            Open::Item(_) if self.tab.array.is_some() => self.array_panel(ui),
            Open::Item(item) if self.tab.placer.is_placing() => self.tool_settings(ui, item),
            _ => hint(ui, "The select tool has no settings. Pick a drawing tool from the tool bar."),
        }
        if self.tab.fields.end_frame() {
            self.lib.doc.close_group();
        }
    }

    /// The selected element, the open item or part (Properties panel).
    pub fn properties_ui(&mut self, ui: &mut egui::Ui) {
        self.tab.fields.begin_frame();
        match self.tab.open {
            Open::Item(item) => match self.tab.it.selection.as_slice() {
                [] => self.item_summary(ui, item),
                [e] => self.element(ui, item, *e),
                many if many.iter().all(|e| matches!(e, Elem::Pad(_))) => {
                    let many = many.to_vec();
                    self.pads(ui, item, &many);
                }
                many => {
                    fields::heading(ui, &format!("{} elements", many.len()), "");
                    hint(ui, "R rotate · X mirror · Del delete · drag to move");
                }
            },
            Open::Part(_) => {
                fields::heading(ui, "Part", "");
                hint(ui, "Edit units, footprint variants and the pin-pad mapping in the form.");
            }
        }
        if self.tab.fields.end_frame() {
            self.lib.doc.close_group();
        }
    }

    /// Several pads: their mask and paste openings, edited together.
    fn pads(&mut self, ui: &mut egui::Ui, item: Item, elems: &[Elem]) {
        let Some(mut content) = self.tab.view.content().cloned() else { return };
        let Content::Footprint(fp) = &mut content else { return };
        let rules = self.lib.doc.project().board.rules.clone();
        fields::heading(ui, &format!("{} pads", elems.len()), "");
        let mut pads: Vec<Pad> =
            elems.iter().filter_map(|e| if let Elem::Pad(id) = e { fp.pads.get(*id).cloned() } else { None }).collect();
        let mut changed = None;
        grid(ui, "pads", |ui| changed = opening_rows(ui, &mut self.tab.fields, &mut pads, &rules));
        hint(ui, "R rotate · X mirror · Del delete · drag to move");
        if let Some(grouped) = changed {
            for p in pads {
                if let Some(slot) = fp.pads.get_mut(p.id) {
                    *slot = p;
                }
            }
            self.commit(item, content, grouped);
        }
    }

    fn commit(&mut self, item: Item, content: Content, grouped: bool) {
        let group = grouped.then(|| self.tab.fields.group());
        let r = view::item::update_item(&mut self.lib.doc, item, group, |c| {
            *c = content;
            Ok(())
        });
        self.report(r);
        self.refresh();
    }

    fn item_summary(&mut self, ui: &mut egui::Ui, item: Item) {
        let Some(content) = self.tab.view.content().cloned() else { return };
        let lib = self.lib.library();
        let users: Vec<String> = lib
            .parts
            .iter()
            .filter(|p| match item {
                Item::Symbol(s) => p.units.iter().any(|u| u.symbol == s),
                Item::Footprint(f) => p.variants.iter().any(|v| v.footprint == Some(f)),
            })
            .map(|p| p.name.clone())
            .collect();
        match content {
            Content::Symbol(mut s) => {
                fields::heading(ui, "Symbol", "");
                grid(ui, "sym_summary", |ui| {
                    label(ui, "Name");
                    if let Some(n) = self.tab.fields.text(ui, ("sym_name", s.id), &s.name) {
                        s.name = n;
                        self.commit(item, Content::Symbol(s.clone()), false);
                    }
                    ui.end_row();
                    fields::row(ui, "Pins", s.pins.len().to_string());
                    fields::row(ui, "Graphics", s.graphics.len().to_string());
                });
            }
            Content::Footprint(mut f) => {
                fields::heading(ui, "Footprint", "");
                grid(ui, "fp_summary", |ui| {
                    label(ui, "Name");
                    if let Some(n) = self.tab.fields.text(ui, ("fp_name", f.id), &f.name) {
                        f.name = n;
                        self.commit(item, Content::Footprint(f.clone()), false);
                    }
                    ui.end_row();
                    label(ui, "Description");
                    if let Some(d) = self.tab.fields.text(ui, ("fp_desc", f.id), &f.description) {
                        f.description = d;
                        self.commit(item, Content::Footprint(f.clone()), false);
                    }
                    ui.end_row();
                    fields::row(ui, "Pads", f.pads.len().to_string());
                    fields::row(ui, "Graphics", f.graphics.len().to_string());
                });
            }
        }
        ui.add_space(8.0);
        if users.is_empty() {
            hint(ui, "Not used by any part yet.");
            if ui.button("Create a part with it").clicked() {
                self.create_part(item);
            }
        } else {
            label(ui, "Used by");
            for u in users {
                ui.label(u);
            }
        }
        ui.add_space(8.0);
        hint(ui, "Texts ${REF} and ${VALUE} show the component's reference and value.");
    }

    fn tool_settings(&mut self, ui: &mut egui::Ui, item: Item) {
        let tool = self.tab.placer.tool;
        fields::heading(ui, tool.label(), "new elements");
        let s = &mut self.tab.placer.settings;
        let f = &mut self.tab.fields;
        grid(ui, "tool_settings", |ui| match tool {
            Tool::Pin => {
                label(ui, "Name");
                if let Some(n) = f.text(ui, "tool_name", &s.name) {
                    s.name = n;
                }
                ui.end_row();
                label(ui, "Type");
                if let Some(k) = pin_kind_combo(ui, "tool_pin_kind", s.pin_kind) {
                    s.pin_kind = k;
                }
                ui.end_row();
                label(ui, "Length");
                if let Some(v) = f.length(ui, s.pin_length, 0.0) {
                    s.pin_length = v;
                }
                ui.end_row();
                label(ui, "Body side");
                if let Some(a) = fields::rotation_combo(ui, "tool_rot", s.rotation, PIN_SIDES) {
                    s.rotation = a;
                }
                ui.end_row();
            }
            Tool::Pad => {
                label(ui, "Name");
                if let Some(n) = f.text(ui, "tool_name", &s.name) {
                    s.name = n;
                }
                ui.end_row();
                pad_fields(ui, f, &mut s.pad_shape, &mut s.pad_kind, &mut s.pad_size);
                label(ui, "Rotation");
                if let Some(a) = fields::rotation_combo(ui, "tool_rot", s.rotation, DEGREES) {
                    s.rotation = a;
                }
                ui.end_row();
            }
            Tool::Line | Tool::Rect | Tool::Circle | Tool::Arc | Tool::Polygon => {
                if let Item::Footprint(_) = item {
                    label(ui, "Layer");
                    if let Some(l) = layer_combo(ui, "tool_layer", s.layer) {
                        s.layer = l;
                    }
                    ui.end_row();
                }
                label(ui, "Width");
                let w = match item {
                    Item::Symbol(_) => &mut s.symbol_width,
                    Item::Footprint(_) => &mut s.footprint_width,
                };
                if let Some(v) = f.length(ui, *w, 0.0) {
                    *w = v;
                }
                ui.end_row();
                if !matches!(tool, Tool::Line | Tool::Arc) {
                    label(ui, "Filled");
                    ui.checkbox(&mut s.fill, "");
                    ui.end_row();
                }
            }
            Tool::Text => {
                label(ui, "Text");
                if let Some(t) = f.text(ui, "tool_text", &s.text) {
                    s.text = t;
                }
                ui.end_row();
                label(ui, "Size");
                if let Some(v) = f.length(ui, s.text_size, 0.1) {
                    s.text_size = v;
                }
                ui.end_row();
                if let Item::Footprint(_) = item {
                    label(ui, "Layer");
                    if let Some(l) = layer_combo(ui, "tool_layer", s.layer) {
                        s.layer = l;
                    }
                    ui.end_row();
                }
            }
            Tool::Select | Tool::Measure => {}
        });
        ui.add_space(6.0);
        hint(ui, "R rotates · Esc returns to Select");
    }

    fn element(&mut self, ui: &mut egui::Ui, item: Item, e: Elem) {
        let Some(mut content) = self.tab.view.content().cloned() else { return };
        let rules = self.lib.doc.project().board.rules.clone();
        let mut changed = None; // Some(grouped)
        let f = &mut self.tab.fields;
        match (&mut content, e) {
            (Content::Symbol(s), Elem::Pin(id)) => {
                let Some(pin) = s.pins.get_mut(id) else { return };
                fields::heading(ui, "Pin", &pin.name);
                grid(ui, "pin", |ui| {
                    label(ui, "Name");
                    if let Some(n) = f.text(ui, ("pin_name", id), &pin.name) {
                        pin.name = n;
                        changed = Some(false);
                    }
                    ui.end_row();
                    label(ui, "Number");
                    if let Some(n) = f.text(ui, ("pin_number", id), &pin.number) {
                        pin.number = n.trim().to_string();
                        changed = Some(false);
                    }
                    ui.end_row();
                    label(ui, "Type");
                    if let Some(k) = pin_kind_combo(ui, "pin_kind", pin.kind) {
                        pin.kind = k;
                        changed = Some(false);
                    }
                    ui.end_row();
                    label(ui, "Position");
                    if let Some(p) = f.point(ui, pin.pos) {
                        pin.pos = p;
                        changed = Some(true);
                    }
                    ui.end_row();
                    label(ui, "Length");
                    if let Some(v) = f.length(ui, pin.length, 0.0) {
                        pin.length = v;
                        changed = Some(true);
                    }
                    ui.end_row();
                    label(ui, "Body side");
                    if let Some(a) = fields::rotation_combo(ui, "pin_dir", pin.direction, PIN_SIDES) {
                        pin.direction = a;
                        changed = Some(false);
                    }
                    ui.end_row();
                    label(ui, "Show name");
                    changed = changed.or(ui.checkbox(&mut pin.show_name, "").changed().then_some(false));
                    ui.end_row();
                });
            }
            (Content::Footprint(fp), Elem::Pad(id)) => {
                let Some(pad) = fp.pads.get_mut(id) else { return };
                fields::heading(ui, "Pad", &pad.name);
                grid(ui, "pad", |ui| {
                    label(ui, "Name");
                    if let Some(n) = f.text(ui, ("pad_name", id), &pad.name) {
                        pad.name = n;
                        changed = Some(false);
                    }
                    ui.end_row();
                    label(ui, "Position");
                    if let Some(p) = f.point(ui, pad.pos) {
                        pad.pos = p;
                        changed = Some(true);
                    }
                    ui.end_row();
                    let before = (pad.shape.clone(), pad.kind.clone(), pad.size);
                    pad_fields(ui, f, &mut pad.shape, &mut pad.kind, &mut pad.size);
                    if before != (pad.shape.clone(), pad.kind.clone(), pad.size) {
                        changed = Some(true);
                    }
                    let smd = pad.kind == PadKind::Smd;
                    if smd {
                        label(ui, "Back side");
                        if ui
                            .checkbox(&mut pad.back, "")
                            .on_hover_text("On the footprint's other side (card edge fingers)")
                            .changed()
                        {
                            changed = Some(false);
                        }
                        ui.end_row();
                    }
                    if let Some(g) = opening_rows(ui, f, std::slice::from_mut(pad), &rules) {
                        changed = Some(g);
                    }
                    label(ui, "Rotation");
                    if let Some(a) = fields::rotation_combo(ui, "pad_rot", pad.rotation, DEGREES) {
                        pad.rotation = a;
                        changed = Some(false);
                    }
                    ui.end_row();
                });
            }
            (Content::Symbol(s), Elem::Graphic(i)) => {
                let Some(g) = s.graphics.get_mut(i) else { return };
                fields::heading(ui, shape_name(g), "");
                grid(ui, "graphic", |ui| changed = shape_fields(ui, f, g));
            }
            (Content::Footprint(fp), Elem::Graphic(i)) => {
                let Some(g) = fp.graphics.get_mut(i) else { return };
                fields::heading(ui, shape_name(&g.shape), "");
                grid(ui, "graphic", |ui| {
                    label(ui, "Layer");
                    if let Some(l) = layer_combo(ui, "g_layer", g.layer) {
                        g.layer = l;
                        changed = Some(false);
                    }
                    ui.end_row();
                    label(ui, "Back side");
                    if ui.checkbox(&mut g.back, "").changed() {
                        changed = Some(false);
                    }
                    ui.end_row();
                    changed = changed.or(shape_fields(ui, f, &mut g.shape));
                });
            }
            (Content::Symbol(s), Elem::Text(i)) => {
                let Some(t) = s.texts.get_mut(i) else { return };
                fields::heading(ui, "Text", "");
                grid(ui, "text", |ui| changed = text_fields(ui, f, i, t));
            }
            (Content::Footprint(fp), Elem::Text(i)) => {
                let Some(t) = fp.texts.get_mut(i) else { return };
                fields::heading(ui, "Text", "");
                grid(ui, "text", |ui| {
                    label(ui, "Layer");
                    if let Some(l) = layer_combo(ui, "t_layer", t.layer) {
                        t.layer = l;
                        changed = Some(false);
                    }
                    ui.end_row();
                    label(ui, "Back side");
                    if ui.checkbox(&mut t.back, "").changed() {
                        changed = Some(false);
                    }
                    ui.end_row();
                    changed = changed.or(text_fields(ui, f, i, &mut t.text));
                });
            }
            _ => return,
        }
        if let Some(grouped) = changed {
            self.commit(item, content, grouped);
        }
    }
}

const PIN_SIDES: [&str; 4] = ["Right", "Up", "Left", "Down"];
const DEGREES: [&str; 4] = ["0°", "90°", "180°", "270°"];

pub fn issues_ui(ui: &mut egui::Ui, issues: &[String]) {
    if issues.is_empty() {
        return;
    }
    ui.add_space(8.0);
    ui.label(RichText::new(format!("{} problem(s)", issues.len())).color(theme::warn()).strong());
    for i in issues {
        ui.label(RichText::new(i).color(theme::warn()));
    }
}

pub fn shape_name(s: &Shape) -> &'static str {
    match s {
        Shape::Line { .. } => "Line",
        Shape::Arc { .. } => "Arc",
        Shape::Circle { .. } => "Circle",
        Shape::Rect { .. } => "Rectangle",
        Shape::Polygon { .. } => "Polygon",
    }
}

const PIN_KINDS: &[(PinKind, &str)] = &[
    (PinKind::Passive, "Passive"),
    (PinKind::Input, "Input"),
    (PinKind::Output, "Output"),
    (PinKind::Bidirectional, "Bidirectional"),
    (PinKind::TriState, "Tri-state"),
    (PinKind::PowerIn, "Power in"),
    (PinKind::PowerOut, "Power out"),
    (PinKind::OpenCollector, "Open collector"),
    (PinKind::NotConnected, "Not connected"),
];

fn pin_kind_combo(ui: &mut egui::Ui, salt: &str, current: PinKind) -> Option<PinKind> {
    let options: Vec<(PinKind, String)> = PIN_KINDS.iter().map(|(k, n)| (*k, n.to_string())).collect();
    fields::combo(ui, salt, &current, &options)
}

const LAYERS: &[(FootprintLayer, &str)] = &[
    (FootprintLayer::Silkscreen, "Silkscreen"),
    (FootprintLayer::Fabrication, "Fabrication"),
    (FootprintLayer::Courtyard, "Courtyard"),
    (FootprintLayer::Copper, "Copper"),
    (FootprintLayer::SolderMask, "Solder mask"),
    (FootprintLayer::SolderPaste, "Solder paste"),
];

fn layer_combo(ui: &mut egui::Ui, salt: &str, current: FootprintLayer) -> Option<FootprintLayer> {
    let options: Vec<(FootprintLayer, String)> = LAYERS.iter().map(|(k, n)| (*k, n.to_string())).collect();
    fields::combo(ui, salt, &current, &options)
}

/// Shape, kind/drill and size rows of a pad (or the pad tool).
fn pad_fields(ui: &mut egui::Ui, f: &mut Fields, shape: &mut PadShape, kind: &mut PadKind, size: &mut Size) {
    label(ui, "Shape");
    let shapes = [
        (PadShape::Circle, "Circle".to_string()),
        (PadShape::Rect, "Rectangle".to_string()),
        (PadShape::RoundRect { ratio: 0.25 }, "Rounded rectangle".to_string()),
        (PadShape::Oval, "Oval".to_string()),
    ];
    let current = match &*shape {
        PadShape::RoundRect { .. } => PadShape::RoundRect { ratio: 0.25 },
        s => s.clone(),
    };
    if let Some(s) = fields::combo(ui, "pad_shape", &current, &shapes) {
        *shape = s;
    }
    ui.end_row();
    if let PadShape::RoundRect { ratio } = shape {
        label(ui, "Corner ratio");
        if let Some(v) = f.number(ui, *ratio, 0.0..=0.5, 0.01) {
            *ratio = v;
        }
        ui.end_row();
    }
    label(ui, "Size");
    if let Some(s) = f.size(ui, *size) {
        *size = s;
        if matches!(shape, PadShape::Circle) {
            size.h = size.w;
        }
    }
    ui.end_row();
    label(ui, "Type");
    let drill = match kind {
        PadKind::ThroughHole { drill } | PadKind::NonPlatedHole { drill } => *drill,
        PadKind::Smd => Size::square(mm(1.0).min(size.w.min(size.h) * 6 / 10)),
    };
    let kinds = [
        (PadKind::Smd, "SMD".to_string()),
        (PadKind::ThroughHole { drill }, "Through hole".to_string()),
        (PadKind::NonPlatedHole { drill }, "Hole (non-plated)".to_string()),
    ];
    if let Some(k) = fields::combo(ui, "pad_kind", kind, &kinds) {
        *kind = k;
    }
    ui.end_row();
    if let PadKind::ThroughHole { drill } | PadKind::NonPlatedHole { drill } = kind {
        // Equal width and height: a round hole; else an oval slot.
        label(ui, if drill.w == drill.h { "Drill" } else { "Drill (slot)" });
        if let Some(v) = f.size(ui, *drill) {
            *drill = v;
        }
        ui.end_row();
    }
}

/// Geometry rows of a shape. Returns `Some(grouped)` if something changed.
fn shape_fields(ui: &mut egui::Ui, f: &mut Fields, s: &mut Shape) -> Option<bool> {
    let mut changed = None;
    let mut pt = |ui: &mut egui::Ui, f: &mut Fields, name: &str, p: &mut Point| {
        label(ui, name);
        if let Some(n) = f.point(ui, *p) {
            *p = n;
            changed = Some(true);
        }
        ui.end_row();
    };
    match s {
        Shape::Line { a, b, .. } | Shape::Rect { a, b, .. } => {
            pt(ui, f, "Start", a);
            pt(ui, f, "End", b);
        }
        Shape::Circle { center, radius, .. } | Shape::Arc { center, radius, .. } => {
            pt(ui, f, "Center", center);
            label(ui, "Radius");
            if let Some(v) = f.length(ui, *radius, 0.0) {
                *radius = v;
                changed = Some(true);
            }
            ui.end_row();
        }
        Shape::Polygon { points, .. } => {
            fields::row(ui, "Points", points.len().to_string());
        }
    }
    let (width, fill) = match s {
        Shape::Line { width, .. } | Shape::Arc { width, .. } => (width, None),
        Shape::Circle { width, fill, .. } | Shape::Rect { width, fill, .. } | Shape::Polygon { width, fill, .. } => {
            (width, Some(fill))
        }
    };
    label(ui, "Width");
    if let Some(v) = f.length(ui, *width, 0.0) {
        *width = v;
        changed = Some(true);
    }
    ui.end_row();
    if let Some(fill) = fill {
        label(ui, "Filled");
        if ui.checkbox(fill, "").changed() {
            changed = Some(false);
        }
        ui.end_row();
    }
    changed
}

fn text_fields(ui: &mut egui::Ui, f: &mut Fields, index: usize, t: &mut Text) -> Option<bool> {
    let mut changed = None;
    label(ui, "Text");
    if let Some(s) = f.text(ui, ("text", index), &t.text) {
        t.text = s;
        changed = Some(false);
    }
    ui.end_row();
    label(ui, "Size");
    if let Some(v) = f.length(ui, t.size, 0.1) {
        t.size = v;
        changed = Some(true);
    }
    ui.end_row();
    label(ui, "Bold");
    if ui.checkbox(&mut t.bold, "").changed() {
        changed = Some(false);
    }
    ui.end_row();
    label(ui, "Position");
    if let Some(p) = f.point(ui, t.pos) {
        t.pos = p;
        changed = Some(true);
    }
    ui.end_row();
    label(ui, "Rotation");
    if let Some(a) = fields::rotation_combo(ui, "text_rot", t.rotation, DEGREES) {
        t.rotation = a;
        changed = Some(false);
    }
    ui.end_row();
    label(ui, "Alignment");
    let h = [
        (HAlign::Left, "Left".to_string()),
        (HAlign::Center, "Center".to_string()),
        (HAlign::Right, "Right".to_string()),
    ];
    if let Some(a) = fields::combo(ui, "text_halign", &t.halign, &h) {
        t.halign = a;
        changed = Some(false);
    }
    ui.end_row();
    changed
}

/// The mask and paste rows of one or more pads: on/off and the margin, the design rule's
/// until a value is typed (↺ goes back to it). Mixed values show the first pad's.
/// Returns `Some(grouped)` when something changed.
fn opening_rows(ui: &mut egui::Ui, f: &mut fields::Fields, pads: &mut [Pad], rules: &DesignRules) -> Option<bool> {
    let first = pads.first()?.clone();
    let mut changed = None;
    let smd = pads.iter().all(|p| p.kind == PadKind::Smd);
    let rows: [(&str, bool, Option<Coord>, Coord); 2] = [
        ("Solder mask", first.mask, first.mask_margin, rules.mask_expansion),
        ("Solder paste", first.paste, first.paste_margin, rules.paste_margin),
    ];
    for (i, (name, on, margin, rule)) in rows.into_iter().enumerate() {
        if i == 1 && !smd {
            continue;
        }
        label(ui, name);
        ui.horizontal(|ui| {
            let mut v = on;
            let tip = if i == 0 { "The solder mask opens over the pad" } else { "Off for test points and fiducials" };
            if ui.checkbox(&mut v, "").on_hover_text(tip).changed() {
                for p in pads.iter_mut() {
                    if i == 0 { p.mask = v } else { p.paste = v }
                }
                changed = Some(false);
            }
            if !v {
                return;
            }
            if let Some(m) = f.length(ui, margin.unwrap_or(rule), -10.0) {
                for p in pads.iter_mut() {
                    if i == 0 { p.mask_margin = Some(m) } else { p.paste_margin = Some(m) }
                }
                changed = Some(true);
            }
            if margin.is_some() {
                let tip = format!("Back to the design rule ({})", crate::units::fmt(rule));
                if crate::project_panel::header_button(ui, Icon::Reset, &tip, false).clicked() {
                    for p in pads.iter_mut() {
                        if i == 0 { p.mask_margin = None } else { p.paste_margin = None }
                    }
                    changed = Some(false);
                }
            } else {
                ui.label(egui::RichText::new("rule").color(theme::text_dim()))
                    .on_hover_text("The board's design rule; placed on a board, that board's rule applies");
            }
        });
        ui.end_row();
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tab_sees_its_item_deleted() {
        let mut ed = LibraryEditor::new(0, LibraryDoc::new(builtin::builtin_library()));
        let ObjectId::LibPart(copy) = ed.lib.duplicate(ObjectId::LibPart(builtin::PART_LED)).unwrap() else { panic!() };
        let tab = ed.add_tab(Open::Part(copy), true);
        assert_eq!(ed.tab_of(Open::Part(copy)).map(|t| t.id), Some(tab));
        assert!(ed.editor(tab).unwrap().exists());
        ed.lib.delete(ObjectId::LibPart(copy)).unwrap();
        let mut tab = ed.editor(tab).unwrap();
        tab.refresh();
        assert!(!tab.exists());
    }
}
