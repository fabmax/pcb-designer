//! Board creation tools: route tracks (45° or 90°), place vias, draw the board outline
//! copper zones and keepouts, measure, place board parts.
//! Independent of the GUI toolkit.

use editor_core::Command;
use editor_core::copper::CopperItem;
use model::*;

use crate::bend::{self, Posture};
pub use crate::bend::{Bend, Corners};
use crate::document::Document;
use crate::measure::{Measure, board_points};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PcbTool {
    Select,
    Route,
    Via,
    Outline,
    Zone,
    Keepout,
    Measure,
    /// Place board parts (see [`PcbPlacer::placing`]); started from the project tree.
    Place,
}

impl PcbTool {
    pub const ALL: &[PcbTool] = &[
        PcbTool::Select,
        PcbTool::Route,
        PcbTool::Via,
        PcbTool::Outline,
        PcbTool::Zone,
        PcbTool::Keepout,
        PcbTool::Measure,
    ];

    pub fn label(self) -> &'static str {
        match self {
            PcbTool::Select => "Select",
            PcbTool::Route => "Route",
            PcbTool::Via => "Via",
            PcbTool::Outline => "Outline",
            PcbTool::Zone => "Zone",
            PcbTool::Keepout => "Keepout",
            PcbTool::Measure => "Measure",
            PcbTool::Place => "Place",
        }
    }

    pub fn hint(self) -> &'static str {
        match self {
            PcbTool::Select => "Click/box to select · drag to move · R rotate · F flip side · Del delete",
            PcbTool::Route => {
                "Click a pad to start · click corners · ends on copper · L next layer (adds a via) · / corner · Shift+Space style · Esc done"
            }
            PcbTool::Via => "Click to place a via · Esc done",
            PcbTool::Outline => "Click two corners of the board outline · Esc done",
            PcbTool::Zone => "Click the zone's corners · click the first or last corner again to close · Esc cancels",
            PcbTool::Keepout => {
                "Click the keepout's corners · click the first or last corner again to close · Esc cancels"
            }
            PcbTool::Measure => crate::measure::HINT,
            PcbTool::Place => "Click to place · R rotate · F flip side · Esc done",
        }
    }
}

#[derive(Clone, Debug)]
pub struct PcbSettings {
    /// Active copper layer for routing.
    pub layer: LayerId,
    /// Track width; `None`: the design rules' width.
    pub width: Option<Coord>,
    pub corners: Corners,
    /// Which leg comes first (`/` flips it).
    pub bend: Bend,
    /// Net of new zones; empty: the net of the copper under the first corner.
    pub zone_net: String,
    /// New keepouts cover all copper layers (else the active one).
    pub keepout_all_layers: bool,
    /// Rotation and side of board parts being placed.
    pub rotation: Angle,
    pub side: Side,
}

impl Default for PcbSettings {
    fn default() -> Self {
        PcbSettings {
            layer: LayerId::TOP_COPPER,
            width: None,
            corners: Corners::Deg45,
            bend: Bend::Auto,
            zone_net: String::new(),
            keepout_all_layers: true,
            rotation: Angle::ZERO,
            side: Side::Top,
        }
    }
}

/// Something a tool would create, drawn by [`crate::View::set_board_preview`].
#[derive(Clone, Debug, PartialEq)]
pub enum PcbGhost {
    Track {
        layer: LayerId,
        a: Point,
        b: Point,
        width: Coord,
    },
    Via {
        pos: Point,
        diameter: Coord,
    },
    Outline(Rect),
    /// A zone being drawn (the last point is the cursor).
    Zone {
        layer: LayerId,
        points: Vec<Point>,
    },
    /// The cursor is on copper (a route would start or end there).
    Target(Point),
    /// A board part being placed (its first variant's footprint).
    Footprint {
        part: Id,
        pos: Point,
        rotation: Angle,
        side: Side,
    },
}

/// Active board tool and its in-progress state.
#[derive(Clone, Debug)]
pub struct PcbPlacer {
    tool: PcbTool,
    pub settings: PcbSettings,
    /// Start of the track segment being routed, or the first outline corner.
    start: Option<Point>,
    /// Net being routed, or of the zone being drawn.
    net: Option<String>,
    /// Corners of the zone being drawn.
    corners: Vec<Point>,
    cursor: Option<Point>,
    /// The cursor is on copper of the active layer.
    on_copper: bool,
    shown: Vec<PcbGhost>,
    pub measure: Measure,
    /// The board part placed by [`PcbTool::Place`].
    pub placing: Option<Id>,
    /// Snap nowhere (Alt held): set by the app before [`PcbPlacer::hover`].
    pub free: bool,
    /// Automatic posture of the track leg being routed.
    posture: Posture,
}

impl Default for PcbPlacer {
    fn default() -> Self {
        PcbPlacer {
            tool: PcbTool::Select,
            settings: PcbSettings::default(),
            start: None,
            net: None,
            corners: Vec::new(),
            cursor: None,
            on_copper: false,
            shown: Vec::new(),
            measure: Measure::default(),
            placing: None,
            free: false,
            posture: Posture::default(),
        }
    }
}

/// Where a click at `raw` (grid-snapped: `snapped`) attaches on `layer`: the nearest pad
/// or via center or track end within `tol`, else the center of a pad or via under the
/// cursor, else the snapped point (on a track there, or in free space). Also the copper
/// there.
pub fn snap_to_copper(
    doc: &mut Document,
    raw: Point,
    snapped: Point,
    layer: LayerId,
    tol: Coord,
) -> (Point, Option<CopperItem>) {
    let copper = doc.current_copper();
    if let Some((p, c)) = copper.nearest_anchor(raw, layer, tol) {
        return (p, Some(c.item));
    }
    match copper.at(raw, layer) {
        Some(c) if matches!(c.item, CopperItem::Pad { .. } | CopperItem::Via(_)) => (c.anchors[0], Some(c.item)),
        _ => (snapped, copper.at(snapped, layer).map(|c| c.item)),
    }
}

impl PcbPlacer {
    pub fn tool(&self) -> PcbTool {
        self.tool
    }

    pub fn is_placing(&self) -> bool {
        self.tool != PcbTool::Select
    }

    pub fn in_progress(&self) -> bool {
        self.start.is_some() || !self.corners.is_empty()
    }

    /// Net of the track being routed.
    pub fn routing_net(&self) -> Option<&str> {
        self.start.and(self.net.as_deref())
    }

    pub fn set_tool(&mut self, tool: PcbTool) {
        if tool != PcbTool::Place {
            self.placing = None;
        } else if self.tool != PcbTool::Place {
            self.settings.rotation = Angle::ZERO;
            self.settings.side = Side::Top;
        }
        self.tool = tool;
        self.start = None;
        self.net = None;
        self.corners.clear();
        self.on_copper = false;
        self.measure.clear();
    }

    /// Start placing board part `part` (in the project library already).
    pub fn place(&mut self, part: Id) {
        self.set_tool(PcbTool::Place);
        self.placing = Some(part);
    }

    /// Rotate the board part being placed by 90°.
    pub fn rotate(&mut self) {
        self.settings.rotation = self.settings.rotation.rotated_by(Angle::DEG90);
    }

    /// Put the board part being placed on the other side.
    pub fn flip(&mut self) {
        self.settings.side = match self.settings.side {
            Side::Top => Side::Bottom,
            Side::Bottom => Side::Top,
        };
    }

    /// `/`: flip which leg comes first.
    pub fn toggle_posture(&mut self) {
        bend::flip(&mut self.settings.bend, &mut self.posture);
    }

    /// Switch to the next corner style (route tool only). Returns its name.
    pub fn next_corners(&mut self) -> Option<&'static str> {
        if self.tool != PcbTool::Route {
            return None;
        }
        self.settings.corners = self.settings.corners.next_in(Corners::TRACK);
        Some(self.settings.corners.label())
    }

    /// Corner points of the track leg from `start` to `p`.
    fn leg(&self, start: Point, p: Point) -> Vec<Point> {
        let s = &self.settings;
        bend::path(start, p, s.corners, self.posture.second(s.bend))
    }

    /// Escape: end the track being routed, or leave the tool. Returns true if the tool
    /// was left.
    pub fn escape(&mut self) -> bool {
        if self.tool == PcbTool::Measure && self.measure.escape() {
            return false;
        }
        if self.start.take().is_some() | !std::mem::take(&mut self.corners).is_empty() {
            self.net = None;
            return false;
        }
        self.set_tool(PcbTool::Select);
        true
    }

    fn width(&self, doc: &Document) -> Coord {
        self.settings.width.unwrap_or(doc.project().board.rules.track_width)
    }

    /// Switch to the next copper layer. While routing, a via joins the layers at the
    /// current end of the track.
    pub fn next_layer(&mut self, doc: &mut Document) -> Result<(), String> {
        let layers: Vec<LayerId> = doc.project().board.copper_layers().map(|l| l.id).collect();
        let i = layers.iter().position(|&l| l == self.settings.layer).unwrap_or(0);
        let next = layers[(i + 1) % layers.len().max(1)];
        if self.tool == PcbTool::Route
            && let Some(p) = self.start
        {
            // Not needed on a via or a through-hole pad (they are on every layer already).
            let has_via = doc.current_copper().items.iter().any(|c| {
                matches!(c.item, CopperItem::Via(_) | CopperItem::Pad { .. })
                    && c.anchors[0] == p
                    && c.layers.count_ones() > 1
            });
            if !has_via {
                doc.execute(Command::AddVia { pos: p, diameter: None, drill: None }).map_err(|e| e.to_string())?;
            }
        }
        self.settings.layer = next;
        Ok(())
    }

    /// Track the cursor: `raw` position and grid-snapped position. Routing snaps to
    /// copper within `tol` (a few pixels).
    pub fn hover(&mut self, doc: &mut Document, p: Option<(Point, Point)>, tol: Coord) {
        let Some((raw, snapped)) = p else {
            self.cursor = None;
            self.measure.hover(None, false);
            return;
        };
        match self.tool {
            PcbTool::Route => {
                let (q, item) = snap_to_copper(doc, raw, snapped, self.settings.layer, tol);
                self.cursor = Some(q);
                if let Some(start) = self.start {
                    self.posture.track(start, q, self.settings.corners);
                }
                self.on_copper = item.is_some();
            }
            PcbTool::Measure => {
                let (q, on) = crate::measure::snap(raw, snapped, board_points(doc.project()), tol, self.free);
                self.cursor = Some(q);
                self.measure.hover(Some(q), on);
            }
            _ => self.cursor = Some(snapped),
        }
    }

    /// A click at the current cursor (see [`PcbPlacer::hover`]). Returns true when a
    /// track was finished.
    pub fn click(&mut self, doc: &mut Document) -> Result<bool, String> {
        let Some(p) = self.cursor else { return Ok(false) };
        match self.tool {
            PcbTool::Select => Ok(false),
            PcbTool::Measure => {
                self.measure.click(p);
                Ok(false)
            }
            PcbTool::Route => {
                let layer = self.settings.layer;
                let Some(start) = self.start else {
                    self.start = Some(p);
                    self.posture.start();
                    let copper = doc.current_copper();
                    self.net = copper.at(p, layer).and_then(|c| copper.net_of(c.item)).map(str::to_string);
                    return Ok(false);
                };
                if start == p {
                    self.start = None;
                    return Ok(true);
                }
                let ends = doc.current_copper().at(p, layer).is_some();
                let points = self.leg(start, p);
                let width = Some(self.width(doc));
                doc.execute(Command::AddTrack { layer, points, width }).map_err(|e| e.to_string())?;
                self.posture.start();
                if ends {
                    self.start = None;
                    self.net = None;
                } else {
                    self.start = Some(p);
                    if self.net.is_none() {
                        // Started in free space: the net is known once connected.
                        let copper = doc.current_copper();
                        self.net = copper.at(p, layer).and_then(|c| copper.net_of(c.item)).map(str::to_string);
                    }
                }
                Ok(ends)
            }
            PcbTool::Via => {
                doc.execute(Command::AddVia { pos: p, diameter: None, drill: None }).map_err(|e| e.to_string())?;
                Ok(false)
            }
            PcbTool::Place => {
                let Some(part) = self.placing else { return Ok(false) };
                let s = &self.settings;
                let command = Command::PlaceBoardPart {
                    part,
                    variant: None,
                    pos: p,
                    rotation: s.rotation,
                    side: s.side,
                    refdes: None,
                    value: None,
                };
                doc.execute(command).map_err(|e| e.to_string())?;
                Ok(false)
            }
            PcbTool::Zone => self.zone_click(doc, p),
            PcbTool::Keepout => {
                let Some(outline) = self.polygon_click(p, "a keepout")? else { return Ok(false) };
                let layers = if self.settings.keepout_all_layers { Vec::new() } else { vec![self.settings.layer] };
                let k = Keepout::new(layers, outline);
                doc.execute(Command::Insert { objects: vec![Object::Keepout(k)] }).map_err(|e| e.to_string())?;
                Ok(true)
            }
            PcbTool::Outline => {
                let Some(a) = self.start.take() else {
                    self.start = Some(p);
                    return Ok(false);
                };
                if a.x == p.x || a.y == p.y {
                    self.start = Some(a);
                    return Err("the outline needs two different corners".into());
                }
                let shape = Shape::rect(a, p, 100_000);
                let g = BoardGraphic { id: Id::new(), layer: LayerId::OUTLINE, shape };
                doc.execute(Command::Insert { objects: vec![Object::BoardGraphic(g)] }).map_err(|e| e.to_string())?;
                Ok(true)
            }
        }
    }

    /// Add a zone corner; clicking the first or last corner again closes the zone.
    fn zone_click(&mut self, doc: &mut Document, p: Point) -> Result<bool, String> {
        let layer = self.settings.layer;
        if self.corners.is_empty() {
            let copper = doc.current_copper();
            let under = copper.at(p, layer).and_then(|c| copper.net_of(c.item)).map(str::to_string);
            let net = if self.settings.zone_net.is_empty() { under } else { Some(self.settings.zone_net.clone()) };
            let Some(net) = net else {
                return Err("choose the zone's net in the tool settings, or start on copper of the net".into());
            };
            self.net = Some(net);
        }
        let Some(outline) = self.polygon_click(p, "a zone")? else { return Ok(false) };
        let net = self.net.take().unwrap_or_default();
        let zone = Zone::new(layer, net, outline);
        doc.execute(Command::Insert { objects: vec![Object::Zone(zone)] }).map_err(|e| e.to_string())?;
        Ok(true)
    }

    /// Add a corner; clicking the first or last corner again closes the polygon and
    /// returns its corners.
    fn polygon_click(&mut self, p: Point, what: &str) -> Result<Option<Vec<Point>>, String> {
        let closes = self.corners.first() == Some(&p) || self.corners.last() == Some(&p);
        if !closes {
            self.corners.push(p);
            return Ok(None);
        }
        if self.corners.len() < 3 {
            return Err(format!("{what} needs at least three corners"));
        }
        Ok(Some(std::mem::take(&mut self.corners)))
    }

    /// Net of the zone being drawn.
    pub fn zone_net(&self) -> Option<&str> {
        (!self.corners.is_empty()).then_some(self.net.as_deref()).flatten()
    }

    /// What to preview at the cursor.
    pub fn ghosts(&self, doc: &Document) -> Vec<PcbGhost> {
        let Some(p) = self.cursor else { return Vec::new() };
        let s = &self.settings;
        let mut g = Vec::new();
        match self.tool {
            PcbTool::Select | PcbTool::Measure => {}
            PcbTool::Route => {
                if let Some(start) = self.start {
                    let width = self.width(doc);
                    let path = self.leg(start, p);
                    g.extend(path.windows(2).map(|w| PcbGhost::Track { layer: s.layer, a: w[0], b: w[1], width }));
                }
                if self.on_copper {
                    g.push(PcbGhost::Target(p));
                }
            }
            PcbTool::Via => g.push(PcbGhost::Via { pos: p, diameter: doc.project().board.rules.via_diameter }),
            PcbTool::Place => g.extend(self.placing.map(|part| PcbGhost::Footprint {
                part,
                pos: p,
                rotation: s.rotation,
                side: s.side,
            })),
            PcbTool::Outline => {
                if let Some(a) = self.start {
                    g.push(PcbGhost::Outline(Rect::from_points(a, p)));
                }
                g.push(PcbGhost::Target(p));
            }
            PcbTool::Zone | PcbTool::Keepout => {
                if !self.corners.is_empty() {
                    let mut points = self.corners.clone();
                    points.push(p);
                    g.push(PcbGhost::Zone { layer: s.layer, points });
                }
                g.push(PcbGhost::Target(p));
            }
        }
        g
    }

    /// Ghosts to show, if they changed since the last call.
    pub fn take_preview(&mut self, doc: &Document) -> Option<Vec<PcbGhost>> {
        let g = self.ghosts(doc);
        (g != self.shown).then(|| {
            self.shown = g.clone();
            g
        })
    }
}
