//! Schematic and board views: incremental scene building, highlights and hit testing.

use editor_core::ChangeSet;
use editor_core::copper::{BoardConnectivity, Geo};
use editor_core::query::{footprint_of_instance, symbol_of_instance};
use model::*;
use render::{Color, Instance, Scene};
use rustc_hash::{FxHashMap, FxHashSet, FxHasher};
use std::hash::{Hash, Hasher};

use crate::draw::{Painter, mmf, substitute};
use crate::index::SpatialIndex;
use crate::pcb_tools::PcbGhost;
use crate::sch_tools::SchGhost;
use crate::style::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ViewKind {
    Schematic,
    Board,
}

impl ViewKind {
    /// Does this view show objects of `kind`?
    pub fn shows(self, kind: ObjectKind) -> bool {
        match self {
            ViewKind::Schematic => matches!(
                kind,
                ObjectKind::Symbol | ObjectKind::Wire | ObjectKind::Junction | ObjectKind::Label | ObjectKind::SchText
            ),
            ViewKind::Board => kind.is_board(),
        }
    }
}

type RLayer = render::LayerId;

struct SchLayers {
    body_fill: RLayer,
    body: RLayer,
    pins: RLayer,
    pin_text: RLayer,
    wires: RLayer,
    labels: RLayer,
    text: RLayer,
}

struct BoardLayers {
    map: FxHashMap<LayerId, RLayer>,
    copper: Vec<LayerId>,
    vias: RLayer,
    holes: RLayer,
    pad_numbers: RLayer,
    ratsnest: RLayer,
}

enum Layers {
    Sch(SchLayers),
    Board(BoardLayers),
}

pub struct View {
    pub kind: ViewKind,
    pub scene: Scene,
    pub index: SpatialIndex,
    layers: Layers,
    hover_layer: RLayer,
    selection_layer: RLayer,
    painter: Painter,
    selection: FxHashSet<ObjectId>,
    hover: Option<ObjectId>,
    /// Highlighted nets (board), by name.
    nets: Vec<String>,
    /// Positions of design rule violations (board).
    drc: Vec<Point>,
    /// Cross-probed objects.
    probed: Vec<ObjectId>,
}

fn item(id: ObjectId) -> u64 {
    let mut h = FxHasher::default();
    id.hash(&mut h);
    h.finish() & !(3 << 62)
}

/// Scene item of the connectivity markers (junction dots, unconnected pins).
const CONNECTIVITY_ITEM: u64 = 3 << 62;
/// Scene item of the creation tool preview.
const PREVIEW_ITEM: u64 = (3 << 62) | 1;
/// Scene item of the highlighted nets.
const NET_ITEM: u64 = (3 << 62) | 2;
/// Scene item of cross-probed objects.
const PROBE_ITEM: u64 = (3 << 62) | 3;
/// Design rule violation markers.
const DRC_ITEM: u64 = (3 << 62) | 4;

fn overlay_item(id: ObjectId, hover: bool) -> u64 {
    item(id) | if hover { 2 << 62 } else { 1 << 62 }
}

impl View {
    pub fn new(kind: ViewKind, project: &Project) -> Self {
        let (scene, layers, hover_layer, selection_layer) = Self::create_scene(kind, project);
        let mut v = View {
            kind,
            scene,
            index: SpatialIndex::default(),
            layers,
            hover_layer,
            selection_layer,
            painter: Painter::default(),
            selection: FxHashSet::default(),
            hover: None,
            nets: Vec::new(),
            drc: Vec::new(),
            probed: Vec::new(),
        };
        v.draw_all(project);
        v
    }

    fn create_scene(kind: ViewKind, project: &Project) -> (Scene, Layers, RLayer, RLayer) {
        let mut scene = Scene::new();
        let layers = match kind {
            ViewKind::Schematic => {
                let l = SchLayers {
                    body_fill: scene.add_layer("Symbol fill"),
                    body: scene.add_layer("Symbols"),
                    pins: scene.add_layer("Pins"),
                    pin_text: scene.add_layer("Pin numbers"),
                    wires: scene.add_layer("Wires"),
                    labels: scene.add_layer("Labels"),
                    text: scene.add_layer("Text"),
                };
                scene.layer_mut(l.pin_text).min_zoom = 3.0;
                Layers::Sch(l)
            }
            ViewKind::Board => {
                let board = &project.board;
                let name = |id: LayerId| {
                    board.layers.iter().find(|l| l.id == id).map(|l| l.name.clone()).unwrap_or(format!("{id:?}"))
                };
                let mut map = FxHashMap::default();
                let mut add = |scene: &mut Scene, id: LayerId| {
                    let r = scene.add_layer(name(id));
                    map.insert(id, r);
                    r
                };
                for id in [
                    LayerId::BOTTOM_FAB,
                    LayerId::BOTTOM_COURTYARD,
                    LayerId::BOTTOM_PASTE,
                    LayerId::BOTTOM_MASK,
                    LayerId::BOTTOM_SILK,
                ] {
                    add(&mut scene, id);
                }
                let mut copper: Vec<LayerId> = board.copper_layers().map(|l| l.id).collect();
                copper.sort();
                for &id in copper.iter().rev() {
                    add(&mut scene, id);
                }
                let vias = scene.add_layer("Vias");
                let holes = scene.add_layer("Holes");
                for id in [
                    LayerId::TOP_MASK,
                    LayerId::TOP_PASTE,
                    LayerId::TOP_SILK,
                    LayerId::TOP_FAB,
                    LayerId::TOP_COURTYARD,
                    LayerId::OUTLINE,
                ] {
                    add(&mut scene, id);
                }
                let pad_numbers = scene.add_layer("Pad numbers");
                scene.layer_mut(pad_numbers).min_zoom = 25.0;
                let ratsnest = scene.add_layer("Ratsnest");
                for id in [LayerId::TOP_MASK, LayerId::BOTTOM_MASK, LayerId::TOP_PASTE, LayerId::BOTTOM_PASTE] {
                    scene.layer_mut(map[&id]).visible = false;
                }
                Layers::Board(BoardLayers { map, copper, vias, holes, pad_numbers, ratsnest })
            }
        };
        let hover = scene.add_layer("Hover");
        let selection = scene.add_layer("Selection");
        (scene, layers, hover, selection)
    }

    /// Number of render layers that are user facing (overlays excluded).
    pub fn user_layers(&self) -> std::ops::Range<u16> {
        0..self.hover_layer
    }

    /// Recreate everything (after library or layer stack changes).
    pub fn rebuild(&mut self, project: &Project) {
        let visible: FxHashMap<String, bool> = (0..self.scene.layer_count() as u16)
            .map(|i| (self.scene.layer(i).name.clone(), self.scene.layer(i).visible))
            .collect();
        let (scene, layers, hover, selection) = Self::create_scene(self.kind, project);
        self.scene = scene;
        for i in 0..self.scene.layer_count() as u16 {
            let l = self.scene.layer_mut(i);
            if let Some(&v) = visible.get(&l.name) {
                l.visible = v;
            }
        }
        self.layers = layers;
        self.hover_layer = hover;
        self.selection_layer = selection;
        self.index.clear();
        self.draw_all(project);
        let sel: Vec<ObjectId> = self.selection.drain().collect();
        let hover = self.hover.take();
        self.set_highlight(project, &sel, hover);
        self.draw_drc();
    }

    fn draw_all(&mut self, project: &Project) {
        let ids: Vec<ObjectId> = match self.kind {
            ViewKind::Schematic => {
                let s = &project.schematic;
                s.symbols
                    .ids()
                    .map(ObjectId::Symbol)
                    .chain(s.wires.ids().map(ObjectId::Wire))
                    .chain(s.junctions.ids().map(ObjectId::Junction))
                    .chain(s.labels.ids().map(ObjectId::Label))
                    .chain(s.texts.ids().map(ObjectId::SchText))
                    .collect()
            }
            ViewKind::Board => {
                let b = &project.board;
                b.footprints
                    .ids()
                    .map(ObjectId::Footprint)
                    .chain(b.tracks.ids().map(ObjectId::Track))
                    .chain(b.vias.ids().map(ObjectId::Via))
                    .chain(b.zones.ids().map(ObjectId::Zone))
                    .chain(b.graphics.ids().map(ObjectId::BoardGraphic))
                    .chain(b.texts.ids().map(ObjectId::BoardText))
                    .collect()
            }
        };
        for id in ids {
            self.redraw(project, id);
        }
    }

    /// Update after a command, undo or redo.
    pub fn apply(&mut self, project: &Project, changes: &ChangeSet) {
        if changes.setup.is_some() && self.kind == ViewKind::Board {
            // Layers come and go.
            self.rebuild(project);
            return;
        }
        let mut ids: Vec<ObjectId> = Vec::new();
        for c in &changes.changes {
            let kind = c.id.kind();
            if kind.is_library() {
                self.rebuild(project);
                return;
            }
            if kind == ObjectKind::Component {
                // Refdes/value/variant changes affect how the component is drawn.
                let cid = c.id.raw();
                match self.kind {
                    ViewKind::Schematic => {
                        ids.extend(project.schematic.symbols_of(cid).map(|s| ObjectId::Symbol(s.id)))
                    }
                    ViewKind::Board => ids.extend(
                        project
                            .board
                            .footprints
                            .iter()
                            .filter(|f| f.component == cid)
                            .map(|f| ObjectId::Footprint(f.id)),
                    ),
                }
            } else if self.kind.shows(kind) {
                ids.push(c.id);
            }
        }
        let mut seen = FxHashSet::default();
        for id in ids {
            if seen.insert(id) {
                self.redraw(project, id);
            }
        }
    }

    fn redraw(&mut self, project: &Project, id: ObjectId) {
        let mut p = std::mem::take(&mut self.painter);
        p.clear();
        if self.draw_object(project, id, &mut p) {
            self.scene.set_item(item(id), p.out.drain(..));
            match p.bounds {
                Some(b) => self.index.set(id, b),
                None => self.index.remove(id),
            }
        } else {
            self.scene.remove_item(item(id));
            self.index.remove(id);
        }
        self.painter = p;
        if self.selection.contains(&id) {
            self.draw_overlay(project, id, false);
        }
        if self.hover == Some(id) {
            self.draw_overlay(project, id, true);
        }
    }

    /// Draw automatic junction dots and unconnected pin markers (schematic only).
    pub fn set_connectivity(&mut self, c: &editor_core::connectivity::Connectivity) {
        let Layers::Sch(l) = &self.layers else { return };
        let mut out = Vec::with_capacity(c.dots.len() + c.unconnected.len());
        for &(_, p) in &c.dots {
            out.push((l.wires, Instance::circle(mmf(p), JUNCTION_RADIUS, SCH_JUNCTION)));
        }
        for &(_, _, p) in &c.unconnected {
            out.push((l.pins, Instance::circle_outline(mmf(p), 0.4, 0.12, SCH_PIN)));
        }
        self.scene.set_item(CONNECTIVITY_ITEM, out);
    }

    /// Show what a schematic creation tool would add (empty slice clears).
    pub fn set_preview(&mut self, project: &Project, ghosts: &[SchGhost]) {
        let Layers::Sch(l) = &self.layers else { return };
        if ghosts.is_empty() {
            self.scene.remove_item(PREVIEW_ITEM);
            return;
        }
        let mut p = Painter::default();
        for g in ghosts {
            match g {
                SchGhost::Wire(a, b) => p.line(l.wires, *a, *b, WIRE_WIDTH, SCH_WIRE),
                SchGhost::Junction(pos) => p.circle(l.wires, *pos, 450_000, SCH_JUNCTION),
                SchGhost::Target(pos) => {
                    p.push(l.wires, Instance::circle_outline(mmf(*pos), 0.6, 0.12, HOVER.with_alpha(0xc0)))
                }
                SchGhost::Label(lab) => draw_label(lab, l, &mut p),
                SchGhost::Symbol { part, component, unit, transform } => {
                    let Some(part) = project.library.parts.get(*part) else { continue };
                    let Some(sym) = part.units.get(*unit as usize).and_then(|u| project.library.symbols.get(u.symbol))
                    else {
                        continue;
                    };
                    let c = component.and_then(|c| project.schematic.components.get(c));
                    let refdes = c.map_or(format!("{}?", part.prefix), |c| c.refdes.clone());
                    let value = c.map_or(part.default_value.as_str(), |c| c.value.as_str());
                    let variant = c.map_or(part.variants.first(), |c| part.variant(c.variant));
                    let footprint = variant.and_then(|v| project.library.footprints.get(v.footprint));
                    draw_symbol_graphics(sym, *unit, transform, &refdes, value, variant.zip(footprint), l, &mut p);
                }
            }
        }
        // Ghosts are drawn translucent on top of everything.
        let layer = self.hover_layer;
        let out: Vec<(RLayer, Instance)> = p
            .out
            .into_iter()
            .map(|(_, mut inst)| {
                inst.color = inst.color.with_alpha((inst.color.0[3] as u16 * 3 / 4) as u8);
                (layer, inst)
            })
            .collect();
        self.scene.set_item(PREVIEW_ITEM, out);
    }

    /// Show what a board tool would create (empty slice clears).
    pub fn set_board_preview(&mut self, ghosts: &[PcbGhost]) {
        if ghosts.is_empty() {
            self.scene.remove_item(PREVIEW_ITEM);
            return;
        }
        let layer = self.hover_layer;
        let mut out = Vec::new();
        for g in ghosts {
            match *g {
                PcbGhost::Zone { layer: l, ref points } => {
                    let color = board_layer_color(l).with_alpha(0xc0);
                    let n = points.len();
                    for i in 0..n {
                        let w = if i + 1 == n { 0.05 } else { 0.1 };
                        out.push((layer, Instance::line(mmf(points[i]), mmf(points[(i + 1) % n]), w, color)));
                    }
                }
                PcbGhost::Track { layer: l, a, b, width } => out.push((
                    layer,
                    Instance::line(mmf(a), mmf(b), (width as f64 * 1e-6) as f32, board_layer_color(l).with_alpha(0xc0)),
                )),
                PcbGhost::Via { pos, diameter } => out
                    .push((layer, Instance::circle(mmf(pos), (diameter as f64 * 0.5e-6) as f32, VIA.with_alpha(0xc0)))),
                PcbGhost::Outline(r) => {
                    let c = [r.min, Point::new(r.max.x, r.min.y), r.max, Point::new(r.min.x, r.max.y)];
                    let color = board_layer_color(LayerId::OUTLINE);
                    for i in 0..4 {
                        out.push((layer, Instance::line(mmf(c[i]), mmf(c[(i + 1) % 4]), 0.1, color)));
                    }
                }
                PcbGhost::Target(p) => {
                    out.push((layer, Instance::circle_outline(mmf(p), 0.5, 0.08, HOVER.with_alpha(0xc0))))
                }
            }
        }
        self.scene.set_item(PREVIEW_ITEM, out);
    }

    /// Draw the ratsnest and refresh net highlights (board only).
    pub fn set_copper(&mut self, _project: &Project, c: &BoardConnectivity) {
        let Layers::Board(l) = &self.layers else { return };
        let mut out: Vec<(RLayer, Instance)> = Vec::new();
        // Zone fills, on their copper layer.
        for fill in &c.fills {
            let Some(&layer) = l.map.get(&fill.layer) else { continue };
            let color = board_layer_color(fill.layer).with_alpha(0x90);
            for shape in &fill.shapes {
                out.extend(region_instances(shape, color).into_iter().map(|i| (layer, i)));
            }
        }
        out.extend(c.ratsnest.iter().map(|r| (l.ratsnest, Instance::line(mmf(r.a), mmf(r.b), 0.05, RATSNEST))));
        // Flagged copper: a halo around it.
        for item in c.flagged() {
            if let Geo::Capsule { a, b, r } = item.geo {
                let w = (r as f64 * 2e-6) as f32 + 0.3;
                out.push((l.ratsnest, Instance::line(mmf(a), mmf(b), w, FLAGGED)));
            }
        }
        self.scene.set_item(CONNECTIVITY_ITEM, out);
        let nets = std::mem::take(&mut self.nets);
        self.set_nets(c, &nets);
    }

    /// Mark design rule violations (board only; empty clears).
    pub fn set_drc(&mut self, violations: &[editor_core::drc::Violation]) {
        self.drc = violations.iter().map(|v| v.pos).collect();
        self.draw_drc();
    }

    fn draw_drc(&mut self) {
        if self.drc.is_empty() {
            self.scene.remove_item(DRC_ITEM);
            return;
        }
        let layer = self.hover_layer;
        let mut out = Vec::new();
        for &p in &self.drc {
            let c = mmf(p);
            out.push((layer, Instance::circle_outline(c, 0.6, 0.12, DRC_MARKER)));
            let d = 0.35;
            out.push((layer, Instance::line([c[0] - d, c[1] - d], [c[0] + d, c[1] + d], 0.12, DRC_MARKER)));
            out.push((layer, Instance::line([c[0] - d, c[1] + d], [c[0] + d, c[1] - d], 0.12, DRC_MARKER)));
        }
        self.scene.set_item(DRC_ITEM, out);
    }

    /// Highlight all copper of these nets (board only; empty clears).
    pub fn set_nets(&mut self, c: &BoardConnectivity, nets: &[String]) {
        self.nets = nets.to_vec();
        let indices: Vec<u32> = nets.iter().filter_map(|n| c.net_index(n)).collect();
        if indices.is_empty() {
            self.scene.remove_item(NET_ITEM);
            return;
        }
        let layer = self.hover_layer;
        let mut out = Vec::new();
        for item in c.items.iter().filter(|i| i.net.is_some_and(|n| indices.contains(&n))) {
            match &item.geo {
                Geo::Capsule { a, b, r } => {
                    out.push((layer, Instance::line(mmf(*a), mmf(*b), (*r as f64 * 2e-6) as f32, NET_HIGHLIGHT)))
                }
                Geo::Polygon(points) => {
                    let pts: Vec<[f32; 2]> = points.iter().map(|&p| mmf(p)).collect();
                    out.extend(render::tess::fill_polygon(&pts, NET_HIGHLIGHT).into_iter().map(|i| (layer, i)));
                }
                Geo::Region { outer, holes } => {
                    let shape: Vec<Vec<Point>> = std::iter::once(outer).chain(holes).cloned().collect();
                    out.extend(region_instances(&shape, NET_HIGHLIGHT).into_iter().map(|i| (layer, i)));
                }
            }
        }
        self.scene.set_item(NET_ITEM, out);
    }

    pub fn highlighted_nets(&self) -> &[String] {
        &self.nets
    }

    pub fn probed(&self) -> &[ObjectId] {
        &self.probed
    }

    /// Highlight objects selected in the other view (cross-probing; empty clears).
    pub fn set_probe(&mut self, project: &Project, ids: &[ObjectId]) {
        self.probed = ids.to_vec();
        if ids.is_empty() {
            self.scene.remove_item(PROBE_ITEM);
            return;
        }
        let layer = self.hover_layer;
        let mut out = Vec::new();
        for &id in ids {
            let mut p = Painter::default();
            if !self.draw_object(project, id, &mut p) {
                continue;
            }
            out.extend(p.out.into_iter().map(|(_, mut inst)| {
                inst.color = PROBE;
                (layer, inst)
            }));
            if matches!(id, ObjectId::Symbol(_) | ObjectId::Footprint(_))
                && let Some(b) = p.bounds
            {
                let size = [(b.width() as f64 * 1e-6) as f32, (b.height() as f64 * 1e-6) as f32];
                out.push((layer, Instance::rect(mmf(b.center()), size, 0.0, 0.0, PROBE_FILL)));
            }
        }
        self.scene.set_item(PROBE_ITEM, out);
    }

    // ------------------------------------------------------------------------------------
    // Highlights

    /// Show `selection` and `hover`. Only changed overlays are redrawn.
    pub fn set_highlight(&mut self, project: &Project, selection: &[ObjectId], hover: Option<ObjectId>) {
        let new: FxHashSet<ObjectId> = selection.iter().copied().collect();
        let removed: Vec<ObjectId> = self.selection.difference(&new).copied().collect();
        let added: Vec<ObjectId> = new.difference(&self.selection).copied().collect();
        for id in removed {
            self.scene.remove_item(overlay_item(id, false));
        }
        self.selection = new;
        for id in added {
            self.draw_overlay(project, id, false);
        }
        if hover != self.hover {
            if let Some(old) = self.hover {
                self.scene.remove_item(overlay_item(old, true));
            }
            self.hover = hover;
            if let Some(h) = hover {
                self.draw_overlay(project, h, true);
            }
        }
    }

    fn draw_overlay(&mut self, project: &Project, id: ObjectId, hover: bool) {
        let mut p = Painter::default();
        let key = overlay_item(id, hover);
        if !self.draw_object(project, id, &mut p) {
            self.scene.remove_item(key);
            return;
        }
        let (layer, color) = if hover { (self.hover_layer, HOVER) } else { (self.selection_layer, SELECTION) };
        let mut out: Vec<(RLayer, Instance)> = p
            .out
            .into_iter()
            .map(|(_, mut inst)| {
                inst.color = color;
                (layer, inst)
            })
            .collect();
        if matches!(id, ObjectId::Symbol(_) | ObjectId::Footprint(_))
            && let Some(b) = p.bounds
        {
            let fill = if hover { HOVER.with_alpha(0x18) } else { SELECTION_FILL };
            let size = [(b.width() as f64 * 1e-6) as f32, (b.height() as f64 * 1e-6) as f32];
            out.push((layer, Instance::rect(mmf(b.center()), size, 0.0, 0.0, fill)));
        }
        self.scene.set_item(key, out);
    }

    // ------------------------------------------------------------------------------------
    // Hit testing

    /// Topmost object at `p` within `tol`. Thin objects (wires, tracks) win over bodies,
    /// smaller bodies over larger ones.
    pub fn hit(&self, project: &Project, p: Point, tol: Coord) -> Option<ObjectId> {
        let mut best: Option<((u8, f64), ObjectId)> = None;
        for id in self.index.near(p, tol) {
            let score = match id {
                ObjectId::Wire(w) => project.schematic.wires.get(w).map(|w| (w.a, w.b, WIRE_WIDTH)),
                ObjectId::Track(t) => project.board.tracks.get(t).map(|t| (t.a, t.b, t.width)),
                _ => None,
            }
            .map(|(a, b, width)| seg_dist(p, a, b) - width as f64 / 2.0)
            .or_else(|| match id {
                ObjectId::Junction(j) => project.schematic.junctions.get(j).map(|j| dist(p, j.pos) - 450_000.0),
                ObjectId::Via(v) => project.board.vias.get(v).map(|v| dist(p, v.pos) - v.diameter as f64 / 2.0),
                ObjectId::BoardGraphic(g) => match project.board.graphics.get(g).map(|g| &g.shape) {
                    Some(Shape::Line { a, b, width }) => Some(seg_dist(p, *a, *b) - *width as f64 / 2.0),
                    Some(Shape::Rect { a, b, width, fill: false }) => {
                        let r = Rect::from_points(*a, *b);
                        let c = [r.min, Point::new(r.max.x, r.min.y), r.max, Point::new(r.min.x, r.max.y)];
                        Some(
                            (0..4).map(|i| seg_dist(p, c[i], c[(i + 1) % 4])).fold(f64::MAX, f64::min)
                                - *width as f64 / 2.0,
                        )
                    }
                    _ => None,
                },
                _ => None,
            })
            .map(|d| (d <= tol as f64).then_some((0u8, d)))
            .unwrap_or_else(|| {
                let b = self.index.bounds(id)?;
                let grown = Rect { min: b.min - Point::new(tol, tol), max: b.max + Point::new(tol, tol) };
                grown.contains(p).then(|| (1u8, b.width() as f64 * b.height() as f64))
            });
            if let Some(s) = score
                && best.as_ref().is_none_or(|(bs, _)| s < *bs)
            {
                best = Some((s, id));
            }
        }
        best.map(|(_, id)| id)
    }

    /// Objects inside `r` (fully, or touching when `crossing`).
    pub fn select_rect(&self, r: Rect, crossing: bool) -> Vec<ObjectId> {
        let mut ids = self.index.in_rect(r, crossing);
        ids.sort();
        ids
    }

    /// Bounds of all drawn objects.
    pub fn bounds(&self) -> Option<Rect> {
        self.index.total()
    }

    // ------------------------------------------------------------------------------------
    // Drawing

    fn draw_object(&self, project: &Project, id: ObjectId, p: &mut Painter) -> bool {
        match (&self.layers, id) {
            (Layers::Sch(l), ObjectId::Symbol(i)) => {
                project.schematic.symbols.get(i).map(|s| draw_symbol(project, s, l, p)).is_some()
            }
            (Layers::Sch(l), ObjectId::Wire(i)) => {
                project.schematic.wires.get(i).map(|w| p.line(l.wires, w.a, w.b, WIRE_WIDTH, SCH_WIRE)).is_some()
            }
            (Layers::Sch(l), ObjectId::Junction(i)) => {
                project.schematic.junctions.get(i).map(|j| p.circle(l.wires, j.pos, 450_000, SCH_JUNCTION)).is_some()
            }
            (Layers::Sch(l), ObjectId::Label(i)) => {
                project.schematic.labels.get(i).map(|lab| draw_label(lab, l, p)).is_some()
            }
            (Layers::Sch(l), ObjectId::SchText(i)) => project
                .schematic
                .texts
                .get(i)
                .map(|t| p.text(l.text, &t.text, SCH_TEXT, &|s| s.to_string(), true))
                .is_some(),
            (Layers::Board(l), ObjectId::Footprint(i)) => {
                project.board.footprints.get(i).map(|f| draw_footprint(project, f, l, p)).is_some()
            }
            (Layers::Board(l), ObjectId::Track(i)) => project
                .board
                .tracks
                .get(i)
                .map(|t| {
                    if let Some(&layer) = l.map.get(&t.layer) {
                        p.line(layer, t.a, t.b, t.width, board_layer_color(t.layer));
                    }
                })
                .is_some(),
            (Layers::Board(l), ObjectId::Via(i)) => project
                .board
                .vias
                .get(i)
                .map(|v| {
                    p.circle(l.vias, v.pos, v.diameter / 2, VIA);
                    p.push(l.holes, Instance::circle(mmf(v.pos), (v.drill as f64 * 0.5e-6) as f32, HOLE));
                })
                .is_some(),
            (Layers::Board(l), ObjectId::Zone(i)) => project
                .board
                .zones
                .get(i)
                .map(|z| {
                    // The outline; fills are derived (see `set_copper`).
                    if let Some(&layer) = l.map.get(&z.layer) {
                        let shape = Shape::Polygon { points: z.outline.clone(), width: 100_000, fill: false };
                        p.shape(layer, &shape, board_layer_color(z.layer).with_alpha(0xa0), None);
                    }
                })
                .is_some(),
            (Layers::Board(l), ObjectId::BoardGraphic(i)) => project
                .board
                .graphics
                .get(i)
                .map(|g| {
                    if let Some(&layer) = l.map.get(&g.layer) {
                        p.shape(layer, &g.shape, board_layer_color(g.layer), None);
                    }
                })
                .is_some(),
            (Layers::Board(l), ObjectId::BoardText(i)) => project
                .board
                .texts
                .get(i)
                .map(|t| {
                    if let Some(&layer) = l.map.get(&t.layer) {
                        p.text(layer, &t.text, board_layer_color(t.layer), &|s| s.to_string(), true);
                    }
                })
                .is_some(),
            _ => false,
        }
    }
}

fn dist(p: Point, q: Point) -> f64 {
    (p - q).length()
}

fn seg_dist(p: Point, a: Point, b: Point) -> f64 {
    let (ax, ay, bx, by, px, py) = (a.x as f64, a.y as f64, b.x as f64, b.y as f64, p.x as f64, p.y as f64);
    let (dx, dy) = (bx - ax, by - ay);
    let len2 = dx * dx + dy * dy;
    let t = if len2 == 0.0 { 0.0 } else { (((px - ax) * dx + (py - ay) * dy) / len2).clamp(0.0, 1.0) };
    ((ax + t * dx - px).powi(2) + (ay + t * dy - py).powi(2)).sqrt()
}

fn draw_symbol(project: &Project, inst: &SymbolInstance, l: &SchLayers, p: &mut Painter) {
    let Some(sym) = symbol_of_instance(project, inst) else { return };
    let component = project.schematic.components.get(inst.component);
    let (refdes, value) = component.map_or(("?", ""), |c| (c.refdes.as_str(), c.value.as_str()));
    // Pad names per pin for the component's variant.
    let variant = component.and_then(|c| project.library.parts.get(c.part).and_then(|part| part.variant(c.variant)));
    let footprint = component.and_then(|c| project.library.footprint_of(c.part, c.variant));
    let pads = variant.zip(footprint);
    draw_symbol_graphics(sym, inst.unit, &inst.transform(), refdes, value, pads, l, p);
}

#[allow(clippy::too_many_arguments)]
fn draw_symbol_graphics(
    sym: &Symbol,
    unit: u32,
    t: &Transform,
    refdes: &str,
    value: &str,
    pads: Option<(&PartVariant, &Footprint)>,
    l: &SchLayers,
    p: &mut Painter,
) {
    let t = *t;
    for shape in &sym.graphics {
        p.shape(l.body, &shape.transformed(&t), SCH_BODY, Some((l.body_fill, SCH_BODY_FILL)));
    }

    for pin in sym.pins.iter() {
        let (a, b) = (t.point(pin.pos), t.point(pin.inner_end()));
        p.line(l.pins, a, b, WIRE_WIDTH, SCH_PIN);
        let d = b - a;
        let horizontal = d.x.abs() >= d.y.abs();
        if pin.show_number
            && let Some((v, fp)) = pads
        {
            let pads: Vec<&str> = v
                .connections
                .iter()
                .filter(|c| c.unit == unit && c.pin == pin.id)
                .filter_map(|c| fp.pads.get(c.pad).map(|pad| pad.name.as_str()))
                .collect();
            if !pads.is_empty() {
                let mid = Point::new((a.x + b.x) / 2, (a.y + b.y) / 2);
                let (pos, rotation) = if horizontal {
                    (mid + Point::new(0, 250_000), Angle::ZERO)
                } else {
                    (mid - Point::new(250_000, 0), Angle::DEG90)
                };
                let mut text = Text::new(pads.join(","), pos, 1_000_000).aligned(HAlign::Center, VAlign::Bottom);
                text.rotation = rotation;
                p.text(l.pin_text, &text, SCH_PIN_TEXT, &|s| s.to_string(), false);
            }
        }
        if pin.show_name {
            let inward = if horizontal { d.x > 0 } else { d.y > 0 };
            let step =
                if horizontal { Point::new(d.x.signum() * 500_000, 0) } else { Point::new(0, d.y.signum() * 500_000) };
            let mut text = Text::new(pin.name.clone(), b + step, 1_270_000)
                .aligned(if inward { HAlign::Left } else { HAlign::Right }, VAlign::Center);
            text.rotation = if horizontal { Angle::ZERO } else { Angle::DEG90 };
            p.text(l.body, &text, SCH_BODY, &|s| s.to_string(), false);
        }
    }

    let sideways = t.rotation.quarter_turns().is_some_and(|q| q % 2 == 1);
    let body = Rect::union_all(sym.graphics.iter().map(|g| g.transformed(&t).bounds()));
    for text in &sym.texts {
        let mut text = text.transformed(&t);
        if sideways {
            // Keep reference and value horizontal next to vertical symbols.
            text.rotation = Angle::ZERO;
            let left = text.pos.x < t.offset.x;
            text.halign = if left { HAlign::Right } else { HAlign::Left };
            text.valign = VAlign::Center;
            if let Some(b) = body {
                let gap = 500_000;
                text.pos.x = if left { text.pos.x.min(b.min.x - gap) } else { text.pos.x.max(b.max.x + gap) };
            }
        }
        p.text(l.text, &text, SCH_TEXT, &|s| substitute(s, refdes, value), false);
    }
}

fn draw_label(lab: &NetLabel, l: &SchLayers, p: &mut Painter) {
    let t = Transform::new(lab.pos, lab.rotation, false);
    let line = |p: &mut Painter, a: (f64, f64), b: (f64, f64), color: Color| {
        p.line(l.labels, t.point(Point::mm(a.0, a.1)), t.point(Point::mm(b.0, b.1)), WIRE_WIDTH, color)
    };
    match lab.kind {
        LabelKind::Net => {
            p.circle(l.labels, lab.pos, 200_000, SCH_LABEL);
            let text = Text::new(lab.name.clone(), Point::mm(0.4, 0.3), 1_270_000).transformed(&t);
            p.text(l.labels, &text, SCH_LABEL, &|s| s.to_string(), true);
        }
        LabelKind::Power if lab.name.to_uppercase().contains("GND") => {
            line(p, (0.0, 0.0), (0.0, -1.27), SCH_POWER);
            for (y, w) in [(-1.27, 1.27), (-1.78, 0.76), (-2.29, 0.25)] {
                line(p, (-w, y), (w, y), SCH_POWER);
            }
            let text = Text::new(lab.name.clone(), Point::mm(0.0, -2.8), 1_270_000)
                .aligned(HAlign::Center, VAlign::Top)
                .transformed(&t);
            p.text(l.labels, &text, SCH_POWER, &|s| s.to_string(), true);
        }
        LabelKind::Power => {
            line(p, (0.0, 0.0), (0.0, 1.27), SCH_POWER);
            line(p, (-1.27, 1.27), (1.27, 1.27), SCH_POWER);
            let text = Text::new(lab.name.clone(), Point::mm(0.0, 1.8), 1_270_000)
                .aligned(HAlign::Center, VAlign::Bottom)
                .transformed(&t);
            p.text(l.labels, &text, SCH_POWER, &|s| s.to_string(), true);
        }
    }
}

fn draw_footprint(project: &Project, inst: &FootprintInstance, l: &BoardLayers, p: &mut Painter) {
    let Some(fp) = footprint_of_instance(project, inst) else { return };
    let component = project.schematic.components.get(inst.component);
    let (refdes, value) = component.map_or(("?", ""), |c| (c.refdes.as_str(), c.value.as_str()));
    let t = inst.transform();
    let side = inst.side;
    let top_copper = LayerId::for_footprint(FootprintLayer::Copper, side);

    let mut body: Option<Rect> = None;
    let mut courtyard: Option<Rect> = None;
    for pad in fp.pads.iter() {
        let center = t.point(pad.pos);
        let pad_rot = if t.mirror { -pad.rotation.degrees() } else { pad.rotation.degrees() };
        let rot = (t.rotation.degrees() + pad_rot).to_radians() as f32;
        let (w, h) = ((pad.size.w as f64 * 1e-6) as f32, (pad.size.h as f64 * 1e-6) as f32);
        let layers: Vec<LayerId> = match pad.kind {
            PadKind::Smd => vec![top_copper],
            PadKind::ThroughHole { .. } => l.copper.clone(),
            PadKind::NonPlatedHole { .. } => vec![],
        };
        for layer in layers {
            let (Some(&rl), color) = (l.map.get(&layer), board_layer_color(layer)) else { continue };
            let c = mmf(center);
            match &pad.shape {
                PadShape::Circle => p.push(rl, Instance::circle(c, w * 0.5, color)),
                PadShape::Rect => p.push(rl, Instance::rect(c, [w, h], 0.0, rot, color)),
                PadShape::RoundRect { ratio } => {
                    p.push(rl, Instance::rect(c, [w, h], *ratio as f32 * w.min(h), rot, color))
                }
                PadShape::Oval => p.push(rl, Instance::rect(c, [w, h], 0.5 * w.min(h), rot, color)),
                PadShape::Polygon { points } => {
                    p.fill(rl, points.iter().map(|&q| t.point(pad.pos + q.rotated(pad.rotation))), color);
                }
            }
        }
        if let PadKind::ThroughHole { drill } | PadKind::NonPlatedHole { drill } = pad.kind {
            p.push(l.holes, Instance::circle(mmf(center), (drill as f64 * 0.5e-6) as f32, HOLE));
        }
        let r = Rect::around(center, pad.size.w.max(pad.size.h) / 2);
        body = Some(body.map_or(r, |b| b.union(r)));
        // Pad number, readable on top of the pad.
        let size = (pad.size.w.min(pad.size.h) as f64 * 0.45) as Coord;
        let text = Text::new(pad.name.clone(), center, size).aligned(HAlign::Center, VAlign::Center);
        p.text(l.pad_numbers, &text, Color::rgba(0xff, 0xff, 0xff, 0xc0), &|s| s.to_string(), false);
    }

    for g in &fp.graphics {
        let layer = LayerId::for_footprint(g.layer, side);
        let shape = g.shape.transformed(&t);
        let b = shape.bounds();
        if g.layer == FootprintLayer::Courtyard {
            courtyard = Some(courtyard.map_or(b, |c| c.union(b)));
        } else {
            body = Some(body.map_or(b, |r| r.union(b)));
        }
        if let Some(&rl) = l.map.get(&layer) {
            p.push_shape(rl, &shape, board_layer_color(layer));
        }
    }
    for ft in &fp.texts {
        let layer = LayerId::for_footprint(ft.layer, side);
        if let Some(&rl) = l.map.get(&layer) {
            p.text(rl, &ft.text.transformed(&t), board_layer_color(layer), &|s| substitute(s, refdes, value), false);
        }
    }
    // Hit-test bounds: courtyard if the footprint has one, otherwise pads and graphics.
    p.bounds = courtyard.or(body);
}

impl Painter {
    /// Shape without touching the hit bounds.
    fn push_shape(&mut self, layer: RLayer, shape: &Shape, color: Color) {
        let saved = self.bounds;
        self.shape(layer, shape, color, None);
        self.bounds = saved;
    }
}

/// A filled region: `[outline, holes…]`.
fn region_instances(shape: &[Vec<Point>], color: Color) -> Vec<Instance> {
    let contours: Vec<Vec<[f32; 2]>> = shape.iter().map(|c| c.iter().map(|&p| mmf(p)).collect()).collect();
    render::tess::fill_region(&contours, color)
}
