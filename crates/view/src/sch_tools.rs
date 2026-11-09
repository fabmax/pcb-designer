//! Schematic creation tools: wires, buses, bus entries, net labels, power ports, junctions
//! and part placement, with a ghost preview under the cursor. Independent of the GUI
//! toolkit.

use editor_core::connectivity::{Connectivity, on_segment_inside};
use editor_core::{Command, bus};
use model::*;

use crate::document::Document;
use crate::tools::increment_name;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SchTool {
    Select,
    Wire,
    Bus,
    BusEntry,
    Label,
    Power,
    Junction,
    /// Place parts (see [`SchPlacer::placing`]).
    Place,
}

impl SchTool {
    /// Tools offered in the toolbar (placement starts from the library).
    pub const BAR: &[SchTool] = &[
        SchTool::Select,
        SchTool::Wire,
        SchTool::Bus,
        SchTool::BusEntry,
        SchTool::Label,
        SchTool::Power,
        SchTool::Junction,
    ];

    pub fn label(self) -> &'static str {
        match self {
            SchTool::Select => "Select",
            SchTool::Wire => "Wire",
            SchTool::Bus => "Bus",
            SchTool::BusEntry => "Bus entry",
            SchTool::Label => "Label",
            SchTool::Power => "Power",
            SchTool::Junction => "Junction",
            SchTool::Place => "Place",
        }
    }

    /// Usage hint for the toolbar.
    pub fn hint(self) -> &'static str {
        match self {
            SchTool::Select => "Click/box to select · drag to move (wires follow) · R rotate · X mirror · Del delete",
            SchTool::Wire => "Click to start · click corners · ends on a pin or wire · / corner direction · Esc done",
            SchTool::Bus => {
                "Click to start · click corners · ends on a bus · / corner direction · Esc done (asks for a name)"
            }
            SchTool::BusEntry => "Click on a bus to place an entry · R rotate · X mirror · Esc done",
            SchTool::Label | SchTool::Power => "Click to place · R rotate · Esc done",
            SchTool::Junction => "Click to place a junction · Esc done",
            SchTool::Place => "Click to place · R rotate · X mirror · Esc done",
        }
    }
}

/// The part being placed by [`SchTool::Place`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Placing {
    pub part: Id,
    /// Component whose remaining units are being placed (multi-unit parts).
    pub component: Option<Id>,
    /// Unit placed by the next click.
    pub unit: u32,
}

/// Properties of newly placed objects (editable in the side panel).
#[derive(Clone, Debug)]
pub struct SchSettings {
    /// Name of the next net label; auto-incremented after each placement.
    pub label: String,
    pub power: String,
    pub rotation: Angle,
    pub mirror: bool,
    /// Wire corners go vertical first (toggled with `/`).
    pub vertical_first: bool,
    /// Direction of new bus entries.
    pub entry: EntryDir,
}

impl Default for SchSettings {
    fn default() -> Self {
        SchSettings {
            label: "NET1".into(),
            power: "GND".into(),
            rotation: Angle::ZERO,
            mirror: false,
            vertical_first: false,
            entry: EntryDir::UpRight,
        }
    }
}

/// Common power port names offered in the side panel.
pub const POWER_NAMES: &[&str] = &["GND", "+3V3", "+5V", "+12V", "VCC", "VDD", "VBUS", "AGND"];

/// Something a tool would create, drawn by [`crate::View::set_preview`].
#[derive(Clone, Debug, PartialEq)]
pub enum SchGhost {
    Wire(Point, Point),
    Bus(Point, Point),
    /// A bus entry's bus end and direction.
    Entry(Point, EntryDir),
    Junction(Point),
    Label(NetLabel),
    Symbol {
        part: Id,
        component: Option<Id>,
        unit: u32,
        transform: Transform,
    },
    /// The cursor is on a connection point (a wire would end there).
    Target(Point),
}

/// A name the editor asks for after drawing (see [`SchPlacer::take_prompt`]). Answering
/// adds a label ([`SchPrompt::label`]).
#[derive(Clone, Debug, PartialEq)]
pub enum SchPrompt {
    /// A new bus has no name; its label goes to `pos`.
    BusName { pos: Point, rotation: Angle },
    /// An unnamed net leaves bus `bus` through an entry; its label goes to `pos`.
    EntryNet {
        bus: String,
        /// The bus's members, and those that are already a net.
        members: Vec<String>,
        used: Vec<String>,
        pos: Point,
        rotation: Angle,
    },
}

impl SchPrompt {
    /// Why `name` can't be the answer, if it can't.
    pub fn check(&self, name: &str) -> Result<(), String> {
        let name = name.trim();
        match self {
            _ if name.is_empty() => Err("Enter a name".into()),
            SchPrompt::BusName { .. } if !bus::is_bus_name(name) => {
                Err("Not a bus name: use D[0..7], {SDA SCL} or I2C{SDA SCL}".into())
            }
            SchPrompt::EntryNet { .. } if bus::is_bus_name(name) => Err("A signal name, not a bus name".into()),
            _ => Ok(()),
        }
    }

    /// The command adding the label `name`.
    pub fn label(&self, name: &str) -> Result<Command, String> {
        self.check(name)?;
        let (SchPrompt::BusName { pos, rotation } | SchPrompt::EntryNet { pos, rotation, .. }) = self;
        Ok(Command::AddLabel {
            sheet: None,
            pos: *pos,
            name: name.trim().to_string(),
            kind: LabelKind::Net,
            rotation: *rotation,
        })
    }
}

/// Direction of an axis-aligned vector as a label rotation (`None` if diagonal or zero).
fn axis_rotation(d: Point) -> Option<Angle> {
    match (d.x.signum(), d.y.signum()) {
        (1, 0) => Some(Angle::ZERO),
        (0, 1) => Some(Angle::DEG90),
        (-1, 0) => Some(Angle::deg(180.0)),
        (0, -1) => Some(Angle::DEG270),
        _ => None,
    }
}

fn prompt_for(project: &Project, conn: &Connectivity, finished: &[ObjectId], grid: Coord) -> Option<SchPrompt> {
    let s = &project.schematic;
    // A new bus without a name: label the middle of its longest new segment.
    let new_bus: Vec<&BusWire> = finished
        .iter()
        .filter_map(|id| if let ObjectId::BusWire(w) = id { s.bus_wires.get(*w) } else { None })
        .collect();
    if let Some(&first) = new_bus.first() {
        if !conn.bus_of(ObjectId::BusWire(first.id)).is_some_and(|b| b.name.is_empty()) {
            return None;
        }
        let w = new_bus.iter().max_by_key(|w| (w.b - w.a).length() as i64)?;
        let rotation = match axis_rotation(w.b - w.a) {
            Some(r) if r == Angle::ZERO || r == Angle::deg(180.0) => Angle::ZERO,
            Some(_) => Angle::DEG90,
            None => Angle::ZERO,
        };
        let mid = Point::new((w.a.x + w.b.x) / 2, (w.a.y + w.b.y) / 2);
        let snapped = if grid > 0 {
            Point::new(
                (mid.x as f64 / grid as f64).round() as Coord * grid,
                (mid.y as f64 / grid as f64).round() as Coord * grid,
            )
        } else {
            mid
        };
        let on = snapped == w.a || snapped == w.b || on_segment_inside(snapped, w.a, w.b);
        return Some(SchPrompt::BusName { pos: if on { snapped } else { mid }, rotation });
    }

    // An unnamed net now reaching a bus entry: offer the bus's members.
    for &id in finished {
        let Some(net) = conn.net_of(id) else { continue };
        if net.named || (net.wires.is_empty() && net.pins.is_empty()) {
            continue;
        }
        for &e in &net.entries {
            let Some(entry) = s.bus_entries.get(e) else { continue };
            let Some(bus) = conn.bus_of(ObjectId::BusEntry(e)) else { continue };
            if bus.members.is_empty() {
                continue;
            }
            // On the wire leaving the entry, a grid step away from it.
            let end = entry.wire_end();
            let leaving = net.wires.iter().filter_map(|&w| s.wires.get(w)).find_map(|w| {
                let other = if w.a == end {
                    w.b
                } else if w.b == end {
                    w.a
                } else {
                    return None;
                };
                let rotation = axis_rotation(other - end)?;
                let len = (other - end).length();
                let step = (BusEntry::SIZE as f64).min(len);
                let d = other - end;
                let pos = end + Point::new((d.x as f64 * step / len) as Coord, (d.y as f64 * step / len) as Coord);
                Some((pos, rotation))
            });
            let (pos, rotation) = leaving
                .unwrap_or_else(|| (end, if entry.dir.offset().x >= 0 { Angle::ZERO } else { Angle::deg(180.0) }));
            let used = bus.members.iter().filter(|m| conn.net(m).is_some()).cloned().collect();
            return Some(SchPrompt::EntryNet {
                bus: bus.name.clone(),
                members: bus.members.clone(),
                used,
                pos,
                rotation,
            });
        }
    }
    None
}

/// Id of ghost objects (never in a project).
const GHOST_ID: Id = Id::from_u128(0);

/// What a click did.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Placed {
    /// Created objects.
    pub objects: Vec<ObjectId>,
    /// The wire chain ended.
    pub finished: bool,
}

/// Active schematic tool and its in-progress state.
#[derive(Clone, Debug)]
pub struct SchPlacer {
    tool: SchTool,
    pub settings: SchSettings,
    pub placing: Option<Placing>,
    /// Start of the wire segment being drawn.
    start: Option<Point>,
    /// Wires or bus wires of the chain being drawn.
    chain: Vec<ObjectId>,
    /// Objects of finished chains and placed entries, for [`SchPlacer::take_prompt`].
    finished: Vec<ObjectId>,
    cursor: Option<Point>,
    /// The cursor is on a connection point.
    on_target: bool,
    /// Ghosts currently shown.
    shown: Vec<SchGhost>,
}

impl Default for SchPlacer {
    fn default() -> Self {
        SchPlacer {
            tool: SchTool::Select,
            settings: SchSettings::default(),
            placing: None,
            start: None,
            chain: Vec::new(),
            finished: Vec::new(),
            cursor: None,
            on_target: false,
            shown: Vec::new(),
        }
    }
}

/// Corner points of an orthogonal wire from `a` to `b`.
pub fn wire_path(a: Point, b: Point, vertical_first: bool) -> Vec<Point> {
    if a.x == b.x || a.y == b.y {
        return vec![a, b];
    }
    let corner = if vertical_first { Point::new(a.x, b.y) } else { Point::new(b.x, a.y) };
    vec![a, corner, b]
}

/// Is `p` a connection point: a pin, wire end or wire interior, label, junction or a bus
/// entry's wire end?
pub fn connects(project: &Project, conn: &Connectivity, p: Point) -> bool {
    let s = &project.schematic;
    s.wires.iter().any(|w| w.a == p || w.b == p || on_segment_inside(p, w.a, w.b))
        || s.labels.iter().any(|l| l.pos == p)
        || s.junctions.iter().any(|j| j.pos == p)
        || s.bus_entries.iter().any(|e| e.wire_end() == p)
        || conn.nets.iter().any(|n| n.pins.iter().any(|pin| pin.pos == p))
}

/// Is `p` on a bus: a bus wire's end or interior, or a bus entry's bus end?
pub fn on_bus(project: &Project, p: Point) -> bool {
    let s = &project.schematic;
    s.bus_wires.iter().any(|w| w.a == p || w.b == p || on_segment_inside(p, w.a, w.b))
        || s.bus_entries.iter().any(|e| e.pos == p)
}

impl Document {
    /// Is `p` a connection point of the schematic (see [`connects`])?
    pub fn connects(&mut self, p: Point) -> bool {
        self.update_connectivity(false);
        connects(self.project(), self.connectivity(), p)
    }
}

impl SchPlacer {
    pub fn tool(&self) -> SchTool {
        self.tool
    }

    pub fn is_placing(&self) -> bool {
        self.tool != SchTool::Select
    }

    /// A wire is half-done.
    pub fn in_progress(&self) -> bool {
        self.start.is_some()
    }

    pub fn set_tool(&mut self, tool: SchTool) {
        self.end_chain();
        self.tool = tool;
        self.on_target = false;
        if tool != SchTool::Place {
            self.placing = None;
        }
        if matches!(tool, SchTool::Label | SchTool::Power | SchTool::Place) {
            self.settings.rotation = Angle::ZERO;
            self.settings.mirror = false;
        }
    }

    /// Start placing `part` (imported into the project already).
    pub fn place(&mut self, part: Id) {
        self.set_tool(SchTool::Place);
        self.placing = Some(Placing { part, component: None, unit: 0 });
    }

    pub fn rotate(&mut self) {
        if self.tool == SchTool::BusEntry {
            // Counter-clockwise, like rotating placed objects.
            self.settings.entry = EntryDir::of_offset(self.settings.entry.offset().rotated(Angle::DEG90));
            return;
        }
        self.settings.rotation = self.settings.rotation.rotated_by(Angle::DEG90);
    }

    pub fn mirror(&mut self) {
        if self.tool == SchTool::BusEntry {
            let o = self.settings.entry.offset();
            self.settings.entry = EntryDir::of_offset(Point::new(-o.x, o.y));
            return;
        }
        self.settings.mirror = !self.settings.mirror;
    }

    pub fn toggle_posture(&mut self) {
        self.settings.vertical_first = !self.settings.vertical_first;
    }

    /// Escape: end the wire being drawn, or leave the tool. Returns true if the tool was left.
    pub fn escape(&mut self) -> bool {
        if self.start.is_some() {
            self.end_chain();
            return false;
        }
        self.set_tool(SchTool::Select);
        true
    }

    /// Track the (snapped) cursor.
    pub fn hover(&mut self, doc: &mut Document, p: Option<Point>) {
        if p != self.cursor {
            self.cursor = p;
            self.on_target = match self.tool {
                SchTool::Wire => p.is_some_and(|p| doc.connects(p)),
                SchTool::Bus | SchTool::BusEntry => p.is_some_and(|p| on_bus(doc.project(), p)),
                _ => false,
            };
        }
    }

    /// A click at the snapped point `p`.
    pub fn click(&mut self, doc: &mut Document, p: Point) -> Result<Placed, String> {
        self.cursor = Some(p);
        let s = &self.settings;
        let mut placed = Placed::default();
        let command = match self.tool {
            SchTool::Select => return Ok(placed),
            SchTool::Wire => {
                let Some(start) = self.start else {
                    self.start = Some(p);
                    return Ok(placed);
                };
                if start == p {
                    // Clicking the last point again ends the chain.
                    self.end_chain();
                    placed.finished = true;
                    return Ok(placed);
                }
                let ends = doc.connects(p);
                self.start = if ends { None } else { Some(p) };
                placed.finished = ends;
                Command::AddWire { sheet: None, points: wire_path(start, p, s.vertical_first) }
            }
            SchTool::Bus => {
                let Some(start) = self.start else {
                    self.start = Some(p);
                    return Ok(placed);
                };
                if start == p {
                    self.end_chain();
                    placed.finished = true;
                    return Ok(placed);
                }
                let ends = on_bus(doc.project(), p);
                self.start = if ends { None } else { Some(p) };
                placed.finished = ends;
                Command::AddBus { sheet: None, points: wire_path(start, p, s.vertical_first) }
            }
            SchTool::BusEntry => Command::AddBusEntry { sheet: None, pos: p, dir: s.entry },
            SchTool::Label => Command::AddLabel {
                sheet: None,
                pos: p,
                name: s.label.clone(),
                kind: LabelKind::Net,
                rotation: s.rotation,
            },
            SchTool::Power => Command::AddLabel {
                sheet: None,
                pos: p,
                name: s.power.clone(),
                kind: LabelKind::Power,
                rotation: s.rotation,
            },
            SchTool::Junction => Command::AddJunction { sheet: None, pos: p },
            SchTool::Place => {
                let Some(pl) = self.placing else { return Ok(placed) };
                match pl.component {
                    None => Command::PlacePart {
                        part: pl.part,
                        variant: None,
                        sheet: None,
                        pos: p,
                        rotation: s.rotation,
                        mirror: s.mirror,
                        refdes: None,
                        value: None,
                    },
                    Some(component) => Command::PlaceUnit {
                        component,
                        unit: pl.unit,
                        sheet: None,
                        pos: p,
                        rotation: s.rotation,
                        mirror: s.mirror,
                    },
                }
            }
        };
        let changes = doc.execute(command).map_err(|e| e.to_string())?;
        placed.objects = changes.changes.iter().filter(|c| c.is_insert()).map(|c| c.id).collect();
        match self.tool {
            SchTool::Wire | SchTool::Bus => {
                self.chain
                    .extend(placed.objects.iter().filter(|id| matches!(id, ObjectId::Wire(_) | ObjectId::BusWire(_))));
                if placed.finished {
                    self.end_chain();
                }
            }
            SchTool::BusEntry => self.finished.extend(placed.objects.iter().copied()),
            SchTool::Label => self.settings.label = increment_name(&self.settings.label),
            SchTool::Place => self.advance_unit(doc.project(), &placed.objects),
            _ => {}
        }
        Ok(placed)
    }

    /// Stop drawing the current chain; its wires are checked by [`Self::take_prompt`].
    fn end_chain(&mut self) {
        self.start = None;
        self.finished.append(&mut self.chain);
    }

    /// A label to ask for after a chain or entry was finished: the name of a new bus that
    /// has none, or the signal of an unnamed net reaching a bus entry. Asked once per
    /// finished chain or entry; `grid` places bus labels on grid points.
    pub fn take_prompt(&mut self, doc: &mut Document, grid: Coord) -> Option<SchPrompt> {
        if self.finished.is_empty() {
            return None;
        }
        let finished = std::mem::take(&mut self.finished);
        let conn = doc.current_connectivity().clone();
        prompt_for(doc.project(), &conn, &finished, grid)
    }

    /// After placing a unit: the next click places the component's next unit, or a new
    /// component once all units are placed.
    fn advance_unit(&mut self, project: &Project, created: &[ObjectId]) {
        let Some(pl) = &mut self.placing else { return };
        let units = project.library.parts.get(pl.part).map_or(1, |p| p.units.len()) as u32;
        let component = pl
            .component
            .or_else(|| created.iter().find_map(|id| if let ObjectId::Component(c) = id { Some(*c) } else { None }));
        // Skip units that are placed already.
        let placed = |u: u32| component.is_some_and(|c| project.schematic.symbols_of(c).any(|s| s.unit == u));
        let next = (pl.unit + 1..units).find(|&u| !placed(u));
        match next {
            Some(u) => {
                pl.component = component;
                pl.unit = u;
            }
            None => {
                pl.component = None;
                pl.unit = 0;
            }
        }
    }

    /// Ghosts to show, if they changed since the last call.
    pub fn take_preview(&mut self) -> Option<Vec<SchGhost>> {
        let g = self.ghosts();
        (g != self.shown).then(|| {
            self.shown = g.clone();
            g
        })
    }

    /// What to preview at the cursor.
    pub fn ghosts(&self) -> Vec<SchGhost> {
        let Some(p) = self.cursor else { return Vec::new() };
        let s = &self.settings;
        match self.tool {
            SchTool::Select => Vec::new(),
            SchTool::Wire | SchTool::Bus => {
                let mut g = Vec::new();
                if let Some(start) = self.start {
                    let path = wire_path(start, p, s.vertical_first);
                    let ghost = if self.tool == SchTool::Bus { SchGhost::Bus } else { SchGhost::Wire };
                    g.extend(path.windows(2).map(|w| ghost(w[0], w[1])));
                }
                if self.on_target {
                    g.push(SchGhost::Target(p));
                }
                g
            }
            SchTool::BusEntry => {
                let mut g = vec![SchGhost::Entry(p, s.entry)];
                if self.on_target {
                    g.push(SchGhost::Target(p));
                }
                g
            }
            SchTool::Label | SchTool::Power => {
                let (name, kind) = if self.tool == SchTool::Label {
                    (s.label.clone(), LabelKind::Net)
                } else {
                    (s.power.clone(), LabelKind::Power)
                };
                let (id, sheet) = (GHOST_ID, GHOST_ID);
                vec![SchGhost::Label(NetLabel { id, sheet, pos: p, name, kind, rotation: s.rotation })]
            }
            SchTool::Junction => vec![SchGhost::Junction(p)],
            SchTool::Place => self
                .placing
                .map(|pl| SchGhost::Symbol {
                    part: pl.part,
                    component: pl.component,
                    unit: pl.unit,
                    transform: Transform::new(p, s.rotation, s.mirror),
                })
                .into_iter()
                .collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wire_paths_are_orthogonal() {
        let (a, b) = (Point::mm(0.0, 0.0), Point::mm(5.08, 2.54));
        assert_eq!(wire_path(a, b, false), vec![a, Point::mm(5.08, 0.0), b]);
        assert_eq!(wire_path(a, b, true), vec![a, Point::mm(0.0, 2.54), b]);
        assert_eq!(wire_path(a, Point::mm(0.0, 5.0), false).len(), 2);
    }
}
