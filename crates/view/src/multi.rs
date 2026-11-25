//! Editing several selected objects at once (the Properties panel): the properties all of
//! them have, their shared value (or none when they differ), and one command setting it
//! on all of them.
//!
//! Positions are left out on purpose: one position for several objects would stack them.
//! Reference and value texts whose symbol or footprint is selected too ride along with
//! it and don't count.

use editor_core::Command;
use model::*;

/// A property several kinds of objects can share.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Prop {
    /// Tracks.
    TrackWidth,
    /// Tracks and zones: a copper layer.
    CopperLayer,
    /// Tracks, vias and footprints.
    Locked,
    ViaDiameter,
    ViaDrill,
    /// Schematic and board texts, reference and value texts.
    TextSize,
    TextBold,
    /// Reference and value texts.
    Visible,
    /// Symbols and footprints.
    Rotation,
    /// Symbols.
    Mirrored,
}

impl Prop {
    /// In the order the panel shows them.
    pub const ALL: [Prop; 10] = [
        Prop::CopperLayer,
        Prop::TrackWidth,
        Prop::ViaDiameter,
        Prop::ViaDrill,
        Prop::TextSize,
        Prop::TextBold,
        Prop::Visible,
        Prop::Rotation,
        Prop::Mirrored,
        Prop::Locked,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Prop::TrackWidth => "Width",
            Prop::CopperLayer => "Layer",
            Prop::Locked => "Locked",
            Prop::ViaDiameter => "Via diameter",
            Prop::ViaDrill => "Via drill",
            Prop::TextSize => "Text size",
            Prop::TextBold => "Bold",
            Prop::Visible => "Visible",
            Prop::Rotation => "Rotation",
            Prop::Mirrored => "Mirrored",
        }
    }
}

/// A property's value.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Value {
    Length(Coord),
    Layer(LayerId),
    Flag(bool),
    Angle(Angle),
}

/// The property `prop` of `object`, if it has it.
fn get(object: &Object, prop: Prop) -> Option<Value> {
    use Object as O;
    Some(match (prop, object) {
        (Prop::TrackWidth, O::Track(t)) => Value::Length(t.width),
        (Prop::CopperLayer, O::Track(t)) => Value::Layer(t.layer),
        (Prop::CopperLayer, O::Zone(z)) => Value::Layer(z.layer),
        (Prop::Locked, O::Track(t)) => Value::Flag(t.locked),
        (Prop::Locked, O::Via(v)) => Value::Flag(v.locked),
        (Prop::Locked, O::Footprint(f)) => Value::Flag(f.locked),
        (Prop::ViaDiameter, O::Via(v)) => Value::Length(v.diameter),
        (Prop::ViaDrill, O::Via(v)) => Value::Length(v.drill),
        (Prop::TextSize | Prop::TextBold, _) => {
            let text = match object {
                O::SchText(t) => &t.text,
                O::BoardText(t) => &t.text,
                O::SymbolField(f) => &f.text,
                O::FootprintField(f) => &f.text,
                _ => return None,
            };
            if prop == Prop::TextSize { Value::Length(text.size) } else { Value::Flag(text.bold) }
        }
        (Prop::Visible, O::SymbolField(f)) => Value::Flag(!f.hidden),
        (Prop::Visible, O::FootprintField(f)) => Value::Flag(!f.hidden),
        (Prop::Rotation, O::Symbol(s)) => Value::Angle(s.rotation),
        (Prop::Rotation, O::Footprint(f)) => Value::Angle(f.rotation),
        (Prop::Mirrored, O::Symbol(s)) => Value::Flag(s.mirror),
        _ => return None,
    })
}

/// `object` with `prop` set to `value` (unchanged when it hasn't the property or the
/// value is of another kind). Via sizes keep the drill 0.05 mm inside the diameter.
fn set(object: &mut Object, prop: Prop, value: Value) {
    use Object as O;
    const RING: Coord = 50_000;
    match (prop, object, value) {
        (Prop::TrackWidth, O::Track(t), Value::Length(w)) => t.width = w,
        (Prop::CopperLayer, O::Track(t), Value::Layer(l)) => t.layer = l,
        (Prop::CopperLayer, O::Zone(z), Value::Layer(l)) => z.layer = l,
        (Prop::Locked, O::Track(t), Value::Flag(b)) => t.locked = b,
        (Prop::Locked, O::Via(v), Value::Flag(b)) => v.locked = b,
        (Prop::Locked, O::Footprint(f), Value::Flag(b)) => f.locked = b,
        (Prop::ViaDiameter, O::Via(v), Value::Length(d)) => v.diameter = d.max(v.drill + RING),
        (Prop::ViaDrill, O::Via(v), Value::Length(d)) => v.drill = d.min(v.diameter - RING),
        (Prop::TextSize | Prop::TextBold, object, value) => {
            let text = match object {
                O::SchText(t) => &mut t.text,
                O::BoardText(t) => &mut t.text,
                O::SymbolField(f) => &mut f.text,
                O::FootprintField(f) => &mut f.text,
                _ => return,
            };
            match (prop, value) {
                (Prop::TextSize, Value::Length(s)) => text.size = s,
                (Prop::TextBold, Value::Flag(b)) => text.bold = b,
                _ => {}
            }
        }
        (Prop::Visible, O::SymbolField(f), Value::Flag(b)) => f.hidden = !b,
        (Prop::Visible, O::FootprintField(f), Value::Flag(b)) => f.hidden = !b,
        (Prop::Rotation, O::Symbol(s), Value::Angle(a)) => s.rotation = a,
        (Prop::Rotation, O::Footprint(f), Value::Angle(a)) => f.rotation = a,
        (Prop::Mirrored, O::Symbol(s), Value::Flag(b)) => s.mirror = b,
        _ => {}
    }
}

/// The selected objects that count: without reference and value texts whose symbol or
/// footprint is selected too.
fn objects(project: &Project, ids: &[ObjectId]) -> Vec<Object> {
    let owner_selected = |id: &ObjectId| match id {
        ObjectId::SymbolField(f) => {
            project.schematic.fields.get(*f).is_some_and(|f| ids.contains(&ObjectId::Symbol(f.symbol)))
        }
        ObjectId::FootprintField(f) => {
            project.board.fields.get(*f).is_some_and(|f| ids.contains(&ObjectId::Footprint(f.footprint)))
        }
        _ => false,
    };
    ids.iter().filter(|id| !owner_selected(id)).filter_map(|&id| project.get(id)).collect()
}

/// A property all the selected objects have.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Shared {
    pub prop: Prop,
    /// Their value when all agree (`None`: they differ).
    pub value: Option<Value>,
    /// The first object's value (where an edit of differing values starts).
    pub first: Value,
}

/// The properties every selected object has.
pub fn shared(project: &Project, ids: &[ObjectId]) -> Vec<Shared> {
    let objects = objects(project, ids);
    if objects.is_empty() {
        return Vec::new();
    }
    Prop::ALL
        .into_iter()
        .filter_map(|prop| {
            let values: Vec<Value> = objects.iter().map(|o| get(o, prop)).collect::<Option<_>>()?;
            let same = values.windows(2).all(|w| w[0] == w[1]);
            Some(Shared { prop, value: same.then_some(values[0]), first: values[0] })
        })
        .collect()
}

/// Whether a selected footprint is locked (its rotation can't change).
pub fn any_locked_footprint(project: &Project, ids: &[ObjectId]) -> bool {
    ids.iter()
        .any(|id| matches!(id, ObjectId::Footprint(f) if project.board.footprints.get(*f).is_some_and(|f| f.locked)))
}

/// Set `prop` to `value` on every selected object that has it, as one command (`None`
/// when nothing would change).
pub fn set_all(project: &Project, ids: &[ObjectId], prop: Prop, value: Value) -> Option<Command> {
    let objects: Vec<Object> = objects(project, ids)
        .into_iter()
        .filter_map(|mut o| {
            get(&o, prop)?;
            let before = o.clone();
            set(&mut o, prop, value);
            (o != before).then_some(o)
        })
        .collect();
    (!objects.is_empty()).then_some(Command::Update { objects })
}
