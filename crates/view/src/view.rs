//! Schematic and board views: incremental scene building, highlights and hit testing.

use editor_core::ChangeSet;
use editor_core::copper::{BoardConnectivity, Geo};
use editor_core::fab::{PadOutline, pad_copper_layers, pad_mask_layers, pad_outline, pad_paste_layer};
use editor_core::query::{footprint_of_instance, symbol_of_instance};
use model::*;
use render::{Color, Instance, Scene, font};
use rustc_hash::{FxHashMap, FxHashSet, FxHasher};
use std::hash::{Hash, Hasher};

use crate::draw::{Painter, mmf, substitute};
use crate::index::SpatialIndex;
use crate::pcb_tools::PcbGhost;
use crate::sch_tools::SchGhost;
use crate::sim_overlay::SimOverlay;
use crate::style::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ViewKind {
    Schematic,
    Board,
}

/// One view of a project: a schematic sheet, or the board.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ViewId {
    Sheet(Id),
    Board,
}

impl ViewId {
    pub fn kind(self) -> ViewKind {
        match self {
            ViewId::Sheet(_) => ViewKind::Schematic,
            ViewId::Board => ViewKind::Board,
        }
    }

    pub fn sheet(self) -> Option<Id> {
        match self {
            ViewId::Sheet(s) => Some(s),
            ViewId::Board => None,
        }
    }

    /// The view of the root sheet.
    pub fn root(project: &Project) -> ViewId {
        ViewId::Sheet(project.schematic.root_sheet().unwrap_or(Id::from_u128(0)))
    }
}

impl ViewKind {
    /// Does this view show objects of `kind`?
    pub fn shows(self, kind: ObjectKind) -> bool {
        match self {
            ViewKind::Schematic => matches!(
                kind,
                ObjectKind::Sheet
                    | ObjectKind::Symbol
                    | ObjectKind::Wire
                    | ObjectKind::BusWire
                    | ObjectKind::BusEntry
                    | ObjectKind::Junction
                    | ObjectKind::NoConnect
                    | ObjectKind::PowerFlag
                    | ObjectKind::Label
                    | ObjectKind::SchText
                    | ObjectKind::SymbolField
            ),
            ViewKind::Board => kind.is_board(),
        }
    }
}

type RLayer = render::LayerId;

/// The library objects in `changes` whose edits change how placed parts look: symbols,
/// footprints, and parts whose units or variants changed (which symbol, which footprint,
/// pin-pad names). A part's name or description isn't drawn.
pub fn drawn_library_changes(changes: &ChangeSet) -> Vec<ObjectId> {
    changes
        .changes
        .iter()
        .filter(|c| c.id.kind().is_library())
        .filter(|c| c.id.kind() != ObjectKind::LibPart || c.affects_connectivity())
        .map(|c| c.id)
        .collect()
}

/// A layer the user can show or hide (layer menus).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LayerEntry {
    pub layer: RLayer,
    /// What the layer's content is drawn in, if it has one colour.
    pub color: Option<Color>,
    /// Menus separate entries of different groups.
    pub group: u8,
}

struct SchLayers {
    frame: RLayer,
    body_fill: RLayer,
    body: RLayer,
    pins: RLayer,
    pin_text: RLayer,
    wires: RLayer,
    labels: RLayer,
    text: RLayer,
    /// Markers of ERC findings (not printed).
    erc: RLayer,
    /// Probes and operating point values of a simulation (not printed).
    sim: RLayer,
}

struct BoardLayers {
    map: FxHashMap<LayerId, RLayer>,
    copper: Vec<LayerId>,
    vias: RLayer,
    holes: RLayer,
    pad_numbers: RLayer,
    /// Track net names, per copper layer.
    track_labels: FxHashMap<LayerId, RLayer>,
    /// Pad numbers and net names at high zoom.
    pad_labels: RLayer,
    ratsnest: RLayer,
    /// Markers of design rule violations (not printed).
    drc: RLayer,
}

enum Layers {
    Sch(SchLayers),
    Board(BoardLayers),
}

pub struct View {
    pub id: ViewId,
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
    drc: Vec<(Point, Severity)>,
    /// Cross-probed objects.
    probed: Vec<ObjectId>,
    /// What the net labels were drawn for (board); None: none drawn, or out of date.
    labels: Option<LabelsDrawn>,
}

/// The area, zoom step and layer visibility net labels were drawn for.
#[derive(Clone, Copy, Debug, PartialEq)]
struct LabelsDrawn {
    area: Rect,
    step: i32,
    visible: u128,
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
/// The drawing frame of a schematic sheet.
const FRAME_ITEM: u64 = (3 << 62) | 5;
/// A text being edited in place.
const TEXT_EDIT_ITEM: u64 = (3 << 62) | 6;
/// Probes and values of a simulation (schematic sheets).
const SIM_ITEM: u64 = (3 << 62) | 7;
/// Net names on pads and tracks near the viewport (board, high zoom).
const NET_LABEL_ITEM: u64 = (3 << 62) | 8;

fn overlay_item(id: ObjectId, hover: bool) -> u64 {
    item(id) | if hover { 2 << 62 } else { 1 << 62 }
}

impl View {
    pub fn new(id: ViewId, project: &Project) -> Self {
        let kind = id.kind();
        let (scene, layers, hover_layer, selection_layer) = Self::create_scene(kind, project);
        let mut v = View {
            id,
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
            labels: None,
        };
        v.draw_all(project);
        v
    }

    fn create_scene(kind: ViewKind, project: &Project) -> (Scene, Layers, RLayer, RLayer) {
        let mut scene = Scene::new();
        let layers = match kind {
            ViewKind::Schematic => {
                let l = SchLayers {
                    frame: scene.add_layer("Frame"),
                    body_fill: scene.add_layer("Symbol fill"),
                    body: scene.add_layer("Symbols"),
                    pins: scene.add_layer("Pins"),
                    pin_text: scene.add_layer("Pin numbers"),
                    wires: scene.add_layer("Wires"),
                    labels: scene.add_layer("Labels"),
                    text: scene.add_layer("Text"),
                    erc: scene.add_layer("ERC markers"),
                    sim: scene.add_layer("Simulation"),
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
                // Each copper layer's track net names right above it (hidden under
                // the copper of the layers above).
                let mut track_labels = FxHashMap::default();
                for &id in copper.iter().rev() {
                    add(&mut scene, id);
                    track_labels.insert(id, scene.add_layer(format!("{} net names", name(id))));
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
                // Further in, the net labels draw the pad numbers.
                scene.layer_mut(pad_numbers).max_zoom = crate::net_labels::PAD_LABEL_ZOOM as f32;
                let pad_labels = scene.add_layer("Pad net names");
                let ratsnest = scene.add_layer("Ratsnest");
                let drc = scene.add_layer("DRC markers");
                for id in [LayerId::TOP_MASK, LayerId::BOTTOM_MASK, LayerId::TOP_PASTE, LayerId::BOTTOM_PASTE] {
                    scene.layer_mut(map[&id]).visible = false;
                }
                Layers::Board(BoardLayers {
                    map,
                    copper,
                    vias,
                    holes,
                    pad_numbers,
                    track_labels,
                    pad_labels,
                    ratsnest,
                    drc,
                })
            }
        };
        let hover = scene.add_layer("Hover");
        let selection = scene.add_layer("Selection");
        (scene, layers, hover, selection)
    }

    /// The user-facing layers in menu order with their colours: copper top to bottom,
    /// then top and bottom technical layers, the outline and the overlays.
    pub fn layer_menu(&self) -> Vec<LayerEntry> {
        let p = pal();
        let e = |layer, color, group| LayerEntry { layer, color, group };
        match &self.layers {
            Layers::Sch(l) => vec![
                e(l.body_fill, Some(p.sch_body_fill), 0),
                e(l.body, Some(p.sch_body), 0),
                e(l.pins, Some(p.sch_pin), 0),
                e(l.pin_text, Some(p.sch_pin_text), 0),
                e(l.wires, Some(p.sch_wire), 1),
                e(l.labels, Some(p.sch_label), 1),
                e(l.text, Some(p.sch_text), 1),
                e(l.frame, Some(p.sch_frame), 2),
                e(l.erc, Some(p.drc_marker), 3),
                e(l.sim, Some(p.sim_value), 3),
            ],
            Layers::Board(b) => {
                let board = |id: LayerId, group| e(b.map[&id], Some(board_layer_color(id)), group);
                let mut v: Vec<LayerEntry> = b.copper.iter().map(|&id| board(id, 0)).collect();
                v.extend([e(b.vias, Some(p.via), 1), e(b.holes, None, 1)]);
                use LayerId as L;
                for (group, ids) in [
                    (2, [L::TOP_SILK, L::TOP_FAB, L::TOP_COURTYARD, L::TOP_MASK, L::TOP_PASTE]),
                    (3, [L::BOTTOM_SILK, L::BOTTOM_FAB, L::BOTTOM_COURTYARD, L::BOTTOM_MASK, L::BOTTOM_PASTE]),
                ] {
                    v.extend(ids.into_iter().map(|id| board(id, group)));
                }
                v.push(board(L::OUTLINE, 4));
                v.extend([
                    e(b.pad_numbers, None, 5),
                    e(b.ratsnest, Some(p.ratsnest), 5),
                    e(b.drc, Some(p.drc_marker), 5),
                ]);
                v
            }
        }
    }

    /// Names of the hidden user-facing layers (saved with the workspace).
    pub fn hidden_layers(&self) -> Vec<String> {
        self.user_layers().map(|i| self.scene.layer(i)).filter(|l| !l.visible).map(|l| l.name.clone()).collect()
    }

    /// Show the user-facing layers except those named.
    pub fn set_hidden_layers(&mut self, hidden: &[String]) {
        for i in self.user_layers().collect::<Vec<_>>() {
            let l = self.scene.layer_mut(i);
            l.visible = !hidden.contains(&l.name);
        }
    }

    /// The render layers that are user facing (overlays and net labels excluded).
    pub fn user_layers(&self) -> impl Iterator<Item = u16> + '_ {
        let labels: Vec<RLayer> = match &self.layers {
            Layers::Board(l) => l.track_labels.values().copied().chain([l.pad_labels]).collect(),
            Layers::Sch(_) => Vec::new(),
        };
        (0..self.hover_layer).filter(move |i| !labels.contains(i))
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
        self.labels = None;
        self.draw_all(project);
        let sel: Vec<ObjectId> = self.selection.drain().collect();
        let hover = self.hover.take();
        self.set_highlight(project, &sel, hover);
        self.draw_drc();
    }

    fn draw_all(&mut self, project: &Project) {
        let ids: Vec<ObjectId> = match self.kind {
            ViewKind::Schematic => {
                // `draw_object` skips what is on other sheets.
                let s = &project.schematic;
                s.sheets
                    .ids()
                    .map(ObjectId::Sheet)
                    .chain(s.symbols.ids().map(ObjectId::Symbol))
                    .chain(s.wires.ids().map(ObjectId::Wire))
                    .chain(s.bus_wires.ids().map(ObjectId::BusWire))
                    .chain(s.bus_entries.ids().map(ObjectId::BusEntry))
                    .chain(s.junctions.ids().map(ObjectId::Junction))
                    .chain(s.no_connects.ids().map(ObjectId::NoConnect))
                    .chain(s.power_flags.ids().map(ObjectId::PowerFlag))
                    .chain(s.labels.ids().map(ObjectId::Label))
                    .chain(s.texts.ids().map(ObjectId::SchText))
                    .chain(s.fields.ids().map(ObjectId::SymbolField))
                    .collect()
            }
            ViewKind::Board => {
                let b = &project.board;
                b.footprints
                    .ids()
                    .map(ObjectId::Footprint)
                    .chain(b.fields.ids().map(ObjectId::FootprintField))
                    .chain(b.tracks.ids().map(ObjectId::Track))
                    .chain(b.vias.ids().map(ObjectId::Via))
                    .chain(b.zones.ids().map(ObjectId::Zone))
                    .chain(b.keepouts.ids().map(ObjectId::Keepout))
                    .chain(b.graphics.ids().map(ObjectId::BoardGraphic))
                    .chain(b.texts.ids().map(ObjectId::BoardText))
                    .collect()
            }
        };
        for id in ids {
            self.redraw(project, id);
        }
        self.draw_frame(project);
    }

    /// The sheet's drawing frame (it shows the title block, sheet names and the page count,
    /// so any sheet change redraws it).
    fn draw_frame(&mut self, project: &Project) {
        let (Layers::Sch(l), Some(sheet)) = (&self.layers, self.id.sheet()) else { return };
        let mut p = std::mem::take(&mut self.painter);
        p.clear();
        crate::frame::draw(project, sheet, l.frame, &mut p);
        self.scene.set_item(FRAME_ITEM, p.out.drain(..));
        self.painter = p;
    }

    /// Update after a command, undo or redo: [`apply_objects`](Self::apply_objects), then
    /// [`redraw_users`](Self::redraw_users) of the library edits.
    pub fn apply(&mut self, project: &Project, changes: &ChangeSet) {
        self.apply_objects(project, changes);
        self.redraw_users(project, &drawn_library_changes(changes));
    }

    /// Redraw the changed schematic or board objects (library edits aside).
    pub fn apply_objects(&mut self, project: &Project, changes: &ChangeSet) {
        self.labels = None;
        if changes.board_setup_changed() && self.kind == ViewKind::Board {
            // Layers come and go.
            self.rebuild(project);
            return;
        }
        let mut ids: Vec<ObjectId> = Vec::new();
        for c in &changes.changes {
            let kind = c.id.kind();
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
                // A sub-sheet's hierarchical labels are its symbol's pins on the parent.
                for o in [c.before.as_ref().map(|b| &b.1), c.after.as_ref()].into_iter().flatten() {
                    if let Object::Label(l) = o
                        && l.kind == LabelKind::Hierarchical
                    {
                        ids.push(ObjectId::Sheet(l.sheet));
                    }
                }
            }
        }
        // Reference and value texts move with their footprint or symbol and show its
        // component's reference.
        let footprints: FxHashSet<Id> =
            ids.iter().filter_map(|id| if let ObjectId::Footprint(f) = id { Some(*f) } else { None }).collect();
        if !footprints.is_empty() {
            let fields = project.board.fields.iter().filter(|f| footprints.contains(&f.footprint));
            ids.extend(fields.map(|f| ObjectId::FootprintField(f.id)).collect::<Vec<_>>());
        }
        let symbols: FxHashSet<Id> =
            ids.iter().filter_map(|id| if let ObjectId::Symbol(s) = id { Some(*s) } else { None }).collect();
        if !symbols.is_empty() {
            let fields = project.schematic.fields.iter().filter(|f| symbols.contains(&f.symbol));
            ids.extend(fields.map(|f| ObjectId::SymbolField(f.id)).collect::<Vec<_>>());
        }
        let mut seen = FxHashSet::default();
        for id in ids {
            if seen.insert(id) {
                self.redraw(project, id);
            }
        }
        if changes.ids().any(|id| id.kind() == ObjectKind::Sheet) {
            self.draw_frame(project);
        }
    }

    /// Redraw the placed symbols or footprints drawn from these library objects.
    pub fn redraw_users(&mut self, project: &Project, library: &[ObjectId]) {
        if library.is_empty() {
            return;
        }
        self.labels = None;
        for id in self.users_of(project, library) {
            self.redraw(project, id);
        }
    }

    /// The placed symbols (schematic) or footprints (board) drawn from these library
    /// objects: a library edit redraws only those. One pass over the instances. Symbols
    /// show their footprint's pad names, so footprints count for the schematic too.
    pub fn users_of(&self, project: &Project, library: &[ObjectId]) -> Vec<ObjectId> {
        let library: FxHashSet<ObjectId> = library.iter().copied().collect();
        let lib = &project.library;
        let components = &project.schematic.components;
        let uses_footprint = |c: &Component| {
            lib.footprint_of(c.part, c.variant).is_some_and(|fp| library.contains(&ObjectId::LibFootprint(fp.id)))
        };
        match self.kind {
            ViewKind::Schematic => project
                .schematic
                .symbols
                .iter()
                .filter(|s| Some(s.sheet) == self.id.sheet())
                .filter(|s| {
                    let Some(c) = components.get(s.component) else { return false };
                    library.contains(&ObjectId::LibPart(c.part))
                        || uses_footprint(c)
                        || lib
                            .parts
                            .get(c.part)
                            .and_then(|p| p.units.get(s.unit as usize))
                            .is_some_and(|u| library.contains(&ObjectId::LibSymbol(u.symbol)))
                })
                .map(|s| ObjectId::Symbol(s.id))
                .collect(),
            ViewKind::Board => project
                .board
                .footprints
                .iter()
                .filter(|f| {
                    let Some(c) = components.get(f.component) else { return false };
                    library.contains(&ObjectId::LibPart(c.part)) || uses_footprint(c)
                })
                .map(|f| ObjectId::Footprint(f.id))
                .collect(),
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
        for &(_, p) in c.dots.iter().filter(|d| Some(d.0) == self.id.sheet()) {
            out.push((l.wires, Instance::circle(mmf(p), to_mm(JUNCTION_RADIUS) as f32, pal().sch_junction)));
        }
        // Pins of the symbols drawn here.
        for &(_, _, p) in c.unconnected.iter().filter(|u| self.index.bounds(ObjectId::Symbol(u.0)).is_some()) {
            out.push((l.pins, Instance::circle_outline(mmf(p), 0.4, 0.12, pal().sch_pin)));
        }
        // Findings about objects drawn here (unconnected pins have their rings).
        for f in c.problems.iter().filter(|f| !f.excluded && f.rule != ErcRule::UnconnectedPin) {
            let here = f.objects.iter().any(|&o| self.index.bounds(o).is_some());
            if let (true, Some(p)) = (here, f.pos) {
                push_marker(&mut out, l.erc, p, f.severity);
            }
        }
        self.scene.set_item(CONNECTIVITY_ITEM, out);
    }

    /// Only the junction dots of `c` (a print: no unconnected pin rings, no findings).
    pub fn set_junctions(&mut self, c: &editor_core::connectivity::Connectivity) {
        let Layers::Sch(l) = &self.layers else { return };
        let dots = c.dots.iter().filter(|d| Some(d.0) == self.id.sheet());
        let out: Vec<_> = dots
            .map(|&(_, p)| (l.wires, Instance::circle(mmf(p), to_mm(JUNCTION_RADIUS) as f32, pal().sch_junction)))
            .collect();
        self.scene.set_item(CONNECTIVITY_ITEM, out);
    }

    /// The instances of the drawing, back to front, without the overlays (hover,
    /// selection), ERC and DRC markers and simulation results: what a print shows.
    pub fn drawing(&self) -> Vec<Instance> {
        let skip = match &self.layers {
            Layers::Sch(l) => vec![l.erc, l.sim],
            Layers::Board(l) => vec![l.drc],
        };
        self.user_layers().filter(|l| !skip.contains(l)).flat_map(|l| self.scene.instances(l).iter().copied()).collect()
    }

    /// Show a simulation's probes and values (schematic sheets; see
    /// [`crate::sim_overlay`]).
    pub fn set_sim(&mut self, project: &Project, c: &editor_core::connectivity::Connectivity, o: &SimOverlay) {
        let (Some(sheet), Layers::Sch(l)) = (self.id.sheet(), &self.layers) else { return };
        let layers = crate::sim_overlay::Layers { sim: l.sim, hover: self.hover_layer };
        if o.is_empty() {
            self.scene.remove_item(SIM_ITEM);
            return;
        }
        let mut p = Painter::default();
        let index = &self.index;
        crate::sim_overlay::draw(project, c, sheet, o, layers, &mut p, &|id| index.bounds(id));
        self.scene.set_item(SIM_ITEM, p.out);
    }

    /// The markers layer is shown: ERC markers on schematic sheets, DRC markers on the
    /// board.
    pub fn markers_visible(&self) -> bool {
        let layer = match &self.layers {
            Layers::Sch(l) => l.erc,
            Layers::Board(l) => l.drc,
        };
        self.scene.layer(layer).visible
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
                SchGhost::Wire(a, b) => p.line(l.wires, *a, *b, WIRE_WIDTH, pal().sch_wire),
                SchGhost::Bus(a, b) => p.line(l.wires, *a, *b, BUS_WIDTH, pal().sch_bus),
                SchGhost::Entry(pos, dir) => p.line(l.wires, *pos, *pos + dir.offset(), BUS_WIDTH, pal().sch_bus),
                SchGhost::Junction(pos) => p.circle(l.wires, *pos, JUNCTION_RADIUS, pal().sch_junction),
                SchGhost::NoConnect(pos) => draw_no_connect(*pos, l, &mut p),
                SchGhost::PowerFlag(pos, rotation) => draw_power_flag(*pos, *rotation, l, &mut p),
                SchGhost::Target(pos) => {
                    p.push(l.wires, Instance::circle_outline(mmf(*pos), 0.6, 0.12, pal().hover.with_alpha(0xc0)))
                }
                SchGhost::Label(lab) => draw_label(lab, l, &mut p),
                SchGhost::Text(t) => p.text(l.text, t, pal().sch_text.with_alpha(0x90), &|s| s.to_string(), false),
                SchGhost::Sheet { rect, name } => {
                    let sheet = Sheet {
                        id: Id::from_u128(0),
                        name: name.clone(),
                        parent: None,
                        pos: rect.min,
                        size: Size { w: rect.width(), h: rect.height() },
                        page: Page::default(),
                        title_block: None,
                    };
                    draw_sheet_symbol(&project.schematic, &sheet, l, &mut p);
                }
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
                    let footprint = variant.and_then(|v| project.library.footprints.get(v.footprint?));
                    draw_symbol_graphics(
                        sym,
                        *unit,
                        transform,
                        &refdes,
                        value,
                        variant.zip(footprint),
                        true,
                        l,
                        &mut p,
                    );
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
    pub fn set_board_preview(&mut self, project: &Project, ghosts: &[PcbGhost]) {
        if ghosts.is_empty() {
            self.scene.remove_item(PREVIEW_ITEM);
            return;
        }
        let layer = self.hover_layer;
        let mut out = Vec::new();
        for g in ghosts {
            match *g {
                PcbGhost::Footprint { part, pos, rotation, side } => {
                    let Layers::Board(l) = &self.layers else { continue };
                    let Some(part) = project.library.parts.get(part) else { continue };
                    let fp = part.variants.first().and_then(|v| project.library.footprints.get(v.footprint?));
                    let Some(fp) = fp else { continue };
                    let mut p = Painter::default();
                    let t = Transform::new(pos, rotation, side == Side::Bottom);
                    let refdes = format!("{}?", part.prefix);
                    draw_footprint_at(project, fp, &t, side, &refdes, &part.default_value, true, l, &mut p);
                    // Translucent, on top of everything.
                    out.extend(p.out.into_iter().map(|(_, mut inst)| {
                        inst.color = inst.color.with_alpha((inst.color.0[3] as u16 * 3 / 4) as u8);
                        (layer, inst)
                    }));
                }
                PcbGhost::Zone { layer: l, ref points } => {
                    let color = board_layer_color(l).with_alpha(0xc0);
                    let n = points.len();
                    for i in 0..n {
                        let w = if i + 1 == n { 0.05 } else { 0.1 };
                        out.push((layer, Instance::line(mmf(points[i]), mmf(points[(i + 1) % n]), w, color)));
                    }
                }
                PcbGhost::Track { layer: l, a, b, mid, width, bad } => {
                    let color = if bad { pal().flagged } else { board_layer_color(l) }.with_alpha(0xc0);
                    let arc = mid.and_then(|m| path_instance(&[a, m, b], width / 2, 0.0, color));
                    out.push((
                        layer,
                        arc.unwrap_or_else(|| Instance::line(mmf(a), mmf(b), (width as f64 * 1e-6) as f32, color)),
                    ));
                }
                PcbGhost::Via { pos, diameter, bad } => {
                    let color = if bad { pal().flagged } else { pal().via };
                    out.push((
                        layer,
                        Instance::circle(mmf(pos), (diameter as f64 * 0.5e-6) as f32, color.with_alpha(0xc0)),
                    ))
                }
                PcbGhost::Clash(p) => {
                    // A ring with a cross: where the rule is broken.
                    let c = pal().flagged;
                    out.push((layer, Instance::circle_outline(mmf(p), 0.6, 0.12, c)));
                    for (dx, dy) in [(0.35, 0.35), (0.35, -0.35)] {
                        let (x, y) = (p.x as f32 * 1e-6, p.y as f32 * 1e-6);
                        out.push((layer, Instance::line([x - dx, y - dy], [x + dx, y + dy], 0.12, c)));
                    }
                }
                PcbGhost::Outline(r) => {
                    let c = [r.min, Point::new(r.max.x, r.min.y), r.max, Point::new(r.min.x, r.max.y)];
                    let color = board_layer_color(LayerId::OUTLINE);
                    for i in 0..4 {
                        out.push((layer, Instance::line(mmf(c[i]), mmf(c[(i + 1) % 4]), 0.1, color)));
                    }
                }
                PcbGhost::Text { layer: l, ref text } => {
                    let mut p = Painter::default();
                    p.text(layer, text, board_layer_color(l).with_alpha(0x90), &|s| s.to_string(), false);
                    out.extend(p.out);
                }
                PcbGhost::Target(p) => {
                    out.push((layer, Instance::circle_outline(mmf(p), 0.5, 0.08, pal().hover.with_alpha(0xc0))))
                }
            }
        }
        self.scene.set_item(PREVIEW_ITEM, out);
    }

    /// Draw the ratsnest and refresh net highlights (board only).
    pub fn set_copper(&mut self, _project: &Project, c: &BoardConnectivity) {
        self.labels = None;
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
        out.extend(c.ratsnest.iter().map(|r| (l.ratsnest, Instance::line(mmf(r.a), mmf(r.b), 0.05, pal().ratsnest))));
        // Flagged copper: a halo around it.
        for item in c.flagged() {
            match &item.geo {
                Geo::Capsule { a, b, r } => {
                    let w = (*r as f64 * 2e-6) as f32 + 0.3;
                    out.push((l.ratsnest, Instance::line(mmf(*a), mmf(*b), w, pal().flagged)));
                }
                Geo::Path { points, r } => {
                    out.extend(path_instance(points, *r, 0.3, pal().flagged).map(|i| (l.ratsnest, i)))
                }
                _ => {}
            }
        }
        self.scene.set_item(CONNECTIVITY_ITEM, out);
        let nets = std::mem::take(&mut self.nets);
        self.set_nets(c, &nets);
    }

    /// Draw the net labels of what's near `visible` (the viewport) at `zoom` (pixels per
    /// mm), if they're out of date: drawn for an area the viewport left, at another zoom
    /// step, before the board, its copper or a layer's visibility changed. Board only;
    /// call it every frame. Returns true if they were redrawn.
    pub fn update_net_labels(&mut self, project: &Project, c: &BoardConnectivity, visible: Rect, zoom: f64) -> bool {
        use crate::net_labels::{LABEL_ZOOM, PAD_LABEL_ZOOM, pad_labels, track_labels};
        let Layers::Board(l) = &self.layers else { return false };
        if zoom < LABEL_ZOOM || !zoom.is_finite() {
            let drawn = self.labels.take().is_some();
            if drawn {
                self.scene.remove_item(NET_LABEL_ITEM);
            }
            return drawn;
        }
        let step = (zoom.ln() / 1.15f64.ln()).floor() as i32;
        let shown = |r: RLayer| self.scene.layer(r).visible;
        let mut bits = 0u128;
        for (i, id) in l.copper.iter().enumerate() {
            bits |= (shown(l.map[id]) as u128) << i;
        }
        bits |= (shown(l.pad_numbers) as u128) << 127;
        let inside = |a: &Rect| a.contains(visible.min) && a.contains(visible.max);
        if self.labels.is_some_and(|d| d.step == step && d.visible == bits && inside(&d.area)) {
            return false;
        }
        // Half a viewport around it: a pan redraws them now and then, not every frame.
        let area = visible.grow(visible.width().max(visible.height()) / 2);
        let mut out: Vec<(RLayer, Instance)> = Vec::new();
        let mut buf = Vec::new();
        for id in self.index.in_rect(area, true) {
            match id {
                ObjectId::Track(t) => {
                    let Some(tr) = project.board.tracks.get(t) else { continue };
                    let Some(&layer) = l.track_labels.get(&tr.layer) else { continue };
                    if !shown(l.map[&tr.layer]) {
                        continue;
                    }
                    let Some(name) = c.net_of(editor_core::copper::CopperItem::Track(t)) else { continue };
                    // Not over pads (a track's ends usually sit in one).
                    let on_pad = |g: &Geo| {
                        c.query(g.bounds()).any(|p| {
                            matches!(p.item, editor_core::copper::CopperItem::Pad { .. })
                                && p.layers & editor_core::copper::layer_bit(tr.layer) != 0
                                && p.geo.touches(g)
                        })
                    };
                    track_labels(tr, name, area, zoom, &on_pad, &mut buf);
                    out.extend(buf.drain(..).map(|i| (layer, i)));
                }
                ObjectId::Footprint(f) if zoom >= PAD_LABEL_ZOOM && shown(l.pad_numbers) => {
                    let Some(inst) = project.board.footprints.get(f) else { continue };
                    let Some(fp) = footprint_of_instance(project, inst) else { continue };
                    let net = |pad| c.net_of(editor_core::copper::CopperItem::Pad { footprint: f, pad });
                    pad_labels(fp, inst, net, pad_number_limit(fp), zoom, &mut buf);
                    out.extend(buf.drain(..).map(|i| (l.pad_labels, i)));
                }
                _ => {}
            }
        }
        self.scene.set_item(NET_LABEL_ITEM, out);
        self.labels = Some(LabelsDrawn { area, step, visible: bits });
        true
    }

    /// Mark design rule violations (board only; empty clears).
    pub fn set_drc(&mut self, violations: &[editor_core::drc::Violation]) {
        self.drc = violations.iter().filter(|v| !v.excluded).map(|v| (v.pos, v.severity)).collect();
        self.draw_drc();
    }

    fn draw_drc(&mut self) {
        let Layers::Board(l) = &self.layers else { return };
        if self.drc.is_empty() {
            self.scene.remove_item(DRC_ITEM);
            return;
        }
        let layer = l.drc;
        let mut out = Vec::new();
        for &(p, severity) in &self.drc {
            push_marker(&mut out, layer, p, severity);
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
                    out.push((layer, Instance::line(mmf(*a), mmf(*b), (*r as f64 * 2e-6) as f32, pal().net_highlight)))
                }
                Geo::Path { points, r } => {
                    out.extend(path_instance(points, *r, 0.0, pal().net_highlight).map(|i| (layer, i)))
                }
                Geo::Polygon(points) => {
                    let pts: Vec<[f32; 2]> = points.iter().map(|&p| mmf(p)).collect();
                    out.extend(render::tess::fill_polygon(&pts, pal().net_highlight).into_iter().map(|i| (layer, i)));
                }
                Geo::Region { outer, holes } => {
                    let shape: Vec<Vec<Point>> = std::iter::once(outer).chain(holes).cloned().collect();
                    out.extend(region_instances(&shape, pal().net_highlight).into_iter().map(|i| (layer, i)));
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

    /// The layer of symbol and sheet fills: left out of highlights (which cover the
    /// body with a translucent rectangle instead).
    fn fill_layer(&self) -> Option<RLayer> {
        match &self.layers {
            Layers::Sch(l) => Some(l.body_fill),
            _ => None,
        }
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
            let fill = self.fill_layer();
            out.extend(p.out.into_iter().filter(|(l, _)| Some(*l) != fill).map(|(_, mut inst)| {
                inst.color = pal().probe;
                (layer, inst)
            }));
            if matches!(id, ObjectId::Symbol(_) | ObjectId::Footprint(_) | ObjectId::Sheet(_))
                && let Some(b) = p.bounds
            {
                let size = [(b.width() as f64 * 1e-6) as f32, (b.height() as f64 * 1e-6) as f32];
                out.push((layer, Instance::rect(mmf(b.center()), size, 0.0, 0.0, pal().probe_fill)));
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
        let (layer, color) =
            if hover { (self.hover_layer, pal().hover) } else { (self.selection_layer, pal().selection) };
        let fill = self.fill_layer();
        // Pad numbers stay as they are (a copy would sit beside the net labels' ones).
        let numbers = match &self.layers {
            Layers::Board(l) => Some(l.pad_numbers),
            Layers::Sch(_) => None,
        };
        let mut out: Vec<(RLayer, Instance)> = p
            .out
            .into_iter()
            .filter(|(l, _)| Some(*l) != fill && Some(*l) != numbers)
            .map(|(_, mut inst)| {
                inst.color = color;
                (layer, inst)
            })
            .collect();
        if matches!(id, ObjectId::Symbol(_) | ObjectId::Footprint(_) | ObjectId::Sheet(_))
            && let Some(b) = p.bounds
        {
            let fill = if hover { pal().hover.with_alpha(0x18) } else { pal().selection_fill };
            let size = [(b.width() as f64 * 1e-6) as f32, (b.height() as f64 * 1e-6) as f32];
            out.push((layer, Instance::rect(mmf(b.center()), size, 0.0, 0.0, fill)));
        }
        // A selected reference or value text shows its owner, tied to it by a thin line.
        if !hover && let Some((owner, origin, at)) = field_owner(project, id) {
            let mut q = Painter::default();
            if self.draw_object(project, owner, &mut q) {
                let color = pal().hover.with_alpha(0x90);
                out.extend(q.out.into_iter().filter(|(l, _)| Some(*l) != fill).map(|(_, mut inst)| {
                    inst.color = color;
                    (self.hover_layer, inst)
                }));
                out.push((self.hover_layer, Instance::line(mmf(origin), mmf(at), 0.05, color)));
            }
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
                ObjectId::BusWire(w) => project.schematic.bus_wires.get(w).map(|w| (w.a, w.b, BUS_WIDTH)),
                ObjectId::BusEntry(e) => project.schematic.bus_entries.get(e).map(|e| (e.pos, e.wire_end(), BUS_WIDTH)),
                ObjectId::Track(t) => {
                    project.board.tracks.get(t).filter(|t| t.mid.is_none()).map(|t| (t.a, t.b, t.width))
                }
                _ => None,
            }
            .map(|(a, b, width)| seg_dist(p, a, b) - width as f64 / 2.0)
            .or_else(|| match id {
                ObjectId::Junction(j) => {
                    project.schematic.junctions.get(j).map(|j| dist(p, j.pos) - JUNCTION_RADIUS as f64)
                }
                ObjectId::NoConnect(n) => {
                    project.schematic.no_connects.get(n).map(|n| dist(p, n.pos) - NO_CONNECT_HALF as f64)
                }
                ObjectId::PowerFlag(f) => project.schematic.power_flags.get(f).map(|f| {
                    let t = Transform::new(f.pos, f.rotation, false);
                    let tip = t.point(Point::mm(0.0, POWER_FLAG_HEIGHT));
                    let flag = t.point(Point::mm(0.9, POWER_FLAG_HEIGHT - 0.6));
                    seg_dist(p, f.pos, tip).min(dist(p, flag) - mm(0.7) as f64)
                }),
                ObjectId::Via(v) => project.board.vias.get(v).map(|v| dist(p, v.pos) - v.diameter as f64 / 2.0),
                ObjectId::Track(t) => project.board.tracks.get(t).map(|t| {
                    t.path(10_000).windows(2).map(|w| seg_dist(p, w[0], w[1])).fold(f64::MAX, f64::min)
                        - t.width as f64 / 2.0
                }),
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

    /// The object's own text as drawn (a label's name, a field's reference) and its colour.
    pub fn object_text(&self, project: &Project, id: ObjectId) -> Option<(Text, Color)> {
        let mut p = Painter::default();
        self.draw_object(project, id, &mut p);
        p.texts.into_iter().next()
    }

    /// Show a text being edited in place (`None` clears): a box over the original, the
    /// selection, the text and the caret.
    pub fn set_text_edit(&mut self, edit: Option<(&crate::text_edit::EditGeometry, Color)>) {
        let Some((g, color)) = edit else {
            self.scene.remove_item(TEXT_EDIT_ITEM);
            return;
        };
        let mut p = Painter::default();
        crate::text_edit::draw(&mut p, self.hover_layer, g, color);
        self.scene.set_item(TEXT_EDIT_ITEM, p.out);
    }

    fn draw_object(&self, project: &Project, id: ObjectId, p: &mut Painter) -> bool {
        if self.kind == ViewKind::Schematic && project.schematic.sheet_of(id) != self.id.sheet() {
            return false; // on another sheet (or not a schematic object)
        }
        match (&self.layers, id) {
            (Layers::Sch(l), ObjectId::Sheet(i)) => {
                project.schematic.sheets.get(i).map(|s| draw_sheet_symbol(&project.schematic, s, l, p)).is_some()
            }
            (Layers::Sch(l), ObjectId::Symbol(i)) => {
                project.schematic.symbols.get(i).map(|s| draw_symbol(project, s, l, p)).is_some()
            }
            (Layers::Sch(l), ObjectId::Wire(i)) => {
                project.schematic.wires.get(i).map(|w| p.line(l.wires, w.a, w.b, WIRE_WIDTH, pal().sch_wire)).is_some()
            }
            (Layers::Sch(l), ObjectId::BusWire(i)) => project
                .schematic
                .bus_wires
                .get(i)
                .map(|w| p.line(l.wires, w.a, w.b, BUS_WIDTH, pal().sch_bus))
                .is_some(),
            (Layers::Sch(l), ObjectId::BusEntry(i)) => project
                .schematic
                .bus_entries
                .get(i)
                .map(|e| p.line(l.wires, e.pos, e.wire_end(), BUS_WIDTH, pal().sch_bus))
                .is_some(),
            (Layers::Sch(l), ObjectId::Junction(i)) => project
                .schematic
                .junctions
                .get(i)
                .map(|j| p.circle(l.wires, j.pos, JUNCTION_RADIUS, pal().sch_junction))
                .is_some(),
            (Layers::Sch(l), ObjectId::NoConnect(i)) => {
                project.schematic.no_connects.get(i).map(|n| draw_no_connect(n.pos, l, p)).is_some()
            }
            (Layers::Sch(l), ObjectId::PowerFlag(i)) => {
                project.schematic.power_flags.get(i).map(|f| draw_power_flag(f.pos, f.rotation, l, p)).is_some()
            }
            (Layers::Sch(l), ObjectId::Label(i)) => {
                project.schematic.labels.get(i).map(|lab| draw_label(lab, l, p)).is_some()
            }
            (Layers::Sch(l), ObjectId::SchText(i)) => project
                .schematic
                .texts
                .get(i)
                .map(|t| p.text(l.text, &t.text, pal().sch_text, &|s| s.to_string(), true))
                .is_some(),
            (Layers::Board(l), ObjectId::Footprint(i)) => {
                project.board.footprints.get(i).map(|f| draw_footprint(project, f, l, p)).is_some()
            }
            (Layers::Sch(l), ObjectId::SymbolField(i)) => {
                project.schematic.fields.get(i).is_some_and(|f| draw_symbol_field(project, f, l, p))
            }
            (Layers::Board(l), ObjectId::FootprintField(i)) => {
                project.board.fields.get(i).is_some_and(|f| draw_field(project, f, l, p))
            }
            (Layers::Board(l), ObjectId::Track(i)) => project
                .board
                .tracks
                .get(i)
                .map(|t| {
                    if let Some(&layer) = l.map.get(&t.layer) {
                        match t.arc() {
                            Some(arc) => p.shape(layer, &arc, board_layer_color(t.layer), None),
                            None => p.line(layer, t.a, t.b, t.width, board_layer_color(t.layer)),
                        }
                    }
                })
                .is_some(),
            (Layers::Board(l), ObjectId::Via(i)) => project
                .board
                .vias
                .get(i)
                .map(|v| {
                    p.circle(l.vias, v.pos, v.diameter / 2, pal().via);
                    p.push(l.holes, Instance::circle(mmf(v.pos), (v.drill as f64 * 0.5e-6) as f32, pal().hole));
                })
                .is_some(),
            (Layers::Board(l), ObjectId::Zone(i)) => project
                .board
                .zones
                .get(i)
                .map(|z| {
                    // The outline and holes; fills are derived (see `set_copper`).
                    if let Some(&layer) = l.map.get(&z.layer) {
                        for ring in std::iter::once(&z.outline).chain(&z.holes) {
                            let shape = Shape::Polygon { points: ring.clone(), width: 100_000, fill: false };
                            p.shape(layer, &shape, board_layer_color(z.layer).with_alpha(0xa0), None);
                        }
                    }
                })
                .is_some(),
            (Layers::Board(l), ObjectId::Keepout(i)) => project
                .board
                .keepouts
                .get(i)
                .map(|k| {
                    // Outline and holes, hatched inside, on every copper layer it covers.
                    let hatch = keepout_hatch(&k.outline, &k.holes, KEEPOUT_HATCH);
                    for bl in project.board.copper_layers().filter(|bl| k.on_layer(bl.id)) {
                        let Some(&layer) = l.map.get(&bl.id) else { continue };
                        let color = board_layer_color(bl.id).with_alpha(0xc0);
                        for ring in std::iter::once(&k.outline).chain(&k.holes) {
                            let shape = Shape::Polygon { points: ring.clone(), width: 100_000, fill: false };
                            p.shape(layer, &shape, color, None);
                        }
                        for &(a, b) in &hatch {
                            p.line(layer, a, b, 50_000, color.with_alpha(0x80));
                        }
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

/// The owner of a reference or value text (symbol or footprint), its origin and where
/// the text is.
fn field_owner(project: &Project, id: ObjectId) -> Option<(ObjectId, Point, Point)> {
    match id {
        ObjectId::SymbolField(i) => {
            let f = project.schematic.fields.get(i)?;
            let s = project.schematic.symbols.get(f.symbol)?;
            Some((ObjectId::Symbol(s.id), s.pos, s.transform().point(f.text.pos)))
        }
        ObjectId::FootprintField(i) => {
            let f = project.board.fields.get(i)?;
            let fp = project.board.footprints.get(f.footprint)?;
            Some((ObjectId::Footprint(fp.id), fp.pos, fp.transform().point(f.text.pos)))
        }
        _ => None,
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
    let part = component.and_then(|c| project.library.parts.get(c.part));
    let (refdes, value) = component.map_or(("?", ""), |c| (c.refdes.as_str(), c.value.as_str()));
    // Units of a multi-unit part show which one they are: U1-A.
    let refdes = part.map_or_else(|| refdes.to_string(), |part| part.unit_reference(refdes, inst.unit));
    let refdes = refdes.as_str();
    // Pad names per pin for the component's variant.
    let variant = component.and_then(|c| part.and_then(|part| part.variant(c.variant)));
    let footprint = component.and_then(|c| project.library.footprint_of(c.part, c.variant));
    let pads = variant.zip(footprint);
    let start = p.out.len();
    draw_symbol_graphics(sym, inst.unit, &inst.transform(), refdes, value, pads, false, l, p);
    if component.is_some_and(|c| c.dnp) {
        p.fade(start);
        // A cross over the body, as KiCad marks it.
        let t = inst.transform();
        if let Some(b) = Rect::union_all(sym.graphics.iter().map(|g| g.transformed(&t).bounds())) {
            let w = WIRE_WIDTH * 2;
            let saved = p.bounds;
            p.line(l.body, b.min, b.max, w, pal().sch_dnp);
            p.line(l.body, Point::new(b.min.x, b.max.y), Point::new(b.max.x, b.min.y), w, pal().sch_dnp);
            p.bounds = saved;
        }
    }
}

/// Alpha of a "do not populate" component's drawing.
const DNP_ALPHA: f32 = 0.35;

impl Painter {
    /// Fade what was drawn since `start` (a "do not populate" component).
    fn fade(&mut self, start: usize) {
        for (_, inst) in &mut self.out[start..] {
            let a = inst.color.0[3];
            inst.color.0[3] = (a as f32 * DNP_ALPHA).round() as u8;
        }
    }
}

fn is_dnp(project: &Project, component: Id) -> bool {
    project.schematic.components.get(component).is_some_and(|c| c.dnp)
}

/// The displayed reference of a symbol's component (`U1-A` for units).
fn symbol_refdes(project: &Project, inst: &SymbolInstance) -> (String, String) {
    let component = project.schematic.components.get(inst.component);
    let part = component.and_then(|c| project.library.parts.get(c.part));
    let (refdes, value) = component.map_or(("?", ""), |c| (c.refdes.as_str(), c.value.as_str()));
    let refdes = part.map_or_else(|| refdes.to_string(), |part| part.unit_reference(refdes, inst.unit));
    (refdes, value.to_string())
}

/// A symbol's reference or value text, upright.
fn draw_symbol_field(project: &Project, field: &SymbolField, l: &SchLayers, p: &mut Painter) -> bool {
    let Some(inst) = project.schematic.symbols.get(field.symbol) else { return false };
    if field.hidden {
        return false;
    }
    let (refdes, value) = symbol_refdes(project, inst);
    let text = field.text.transformed(&inst.transform()).upright();
    let start = p.out.len();
    p.text(l.text, &text, pal().sch_text, &|s| substitute(s, &refdes, &value), true);
    if is_dnp(project, inst.component) {
        p.fade(start);
    }
    true
}

#[allow(clippy::too_many_arguments)]
fn draw_symbol_graphics(
    sym: &Symbol,
    unit: u32,
    t: &Transform,
    refdes: &str,
    value: &str,
    pads: Option<(&PartVariant, &Footprint)>,
    with_fields: bool,
    l: &SchLayers,
    p: &mut Painter,
) {
    let t = *t;
    for shape in &sym.graphics {
        p.shape(l.body, &shape.transformed(&t), pal().sch_body, Some((l.body_fill, pal().sch_body_fill)));
    }

    for pin in sym.pins.iter() {
        let (a, b) = (t.point(pin.pos), t.point(pin.inner_end()));
        p.line(l.pins, a, b, WIRE_WIDTH, pal().sch_pin);
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
                p.text(l.pin_text, &text, pal().sch_pin_text, &|s| s.to_string(), false);
            }
        }
        if pin.show_name {
            let inward = if horizontal { d.x > 0 } else { d.y > 0 };
            let step =
                if horizontal { Point::new(d.x.signum() * 500_000, 0) } else { Point::new(0, d.y.signum() * 500_000) };
            let mut text = Text::new(pin.name.clone(), b + step, 1_270_000)
                .aligned(if inward { HAlign::Left } else { HAlign::Right }, VAlign::Center);
            text.rotation = if horizontal { Angle::ZERO } else { Angle::DEG90 };
            p.text(l.body, &text, pal().sch_body, &|s| s.to_string(), false);
        }
    }

    let sideways = t.rotation.quarter_turns().is_some_and(|q| q % 2 == 1);
    let body = Rect::union_all(sym.graphics.iter().map(|g| g.transformed(&t).bounds()));
    for text in &sym.texts {
        // Placed symbols have their own reference and value texts ([`draw_symbol_field`]).
        if !with_fields && model::geom::is_field_text(&text.text) {
            continue;
        }
        if model::geom::is_field_text(&text.text) {
            // As the placed symbol's fields will be.
            let text = text.transformed(&t).upright();
            p.text(l.text, &text, pal().sch_text, &|s| substitute(s, refdes, value), false);
            continue;
        }
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
        p.text(l.text, &text, pal().sch_text, &|s| substitute(s, refdes, value), false);
    }
}

/// A sheet symbol: a filled rectangle with the sheet's name above its top left corner.
/// A sheet symbol: the rectangle with the sheet's name above it, and a pin for each of
/// the sheet's hierarchical labels (drawn like one, pointing the other way, with the name
/// inside the rectangle).
fn draw_sheet_symbol(s: &Schematic, sheet: &Sheet, l: &SchLayers, p: &mut Painter) {
    let (r, pins) = sheet.layout(&s.hier_ports(sheet.id));
    let shape = Shape::Rect { a: r.min, b: r.max, width: 254_000, fill: true };
    p.shape(l.body, &shape, pal().sch_sheet, Some((l.body_fill, pal().sch_sheet_fill)));
    p.grow(r);
    let name = Text::new(sheet.name.clone(), Point::new(r.min.x, r.max.y + 500_000), 1_270_000);
    p.text(l.body, &name, pal().sch_sheet, &|s| s.to_string(), true);
    for pin in pins {
        let lab = NetLabel {
            id: sheet.id,
            sheet: sheet.id,
            pos: pin.pos,
            name: pin.name,
            kind: LabelKind::Hierarchical,
            shape: pin.shape.reversed(),
            style: None,
            rotation: if pin.right { Angle::DEG180 } else { Angle::ZERO },
        };
        draw_label(&lab, l, p);
    }
}

/// Half the size of a no-connect flag's X.
const NO_CONNECT_HALF: Coord = 635_000;

/// A no-connect flag: an X centred on the pin end.
fn draw_no_connect(pos: Point, l: &SchLayers, p: &mut Painter) {
    let h = NO_CONNECT_HALF;
    for (a, b) in [(Point::new(-h, -h), Point::new(h, h)), (Point::new(-h, h), Point::new(h, -h))] {
        p.line(l.labels, pos + a, pos + b, WIRE_WIDTH, pal().sch_no_connect);
    }
}

/// A finding's marker: a circled X in the error or warning colour.
fn push_marker(out: &mut Vec<(RLayer, Instance)>, layer: RLayer, p: Point, severity: Severity) {
    let color = if severity == Severity::Error { pal().drc_marker } else { pal().warning_marker };
    let c = mmf(p);
    out.push((layer, Instance::circle_outline(c, 0.6, 0.12, color)));
    let d = 0.35;
    out.push((layer, Instance::line([c[0] - d, c[1] - d], [c[0] + d, c[1] + d], 0.12, color)));
    out.push((layer, Instance::line([c[0] - d, c[1] + d], [c[0] + d, c[1] - d], 0.12, color)));
}

/// Height of a power flag's stem, in mm.
const POWER_FLAG_HEIGHT: f64 = 2.54;

/// A power flag: a stem from the attach point with a pennant at its end.
fn draw_power_flag(pos: Point, rotation: Angle, l: &SchLayers, p: &mut Painter) {
    let t = Transform::new(pos, rotation, false);
    let c = pal().sch_power;
    let h = POWER_FLAG_HEIGHT;
    let pts = [(0.0, 0.0), (0.0, h), (1.8, h - 0.6), (0.0, h - 1.2)];
    for w in pts.windows(2) {
        p.line(l.labels, t.point(Point::mm(w[0].0, w[0].1)), t.point(Point::mm(w[1].0, w[1].1)), WIRE_WIDTH, c);
    }
    p.fill(l.labels, pts[1..].iter().map(|&(x, y)| t.point(Point::mm(x, y))), c.with_alpha(0x60));
}

/// Half the height of a global or hierarchical label's outline, in mm.
const LABEL_HALF_HEIGHT: f64 = 1.0;

fn draw_label(lab: &NetLabel, l: &SchLayers, p: &mut Painter) {
    let t = Transform::new(lab.pos, lab.rotation, false);
    let line = |p: &mut Painter, a: (f64, f64), b: (f64, f64), color: Color| {
        p.line(l.labels, t.point(Point::mm(a.0, a.1)), t.point(Point::mm(b.0, b.1)), WIRE_WIDTH, color)
    };
    match lab.kind {
        LabelKind::Local => {
            p.circle(l.labels, lab.pos, 200_000, pal().sch_label);
            let text = Text::new(lab.name.clone(), Point::mm(0.4, 0.3), 1_270_000).transformed(&t).upright();
            p.text(l.labels, &text, pal().sch_label, &|s| s.to_string(), true);
        }
        LabelKind::Global | LabelKind::Hierarchical => {
            // KiCad style: a global label is an outline around its name, a hierarchical one a
            // small arrow in front of it; the shape says which ends are pointed.
            let global = lab.kind == LabelKind::Global;
            let color = if global { pal().sch_global_label } else { pal().sch_hier_label };
            let (near, far) = lab.shape.points();
            let h = LABEL_HALF_HEIGHT;
            let tip = |pointed: bool| if pointed { h } else { 0.0 };
            let width = font::text_width(&lab.name, 1.27) as f64;
            let (end, text_x) = if global {
                (tip(near) + 0.4 + width + 0.4 + tip(far), tip(near) + 0.4)
            } else {
                (2.0 * h, 2.0 * h + 0.4)
            };
            let mut outline = vec![(tip(near), h), (end - tip(far), h)];
            if far {
                outline.push((end, 0.0));
            }
            outline.extend([(end - tip(far), -h), (tip(near), -h)]);
            if near {
                outline.push((0.0, 0.0));
            }
            outline.push(outline[0]);
            for w in outline.windows(2) {
                line(p, w[0], w[1], color);
            }
            let text = Text::new(lab.name.clone(), Point::mm(text_x, 0.0), 1_270_000)
                .aligned(HAlign::Left, VAlign::Center)
                .transformed(&t)
                .upright();
            p.text(l.labels, &text, color, &|s| s.to_string(), true);
        }
        LabelKind::Power => {
            let c = pal().sch_power;
            let ring = |p: &mut Painter, y: f64, r: f64| {
                let shape =
                    Shape::Circle { center: t.point(Point::mm(0.0, y)), radius: mm(r), width: WIRE_WIDTH, fill: false };
                p.shape(l.labels, &shape, c, None);
            };
            let style = lab.power_style();
            // Where the name goes: past the glyph's far end.
            let text_y = match style {
                PowerStyle::Bar => {
                    line(p, (0.0, 0.0), (0.0, 1.27), c);
                    line(p, (-1.27, 1.27), (1.27, 1.27), c);
                    1.8
                }
                PowerStyle::Arrow => {
                    line(p, (0.0, 0.0), (0.0, 2.54), c);
                    line(p, (-0.76, 1.52), (0.0, 2.54), c);
                    line(p, (0.76, 1.52), (0.0, 2.54), c);
                    3.05
                }
                PowerStyle::Circle => {
                    line(p, (0.0, 0.0), (0.0, 1.27), c);
                    ring(p, 1.905, 0.635);
                    3.05
                }
                PowerStyle::Ground => {
                    line(p, (0.0, 0.0), (0.0, -1.27), c);
                    for (y, w) in [(-1.27, 1.27), (-1.78, 0.76), (-2.29, 0.25)] {
                        line(p, (-w, y), (w, y), c);
                    }
                    -2.8
                }
                PowerStyle::SignalGround => {
                    line(p, (0.0, 0.0), (0.0, -1.27), c);
                    let tri = [(-1.27, -1.27), (1.27, -1.27), (0.0, -2.54), (-1.27, -1.27)];
                    for w in tri.windows(2) {
                        line(p, w[0], w[1], c);
                    }
                    -3.05
                }
                PowerStyle::Chassis => {
                    line(p, (0.0, 0.0), (0.0, -1.27), c);
                    line(p, (-1.27, -1.27), (1.27, -1.27), c);
                    for x in [-1.27, 0.0, 1.27] {
                        line(p, (x, -1.27), (x - 0.76, -2.29), c);
                    }
                    -2.8
                }
                PowerStyle::ProtectiveEarth => {
                    line(p, (0.0, 0.0), (0.0, -1.65), c);
                    for (y, w) in [(-1.65, 0.76), (-2.08, 0.46), (-2.51, 0.15)] {
                        line(p, (-w, y), (w, y), c);
                    }
                    ring(p, -2.03, 1.27);
                    -3.6
                }
            };
            let v = if style.points_down() { VAlign::Top } else { VAlign::Bottom };
            let text = Text::new(lab.name.clone(), Point::mm(0.0, text_y), 1_270_000)
                .aligned(HAlign::Center, v)
                .transformed(&t);
            p.text(l.labels, &text, c, &|s| s.to_string(), true);
        }
    }
}

fn draw_footprint(project: &Project, inst: &FootprintInstance, l: &BoardLayers, p: &mut Painter) {
    let Some(fp) = footprint_of_instance(project, inst) else { return };
    let component = project.schematic.components.get(inst.component);
    let (refdes, value) = component.map_or(("?", ""), |c| (c.refdes.as_str(), c.value.as_str()));
    let start = p.out.len();
    draw_footprint_at(project, fp, &inst.transform(), inst.side, refdes, value, false, l, p);
    if component.is_some_and(|c| c.dnp) {
        p.fade(start);
    }
}

/// A footprint's reference or value text; nothing if hidden.
fn draw_field(project: &Project, field: &FootprintField, l: &BoardLayers, p: &mut Painter) -> bool {
    let Some(inst) = project.board.footprints.get(field.footprint) else { return false };
    if field.hidden {
        return false;
    }
    let component = project.schematic.components.get(inst.component);
    let (refdes, value) = component.map_or(("?", ""), |c| (c.refdes.as_str(), c.value.as_str()));
    let layer = field.board_layer(inst.side);
    let Some(&rl) = l.map.get(&layer) else { return false };
    let text = field.text.transformed(&inst.transform()).upright();
    let start = p.out.len();
    p.text(rl, &text, board_layer_color(layer), &|s| substitute(s, refdes, value), true);
    if is_dnp(project, inst.component) {
        p.fade(start);
    }
    true
}

/// The largest pad number on footprint `fp`: 70% of its reference text (else of 1 mm),
/// so a large pad (an exposed pad) doesn't get a number larger than the part's label.
fn pad_number_limit(fp: &Footprint) -> Coord {
    let reference = fp.texts.iter().find(|t| t.text.text == "${REF}").map_or(mm(1.0), |t| t.text.size);
    (reference as f64 * 0.7) as Coord
}

/// Draw footprint `fp` placed by `t` on `side` (also the ghost of a board part being
/// placed). The reference and value texts are drawn only `with_fields` (the ghost):
/// placed footprints have their own ([`draw_field`]).
#[allow(clippy::too_many_arguments)]
fn draw_footprint_at(
    project: &Project,
    fp: &Footprint,
    t: &Transform,
    side: Side,
    refdes: &str,
    value: &str,
    with_fields: bool,
    l: &BoardLayers,
    p: &mut Painter,
) {
    let t = *t;

    let mut body: Option<Rect> = None;
    let mut courtyard: Option<Rect> = None;
    let rules = &project.board.rules;
    let max_number = pad_number_limit(fp);
    for pad in fp.pads.iter() {
        let center = t.point(pad.pos);
        let mut draw = |layer: LayerId, margin: Coord| {
            if let Some(&rl) = l.map.get(&layer)
                && let Some(outline) = pad_outline(pad, &t, margin)
            {
                push_pad(p, rl, &outline, board_layer_color(layer));
            }
        };
        for layer in pad_copper_layers(pad, side, &l.copper) {
            draw(layer, 0);
        }
        for layer in pad_mask_layers(pad, side) {
            draw(layer, editor_core::fab::mask_margin(pad, rules));
        }
        if let Some(layer) = pad_paste_layer(pad, side) {
            draw(layer, editor_core::fab::paste_margin(pad, rules));
        }
        if let Some(h) = editor_core::fab::pad_hole(pad, &t) {
            if h.is_slot() {
                p.push(l.holes, Instance::line(mmf(h.a), mmf(h.b), (h.diameter as f64 * 1e-6) as f32, pal().hole));
            } else {
                p.push(l.holes, Instance::circle(mmf(center), (h.diameter as f64 * 0.5e-6) as f32, pal().hole));
            }
        }
        let r = Rect::around(center, pad.size.w.max(pad.size.h) / 2);
        body = Some(body.map_or(r, |b| b.union(r)));
        // Pad number, readable on top of the pad (large pads: no larger than the limit).
        let size = ((pad.size.w.min(pad.size.h) as f64 * 0.45) as Coord).min(max_number);
        let text = Text::new(pad.name.clone(), center, size).aligned(HAlign::Center, VAlign::Center);
        p.text(l.pad_numbers, &text, Color::rgba(0xff, 0xff, 0xff, 0xc0), &|s| s.to_string(), false);
    }

    for g in &fp.graphics {
        let layer = g.board_layer(side);
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
    for ft in fp.texts.iter().filter(|ft| with_fields || !ft.is_field()) {
        let layer = ft.board_layer(side);
        if let Some(&rl) = l.map.get(&layer) {
            p.text(
                rl,
                &ft.text.transformed(&t).upright(),
                board_layer_color(layer),
                &|s| substitute(s, refdes, value),
                false,
            );
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

/// A placed pad outline (see [`editor_core::fab`]).
pub(crate) fn push_pad(p: &mut Painter, rl: RLayer, outline: &PadOutline, color: Color) {
    let f = |c: Coord| (c as f64 * 1e-6) as f32;
    let rect = |c: Point, s: Size, r: f32, rot: Angle| {
        Instance::rect(mmf(c), [f(s.w), f(s.h)], r, rot.radians() as f32, color)
    };
    match outline {
        PadOutline::Circle { center, diameter } => {
            p.push(rl, Instance::circle(mmf(*center), f(*diameter) * 0.5, color))
        }
        PadOutline::Rect { center, size, rotation } => p.push(rl, rect(*center, *size, 0.0, *rotation)),
        PadOutline::Oval { center, size, rotation } => {
            p.push(rl, rect(*center, *size, f(size.w.min(size.h)) * 0.5, *rotation))
        }
        PadOutline::RoundRect { center, size, radius, rotation } => {
            p.push(rl, rect(*center, *size, f(*radius), *rotation))
        }
        PadOutline::Polygon(shapes) => {
            for shape in shapes {
                for i in region_instances(shape, color) {
                    p.push(rl, i);
                }
            }
        }
    }
}

/// A filled region: `[outline, holes…]`.
fn region_instances(shape: &[Vec<Point>], color: Color) -> Vec<Instance> {
    let contours: Vec<Vec<[f32; 2]>> = shape.iter().map(|c| c.iter().map(|&p| mmf(p)).collect()).collect();
    render::tess::fill_region(&contours, color)
}

/// Spacing of a keepout's hatch lines.
const KEEPOUT_HATCH: Coord = 1_000_000;

/// Diagonal hatch lines (rising at 45°) inside a polygon with holes (even-odd), `step`
/// apart.
pub fn keepout_hatch(outline: &[Point], holes: &[Vec<Point>], step: Coord) -> Vec<(Point, Point)> {
    let rings: Vec<&[Point]> = std::iter::once(outline).chain(holes.iter().map(Vec::as_slice)).collect();
    let edges: Vec<(Point, Point)> = rings
        .iter()
        .filter(|r| r.len() >= 3)
        .flat_map(|r| (0..r.len()).map(move |i| (r[i], r[(i + 1) % r.len()])))
        .collect();
    let Some((lo, hi)) = edges
        .iter()
        .map(|(a, _)| a.x - a.y)
        .fold(None, |m: Option<(Coord, Coord)>, u| Some(m.map_or((u, u), |(lo, hi)| (lo.min(u), hi.max(u)))))
    else {
        return Vec::new();
    };
    // Lines x - y = u; the parameter along a line is x + y.
    let mut out = Vec::new();
    let mut u = lo.div_euclid(step) * step + step;
    while u < hi && out.len() < 100_000 {
        let mut ts: Vec<f64> = edges
            .iter()
            .filter_map(|&(a, b)| {
                let (ua, ub) = ((a.x - a.y) as f64, (b.x - b.y) as f64);
                let uf = u as f64;
                if (ua <= uf) == (ub <= uf) {
                    return None;
                }
                let f = (uf - ua) / (ub - ua);
                Some((a.x + a.y) as f64 + f * ((b.x + b.y) - (a.x + a.y)) as f64)
            })
            .collect();
        ts.sort_by(f64::total_cmp);
        for pair in ts.as_chunks::<2>().0 {
            // x = (t + u) / 2, y = (t - u) / 2
            let at =
                |t: f64| Point::new(((t + u as f64) / 2.0).round() as Coord, ((t - u as f64) / 2.0).round() as Coord);
            out.push((at(pair[0]), at(pair[1])));
        }
        u += step;
    }
    out
}

/// One arc instance for an arc track's copper path (its points lie on the arc), `extra`
/// mm wider.
fn path_instance(points: &[Point], r: Coord, extra: f32, color: Color) -> Option<Instance> {
    let g = model::board::arc_through(points[0], points[points.len() / 2], *points.last()?)?;
    Some(Instance::arc(
        [(g.center.0 * 1e-6) as f32, (g.center.1 * 1e-6) as f32],
        (g.radius * 1e-6) as f32,
        g.start as f32,
        g.sweep as f32,
        (r as f64 * 2e-6) as f32 + extra,
        color,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pad_numbers_are_at_most_70_percent_of_the_reference() {
        let lib = model::builtin::builtin_library();
        let mut fp = lib.footprints.iter().find(|f| f.texts.iter().any(|t| t.text.text == "${REF}")).unwrap().clone();
        for t in fp.texts.iter_mut().filter(|t| t.text.text == "${REF}") {
            t.text.size = mm(1.5);
        }
        assert_eq!(pad_number_limit(&fp), mm(1.05));
        fp.texts.retain(|t| t.text.text != "${REF}");
        assert_eq!(pad_number_limit(&fp), mm(0.7), "without a reference: of 1 mm");
    }
}
