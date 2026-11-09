//! Toolkit-independent selection and manipulation: click/shift-click selection, box
//! selection, hover, dragging with grid snapping (one undo step per drag), rotate, mirror,
//! flip, delete.

use std::sync::atomic::{AtomicU64, Ordering};

use model::*;

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

/// An edit requested by the interaction, applied by the [`Target`].
#[derive(Clone, Debug, PartialEq)]
pub enum Edit<I> {
    Move {
        ids: Vec<I>,
        delta: Point,
    },
    Rotate {
        ids: Vec<I>,
        center: Point,
        angle: Angle,
    },
    Mirror {
        ids: Vec<I>,
        axis_x: Coord,
    },
    /// Board side flip (footprints); targets without sides may treat it like `Mirror`.
    Flip {
        ids: Vec<I>,
        axis_x: Coord,
    },
    Delete {
        ids: Vec<I>,
    },
}

/// Something editable on a canvas: a schematic or board view of a project, a symbol, a
/// footprint. [`Interaction`] implements the generic behavior on top of it.
pub trait Target {
    type Id: Copy + Eq + Ord + std::hash::Hash + std::fmt::Debug;

    /// Topmost item at `p` (within `tol`).
    fn hit(&self, p: Point, tol: Coord) -> Option<Self::Id>;
    /// Items inside `r` (fully, or touching when `crossing`).
    fn in_rect(&self, r: Rect, crossing: bool) -> Vec<Self::Id>;
    fn all(&self) -> Vec<Self::Id>;
    fn bounds(&self, id: Self::Id) -> Option<Rect>;
    fn exists(&self, id: Self::Id) -> bool;
    /// Point that snaps to the grid when the item is dragged; `None` if it can't move.
    fn anchor(&self, id: Self::Id) -> Option<Point>;
    /// Apply an edit, as part of undo group `group` if given. `Ok(Some(note))`: applied,
    /// with a remark for the user (e.g. some items were skipped).
    fn edit(&mut self, edit: Edit<Self::Id>, group: Option<u64>) -> Result<Option<String>, String>;
    fn close_group(&mut self);
    fn undo(&mut self);
    fn highlight(&mut self, selection: &[Self::Id], hover: Option<Self::Id>);
}

#[derive(Clone, Debug, PartialEq)]
enum State<I> {
    Idle,
    /// Primary button down, not moved far enough to be a drag yet.
    Pressed {
        start: Point,
        hit: Option<I>,
        was_selected: bool,
    },
    Dragging(Drag<I>),
    Boxing {
        start: Point,
        current: Point,
    },
}

#[derive(Clone, Debug, PartialEq)]
struct Drag<I> {
    ids: Vec<I>,
    start: Point,
    /// Reference point of the grabbed item; it is snapped to the grid.
    anchor: Point,
    /// Total offset applied so far.
    moved: Point,
    group: u64,
    /// Something was executed in `group` (and can be undone on cancel).
    executed: bool,
}

static NEXT_GROUP: AtomicU64 = AtomicU64::new(1);

/// A fresh undo group id (see [`Document::execute_grouped`](crate::Document::execute_grouped)).
pub fn new_group() -> u64 {
    NEXT_GROUP.fetch_add(1, Ordering::Relaxed)
}

/// Selection state and pointer/keyboard handling for one canvas.
pub struct Interaction<I> {
    pub selection: Vec<I>,
    pub hover: Option<I>,
    /// Snap grid in nm (0 disables snapping).
    pub grid: Coord,
    /// Snap to the grid (off: positions are free; the grid stays for display).
    pub snap: bool,
    state: State<I>,
    /// Last error or remark from an edit (e.g. moving a locked footprint), for the status bar.
    pub message: Option<String>,
}

impl<I: Copy + Eq + Ord + std::hash::Hash + std::fmt::Debug> Interaction<I> {
    pub fn new(grid: Coord) -> Self {
        Interaction { selection: Vec::new(), hover: None, grid, snap: true, state: State::Idle, message: None }
    }

    pub fn snap(&self, p: Point) -> Point {
        if !self.snap || self.grid <= 0 {
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

    pub fn set_selection<T: Target<Id = I>>(&mut self, t: &mut T, ids: Vec<I>) {
        self.selection = ids;
        self.refresh(t);
    }

    /// Forget selection and any gesture in progress (e.g. when the target is replaced).
    pub fn reset(&mut self) {
        self.selection.clear();
        self.hover = None;
        self.state = State::Idle;
        self.message = None;
    }

    /// Drop stale ids (after undo, delete, file reload) and push highlights to the target.
    pub fn refresh<T: Target<Id = I>>(&mut self, t: &mut T) {
        self.selection.retain(|&id| t.exists(id));
        if self.hover.is_some_and(|h| !t.exists(h)) {
            self.hover = None;
        }
        let hover = if matches!(self.state, State::Idle) { self.hover } else { None };
        t.highlight(&self.selection, hover);
    }

    pub fn pointer<T: Target<Id = I>>(&mut self, t: &mut T, input: &PointerInput) {
        let state = std::mem::replace(&mut self.state, State::Idle);
        self.state = match (state, input.world) {
            (State::Idle, Some(p)) => {
                self.hover = t.hit(p, input.tolerance);
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
                        Some(h) => self.start_drag(t, start, h).map_or(State::Idle, State::Dragging),
                        None => State::Boxing { start, current: p },
                    }
                } else {
                    State::Pressed { start, hit, was_selected }
                }
            }
            (State::Dragging(mut drag), p) => {
                if let Some(p) = p {
                    self.drag_to(t, &mut drag, p);
                }
                if input.released || !input.down {
                    t.close_group();
                    State::Idle
                } else {
                    State::Dragging(drag)
                }
            }
            (State::Boxing { start, current }, p) => {
                let current = p.unwrap_or(current);
                if input.released || !input.down {
                    let crossing = current.x < start.x;
                    let mut found = t.in_rect(Rect::from_points(start, current), crossing);
                    found.sort();
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
        self.refresh(t);
    }

    fn start_drag<T: Target<Id = I>>(&mut self, t: &T, start: Point, grabbed: I) -> Option<Drag<I>> {
        let ids: Vec<I> = self.selection.iter().copied().filter(|&id| t.anchor(id).is_some()).collect();
        if ids.is_empty() {
            self.message = Some("Nothing movable selected (locked items stay in place)".into());
            return None;
        }
        let anchor = t.anchor(grabbed).unwrap_or(start);
        Some(Drag { ids, start, anchor, moved: Point::ZERO, group: new_group(), executed: false })
    }

    fn drag_to<T: Target<Id = I>>(&mut self, t: &mut T, drag: &mut Drag<I>, p: Point) {
        let target = self.snap(drag.anchor + (p - drag.start)) - drag.anchor;
        let step = target - drag.moved;
        if step == Point::ZERO {
            return;
        }
        match t.edit(Edit::Move { ids: drag.ids.clone(), delta: step }, Some(drag.group)) {
            Ok(note) => {
                drag.moved = target;
                drag.executed = true;
                if note.is_some() {
                    self.message = note;
                }
            }
            Err(e) => self.message = Some(e),
        }
    }

    pub fn key<T: Target<Id = I>>(&mut self, t: &mut T, action: KeyAction) {
        self.message = None;
        match action {
            KeyAction::Escape => match std::mem::replace(&mut self.state, State::Idle) {
                State::Dragging(drag) => {
                    t.close_group();
                    if drag.executed {
                        t.undo();
                    }
                }
                State::Boxing { .. } | State::Pressed { .. } => {}
                State::Idle => self.selection.clear(),
            },
            KeyAction::SelectAll => {
                let mut ids = t.all();
                ids.sort();
                self.selection = ids;
            }
            KeyAction::Delete => {
                if !self.is_dragging() && !self.selection.is_empty() {
                    let ids = self.selection.clone();
                    self.run(t, Edit::Delete { ids }, None);
                }
            }
            KeyAction::Rotate => {
                let angle = Angle::DEG90;
                if let State::Dragging(drag) = &mut self.state {
                    let center = drag.anchor + drag.moved;
                    let edit = Edit::Rotate { ids: drag.ids.clone(), center, angle };
                    match t.edit(edit, Some(drag.group)) {
                        Ok(_) => drag.executed = true,
                        Err(e) => self.message = Some(e),
                    }
                } else if let Some(center) = self.pivot(t) {
                    let ids = self.selection.clone();
                    self.run(t, Edit::Rotate { ids, center, angle }, None);
                }
            }
            KeyAction::Mirror | KeyAction::Flip => {
                if self.is_dragging() {
                    return;
                }
                let Some(center) = self.pivot(t) else { return };
                let ids = self.selection.clone();
                let edit = if action == KeyAction::Mirror {
                    Edit::Mirror { ids, axis_x: center.x }
                } else {
                    Edit::Flip { ids, axis_x: center.x }
                };
                self.run(t, edit, None);
            }
        }
        self.refresh(t);
    }

    fn run<T: Target<Id = I>>(&mut self, t: &mut T, edit: Edit<I>, group: Option<u64>) {
        match t.edit(edit, group) {
            Ok(note) => self.message = note,
            Err(e) => self.message = Some(e),
        }
    }

    /// Rotation/mirror pivot: the anchor of a single item, otherwise the snapped center of
    /// the selection bounds.
    fn pivot<T: Target<Id = I>>(&self, t: &T) -> Option<Point> {
        if let [id] = self.selection[..]
            && let Some(p) = t.anchor(id)
        {
            return Some(p);
        }
        let bounds = Rect::union_all(self.selection.iter().filter_map(|&id| t.bounds(id)))?;
        Some(self.snap(bounds.center()))
    }
}

/// The point that snaps to the grid when an object is dragged.
pub fn reference_point(project: &Project, id: ObjectId) -> Option<Point> {
    let s = &project.schematic;
    let b = &project.board;
    match id {
        ObjectId::Symbol(i) => s.symbols.get(i).map(|o| o.pos),
        ObjectId::Wire(i) => s.wires.get(i).map(|o| o.a),
        ObjectId::BusWire(i) => s.bus_wires.get(i).map(|o| o.a),
        ObjectId::BusEntry(i) => s.bus_entries.get(i).map(|o| o.pos),
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
