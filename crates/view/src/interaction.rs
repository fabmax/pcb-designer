//! Toolkit-independent selection and manipulation: click/shift-click selection, box
//! selection, hover, dragging with grid snapping (one undo step per drag), rotate, mirror,
//! flip, delete.

use std::sync::atomic::{AtomicU64, Ordering};

use editor_core::Command;
use model::*;

use crate::document::Document;
use crate::view::ViewKind;

/// Pointer state for one frame, in world coordinates.
#[derive(Clone, Copy, Debug, Default)]
pub struct PointerInput {
    /// Cursor position; `None` when the pointer is outside the canvas.
    pub world: Option<Point>,
    /// Primary button went down / up this frame.
    pub pressed: bool,
    pub released: bool,
    /// Primary button is held.
    pub down: bool,
    /// Add to the selection.
    pub shift: bool,
    /// Toggle in the selection (Cmd on macOS, Ctrl elsewhere).
    pub command: bool,
    /// Hit test tolerance (a few pixels, in nm).
    pub tolerance: Coord,
    /// Distance the pointer must move before a press becomes a drag (nm).
    pub drag_threshold: Coord,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyAction {
    Rotate,
    Mirror,
    Flip,
    Delete,
    Escape,
    SelectAll,
}

#[derive(Clone, Debug, PartialEq)]
enum State {
    Idle,
    /// Primary button down, not moved far enough to be a drag yet.
    Pressed {
        start: Point,
        hit: Option<ObjectId>,
        was_selected: bool,
    },
    Dragging(Drag),
    Boxing {
        start: Point,
        current: Point,
    },
}

#[derive(Clone, Debug, PartialEq)]
struct Drag {
    ids: Vec<ObjectId>,
    start: Point,
    /// Reference point of the grabbed object; it is snapped to the grid.
    anchor: Point,
    /// Total offset applied so far.
    moved: Point,
    group: u64,
    /// Something was executed in `group` (and can be undone on cancel).
    executed: bool,
}

static NEXT_GROUP: AtomicU64 = AtomicU64::new(1);

/// A fresh undo group id (see [`Document::execute_grouped`]).
pub fn new_group() -> u64 {
    NEXT_GROUP.fetch_add(1, Ordering::Relaxed)
}

pub struct Interaction {
    pub kind: ViewKind,
    pub selection: Vec<ObjectId>,
    pub hover: Option<ObjectId>,
    /// Snap grid in nm (0 disables snapping).
    pub grid: Coord,
    state: State,
    /// Last error from a command (e.g. moving a locked footprint), for the status bar.
    pub message: Option<String>,
}

impl Interaction {
    pub fn new(kind: ViewKind, grid: Coord) -> Self {
        Interaction { kind, selection: Vec::new(), hover: None, grid, state: State::Idle, message: None }
    }

    pub fn snap(&self, p: Point) -> Point {
        if self.grid <= 0 {
            return p;
        }
        let g = self.grid;
        let s = |v: Coord| (v as f64 / g as f64).round() as Coord * g;
        Point::new(s(p.x), s(p.y))
    }

    /// No press, drag or box selection in progress.
    pub fn is_idle(&self) -> bool {
        matches!(self.state, State::Idle)
    }

    pub fn is_dragging(&self) -> bool {
        matches!(self.state, State::Dragging(_))
    }

    /// Active box selection: rectangle and whether it is a crossing (right-to-left) box.
    pub fn box_rect(&self) -> Option<(Rect, bool)> {
        match self.state {
            State::Boxing { start, current } => Some((Rect::from_points(start, current), current.x < start.x)),
            _ => None,
        }
    }

    pub fn set_selection(&mut self, doc: &mut Document, ids: Vec<ObjectId>) {
        self.selection = ids;
        self.refresh(doc);
    }

    /// Drop stale ids (after undo, delete, file reload) and push highlights to the view.
    pub fn refresh(&mut self, doc: &mut Document) {
        let (project, view) = doc.project_and_view(self.kind);
        self.selection.retain(|id| project.contains(*id));
        if self.hover.is_some_and(|h| !project.contains(h)) {
            self.hover = None;
        }
        let hover = if matches!(self.state, State::Idle) { self.hover } else { None };
        view.set_highlight(project, &self.selection, hover);
    }

    pub fn pointer(&mut self, doc: &mut Document, input: &PointerInput) {
        let state = std::mem::replace(&mut self.state, State::Idle);
        self.state = match (state, input.world) {
            (State::Idle, Some(p)) => {
                self.hover = doc.view(self.kind).hit(doc.project(), p, input.tolerance);
                if input.pressed {
                    let hit = self.hover;
                    let was_selected = hit.is_some_and(|h| self.selection.contains(&h));
                    if let Some(h) = hit
                        && !was_selected
                    {
                        if !(input.shift || input.command) {
                            self.selection.clear();
                        }
                        self.selection.push(h);
                    }
                    State::Pressed { start: p, hit, was_selected }
                } else {
                    State::Idle
                }
            }
            (State::Idle, None) => {
                self.hover = None;
                State::Idle
            }
            (State::Pressed { start, hit, was_selected }, p) => {
                let p = p.unwrap_or(start);
                if input.released || !input.down {
                    // Click without drag.
                    match hit {
                        Some(h) if was_selected && input.command => self.selection.retain(|&s| s != h),
                        Some(h) if was_selected && !input.shift => self.selection = vec![h],
                        Some(_) => {}
                        None if !(input.shift || input.command) => self.selection.clear(),
                        None => {}
                    }
                    State::Idle
                } else if (p - start).length() > input.drag_threshold as f64 {
                    match hit {
                        Some(h) => self.start_drag(doc, start, h).map_or(State::Idle, State::Dragging),
                        None => State::Boxing { start, current: p },
                    }
                } else {
                    State::Pressed { start, hit, was_selected }
                }
            }
            (State::Dragging(mut drag), p) => {
                if let Some(p) = p {
                    self.drag_to(doc, &mut drag, p);
                }
                if input.released || !input.down {
                    doc.close_group();
                    State::Idle
                } else {
                    State::Dragging(drag)
                }
            }
            (State::Boxing { start, current }, p) => {
                let current = p.unwrap_or(current);
                if input.released || !input.down {
                    let crossing = current.x < start.x;
                    let found = doc.view(self.kind).select_rect(Rect::from_points(start, current), crossing);
                    if !(input.shift || input.command) {
                        self.selection.clear();
                    }
                    for id in found {
                        if !self.selection.contains(&id) {
                            self.selection.push(id);
                        }
                    }
                    State::Idle
                } else {
                    State::Boxing { start, current }
                }
            }
        };
        self.refresh(doc);
    }

    fn start_drag(&mut self, doc: &Document, start: Point, grabbed: ObjectId) -> Option<Drag> {
        let project = doc.project();
        let ids: Vec<ObjectId> = self.selection.iter().copied().filter(|&id| movable(project, id)).collect();
        if ids.is_empty() {
            self.message = Some("Nothing movable selected (locked footprints stay in place)".into());
            return None;
        }
        let anchor = reference_point(project, grabbed).unwrap_or(start);
        let group = new_group();
        Some(Drag { ids, start, anchor, moved: Point::ZERO, group, executed: false })
    }

    fn drag_to(&mut self, doc: &mut Document, drag: &mut Drag, p: Point) {
        let target = self.snap(drag.anchor + (p - drag.start)) - drag.anchor;
        let step = target - drag.moved;
        if step == Point::ZERO {
            return;
        }
        let cmd = Command::Move { ids: drag.ids.clone(), delta: step };
        match doc.execute_grouped(cmd, Some(drag.group)) {
            Ok(_) => {
                drag.moved = target;
                drag.executed = true;
            }
            Err(e) => self.message = Some(e.to_string()),
        }
    }

    /// Keyboard actions. `cursor` is the pointer position, used as pivot when nothing is
    /// being dragged and the selection is a single object.
    pub fn key(&mut self, doc: &mut Document, action: KeyAction) {
        self.message = None;
        match action {
            KeyAction::Escape => match std::mem::replace(&mut self.state, State::Idle) {
                State::Dragging(drag) => {
                    doc.close_group();
                    if drag.executed {
                        doc.undo();
                    }
                }
                State::Boxing { .. } | State::Pressed { .. } => {}
                State::Idle => self.selection.clear(),
            },
            KeyAction::SelectAll => {
                let mut ids = doc.view(self.kind).index.ids();
                ids.sort();
                self.selection = ids;
            }
            KeyAction::Delete => {
                if self.is_dragging() {
                    return;
                }
                let (ids, footprints): (Vec<ObjectId>, Vec<ObjectId>) =
                    self.selection.iter().partition(|id| !matches!(id, ObjectId::Footprint(_)));
                if !footprints.is_empty() {
                    self.message = Some("Footprints follow the schematic: delete the component there".into());
                }
                if !ids.is_empty() {
                    self.run(doc, Command::Delete { ids });
                }
            }
            KeyAction::Rotate => {
                let angle = Angle::DEG90;
                if let State::Dragging(drag) = &mut self.state {
                    let center = drag.anchor + drag.moved;
                    let cmd = Command::Rotate { ids: drag.ids.clone(), center, angle };
                    match doc.execute_grouped(cmd, Some(drag.group)) {
                        Ok(_) => drag.executed = true,
                        Err(e) => self.message = Some(e.to_string()),
                    }
                } else if let Some(center) = self.pivot(doc) {
                    let ids = self.selection.clone();
                    self.run(doc, Command::Rotate { ids, center, angle });
                }
            }
            KeyAction::Mirror | KeyAction::Flip => {
                if self.is_dragging() {
                    return;
                }
                let Some(center) = self.pivot(doc) else { return };
                match self.kind {
                    ViewKind::Schematic => {
                        let ids = self.selection.clone();
                        self.run(doc, Command::Mirror { ids, axis_x: center.x });
                    }
                    ViewKind::Board => {
                        let ids: Vec<ObjectId> =
                            self.selection.iter().copied().filter(|id| matches!(id, ObjectId::Footprint(_))).collect();
                        if ids.is_empty() {
                            self.message = Some("Only footprints can be flipped".into());
                        } else {
                            self.run(doc, Command::FlipSide { ids, axis_x: center.x });
                        }
                    }
                }
            }
        }
        self.refresh(doc);
    }

    fn run(&mut self, doc: &mut Document, cmd: Command) {
        if let Err(e) = doc.execute(cmd) {
            self.message = Some(e.to_string());
        }
    }

    /// Rotation/mirror pivot: the reference point of a single object, otherwise the
    /// snapped center of the selection bounds.
    fn pivot(&self, doc: &Document) -> Option<Point> {
        if let [id] = self.selection[..]
            && let Some(p) = reference_point(doc.project(), id)
        {
            return Some(p);
        }
        let view = doc.view(self.kind);
        let bounds = Rect::union_all(self.selection.iter().filter_map(|&id| view.index.bounds(id)))?;
        Some(self.snap(bounds.center()))
    }
}

/// Can `id` be moved interactively?
fn movable(project: &Project, id: ObjectId) -> bool {
    match id {
        ObjectId::Footprint(f) => project.board.footprints.get(f).is_some_and(|f| !f.locked),
        _ => reference_point(project, id).is_some(),
    }
}

/// The point that snaps to the grid when an object is dragged.
pub fn reference_point(project: &Project, id: ObjectId) -> Option<Point> {
    let s = &project.schematic;
    let b = &project.board;
    match id {
        ObjectId::Symbol(i) => s.symbols.get(i).map(|o| o.pos),
        ObjectId::Wire(i) => s.wires.get(i).map(|o| o.a),
        ObjectId::Junction(i) => s.junctions.get(i).map(|o| o.pos),
        ObjectId::Label(i) => s.labels.get(i).map(|o| o.pos),
        ObjectId::SchText(i) => s.texts.get(i).map(|o| o.text.pos),
        ObjectId::Footprint(i) => b.footprints.get(i).map(|o| o.pos),
        ObjectId::Track(i) => b.tracks.get(i).map(|o| o.a),
        ObjectId::Via(i) => b.vias.get(i).map(|o| o.pos),
        ObjectId::Zone(i) => b.zones.get(i).and_then(|o| o.outline.first().copied()),
        ObjectId::BoardGraphic(i) => b.graphics.get(i).map(|o| match &o.shape {
            Shape::Line { a, .. } | Shape::Rect { a, .. } => *a,
            Shape::Arc { center, .. } | Shape::Circle { center, .. } => *center,
            Shape::Polygon { points, .. } => points.first().copied().unwrap_or_default(),
        }),
        ObjectId::BoardText(i) => b.texts.get(i).map(|o| o.text.pos),
        _ => None,
    }
}
