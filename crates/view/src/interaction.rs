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
    /// Alt (Option): a box selects what it touches, not only what it encloses.
    pub alt: bool,
    /// Hit test tolerance (a few pixels, in nm).
    pub tolerance: Coord,
    /// Distance the pointer must move before a press becomes a drag (nm).
    pub drag_threshold: Coord,
    /// Cursor position on screen (points, y down), if known. A press becomes a drag only
    /// when the hand moves the pointer, not when the view moves under a still pointer
    /// (a panel changing width shifts the canvas).
    pub screen: Option<[f64; 2]>,
    /// World length of one screen point (nm); 0 if unknown.
    pub nm_per_point: f64,
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
    /// Carrying a paste: the edit moves only these objects; schematic wires attached to
    /// them don't follow (they belong to whatever the paste passes over).
    Carry(Box<Edit<I>>),
}

impl<I> Edit<I> {
    /// Wrapped in [`Edit::Carry`] if `carry`.
    fn carried(self, carry: bool) -> Self {
        if carry { Edit::Carry(Box::new(self)) } else { self }
    }
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
    /// Take back the open undo group `group` without a redo step (see
    /// [`Document::retract`](crate::Document::retract)).
    fn retract(&mut self, group: u64);
    /// Show `selection` as selected (ids may repeat) and `hover` as hovered.
    fn highlight(&mut self, selection: &[Self::Id], hover: Option<Self::Id>);
    /// What a double click on `id` selects (a wire: its whole chain).
    fn chain(&self, id: Self::Id) -> Vec<Self::Id> {
        vec![id]
    }
    /// The only direction dragging `ids` may move them in (a single straight segment
    /// moves perpendicular to itself): a vector along it, or `None` if free.
    fn drag_axis(&self, _ids: &[Self::Id]) -> Option<Point> {
        None
    }
}

#[derive(Clone, Debug, PartialEq)]
enum State<I> {
    Idle,
    /// Primary button down, not moved far enough to be a drag yet.
    Pressed {
        start: Point,
        /// The pointer's screen position at the press (if known).
        start_screen: Option<[f64; 2]>,
        hit: Option<I>,
        was_selected: bool,
    },
    Dragging(Drag<I>),
    Boxing {
        start: Point,
        current: Point,
        /// Alt is held: the box selects what it touches, not only what it encloses.
        crossing: bool,
        /// Shift or Cmd is held: the box adds to the selection.
        add: bool,
        /// What the box selects as it stands, shown as selected while dragging.
        found: Vec<I>,
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
    /// Rotation applied so far (about `anchor`, before the offset).
    angle: Angle,
    /// Direction a single segment is constrained to (see [`Target::drag_axis`]).
    axis: Option<Point>,
    group: u64,
    /// Something was executed in `group` (and can be undone on cancel).
    executed: bool,
    /// Pasted objects following the pointer without a button held; a click drops them.
    carry: bool,
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

    /// Pasted objects are following the pointer.
    pub fn is_carrying(&self) -> bool {
        matches!(&self.state, State::Dragging(d) if d.carry)
    }

    /// The undo group of the objects being carried.
    pub fn carry_group(&self) -> Option<u64> {
        match &self.state {
            State::Dragging(d) if d.carry => Some(d.group),
            _ => None,
        }
    }

    /// Let `ids`, just pasted in undo group `group` with their reference point at
    /// `anchor`, follow the pointer (now at `pointer`) until a click drops them. Moves
    /// join `group`; Esc undoes it, paste included.
    pub fn carry<T: Target<Id = I>>(&mut self, t: &mut T, ids: Vec<I>, anchor: Point, pointer: Point, group: u64) {
        self.selection = ids.clone();
        self.hover = None;
        self.message = None;
        self.state = State::Dragging(Drag {
            ids,
            start: pointer,
            anchor,
            moved: Point::ZERO,
            angle: Angle::ZERO,
            axis: None,
            group,
            executed: true,
            carry: true,
        });
        self.refresh(t);
    }

    /// Active box selection: rectangle and whether it is a crossing (Alt held) box.
    pub fn box_rect(&self) -> Option<(Rect, bool)> {
        match self.state {
            State::Boxing { start, current, crossing, .. } => Some((Rect::from_points(start, current), crossing)),
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
        match &mut self.state {
            // The selection the box would leave, live.
            State::Boxing { add, found, .. } => {
                found.retain(|&id| t.exists(id));
                // Ids may repeat: `highlight` takes them as a set.
                let kept: &[I] = if *add { &self.selection } else { &[] };
                let shown: Vec<I> = kept.iter().chain(found.iter()).copied().collect();
                t.highlight(&shown, None);
            }
            _ => t.highlight(&self.selection, hover),
        }
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
                    State::Pressed { start: p, start_screen: input.screen, hit, was_selected }
                } else {
                    State::Idle
                }
            }
            (State::Idle, None) => {
                self.hover = None;
                State::Idle
            }
            (State::Pressed { start, start_screen, hit, was_selected }, p) => {
                let p = p.unwrap_or(start);
                // How far the hand moved, in world units: on screen when known.
                let moved = match (start_screen, input.screen) {
                    (Some(a), Some(b)) if input.nm_per_point > 0.0 => {
                        let (dx, dy) = ((b[0] - a[0]) * input.nm_per_point, (b[1] - a[1]) * input.nm_per_point);
                        Point::new(dx.round() as Coord, -dy.round() as Coord)
                    }
                    _ => p - start,
                };
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
                } else if moved.length() > input.drag_threshold as f64 {
                    match hit {
                        // Grabbed where the pointer is now, less the hand's motion: if the
                        // view moved meanwhile, the object doesn't jump with it.
                        Some(h) => self.start_drag(t, p - moved, h).map_or(State::Idle, State::Dragging),
                        None => State::Boxing {
                            start,
                            current: p,
                            crossing: input.alt,
                            add: input.shift || input.command,
                            found: t.in_rect(Rect::from_points(start, p), input.alt),
                        },
                    }
                } else {
                    State::Pressed { start, start_screen, hit, was_selected }
                }
            }
            (State::Dragging(mut drag), p) => {
                if let Some(p) = p {
                    self.drag_to(t, &mut drag, p);
                }
                if drag.carry {
                    if input.pressed && p.is_some() {
                        // Dropped; Idle ignores this click's release.
                        t.close_group();
                        State::Idle
                    } else {
                        State::Dragging(drag)
                    }
                } else if input.released || !input.down {
                    t.close_group();
                    State::Idle
                } else {
                    State::Dragging(drag)
                }
            }
            (State::Boxing { start, current: old, crossing: was, mut found, .. }, p) => {
                let current = p.unwrap_or(old);
                // Modifiers may go down or up during the drag; their state at the release
                // counts. The box is only searched again when it changed.
                let (crossing, add) = (input.alt, input.shift || input.command);
                if current != old || crossing != was {
                    found = t.in_rect(Rect::from_points(start, current), crossing);
                }
                if input.released || !input.down {
                    found.sort();
                    if !add {
                        self.selection.clear();
                    }
                    for id in found {
                        if !self.selection.contains(&id) {
                            self.selection.push(id);
                        }
                    }
                    State::Idle
                } else {
                    State::Boxing { start, current, crossing, add, found }
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
        let axis = t.drag_axis(&ids);
        Some(Drag {
            ids,
            start,
            anchor,
            moved: Point::ZERO,
            angle: Angle::ZERO,
            axis,
            group: new_group(),
            executed: false,
            carry: false,
        })
    }

    /// Offset of a drag to pointer `p`: snapped, and along the segment's axis if any.
    fn drag_offset(&self, drag: &Drag<I>, p: Point) -> Point {
        let free = p - drag.start;
        let Some(axis) = drag.axis.filter(|_| drag.angle == Angle::ZERO) else {
            return self.snap(drag.anchor + free) - drag.anchor;
        };
        // Perpendicular to the segment.
        let n = Point::new(-axis.y, axis.x);
        if n.x == 0 || n.y == 0 {
            let snapped = self.snap(drag.anchor + free) - drag.anchor;
            return if n.x == 0 { Point::new(0, snapped.y) } else { Point::new(snapped.x, 0) };
        }
        let (nx, ny) = (n.x as f64, n.y as f64);
        let k = (free.x as f64 * nx + free.y as f64 * ny) / (nx * nx + ny * ny);
        if n.x.abs() == n.y.abs() && self.snap && self.grid > 0 {
            // 45°: whole grid steps on both axes.
            let unit = Point::new(n.x.signum(), n.y.signum());
            let steps = (k * nx.abs() / self.grid as f64).round() as Coord;
            return Point::new(unit.x * steps * self.grid, unit.y * steps * self.grid);
        }
        Point::new((k * nx).round() as Coord, (k * ny).round() as Coord)
    }

    fn drag_to<T: Target<Id = I>>(&mut self, t: &mut T, drag: &mut Drag<I>, p: Point) {
        let target = self.drag_offset(drag, p);
        if target == drag.moved {
            return;
        }
        if drag.carry {
            // A carried paste moves by steps: its undo group holds the paste itself.
            let step = target - drag.moved;
            match t.edit(Edit::Move { ids: drag.ids.clone(), delta: step }.carried(true), Some(drag.group)) {
                Ok(note) => {
                    drag.moved = target;
                    drag.executed = true;
                    if note.is_some() {
                        self.message = note;
                    }
                }
                Err(e) => self.message = Some(e),
            }
            return;
        }
        let last = drag.moved;
        drag.moved = target;
        self.replay(t, drag);
        if !drag.executed && last != Point::ZERO {
            // Refused (e.g. a shove that can't push something aside): stay where it last
            // went, keeping the message.
            let message = self.message.take();
            drag.moved = last;
            self.replay(t, drag);
            self.message = message;
        }
    }

    /// Redo a drag from its start: undo what it did so far and apply its whole rotation
    /// and offset at once. What stays attached is then decided by the geometry at the
    /// start, never by what the drag passed over.
    fn replay<T: Target<Id = I>>(&mut self, t: &mut T, drag: &mut Drag<I>) {
        if drag.executed {
            t.retract(drag.group);
            drag.executed = false;
        }
        let mut edits = Vec::new();
        if drag.angle != Angle::ZERO {
            edits.push(Edit::Rotate { ids: drag.ids.clone(), center: drag.anchor, angle: drag.angle });
        }
        if drag.moved != Point::ZERO {
            edits.push(Edit::Move { ids: drag.ids.clone(), delta: drag.moved });
        }
        for edit in edits {
            match t.edit(edit, Some(drag.group)) {
                Ok(note) => {
                    drag.executed = true;
                    if note.is_some() {
                        self.message = note;
                    }
                }
                Err(e) => {
                    self.message = Some(e);
                    break;
                }
            }
        }
    }

    /// Double click: select the chain of the item under the pointer (with `add`, add it
    /// to the selection).
    pub fn double_click<T: Target<Id = I>>(&mut self, t: &mut T, add: bool) {
        let Some(h) = self.hover else { return };
        match self.state {
            State::Idle => {}
            // The second press may not have been released yet: the double click uses it up.
            State::Pressed { .. } => self.state = State::Idle,
            _ => return,
        }
        let chain = t.chain(h);
        if !add {
            self.selection.clear();
        }
        for id in chain {
            if !self.selection.contains(&id) {
                self.selection.push(id);
            }
        }
        self.refresh(t);
    }

    pub fn key<T: Target<Id = I>>(&mut self, t: &mut T, action: KeyAction) {
        self.message = None;
        match action {
            KeyAction::Escape => match std::mem::replace(&mut self.state, State::Idle) {
                State::Dragging(drag) => {
                    if drag.executed {
                        t.retract(drag.group);
                    }
                    t.close_group();
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
                    if drag.carry {
                        let center = drag.anchor + drag.moved;
                        let edit = Edit::Carry(Box::new(Edit::Rotate { ids: drag.ids.clone(), center, angle }));
                        match t.edit(edit, Some(drag.group)) {
                            Ok(_) => drag.executed = true,
                            Err(e) => self.message = Some(e),
                        }
                    } else {
                        // Rotating about the anchor, then moving, is the same as moving,
                        // then rotating about the moved anchor.
                        drag.angle = drag.angle.rotated_by(angle);
                        let mut drag = drag.clone();
                        self.replay(t, &mut drag);
                        self.state = State::Dragging(drag);
                    }
                } else if let Some(center) = self.pivot(t) {
                    let ids = self.selection.clone();
                    self.run(t, Edit::Rotate { ids, center, angle }, None);
                }
            }
            KeyAction::Mirror | KeyAction::Flip => {
                if let State::Dragging(drag) = &mut self.state {
                    if drag.carry {
                        let axis_x = (drag.anchor + drag.moved).x;
                        let ids = drag.ids.clone();
                        let edit = if action == KeyAction::Mirror {
                            Edit::Mirror { ids, axis_x }
                        } else {
                            Edit::Flip { ids, axis_x }
                        };
                        match t.edit(Edit::Carry(Box::new(edit)), Some(drag.group)) {
                            Ok(_) => drag.executed = true,
                            Err(e) => self.message = Some(e),
                        }
                        self.refresh(t);
                    }
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
        ObjectId::Sheet(i) => s.sheets.get(i).map(|o| o.pos),
        ObjectId::Symbol(i) => s.symbols.get(i).map(|o| o.pos),
        ObjectId::Wire(i) => s.wires.get(i).map(|o| o.a),
        ObjectId::BusWire(i) => s.bus_wires.get(i).map(|o| o.a),
        ObjectId::BusEntry(i) => s.bus_entries.get(i).map(|o| o.pos),
        ObjectId::Junction(i) => s.junctions.get(i).map(|o| o.pos),
        ObjectId::NoConnect(i) => s.no_connects.get(i).map(|o| o.pos),
        ObjectId::PowerFlag(i) => s.power_flags.get(i).map(|o| o.pos),
        ObjectId::Label(i) => s.labels.get(i).map(|o| o.pos),
        ObjectId::SchText(i) => s.texts.get(i).map(|o| o.text.pos),
        ObjectId::Footprint(i) => b.footprints.get(i).map(|o| o.pos),
        ObjectId::SymbolField(i) => {
            let f = s.fields.get(i)?;
            s.symbols.get(f.symbol).map(|sym| sym.transform().point(f.text.pos))
        }
        ObjectId::FootprintField(i) => {
            let f = b.fields.get(i)?;
            b.footprints.get(f.footprint).map(|fp| fp.transform().point(f.text.pos))
        }
        ObjectId::Track(i) => b.tracks.get(i).map(|o| o.a),
        ObjectId::Via(i) => b.vias.get(i).map(|o| o.pos),
        ObjectId::Zone(i) => b.zones.get(i).and_then(|o| o.outline.first().copied()),
        ObjectId::Keepout(i) => b.keepouts.get(i).and_then(|o| o.outline.first().copied()),
        ObjectId::BoardGraphic(i) => b.graphics.get(i).map(|o| match &o.shape {
            Shape::Line { a, .. } | Shape::Rect { a, .. } => *a,
            Shape::Arc { center, .. } | Shape::Circle { center, .. } => *center,
            Shape::Polygon { points, .. } => points.first().copied().unwrap_or_default(),
        }),
        ObjectId::BoardText(i) => b.texts.get(i).map(|o| o.text.pos),
        _ => None,
    }
}
