//! Board creation tools: route tracks (45° or 90°), place vias, draw the board outline.
//! Independent of the GUI toolkit.

use editor_core::Command;
use editor_core::copper::CopperItem;
use model::*;

use crate::document::Document;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PcbTool {
    Select,
    Route,
    Via,
    Outline,
}

impl PcbTool {
    pub const ALL: &[PcbTool] = &[PcbTool::Select, PcbTool::Route, PcbTool::Via, PcbTool::Outline];

    pub fn label(self) -> &'static str {
        match self {
            PcbTool::Select => "Select",
            PcbTool::Route => "Route",
            PcbTool::Via => "Via",
            PcbTool::Outline => "Outline",
        }
    }

    pub fn hint(self) -> &'static str {
        match self {
            PcbTool::Select => "Click/box to select · drag to move · R rotate · F flip side · Del delete",
            PcbTool::Route => {
                "Click a pad to start · click corners · ends on copper · L next layer (adds a via) · / corner · Esc done"
            }
            PcbTool::Via => "Click to place a via · Esc done",
            PcbTool::Outline => "Click two corners of the board outline · Esc done",
        }
    }
}

/// Track corner angles.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Corners {
    Deg45,
    Deg90,
}

#[derive(Clone, Debug)]
pub struct PcbSettings {
    /// Active copper layer for routing.
    pub layer: LayerId,
    /// Track width; `None`: the design rules' width.
    pub width: Option<Coord>,
    pub corners: Corners,
    /// The diagonal (45°) or vertical (90°) part comes first.
    pub bend_first: bool,
}

impl Default for PcbSettings {
    fn default() -> Self {
        PcbSettings { layer: LayerId::TOP_COPPER, width: None, corners: Corners::Deg45, bend_first: false }
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
    /// The cursor is on copper (a route would start or end there).
    Target(Point),
}

/// Active board tool and its in-progress state.
#[derive(Clone, Debug)]
pub struct PcbPlacer {
    tool: PcbTool,
    pub settings: PcbSettings,
    /// Start of the track segment being routed, or the first outline corner.
    start: Option<Point>,
    /// Net being routed.
    net: Option<String>,
    cursor: Option<Point>,
    /// The cursor is on copper of the active layer.
    on_copper: bool,
    shown: Vec<PcbGhost>,
}

impl Default for PcbPlacer {
    fn default() -> Self {
        PcbPlacer {
            tool: PcbTool::Select,
            settings: PcbSettings::default(),
            start: None,
            net: None,
            cursor: None,
            on_copper: false,
            shown: Vec::new(),
        }
    }
}

/// Corner points of a track from `a` to `b` with 45° or 90° corners.
pub fn track_path(a: Point, b: Point, corners: Corners, bend_first: bool) -> Vec<Point> {
    let (dx, dy) = (b.x - a.x, b.y - a.y);
    if dx == 0 || dy == 0 || (corners == Corners::Deg45 && dx.abs() == dy.abs()) {
        return vec![a, b];
    }
    let corner = match corners {
        Corners::Deg90 if bend_first => Point::new(a.x, b.y),
        Corners::Deg90 => Point::new(b.x, a.y),
        Corners::Deg45 => {
            let m = dx.abs().min(dy.abs());
            let diag = Point::new(dx.signum() * m, dy.signum() * m);
            if bend_first { a + diag } else { b - diag }
        }
    };
    vec![a, corner, b]
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
        self.start.is_some()
    }

    /// Net of the track being routed.
    pub fn routing_net(&self) -> Option<&str> {
        self.start.and(self.net.as_deref())
    }

    pub fn set_tool(&mut self, tool: PcbTool) {
        self.tool = tool;
        self.start = None;
        self.net = None;
        self.on_copper = false;
    }

    pub fn toggle_posture(&mut self) {
        self.settings.bend_first = !self.settings.bend_first;
    }

    /// Escape: end the track being routed, or leave the tool. Returns true if the tool
    /// was left.
    pub fn escape(&mut self) -> bool {
        if self.start.take().is_some() {
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
            return;
        };
        match self.tool {
            PcbTool::Route => {
                let (q, item) = snap_to_copper(doc, raw, snapped, self.settings.layer, tol);
                self.cursor = Some(q);
                self.on_copper = item.is_some();
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
            PcbTool::Route => {
                let layer = self.settings.layer;
                let Some(start) = self.start else {
                    self.start = Some(p);
                    let copper = doc.current_copper();
                    self.net = copper.at(p, layer).and_then(|c| copper.net_of(c.item)).map(str::to_string);
                    return Ok(false);
                };
                if start == p {
                    self.start = None;
                    return Ok(true);
                }
                let ends = doc.current_copper().at(p, layer).is_some();
                let points = track_path(start, p, self.settings.corners, self.settings.bend_first);
                let width = Some(self.width(doc));
                doc.execute(Command::AddTrack { layer, points, width }).map_err(|e| e.to_string())?;
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

    /// What to preview at the cursor.
    pub fn ghosts(&self, doc: &Document) -> Vec<PcbGhost> {
        let Some(p) = self.cursor else { return Vec::new() };
        let s = &self.settings;
        let mut g = Vec::new();
        match self.tool {
            PcbTool::Select => {}
            PcbTool::Route => {
                if let Some(start) = self.start {
                    let width = self.width(doc);
                    let path = track_path(start, p, s.corners, s.bend_first);
                    g.extend(path.windows(2).map(|w| PcbGhost::Track { layer: s.layer, a: w[0], b: w[1], width }));
                }
                if self.on_copper {
                    g.push(PcbGhost::Target(p));
                }
            }
            PcbTool::Via => g.push(PcbGhost::Via { pos: p, diameter: doc.project().board.rules.via_diameter }),
            PcbTool::Outline => {
                if let Some(a) = self.start {
                    g.push(PcbGhost::Outline(Rect::from_points(a, p)));
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths() {
        let a = Point::mm(0.0, 0.0);
        let b = Point::mm(5.0, 2.0);
        assert_eq!(track_path(a, b, Corners::Deg45, false), vec![a, Point::mm(3.0, 0.0), b]);
        assert_eq!(track_path(a, b, Corners::Deg45, true), vec![a, Point::mm(2.0, 2.0), b]);
        assert_eq!(track_path(a, b, Corners::Deg90, false), vec![a, Point::mm(5.0, 0.0), b]);
        assert_eq!(track_path(a, b, Corners::Deg90, true), vec![a, Point::mm(0.0, 2.0), b]);
        let c = Point::mm(-2.0, -5.0);
        assert_eq!(track_path(a, c, Corners::Deg45, false), vec![a, Point::mm(0.0, -3.0), c]);
        assert_eq!(track_path(a, Point::mm(3.0, -3.0), Corners::Deg45, false).len(), 2);
    }
}
