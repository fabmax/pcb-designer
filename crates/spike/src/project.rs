//! Synthetic project, edits and the live schematic → board sync chain.

use std::hash::{Hash, Hasher};
use std::time::{Duration, Instant};

use render::Scene;
use rustc_hash::{FxHashMap, FxHashSet, FxHasher};

use crate::connectivity::{Connectivity, NetChanges, NetKey, Node, NodeDesc};
use crate::draw::{self, PcbLayers, Prims, SchLayers, ids};
use crate::library::{self, standard_parts};
use crate::model::*;
use crate::ratsnest;

/// Small deterministic RNG (xorshift64*), so the spike needs no extra dependency.
#[derive(Clone)]
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Self(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1)
    }
    pub fn next_u64(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    pub fn below(&mut self, n: u64) -> u64 {
        self.next_u64() % n.max(1)
    }
    pub fn chance(&mut self, p: f64) -> bool {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64 <= p
    }
}

#[derive(Clone, Debug)]
pub enum Edit {
    AddWire {
        a: Pt,
        b: Pt,
    },
    RemoveWire(u32),
    AddLabel {
        pos: Pt,
        name: String,
    },
    /// Add a component in the staging area and connect both pins via labels.
    AddComponent {
        part: u16,
        labels: [String; 2],
    },
    RemoveComponent(u32),
    MoveComponent {
        id: u32,
        delta: Pt,
    },
}

impl Edit {
    pub fn kind(&self) -> &'static str {
        match self {
            Edit::AddWire { .. } => "add wire",
            Edit::RemoveWire(_) => "remove wire",
            Edit::AddLabel { .. } => "add label",
            Edit::AddComponent { .. } => "add component",
            Edit::RemoveComponent(_) => "remove component",
            Edit::MoveComponent { .. } => "move component",
        }
    }
}

/// Timing of one edit through the whole chain.
#[derive(Clone, Debug, Default)]
pub struct SyncReport {
    pub kind: &'static str,
    /// Schematic model + schematic scene update.
    pub schematic: Duration,
    pub connectivity: Duration,
    /// Footprint add/remove + PCB scene update.
    pub board: Duration,
    /// Ratsnest recomputation + PCB scene update.
    pub ratsnest: Duration,
    pub total: Duration,
    pub nets_removed: usize,
    pub nets_added: usize,
    pub largest_net_pads: usize,
}

#[derive(Clone, Debug, Default)]
pub struct GenStats {
    pub components: usize,
    pub pins: usize,
    pub wires: usize,
    pub labels: usize,
    pub nets: usize,
    pub tracks: usize,
    pub vias: usize,
    pub sch_instances: usize,
    pub pcb_instances: usize,
    pub t_generate: Duration,
    pub t_connectivity: Duration,
    pub t_scenes: Duration,
    pub t_ratsnest: Duration,
}

pub struct Project {
    pub lib: Vec<PartDef>,
    pub sch: Schematic,
    pub board: Board,
    pub conn: Connectivity,
    /// Copper island per `(component, pad)`. In the spike, islands are known by construction
    /// (tracks are generated between pads). The real editor derives them geometrically.
    pad_island: FxHashMap<(u32, u16), u32>,
    next_island: u32,
    pub sch_scene: Scene,
    pub pcb_scene: Scene,
    sl: SchLayers,
    pl: PcbLayers,
    sch_staging: Pt,
    pcb_staging: Pt,
    refdes_next: Vec<u32>,
    pub rng: Rng,
}

fn key_hash(k: &NetKey) -> u64 {
    let mut h = FxHasher::default();
    k.hash(&mut h);
    h.finish()
}

impl Project {
    fn pin_pos(&self, comp: &SchComponent, pin: usize) -> Pt {
        comp.pos + self.lib[comp.part as usize].pins[pin].pos.rotated(comp.rot)
    }

    fn pin_nodes(&self, id: u32) -> Vec<NodeDesc> {
        let c = self.sch.components.get(id).unwrap();
        (0..self.lib[c.part as usize].pins.len())
            .map(|p| NodeDesc::Pin {
                comp: id,
                pin: p as u16,
                pos: self.pin_pos(c, p),
            })
            .collect()
    }

    fn next_refdes(&mut self, part: u16) -> String {
        let n = &mut self.refdes_next[part as usize];
        *n += 1;
        format!("{}{}", self.lib[part as usize].prefix, n)
    }

    /// Generate a synthetic project with roughly `n` components.
    pub fn generate(n: usize, seed: u64) -> (Self, GenStats) {
        let t0 = Instant::now();
        let lib = standard_parts();
        let mut sch_scene = Scene::new();
        let mut pcb_scene = Scene::new();
        let sl = SchLayers::create(&mut sch_scene);
        let pl = PcbLayers::create(&mut pcb_scene);
        let mut p = Project {
            refdes_next: vec![0; lib.len()],
            lib,
            sch: Schematic::default(),
            board: Board::default(),
            conn: Connectivity::default(),
            pad_island: FxHashMap::default(),
            next_island: 0,
            sch_scene,
            pcb_scene,
            sl,
            pl,
            sch_staging: Pt::default(),
            pcb_staging: Pt::default(),
            rng: Rng::new(seed),
        };

        // Part mix.
        let mix = [
            (library::PART_R, 0.30),
            (library::PART_C, 0.25),
            (library::PART_SOIC8, 0.30),
            (library::PART_QFP48, 0.10),
            (library::PART_HDR10, 0.05),
        ];
        let row_len = ((n as f64).sqrt().ceil() as usize).max(4);
        let g = Pt::mm(2.54, 0.0).x;
        let mut y = 0;
        let mut order = Vec::with_capacity(n);
        for (part, frac) in mix {
            let count = ((n as f64) * frac).round() as usize;
            let def = p.lib[part as usize].clone();
            let span_x = def.pins.iter().map(|q| q.pos.x.abs()).max().unwrap_or(0) * 2;
            let pitch_x = span_x + 4 * g;
            let row_h = (def.body.1.y - def.body.0.y) + 6 * g;
            // Pins facing each other between neighbors in a row (same y).
            let facing: Vec<(usize, usize)> = (0..def.pins.len())
                .filter(|&r| def.pins[r].pos.x > 0)
                .filter_map(|r| {
                    (0..def.pins.len())
                        .find(|&l| def.pins[l].pos.x < 0 && def.pins[l].pos.y == def.pins[r].pos.y)
                        .map(|l| (r, l))
                })
                .collect();

            let mut placed = 0;
            let mut row = 0;
            while placed < count {
                let in_row = row_len.min(count - placed);
                let ids: Vec<u32> = (0..in_row)
                    .map(|k| {
                        let refdes = p.next_refdes(part);
                        p.sch.components.insert(SchComponent {
                            part,
                            refdes,
                            pos: Pt::new(k as Nm * pitch_x, y - row_h / 2),
                            rot: 0,
                        })
                    })
                    .collect();
                order.extend_from_slice(&ids);

                let mut wired: FxHashSet<(u32, usize)> = FxHashSet::default();
                for k in 0..in_row.saturating_sub(1) {
                    for &(r, l) in &facing {
                        if p.rng.chance(0.5) {
                            let a = p.pin_pos(p.sch.components.get(ids[k]).unwrap(), r);
                            let b = p.pin_pos(p.sch.components.get(ids[k + 1]).unwrap(), l);
                            p.sch.wires.insert(Wire { a, b });
                            wired.insert((ids[k], r));
                            wired.insert((ids[k + 1], l));
                        }
                    }
                }
                let pool = (in_row * 2) as u64;
                for &id in &ids {
                    for pin in 0..def.pins.len() {
                        if wired.contains(&(id, pin)) {
                            continue;
                        }
                        let a = p.pin_pos(p.sch.components.get(id).unwrap(), pin);
                        let out = if def.pins[pin].pos.x > 0 { g } else { -g };
                        let b = Pt::new(a.x + out, a.y);
                        p.sch.wires.insert(Wire { a, b });
                        let name = if p.rng.chance(0.15) {
                            "GND".to_string()
                        } else if p.rng.chance(0.08) {
                            "VCC".to_string()
                        } else {
                            format!("P{part}R{row}N{}", p.rng.below(pool))
                        };
                        p.sch.labels.insert(Label { pos: b, name });
                    }
                }
                placed += in_row;
                row += 1;
                y -= row_h;
            }
        }
        p.sch_staging = Pt::new(0, y - 20 * g);

        // Board: flow layout of footprints in schematic order, roughly square.
        let area: f64 = order
            .iter()
            .map(|&id| {
                let bp = p.lib[p.sch.components.get(id).unwrap().part as usize]
                    .board_pitch
                    .to_mm64();
                bp[0] * bp[1]
            })
            .sum();
        let width = Pt::mm(area.sqrt() * 1.05, 0.0).x;
        let (mut x, mut y, mut row_h) = (0, 0, 0);
        for &id in &order {
            let bp = p.lib[p.sch.components.get(id).unwrap().part as usize].board_pitch;
            if x + bp.x > width {
                x = 0;
                y -= row_h;
                row_h = 0;
            }
            p.board.footprints.insert(
                id,
                BoardFootprint {
                    pos: Pt::new(x + bp.x / 2, y - bp.y / 2),
                    rot: 0,
                },
            );
            x += bp.x;
            row_h = row_h.max(bp.y);
        }
        let margin = Pt::mm(5.0, 5.0).x;
        p.board.outline = (Pt::new(-margin, y - row_h - margin), Pt::new(width + margin, margin));
        p.pcb_staging = Pt::new(width + 4 * margin, 0);
        let t_generate = t0.elapsed();

        // Connectivity.
        let t1 = Instant::now();
        let mut nodes = Vec::new();
        for (id, _) in p.sch.components.iter() {
            nodes.extend(p.pin_nodes(id));
        }
        for (id, w) in p.sch.wires.iter() {
            nodes.push(NodeDesc::Wire { id, a: w.a, b: w.b });
        }
        for (id, l) in p.sch.labels.iter() {
            nodes.push(NodeDesc::Label {
                id,
                pos: l.pos,
                name: l.name.clone(),
            });
        }
        let pins = nodes.iter().filter(|n| matches!(n, NodeDesc::Pin { .. })).count();
        let (conn, _) = Connectivity::build(nodes);
        p.conn = conn;
        let t_connectivity = t1.elapsed();

        // Tracks for small nets, so some ratsnest lines are already routed.
        let small_nets: Vec<Vec<(u32, u16)>> = p
            .conn
            .nets()
            .filter(|(_, n)| (2..=4).contains(&n.pins.len()))
            .map(|(_, n)| n.pins.clone())
            .collect();
        for pins in small_nets {
            if !p.rng.chance(0.6) {
                continue;
            }
            let mut pads: Vec<((u32, u16), Pt)> = pins.iter().map(|&(c, pin)| ((c, pin), p.pad_pos(c, pin))).collect();
            pads.sort_by_key(|(_, pos)| (pos.x, pos.y));
            let layer = if p.rng.chance(0.8) {
                CopperLayer::Top
            } else {
                CopperLayer::Bottom
            };
            // Pads joined by consecutive routed segments share a copper island.
            let mut island = p.new_island();
            p.set_island(pads[0].0, island);
            for w in pads.windows(2) {
                let (a, b) = (w[0].1, w[1].1);
                if (a.x - b.x).abs() > 30 * MM || (a.y - b.y).abs() > 30 * MM {
                    island = p.new_island();
                    p.set_island(w[1].0, island);
                    continue;
                }
                let (dx, dy) = (b.x - a.x, b.y - a.y);
                let corner = if dx.abs() > dy.abs() {
                    Pt::new(b.x - dx.signum() * dy.abs(), a.y)
                } else {
                    Pt::new(a.x, b.y - dy.signum() * dx.abs())
                };
                let width = Pt::mm(0.2, 0.0).x;
                p.board.tracks.insert(Track {
                    a,
                    b: corner,
                    width,
                    layer,
                });
                p.board.tracks.insert(Track {
                    a: corner,
                    b,
                    width,
                    layer,
                });
                if layer == CopperLayer::Bottom {
                    for pos in [a, b] {
                        p.board.vias.insert(Via {
                            pos,
                            diameter: Pt::mm(0.6, 0.0).x,
                            drill: Pt::mm(0.3, 0.0).x,
                        });
                    }
                }
                p.set_island(w[1].0, island);
            }
        }

        // Scenes.
        let t2 = Instant::now();
        let mut prims = Prims::new();
        for (id, c) in p.sch.components.iter() {
            prims.clear();
            draw::sch_component(&p.lib[c.part as usize], c, &p.sl, &mut prims);
            p.sch_scene.set_item(ids::component(id), prims.drain(..));
        }
        for (id, w) in p.sch.wires.iter() {
            draw::sch_wire(w, &p.sl, &mut prims);
            p.sch_scene.set_item(ids::wire(id), prims.drain(..));
        }
        for (id, l) in p.sch.labels.iter() {
            draw::sch_label(l, &p.sl, &mut prims);
            p.sch_scene.set_item(ids::label(id), prims.drain(..));
        }
        let fp_ids: Vec<u32> = p.board.footprints.keys().copied().collect();
        for id in fp_ids {
            p.draw_footprint(id, &mut prims);
        }
        for (id, t) in p.board.tracks.iter() {
            draw::pcb_track(t, &p.pl, &mut prims);
            p.pcb_scene.set_item(ids::track(id), prims.drain(..));
        }
        for (id, v) in p.board.vias.iter() {
            draw::pcb_via(v, &p.pl, &mut prims);
            p.pcb_scene.set_item(ids::via(id), prims.drain(..));
        }
        draw::pcb_outline(p.board.outline.0, p.board.outline.1, &p.pl, &mut prims);
        p.pcb_scene.set_item(ids::outline(), prims.drain(..));
        let t_scenes = t2.elapsed();

        let t3 = Instant::now();
        let net_ids: Vec<u32> = p.conn.nets().map(|(id, _)| id).collect();
        for id in net_ids {
            p.update_ratsnest(id, &mut prims);
        }
        let t_ratsnest = t3.elapsed();

        let stats = GenStats {
            components: p.sch.components.len(),
            pins,
            wires: p.sch.wires.len(),
            labels: p.sch.labels.len(),
            nets: p.conn.net_count(),
            tracks: p.board.tracks.len(),
            vias: p.board.vias.len(),
            sch_instances: p.sch_scene.instance_count(),
            pcb_instances: p.pcb_scene.instance_count(),
            t_generate,
            t_connectivity,
            t_scenes,
            t_ratsnest,
        };
        (p, stats)
    }

    fn new_island(&mut self) -> u32 {
        self.next_island += 1;
        self.next_island
    }

    fn set_island(&mut self, (comp, pin): (u32, u16), island: u32) {
        let part = self.sch.components.get(comp).unwrap().part;
        let pad = self.lib[part as usize].pin_to_pad[pin as usize];
        self.pad_island.insert((comp, pad), island);
    }

    fn pad_pos(&self, comp: u32, pin: u16) -> Pt {
        let c = self.sch.components.get(comp).unwrap();
        let part = &self.lib[c.part as usize];
        let fp = self.board.footprints[&comp];
        fp.pos + part.pads[part.pin_to_pad[pin as usize] as usize].pos.rotated(fp.rot)
    }

    fn draw_footprint(&mut self, id: u32, prims: &mut Prims) {
        let c = self.sch.components.get(id).unwrap();
        let fp = self.board.footprints[&id];
        prims.clear();
        draw::pcb_footprint(&self.lib[c.part as usize], &fp, &c.refdes, &self.pl, prims);
        self.pcb_scene.set_item(ids::footprint(id), prims.drain(..));
    }

    /// Recompute and redraw the ratsnest of net `net_id`. Returns its pad count.
    fn update_ratsnest(&mut self, net_id: u32, prims: &mut Prims) -> usize {
        let net = self.conn.net(net_id).unwrap();
        let item = ids::ratsnest(key_hash(&net.key));
        let mut pts = Vec::with_capacity(net.pins.len());
        let mut islands = Vec::with_capacity(net.pins.len());
        for (i, &(c, pin)) in net.pins.iter().enumerate() {
            if !self.board.footprints.contains_key(&c) {
                continue;
            }
            pts.push(self.pad_pos(c, pin).to_mm64());
            let pad = self.lib[self.sch.components.get(c).unwrap().part as usize].pin_to_pad[pin as usize];
            // Pads without an island get a unique one.
            islands.push(self.pad_island.get(&(c, pad)).copied().unwrap_or(u32::MAX - i as u32));
        }
        let lines = ratsnest::mst(&pts, &islands);
        prims.clear();
        for (a, b) in &lines {
            let (pa, pb) = (pts[*a], pts[*b]);
            draw::ratsnest_line(
                [pa[0] as f32, pa[1] as f32],
                [pb[0] as f32, pb[1] as f32],
                &self.pl,
                prims,
            );
        }
        if prims.is_empty() {
            self.pcb_scene.remove_item(item);
        } else {
            self.pcb_scene.set_item(item, prims.drain(..));
        }
        pts.len()
    }

    /// Pick a random edit that makes sense for the current project.
    pub fn random_edit(&mut self) -> Edit {
        let pick_comp = |p: &mut Self| loop {
            let id = p.rng.below(p.sch.components.capacity_ids() as u64) as u32;
            if p.sch.components.get(id).is_some() {
                return id;
            }
        };
        match self.rng.below(6) {
            0 => {
                // Wire between two pins of neighboring components.
                let a_id = pick_comp(self);
                let b_id = loop {
                    let b = pick_comp(self);
                    if b != a_id {
                        break b;
                    }
                };
                let (ca, cb) = (
                    self.sch.components.get(a_id).unwrap(),
                    self.sch.components.get(b_id).unwrap(),
                );
                let pa = self.rng.below(self.lib[ca.part as usize].pins.len() as u64) as usize;
                let pb = self.rng.below(self.lib[cb.part as usize].pins.len() as u64) as usize;
                Edit::AddWire {
                    a: self.pin_pos(ca, pa),
                    b: self.pin_pos(cb, pb),
                }
            }
            1 => loop {
                let id = self.rng.below(self.sch.wires.capacity_ids() as u64) as u32;
                if self.sch.wires.get(id).is_some() {
                    break Edit::RemoveWire(id);
                }
            },
            2 => {
                let id = pick_comp(self);
                let c = self.sch.components.get(id).unwrap();
                let pin = self.rng.below(self.lib[c.part as usize].pins.len() as u64) as usize;
                Edit::AddLabel {
                    pos: self.pin_pos(c, pin),
                    name: "GND".into(),
                }
            }
            3 => {
                let other = loop {
                    let id = self.rng.below(self.sch.labels.capacity_ids() as u64) as u32;
                    if let Some(l) = self.sch.labels.get(id) {
                        break l.name.clone();
                    }
                };
                Edit::AddComponent {
                    part: library::PART_R,
                    labels: ["GND".into(), other],
                }
            }
            4 => Edit::RemoveComponent(pick_comp(self)),
            _ => Edit::MoveComponent {
                id: pick_comp(self),
                delta: Pt::mm(0.0, 2.54),
            },
        }
    }

    /// Apply a schematic edit and propagate it live to the board.
    pub fn apply(&mut self, edit: Edit) -> SyncReport {
        let kind = edit.kind();
        let t0 = Instant::now();
        let mut prims = Prims::new();
        let mut removed: Vec<Node> = Vec::new();
        let mut added: Vec<NodeDesc> = Vec::new();
        let mut comps_added = Vec::new();
        let mut comps_removed = Vec::new();

        match edit {
            Edit::AddWire { a, b } => {
                let w = Wire { a, b };
                let id = self.sch.wires.insert(w);
                draw::sch_wire(&w, &self.sl, &mut prims);
                self.sch_scene.set_item(ids::wire(id), prims.drain(..));
                added.push(NodeDesc::Wire { id, a, b });
            }
            Edit::RemoveWire(id) => {
                if self.sch.wires.remove(id).is_some() {
                    self.sch_scene.remove_item(ids::wire(id));
                    removed.push(Node::Wire(id));
                }
            }
            Edit::AddLabel { pos, name } => {
                let lab = Label {
                    pos,
                    name: name.clone(),
                };
                let id = self.sch.labels.insert(lab.clone());
                draw::sch_label(&lab, &self.sl, &mut prims);
                self.sch_scene.set_item(ids::label(id), prims.drain(..));
                added.push(NodeDesc::Label { id, pos, name });
            }
            Edit::AddComponent { part, labels } => {
                let refdes = self.next_refdes(part);
                let pos = self.sch_staging;
                self.sch_staging.x += Pt::mm(20.32, 0.0).x;
                let id = self.sch.components.insert(SchComponent {
                    part,
                    refdes,
                    pos,
                    rot: 0,
                });
                let c = self.sch.components.get(id).unwrap();
                draw::sch_component(&self.lib[part as usize], c, &self.sl, &mut prims);
                self.sch_scene.set_item(ids::component(id), prims.drain(..));
                added.extend(self.pin_nodes(id));
                for (pin, name) in labels.into_iter().enumerate() {
                    let lpos = self.pin_pos(self.sch.components.get(id).unwrap(), pin);
                    let lab = Label {
                        pos: lpos,
                        name: name.clone(),
                    };
                    let lid = self.sch.labels.insert(lab.clone());
                    draw::sch_label(&lab, &self.sl, &mut prims);
                    self.sch_scene.set_item(ids::label(lid), prims.drain(..));
                    added.push(NodeDesc::Label {
                        id: lid,
                        pos: lpos,
                        name,
                    });
                }
                comps_added.push(id);
            }
            Edit::RemoveComponent(id) => {
                if let Some(c) = self.sch.components.get(id) {
                    let n = self.lib[c.part as usize].pins.len();
                    removed.extend((0..n).map(|p| Node::Pin(id, p as u16)));
                    self.sch_scene.remove_item(ids::component(id));
                    comps_removed.push(id);
                }
            }
            Edit::MoveComponent { id, delta } => {
                if self.sch.components.get(id).is_some() {
                    let c = self.sch.components.get_mut(id).unwrap();
                    c.pos = c.pos + delta;
                    let c = self.sch.components.get(id).unwrap();
                    draw::sch_component(&self.lib[c.part as usize], c, &self.sl, &mut prims);
                    self.sch_scene.set_item(ids::component(id), prims.drain(..));
                    let nodes = self.pin_nodes(id);
                    removed.extend(nodes.iter().map(|n| n.node()));
                    added.extend(nodes);
                }
            }
        }
        let t1 = Instant::now();

        let changes: NetChanges = self.conn.update(&removed, &added);
        let t2 = Instant::now();

        // Board: footprints follow schematic components.
        for &id in &comps_removed {
            self.board.footprints.remove(&id);
            self.pcb_scene.remove_item(ids::footprint(id));
            self.sch.components.remove(id);
        }
        for &id in &comps_added {
            self.board.footprints.insert(
                id,
                BoardFootprint {
                    pos: self.pcb_staging,
                    rot: 0,
                },
            );
            self.pcb_staging.y -= Pt::mm(3.0, 0.0).x;
            self.draw_footprint(id, &mut prims);
        }
        let t3 = Instant::now();

        // Ratsnest for affected nets only.
        let added_keys: FxHashSet<u64> = changes
            .added
            .iter()
            .map(|&id| key_hash(&self.conn.net(id).unwrap().key))
            .collect();
        for key in &changes.removed {
            let h = key_hash(key);
            if !added_keys.contains(&h) {
                self.pcb_scene.remove_item(ids::ratsnest(h));
            }
        }
        let mut largest = 0;
        for &id in &changes.added {
            largest = largest.max(self.update_ratsnest(id, &mut prims));
        }
        let t4 = Instant::now();

        SyncReport {
            kind,
            schematic: t1 - t0,
            connectivity: t2 - t1,
            board: t3 - t2,
            ratsnest: t4 - t3,
            total: t4 - t0,
            nets_removed: changes.removed.len(),
            nets_added: changes.added.len(),
            largest_net_pads: largest,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// After many random incremental edits, connectivity equals a full rebuild.
    #[test]
    fn incremental_sync_matches_rebuild() {
        for seed in 1..=4 {
            check_incremental(seed);
        }
    }

    fn check_incremental(seed: u64) {
        let (mut p, _) = Project::generate(400, seed);
        for _ in 0..800 {
            let e = p.random_edit();
            p.apply(e);
        }
        let mut nodes = Vec::new();
        for (id, _) in p.sch.components.iter() {
            nodes.extend(p.pin_nodes(id));
        }
        for (id, w) in p.sch.wires.iter() {
            nodes.push(NodeDesc::Wire { id, a: w.a, b: w.b });
        }
        for (id, l) in p.sch.labels.iter() {
            nodes.push(NodeDesc::Label {
                id,
                pos: l.pos,
                name: l.name.clone(),
            });
        }
        let (full, _) = Connectivity::build(nodes);
        assert_eq!(p.conn.partition(), full.partition());
        // Every component has a footprint and vice versa.
        assert_eq!(p.sch.components.len(), p.board.footprints.len());
    }
}
