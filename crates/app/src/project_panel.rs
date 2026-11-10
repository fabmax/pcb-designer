//! The Project panel, IntelliJ style: a tree of
//!
//! * the project: Schematic, PCB and the Project Library (the parts the design uses);
//! * the Built-in Library (read-only);
//! * External Libraries: the project's library files, unsaved libraries, and files that
//!   could not be loaded.
//!
//! Libraries expand to Parts / Symbols / Footprints and their items. The app builds the
//! nodes each frame ([`Node`] with [`Info`]); the tree state (expansion, selection,
//! search, keys) is [`view::tree::TreeState`]. Context menus and the header's + menu turn
//! into [`Act`]s that the app carries out.

use std::path::{Path, PathBuf};

use egui::{Color32, Key, Modifiers, RichText, Sense, Ui, vec2};
use model::{Id, ObjectId};
use view::ViewKind;
use view::item::Item;
use view::tree::{Node, Row, TreeEvent, TreeKey, TreeState};

use crate::icons::{self, Icon};
use crate::library::Open;
use crate::theme;

/// Whose library: the project's own, or a loaded library (by its key).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Owner {
    Project,
    Lib(u64),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Group {
    Parts,
    Symbols,
    Footprints,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum NodeId {
    Project,
    Schematic,
    Board,
    ExternalLibraries,
    /// `Owner::Project`: the Project Library.
    Library(Owner),
    /// A listed library file that could not be loaded (by a hash of its path).
    Missing(u64),
    Group(Owner, Group),
    Item(Owner, ObjectId),
}

/// How a node is drawn, and what its menu offers.
#[derive(Clone, Debug, Default)]
pub struct Info {
    pub icon: Option<Icon>,
    /// Dim text after the label (a path, a count, a part's state).
    pub detail: String,
    /// The detail is a warning (a missing file, a changed library part).
    pub warn: bool,
    pub strong: bool,
    pub dim: bool,
    pub tip: String,
    /// The library can be changed (not the built-in one).
    pub editable: bool,
    /// Libraries: in the project's list. Project parts: the library has a newer version.
    pub flag: bool,
    /// Project Library: parts whose library changed.
    pub count: usize,
    pub path: Option<PathBuf>,
}

impl Info {
    pub fn icon(icon: Icon) -> Self {
        Info { icon: Some(icon), ..Default::default() }
    }
}

pub type TreeNode = Node<NodeId, Info>;

/// Something the user asked for in the tree; the app carries it out.
#[derive(Clone, Debug, PartialEq)]
pub enum Act {
    ShowView(ViewKind),
    SaveProject,
    SaveProjectAs,
    Reveal(PathBuf),
    NewLibrary,
    AddLibrary,
    ImportKicad,
    OpenLibrary(u64),
    OpenItem(u64, Open),
    SaveLibrary(u64),
    SaveLibraryAs(u64),
    /// Close an unsaved library (asking about its changes).
    CloseLibrary(u64),
    RemoveLibrary(u64),
    Locate(PathBuf),
    Forget(PathBuf),
    NewItem(u64, Group),
    /// A new part using this symbol or footprint.
    PartFrom(u64, ObjectId),
    Duplicate(u64, ObjectId),
    Rename(u64, ObjectId),
    Delete(u64, ObjectId),
    Place(Owner, Id),
    CopyToProject(u64, Id),
    /// Update project parts from their libraries (`None`: all that changed).
    Update(Option<Id>),
}

/// The + menu (header) and the External Libraries menu.
pub fn add_menu() -> Vec<Entry> {
    vec![
        Entry::Act("New library…", Act::NewLibrary),
        Entry::Act("Add library…", Act::AddLibrary),
        Entry::Act("Import from KiCad…", Act::ImportKicad),
    ]
}

pub enum Entry {
    Act(&'static str, Act),
    Separator,
}

fn group_items(g: Group) -> (&'static str, fn(u64) -> Act) {
    match g {
        Group::Parts => ("New part", |k| Act::NewItem(k, Group::Parts)),
        Group::Symbols => ("New symbol", |k| Act::NewItem(k, Group::Symbols)),
        Group::Footprints => ("New footprint", |k| Act::NewItem(k, Group::Footprints)),
    }
}

/// The context menu of a node.
pub fn menu(id: NodeId, info: &Info) -> Vec<Entry> {
    use Entry::{Act as A, Separator};
    let reveal = |out: &mut Vec<Entry>| {
        if let Some(p) = &info.path {
            out.push(A(REVEAL, Act::Reveal(p.clone())));
        }
    };
    let mut out = Vec::new();
    match id {
        NodeId::Project => {
            out.push(A("Save project", Act::SaveProject));
            out.push(A("Save project as…", Act::SaveProjectAs));
            reveal(&mut out);
        }
        NodeId::Schematic => out.push(A("Open", Act::ShowView(ViewKind::Schematic))),
        NodeId::Board => out.push(A("Open", Act::ShowView(ViewKind::Board))),
        NodeId::ExternalLibraries => out = add_menu(),
        NodeId::Library(Owner::Project) => {
            if info.count > 0 {
                out.push(A("Update changed parts from their libraries", Act::Update(None)));
            }
        }
        NodeId::Library(Owner::Lib(k)) => {
            out.push(A("Open", Act::OpenLibrary(k)));
            if !info.editable {
                out.push(A("Save a copy as…", Act::SaveLibraryAs(k)));
                return out;
            }
            out.push(Separator);
            for g in [Group::Parts, Group::Symbols, Group::Footprints] {
                let (label, act) = group_items(g);
                out.push(A(label, act(k)));
            }
            out.push(Separator);
            if info.path.is_some() {
                out.push(A("Save", Act::SaveLibrary(k)));
            }
            out.push(A("Save as…", Act::SaveLibraryAs(k)));
            reveal(&mut out);
            out.push(Separator);
            if info.flag {
                out.push(A("Remove from project", Act::RemoveLibrary(k)));
            } else {
                out.push(A("Close", Act::CloseLibrary(k)));
            }
        }
        NodeId::Missing(_) => {
            if let Some(p) = &info.path {
                out.push(A("Locate…", Act::Locate(p.clone())));
                out.push(A("Remove from project", Act::Forget(p.clone())));
            }
        }
        NodeId::Group(Owner::Lib(k), g) if info.editable => {
            let (label, act) = group_items(g);
            out.push(A(label, act(k)));
        }
        NodeId::Group(..) => {}
        NodeId::Item(Owner::Project, ObjectId::LibPart(p)) => {
            out.push(A("Place in schematic", Act::Place(Owner::Project, p)));
            if info.flag {
                out.push(A("Update from library", Act::Update(Some(p))));
            }
        }
        NodeId::Item(Owner::Project, _) => {}
        NodeId::Item(Owner::Lib(k), item) => {
            out.push(A("Open", Act::OpenItem(k, open_of(item))));
            if let ObjectId::LibPart(p) = item {
                out.push(A("Place in schematic", Act::Place(Owner::Lib(k), p)));
                out.push(A("Copy to Project Library", Act::CopyToProject(k, p)));
            }
            if info.editable {
                out.push(Separator);
                if !matches!(item, ObjectId::LibPart(_)) {
                    out.push(A("New part using this", Act::PartFrom(k, item)));
                }
                out.push(A("Duplicate", Act::Duplicate(k, item)));
                out.push(A("Rename…", Act::Rename(k, item)));
                out.push(A("Delete", Act::Delete(k, item)));
            }
        }
    }
    out
}

/// What Enter and double-click do (`None`: expand or collapse).
pub fn activate(id: NodeId, info: &Info) -> Option<Act> {
    match id {
        NodeId::Schematic => Some(Act::ShowView(ViewKind::Schematic)),
        NodeId::Board => Some(Act::ShowView(ViewKind::Board)),
        NodeId::Library(Owner::Lib(k)) => Some(Act::OpenLibrary(k)),
        NodeId::Missing(_) => info.path.clone().map(Act::Locate),
        NodeId::Item(Owner::Lib(k), item) => Some(Act::OpenItem(k, open_of(item))),
        _ => None,
    }
}

pub fn open_of(item: ObjectId) -> Open {
    match item {
        ObjectId::LibPart(p) => Open::Part(p),
        ObjectId::LibSymbol(s) => Open::Item(Item::Symbol(s)),
        ObjectId::LibFootprint(f) => Open::Item(Item::Footprint(f)),
        _ => Open::None,
    }
}

/// "1 part", "3 parts".
pub fn count(n: usize, noun: &str) -> String {
    if n == 1 { format!("1 {noun}") } else { format!("{n} {noun}s") }
}

pub fn path_hash(path: &Path) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    path.hash(&mut h);
    h.finish()
}

/// A part's detail in the tree: (text, is a warning, an update is available, tooltip).
pub type PartDetail = (String, bool, bool, String);

/// Parts, Symbols and Footprints of a library, items sorted by name. `part` gives a
/// part's state (Project Library).
pub fn library_children(
    owner: Owner,
    lib: &model::Library,
    editable: bool,
    part: impl Fn(Id) -> Option<PartDetail>,
) -> Vec<TreeNode> {
    let item = |id: ObjectId, name: &str, icon: Icon, tip: &str| {
        TreeNode::new(NodeId::Item(owner, id), name, Info { editable, tip: tip.to_string(), ..Info::icon(icon) })
    };
    let mut parts: Vec<TreeNode> = lib
        .parts
        .iter()
        .map(|p| {
            let mut n = item(ObjectId::LibPart(p.id), &p.name, Icon::Part, &p.description);
            if let Some((detail, warn, flag, tip)) = part(p.id) {
                n.data.detail = detail;
                n.data.warn = warn;
                n.data.flag = flag;
                if !tip.is_empty() {
                    n.data.tip = if n.data.tip.is_empty() { tip } else { format!("{}\n{tip}", n.data.tip) };
                }
            }
            n
        })
        .collect();
    let mut symbols: Vec<TreeNode> =
        lib.symbols.iter().map(|s| item(ObjectId::LibSymbol(s.id), &s.name, Icon::Symbol, "")).collect();
    let mut footprints: Vec<TreeNode> =
        lib.footprints.iter().map(|f| item(ObjectId::LibFootprint(f.id), &f.name, Icon::Footprint, "")).collect();
    for list in [&mut parts, &mut symbols, &mut footprints] {
        list.sort_by(|a, b| model::natural_cmp(&a.label, &b.label));
    }
    let group = |g: Group, label: &str, children: Vec<TreeNode>| {
        let info = Info { detail: children.len().to_string(), editable, ..Info::icon(Icon::Folder) };
        TreeNode::new(NodeId::Group(owner, g), label, info).with_children(children)
    };
    vec![
        group(Group::Parts, "Parts", parts),
        group(Group::Symbols, "Symbols", symbols),
        group(Group::Footprints, "Footprints", footprints),
    ]
}

#[cfg(target_os = "macos")]
const REVEAL: &str = "Reveal in Finder";
#[cfg(target_os = "windows")]
const REVEAL: &str = "Show in Explorer";
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
const REVEAL: &str = "Open containing folder";

/// Show a file in the system's file manager.
pub fn reveal(path: &Path) -> std::io::Result<()> {
    use std::process::Command;
    #[cfg(target_os = "macos")]
    let mut c = {
        let mut c = Command::new("open");
        c.arg("-R").arg(path);
        c
    };
    #[cfg(target_os = "windows")]
    let mut c = {
        let mut c = Command::new("explorer");
        c.arg(format!("/select,{}", path.display()));
        c
    };
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let mut c = {
        let mut c = Command::new("xdg-open");
        c.arg(path.parent().unwrap_or(path));
        c
    };
    c.spawn().map(drop)
}

/// `~/…` for paths in the home folder.
pub fn short_path(path: &Path) -> String {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    match home.as_deref().and_then(|h| path.strip_prefix(h).ok()) {
        Some(rest) => format!("~/{}", rest.display()),
        None => path.display().to_string(),
    }
}

// ----------------------------------------------------------------------------------------
// Panel

const ROW: f32 = 22.0;
const INDENT: f32 = 16.0;

#[derive(Default)]
pub struct ProjectPanel {
    pub state: TreeState<NodeId>,
    /// The node of the focused tab when it was last revealed.
    followed: Option<NodeId>,
    /// Scroll the selection into view next frame.
    scroll: bool,
    /// Give the search field keyboard focus next frame (cursor at the end).
    focus_search: bool,
    /// Give the tree keyboard focus next frame.
    focus_tree: bool,
}

fn tree_id() -> egui::Id {
    egui::Id::unique("project_tree")
}

fn search_id() -> egui::Id {
    egui::Id::unique("project_tree_search")
}

impl ProjectPanel {
    /// Select (and reveal) the node of the focused tab when that changes.
    pub fn follow(&mut self, roots: &[TreeNode], node: Option<NodeId>) {
        if node == self.followed {
            return;
        }
        self.followed = node;
        if let Some(n) = node
            && !self.state.is_searching()
            && self.state.reveal(roots, n)
        {
            self.scroll = true;
        }
    }

    /// Expand the project and External Libraries on first use.
    pub fn expand_defaults(&mut self) {
        if self.state.expanded.is_empty() && self.state.selected.is_none() {
            self.state.set_expanded(NodeId::Project, true);
            self.state.set_expanded(NodeId::ExternalLibraries, true);
        }
    }

    /// Header buttons, laid out right to left after the hide button: collapse all,
    /// search, +. Returns an act from the + menu.
    pub fn header_ui(&mut self, ui: &mut Ui, roots: &[TreeNode]) -> Option<Act> {
        let mut act = None;
        if header_button(ui, Icon::CollapseAll, "Collapse all", false).clicked() {
            self.state.close_search(roots);
            self.state.collapse_all();
        }
        let searching = self.state.is_searching();
        if header_button(ui, Icon::Search, "Search (or just start typing in the tree)", searching).clicked() {
            if searching {
                self.state.close_search(roots);
                self.focus_tree = true;
            } else {
                self.state.open_search();
                self.focus_search = true;
            }
        }
        let plus = header_button(ui, Icon::Plus, "Add a library", false);
        egui::Popup::menu(&plus).show(|ui| {
            ui.set_min_width(170.0);
            act = menu_ui(ui, add_menu());
        });
        act
    }

    /// The tree. Returns what the user asked for.
    pub fn ui(&mut self, ui: &mut Ui, roots: &[TreeNode]) -> Option<Act> {
        let mut act = None;
        if self.state.is_searching() {
            act = self.search_ui(ui, roots);
        }
        let rows = self.state.rows(roots);
        let width = ui.available_width();
        let height = (rows.len() as f32 * ROW).max(ui.available_height());
        let (area, _) = ui.allocate_exact_size(vec2(width, height), Sense::hover());
        // The tree's keyboard focus: a widget under the rows (clicks on empty space).
        let tree = ui.interact(area, tree_id(), Sense::click());
        if tree.clicked() {
            tree.request_focus();
        }
        if std::mem::take(&mut self.focus_tree) {
            tree.request_focus();
        }
        let focused = tree.has_focus();
        if focused {
            ui.memory_mut(|m| {
                m.set_focus_lock_filter(
                    tree_id(),
                    egui::EventFilter { horizontal_arrows: true, vertical_arrows: true, escape: true, tab: false },
                )
            });
        }

        let clip = ui.clip_rect();
        for (i, row) in rows.iter().enumerate() {
            let rect = egui::Rect::from_min_size(area.min + vec2(0.0, i as f32 * ROW), vec2(width, ROW));
            let selected = self.state.selected == Some(row.node.id);
            if selected && std::mem::take(&mut self.scroll) {
                ui.scroll_to_rect(rect, None);
            }
            if !clip.intersects(rect) {
                continue;
            }
            if let Some(a) = self.row_ui(ui, rect, row, selected, focused) {
                act = Some(a);
            }
        }
        self.scroll = false;

        if focused && let Some(a) = self.keys(ui, &rows, roots) {
            act = Some(a);
        }
        act
    }

    fn row_ui(
        &mut self,
        ui: &mut Ui,
        rect: egui::Rect,
        row: &Row<NodeId, Info>,
        selected: bool,
        focused: bool,
    ) -> Option<Act> {
        let mut act = None;
        let node = row.node;
        let info = &node.data;
        let r = ui.interact(rect, tree_id().with(node.id), Sense::click());
        let painter = ui.painter_at(rect);
        let visuals = ui.visuals();
        if selected {
            let fill = if focused { visuals.selection.bg_fill } else { visuals.widgets.inactive.weak_bg_fill };
            painter.rect_filled(rect, 4.0, fill);
        } else if r.hovered() {
            painter.rect_filled(rect, 4.0, visuals.widgets.hovered.weak_bg_fill);
        }
        let text_color = if info.dim { theme::text_dim() } else { visuals.text_color() };

        // Chevron, icon, label (match highlighted), detail.
        let mut x = rect.left() + 4.0 + row.depth as f32 * INDENT;
        let chevron = egui::Rect::from_center_size(egui::pos2(x + 7.0, rect.center().y), vec2(14.0, 14.0));
        if row.expandable {
            chevron_paint(&painter, chevron, row.expanded, theme::text_dim());
        }
        x += 16.0;
        if let Some(icon) = info.icon {
            let r = egui::Rect::from_center_size(egui::pos2(x + 8.0, rect.center().y), vec2(15.0, 15.0));
            icons::paint(&painter, r, icon, if info.dim { theme::text_dim() } else { icon_color(icon) });
            x += 21.0;
        }
        let font = egui::FontId::proportional(13.0);
        let mut job = egui::text::LayoutJob::default();
        let fmt = |color: Color32, bg: Color32| egui::TextFormat {
            font_id: font.clone(),
            color,
            background: bg,
            ..Default::default()
        };
        let label = &node.label;
        let normal = fmt(text_color, Color32::TRANSPARENT);
        match row.matched.clone().filter(|m| !m.is_empty()) {
            Some(m) => {
                job.append(&label[..m.start], 0.0, normal.clone());
                job.append(&label[m.clone()], 0.0, fmt(theme::panel_dark(), theme::accent()));
                job.append(&label[m.end..], 0.0, normal);
            }
            None => job.append(label, 0.0, normal),
        }
        if info.strong {
            // No bold face: draw the label twice, slightly offset (like theme::bold_label).
            let galley = ui.fonts_mut(|f| f.layout_job(job.clone()));
            painter.galley(egui::pos2(x + 0.6, rect.center().y - galley.size().y / 2.0), galley, text_color);
        }
        let galley = ui.fonts_mut(|f| f.layout_job(job));
        let label_w = galley.size().x;
        painter.galley(egui::pos2(x, rect.center().y - galley.size().y / 2.0), galley, text_color);
        let detail_x = x + label_w + 8.0;
        let room = rect.right() - 4.0 - detail_x;
        if !info.detail.is_empty() && room > 12.0 {
            let color = if info.warn { theme::warn() } else { theme::text_dim() };
            let mut job = egui::text::LayoutJob::single_section(
                info.detail.clone(),
                egui::TextFormat { font_id: egui::FontId::proportional(11.5), color, ..Default::default() },
            );
            job.wrap = egui::text::TextWrapping {
                max_width: room,
                max_rows: 1,
                break_anywhere: true,
                overflow_character: Some('…'),
            };
            let galley = ui.fonts_mut(|f| f.layout_job(job));
            painter.galley(egui::pos2(detail_x, rect.center().y - galley.size().y / 2.0), galley, color);
        }
        let r = if info.tip.is_empty() { r } else { r.on_hover_text(&info.tip) };
        if r.clicked() || r.secondary_clicked() {
            self.state.select(node.id);
            r.request_focus();
            self.focus_tree = true;
            let on_chevron = r.interact_pointer_pos().is_some_and(|p| chevron.expand(3.0).contains(p));
            if r.clicked() && on_chevron && row.expandable && !self.state.is_searching() {
                self.state.toggle(node.id);
            }
        }
        if r.double_clicked() {
            match activate(node.id, info) {
                Some(a) => act = Some(a),
                None if !self.state.is_searching() => self.state.toggle(node.id),
                None => {}
            }
        }
        r.context_menu(|ui| {
            let entries = menu(node.id, info);
            if entries.is_empty() {
                ui.label(RichText::new("Nothing to do here").color(theme::text_dim()));
            }
            if let Some(a) = menu_ui(ui, entries) {
                act = Some(a);
            }
        });
        act
    }

    /// Keys while the tree has focus: arrows, Enter, Delete, P (place), Esc, and typing
    /// starts a search.
    fn keys(&mut self, ui: &mut Ui, rows: &[Row<NodeId, Info>], roots: &[TreeNode]) -> Option<Act> {
        let mut act = None;
        let pressed = |ui: &mut Ui, key| ui.input_mut(|i| i.consume_key(Modifiers::NONE, key));
        for (key, tk) in [
            (Key::ArrowUp, TreeKey::Up),
            (Key::ArrowDown, TreeKey::Down),
            (Key::ArrowLeft, TreeKey::Left),
            (Key::ArrowRight, TreeKey::Right),
            (Key::Home, TreeKey::Home),
            (Key::End, TreeKey::End),
            (Key::Enter, TreeKey::Enter),
        ] {
            if pressed(ui, key) {
                act = self.tree_key(rows, tk).or(act);
                self.scroll = true;
            }
        }
        let selected = self.state.selected.and_then(|s| rows.iter().find(|r| r.node.id == s));
        let find = |row: &Row<NodeId, Info>, f: fn(&Act) -> bool| {
            menu(row.node.id, &row.node.data).into_iter().find_map(|e| match e {
                Entry::Act(_, a) if f(&a) => Some(a),
                _ => None,
            })
        };
        let delete = selected.and_then(|r| find(r, |a| matches!(a, Act::Delete(..))));
        if let Some(d) = delete
            && (pressed(ui, Key::Delete) || pressed(ui, Key::Backspace))
        {
            act = Some(d);
        }
        // P places the selected part; on anything else it is typed (a search).
        let place = selected.and_then(|r| find(r, |a| matches!(a, Act::Place(..))));
        if let Some(p) = place
            && pressed(ui, Key::P)
        {
            ui.input_mut(|i| i.events.retain(|e| !matches!(e, egui::Event::Text(t) if t.eq_ignore_ascii_case("p"))));
            act = Some(p);
        }
        if pressed(ui, Key::Escape) && self.state.is_searching() {
            self.state.close_search(roots);
        }
        // Typing searches.
        let typed: String = ui.input(|i| {
            i.events
                .iter()
                .filter_map(|e| match e {
                    egui::Event::Text(t) if !i.modifiers.command && !i.modifiers.ctrl => Some(t.as_str()),
                    _ => None,
                })
                .collect()
        });
        if !typed.trim().is_empty() {
            self.state.query_mut().push_str(&typed);
            self.state.select_first_match(roots);
            self.focus_search = true;
            self.scroll = true;
        }
        act
    }

    fn tree_key(&mut self, rows: &[Row<NodeId, Info>], key: TreeKey) -> Option<Act> {
        match self.state.key(rows, key)? {
            TreeEvent::Activate(id) => {
                let row = rows.iter().find(|r| r.node.id == id)?;
                match activate(id, &row.node.data) {
                    Some(a) => Some(a),
                    None => {
                        if !self.state.is_searching() {
                            self.state.toggle(id);
                        }
                        None
                    }
                }
            }
        }
    }

    /// The search field above the tree. Arrows and Enter go to the tree, Esc closes it.
    fn search_ui(&mut self, ui: &mut Ui, roots: &[TreeNode]) -> Option<Act> {
        let mut act = None;
        let field_focused = ui.memory(|m| m.has_focus(search_id()));
        if field_focused {
            let rows = self.state.rows(roots);
            for (key, tk) in
                [(Key::ArrowUp, TreeKey::Up), (Key::ArrowDown, TreeKey::Down), (Key::Enter, TreeKey::Enter)]
            {
                if ui.input_mut(|i| i.consume_key(Modifiers::NONE, key)) {
                    act = self.tree_key(&rows, tk).or(act);
                    self.scroll = true;
                }
            }
            if ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape)) {
                self.state.close_search(roots);
                self.focus_tree = true;
                self.scroll = true;
                return act;
            }
        }
        let before = self.state.query().unwrap_or_default().to_string();
        let mut changed = false;
        ui.horizontal(|ui| {
            let (r, _) = ui.allocate_exact_size(vec2(16.0, 16.0), Sense::hover());
            icons::paint(ui.painter(), r, Icon::Search, theme::text_dim());
            let edit = egui::TextEdit::singleline(self.state.query_mut())
                .id(search_id())
                .hint_text("Search")
                // Esc and ↑/↓ stay with the field (they go to the tree, see above).
                .event_filter(egui::EventFilter {
                    horizontal_arrows: true,
                    vertical_arrows: true,
                    escape: true,
                    tab: false,
                })
                .desired_width(f32::INFINITY);
            let r = ui.add(edit);
            if std::mem::take(&mut self.focus_search) {
                r.request_focus();
                // Typing in the tree started the search: continue after its text.
                if let Some(mut s) = egui::TextEdit::load_state(ui.ctx(), search_id()) {
                    let end = egui::text::CCursor::new(self.state.query().unwrap_or_default().chars().count());
                    s.cursor.set_char_range(Some(egui::text::CCursorRange::one(end)));
                    s.store(ui.ctx(), search_id());
                }
            }
            changed = r.changed();
        });
        if changed || before != self.state.query().unwrap_or_default() {
            self.state.select_first_match(roots);
            self.scroll = true;
        }
        ui.add_space(4.0);
        act
    }
}

fn menu_ui(ui: &mut Ui, entries: Vec<Entry>) -> Option<Act> {
    let mut act = None;
    for e in entries {
        match e {
            Entry::Separator => {
                ui.separator();
            }
            Entry::Act(label, a) => {
                if ui.button(label).clicked() {
                    act = Some(a);
                    ui.close();
                }
            }
        }
    }
    act
}

/// A small icon button in a panel header.
pub fn header_button(ui: &mut Ui, icon: Icon, tip: &str, active: bool) -> egui::Response {
    let (rect, r) = ui.allocate_exact_size(vec2(20.0, 20.0), Sense::click());
    if active {
        ui.painter().rect_filled(rect, 4.0, ui.visuals().selection.bg_fill);
    } else if r.hovered() {
        ui.painter().rect_filled(rect, 4.0, ui.visuals().widgets.hovered.weak_bg_fill);
    }
    let color = if active { theme::accent() } else { theme::text_dim() };
    icons::paint(ui.painter(), rect.shrink(3.5), icon, color);
    r.on_hover_text(tip)
}

fn chevron_paint(painter: &egui::Painter, rect: egui::Rect, open: bool, color: Color32) {
    let c = rect.center();
    let s = rect.width() * 0.22;
    let pts = if open {
        vec![c + vec2(-s, -s * 0.5), c + vec2(0.0, s * 0.5), c + vec2(s, -s * 0.5)]
    } else {
        vec![c + vec2(-s * 0.5, -s), c + vec2(s * 0.5, 0.0), c + vec2(-s * 0.5, s)]
    };
    painter.add(egui::Shape::line(pts, egui::Stroke::new(1.5, color)));
}

/// Node icons take the colour of what they stand for (like the tab icons).
fn icon_color(icon: Icon) -> Color32 {
    let pal = view::style::pal();
    let c = |c: render::Color| {
        let [r, g, b, _] = c.0;
        Color32::from_rgb(r, g, b)
    };
    match icon {
        Icon::Schematic | Icon::Symbol => c(pal.sch_wire),
        Icon::Board | Icon::Footprint => c(pal.top_copper),
        Icon::Library | Icon::Part => c(pal.sch_label),
        Icon::Missing => theme::warn(),
        _ => theme::text_dim(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn acts(entries: Vec<Entry>) -> Vec<Act> {
        entries
            .into_iter()
            .filter_map(|e| match e {
                Entry::Act(_, a) => Some(a),
                Entry::Separator => None,
            })
            .collect()
    }

    #[test]
    fn menus_follow_what_a_node_allows() {
        let builtin = Info { editable: false, ..Default::default() };
        assert_eq!(acts(menu(NodeId::Library(Owner::Lib(0)), &builtin)), [Act::OpenLibrary(0), Act::SaveLibraryAs(0)]);
        let listed = Info { editable: true, flag: true, path: Some("/l.pcblib".into()), ..Default::default() };
        let a = acts(menu(NodeId::Library(Owner::Lib(1)), &listed));
        assert!(a.contains(&Act::RemoveLibrary(1)) && a.contains(&Act::SaveLibrary(1)));
        let unsaved = Info { editable: true, ..Default::default() };
        let a = acts(menu(NodeId::Library(Owner::Lib(2)), &unsaved));
        assert!(a.contains(&Act::CloseLibrary(2)) && !a.contains(&Act::SaveLibrary(2)));

        let part = ObjectId::LibPart(Id::new());
        let a = acts(menu(NodeId::Item(Owner::Lib(0), part), &builtin));
        assert!(!a.iter().any(|a| matches!(a, Act::Delete(..) | Act::Rename(..))), "built-in items are read-only");
        assert!(a.iter().any(|a| matches!(a, Act::CopyToProject(..))));
        let a = acts(menu(NodeId::Item(Owner::Lib(1), part), &listed));
        assert!(a.iter().any(|a| matches!(a, Act::Delete(..))));

        assert_eq!(acts(menu(NodeId::ExternalLibraries, &Info::default())), acts(add_menu()));
        assert_eq!(activate(NodeId::Board, &Info::default()), Some(Act::ShowView(ViewKind::Board)));
        assert_eq!(activate(NodeId::Project, &Info::default()), None, "expands");
    }
}
