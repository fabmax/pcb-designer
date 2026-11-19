//! The per-project workspace: the editor tabs and their splits, each view's camera and
//! hidden layers, the library item tabs' cameras and recency, and the routing layer. Saved in the project folder as
//! `.pcb-designer/workspace.json`; the folder ignores itself for git (a `.gitignore`
//! with `*`). It is written on save, when the project closes and on quit, and is never
//! part of the project's edits or undo.

use std::path::{Path, PathBuf};

use egui_tiles::{Container, LinearDir, Tile, TileId, Tiles, Tree};
use model::{Id, LayerId};
use serde::{Deserialize, Serialize};
use view::ViewId;

/// A tab, by what it shows (library item tabs by library file, as their keys change every
/// run).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TabRef {
    Sheet(Id),
    Board,
    Library {
        lib: LibRef,
        item: ItemRef,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        preview: bool,
    },
    Gerber,
    Simulation,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LibRef {
    Project,
    Builtin,
    File(PathBuf),
}

/// The item a library item tab shows.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ItemRef {
    Symbol(Id),
    Footprint(Id),
    Part(Id),
}

/// The tab tree: tab groups, and splits with their shares.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Node {
    Tabs { tabs: Vec<TabRef>, active: Option<usize> },
    Split { vertical: bool, shares: Vec<f32>, children: Vec<Node> },
}

/// A schematic sheet's or the board's camera and hidden layers (by name).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ViewState {
    pub view: TabRef,
    pub center: [f64; 2],
    pub zoom: f64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hidden: Vec<String>,
}

/// A library item tab's camera.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ItemState {
    pub lib: LibRef,
    pub item: ItemRef,
    pub center: [f64; 2],
    pub zoom: f64,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Workspace {
    pub layout: Option<Node>,
    /// The tab worked on last.
    pub active: Option<TabRef>,
    pub views: Vec<ViewState>,
    /// The library item tabs, the least recently used first.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub items: Vec<ItemState>,
    /// The board's active copper layer.
    pub layer: Option<LayerId>,
}

/// Where a project folder's workspace lives.
pub fn path(project: &Path) -> PathBuf {
    project.join(".pcb-designer").join("workspace.json")
}

impl Workspace {
    /// The saved workspace of a project folder; `None` if there is none or it can't be read.
    pub fn load(project: &Path) -> Option<Workspace> {
        let json = std::fs::read_to_string(path(project)).ok()?;
        serde_json::from_str(&json).map_err(|e| log::warn!("ignoring workspace of {}: {e}", project.display())).ok()
    }

    pub fn save(&self, project: &Path) -> Result<(), String> {
        let file = path(project);
        let dir = file.parent().expect("in a folder");
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        let ignore = dir.join(".gitignore");
        if !ignore.exists() {
            std::fs::write(&ignore, "*\n").map_err(|e| format!("{}: {e}", ignore.display()))?;
        }
        let json = serde_json::to_string_pretty(self).expect("workspace serializes");
        std::fs::write(&file, json).map_err(|e| format!("{}: {e}", file.display()))
    }
}

pub fn view_ref(view: ViewId) -> TabRef {
    match view {
        ViewId::Sheet(s) => TabRef::Sheet(s),
        ViewId::Board => TabRef::Board,
    }
}

pub fn view_of(tab: &TabRef) -> Option<ViewId> {
    match tab {
        TabRef::Sheet(s) => Some(ViewId::Sheet(*s)),
        TabRef::Board => Some(ViewId::Board),
        _ => None,
    }
}

/// The tab tree as saved; tabs `name` gives no reference for are left out.
pub fn layout_of<T>(tree: &Tree<T>, name: &dyn Fn(&T) -> Option<TabRef>) -> Option<Node> {
    node_of(&tree.tiles, tree.root?, name)
}

fn node_of<T>(tiles: &Tiles<T>, id: TileId, name: &dyn Fn(&T) -> Option<TabRef>) -> Option<Node> {
    match tiles.get(id)? {
        Tile::Pane(p) => Some(Node::Tabs { tabs: vec![name(p)?], active: Some(0) }),
        Tile::Container(Container::Tabs(t)) => {
            let mut tabs = Vec::new();
            let mut active = None;
            for &c in &t.children {
                let Some(Tile::Pane(p)) = tiles.get(c) else { continue };
                let Some(r) = name(p) else { continue };
                if t.active == Some(c) {
                    active = Some(tabs.len());
                }
                tabs.push(r);
            }
            (!tabs.is_empty()).then_some(Node::Tabs { tabs, active })
        }
        Tile::Container(Container::Linear(l)) => {
            let mut children = Vec::new();
            let mut shares = Vec::new();
            for &c in &l.children {
                if let Some(n) = node_of(tiles, c, name) {
                    children.push(n);
                    shares.push(l.shares[c]);
                }
            }
            match children.len() {
                0 => None,
                1 => children.pop(),
                _ => Some(Node::Split { vertical: l.dir == LinearDir::Vertical, shares, children }),
            }
        }
        Tile::Container(Container::Grid(g)) => {
            let children: Vec<Node> = g.children().filter_map(|&c| node_of(tiles, c, name)).collect();
            let shares = vec![1.0; children.len()];
            match children.len() {
                0 => None,
                1 => children.into_iter().next(),
                _ => Some(Node::Split { vertical: false, shares, children }),
            }
        }
    }
}

/// A tab tree from a saved layout; tabs `open` can't resolve are left out. `None` if
/// nothing is left.
pub fn tree_of<T: Clone + PartialEq>(
    node: &Node,
    id: egui::Id,
    open: &mut dyn FnMut(&TabRef) -> Option<T>,
) -> Option<Tree<T>> {
    let mut tiles = Tiles::default();
    let mut seen = Vec::new();
    let root = insert(&mut tiles, node, open, &mut seen)?;
    Some(Tree::new(id, root, tiles))
}

fn insert<T: Clone + PartialEq>(
    tiles: &mut Tiles<T>,
    node: &Node,
    open: &mut dyn FnMut(&TabRef) -> Option<T>,
    seen: &mut Vec<T>,
) -> Option<TileId> {
    match node {
        Node::Tabs { tabs, active } => {
            let mut children = Vec::new();
            let mut front = None;
            for (i, r) in tabs.iter().enumerate() {
                // A tab shows once (a hand-edited file might repeat one).
                let Some(t) = open(r).filter(|t| !seen.contains(t)) else { continue };
                seen.push(t.clone());
                let id = tiles.insert_pane(t);
                if *active == Some(i) {
                    front = Some(id);
                }
                children.push(id);
            }
            if children.is_empty() {
                return None;
            }
            let id = tiles.insert_tab_tile(children);
            if let (Some(f), Some(Tile::Container(Container::Tabs(t)))) = (front, tiles.get_mut(id)) {
                t.set_active(f);
            }
            Some(id)
        }
        Node::Split { vertical, shares, children } => {
            let mut ids = Vec::new();
            let mut kept = Vec::new();
            for (i, c) in children.iter().enumerate() {
                if let Some(id) = insert(tiles, c, open, seen) {
                    ids.push(id);
                    kept.push(shares.get(i).copied().filter(|s| s.is_finite() && *s > 0.0).unwrap_or(1.0));
                }
            }
            match ids.len() {
                0 => None,
                1 => ids.pop(),
                _ => {
                    let id = if *vertical {
                        tiles.insert_vertical_tile(ids.clone())
                    } else {
                        tiles.insert_horizontal_tile(ids.clone())
                    };
                    if let Some(Tile::Container(Container::Linear(l))) = tiles.get_mut(id) {
                        for (c, s) in ids.into_iter().zip(kept) {
                            l.shares.set_share(c, s);
                        }
                    }
                    Some(id)
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHEET: Id = Id::from_u128(1);

    #[derive(Clone, Copy, Debug, PartialEq)]
    enum T {
        Sch,
        Pcb,
        Lib(u64),
    }

    fn name(t: &T) -> Option<TabRef> {
        match t {
            T::Sch => Some(TabRef::Sheet(SHEET)),
            T::Pcb => Some(TabRef::Board),
            T::Lib(1) => Some(TabRef::Library { lib: LibRef::Builtin, item: ItemRef::Part(SHEET), preview: true }),
            T::Lib(_) => None,
        }
    }

    fn open(r: &TabRef) -> Option<T> {
        match r {
            TabRef::Sheet(_) => Some(T::Sch),
            TabRef::Board => Some(T::Pcb),
            TabRef::Library { lib: LibRef::Builtin, .. } => Some(T::Lib(1)),
            _ => None,
        }
    }

    #[test]
    fn splits_tabs_and_shares_round_trip() {
        let mut tiles = Tiles::default();
        let sch = tiles.insert_pane(T::Sch);
        let lib = tiles.insert_pane(T::Lib(1));
        let gone = tiles.insert_pane(T::Lib(2));
        let pcb = tiles.insert_pane(T::Pcb);
        let left = tiles.insert_tab_tile(vec![sch, gone, lib]);
        let right = tiles.insert_tab_tile(vec![pcb]);
        let root = tiles.insert_horizontal_tile(vec![left, right]);
        if let Some(Tile::Container(Container::Linear(l))) = tiles.get_mut(root) {
            l.shares.set_share(left, 3.0);
        }
        if let Some(Tile::Container(Container::Tabs(t))) = tiles.get_mut(left) {
            t.set_active(lib);
        }
        let tree = Tree::new(egui::Id::unique("t"), root, tiles);
        let node = layout_of(&tree, &name).unwrap();
        let Node::Split { vertical: false, shares, children } = &node else { panic!("{node:?}") };
        assert_eq!(shares, &[3.0, 1.0]);
        assert_eq!(
            children[0],
            Node::Tabs { tabs: vec![TabRef::Sheet(SHEET), name(&T::Lib(1)).unwrap()], active: Some(1) }
        );

        // Through JSON and back into a tree.
        let node: Node = serde_json::from_str(&serde_json::to_string(&node).unwrap()).unwrap();
        let back = tree_of(&node, egui::Id::unique("t2"), &mut open).unwrap();
        assert_eq!(layout_of(&back, &name).unwrap(), node);
        let front: Vec<T> = back.active_tiles().into_iter().filter_map(|t| back.tiles.get_pane(&t).copied()).collect();
        assert_eq!(front, [T::Lib(1), T::Pcb]);
    }

    #[test]
    fn missing_tabs_collapse_and_nothing_left_is_none() {
        let node = Node::Split {
            vertical: true,
            shares: vec![1.0, 1.0],
            children: vec![
                Node::Tabs { tabs: vec![TabRef::Gerber], active: Some(0) },
                Node::Tabs { tabs: vec![TabRef::Board, TabRef::Board], active: Some(1) },
            ],
        };
        let tree = tree_of(&node, egui::Id::unique("t"), &mut open).unwrap();
        assert_eq!(layout_of(&tree, &name).unwrap(), Node::Tabs { tabs: vec![TabRef::Board], active: Some(0) });
        let only_gerber = Node::Tabs { tabs: vec![TabRef::Gerber], active: None };
        assert!(tree_of(&only_gerber, egui::Id::unique("t"), &mut open).is_none());
    }

    #[test]
    fn saved_with_a_self_ignoring_folder() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(Workspace::load(dir.path()), None);
        let ws = Workspace { layer: Some(LayerId::BOTTOM_COPPER), ..Default::default() };
        ws.save(dir.path()).unwrap();
        assert_eq!(Workspace::load(dir.path()), Some(ws));
        let ignore = std::fs::read_to_string(dir.path().join(".pcb-designer/.gitignore")).unwrap();
        assert_eq!(ignore, "*\n");
        std::fs::write(path(dir.path()), "{ broken").unwrap();
        assert_eq!(Workspace::load(dir.path()), None);
    }
}
