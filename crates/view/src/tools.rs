//! Creation tools of the symbol and footprint editors: place pins, pads, lines,
//! rectangles, circles and texts with a ghost preview under the cursor.

use model::*;

use crate::document::Document;
use crate::item::{Content, Elem, Ghost, Item, ItemView, update_item};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Tool {
    Select,
    Pin,
    Pad,
    Line,
    Rect,
    Circle,
    Text,
}

impl Tool {
    pub fn label(self) -> &'static str {
        match self {
            Tool::Select => "Select",
            Tool::Pin => "Pin",
            Tool::Pad => "Pad",
            Tool::Line => "Line",
            Tool::Rect => "Rectangle",
            Tool::Circle => "Circle",
            Tool::Text => "Text",
        }
    }

    /// Tools available for an item kind.
    pub fn for_item(item: Item) -> &'static [Tool] {
        match item {
            Item::Symbol(_) => &[Tool::Select, Tool::Pin, Tool::Line, Tool::Rect, Tool::Circle, Tool::Text],
            Item::Footprint(_) => &[Tool::Select, Tool::Pad, Tool::Line, Tool::Rect, Tool::Circle, Tool::Text],
        }
    }
}

/// Properties of newly placed elements (editable in the tool's side panel).
#[derive(Clone, Debug)]
pub struct Settings {
    /// Rotation of pins (direction toward the body), pads and texts.
    pub rotation: Angle,
    /// Name of the next pin or pad; auto-incremented after each placement.
    pub name: String,
    pub pin_length: Coord,
    pub pin_kind: PinKind,
    pub pad_size: Size,
    pub pad_shape: PadShape,
    pub pad_kind: PadKind,
    /// Footprint layer for graphics and texts.
    pub layer: FootprintLayer,
    pub symbol_width: Coord,
    pub footprint_width: Coord,
    pub fill: bool,
    pub text: String,
    pub text_size: Coord,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            rotation: Angle::ZERO,
            name: "1".into(),
            pin_length: 2_540_000,
            pin_kind: PinKind::Passive,
            pad_size: Size::mm(1.5, 1.5),
            pad_shape: PadShape::RoundRect { ratio: 0.25 },
            pad_kind: PadKind::Smd,
            layer: FootprintLayer::Silkscreen,
            symbol_width: 254_000,
            footprint_width: 150_000,
            fill: false,
            text: "${REF}".into(),
            text_size: 1_000_000,
        }
    }
}

/// "3" → "4", "A9" → "A10", "007" → "008", "GND" → "GND".
pub fn increment_name(name: &str) -> String {
    let digits = name.chars().rev().take_while(|c| c.is_ascii_digit()).count();
    if digits == 0 {
        return name.to_string();
    }
    let (prefix, number) = name.split_at(name.len() - digits);
    match number.parse::<u64>() {
        Ok(n) => format!("{prefix}{:0digits$}", n + 1),
        Err(_) => name.to_string(),
    }
}

/// Active creation tool and its in-progress geometry.
#[derive(Clone, Debug)]
pub struct Placer {
    pub tool: Tool,
    pub settings: Settings,
    /// Points clicked so far (line chains, rectangle corner, circle center).
    points: Vec<Point>,
    cursor: Option<Point>,
}

impl Default for Placer {
    fn default() -> Self {
        Placer { tool: Tool::Select, settings: Settings::default(), points: Vec::new(), cursor: None }
    }
}

impl Placer {
    pub fn is_placing(&self) -> bool {
        self.tool != Tool::Select
    }

    /// A shape is half-done (between clicks).
    pub fn in_progress(&self) -> bool {
        !self.points.is_empty()
    }

    /// Switch tools; picks the next free pin/pad number from the item.
    pub fn set_tool(&mut self, tool: Tool, content: Option<&Content>) {
        self.tool = tool;
        self.points.clear();
        if let Some(c) = content
            && matches!(tool, Tool::Pin | Tool::Pad)
        {
            self.settings.name = next_name(c);
            self.settings.rotation = Angle::ZERO;
        }
    }

    pub fn rotate(&mut self) {
        self.settings.rotation = self.settings.rotation.rotated_by(Angle::DEG90);
    }

    /// Escape: abort the current shape, or leave the tool. Returns true if the tool was left.
    pub fn escape(&mut self) -> bool {
        if self.points.is_empty() {
            self.tool = Tool::Select;
            true
        } else {
            self.points.clear();
            false
        }
    }

    /// Track the (snapped) cursor.
    pub fn hover(&mut self, p: Option<Point>) {
        self.cursor = p;
    }

    /// A click at the snapped point `p`. Returns the element created, if any.
    pub fn click(&mut self, doc: &mut Document, view: &mut ItemView, p: Point) -> Result<Option<Elem>, String> {
        let Some(item) = view.item() else { return Ok(None) };
        self.cursor = Some(p);
        let s = &self.settings;
        let width = self.width(item);
        let created = match self.tool {
            Tool::Select => None,
            Tool::Pin | Tool::Pad | Tool::Text => {
                let ghost = self.ghost_at(item, p).expect("single-click tools have a ghost");
                Some(add(doc, item, ghost, s.layer)?)
            }
            Tool::Line => {
                if let Some(&prev) = self.points.last() {
                    if prev == p {
                        // Clicking the last point again ends the chain.
                        self.points.clear();
                        return Ok(None);
                    }
                    self.points = vec![p];
                    Some(add(doc, item, Ghost::Shape(Shape::line(prev, p, width)), s.layer)?)
                } else {
                    self.points.push(p);
                    None
                }
            }
            Tool::Rect | Tool::Circle => {
                if let Some(&first) = self.points.first() {
                    self.points.clear();
                    let shape = self.shape(first, p, width);
                    if shape.bounds().width() == 0 && shape.bounds().height() == 0 {
                        return Ok(None);
                    }
                    Some(add(doc, item, Ghost::Shape(shape), s.layer)?)
                } else {
                    self.points.push(p);
                    None
                }
            }
        };
        if created.is_some() && matches!(self.tool, Tool::Pin | Tool::Pad) {
            self.settings.name = increment_name(&self.settings.name);
        }
        view.sync(doc);
        Ok(created)
    }

    fn width(&self, item: Item) -> Coord {
        match item {
            Item::Symbol(_) => self.settings.symbol_width,
            Item::Footprint(_) => self.settings.footprint_width,
        }
    }

    fn shape(&self, a: Point, b: Point, width: Coord) -> Shape {
        let fill = self.settings.fill;
        match self.tool {
            Tool::Circle => Shape::Circle { center: a, radius: (b - a).length().round() as Coord, width, fill },
            Tool::Line => Shape::line(a, b, width),
            _ => Shape::Rect { a, b, width, fill },
        }
    }

    fn ghost_at(&self, item: Item, p: Point) -> Option<Ghost> {
        let s = &self.settings;
        Some(match self.tool {
            Tool::Pin => Ghost::Pin(Pin {
                id: Id::new(),
                name: s.name.clone(),
                pos: p,
                direction: s.rotation,
                length: s.pin_length,
                kind: s.pin_kind,
                show_name: true,
                show_number: true,
            }),
            Tool::Pad => Ghost::Pad(Pad {
                id: Id::new(),
                name: s.name.clone(),
                pos: p,
                rotation: s.rotation,
                size: s.pad_size,
                shape: s.pad_shape.clone(),
                kind: s.pad_kind.clone(),
            }),
            Tool::Text => {
                let mut t = Text::new(s.text.clone(), p, s.text_size);
                t.rotation = s.rotation;
                Ghost::Text(t)
            }
            Tool::Line | Tool::Rect | Tool::Circle => {
                let &a = self.points.first()?;
                Ghost::Shape(self.shape(*self.points.last().unwrap_or(&a), p, self.width(item)))
            }
            Tool::Select => return None,
        })
    }

    /// What to preview at the cursor.
    pub fn ghosts(&self, item: Option<Item>) -> Vec<Ghost> {
        match (item, self.cursor) {
            (Some(item), Some(p)) if self.is_placing() => self.ghost_at(item, p).into_iter().collect(),
            _ => Vec::new(),
        }
    }
}

fn next_name(c: &Content) -> String {
    match c {
        Content::Symbol(s) => crate::item::next_number(s.pins.iter().map(|p| p.name.as_str())),
        Content::Footprint(f) => crate::item::next_number(f.pads.iter().map(|p| p.name.as_str())),
    }
}

/// Add a new element to the item (one undo step).
pub fn add(doc: &mut Document, item: Item, ghost: Ghost, layer: FootprintLayer) -> Result<Elem, String> {
    update_item(doc, item, None, |c| {
        Ok(match (c, ghost) {
            (Content::Symbol(s), Ghost::Pin(pin)) => {
                let id = pin.id;
                s.pins.insert(pin);
                Elem::Pin(id)
            }
            (Content::Symbol(s), Ghost::Shape(shape)) => {
                s.graphics.push(shape);
                Elem::Graphic(s.graphics.len() - 1)
            }
            (Content::Symbol(s), Ghost::Text(t)) => {
                s.texts.push(t);
                Elem::Text(s.texts.len() - 1)
            }
            (Content::Footprint(f), Ghost::Pad(pad)) => {
                let id = pad.id;
                f.pads.insert(pad);
                Elem::Pad(id)
            }
            (Content::Footprint(f), Ghost::Shape(shape)) => {
                f.graphics.push(FootprintGraphic { layer, shape });
                Elem::Graphic(f.graphics.len() - 1)
            }
            (Content::Footprint(f), Ghost::Text(text)) => {
                f.texts.push(FootprintText { layer, text });
                Elem::Text(f.texts.len() - 1)
            }
            (Content::Symbol(_), Ghost::Pad(_)) => return Err("symbols have pins, not pads".into()),
            (Content::Footprint(_), Ghost::Pin(_)) => return Err("footprints have pads, not pins".into()),
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_increment() {
        assert_eq!(increment_name("1"), "2");
        assert_eq!(increment_name("A9"), "A10");
        assert_eq!(increment_name("GND"), "GND");
        assert_eq!(increment_name("09"), "10");
        assert_eq!(increment_name("007"), "008");
    }
}
