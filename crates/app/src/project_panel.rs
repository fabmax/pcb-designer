//! The Project panel, IntelliJ style: a tree of
//!
//! * the project: the schematic (the root sheet, its sub-sheets below it), the PCB, the
//!   simulations (the analyses of the simulation setup) and the Project Library (the
//!   parts the design uses);
//! * the Built-in Library (read-only);
//! * External Libraries: the project's library files, unsaved libraries, and files that
//!   could not be loaded;
//! * Outputs › Gerber: the files of the last Gerber export (`outputs.rs`).
//!
//! Libraries expand to Parts / Symbols / Footprints and their items. The app builds the
//! nodes each frame ([`Node`] with [`Info`]); the tree state (expansion, selection,
//! search, keys) is [`view::tree::TreeState`]. Context menus and the header's + menu turn
//! into [`Act`]s that the app carries out.

use std::path::{Path, PathBuf};

use egui::{Color32, Key, Modifiers, RichText, Sense, Ui, vec2};
use model::{Id, ObjectId};
use view::ViewId;
use view::tree::{Node, Row, TreeEvent, TreeKey, TreeState};

use crate::icons::{self, Icon};
use crate::library::{Open, PROJECT_LIB};
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
    /// A schematic sheet (the root sheet is the Schematic node).
    Sheet(Id),
    Board,
    /// The simulation setup: its node shows the circuit settings in the Simulation panel.
    Simulations,
    /// A simulation (an analysis of the setup).
    Analysis(Id),
    ExternalLibraries,
    /// `Owner::Project`: the Project Library.
    Library(Owner),
    /// A listed library file that could not be loaded (by a hash of its path).
    Missing(u64),
    Group(Owner, Group),
    /// A category folder in a group (by a hash of its name, [`text_hash`]).
    Category(Owner, Group, u64),
    Item(Owner, ObjectId),
    Outputs,
    /// The Gerber export folder.
    Gerber,
    /// A file in it (by a hash of its name).
    OutputFile(u64),
    /// The assembly folder (bill of materials, positions).
    Assembly,
    /// The schematic PDF (File › Export schematic PDF).
    SchematicPdf,
    /// A file of either folder without an image (job file, BOM, positions), opened in
    /// the system's app (by a hash of its path in the output folder).
    DataFile(u64),
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
    /// Project Library: parts no component uses.
    pub unused: usize,
    pub path: Option<PathBuf>,
    /// Category folders: the category.
    pub category: String,
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
    ShowView(ViewId),
    /// Ask for a name, then place the new sheet's symbol on `parent` (`None`: the sheet
    /// shown last).
    NewSheet(Option<Id>),
    RenameSheet(Id),
    /// Delete a sheet with its contents (asking first if it has any).
    DeleteSheet(Id),
    /// A sheet was dragged onto another: make it a sub-sheet of that one.
    MoveSheet {
        sheet: Id,
        to: Id,
    },
    SaveProject,
    SaveProjectAs,
    Reveal(PathBuf),
    /// Open a file in the system's default app.
    OpenFile(PathBuf),
    NewLibrary,
    AddLibrary,
    ImportKicad,
    /// Open an item in a tab of its own.
    OpenItem(u64, Open),
    /// Show an item in the preview tab.
    PreviewItem(u64, Open),
    SaveLibrary(u64),
    SaveLibraryAs(u64),
    /// Write a library as KiCad libraries (`.kicad_sym` and `.pretty`).
    ExportKicadLibrary(u64),
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
    /// Ask for a new name of a category, set it on all the group's items in it.
    RenameCategory(u64, Group, String),
    Delete(u64, ObjectId),
    Place(Owner, Id),
    CopyToProject(u64, Id),
    /// Update project parts from their libraries (`None`: all that changed).
    Update(Option<Id>),
    /// Remove the Project Library's unused parts, symbols and footprints.
    RemoveUnused,
    ExportGerber,
    ExportSchematicPdf,
    /// Show the Gerber tab, with only this file (`None`: as it was).
    OpenGerber(Option<PathBuf>),
    /// Delete the export folder (and its zip), after asking.
    DeleteOutputs,
    /// Add a simulation of a kind ([`crate::sim_ui::KINDS`]).
    AddAnalysis(&'static str),
    /// Show a simulation in the panel and its output in the preview tab.
    PreviewAnalysis(Id),
    /// Show a simulation's output in a tab of its own.
    OpenAnalysis(Id),
    RenameAnalysis(Id),
    DuplicateAnalysis(Id),
    DeleteAnalysis(Id),
    RunAnalysis(Id),
    RunAllSimulations,
}

/// The library editor key of an owner (the Project Library has its own).
pub fn key(owner: Owner) -> u64 {
    match owner {
        Owner::Project => PROJECT_LIB,
        Owner::Lib(k) => k,
    }
}

/// The header's + menu.
pub fn plus_menu() -> Vec<Entry> {
    let mut out = vec![Entry::Act("New sheet…", Act::NewSheet(None)), add_simulation(), Entry::Separator];
    out.extend(add_menu());
    out
}

/// The Add simulation submenu: one entry per kind.
pub fn add_simulation() -> Entry {
    let kinds = crate::sim_ui::KINDS.iter().map(|&(kind, label)| Entry::Act(label, Act::AddAnalysis(kind))).collect();
    Entry::Sub("Add simulation", kinds)
}

/// The External Libraries menu.
pub fn add_menu() -> Vec<Entry> {
    vec![
        Entry::Act("New library…", Act::NewLibrary),
        Entry::Act("Add library…", Act::AddLibrary),
        Entry::Act("Import KiCad parts…", Act::ImportKicad),
    ]
}

pub enum Entry {
    Act(&'static str, Act),
    /// A submenu.
    Sub(&'static str, Vec<Entry>),
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
        NodeId::Sheet(s) => {
            out.push(A("Open", Act::ShowView(ViewId::Sheet(s))));
            out.push(A("New sub-sheet…", Act::NewSheet(Some(s))));
            // Sub-sheets (the root sheet is the schematic itself).
            if info.editable {
                out.push(Separator);
                out.push(A("Rename…", Act::RenameSheet(s)));
                out.push(A("Delete", Act::DeleteSheet(s)));
            }
        }
        NodeId::Board => out.push(A("Open", Act::ShowView(ViewId::Board))),
        NodeId::Simulations => {
            out.push(add_simulation());
            if info.count > 0 {
                out.push(Separator);
                out.push(A("Run all simulations", Act::RunAllSimulations));
            }
        }
        NodeId::Analysis(a) => {
            out.push(A("Open", Act::OpenAnalysis(a)));
            out.push(A("Run", Act::RunAnalysis(a)));
            out.push(Separator);
            out.push(A("Rename…", Act::RenameAnalysis(a)));
            out.push(A("Duplicate", Act::DuplicateAnalysis(a)));
            out.push(A("Delete", Act::DeleteAnalysis(a)));
        }
        NodeId::ExternalLibraries => out = add_menu(),
        NodeId::Library(Owner::Project) => {
            for g in [Group::Parts, Group::Symbols, Group::Footprints] {
                let (label, act) = group_items(g);
                out.push(A(label, act(PROJECT_LIB)));
            }
            if info.count > 0 || info.unused > 0 {
                out.push(Separator);
            }
            if info.count > 0 {
                out.push(A("Update changed parts from their libraries", Act::Update(None)));
            }
            if info.unused > 0 {
                out.push(A("Remove unused parts", Act::RemoveUnused));
            }
        }
        NodeId::Library(Owner::Lib(k)) => {
            if !info.editable {
                out.push(A("Save a copy as…", Act::SaveLibraryAs(k)));
                out.push(A("Export as KiCad library…", Act::ExportKicadLibrary(k)));
                return out;
            }
            for g in [Group::Parts, Group::Symbols, Group::Footprints] {
                let (label, act) = group_items(g);
                out.push(A(label, act(k)));
            }
            out.push(Separator);
            if info.path.is_some() {
                out.push(A("Save", Act::SaveLibrary(k)));
            }
            out.push(A("Save as…", Act::SaveLibraryAs(k)));
            out.push(A("Export as KiCad library…", Act::ExportKicadLibrary(k)));
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
        NodeId::Group(owner, g) if info.editable => {
            let (label, act) = group_items(g);
            out.push(A(label, act(key(owner))));
        }
        NodeId::Group(..) => {}
        NodeId::Category(owner, g, _) if info.editable => {
            let (label, act) = group_items(g);
            out.push(A(label, act(key(owner))));
            out.push(Separator);
            out.push(A("Rename category…", Act::RenameCategory(key(owner), g, info.category.clone())));
        }
        NodeId::Category(..) => {}
        NodeId::Outputs => out.push(A("Export Gerber files", Act::ExportGerber)),
        NodeId::Gerber => {
            if info.flag {
                out.push(A("Open", Act::OpenGerber(None)));
                out.push(A("Re-export", Act::ExportGerber));
            } else {
                out.push(A("Export Gerber files", Act::ExportGerber));
            }
            if info.flag {
                reveal(&mut out);
                out.push(Separator);
                out.push(A("Delete outputs", Act::DeleteOutputs));
            }
        }
        NodeId::OutputFile(_) => {
            out.push(A("Open", Act::OpenGerber(info.path.clone())));
            reveal(&mut out);
        }
        NodeId::Assembly => {
            out.push(A(if info.flag { "Re-export" } else { "Export Gerber and assembly files" }, Act::ExportGerber));
            if info.flag {
                reveal(&mut out);
            }
        }
        NodeId::SchematicPdf => {
            if info.flag
                && let Some(p) = &info.path
            {
                out.push(A("Open", Act::OpenFile(p.clone())));
                out.push(A("Re-export", Act::ExportSchematicPdf));
                reveal(&mut out);
            } else {
                out.push(A("Export schematic PDF", Act::ExportSchematicPdf));
            }
        }
        NodeId::DataFile(_) => {
            if let Some(p) = &info.path {
                out.push(A("Open", Act::OpenFile(p.clone())));
            }
            reveal(&mut out);
        }
        NodeId::Item(owner, item) => {
            let k = key(owner);
            out.push(A("Open", Act::OpenItem(k, open_of(item))));
            out.push(Separator);
            if let ObjectId::LibPart(p) = item {
                // Board parts (their own icon) are placed on the board.
                let board = info.icon == Some(Icon::BoardPart);
                out.push(A(if board { "Place on board" } else { "Place in schematic" }, Act::Place(owner, p)));
                match owner {
                    Owner::Lib(k) => out.push(A("Copy to Project Library", Act::CopyToProject(k, p))),
                    Owner::Project if info.flag => out.push(A("Update from library", Act::Update(Some(p)))),
                    Owner::Project => {}
                }
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
        NodeId::Sheet(s) => Some(Act::ShowView(ViewId::Sheet(s))),
        NodeId::Board => Some(Act::ShowView(ViewId::Board)),
        NodeId::Analysis(a) => Some(Act::OpenAnalysis(a)),
        NodeId::Missing(_) => info.path.clone().map(Act::Locate),
        NodeId::Item(owner, item) => Some(Act::OpenItem(key(owner), open_of(item))),
        NodeId::Gerber if info.flag => Some(Act::OpenGerber(None)),
        NodeId::Gerber if info.editable => Some(Act::ExportGerber),
        NodeId::OutputFile(_) => Some(Act::OpenGerber(info.path.clone())),
        NodeId::Assembly if !info.flag && info.editable => Some(Act::ExportGerber),
        NodeId::DataFile(_) => info.path.clone().map(Act::OpenFile),
        NodeId::SchematicPdf if info.flag => info.path.clone().map(Act::OpenFile),
        NodeId::SchematicPdf if info.editable => Some(Act::ExportSchematicPdf),
        _ => None,
    }
}

/// What a single click or arrow key landing on a node does: items and simulations show
/// in the preview tab.
pub fn preview(id: NodeId) -> Option<Act> {
    match id {
        NodeId::Item(owner, item) => Some(Act::PreviewItem(key(owner), open_of(item))),
        NodeId::Analysis(a) => Some(Act::PreviewAnalysis(a)),
        _ => None,
    }
}

/// A simulation's state in the tree: "42 %" while it runs, "outdated", "2 errors", or
/// nothing (never run, or up to date).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AnalysisDetail {
    pub text: String,
    pub warn: bool,
}

/// The Simulation node: the setup's analyses in order (`detail` gives each one's
/// state). Its detail is the count, or how many are outdated.
pub fn simulation_node(setup: &model::Simulation, detail: impl Fn(&model::Analysis) -> AnalysisDetail) -> TreeNode {
    let mut outdated = 0;
    let children: Vec<TreeNode> = setup
        .analyses
        .iter()
        .map(|a| {
            let d = detail(a);
            outdated += usize::from(d.text == OUTDATED);
            let info =
                Info { detail: d.text, warn: d.warn, tip: a.kind.label().into(), ..Info::icon(kind_icon(&a.kind)) };
            TreeNode::new(NodeId::Analysis(a.id), a.name.clone(), info).with_keywords(a.kind.label())
        })
        .collect();
    let (text, warn) = match (children.len(), outdated) {
        (0, _) => (String::new(), false),
        (n, 0) => (n.to_string(), false),
        (_, o) => (format!("{o} {OUTDATED}"), true),
    };
    let info = Info {
        detail: text,
        warn,
        count: children.len(),
        tip: "Simulations of the schematic with ngspice; the circuit settings they share".into(),
        ..Info::icon(Icon::Simulation)
    };
    TreeNode::new(NodeId::Simulations, "Simulation", info).with_children(children)
}

/// The detail of a stale simulation.
pub const OUTDATED: &str = "outdated";

/// The icon of an analysis kind.
pub fn kind_icon(kind: &model::AnalysisKind) -> Icon {
    match kind {
        model::AnalysisKind::Op => Icon::OperatingPoint,
        model::AnalysisKind::Dc { .. } => Icon::DcSweep,
        model::AnalysisKind::Ac { .. } => Icon::AcSweep,
        model::AnalysisKind::Tran { .. } => Icon::Simulation,
    }
}

/// The item a library item node shows (nodes hold library objects only).
pub fn open_of(item: ObjectId) -> Open {
    Open::of(item).unwrap_or_else(|| panic!("not a library item: {item:?}"))
}

/// "1 part", "3 parts".
pub fn count(n: usize, noun: &str) -> String {
    if n == 1 { format!("1 {noun}") } else { format!("{n} {noun}s") }
}

pub fn text_hash(text: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    text.hash(&mut h);
    h.finish()
}

pub fn path_hash(path: &Path) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    path.hash(&mut h);
    h.finish()
}

/// A Project Library part's state in the tree.
pub struct PartDetail {
    pub text: String,
    /// The text is a warning.
    pub warn: bool,
    /// Its library has a newer version.
    pub update: bool,
    /// No component uses it.
    pub dim: bool,
    pub tip: String,
}

/// Parts, Symbols and Footprints of a library, items sorted by name. `part` gives a
/// part's state (Project Library).
pub fn library_children(
    owner: Owner,
    lib: &model::Library,
    editable: bool,
    part: impl Fn(Id) -> Option<PartDetail>,
) -> Vec<TreeNode> {
    let item = |id: ObjectId, name: &str, category: &str, icon: Icon, tip: &str| {
        let info = Info { editable, tip: tip.to_string(), ..Info::icon(icon) };
        (category.to_string(), TreeNode::new(NodeId::Item(owner, id), name, info).with_keywords(category))
    };
    let parts: Vec<(String, TreeNode)> = lib
        .parts
        .iter()
        .map(|p| {
            let icon = if p.is_board_part() { Icon::BoardPart } else { Icon::Part };
            let (c, mut n) = item(ObjectId::LibPart(p.id), &p.name, &p.category, icon, &p.description);
            if let Some(d) = part(p.id) {
                n.data.detail = d.text;
                n.data.warn = d.warn;
                n.data.flag = d.update;
                n.data.dim = d.dim;
                if !d.tip.is_empty() {
                    n.data.tip = if n.data.tip.is_empty() { d.tip } else { format!("{}\n{}", n.data.tip, d.tip) };
                }
            }
            (c, n)
        })
        .collect();
    let symbols =
        lib.symbols.iter().map(|s| item(ObjectId::LibSymbol(s.id), &s.name, &s.category, Icon::Symbol, "")).collect();
    let footprints = lib
        .footprints
        .iter()
        .map(|f| item(ObjectId::LibFootprint(f.id), &f.name, &f.category, Icon::Footprint, ""))
        .collect();
    let group = |g: Group, label: &str, items: Vec<(String, TreeNode)>| {
        let count = items.len();
        let children = categorized(items, |name, children| {
            let info = Info {
                detail: children.len().to_string(),
                editable,
                category: name.to_string(),
                ..Info::icon(Icon::Folder)
            };
            TreeNode::new(NodeId::Category(owner, g, text_hash(name)), name, info).with_children(children)
        });
        let info = Info { detail: count.to_string(), editable, ..Info::icon(Icon::Folder) };
        TreeNode::new(NodeId::Group(owner, g), label, info).with_children(children)
    };
    vec![
        group(Group::Parts, "Parts", parts),
        group(Group::Symbols, "Symbols", symbols),
        group(Group::Footprints, "Footprints", footprints),
    ]
}

/// Items by category: a folder per category (`folder(name, items)`) in natural order,
/// then the uncategorised items. Items are sorted by label.
pub fn categorized<T>(
    mut items: Vec<(String, Node<NodeId, T>)>,
    folder: impl Fn(&str, Vec<Node<NodeId, T>>) -> Node<NodeId, T>,
) -> Vec<Node<NodeId, T>> {
    items.sort_by(|(ca, a), (cb, b)| {
        (ca.is_empty().cmp(&cb.is_empty()))
            .then_with(|| model::natural_cmp(ca, cb))
            .then_with(|| ca.cmp(cb))
            .then_with(|| model::natural_cmp(&a.label, &b.label))
    });
    let mut out: Vec<Node<NodeId, T>> = Vec::new();
    let mut open: Option<(String, Vec<Node<NodeId, T>>)> = None;
    for (c, n) in items {
        if open.as_ref().is_some_and(|(o, _)| *o != c) {
            let (o, ch) = open.take().unwrap();
            out.push(folder(&o, ch));
        }
        if c.is_empty() {
            out.push(n);
        } else {
            open.get_or_insert_with(|| (c, Vec::new())).1.push(n);
        }
    }
    if let Some((o, ch)) = open {
        out.push(folder(&o, ch));
    }
    out
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

/// Open a file in the system's default app.
pub fn open_file(path: &Path) -> std::io::Result<()> {
    use std::process::Command;
    #[cfg(target_os = "macos")]
    let mut c = Command::new("open");
    #[cfg(target_os = "windows")]
    let mut c = {
        let mut c = Command::new("cmd");
        c.args(["/C", "start", ""]);
        c
    };
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let mut c = Command::new("xdg-open");
    c.arg(path).spawn().map(drop)
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
    /// The sheet being dragged onto another.
    drag: Option<Id>,
    /// A node to select and reveal next frame.
    reveal: Option<NodeId>,
}

/// Whether sheet `sheet` can become a sub-sheet of `to`: not onto itself, its own
/// subtree, or the sheet it already is on.
pub fn can_drop_sheet(roots: &[TreeNode], sheet: Id, to: Id) -> bool {
    fn find(nodes: &[TreeNode], id: NodeId, parent: Option<NodeId>) -> Option<(&TreeNode, Option<NodeId>)> {
        nodes.iter().find_map(|n| if n.id == id { Some((n, parent)) } else { find(&n.children, id, Some(n.id)) })
    }
    fn contains(n: &TreeNode, id: NodeId) -> bool {
        n.id == id || n.children.iter().any(|c| contains(c, id))
    }
    let Some((node, parent)) = find(roots, NodeId::Sheet(sheet), None) else { return false };
    !contains(node, NodeId::Sheet(to)) && parent != Some(NodeId::Sheet(to))
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

    /// Select and reveal a node (next frame).
    pub fn reveal(&mut self, node: NodeId) {
        self.reveal = Some(node);
    }

    /// Expand the project and External Libraries on first use.
    /// At first: the project (with the root sheet's sub-sheets) and External Libraries
    /// expanded.
    pub fn expand_defaults(&mut self, root_sheet: Option<Id>) {
        if self.state.expanded.is_empty() && self.state.selected.is_none() {
            self.state.set_expanded(NodeId::Project, true);
            if let Some(root) = root_sheet {
                self.state.set_expanded(NodeId::Sheet(root), true);
            }
            self.state.set_expanded(NodeId::ExternalLibraries, true);
            self.state.set_expanded(NodeId::Outputs, true);
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
        let plus = header_button(ui, Icon::Plus, "Add a sheet, a simulation or a library", false);
        egui::Popup::menu(&plus).show(|ui| {
            ui.set_min_width(170.0);
            act = menu_ui(ui, plus_menu());
        });
        act
    }

    /// The tree. Returns what the user asked for.
    pub fn ui(&mut self, ui: &mut Ui, roots: &[TreeNode]) -> Option<Act> {
        let mut act = None;
        if let Some(n) = self.reveal.take()
            && !self.state.is_searching()
            && self.state.reveal(roots, n)
        {
            self.scroll = true;
        }
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
        if let Some(a) = self.drop_ui(ui, area, &rows, roots) {
            act = Some(a);
        }

        if focused && let Some(a) = self.keys(ui, &rows, roots) {
            act = Some(a);
        }
        act
    }

    /// A sheet dragged in the tree: the sheet row under the pointer is highlighted if it
    /// can take it, and the drop makes the dragged sheet its sub-sheet.
    fn drop_ui(
        &mut self,
        ui: &mut Ui,
        area: egui::Rect,
        rows: &[Row<NodeId, Info>],
        roots: &[TreeNode],
    ) -> Option<Act> {
        let sheet = self.drag?;
        let (pos, released) = ui.input(|i| (i.pointer.latest_pos(), !i.pointer.primary_down()));
        let index = pos.filter(|p| area.contains(*p)).map(|p| ((p.y - area.top()) / ROW) as usize);
        let mut act = None;
        let mut ok = false;
        if let Some(i) = index
            && let Some(NodeId::Sheet(to)) = rows.get(i).map(|r| r.node.id)
            && can_drop_sheet(roots, sheet, to)
        {
            ok = true;
            let rect = egui::Rect::from_min_size(area.min + vec2(0.0, i as f32 * ROW), vec2(area.width(), ROW));
            let stroke = egui::Stroke::new(1.5, theme::accent());
            ui.painter().rect_stroke(rect.shrink(1.0), 4.0, stroke, egui::StrokeKind::Inside);
            if released {
                act = Some(Act::MoveSheet { sheet, to });
            }
        }
        ui.ctx().set_cursor_icon(if ok { egui::CursorIcon::Grabbing } else { egui::CursorIcon::NotAllowed });
        if released {
            self.drag = None;
        } else {
            ui.ctx().request_repaint();
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
        // Sub-sheets can be dragged onto other sheets (not while searching: the rows
        // don't show the hierarchy then).
        let draggable = matches!(node.id, NodeId::Sheet(_)) && info.editable && !self.state.is_searching();
        let sense = if draggable { Sense::click_and_drag() } else { Sense::click() };
        let r = ui.interact(rect, tree_id().with(node.id), sense);
        if let NodeId::Sheet(s) = node.id
            && draggable
            && r.drag_started()
        {
            self.drag = Some(s);
        }
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
            if r.clicked() {
                act = preview(node.id);
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
        let delete = selected
            .and_then(|r| find(r, |a| matches!(a, Act::Delete(..) | Act::DeleteSheet(_) | Act::DeleteAnalysis(_))));
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

    /// A key moving the selection previews what it lands on; Enter activates.
    fn tree_key(&mut self, rows: &[Row<NodeId, Info>], key: TreeKey) -> Option<Act> {
        let before = self.state.selected;
        let Some(event) = self.state.key(rows, key) else {
            return self.state.selected.filter(|s| Some(*s) != before).and_then(preview);
        };
        match event {
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
        let changed;
        {
            let r = crate::fields::search_field(ui, self.state.query_mut(), |e| {
                e.id(search_id())
                    .hint_text("Search")
                    // Esc and ↑/↓ stay with the field (they go to the tree, see above).
                    .event_filter(egui::EventFilter {
                        horizontal_arrows: true,
                        vertical_arrows: true,
                        escape: true,
                        tab: false,
                    })
            });
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
        }
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
            Entry::Sub(label, entries) => {
                if let Some(a) = ui.menu_button(label, |ui| menu_ui(ui, entries)).inner.flatten() {
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

pub fn chevron_paint(painter: &egui::Painter, rect: egui::Rect, open: bool, color: Color32) {
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
pub fn icon_color(icon: Icon) -> Color32 {
    let pal = view::style::pal();
    let c = |c: render::Color| {
        let [r, g, b, _] = c.0;
        Color32::from_rgb(r, g, b)
    };
    match icon {
        Icon::Schematic | Icon::Symbol => c(pal.sch_wire),
        Icon::Board | Icon::Footprint | Icon::BoardPart => c(pal.top_copper),
        Icon::Library | Icon::Part => c(pal.sch_label),
        Icon::Simulation | Icon::OperatingPoint | Icon::DcSweep | Icon::AcSweep => c(view::style::trace_color(0)),
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
                Entry::Sub(..) | Entry::Separator => None,
            })
            .collect()
    }

    #[test]
    fn library_items_group_by_category() {
        let lib = model::builtin::builtin_library();
        let groups = library_children(Owner::Lib(7), &lib, false, |_| None);
        let labels = |nodes: &[TreeNode]| nodes.iter().map(|n| n.label.clone()).collect::<Vec<_>>();
        assert_eq!(
            labels(&groups[0].children),
            [
                "Analog",
                "Connectors",
                "Diodes",
                "Logic",
                "Mechanical",
                "Passives",
                "Power",
                "Simulation",
                "Switches",
                "Transistors"
            ]
        );
        assert_eq!(groups[0].data.detail, lib.parts.len().to_string(), "the group counts its items");
        assert_eq!(labels(&groups[2].children), ["Mechanical", "SMD", "Through hole"]);
        let passives = &groups[0].children[5];
        assert_eq!(passives.id, NodeId::Category(Owner::Lib(7), Group::Parts, text_hash("Passives")));
        assert!(labels(&passives.children).starts_with(&["Capacitor".to_string()]));
        // Items keep their category as search keywords; the menu of a read-only folder is empty.
        assert_eq!(passives.children[0].keywords, "Passives");
        assert!(menu(passives.id, &passives.data).is_empty());
        let editable = Info { editable: true, category: "Passives".into(), ..Default::default() };
        assert!(acts(menu(passives.id, &editable)).contains(&Act::RenameCategory(7, Group::Parts, "Passives".into())));
    }

    #[test]
    fn simulations_list_the_analyses_and_count_the_outdated() {
        let mut setup = model::Simulation::default();
        let tran = model::Analysis::new(
            "Transient",
            model::AnalysisKind::Tran { stop: 1e-3, step: None, start: 0.0, max_step: None },
        );
        let op = model::Analysis::new("Bias", model::AnalysisKind::Op);
        setup.analyses = vec![tran.clone(), op.clone()];
        let node = simulation_node(&setup, |_| AnalysisDetail::default());
        assert_eq!(node.label, "Simulation");
        assert_eq!(node.data.detail, "2");
        let labels: Vec<&str> = node.children.iter().map(|n| n.label.as_str()).collect();
        assert_eq!(labels, ["Transient", "Bias"], "in the setup's order");
        assert_eq!(node.children[1].data.icon, Some(Icon::OperatingPoint));

        let stale = |a: &model::Analysis| AnalysisDetail {
            text: if a.id == op.id { OUTDATED.into() } else { "42 %".into() },
            warn: a.id == op.id,
        };
        let node = simulation_node(&setup, stale);
        assert_eq!((node.data.detail.as_str(), node.data.warn), ("1 outdated", true));
        assert_eq!(node.children[0].data.detail, "42 %");

        // Add simulation › one per kind, and Run all once there is one.
        let entries = menu(NodeId::Simulations, &node.data);
        let Some(Entry::Sub(_, kinds)) = entries.first() else { panic!("a submenu first") };
        assert_eq!(kinds.len(), crate::sim_ui::KINDS.len());
        assert!(acts(entries).contains(&Act::RunAllSimulations));
        let empty = simulation_node(&model::Simulation::default(), |_| AnalysisDetail::default());
        assert_eq!(empty.data.detail, "");
        assert!(!acts(menu(NodeId::Simulations, &empty.data)).contains(&Act::RunAllSimulations));
        assert!(matches!(plus_menu()[1], Entry::Sub("Add simulation", _)));

        // A click previews, a double click opens, Delete deletes.
        let a = NodeId::Analysis(op.id);
        assert_eq!(preview(a), Some(Act::PreviewAnalysis(op.id)));
        assert_eq!(activate(a, &Info::default()), Some(Act::OpenAnalysis(op.id)));
        assert!(acts(menu(a, &Info::default())).contains(&Act::DeleteAnalysis(op.id)));
    }

    #[test]
    fn uncategorised_items_follow_the_folders() {
        let item = |c: &str, name: &str| (c.to_string(), TreeNode::new(NodeId::Project, name, Info::default()));
        let out = categorized(vec![item("", "a"), item("B", "z"), item("A", "y"), item("B", "x")], |name, ch| {
            TreeNode::new(NodeId::Board, name, Info::default()).with_children(ch)
        });
        let flat: Vec<String> = out
            .iter()
            .map(|n| format!("{}{:?}", n.label, n.children.iter().map(|c| c.label.as_str()).collect::<Vec<_>>()))
            .collect();
        assert_eq!(flat, ["A[\"y\"]", "B[\"x\", \"z\"]", "a[]"]);
    }

    #[test]
    fn menus_follow_what_a_node_allows() {
        let builtin = Info { editable: false, ..Default::default() };
        assert_eq!(acts(menu(NodeId::Library(Owner::Lib(0)), &builtin)), [Act::SaveLibraryAs(0)]);
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

        // Project parts: no copy to the project, an update when the library changed.
        let changed = Info { editable: true, flag: true, ..Default::default() };
        let a = acts(menu(NodeId::Item(Owner::Project, part), &changed));
        assert!(a.iter().any(|a| matches!(a, Act::Update(Some(_)))));
        assert!(a.iter().any(|a| matches!(a, Act::OpenItem(PROJECT_LIB, _))));
        assert!(!a.iter().any(|a| matches!(a, Act::CopyToProject(..))));
        let lib = Info { editable: true, unused: 2, ..Default::default() };
        assert!(acts(menu(NodeId::Library(Owner::Project), &lib)).contains(&Act::RemoveUnused));
        assert_eq!(activate(NodeId::Board, &Info::default()), Some(Act::ShowView(ViewId::Board)));
        // The root sheet can't be renamed or deleted; sub-sheets can.
        let s = Id::new();
        let root = acts(menu(NodeId::Sheet(s), &Info::default()));
        assert!(root.contains(&Act::NewSheet(Some(s))) && !root.contains(&Act::DeleteSheet(s)));
        let sub = acts(menu(NodeId::Sheet(s), &Info { editable: true, ..Default::default() }));
        assert!(sub.contains(&Act::RenameSheet(s)) && sub.contains(&Act::DeleteSheet(s)));
        assert_eq!(activate(NodeId::Project, &Info::default()), None, "expands");
        assert_eq!(activate(NodeId::Library(Owner::Project), &Info::default()), None, "expands");
    }

    #[test]
    fn items_preview_on_selection_and_open_on_activation() {
        let part = Id::new();
        let node = NodeId::Item(Owner::Lib(3), ObjectId::LibPart(part));
        assert_eq!(preview(node), Some(Act::PreviewItem(3, Open::Part(part))));
        assert_eq!(activate(node, &Info::default()), Some(Act::OpenItem(3, Open::Part(part))));
        assert_eq!(preview(NodeId::Board), None);
        assert_eq!(preview(NodeId::Library(Owner::Project)), None);
    }

    #[test]
    fn sheets_drop_onto_other_sheets_only() {
        let [root, a, b, c] = [Id::new(), Id::new(), Id::new(), Id::new()];
        let node = |s: Id| TreeNode::new(NodeId::Sheet(s), "", Info::default());
        // root › a › b, root › c
        let roots = vec![
            TreeNode::new(NodeId::Project, "", Info::default())
                .with_children(vec![node(root).with_children(vec![node(a).with_children(vec![node(b)]), node(c)])]),
        ];
        assert!(can_drop_sheet(&roots, b, c));
        assert!(can_drop_sheet(&roots, b, root));
        assert!(can_drop_sheet(&roots, c, a));
        assert!(!can_drop_sheet(&roots, a, a), "onto itself");
        assert!(!can_drop_sheet(&roots, a, b), "into its own subtree");
        assert!(!can_drop_sheet(&roots, b, a), "where it already is");
    }
}
