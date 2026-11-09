//! Library editor tab: browser of a library's parts, symbols and footprints, the symbol
//! and footprint editors (canvas + tools + element inspector) and the part form.

use std::path::PathBuf;

use egui::RichText;
use model::*;
use view::item::{Content, Elem, Item, ItemTarget, ItemView};
use view::tools::{Placer, Tool};
use view::{Interaction, KeyAction, LibraryDoc};

use crate::canvas::{self, Canvas};
use crate::fields::{self, Fields, grid, hint, label};
use crate::icons::Icon;
use crate::keymap::{Action, Keymap};
use crate::part_form::PartForm;
use crate::theme;
use crate::toolbar::{self, ToolButton};

pub const SYMBOL_GRIDS: &[(f64, &str)] = &[(2.54, "2.54 mm"), (1.27, "1.27 mm"), (0.635, "0.635 mm")];
pub const FOOTPRINT_GRIDS: &[(f64, &str)] =
    &[(1.0, "1 mm"), (0.5, "0.5 mm"), (0.25, "0.25 mm"), (0.1, "0.1 mm"), (0.05, "0.05 mm"), (0.635, "25 mil")];

/// What the library tab shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Open {
    None,
    Item(Item),
    Part(Id),
}

/// Requests from the library UI to the application.
pub enum LibEvent {
    /// Place a part in the open project.
    Place(Id),
    Save,
    SaveAs,
    Close,
}

pub struct LibraryEditor {
    pub key: u64,
    pub lib: LibraryDoc,
    pub open: Open,
    pub view: ItemView,
    pub canvas: Canvas,
    pub it: Interaction<Elem>,
    pub placer: Placer,
    part_form: PartForm,
    /// Navigation history (browser style): previously shown items and parts.
    back: Vec<Open>,
    forward: Vec<Open>,
    fields: Fields,
    filter: String,
    revision: u64,
    pub message: Option<String>,
}

impl LibraryEditor {
    pub fn new(key: u64, lib: LibraryDoc) -> Self {
        let canvas = Canvas::new(1000 + key, view::style::background(1.27));
        LibraryEditor {
            key,
            lib,
            open: Open::None,
            view: ItemView::new(),
            canvas,
            it: Interaction::new(mm(1.27)),
            placer: Placer::default(),
            part_form: PartForm::default(),
            back: Vec::new(),
            forward: Vec::new(),
            fields: Fields::default(),
            filter: String::new(),
            revision: u64::MAX,
            message: None,
        }
    }

    pub fn title(&self) -> String {
        format!("{}{}", self.lib.name(), if self.lib.is_modified() { " •" } else { "" })
    }

    pub fn path(&self) -> Option<&PathBuf> {
        self.lib.path.as_ref()
    }

    /// An interaction (drag, box, half-drawn shape) is in progress.
    pub fn busy(&self) -> bool {
        !self.it.is_idle() || self.placer.in_progress()
    }

    /// Navigate to `open` (the current item goes onto the back stack).
    pub fn show(&mut self, open: Open) {
        if open == self.open {
            return;
        }
        if self.open != Open::None {
            self.back.push(self.open);
        }
        self.forward.clear();
        self.set_open(open);
    }

    pub fn can_go_back(&self) -> bool {
        !self.back.is_empty()
    }

    pub fn can_go_forward(&self) -> bool {
        !self.forward.is_empty()
    }

    pub fn go_back(&mut self) {
        if let Some(prev) = self.back.pop() {
            if self.open != Open::None {
                self.forward.push(self.open);
            }
            self.set_open(prev);
        }
    }

    pub fn go_forward(&mut self) {
        if let Some(next) = self.forward.pop() {
            if self.open != Open::None {
                self.back.push(self.open);
            }
            self.set_open(next);
        }
    }

    fn exists(&self, open: Open) -> bool {
        let lib = self.lib.library();
        match open {
            Open::None => true,
            Open::Item(Item::Symbol(id)) => lib.symbols.contains(id),
            Open::Item(Item::Footprint(id)) => lib.footprints.contains(id),
            Open::Part(id) => lib.parts.contains(id),
        }
    }

    /// "part LED", "symbol LED", "footprint SOIC-8".
    pub fn describe(&self, open: Open) -> String {
        let lib = self.lib.library();
        let name = |n: Option<&String>| n.cloned().unwrap_or_default();
        match open {
            Open::None => self.lib.name().to_string(),
            Open::Part(id) => format!("part {}", name(lib.parts.get(id).map(|p| &p.name))),
            Open::Item(Item::Symbol(id)) => format!("symbol {}", name(lib.symbols.get(id).map(|s| &s.name))),
            Open::Item(Item::Footprint(id)) => format!("footprint {}", name(lib.footprints.get(id).map(|f| &f.name))),
        }
    }

    fn set_open(&mut self, open: Open) {
        self.open = open;
        let item = match open {
            Open::Item(item) => Some(item),
            _ => None,
        };
        self.view.show(item);
        self.view.sync(&self.lib.doc);
        self.it.reset();
        self.placer.set_tool(Tool::Select, None);
        self.view.set_preview(&[]);
        self.fields.forget();
        if let Some(item) = item {
            let g = match item {
                Item::Symbol(_) => 1.27,
                Item::Footprint(_) => 0.25,
            };
            self.it.grid = mm(g);
            self.canvas.background.grid = g as f32;
            self.canvas.request_fit();
        }
    }

    pub fn set_tool(&mut self, tool: Tool) {
        let Open::Item(item) = self.open else { return };
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
        let mut t = ItemTarget { doc: &mut self.lib.doc, view: &mut self.view };
        self.it.set_selection(&mut t, Vec::new());
        self.placer.set_tool(tool, self.view.content());
        if tool == Tool::Select {
            self.view.set_preview(&[]);
        }
    }

    // ------------------------------------------------------------------------------------
    // Commands

    /// Keep views and selections consistent with the document (after any change).
    pub fn refresh(&mut self) {
        if self.lib.doc.revision == self.revision {
            return;
        }
        self.revision = self.lib.doc.revision;
        // Deleted objects disappear from the view and from the history.
        let mut back = std::mem::take(&mut self.back);
        let mut forward = std::mem::take(&mut self.forward);
        back.retain(|o| self.exists(*o));
        forward.retain(|o| self.exists(*o));
        back.dedup();
        (self.back, self.forward) = (back, forward);
        if !self.exists(self.open) {
            self.set_open(Open::None);
        }
        self.view.sync(&self.lib.doc);
        self.it.refresh(&mut ItemTarget { doc: &mut self.lib.doc, view: &mut self.view });
    }

    pub fn key(&mut self, action: KeyAction) {
        if !matches!(self.open, Open::Item(_)) {
            return;
        }
        if self.placer.is_placing() {
            match action {
                KeyAction::Rotate => self.placer.rotate(),
                KeyAction::Escape if self.placer.escape() => self.view.set_preview(&[]),
                _ => {}
            }
            return;
        }
        let mut t = ItemTarget { doc: &mut self.lib.doc, view: &mut self.view };
        self.it.key(&mut t, action);
    }

    pub fn undo(&mut self) -> Option<String> {
        if self.busy() {
            return None;
        }
        let label = self.lib.doc.editor().history().undo_label().map(str::to_string);
        self.lib.doc.undo();
        self.refresh();
        Some(label.map_or("Nothing to undo".into(), |l| format!("Undo: {l}")))
    }

    pub fn redo(&mut self) -> Option<String> {
        if self.busy() {
            return None;
        }
        let label = self.lib.doc.editor().history().redo_label().map(str::to_string);
        self.lib.doc.redo();
        self.refresh();
        Some(label.map_or("Nothing to redo".into(), |l| format!("Redo: {l}")))
    }

    fn report<T>(&mut self, r: Result<T, String>) -> Option<T> {
        r.map_err(|e| self.message = Some(e)).ok()
    }

    // ------------------------------------------------------------------------------------
    // Browser (left panel)

    pub fn browser_ui(&mut self, ui: &mut egui::Ui, can_place: bool) -> Option<LibEvent> {
        let mut event = None;
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.label(RichText::new(self.lib.name()).strong().size(16.0));
            if self.lib.is_modified() {
                ui.label(RichText::new("modified").color(theme::warn()).small());
            }
        });
        let path = self.path().map_or("Not saved yet".to_string(), |p| p.display().to_string());
        ui.label(RichText::new(path).color(theme::text_dim()).small());
        ui.horizontal(|ui| {
            if ui.button("Save").clicked() {
                event = Some(LibEvent::Save);
            }
            if ui.button("Save as…").clicked() {
                event = Some(LibEvent::SaveAs);
            }
            if ui.button("Close").clicked() {
                event = Some(LibEvent::Close);
            }
        });
        ui.add_space(4.0);
        ui.add(egui::TextEdit::singleline(&mut self.filter).hint_text("Filter").desired_width(f32::INFINITY));
        ui.add_space(4.0);

        let filter = self.filter.to_lowercase();
        let keep = |name: &str| filter.is_empty() || name.to_lowercase().contains(&filter);
        let lib = self.lib.library();
        let mut parts: Vec<(Id, String, String)> =
            lib.parts.iter().filter(|p| keep(&p.name)).map(|p| (p.id, p.name.clone(), p.description.clone())).collect();
        let mut symbols: Vec<(Id, String)> =
            lib.symbols.iter().filter(|s| keep(&s.name)).map(|s| (s.id, s.name.clone())).collect();
        let mut footprints: Vec<(Id, String)> =
            lib.footprints.iter().filter(|f| keep(&f.name)).map(|f| (f.id, f.name.clone())).collect();
        parts.sort_by(|a, b| natural_cmp(&a.1, &b.1));
        symbols.sort_by(|a, b| natural_cmp(&a.1, &b.1));
        footprints.sort_by(|a, b| natural_cmp(&a.1, &b.1));
        let (np, ns, nf) = (lib.parts.len(), lib.symbols.len(), lib.footprints.len());

        let mut action: Option<(ObjectId, &'static str)> = None;
        let mut new_part = false;
        section(
            ui,
            &format!("Parts ({np})"),
            "New part (with new symbol and footprint)",
            &mut || new_part = true,
            |ui| {
                for (id, name, description) in &parts {
                    let selected = self.open == Open::Part(*id);
                    let r = ui.selectable_label(selected, name);
                    let r = if description.is_empty() { r } else { r.on_hover_text(description) };
                    if r.clicked() {
                        action = Some((ObjectId::LibPart(*id), "open"));
                    }
                    if r.double_clicked() && can_place {
                        event = Some(LibEvent::Place(*id));
                    }
                    r.context_menu(|ui| {
                        if ui.add_enabled(can_place, egui::Button::new("Place in schematic")).clicked() {
                            event = Some(LibEvent::Place(*id));
                        }
                        if ui.button("Duplicate").clicked() {
                            action = Some((ObjectId::LibPart(*id), "duplicate"));
                        }
                        if ui.button("Delete").clicked() {
                            action = Some((ObjectId::LibPart(*id), "delete"));
                        }
                    });
                }
            },
        );
        let mut item_list =
            |ui: &mut egui::Ui, items: &[(Id, String)], make: fn(Id) -> ObjectId, open: fn(Id) -> Open| {
                for (id, name) in items {
                    let r = ui.selectable_label(self.open == open(*id), name);
                    if r.clicked() {
                        action = Some((make(*id), "open"));
                    }
                    r.context_menu(|ui| {
                        if ui.button("New part using this").clicked() {
                            action = Some((make(*id), "part"));
                        }
                        if ui.button("Duplicate").clicked() {
                            action = Some((make(*id), "duplicate"));
                        }
                        if ui.button("Delete").clicked() {
                            action = Some((make(*id), "delete"));
                        }
                    });
                }
            };
        let mut new_symbol = false;
        section(ui, &format!("Symbols ({ns})"), "New symbol", &mut || new_symbol = true, |ui| {
            item_list(ui, &symbols, ObjectId::LibSymbol, |id| Open::Item(Item::Symbol(id)))
        });
        let mut new_footprint = false;
        section(ui, &format!("Footprints ({nf})"), "New footprint", &mut || new_footprint = true, |ui| {
            item_list(ui, &footprints, ObjectId::LibFootprint, |id| Open::Item(Item::Footprint(id)))
        });

        if new_part {
            let r = self.lib.new_part("New part", None, None);
            if let Some(p) = self.report(r) {
                self.show(Open::Part(p));
            }
        }
        if new_symbol {
            let r = self.lib.new_symbol("New symbol");
            if let Some(id) = self.report(r) {
                self.show(Open::Item(Item::Symbol(id)));
            }
        }
        if new_footprint {
            let r = self.lib.new_footprint("New footprint");
            if let Some(id) = self.report(r) {
                self.show(Open::Item(Item::Footprint(id)));
            }
        }
        if let Some((id, what)) = action {
            self.browser_action(id, what);
        }
        event
    }

    fn browser_action(&mut self, id: ObjectId, what: &str) {
        let open = |id: ObjectId| match id {
            ObjectId::LibPart(p) => Open::Part(p),
            ObjectId::LibSymbol(s) => Open::Item(Item::Symbol(s)),
            ObjectId::LibFootprint(f) => Open::Item(Item::Footprint(f)),
            _ => Open::None,
        };
        match what {
            "open" => self.show(open(id)),
            "part" => {
                let (s, f) = match id {
                    ObjectId::LibSymbol(s) => (Some(s), None),
                    ObjectId::LibFootprint(f) => (None, Some(f)),
                    _ => (None, None),
                };
                let name = match id {
                    ObjectId::LibSymbol(s) => self.lib.library().symbols.get(s).map(|s| s.name.clone()),
                    ObjectId::LibFootprint(f) => self.lib.library().footprints.get(f).map(|f| f.name.clone()),
                    _ => None,
                };
                let r = self.lib.new_part(&name.unwrap_or("New part".into()), s, f);
                if let Some(p) = self.report(r) {
                    self.show(Open::Part(p));
                }
            }
            "duplicate" => {
                let r = self.lib.duplicate(id);
                if let Some(new) = self.report(r) {
                    self.show(open(new));
                }
            }
            "delete" => {
                let r = self.lib.delete(id);
                self.report(r);
            }
            _ => {}
        }
        self.refresh();
    }

    // ------------------------------------------------------------------------------------
    // Center (tab content)

    /// Tab content. Returns true if the pointer was pressed inside (focus).
    pub fn center_ui(&mut self, ui: &mut egui::Ui, keymap: &Keymap) -> bool {
        self.refresh();
        let pressed = ui.rect_contains_pointer(ui.max_rect()) && ui.input(|i| i.pointer.any_pressed());
        if self.open != Open::None {
            self.nav_bar(ui);
        }
        match self.open {
            Open::None => {
                ui.centered_and_justified(|ui| {
                    ui.label(
                        RichText::new(format!(
                            "{}\n\nOpen a part, symbol or footprint from the list on the left, or create one with +.",
                            self.lib.name()
                        ))
                        .color(theme::text_dim()),
                    );
                });
            }
            Open::Part(id) => {
                let frame = egui::Frame::NONE.fill(theme::panel()).inner_margin(egui::Margin::same(16));
                frame.show(ui, |ui| {
                    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                        let mut message = None;
                        if let Some(open) = self.part_form.ui(ui, &mut self.lib, id, &mut message) {
                            self.show(open);
                        }
                        if message.is_some() {
                            self.message = message;
                        }
                    });
                });
            }
            Open::Item(item) => self.item_ui(ui, item, keymap),
        }
        pressed
    }

    /// Back / forward and what is shown, above the editor or part form.
    fn nav_bar(&mut self, ui: &mut egui::Ui) {
        let width = ui.max_rect().width();
        egui::Frame::NONE.fill(theme::panel()).inner_margin(egui::Margin::symmetric(8, 4)).show(ui, |ui| {
            ui.set_width(width - 16.0);
            ui.horizontal(|ui| {
                let back = self.back.last().map(|o| self.describe(*o));
                let label = back.as_ref().map_or("Back".to_string(), |b| format!("Back to {b}"));
                let r = ui.add_enabled(back.is_some(), egui::Button::new(label));
                if r.clicked() {
                    self.go_back();
                    return;
                }
                if let Some(next) = self.forward.last().map(|o| self.describe(*o))
                    && ui.button("Forward").on_hover_text(next).clicked()
                {
                    self.go_forward();
                    return;
                }
                ui.separator();
                let current = self.describe(self.open);
                ui.add(egui::Label::new(RichText::new(current).strong()).truncate());
            });
        });
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
            b(Tool::Text, Icon::Text, "Place text.", Action::ToolText),
        ];
        let id = egui::Id::unique(("item_toolbar", self.key));
        let (chosen, right) = toolbar::strip(ui, rect, id, &[&select, &connect, &draw], self.placer.tool, keymap);
        if let Some(tool) = chosen {
            self.set_tool(tool);
        }
        let grids = match item {
            Item::Symbol(_) => SYMBOL_GRIDS,
            Item::Footprint(_) => FOOTPRINT_GRIDS,
        };
        toolbar::header(ui, rect, right, id.with("header"), |ui| {
            let current = to_mm(self.it.grid);
            let text = grids
                .iter()
                .find(|g| (g.0 - current).abs() < 1e-9)
                .map_or(format!("{current} mm"), |g| g.1.to_string());
            egui::ComboBox::from_id_salt(("item_grid", self.key)).selected_text(format!("Grid {text}")).show_ui(
                ui,
                |ui| {
                    for &(g, name) in grids {
                        if ui.selectable_label((g - current).abs() < 1e-9, name).clicked() {
                            self.it.grid = mm(g);
                            self.canvas.background.grid = g as f32;
                        }
                    }
                },
            );
            let fit = keymap.menu_text(ui.ctx(), Action::FitView);
            if ui.button("Fit").on_hover_text(format!("Zoom to show the whole item ({fit})")).clicked() {
                self.canvas.request_fit();
            }
        });
        let tip = match self.placer.tool {
            Tool::Select => "Click/box to select · drag to move · R rotate · X mirror · Del delete",
            Tool::Line => "Click points · click the last point again or Esc to finish",
            Tool::Rect => "Click two corners",
            Tool::Circle => "Click center, then a point on the circle",
            _ => "Click to place · R rotate · Esc done",
        };
        toolbar::hint(ui, rect, tip);
    }

    fn item_ui(&mut self, ui: &mut egui::Ui, item: Item, keymap: &Keymap) {
        // The canvas fills the rest of the tab; the tool bars float over it.
        let rect = ui.available_rect_before_wrap();
        let scene = &mut self.view.scene;
        let canvas = &mut self.canvas;
        let response = ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| canvas.ui(ui, scene)).inner;
        self.item_toolbar(ui, rect, item, keymap);
        if self.placer.is_placing() {
            let world = response.hover_pos().map(|p| {
                let [x, y] = self.canvas.screen_to_world(p);
                self.it.snap(Point::mm(x, y))
            });
            self.placer.hover(world);
            if let Some(p) = world
                && response.hovered()
                && ui.input(|i| i.pointer.primary_pressed())
            {
                let r = self.placer.click(&mut self.lib.doc, &mut self.view, p);
                self.report(r);
            }
            self.view.set_preview(&self.placer.ghosts(Some(item)));
            if response.hovered() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
            }
        } else {
            let engaged = !self.it.is_idle();
            if let Some(input) = canvas::pointer_input(ui, &response, &self.canvas, engaged, self.it.hover.is_some()) {
                let mut t = ItemTarget { doc: &mut self.lib.doc, view: &mut self.view };
                self.it.pointer(&mut t, &input);
            }
            if self.it.is_dragging() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
            } else if self.it.hover.is_some() && response.hovered() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            }
            if let Some((r, crossing)) = self.it.box_rect() {
                canvas::draw_box(ui, &self.canvas, response.rect, r, crossing);
            }
            if let Some(m) = self.it.message.take() {
                self.message = Some(m);
            }
        }
        if !self.it.is_idle() {
            ui.ctx().request_repaint();
        }
    }

    // ------------------------------------------------------------------------------------
    // Inspector (right panel)

    pub fn inspector_ui(&mut self, ui: &mut egui::Ui) {
        self.fields.begin_frame();
        match self.open {
            Open::Item(item) if self.placer.is_placing() => self.tool_settings(ui, item),
            Open::Item(item) => match self.it.selection.as_slice() {
                [] => self.item_summary(ui, item),
                [e] => self.element(ui, item, *e),
                many => {
                    fields::heading(ui, &format!("{} elements", many.len()), "");
                    hint(ui, "R rotate · X mirror · Del delete · drag to move");
                }
            },
            Open::Part(_) => {
                fields::heading(ui, "Part", "");
                hint(ui, "Edit units, footprint variants and the pin-pad mapping in the form.");
            }
            Open::None => self.library_summary(ui),
        }
        if self.fields.end_frame() {
            self.lib.doc.close_group();
        }
    }

    fn commit(&mut self, item: Item, content: Content, grouped: bool) {
        let group = grouped.then(|| self.fields.group());
        let r = view::item::update_item(&mut self.lib.doc, item, group, |c| {
            *c = content;
            Ok(())
        });
        self.report(r);
        self.refresh();
    }

    fn library_summary(&mut self, ui: &mut egui::Ui) {
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

    fn item_summary(&mut self, ui: &mut egui::Ui, item: Item) {
        let Some(content) = self.view.content().cloned() else { return };
        let lib = self.lib.library();
        let users: Vec<String> = lib
            .parts
            .iter()
            .filter(|p| match item {
                Item::Symbol(s) => p.units.iter().any(|u| u.symbol == s),
                Item::Footprint(f) => p.variants.iter().any(|v| v.footprint == f),
            })
            .map(|p| p.name.clone())
            .collect();
        match content {
            Content::Symbol(mut s) => {
                fields::heading(ui, "Symbol", "");
                grid(ui, "sym_summary", |ui| {
                    label(ui, "Name");
                    if let Some(n) = self.fields.text(ui, ("sym_name", s.id), &s.name) {
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
                    if let Some(n) = self.fields.text(ui, ("fp_name", f.id), &f.name) {
                        f.name = n;
                        self.commit(item, Content::Footprint(f.clone()), false);
                    }
                    ui.end_row();
                    label(ui, "Description");
                    if let Some(d) = self.fields.text(ui, ("fp_desc", f.id), &f.description) {
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
                let id = item.object_id();
                self.browser_action(id, "part");
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
        let tool = self.placer.tool;
        fields::heading(ui, tool.label(), "new elements");
        let s = &mut self.placer.settings;
        let f = &mut self.fields;
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
            Tool::Line | Tool::Rect | Tool::Circle => {
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
                if tool != Tool::Line {
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
            Tool::Select => {}
        });
        ui.add_space(6.0);
        hint(ui, "R rotates · Esc returns to Select");
    }

    fn element(&mut self, ui: &mut egui::Ui, item: Item, e: Elem) {
        let Some(mut content) = self.view.content().cloned() else { return };
        let mut changed = None; // Some(grouped)
        let f = &mut self.fields;
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

fn section(ui: &mut egui::Ui, title: &str, add_tip: &str, on_add: &mut dyn FnMut(), body: impl FnOnce(&mut egui::Ui)) {
    let id = ui.make_persistent_id(title.split(' ').next().unwrap_or(title));
    egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id, true)
        .show_header(ui, |ui| {
            ui.label(RichText::new(title).strong());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.small_button("+").on_hover_text(add_tip).clicked() {
                    on_add();
                }
            });
        })
        .body(body);
    ui.add_space(4.0);
}

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
        PadKind::Smd => mm(1.0).min(size.w.min(size.h) * 6 / 10),
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
        label(ui, "Drill");
        if let Some(v) = f.length(ui, *drill, 0.05) {
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
        t.width = v / 7;
        t.size = v;
        changed = Some(true);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn back_and_forward_through_part_and_its_footprint() {
        let mut ed = LibraryEditor::new(0, LibraryDoc::new(builtin::starter_library()));
        let led = ed.lib.library().parts.get(builtin::PART_LED).unwrap().clone();
        let part = Open::Part(led.id);
        let fp = Open::Item(Item::Footprint(led.variants[0].footprint));
        ed.show(part);
        ed.show(fp); // "Edit footprint"
        assert_eq!(ed.describe(ed.back[0]), "part LED");
        ed.go_back();
        assert_eq!(ed.open, part);
        ed.go_forward();
        assert_eq!(ed.open, fp);
        // Deleting what the history points to removes it from the history.
        ed.go_back();
        let copy = ed.lib.duplicate(ObjectId::LibPart(led.id)).unwrap();
        let ObjectId::LibPart(copy) = copy else { panic!() };
        ed.show(Open::Part(copy));
        ed.lib.delete(ObjectId::LibPart(copy)).unwrap();
        ed.refresh();
        assert_eq!(ed.open, Open::None);
        assert_eq!(ed.back, vec![part]);
        ed.go_back();
        assert_eq!(ed.open, part);
    }
}
