//! Board creation tools: route tracks (45°, 90° or rounded corners), place vias, draw the board outline
//! copper zones and keepouts, measure, place board parts.
//! Independent of the GUI toolkit.

use editor_core::Command;
use editor_core::command::TrackPath;
use editor_core::copper::{CopperItem, layer_bit};
use editor_core::route_check::{Clash, Probe};
pub use editor_core::router::RouteMode;
use editor_core::router::{self, Routed};
use model::*;

use crate::bend::{self, Posture};
pub use crate::bend::{Bend, Corners};
use crate::document::Document;
use crate::measure::{BoardSnap, Clearance, Measure};
use editor_core::clearance::{self, Gap};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PcbTool {
    Select,
    Route,
    Via,
    Outline,
    Zone,
    Keepout,
    /// Cut an area out of a zone or keepout.
    Cutout,
    Measure,
    /// Free text on a board layer: a click starts typing it in place.
    Text,
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
        PcbTool::Cutout,
        PcbTool::Text,
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
            PcbTool::Cutout => "Cutout",
            PcbTool::Measure => "Measure",
            PcbTool::Text => "Text",
            PcbTool::Place => "Place",
        }
    }

    pub fn hint(self) -> &'static str {
        match self {
            PcbTool::Select => "Click/box to select · drag to move · R rotate · F flip side · Del delete",
            PcbTool::Route => {
                "Click a pad to start · click corners · ends on copper · L next layer (adds a via) · / corner · Shift+Space style · Shift+R mode · Esc done"
            }
            PcbTool::Via => "Click to place a via · Esc done",
            PcbTool::Outline => "Click two corners of the board outline · Esc done",
            PcbTool::Zone => "Click the zone's corners · click the first or last corner again to close · Esc cancels",
            PcbTool::Keepout => {
                "Click the keepout's corners · click the first or last corner again to close · Esc cancels"
            }
            PcbTool::Cutout => {
                "Start inside a zone or keepout · click the cutout's corners · click the first or last corner again to cut · Esc cancels"
            }
            PcbTool::Measure => crate::measure::HINT,
            PcbTool::Text => {
                "Click to place, then type · Shift+Enter new line · Enter done · Esc cancel · R rotate · F other side"
            }
            PcbTool::Place => "Click to place · R rotate · F flip side · Esc done",
        }
    }
}

/// Properties of new board objects (editable in the side panel). The style choices are
/// remembered across sessions ([`crate::tool_memory`]); skipped fields are not.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct PcbSettings {
    /// Active copper layer for routing.
    #[serde(skip)]
    pub layer: LayerId,
    /// Track width; `None`: the design rules' width.
    pub width: Option<Coord>,
    pub corners: Corners,
    /// Which leg comes first (`/` flips it).
    pub bend: Bend,
    /// Radius of Edit → Round track corners.
    pub corner_radius: Coord,
    /// What the Route tool does about copper in the way.
    pub route_mode: RouteMode,
    /// Add tracks and vias that break the clearance, keepout or board edge rules
    /// (otherwise the click is refused).
    pub allow_violations: bool,
    /// Vias added while routing (L) join just the two layers (else they go through).
    pub via_pair: bool,
    /// Layers the via tool's vias join (blind and buried vias); through when the same or
    /// not on the board.
    pub via_span: [LayerId; 2],
    /// Net of new zones; empty: the net of the copper under the first corner.
    #[serde(skip)]
    pub zone_net: String,
    /// New keepouts cover all copper layers (else the active one).
    pub keepout_all_layers: bool,
    /// The Measure tool measures the gap between two objects' edges (else between two
    /// points).
    pub clearance: bool,
    /// Layer, cap height, weight and alignment of new texts.
    pub text_layer: LayerId,
    pub text_size: Coord,
    pub text_bold: bool,
    pub text_align: HAlign,
    /// Rotation of board parts and texts being placed; the side of board parts.
    #[serde(skip)]
    pub rotation: Angle,
    #[serde(skip)]
    pub side: Side,
}

impl Default for PcbSettings {
    fn default() -> Self {
        PcbSettings {
            layer: LayerId::TOP_COPPER,
            width: None,
            corners: Corners::Deg45,
            bend: Bend::Auto,
            corner_radius: 1_000_000,
            route_mode: RouteMode::default(),
            allow_violations: false,
            via_pair: false,
            via_span: [LayerId::TOP_COPPER, LayerId::BOTTOM_COPPER],
            zone_net: String::new(),
            keepout_all_layers: true,
            clearance: false,
            text_layer: LayerId::TOP_SILK,
            text_size: 1_000_000,
            text_bold: true,
            text_align: HAlign::Left,
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
        /// A point on the arc (arc corners).
        mid: Option<Point>,
        width: Coord,
        /// Breaks a rule (see [`PcbPlacer::clash`]).
        bad: bool,
    },
    Via {
        pos: Point,
        diameter: Coord,
        bad: bool,
    },
    /// Where the copper being added breaks a rule.
    Clash(Point),
    /// Where a new text goes.
    Text {
        layer: LayerId,
        text: Text,
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
    /// The net under the Via tool's cursor (its class sizes the via).
    via_net: Option<String>,
    /// Corners of the zone being drawn.
    corners: Vec<Point>,
    cursor: Option<Point>,
    /// The cursor is on copper of the active layer.
    on_copper: bool,
    shown: Vec<PcbGhost>,
    pub measure: Measure,
    /// The Measure tool's Clearance mode and the gap it shows.
    pub clearance: Clearance,
    pub gap: Option<Gap>,
    /// Snap points of the board for the Measure tool (rebuilt on changes).
    snap: Option<BoardSnap>,
    /// The zone or keepout the cutout being drawn cuts into.
    cut_into: Option<ObjectId>,
    /// The board part placed by [`PcbTool::Place`].
    pub placing: Option<Id>,
    /// Snap nowhere (Alt held): set by the app before [`PcbPlacer::hover`].
    pub free: bool,
    /// Add tracks and vias even where they break a rule, this once (Shift held): set by
    /// the app like `free`. DRC flags what it adds.
    pub force: bool,
    /// Automatic posture of the track leg being routed.
    posture: Posture,
    /// The rule the track leg or via at the cursor breaks.
    clash: Option<Clash>,
    /// What [`PcbPlacer::clash`] was checked for (it's only checked again on changes).
    checked: Option<ClashKey>,
    /// The leg as the router lays it (Walk around mode).
    routed: Option<Routed>,
}

/// The inputs of a clash check: document revision, tool, net, layer, width, the leg's
/// segments (or the via position), the via span and how the leg is laid.
type ClashKey =
    (u64, PcbTool, Option<String>, LayerId, Coord, Vec<(Point, Point, Option<Point>)>, [LayerId; 2], RouteMode);

impl Default for PcbPlacer {
    fn default() -> Self {
        PcbPlacer {
            tool: PcbTool::Select,
            settings: PcbSettings::default(),
            start: None,
            net: None,
            via_net: None,
            corners: Vec::new(),
            cursor: None,
            on_copper: false,
            shown: Vec::new(),
            measure: Measure::default(),
            clearance: Clearance::default(),
            gap: None,
            snap: None,
            cut_into: None,
            placing: None,
            free: false,
            force: false,
            posture: Posture::default(),
            clash: None,
            checked: None,
            routed: None,
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
    let copper = doc.route_copper();
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
        if tool == PcbTool::Text && self.tool != PcbTool::Text {
            self.settings.rotation = Angle::ZERO;
        }
        self.tool = tool;
        self.start = None;
        self.net = None;
        self.corners.clear();
        self.on_copper = false;
        self.clash = None;
        self.checked = None;
        self.measure.clear();
        self.clearance.clear();
        self.gap = None;
        self.cut_into = None;
    }

    /// Switch the Measure tool between points and clearances. Returns the new mode's name.
    pub fn toggle_clearance(&mut self) -> &'static str {
        self.settings.clearance = !self.settings.clearance;
        self.measure.clear();
        self.clearance.clear();
        self.gap = None;
        if self.settings.clearance { "Measure clearances" } else { "Measure between points" }
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

    /// Put the board part or text being placed on the other side.
    pub fn flip(&mut self) {
        if self.tool == PcbTool::Text {
            self.settings.text_layer = self.settings.text_layer.other_side();
            return;
        }
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

    /// Segments of the track leg from `start` to `p`: (start, end, point on an arc).
    fn leg(&self, start: Point, p: Point) -> Vec<(Point, Point, Option<Point>)> {
        let s = &self.settings;
        bend::segments(start, p, s.corners, self.posture.second(s.bend))
    }

    /// A new text at `p` as the settings say (empty).
    fn new_text(&self, p: Point) -> Text {
        let s = &self.settings;
        let mut t = Text::new("", p, s.text_size).aligned(s.text_align, VAlign::Bottom);
        t.bold = s.text_bold;
        t.rotation = s.rotation;
        t
    }

    /// Escape: end the track being routed, or leave the tool. Returns true if the tool
    /// was left.
    pub fn escape(&mut self) -> bool {
        if self.tool == PcbTool::Measure {
            let had = if self.settings.clearance { self.clearance.escape() } else { self.measure.escape() };
            if had {
                self.gap = None;
                return false;
            }
        }
        if self.start.take().is_some() | !std::mem::take(&mut self.corners).is_empty() {
            self.cut_into = None;
            self.net = None;
            return false;
        }
        self.set_tool(PcbTool::Select);
        true
    }

    /// The track width: the tool's, else the routed net's class's.
    fn width(&self, doc: &Document) -> Coord {
        self.settings.width.unwrap_or_else(|| doc.project().board.rules.for_net(self.net.as_deref()).track_width)
    }

    fn via_diameter(doc: &Document, net: Option<&str>) -> Coord {
        doc.project().board.rules.for_net(net).via_diameter
    }

    /// Switch to the next copper layer. While routing, a via joins the layers at the
    /// current end of the track (through, or just the two layers: [`PcbSettings::via_pair`]).
    pub fn next_layer(&mut self, doc: &mut Document) -> Result<(), String> {
        let layers: Vec<LayerId> = doc.project().board.copper_layers().map(|l| l.id).collect();
        let cur = self.settings.layer;
        let i = layers.iter().position(|&l| l == cur).unwrap_or(0);
        let next = layers[(i + 1) % layers.len().max(1)];
        if self.tool == PcbTool::Route
            && let Some(p) = self.start
        {
            // Not needed on a via or a through-hole pad joining both layers already.
            let both = layer_bit(cur) | layer_bit(next);
            let joined = doc.current_copper().items.iter().any(|c| {
                matches!(c.item, CopperItem::Via(_) | CopperItem::Pad { .. })
                    && c.anchors[0] == p
                    && c.layers & both == both
            });
            if !joined {
                let span = self.settings.via_pair.then_some([cur, next]).filter(|s| !is_through(s));
                let via = Probe::Via {
                    pos: p,
                    diameter: Self::via_diameter(doc, self.net.as_deref()),
                    layers: span_mask(doc.project(), span),
                };
                if let Some(c) = doc.route_clash(&[via], self.net.as_deref(), &[(cur, p)])
                    && !self.allows_violations()
                {
                    return Err(format!("No via here: {}", c.message));
                }
                doc.execute(Command::AddVia { pos: p, diameter: None, drill: None, span })
                    .map_err(|e| e.to_string())?;
            }
        }
        self.settings.layer = next;
        Ok(())
    }

    /// A click (or L) adds copper that breaks a rule: always (the setting) or this once
    /// ([`PcbPlacer::force`]).
    pub fn allows_violations(&self) -> bool {
        self.settings.allow_violations || self.force
    }

    /// The rule the copper at the cursor would break (route leg or via).
    pub fn clash(&self) -> Option<&Clash> {
        self.clash.as_ref()
    }

    /// Span of the via tool's vias (`None`: through).
    fn tool_span(&self, doc: &Document) -> Option<[LayerId; 2]> {
        let s = self.settings.via_span;
        let on_board = |l: LayerId| doc.project().board.copper_layers().any(|c| c.id == l);
        (s[0] != s[1] && on_board(s[0]) && on_board(s[1]) && !is_through(&s)).then_some(s)
    }

    /// Check what a click at the cursor would add.
    fn update_clash(&mut self, doc: &mut Document) {
        let Some(p) = self.cursor else {
            self.clash = None;
            return;
        };
        let layer = self.settings.layer;
        let segments = match (self.tool, self.start) {
            (PcbTool::Route, Some(start)) => self.leg(start, p),
            _ => vec![(p, p, None)],
        };
        let mode = self.mode();
        let key = (
            doc.route_revision(),
            self.tool,
            self.net.clone(),
            layer,
            self.width(doc),
            segments,
            self.settings.via_span,
            mode,
        );
        if self.checked.as_ref() == Some(&key) {
            return;
        }
        self.checked = Some(key);
        self.clash = None;
        self.routed = None;
        // The router works on the board without the last step's pushes.
        doc.clear_route_preview();
        match self.tool {
            PcbTool::Route if mode != RouteMode::Highlight => {
                let Some(start) = self.start else { return };
                let width = self.width(doc);
                let points = chord_points(start, &self.leg(start, p));
                doc.route_copper();
                let (project, copper) = doc.route_base();
                let routed = router::route(project, copper, mode, layer, width, self.net.as_deref(), &points);
                let paths: Vec<TrackPath> = routed
                    .pushed
                    .iter()
                    .map(|p| TrackPath { tracks: p.line.tracks.clone(), path: p.path.clone() })
                    .collect();
                let shown = if paths.is_empty() {
                    Ok(())
                } else {
                    doc.set_route_preview(vec![Command::SetTrackPaths { paths }])
                };
                self.routed = Some(match shown {
                    Ok(()) => routed,
                    // Not expected: the router's pushes are valid commands.
                    Err(message) => Routed {
                        points: vec![start],
                        reached: false,
                        blocked: Some(Clash { at: start, message }),
                        pushed: Vec::new(),
                    },
                });
            }
            PcbTool::Route => {
                let Some(start) = self.start else { return };
                let width = self.width(doc);
                let probes: Vec<Probe> = self
                    .leg(start, p)
                    .into_iter()
                    .map(|(a, b, mid)| Probe::Track { layer, a, b, mid, width })
                    .collect();
                if probes.is_empty() {
                    return;
                }
                self.clash = doc.route_clash(&probes, self.net.as_deref(), &[(layer, start), (layer, p)]);
            }
            PcbTool::Via => {
                let layers = span_mask(doc.project(), self.tool_span(doc));
                // The via takes the net of the copper under it.
                let under: Vec<LayerId> = doc.project().board.copper_layers().map(|l| l.id).collect();
                let copper = doc.current_copper();
                let found = under
                    .into_iter()
                    .filter(|&l| layers & layer_bit(l) != 0)
                    .find_map(|l| copper.at(p, l).map(|c| (l, copper.net_of(c.item).map(str::to_string))));
                let (ends, net) = match found {
                    Some((l, net)) => (vec![(l, p)], net),
                    None => (Vec::new(), None),
                };
                let diameter = Self::via_diameter(doc, net.as_deref());
                self.clash = doc.route_clash(&[Probe::Via { pos: p, diameter, layers }], net.as_deref(), &ends);
                self.via_net = net;
            }
            _ => {}
        }
    }

    /// How the leg is laid now: the mode, or Highlight while Shift is held (straight
    /// through).
    fn mode(&self) -> RouteMode {
        if self.force { RouteMode::Highlight } else { self.settings.route_mode }
    }

    /// A route leg is being shoved: the document holds its pushes as a preview. Else the
    /// app drops any (see [`Document::drop_route_preview`]).
    pub fn wants_route_preview(&self) -> bool {
        self.tool == PcbTool::Route && self.start.is_some() && self.mode() == RouteMode::Shove
    }

    /// What the leg to `p` adds: the router's path (straight segments; the leg's own
    /// when the router left it as it was), else the leg.
    fn route_segments(&self, start: Point, p: Point) -> Vec<(Point, Point, Option<Point>)> {
        let leg = self.leg(start, p);
        match &self.routed {
            Some(r) if self.mode() != RouteMode::Highlight && r.points != chord_points(start, &leg) => {
                r.points.windows(2).map(|w| (w[0], w[1], None)).collect()
            }
            _ => leg,
        }
    }

    /// Where the router stopped short of the pointer (Walk around mode).
    pub fn stopped_short(&self) -> Option<&Routed> {
        self.routed.as_ref().filter(|r| self.mode() != RouteMode::Highlight && !r.reached)
    }

    /// Refuse a click that breaks a rule (unless allowed or forced).
    fn refuse(&self) -> Result<(), String> {
        match &self.clash {
            Some(c) if !self.allows_violations() => {
                Err(format!("Not added: {} (Shift+click adds it anyway)", c.message))
            }
            _ => Ok(()),
        }
    }

    /// The raw (unsnapped) cursor, `pixel` nm a screen point: decides which leg of a
    /// track comes first (automatic posture).
    pub fn steer(&mut self, raw: Point, pixel: f64) {
        self.posture.track(raw, pixel, self.settings.corners);
    }

    /// Track the cursor: `raw` position and grid-snapped position. Routing snaps to
    /// copper within `tol` (a few pixels).
    pub fn hover(&mut self, doc: &mut Document, p: Option<(Point, Point)>, tol: Coord) {
        let p = p.filter(|_| self.tool != PcbTool::Text || doc.text_edit().is_none());
        let Some((raw, snapped)) = p else {
            self.cursor = None;
            self.measure.hover(None, false);
            self.clearance.hover(None);
            if !self.clearance.is_pinned() {
                self.gap = None;
            }
            return;
        };
        match self.tool {
            PcbTool::Route => {
                let (q, item) = snap_to_copper(doc, raw, snapped, self.settings.layer, tol);
                self.cursor = Some(q);
                self.on_copper = item.is_some();
            }
            PcbTool::Measure if self.settings.clearance => {
                self.cursor = Some(raw);
                doc.current_copper();
                let (project, copper) = (doc.project(), doc.copper());
                self.clearance.hover(clearance::feature_at(project, copper, raw, self.settings.layer, tol));
                self.gap = self.clearance.pair().and_then(|(a, b)| clearance::gap(project, copper, a, b));
            }
            PcbTool::Measure => {
                if self.snap.as_ref().is_none_or(|s| s.revision != doc.revision) {
                    self.snap = Some(BoardSnap::build(doc.project(), doc.revision));
                }
                let snap = self.snap.as_ref().expect("built above");
                let (q, on) = snap.snap(raw, snapped, tol, self.free);
                self.cursor = Some(q);
                self.measure.hover(Some(q), on);
            }
            _ => self.cursor = Some(snapped),
        }
        self.update_clash(doc);
    }

    /// A click at the current cursor (see [`PcbPlacer::hover`]). Returns true when a
    /// track was finished.
    pub fn click(&mut self, doc: &mut Document) -> Result<bool, String> {
        let Some(p) = self.cursor else { return Ok(false) };
        match self.tool {
            PcbTool::Select => Ok(false),
            PcbTool::Measure if self.settings.clearance => {
                if self.clearance.hover.is_none() {
                    return Err("click copper, a drill hole or the board edge".into());
                }
                self.clearance.click();
                doc.current_copper();
                let (project, copper) = (doc.project(), doc.copper());
                self.gap = self.clearance.pair().and_then(|(a, b)| clearance::gap(project, copper, a, b));
                Ok(false)
            }
            PcbTool::Measure => {
                self.measure.click(p);
                Ok(false)
            }
            PcbTool::Cutout => self.cutout_click(doc, p),
            PcbTool::Route => {
                let layer = self.settings.layer;
                let Some(start) = self.start else {
                    self.start = Some(p);
                    self.posture.start();
                    let copper = doc.route_copper();
                    self.net = copper.at(p, layer).and_then(|c| copper.net_of(c.item)).map(str::to_string);
                    return Ok(false);
                };
                if start == p {
                    self.start = None;
                    return Ok(true);
                }
                self.refuse()?;
                let segs = self.route_segments(start, p);
                let Some(&(_, end, _)) = segs.last() else {
                    let why = self.routed.as_ref().and_then(|r| r.blocked.as_ref());
                    return Err(why.map_or("No room for a track here".into(), |c| format!("No room: {}", c.message)));
                };
                let p = end;
                let ends = doc.route_copper().at(p, layer).is_some();
                let points = std::iter::once(start).chain(segs.iter().map(|s| s.1)).collect();
                let mids = segs.iter().map(|s| s.2).collect();
                let width = Some(self.width(doc));
                // With the tracks it pushed (the preview), as one undo step.
                doc.commit_route(Command::AddTrack { layer, points, width, mids }).map_err(|e| e.to_string())?;
                self.posture.start();
                self.clash = None;
                self.checked = None;
                self.routed = None;
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
                self.refuse()?;
                let span = self.tool_span(doc);
                doc.execute(Command::AddVia { pos: p, diameter: None, drill: None, span })
                    .map_err(|e| e.to_string())?;
                self.update_clash(doc);
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
            PcbTool::Text => {
                let t = BoardText { id: Id::new(), layer: self.settings.text_layer, text: self.new_text(p) };
                let (shape, color) = (t.text.clone(), crate::style::board_layer_color(t.layer));
                doc.begin_new_text(
                    crate::view::ViewId::Board,
                    crate::text_edit::TextTarget::NewBoardText(t),
                    shape,
                    color,
                );
                Ok(false)
            }
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

    /// Add a cutout corner. The first one picks the zone or keepout to cut (on the active
    /// layer, the topmost zone first); closing the polygon cuts.
    fn cutout_click(&mut self, doc: &mut Document, p: Point) -> Result<bool, String> {
        if self.corners.is_empty() {
            let target = cutout_target(doc.project(), p, self.settings.layer);
            let Some(target) = target else {
                return Err("start the cutout inside a zone or keepout on the active layer".into());
            };
            self.cut_into = Some(target);
        }
        let Some(outline) = self.polygon_click(p, "a cutout")? else { return Ok(false) };
        let Some(id) = self.cut_into.take() else { return Ok(false) };
        doc.execute(Command::AddCutout { id, outline }).map_err(|e| e.to_string())?;
        Ok(true)
    }

    /// The zone or keepout the cutout being drawn cuts into.
    pub fn cut_into(&self) -> Option<ObjectId> {
        self.cut_into.filter(|_| !self.corners.is_empty())
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
                    let bad = self.clash.is_some();
                    g.extend(self.route_segments(start, p).into_iter().map(|(a, b, mid)| PcbGhost::Track {
                        layer: s.layer,
                        a,
                        b,
                        mid,
                        width,
                        bad,
                    }));
                }
                if self.on_copper {
                    g.push(PcbGhost::Target(p));
                }
            }
            PcbTool::Text => {
                g.push(PcbGhost::Text { layer: s.text_layer, text: Text { text: "Text".into(), ..self.new_text(p) } })
            }
            PcbTool::Via => g.push(PcbGhost::Via {
                pos: p,
                diameter: Self::via_diameter(doc, self.via_net.as_deref()),
                bad: self.clash.is_some(),
            }),
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
            PcbTool::Zone | PcbTool::Keepout | PcbTool::Cutout => {
                if !self.corners.is_empty() {
                    let mut points = self.corners.clone();
                    points.push(p);
                    g.push(PcbGhost::Zone { layer: s.layer, points });
                }
                g.push(PcbGhost::Target(p));
            }
        }
        g.extend(self.clash.as_ref().map(|c| PcbGhost::Clash(c.at)));
        // Where the router stopped: the leg ends here.
        if self.tool == PcbTool::Route
            && let Some(r) = self.stopped_short()
        {
            g.extend(r.points.last().map(|&q| PcbGhost::Clash(q)));
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

/// What a cutout started at `p` cuts into: the zone on `layer` containing `p` with the
/// highest priority, else a keepout on `layer` containing it.
pub fn cutout_target(project: &Project, p: Point, layer: LayerId) -> Option<ObjectId> {
    let inside = |outline: &[Point], holes: &[Vec<Point>]| {
        editor_core::copper::point_in_polygon(p, outline)
            && !holes.iter().any(|h| editor_core::copper::point_in_polygon(p, h))
    };
    let board = &project.board;
    let zone = board
        .zones
        .iter()
        .filter(|z| z.layer == layer && inside(&z.outline, &z.holes))
        .max_by_key(|z| z.priority)
        .map(|z| ObjectId::Zone(z.id));
    zone.or_else(|| {
        board
            .keepouts
            .iter()
            .find(|k| k.on_layer(layer) && inside(&k.outline, &k.holes))
            .map(|k| ObjectId::Keepout(k.id))
    })
}

/// Spans top to bottom.
/// The corners of a leg (arcs as their chords), from its start.
fn chord_points(start: Point, leg: &[(Point, Point, Option<Point>)]) -> Vec<Point> {
    std::iter::once(start).chain(leg.iter().map(|s| s.1)).collect()
}

fn is_through(span: &[LayerId; 2]) -> bool {
    let (lo, hi) = (span[0].0.min(span[1].0), span[0].0.max(span[1].0));
    lo == LayerId::TOP_COPPER.0 && hi == LayerId::BOTTOM_COPPER.0
}

/// Copper layers of a via spanning `span` (`None`: all).
fn span_mask(project: &Project, span: Option<[LayerId; 2]>) -> editor_core::copper::LayerMask {
    let [a, b] = span.unwrap_or([LayerId::TOP_COPPER, LayerId::BOTTOM_COPPER]);
    let (lo, hi) = (a.0.min(b.0), a.0.max(b.0));
    project.board.copper_layers().filter(|l| (lo..=hi).contains(&l.id.0)).fold(0, |m, l| m | layer_bit(l.id))
}
