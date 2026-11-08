//! Editing one library item (a symbol or a footprint) on a canvas.
//!
//! * [`Elem`] identifies a pin, pad, graphic or text inside the item.
//! * [`apply_symbol`] / [`apply_footprint`] perform generic [`Edit`]s on item content.
//! * [`ItemView`] draws the item with per-element hit testing and highlights.
//! * [`ItemTarget`] connects both to [`Interaction`](crate::Interaction): every change is an
//!   `update` command on the library document, so undo works as everywhere else.

use editor_core::Command;
use model::*;
use render::{Color, Instance, Scene};
use rustc_hash::FxHasher;
use std::hash::{Hash, Hasher};

use crate::document::Document;
use crate::draw::{Painter, mmf};
use crate::index::SpatialIndex;
use crate::interaction::{Edit, Target};
use crate::style::*;

/// The library item being edited.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Item {
    Symbol(Id),
    Footprint(Id),
}

impl Item {
    pub fn object_id(self) -> ObjectId {
        match self {
            Item::Symbol(id) => ObjectId::LibSymbol(id),
            Item::Footprint(id) => ObjectId::LibFootprint(id),
        }
    }
}

/// An element inside a symbol or footprint. Graphics and texts are addressed by index.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Elem {
    Pin(Id),
    Pad(Id),
    Graphic(usize),
    Text(usize),
}

/// Current content of the edited item.
#[derive(Clone, Debug, PartialEq)]
pub enum Content {
    Symbol(Symbol),
    Footprint(Footprint),
}

impl Content {
    pub fn get(project: &Project, item: Item) -> Option<Content> {
        match item {
            Item::Symbol(id) => project.library.symbols.get(id).cloned().map(Content::Symbol),
            Item::Footprint(id) => project.library.footprints.get(id).cloned().map(Content::Footprint),
        }
    }

    pub fn into_object(self) -> Object {
        match self {
            Content::Symbol(s) => Object::LibSymbol(s),
            Content::Footprint(f) => Object::LibFootprint(f),
        }
    }

    pub fn exists(&self, e: Elem) -> bool {
        match (self, e) {
            (Content::Symbol(s), Elem::Pin(id)) => s.pins.contains(id),
            (Content::Symbol(s), Elem::Graphic(i)) => i < s.graphics.len(),
            (Content::Symbol(s), Elem::Text(i)) => i < s.texts.len(),
            (Content::Footprint(f), Elem::Pad(id)) => f.pads.contains(id),
            (Content::Footprint(f), Elem::Graphic(i)) => i < f.graphics.len(),
            (Content::Footprint(f), Elem::Text(i)) => i < f.texts.len(),
            _ => false,
        }
    }

    /// The point that snaps to the grid when the element is dragged.
    pub fn anchor(&self, e: Elem) -> Option<Point> {
        match (self, e) {
            (Content::Symbol(s), Elem::Pin(id)) => s.pins.get(id).map(|p| p.pos),
            (Content::Symbol(s), Elem::Graphic(i)) => s.graphics.get(i).map(shape_anchor),
            (Content::Symbol(s), Elem::Text(i)) => s.texts.get(i).map(|t| t.pos),
            (Content::Footprint(f), Elem::Pad(id)) => f.pads.get(id).map(|p| p.pos),
            (Content::Footprint(f), Elem::Graphic(i)) => f.graphics.get(i).map(|g| shape_anchor(&g.shape)),
            (Content::Footprint(f), Elem::Text(i)) => f.texts.get(i).map(|t| t.text.pos),
            _ => None,
        }
    }

    pub fn elements(&self) -> Vec<Elem> {
        match self {
            Content::Symbol(s) => s
                .pins
                .ids()
                .map(Elem::Pin)
                .chain((0..s.graphics.len()).map(Elem::Graphic))
                .chain((0..s.texts.len()).map(Elem::Text))
                .collect(),
            Content::Footprint(f) => f
                .pads
                .ids()
                .map(Elem::Pad)
                .chain((0..f.graphics.len()).map(Elem::Graphic))
                .chain((0..f.texts.len()).map(Elem::Text))
                .collect(),
        }
    }
}

pub fn shape_anchor(s: &Shape) -> Point {
    match s {
        Shape::Line { a, .. } | Shape::Rect { a, .. } => *a,
        Shape::Arc { center, .. } | Shape::Circle { center, .. } => *center,
        Shape::Polygon { points, .. } => points.first().copied().unwrap_or_default(),
    }
}

// ---------------------------------------------------------------------------------------
// Edits

fn edit_transform<I>(edit: &Edit<I>) -> Option<Transform> {
    match edit {
        Edit::Move { delta, .. } => Some(Transform::new(*delta, Angle::ZERO, false)),
        Edit::Rotate { center, angle, .. } => Some(Transform::new(*center - center.rotated(*angle), *angle, false)),
        Edit::Mirror { axis_x, .. } | Edit::Flip { axis_x, .. } => {
            Some(Transform::new(Point::new(2 * axis_x, 0), Angle::ZERO, true))
        }
        Edit::Delete { .. } => None,
    }
}

fn edit_ids<I: Copy>(edit: &Edit<I>) -> &[I] {
    match edit {
        Edit::Move { ids, .. }
        | Edit::Rotate { ids, .. }
        | Edit::Mirror { ids, .. }
        | Edit::Flip { ids, .. }
        | Edit::Delete { ids } => ids,
    }
}

/// Indices of graphics/texts to delete, highest first (so removal keeps the others valid).
fn indices(ids: &[Elem], f: fn(Elem) -> Option<usize>) -> Vec<usize> {
    let mut v: Vec<usize> = ids.iter().filter_map(|&e| f(e)).collect();
    v.sort_unstable_by(|a, b| b.cmp(a));
    v.dedup();
    v
}

fn graphic_index(e: Elem) -> Option<usize> {
    if let Elem::Graphic(i) = e { Some(i) } else { None }
}

fn text_index(e: Elem) -> Option<usize> {
    if let Elem::Text(i) = e { Some(i) } else { None }
}

pub fn apply_symbol(sym: &mut Symbol, edit: &Edit<Elem>) -> Result<(), String> {
    let ids = edit_ids(edit);
    let Some(t) = edit_transform(edit) else {
        for &e in ids {
            if let Elem::Pin(id) = e {
                sym.pins.remove(id);
            }
        }
        for i in indices(ids, graphic_index) {
            if i < sym.graphics.len() {
                sym.graphics.remove(i);
            }
        }
        for i in indices(ids, text_index) {
            if i < sym.texts.len() {
                sym.texts.remove(i);
            }
        }
        return Ok(());
    };
    for &e in ids {
        match e {
            Elem::Pin(id) => {
                let pin = sym.pins.get_mut(id).ok_or("pin not found")?;
                pin.pos = t.point(pin.pos);
                pin.direction = t.angle(pin.direction);
            }
            Elem::Graphic(i) => {
                let g = sym.graphics.get_mut(i).ok_or("graphic not found")?;
                *g = g.transformed(&t);
            }
            Elem::Text(i) => {
                let tx = sym.texts.get_mut(i).ok_or("text not found")?;
                *tx = tx.transformed(&t);
            }
            Elem::Pad(_) => return Err("symbols have no pads".into()),
        }
    }
    Ok(())
}

pub fn apply_footprint(fp: &mut Footprint, edit: &Edit<Elem>) -> Result<(), String> {
    let ids = edit_ids(edit);
    let Some(t) = edit_transform(edit) else {
        for &e in ids {
            if let Elem::Pad(id) = e {
                fp.pads.remove(id);
            }
        }
        for i in indices(ids, graphic_index) {
            if i < fp.graphics.len() {
                fp.graphics.remove(i);
            }
        }
        for i in indices(ids, text_index) {
            if i < fp.texts.len() {
                fp.texts.remove(i);
            }
        }
        return Ok(());
    };
    for &e in ids {
        match e {
            Elem::Pad(id) => {
                let pad = fp.pads.get_mut(id).ok_or("pad not found")?;
                pad.pos = t.point(pad.pos);
                let r = if t.mirror { Angle::deg(-pad.rotation.degrees()) } else { pad.rotation };
                pad.rotation = r.rotated_by(t.rotation);
                if t.mirror
                    && let PadShape::Polygon { points } = &mut pad.shape
                {
                    points.iter_mut().for_each(|p| p.x = -p.x);
                }
            }
            Elem::Graphic(i) => {
                let g = fp.graphics.get_mut(i).ok_or("graphic not found")?;
                g.shape = g.shape.transformed(&t);
            }
            Elem::Text(i) => {
                let tx = fp.texts.get_mut(i).ok_or("text not found")?;
                tx.text = tx.text.transformed(&t);
            }
            Elem::Pin(_) => return Err("footprints have no pins".into()),
        }
    }
    Ok(())
}

/// Apply `edit` to an item's content.
pub fn apply(content: &mut Content, edit: &Edit<Elem>) -> Result<(), String> {
    match content {
        Content::Symbol(s) => apply_symbol(s, edit),
        Content::Footprint(f) => apply_footprint(f, edit),
    }
}

/// Modify an item through an `update` command (grouped for drags).
pub fn update_item<R>(
    doc: &mut Document,
    item: Item,
    group: Option<u64>,
    f: impl FnOnce(&mut Content) -> Result<R, String>,
) -> Result<R, String> {
    let mut content = Content::get(doc.project(), item).ok_or("the item no longer exists")?;
    let result = f(&mut content)?;
    doc.execute_grouped(Command::Update { objects: vec![content.into_object()] }, group).map_err(|e| e.to_string())?;
    Ok(result)
}

/// Next free numeric name ("1", "2", …) given existing names.
pub fn next_number<'a>(names: impl Iterator<Item = &'a str>) -> String {
    let max = names.filter_map(|n| n.trim().parse::<u64>().ok()).max().unwrap_or(0);
    (max + 1).to_string()
}

// ---------------------------------------------------------------------------------------
// Drawing

type RLayer = render::LayerId;

/// Render layers (one fixed set for symbols and footprints; unused ones stay empty).
struct Layers {
    courtyard: RLayer,
    fab: RLayer,
    body_fill: RLayer,
    body: RLayer,
    silk: RLayer,
    copper: RLayer,
    holes: RLayer,
    pins: RLayer,
    names: RLayer,
    text: RLayer,
    origin: RLayer,
    preview: RLayer,
    hover: RLayer,
    selection: RLayer,
}

/// A not-yet-placed element shown under the cursor by a creation tool.
#[derive(Clone, Debug)]
pub enum Ghost {
    Pin(Pin),
    Pad(Pad),
    Shape(Shape),
    Text(Text),
}

pub const PREVIEW: Color = Color::rgba(0x4c, 0x9a, 0xff, 0xc0);
const ORIGIN: Color = Color::rgba(0x9a, 0xa0, 0xb0, 0x90);
const PAD_TEXT: Color = Color::rgba(0xff, 0xff, 0xff, 0xd0);
const NAME_DIM: Color = SCH_PIN_TEXT;

fn elem_key(e: Elem) -> u64 {
    let mut h = FxHasher::default();
    e.hash(&mut h);
    h.finish() & !(3 << 62)
}

fn overlay_key(e: Elem, hover: bool) -> u64 {
    elem_key(e) | if hover { 2 << 62 } else { 1 << 62 }
}

const ORIGIN_KEY: u64 = 3 << 62;
const PREVIEW_KEY: u64 = (3 << 62) | 1;

pub fn footprint_layer_color(layer: FootprintLayer) -> Color {
    board_layer_color(LayerId::for_footprint(layer, Side::Top))
}

/// Draws one symbol or footprint and answers hit tests on its elements.
pub struct ItemView {
    pub scene: Scene,
    layers: Layers,
    item: Option<Item>,
    content: Option<Content>,
    index: SpatialIndex<Elem>,
    /// Document revision drawn (`None`: needs drawing).
    drawn: Option<u64>,
    selection: Vec<Elem>,
    hover: Option<Elem>,
}

impl Default for ItemView {
    fn default() -> Self {
        Self::new()
    }
}

impl ItemView {
    pub fn new() -> Self {
        let mut scene = Scene::new();
        let layers = Layers {
            courtyard: scene.add_layer("Courtyard"),
            fab: scene.add_layer("Fabrication"),
            body_fill: scene.add_layer("Body fill"),
            body: scene.add_layer("Body"),
            silk: scene.add_layer("Silkscreen"),
            copper: scene.add_layer("Pads"),
            holes: scene.add_layer("Holes"),
            pins: scene.add_layer("Pins"),
            names: scene.add_layer("Pin and pad names"),
            text: scene.add_layer("Text"),
            origin: scene.add_layer("Origin"),
            preview: scene.add_layer("Preview"),
            hover: scene.add_layer("Hover"),
            selection: scene.add_layer("Selection"),
        };
        ItemView {
            scene,
            layers,
            item: None,
            content: None,
            index: SpatialIndex::default(),
            drawn: None,
            selection: Vec::new(),
            hover: None,
        }
    }

    pub fn item(&self) -> Option<Item> {
        self.item
    }

    pub fn content(&self) -> Option<&Content> {
        self.content.as_ref()
    }

    /// Switch to another item (or none).
    pub fn show(&mut self, item: Option<Item>) {
        if item != self.item {
            self.item = item;
            self.drawn = None;
            self.selection.clear();
            self.hover = None;
        }
    }

    /// Redraw if the document changed since the last call.
    pub fn sync(&mut self, doc: &Document) {
        if self.drawn == Some(doc.revision) {
            return;
        }
        self.drawn = Some(doc.revision);
        self.content = self.item.and_then(|item| Content::get(doc.project(), item));
        if self.content.is_none() {
            self.item = None;
        }
        self.redraw();
    }

    /// Bounds of everything drawn (for fitting the camera).
    pub fn bounds(&self) -> Option<Rect> {
        let r = self.index.total();
        // Always include the origin.
        Some(r.map_or(Rect::around(Point::ZERO, mm(5.0)), |r| r.union(Rect::around(Point::ZERO, mm(1.0)))))
    }

    fn redraw(&mut self) {
        self.scene.clear();
        self.index.clear();
        let l = &self.layers;
        // Origin cross.
        let s = 1.0;
        let origin = vec![
            (l.origin, Instance::line([-s, 0.0], [s, 0.0], 0.03, ORIGIN)),
            (l.origin, Instance::line([0.0, -s], [0.0, s], 0.03, ORIGIN)),
            (l.origin, Instance::circle_outline([0.0, 0.0], 0.3 * s, 0.03, ORIGIN)),
        ];
        self.scene.set_item(ORIGIN_KEY, origin);
        let Some(content) = &self.content else { return };
        for e in content.elements() {
            let mut p = Painter::default();
            draw_elem(&self.layers, content, e, &mut p, None);
            if let Some(b) = p.bounds {
                self.index.set(e, b);
            }
            self.scene.set_item(elem_key(e), p.out);
        }
        let sel = std::mem::take(&mut self.selection);
        let hover = self.hover.take();
        self.set_highlight(&sel, hover);
    }

    pub fn set_highlight(&mut self, selection: &[Elem], hover: Option<Elem>) {
        for &e in &self.selection {
            if !selection.contains(&e) {
                self.scene.remove_item(overlay_key(e, false));
            }
        }
        for &e in selection {
            if !self.selection.contains(&e) {
                self.draw_overlay(e, false);
            }
        }
        self.selection = selection.to_vec();
        if hover != self.hover {
            if let Some(h) = self.hover {
                self.scene.remove_item(overlay_key(h, true));
            }
            self.hover = hover;
            if let Some(h) = hover {
                self.draw_overlay(h, true);
            }
        }
    }

    fn draw_overlay(&mut self, e: Elem, hover: bool) {
        let Some(content) = &self.content else { return };
        if !content.exists(e) {
            return;
        }
        let (layer, color) = if hover { (self.layers.hover, HOVER) } else { (self.layers.selection, SELECTION) };
        let mut p = Painter::default();
        draw_elem(&self.layers, content, e, &mut p, Some(color));
        let out: Vec<_> = p.out.into_iter().map(|(_, i)| (layer, i)).collect();
        self.scene.set_item(overlay_key(e, hover), out);
    }

    /// Show creation-tool ghosts (empty slice clears).
    pub fn set_preview(&mut self, ghosts: &[Ghost]) {
        if ghosts.is_empty() {
            self.scene.remove_item(PREVIEW_KEY);
            return;
        }
        let l = &self.layers;
        let mut p = Painter::default();
        for g in ghosts {
            match g {
                Ghost::Pin(pin) => draw_pin(l, pin, &mut p, Some(PREVIEW)),
                Ghost::Pad(pad) => draw_pad(l, pad, &mut p, Some(PREVIEW)),
                Ghost::Shape(s) => p.shape(l.preview, s, PREVIEW, None),
                Ghost::Text(t) => p.text(l.preview, t, PREVIEW, &|s| s.to_string(), false),
            }
        }
        let out: Vec<_> = p.out.into_iter().map(|(_, i)| (l.preview, i)).collect();
        self.scene.set_item(PREVIEW_KEY, out);
    }

    // -------------------------------------------------------------------------------------
    // Hit testing

    pub fn hit(&self, p: Point, tol: Coord) -> Option<Elem> {
        let content = self.content.as_ref()?;
        let mut best: Option<((u8, f64), Elem)> = None;
        for e in self.index.near(p, tol) {
            let Some(b) = self.index.bounds(e) else { continue };
            let inside = Rect { min: b.min - Point::new(tol, tol), max: b.max + Point::new(tol, tol) }.contains(p);
            let area = b.width() as f64 * b.height() as f64;
            let score = match (content, e) {
                (Content::Symbol(s), Elem::Pin(id)) => s
                    .pins
                    .get(id)
                    .map(|pin| seg_dist(p, pin.pos, pin.inner_end()))
                    .filter(|&d| d <= tol as f64 + WIRE_WIDTH as f64)
                    .map(|d| (0, d)),
                (Content::Footprint(_), Elem::Pad(_)) => inside.then_some((0, area)),
                (Content::Symbol(s), Elem::Graphic(i)) => {
                    s.graphics.get(i).and_then(|g| shape_score(g, p, tol, inside, area))
                }
                (Content::Footprint(f), Elem::Graphic(i)) => {
                    f.graphics.get(i).and_then(|g| shape_score(&g.shape, p, tol, inside, area))
                }
                _ => inside.then_some((2, area)),
            };
            if let Some(s) = score
                && best.as_ref().is_none_or(|(bs, _)| s < *bs)
            {
                best = Some((s, e));
            }
        }
        best.map(|(_, e)| e)
    }

    pub fn in_rect(&self, r: Rect, crossing: bool) -> Vec<Elem> {
        self.index.in_rect(r, crossing)
    }

    pub fn elem_bounds(&self, e: Elem) -> Option<Rect> {
        self.index.bounds(e)
    }

    pub fn elements(&self) -> Vec<Elem> {
        self.content.as_ref().map(Content::elements).unwrap_or_default()
    }
}

/// Outline hits (class 1, by distance) beat area hits (class 2, by size).
fn shape_score(s: &Shape, p: Point, tol: Coord, inside: bool, area: f64) -> Option<(u8, f64)> {
    let tol = tol as f64;
    let d = match s {
        Shape::Line { a, b, width } => Some(seg_dist(p, *a, *b) - *width as f64 / 2.0),
        Shape::Rect { a, b, width, fill: false } => {
            let r = Rect::from_points(*a, *b);
            let c = [r.min, Point::new(r.max.x, r.min.y), r.max, Point::new(r.min.x, r.max.y)];
            Some((0..4).map(|i| seg_dist(p, c[i], c[(i + 1) % 4])).fold(f64::MAX, f64::min) - *width as f64 / 2.0)
        }
        Shape::Circle { center, radius, width, fill: false } | Shape::Arc { center, radius, width, .. } => {
            Some(((p - *center).length() - *radius as f64).abs() - *width as f64 / 2.0)
        }
        Shape::Polygon { points, width, fill: false } if points.len() > 1 => Some(
            (0..points.len()).map(|i| seg_dist(p, points[i], points[(i + 1) % points.len()])).fold(f64::MAX, f64::min)
                - *width as f64 / 2.0,
        ),
        _ => None,
    };
    match d {
        Some(d) if d <= tol => Some((1, d)),
        Some(_) => None,
        None => inside.then_some((2, area)),
    }
}

fn seg_dist(p: Point, a: Point, b: Point) -> f64 {
    let (ax, ay, bx, by, px, py) = (a.x as f64, a.y as f64, b.x as f64, b.y as f64, p.x as f64, p.y as f64);
    let (dx, dy) = (bx - ax, by - ay);
    let len2 = dx * dx + dy * dy;
    let t = if len2 == 0.0 { 0.0 } else { (((px - ax) * dx + (py - ay) * dy) / len2).clamp(0.0, 1.0) };
    ((ax + t * dx - px).powi(2) + (ay + t * dy - py).powi(2)).sqrt()
}

fn placeholder(s: &str) -> String {
    if s.contains("${") { s.replace("${REF}", "REF").replace("${VALUE}", "VALUE") } else { s.to_string() }
}

/// Draw one element. With `color`, everything is drawn in that color (overlays).
fn draw_elem(l: &Layers, content: &Content, e: Elem, p: &mut Painter, color: Option<Color>) {
    let c = |default: Color| color.unwrap_or(default);
    match (content, e) {
        (Content::Symbol(s), Elem::Pin(id)) => {
            if let Some(pin) = s.pins.get(id) {
                draw_pin(l, pin, p, color);
            }
        }
        (Content::Symbol(s), Elem::Graphic(i)) => {
            if let Some(g) = s.graphics.get(i) {
                let fill = if color.is_some() { None } else { Some((l.body_fill, SCH_BODY_FILL)) };
                p.shape(l.body, g, c(SCH_BODY), fill);
            }
        }
        (Content::Symbol(s), Elem::Text(i)) => {
            if let Some(t) = s.texts.get(i) {
                p.text(l.text, t, c(SCH_TEXT), &placeholder, true);
            }
        }
        (Content::Footprint(f), Elem::Pad(id)) => {
            if let Some(pad) = f.pads.get(id) {
                draw_pad(l, pad, p, color);
            }
        }
        (Content::Footprint(f), Elem::Graphic(i)) => {
            if let Some(g) = f.graphics.get(i) {
                let layer = match g.layer {
                    FootprintLayer::Courtyard => l.courtyard,
                    FootprintLayer::Fabrication => l.fab,
                    FootprintLayer::Copper => l.copper,
                    _ => l.silk,
                };
                p.shape(layer, &g.shape, c(footprint_layer_color(g.layer)), None);
            }
        }
        (Content::Footprint(f), Elem::Text(i)) => {
            if let Some(t) = f.texts.get(i) {
                p.text(l.text, &t.text, c(footprint_layer_color(t.layer)), &placeholder, true);
            }
        }
        _ => {}
    }
}

fn draw_pin(l: &Layers, pin: &Pin, p: &mut Painter, color: Option<Color>) {
    let (a, b) = (pin.pos, pin.inner_end());
    p.line(l.pins, a, b, WIRE_WIDTH, color.unwrap_or(SCH_PIN));
    // Connection point marker.
    p.push(l.pins, Instance::circle_outline(mmf(a), 0.35, 0.05, color.unwrap_or(SCH_PIN)));
    let d = b - a;
    let horizontal = d.x.abs() >= d.y.abs();
    let rotation = if horizontal { Angle::ZERO } else { Angle::DEG90 };
    if pin.show_name {
        let inward = if horizontal { d.x > 0 } else { d.y > 0 };
        let step =
            if horizontal { Point::new(d.x.signum() * 500_000, 0) } else { Point::new(0, d.y.signum() * 500_000) };
        let mut t = Text::new(pin.name.clone(), b + step, 1_270_000)
            .aligned(if inward { HAlign::Left } else { HAlign::Right }, VAlign::Center);
        t.rotation = rotation;
        p.text(l.names, &t, color.unwrap_or(SCH_BODY), &|s| s.to_string(), false);
    }
    // Along the pin: the number, or the hidden name (dimmed) so it can be identified.
    let along =
        if !pin.number.is_empty() { Some(pin.number.clone()) } else { (!pin.show_name).then(|| pin.name.clone()) };
    if let Some(text) = along {
        let mid = Point::new((a.x + b.x) / 2, (a.y + b.y) / 2);
        let pos = if horizontal { mid + Point::new(0, 250_000) } else { mid - Point::new(250_000, 0) };
        let mut t = Text::new(text, pos, 1_000_000).aligned(HAlign::Center, VAlign::Bottom);
        t.rotation = rotation;
        p.text(l.names, &t, color.unwrap_or(NAME_DIM), &|s| s.to_string(), false);
    }
}

fn draw_pad(l: &Layers, pad: &Pad, p: &mut Painter, color: Option<Color>) {
    let copper = color.unwrap_or(board_layer_color(LayerId::TOP_COPPER));
    let c = mmf(pad.pos);
    let rot = pad.rotation.radians() as f32;
    let (w, h) = ((pad.size.w as f64 * 1e-6) as f32, (pad.size.h as f64 * 1e-6) as f32);
    let has_copper = !matches!(pad.kind, PadKind::NonPlatedHole { .. });
    if has_copper {
        match &pad.shape {
            PadShape::Circle => p.push(l.copper, Instance::circle(c, w * 0.5, copper)),
            PadShape::Rect => p.push(l.copper, Instance::rect(c, [w, h], 0.0, rot, copper)),
            PadShape::RoundRect { ratio } => {
                p.push(l.copper, Instance::rect(c, [w, h], *ratio as f32 * w.min(h), rot, copper))
            }
            PadShape::Oval => p.push(l.copper, Instance::rect(c, [w, h], 0.5 * w.min(h), rot, copper)),
            PadShape::Polygon { points } => {
                p.fill(l.copper, points.iter().map(|q| pad.pos + q.rotated(pad.rotation)), copper)
            }
        }
    }
    if let PadKind::ThroughHole { drill } | PadKind::NonPlatedHole { drill } = pad.kind {
        let hole = if color.is_some() && !has_copper { copper } else { HOLE };
        p.push(l.holes, Instance::circle(c, (drill as f64 * 0.5e-6) as f32, hole));
        if !has_copper {
            p.push(l.holes, Instance::circle_outline(c, (drill as f64 * 0.5e-6) as f32, 0.05, copper));
        }
    }
    // Bounds of the rotated pad.
    let r = pad.size.w.max(pad.size.h) / 2;
    let half = if pad.rotation.quarter_turns().is_some_and(|q| q % 2 == 1) {
        Point::new(pad.size.h / 2, pad.size.w / 2)
    } else if pad.rotation.quarter_turns().is_some() {
        Point::new(pad.size.w / 2, pad.size.h / 2)
    } else {
        Point::new(r, r)
    };
    p.grow(Rect { min: pad.pos - half, max: pad.pos + half });
    if color.is_none() {
        let size = (pad.size.w.min(pad.size.h) as f64 * 0.5) as Coord;
        let t = Text::new(pad.name.clone(), pad.pos, size).aligned(HAlign::Center, VAlign::Center);
        p.text(l.names, &t, PAD_TEXT, &|s| s.to_string(), false);
    }
}

// ---------------------------------------------------------------------------------------
// Interaction target

/// The edited item of a library document, as an interaction [`Target`].
pub struct ItemTarget<'a> {
    pub doc: &'a mut Document,
    pub view: &'a mut ItemView,
}

impl Target for ItemTarget<'_> {
    type Id = Elem;

    fn hit(&self, p: Point, tol: Coord) -> Option<Elem> {
        self.view.hit(p, tol)
    }

    fn in_rect(&self, r: Rect, crossing: bool) -> Vec<Elem> {
        self.view.in_rect(r, crossing)
    }

    fn all(&self) -> Vec<Elem> {
        self.view.elements()
    }

    fn bounds(&self, e: Elem) -> Option<Rect> {
        self.view.elem_bounds(e)
    }

    fn exists(&self, e: Elem) -> bool {
        self.view.content().is_some_and(|c| c.exists(e))
    }

    fn anchor(&self, e: Elem) -> Option<Point> {
        self.view.content()?.anchor(e)
    }

    fn edit(&mut self, edit: Edit<Elem>, group: Option<u64>) -> Result<Option<String>, String> {
        let item = self.view.item().ok_or("nothing is being edited")?;
        update_item(self.doc, item, group, |c| apply(c, &edit))?;
        self.view.sync(self.doc);
        Ok(None)
    }

    fn close_group(&mut self) {
        self.doc.close_group();
    }

    fn undo(&mut self) {
        self.doc.undo();
        self.view.sync(self.doc);
    }

    fn highlight(&mut self, selection: &[Elem], hover: Option<Elem>) {
        self.view.sync(self.doc);
        self.view.set_highlight(selection, hover);
    }
}
