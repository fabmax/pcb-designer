//! Incremental schematic connectivity.
//!
//! Nodes are pins, wires and labels. Two nodes are connected when they share a point,
//! when a point lies on the interior of a wire (T junction), or when two labels have the
//! same name. Nets are connected components of that graph.
//!
//! An update only rebuilds the nets that the edit can affect: nets of removed nodes, nets
//! of nodes adjacent to added nodes, and the added nodes themselves. All other nets keep
//! their ids and need no work.

use rustc_hash::{FxHashMap, FxHashSet};
use smallvec::SmallVec;

use crate::model::{MM, Pt, Slab};

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, PartialOrd, Ord)]
pub enum Node {
    Pin(u32, u16),
    Wire(u32),
    Label(u32),
}

#[derive(Clone, Debug)]
pub enum NodeDesc {
    Pin { comp: u32, pin: u16, pos: Pt },
    Wire { id: u32, a: Pt, b: Pt },
    Label { id: u32, pos: Pt, name: String },
}

impl NodeDesc {
    pub fn node(&self) -> Node {
        match *self {
            NodeDesc::Pin { comp, pin, .. } => Node::Pin(comp, pin),
            NodeDesc::Wire { id, .. } => Node::Wire(id),
            NodeDesc::Label { id, .. } => Node::Label(id),
        }
    }
}

/// Stable identity of a net, used by the board to key ratsnest and net assignments.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum NetKey {
    /// Lowest label name in the net.
    Named(String),
    /// Lowest pin in an unnamed net.
    Pin(u32, u16),
    /// Net without pins and labels (e.g. a dangling wire).
    Anon(Node),
}

#[derive(Clone, Debug)]
pub struct Net {
    pub key: NetKey,
    pub nodes: Vec<Node>,
    /// Sorted pins `(component, pin index)`.
    pub pins: Vec<(u32, u16)>,
}

#[derive(Default, Debug)]
pub struct NetChanges {
    /// Keys of nets that were dissolved (some may reappear in `added`).
    pub removed: Vec<NetKey>,
    /// Ids of newly built nets.
    pub added: Vec<u32>,
}

#[derive(Clone, Copy, Debug)]
enum Geom {
    Point(Pt),
    Seg(Pt, Pt),
}

const CELL: i64 = 16 * MM;

fn cell(p: Pt) -> (i32, i32) {
    (p.x.div_euclid(CELL) as i32, p.y.div_euclid(CELL) as i32)
}

fn cells_of_seg(a: Pt, b: Pt) -> impl Iterator<Item = (i32, i32)> {
    let (ca, cb) = (cell(a), cell(b));
    let (x0, x1) = (ca.0.min(cb.0), ca.0.max(cb.0));
    let (y0, y1) = (ca.1.min(cb.1), ca.1.max(cb.1));
    (x0..=x1).flat_map(move |x| (y0..=y1).map(move |y| (x, y)))
}

/// True if `p` lies strictly inside segment `a`–`b` (not on an endpoint).
fn on_interior(p: Pt, a: Pt, b: Pt) -> bool {
    if p == a || p == b {
        return false;
    }
    let (ab, ap) = (b - a, p - a);
    let cross = ab.x as i128 * ap.y as i128 - ab.y as i128 * ap.x as i128;
    if cross != 0 {
        return false;
    }
    let dot = ab.x as i128 * ap.x as i128 + ab.y as i128 * ap.y as i128;
    let len2 = ab.x as i128 * ab.x as i128 + ab.y as i128 * ab.y as i128;
    dot > 0 && dot < len2
}

#[derive(Default)]
pub struct Connectivity {
    geom: FxHashMap<Node, Geom>,
    /// Label id → interned name id.
    label_names: FxHashMap<u32, u32>,
    names: Vec<String>,
    name_ids: FxHashMap<String, u32>,
    points: FxHashMap<Pt, SmallVec<[Node; 4]>>,
    point_cells: FxHashMap<(i32, i32), Vec<Pt>>,
    wire_cells: FxHashMap<(i32, i32), Vec<u32>>,
    /// Name id → labels with that name. Same-name labels are connected, but treated as a
    /// star rather than a clique so huge nets (GND) stay linear to traverse.
    by_name: FxHashMap<u32, SmallVec<[u32; 4]>>,
    net_of: FxHashMap<Node, u32>,
    nets: Slab<Net>,
}

impl Connectivity {
    pub fn net(&self, id: u32) -> Option<&Net> {
        self.nets.get(id)
    }

    pub fn net_of(&self, node: Node) -> Option<u32> {
        self.net_of.get(&node).copied()
    }

    pub fn nets(&self) -> impl Iterator<Item = (u32, &Net)> {
        self.nets.iter()
    }

    pub fn net_count(&self) -> usize {
        self.nets.len()
    }

    fn index_add(&mut self, d: &NodeDesc) {
        let node = d.node();
        let add_point = |this: &mut Self, p: Pt| {
            let list = this.points.entry(p).or_default();
            if list.is_empty() {
                this.point_cells.entry(cell(p)).or_default().push(p);
            }
            list.push(node);
        };
        match d {
            NodeDesc::Pin { pos, .. } => {
                add_point(self, *pos);
                self.geom.insert(node, Geom::Point(*pos));
            }
            NodeDesc::Label { id, pos, name } => {
                add_point(self, *pos);
                self.geom.insert(node, Geom::Point(*pos));
                let name_id = match self.name_ids.get(name) {
                    Some(&n) => n,
                    None => {
                        self.names.push(name.clone());
                        self.name_ids.insert(name.clone(), (self.names.len() - 1) as u32);
                        (self.names.len() - 1) as u32
                    }
                };
                self.label_names.insert(*id, name_id);
                self.by_name.entry(name_id).or_default().push(*id);
            }
            NodeDesc::Wire { id, a, b } => {
                add_point(self, *a);
                if a != b {
                    add_point(self, *b);
                }
                for c in cells_of_seg(*a, *b) {
                    self.wire_cells.entry(c).or_default().push(*id);
                }
                self.geom.insert(node, Geom::Seg(*a, *b));
            }
        }
    }

    fn remove_point(&mut self, p: Pt, node: Node) {
        if let Some(list) = self.points.get_mut(&p) {
            list.retain(|n| *n != node);
            if list.is_empty() {
                self.points.remove(&p);
                if let Some(cell_pts) = self.point_cells.get_mut(&cell(p)) {
                    cell_pts.retain(|q| *q != p);
                }
            }
        }
    }

    fn index_remove(&mut self, node: Node) {
        let Some(g) = self.geom.remove(&node) else {
            return;
        };
        match g {
            Geom::Point(p) => self.remove_point(p, node),
            Geom::Seg(a, b) => {
                self.remove_point(a, node);
                if a != b {
                    self.remove_point(b, node);
                }
                if let Node::Wire(id) = node {
                    for c in cells_of_seg(a, b) {
                        if let Some(ws) = self.wire_cells.get_mut(&c) {
                            ws.retain(|w| *w != id);
                        }
                    }
                }
            }
        }
        if let Node::Label(id) = node
            && let Some(name) = self.label_names.remove(&id)
            && let Some(ids) = self.by_name.get_mut(&name)
        {
            ids.retain(|l| *l != id);
            if ids.is_empty() {
                self.by_name.remove(&name);
            }
        }
    }

    /// Nodes at `p` plus wires passing through `p`.
    fn nodes_touching(&self, p: Pt, skip: Node, out: &mut Vec<Node>) {
        if let Some(list) = self.points.get(&p) {
            out.extend(list.iter().copied().filter(|n| *n != skip));
        }
        if let Some(ws) = self.wire_cells.get(&cell(p)) {
            for &w in ws {
                if let Some(Geom::Seg(a, b)) = self.geom.get(&Node::Wire(w))
                    && on_interior(p, *a, *b)
                {
                    out.push(Node::Wire(w));
                }
            }
        }
    }

    fn neighbors(&self, node: Node, out: &mut Vec<Node>) {
        match self.geom.get(&node) {
            Some(Geom::Point(p)) => self.nodes_touching(*p, node, out),
            Some(Geom::Seg(a, b)) => {
                let (a, b) = (*a, *b);
                self.nodes_touching(a, node, out);
                self.nodes_touching(b, node, out);
                for c in cells_of_seg(a, b) {
                    if let Some(pts) = self.point_cells.get(&c) {
                        for &p in pts {
                            if on_interior(p, a, b) {
                                out.extend(self.points[&p].iter().copied());
                            }
                        }
                    }
                }
            }
            None => {}
        }
    }

    /// Other labels sharing the name of label `id`.
    fn same_name(&self, id: u32) -> impl Iterator<Item = Node> + '_ {
        let name = self.label_names.get(&id).copied();
        name.into_iter()
            .flat_map(move |n| self.by_name[&n].iter().copied())
            .filter(move |l| *l != id)
            .map(Node::Label)
    }

    /// Build all nets from scratch.
    pub fn build(nodes: impl IntoIterator<Item = NodeDesc>) -> (Self, NetChanges) {
        let mut c = Self::default();
        let mut seeds = Vec::new();
        for d in nodes {
            c.index_add(&d);
            seeds.push(d.node());
        }
        let added = c.flood(seeds);
        (
            c,
            NetChanges {
                removed: Vec::new(),
                added,
            },
        )
    }

    /// Apply removals and additions. A moved node is removed and added again.
    ///
    /// * Removals: the removed nodes' former neighbors seed an interleaved search inside
    ///   their net. Searches that meet are merged; a search that runs out of nodes has
    ///   found a part that split off. Cost is bounded by the smaller side(s) of a split,
    ///   and removing a leaf (one neighbor) costs nothing.
    /// * Additions: new nodes join the nets they touch; several touched nets are merged,
    ///   smaller ones into the largest.
    pub fn update(&mut self, removed: &[Node], added: &[NodeDesc]) -> NetChanges {
        let mut changed: FxHashSet<u32> = FxHashSet::default();
        let mut removed_keys = Vec::new();
        // Key of each changed net before this update.
        let mut old_keys: FxHashMap<u32, NetKey> = FxHashMap::default();

        // --- Removals --------------------------------------------------------------
        let removed_set: FxHashSet<Node> = removed.iter().copied().collect();
        let mut seeds_by_net: FxHashMap<u32, Vec<Node>> = FxHashMap::default();
        let mut nb = Vec::new();
        for &n in removed {
            let Some(net) = self.net_of.get(&n).copied() else {
                continue;
            };
            nb.clear();
            self.neighbors(n, &mut nb);
            if let Node::Label(id) = n {
                // Other same-name labels stay connected to each other; one represents them.
                nb.extend(self.same_name(id).find(|m| !removed_set.contains(m)));
            }
            let seeds = seeds_by_net.entry(net).or_default();
            seeds.extend(nb.iter().copied().filter(|m| !removed_set.contains(m)));
        }
        for &n in removed {
            if let Some(net) = self.net_of.remove(&n)
                && changed.insert(net)
            {
                old_keys.insert(net, self.nets.get(net).unwrap().key.clone());
            }
            self.index_remove(n);
        }
        for &net in seeds_by_net.keys() {
            self.nets
                .get_mut(net)
                .unwrap()
                .nodes
                .retain(|m| !removed_set.contains(m));
        }
        // Index additions now so that split checks see the final geometry. Added nodes are
        // unassigned (no net) and therefore ignored by the split search.
        for d in added {
            self.index_add(d);
        }
        let mut split_off = Vec::new();
        for (net, mut seeds) in seeds_by_net {
            seeds.sort_unstable();
            seeds.dedup();
            seeds.retain(|m| self.net_of.get(m) == Some(&net));
            split_off.extend(self.split(net, seeds));
        }
        changed.extend(split_off);

        // --- Additions -------------------------------------------------------------
        let mut stack = Vec::new();
        for d in added {
            let start = d.node();
            if self.net_of.contains_key(&start) {
                continue;
            }
            // Collect the connected group of new nodes and the existing nets it touches.
            let mut group = vec![start];
            let mut touched: Vec<u32> = Vec::new();
            let mut in_group: FxHashSet<Node> = FxHashSet::default();
            in_group.insert(start);
            stack.push(start);
            while let Some(n) = stack.pop() {
                nb.clear();
                self.neighbors(n, &mut nb);
                if let Node::Label(id) = n {
                    nb.extend(self.same_name(id));
                }
                for &m in &nb {
                    match self.net_of.get(&m) {
                        Some(&net) => {
                            if !touched.contains(&net) {
                                touched.push(net);
                            }
                        }
                        None => {
                            if self.geom.contains_key(&m) && in_group.insert(m) {
                                group.push(m);
                                stack.push(m);
                            }
                        }
                    }
                }
            }
            let target = match touched.iter().max_by_key(|&&id| self.nets.get(id).unwrap().nodes.len()) {
                Some(&t) => t,
                None => self.nets.insert(Net {
                    key: NetKey::Anon(start),
                    nodes: Vec::new(),
                    pins: Vec::new(),
                }),
            };
            for &other in touched.iter().filter(|&&t| t != target) {
                let net = self.nets.remove(other).unwrap();
                match old_keys.remove(&other) {
                    Some(k) => removed_keys.push(k),
                    None => removed_keys.push(net.key.clone()),
                }
                changed.remove(&other);
                for &m in &net.nodes {
                    self.net_of.insert(m, target);
                }
                self.nets.get_mut(target).unwrap().nodes.extend(net.nodes);
            }
            for &m in &group {
                self.net_of.insert(m, target);
            }
            if changed.insert(target)
                && !old_keys.contains_key(&target)
                && let Some(net) = self.nets.get(target)
                && !net.nodes.is_empty()
            {
                old_keys.insert(target, net.key.clone());
            }
            self.nets.get_mut(target).unwrap().nodes.extend(group);
        }

        // --- Refresh keys and pin lists of changed nets ------------------------------
        let mut added_ids = Vec::new();
        for id in changed {
            let empty = self.nets.get(id).is_none_or(|n| n.nodes.is_empty());
            if let Some(k) = old_keys.remove(&id) {
                removed_keys.push(k);
            }
            if empty {
                self.nets.remove(id);
                continue;
            }
            self.refresh(id);
            added_ids.push(id);
        }
        NetChanges {
            removed: removed_keys,
            added: added_ids,
        }
    }

    /// Interleaved search from `seeds` (all in `net`). Parts that turn out disconnected
    /// from the rest become new nets. Returns the ids of the new nets.
    fn split(&mut self, net: u32, seeds: Vec<Node>) -> Vec<u32> {
        let k = seeds.len();
        if k <= 1 {
            return Vec::new();
        }
        fn find(uf: &mut [usize], mut i: usize) -> usize {
            while uf[i] != i {
                uf[i] = uf[uf[i]];
                i = uf[i];
            }
            i
        }
        let mut uf: Vec<usize> = (0..k).collect();
        let mut retired = vec![false; k];
        let mut owner: FxHashMap<Node, usize> = FxHashMap::default();
        let mut frontier: Vec<Vec<Node>> = Vec::with_capacity(k);
        let mut members: Vec<Vec<Node>> = Vec::with_capacity(k);
        let mut expanded: Vec<FxHashSet<u32>> = vec![FxHashSet::default(); k];
        for (i, &s) in seeds.iter().enumerate() {
            owner.insert(s, i);
            frontier.push(vec![s]);
            members.push(vec![s]);
        }
        let mut groups = k;
        let mut new_nets = Vec::new();
        let mut nb = Vec::new();
        while groups > 1 {
            for i in 0..k {
                if groups == 1 {
                    break;
                }
                if retired[i] || find(&mut uf, i) != i {
                    continue; // split off already, or merged into another group
                }
                let Some(n) = frontier[i].pop() else {
                    // Exhausted: this group is a separate component.
                    let nodes = std::mem::take(&mut members[i]);
                    let new_id = self.nets.insert(Net {
                        key: NetKey::Anon(nodes[0]),
                        nodes: Vec::new(),
                        pins: Vec::new(),
                    });
                    let moved: FxHashSet<Node> = nodes.iter().copied().collect();
                    for &m in &nodes {
                        self.net_of.insert(m, new_id);
                    }
                    self.nets.get_mut(net).unwrap().nodes.retain(|m| !moved.contains(m));
                    self.nets.get_mut(new_id).unwrap().nodes = nodes;
                    retired[i] = true;
                    new_nets.push(new_id);
                    groups -= 1;
                    continue;
                };
                nb.clear();
                self.neighbors(n, &mut nb);
                if let Node::Label(id) = n
                    && expanded[i].insert(self.label_names[&id])
                {
                    nb.extend(self.same_name(id));
                }
                for &m in &nb {
                    if self.net_of.get(&m) != Some(&net) {
                        continue;
                    }
                    match owner.get(&m).copied() {
                        None => {
                            owner.insert(m, i);
                            frontier[i].push(m);
                            members[i].push(m);
                        }
                        Some(j) => {
                            let rj = find(&mut uf, j);
                            debug_assert!(!retired[rj], "split-off parts are disconnected");
                            if rj != i {
                                // Two searches met: merge group rj into i.
                                uf[rj] = i;
                                let f = std::mem::take(&mut frontier[rj]);
                                frontier[i].extend(f);
                                let mm = std::mem::take(&mut members[rj]);
                                members[i].extend(mm);
                                let ex = std::mem::take(&mut expanded[rj]);
                                expanded[i].extend(ex);
                                groups -= 1;
                            }
                        }
                    }
                }
            }
        }
        new_nets
    }

    /// Recompute key and pin list of a net from its nodes.
    fn refresh(&mut self, id: u32) {
        let net = self.nets.get(id).unwrap();
        let mut pins: Vec<(u32, u16)> = net
            .nodes
            .iter()
            .filter_map(|n| if let Node::Pin(c, p) = n { Some((*c, *p)) } else { None })
            .collect();
        pins.sort_unstable();
        let name = net
            .nodes
            .iter()
            .filter_map(|n| {
                if let Node::Label(l) = n {
                    self.label_names.get(l)
                } else {
                    None
                }
            })
            .map(|&name_id| &self.names[name_id as usize])
            .min();
        let key = match (name, pins.first()) {
            (Some(name), _) => NetKey::Named(name.clone()),
            (None, Some(&(c, p))) => NetKey::Pin(c, p),
            (None, None) => NetKey::Anon(*net.nodes.iter().min().unwrap()),
        };
        let net = self.nets.get_mut(id).unwrap();
        net.key = key;
        net.pins = pins;
    }

    /// Group all unassigned `seeds` (and everything reachable from them) into new nets.
    fn flood(&mut self, seeds: Vec<Node>) -> Vec<u32> {
        let mut new_nets = Vec::new();
        let mut stack = Vec::new();
        let mut nb = Vec::new();
        let mut expanded_names: FxHashSet<u32> = FxHashSet::default();
        for seed in seeds {
            if self.net_of.contains_key(&seed) || !self.geom.contains_key(&seed) {
                continue;
            }
            let net_id = self.nets.insert(Net {
                key: NetKey::Anon(seed),
                nodes: Vec::new(),
                pins: Vec::new(),
            });
            let mut nodes = Vec::new();
            self.net_of.insert(seed, net_id);
            stack.push(seed);
            while let Some(n) = stack.pop() {
                nodes.push(n);
                nb.clear();
                self.neighbors(n, &mut nb);
                if let Node::Label(id) = n
                    && expanded_names.insert(self.label_names[&id])
                {
                    nb.extend(self.same_name(id));
                }
                for &m in &nb {
                    if let std::collections::hash_map::Entry::Vacant(e) = self.net_of.entry(m) {
                        e.insert(net_id);
                        stack.push(m);
                    } else {
                        debug_assert_eq!(self.net_of[&m], net_id, "net invariant violated");
                    }
                }
            }
            self.nets.get_mut(net_id).unwrap().nodes = nodes;
            self.refresh(net_id);
            new_nets.push(net_id);
        }
        new_nets
    }

    /// Check that incremental state equals a from-scratch build (for tests).
    pub fn partition(&self) -> Vec<Vec<Node>> {
        let mut parts: Vec<Vec<Node>> = self
            .nets
            .iter()
            .map(|(_, n)| {
                let mut v = n.nodes.clone();
                v.sort_unstable();
                v
            })
            .collect();
        parts.sort_unstable();
        parts
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pin(comp: u32, x: f64, y: f64) -> NodeDesc {
        NodeDesc::Pin {
            comp,
            pin: 0,
            pos: Pt::mm(x, y),
        }
    }

    fn wire(id: u32, a: (f64, f64), b: (f64, f64)) -> NodeDesc {
        NodeDesc::Wire {
            id,
            a: Pt::mm(a.0, a.1),
            b: Pt::mm(b.0, b.1),
        }
    }

    #[test]
    fn wire_connects_pins_and_t_junction() {
        let (c, _) = Connectivity::build([
            pin(0, 0.0, 0.0),
            pin(1, 10.0, 0.0),
            wire(0, (0.0, 0.0), (10.0, 0.0)),
            // T junction: wire ending on the interior of wire 0.
            wire(1, (5.0, 0.0), (5.0, 5.0)),
            pin(2, 5.0, 5.0),
            // Crossing wire without endpoint on wire 0: not connected.
            wire(2, (7.0, -5.0), (7.0, 5.0)),
        ]);
        assert_eq!(c.net_of(Node::Pin(0, 0)), c.net_of(Node::Pin(2, 0)));
        assert_ne!(c.net_of(Node::Pin(0, 0)), c.net_of(Node::Wire(2)));
    }

    #[test]
    fn labels_join_by_name() {
        let (c, _) = Connectivity::build([
            pin(0, 0.0, 0.0),
            NodeDesc::Label {
                id: 0,
                pos: Pt::mm(0.0, 0.0),
                name: "GND".into(),
            },
            pin(1, 50.0, 50.0),
            NodeDesc::Label {
                id: 1,
                pos: Pt::mm(50.0, 50.0),
                name: "GND".into(),
            },
        ]);
        let net = c.net(c.net_of(Node::Pin(0, 0)).unwrap()).unwrap();
        assert_eq!(net.key, NetKey::Named("GND".into()));
        assert_eq!(net.pins, vec![(0, 0), (1, 0)]);
    }

    #[test]
    fn incremental_matches_full_rebuild() {
        let base = vec![
            pin(0, 0.0, 0.0),
            pin(1, 10.0, 0.0),
            pin(2, 20.0, 0.0),
            wire(0, (0.0, 0.0), (10.0, 0.0)),
            wire(1, (10.0, 0.0), (20.0, 0.0)),
        ];
        let (mut inc, _) = Connectivity::build(base.clone());
        // Remove the middle wire's link: split.
        inc.update(&[Node::Wire(1)], &[]);
        let (full, _) = Connectivity::build(base[..4].to_vec());
        assert_eq!(inc.partition(), full.partition());
        // Add a long wire across that touches pin 1 on its interior: merge everything.
        let w = wire(5, (0.0, 0.0), (20.0, 0.0));
        inc.update(&[], std::slice::from_ref(&w));
        let mut all = base[..4].to_vec();
        all.push(w);
        let (full, _) = Connectivity::build(all);
        assert_eq!(inc.partition(), full.partition());
        assert_eq!(inc.net_count(), 1);
    }
}
