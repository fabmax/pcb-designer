//! A tree view's state, independent of the GUI toolkit: expansion, selection, keyboard
//! navigation and the search filter. The GUI builds the nodes each frame, asks for the
//! visible [`Row`]s and paints them.
//!
//! **Search** filters: only nodes whose label contains the query (ignoring case) and their
//! ancestors are shown. Ancestors are expanded while searching, and matches show as
//! leaves. ↑/↓ move between matches. Closing the search restores the expansion from before
//! it, plus whatever is needed to keep the selection visible.

use std::collections::HashSet;
use std::hash::Hash;
use std::ops::Range;

/// A node: `id` must be unique in the tree. `data` is whatever the GUI needs to paint it.
#[derive(Clone, Debug)]
pub struct Node<K, T> {
    pub id: K,
    pub label: String,
    pub data: T,
    pub children: Vec<Node<K, T>>,
}

impl<K, T> Node<K, T> {
    pub fn new(id: K, label: impl Into<String>, data: T) -> Self {
        Node { id, label: label.into(), data, children: Vec::new() }
    }

    pub fn with_children(mut self, children: Vec<Node<K, T>>) -> Self {
        self.children = children;
        self
    }
}

/// A visible line of the tree.
#[derive(Debug)]
pub struct Row<'a, K, T> {
    pub node: &'a Node<K, T>,
    pub depth: usize,
    /// Has children that can be shown (draw a chevron).
    pub expandable: bool,
    pub expanded: bool,
    /// While searching: where the query is in the label (bytes). `None` for ancestors
    /// shown only because a descendant matches.
    pub matched: Option<Range<usize>>,
    pub parent: Option<K>,
}

/// Keys the tree reacts to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TreeKey {
    Up,
    Down,
    /// Collapse, else go to the parent.
    Left,
    /// Expand, else go to the first child.
    Right,
    Home,
    End,
    Enter,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TreeEvent<K> {
    /// Enter or double-click.
    Activate(K),
}

#[derive(Clone, Debug)]
struct Search<K> {
    query: String,
    /// Expansion before the search started.
    before: HashSet<K>,
}

#[derive(Clone, Debug)]
pub struct TreeState<K> {
    pub expanded: HashSet<K>,
    pub selected: Option<K>,
    search: Option<Search<K>>,
}

impl<K> Default for TreeState<K> {
    fn default() -> Self {
        TreeState { expanded: HashSet::new(), selected: None, search: None }
    }
}

impl<K: Copy + Eq + Hash> TreeState<K> {
    // ------------------------------------------------------------------------------------
    // Rows

    /// The visible rows, top to bottom.
    pub fn rows<'a, T>(&self, roots: &'a [Node<K, T>]) -> Vec<Row<'a, K, T>> {
        let mut out = Vec::new();
        match self.query() {
            Some(q) => {
                let q = q.to_lowercase();
                for n in roots {
                    self.filtered(n, 0, None, &q, &mut out);
                }
            }
            None => {
                for n in roots {
                    self.plain(n, 0, None, &mut out);
                }
            }
        }
        out
    }

    fn plain<'a, T>(&self, n: &'a Node<K, T>, depth: usize, parent: Option<K>, out: &mut Vec<Row<'a, K, T>>) {
        let expanded = !n.children.is_empty() && self.expanded.contains(&n.id);
        out.push(Row { node: n, depth, expandable: !n.children.is_empty(), expanded, matched: None, parent });
        if expanded {
            for c in &n.children {
                self.plain(c, depth + 1, Some(n.id), out);
            }
        }
    }

    /// Adds `n` (and the matching part of its subtree) if anything in it matches.
    fn filtered<'a, T>(
        &self,
        n: &'a Node<K, T>,
        depth: usize,
        parent: Option<K>,
        q: &str,
        out: &mut Vec<Row<'a, K, T>>,
    ) -> bool {
        let at = out.len();
        let mut below = false;
        for c in &n.children {
            below |= self.filtered(c, depth + 1, Some(n.id), q, out);
        }
        let matched = find(&n.label, q);
        if !below && matched.is_none() {
            return false;
        }
        out.insert(at, Row { node: n, depth, expandable: below, expanded: below, matched, parent });
        true
    }

    // ------------------------------------------------------------------------------------
    // Expansion and selection

    pub fn is_expanded(&self, id: K) -> bool {
        self.expanded.contains(&id)
    }

    pub fn set_expanded(&mut self, id: K, open: bool) {
        if open {
            self.expanded.insert(id);
        } else {
            self.expanded.remove(&id);
        }
    }

    pub fn toggle(&mut self, id: K) {
        let open = !self.is_expanded(id);
        self.set_expanded(id, open);
    }

    pub fn collapse_all(&mut self) {
        self.expanded.clear();
    }

    pub fn select(&mut self, id: K) {
        self.selected = Some(id);
    }

    /// Select `id` and expand its ancestors so it is visible. False if it isn't in the tree.
    pub fn reveal<T>(&mut self, roots: &[Node<K, T>], id: K) -> bool {
        let mut path = Vec::new();
        if !path_to(roots, id, &mut path) {
            return false;
        }
        self.expanded.extend(path);
        self.selected = Some(id);
        true
    }

    // ------------------------------------------------------------------------------------
    // Keyboard

    /// React to a key on the given (current) rows.
    pub fn key<T>(&mut self, rows: &[Row<K, T>], key: TreeKey) -> Option<TreeEvent<K>> {
        let searching = self.query().is_some();
        // While searching, ↑/↓ step through the matches only.
        let stops: Vec<usize> = (0..rows.len()).filter(|&i| !searching || rows[i].matched.is_some()).collect();
        let current = self.selected.and_then(|s| rows.iter().position(|r| r.node.id == s));
        let pos = current.and_then(|c| stops.iter().position(|&i| i == c));
        let go = |i: Option<&usize>| i.map(|&i| rows[i].node.id);
        let target = match key {
            TreeKey::Down => match (pos, current) {
                (Some(p), _) => go(stops.get(p + 1)),
                (None, Some(c)) => go(stops.iter().find(|&&i| i > c)),
                (None, None) => go(stops.first()),
            },
            TreeKey::Up => match (pos, current) {
                (Some(p), _) => go(p.checked_sub(1).and_then(|p| stops.get(p))),
                (None, Some(c)) => go(stops.iter().rev().find(|&&i| i < c)),
                (None, None) => go(stops.last()),
            },
            TreeKey::Home => go(stops.first()),
            TreeKey::End => go(stops.last()),
            TreeKey::Left => {
                let row = &rows[current?];
                if row.expanded && !searching {
                    self.set_expanded(row.node.id, false);
                    None
                } else {
                    row.parent
                }
            }
            TreeKey::Right => {
                let c = current?;
                let row = &rows[c];
                if row.expandable && !row.expanded && !searching {
                    self.set_expanded(row.node.id, true);
                    None
                } else if row.expanded {
                    rows.get(c + 1).map(|r| r.node.id)
                } else {
                    None
                }
            }
            TreeKey::Enter => return self.selected.map(TreeEvent::Activate),
        };
        if let Some(t) = target {
            self.selected = Some(t);
        }
        None
    }

    // ------------------------------------------------------------------------------------
    // Search

    /// The search query, if the search is open (it may be empty).
    pub fn query(&self) -> Option<&str> {
        self.search.as_ref().map(|s| s.query.as_str()).filter(|q| !q.is_empty())
    }

    pub fn is_searching(&self) -> bool {
        self.search.is_some()
    }

    /// Open the search (keeps a query that is already there).
    pub fn open_search(&mut self) {
        if self.search.is_none() {
            self.search = Some(Search { query: String::new(), before: self.expanded.clone() });
        }
    }

    /// The query, for editing (opens the search).
    pub fn query_mut(&mut self) -> &mut String {
        self.open_search();
        &mut self.search.as_mut().unwrap().query
    }

    /// Set the query; the selection moves to the first match if it isn't one.
    pub fn set_query<T>(&mut self, roots: &[Node<K, T>], query: &str) {
        *self.query_mut() = query.to_string();
        self.select_first_match(roots);
    }

    /// After the query changed: keep the selection if it still matches, else take the first
    /// match.
    pub fn select_first_match<T>(&mut self, roots: &[Node<K, T>]) {
        let rows = self.rows(roots);
        let matches = |id: K| rows.iter().any(|r| r.node.id == id && r.matched.is_some());
        if self.selected.is_some_and(matches) {
            return;
        }
        if let Some(r) = rows.iter().find(|r| r.matched.is_some()) {
            self.selected = Some(r.node.id);
        }
    }

    /// Close the search: the expansion from before it, plus the selection's ancestors.
    pub fn close_search<T>(&mut self, roots: &[Node<K, T>]) {
        let Some(search) = self.search.take() else { return };
        self.expanded = search.before;
        if let Some(s) = self.selected {
            let mut path = Vec::new();
            if path_to(roots, s, &mut path) {
                self.expanded.extend(path);
            }
        }
    }
}

/// Case-insensitive position of `lower_query` in `label` (byte range in `label`).
fn find(label: &str, lower_query: &str) -> Option<Range<usize>> {
    let lower = label.to_lowercase();
    let start = lower.find(lower_query)?;
    // Lowercasing may change byte lengths (rare); fall back to highlighting nothing.
    if lower.len() != label.len() {
        return Some(0..0);
    }
    Some(start..start + lower_query.len())
}

/// The ids of `id`'s ancestors (root first) in `path`. False if `id` isn't in the tree.
fn path_to<K: Copy + Eq, T>(nodes: &[Node<K, T>], id: K, path: &mut Vec<K>) -> bool {
    for n in nodes {
        if n.id == id {
            return true;
        }
        path.push(n.id);
        if path_to(&n.children, id, path) {
            return true;
        }
        path.pop();
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    type N = Node<u32, ()>;

    fn n(id: u32, label: &str, children: Vec<N>) -> N {
        Node::new(id, label, ()).with_children(children)
    }

    /// project(1): schematic(2), library(3): parts(4): Resistor(5), Capacitor(6)
    /// external(7): mine(8): parts(9): Regulator(10)
    fn tree() -> Vec<N> {
        vec![
            n(
                1,
                "project",
                vec![
                    n(2, "Schematic", vec![]),
                    n(
                        3,
                        "Project Library",
                        vec![n(4, "Parts", vec![n(5, "Resistor", vec![]), n(6, "Capacitor", vec![])])],
                    ),
                ],
            ),
            n(7, "External Libraries", vec![n(8, "Mine", vec![n(9, "Parts", vec![n(10, "Regulator", vec![])])])]),
        ]
    }

    fn ids(t: &TreeState<u32>, roots: &[N]) -> Vec<u32> {
        t.rows(roots).iter().map(|r| r.node.id).collect()
    }

    #[test]
    fn rows_follow_expansion() {
        let roots = tree();
        let mut t = TreeState::default();
        assert_eq!(ids(&t, &roots), [1, 7]);
        t.toggle(1);
        assert_eq!(ids(&t, &roots), [1, 2, 3, 7]);
        let rows = t.rows(&roots);
        assert!(rows[0].expanded && rows[0].expandable);
        assert!(!rows[1].expandable, "a leaf");
        assert_eq!((rows[2].depth, rows[2].parent), (1, Some(1)));
        t.collapse_all();
        assert_eq!(ids(&t, &roots), [1, 7]);
        assert!(t.reveal(&roots, 10));
        assert_eq!(ids(&t, &roots), [1, 7, 8, 9, 10]);
        assert_eq!(t.selected, Some(10));
        assert!(!t.reveal(&roots, 99));
    }

    #[test]
    fn arrow_keys_move_expand_and_collapse() {
        let roots = tree();
        let mut t = TreeState::default();
        let press = |t: &mut TreeState<u32>, k| {
            let rows = t.rows(&roots);
            t.key(&rows, k)
        };
        press(&mut t, TreeKey::Down);
        assert_eq!(t.selected, Some(1), "the first row");
        press(&mut t, TreeKey::Right);
        assert!(t.is_expanded(1));
        press(&mut t, TreeKey::Right);
        assert_eq!(t.selected, Some(2), "into the first child");
        press(&mut t, TreeKey::Down);
        press(&mut t, TreeKey::Right);
        press(&mut t, TreeKey::Right);
        assert_eq!(t.selected, Some(4));
        press(&mut t, TreeKey::Left);
        assert_eq!(t.selected, Some(3), "a collapsed node goes to its parent");
        press(&mut t, TreeKey::Left);
        assert!(!t.is_expanded(3), "an expanded node collapses");
        press(&mut t, TreeKey::End);
        assert_eq!(t.selected, Some(7));
        press(&mut t, TreeKey::Down);
        assert_eq!(t.selected, Some(7), "stays at the end");
        press(&mut t, TreeKey::Home);
        assert_eq!(t.selected, Some(1));
        press(&mut t, TreeKey::Up);
        assert_eq!(t.selected, Some(1));
        assert_eq!(press(&mut t, TreeKey::Enter), Some(TreeEvent::Activate(1)));
    }

    #[test]
    fn search_filters_and_restores_expansion() {
        let roots = tree();
        let mut t = TreeState::default();
        t.set_expanded(7, true);
        t.set_query(&roots, "res");
        let rows = t.rows(&roots);
        let shown: Vec<u32> = rows.iter().map(|r| r.node.id).collect();
        assert_eq!(shown, [1, 3, 4, 5]);
        assert!(rows.iter().take(3).all(|r| r.expanded && r.matched.is_none()), "ancestors open");
        assert_eq!(rows[3].matched, Some(0..3));
        assert_eq!(t.selected, Some(5), "the first match is selected");

        // ↑/↓ step through matches only.
        t.set_query(&roots, "OR");
        let rows = t.rows(&roots);
        let matches: Vec<u32> = rows.iter().filter(|r| r.matched.is_some()).map(|r| r.node.id).collect();
        assert_eq!(matches, [5, 6, 10]);
        assert_eq!(t.selected, Some(5));
        t.key(&rows, TreeKey::Down);
        assert_eq!(t.selected, Some(6));
        t.key(&rows, TreeKey::Down);
        assert_eq!(t.selected, Some(10), "skips the ancestors in between");
        t.key(&rows, TreeKey::Up);
        assert_eq!(t.selected, Some(6));
        assert_eq!(rows.iter().find(|r| r.node.id == 6).unwrap().matched, Some(7..9), "ignoring case");

        // Closing restores the expansion, and keeps the selection visible.
        t.close_search(&roots);
        assert!(!t.is_searching());
        assert_eq!(ids(&t, &roots), [1, 2, 3, 4, 5, 6, 7, 8]);
    }

    #[test]
    fn an_empty_query_shows_the_whole_tree() {
        let roots = tree();
        let mut t = TreeState::default();
        t.open_search();
        assert!(t.is_searching());
        assert_eq!(t.query(), None);
        assert_eq!(ids(&t, &roots), [1, 7]);
        t.set_query(&roots, "zzz");
        assert!(t.rows(&roots).is_empty());
    }
}
