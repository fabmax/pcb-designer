//! Editing a text in place on the canvas: in the stroke font, with the text's own
//! rotation, size and alignment; a caret and a selection. Independent of the GUI toolkit:
//! the app feeds keys, typed text and pointer positions, and draws [`TextEdit::geometry`].

use editor_core::Command;
use model::*;
use render::font::{self, HAlign as FH, VAlign as FV};

use crate::draw::Painter;
use crate::item::{Content, Item};
use crate::style::pal;
use crate::view::ViewId;

/// Where a text is edited: in a schematic or board view, or in a symbol or footprint
/// editor (which draws the edit itself, see [`crate::Document::item_text_edit`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditSite {
    View(ViewId),
    Item(Item),
}

/// What a text edit changes when committed.
#[derive(Clone, Debug, PartialEq)]
pub enum TextTarget {
    SchText(Id),
    BoardText(Id),
    Label(Id),
    Sheet(Id),
    /// A component's reference (edited at a symbol or footprint text).
    Reference(Id),
    /// A component's value.
    Value(Id),
    /// A text being placed: inserted on commit (unless empty).
    NewSchText(SchText),
    NewBoardText(BoardText),
    /// Text `index` of a symbol or footprint (in the library editors). Removed if emptied.
    ItemText {
        item: Item,
        index: usize,
    },
}

impl TextTarget {
    /// The object whose drawing the edit covers.
    pub fn object(&self) -> Option<ObjectId> {
        Some(match self {
            TextTarget::SchText(i) => ObjectId::SchText(*i),
            TextTarget::BoardText(i) => ObjectId::BoardText(*i),
            TextTarget::Label(i) => ObjectId::Label(*i),
            TextTarget::Sheet(i) => ObjectId::Sheet(*i),
            _ => return None,
        })
    }

    fn multiline(&self) -> bool {
        matches!(
            self,
            TextTarget::SchText(_)
                | TextTarget::BoardText(_)
                | TextTarget::NewSchText(_)
                | TextTarget::NewBoardText(_)
                | TextTarget::ItemText { .. }
        )
    }
}

/// Editing keys. Modifiers: `shift` extends the selection, `word` (Alt) moves by words,
/// `line` (Cmd) to the line's ends.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditKey {
    Left,
    Right,
    Up,
    Down,
    Home,
    End,
    Backspace,
    Delete,
    Enter,
    Escape,
    SelectAll,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Mods {
    pub shift: bool,
    pub word: bool,
    pub line: bool,
}

/// What a key did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Editing,
    Commit,
    Cancel,
}

/// Where to draw the edit: everything in world coordinates.
#[derive(Clone, Debug, PartialEq)]
pub struct EditGeometry {
    /// The text as it reads now (position, size, rotation and alignment of the original).
    pub text: Text,
    /// The box behind it (four corners).
    pub frame: [Point; 4],
    /// Selected spans (four corners each).
    pub selection: Vec<[Point; 4]>,
    /// The caret: a segment.
    pub caret: (Point, Point),
}

#[derive(Clone, Debug, PartialEq)]
pub struct TextEdit {
    pub target: TextTarget,
    /// The text being edited.
    pub text: String,
    /// How it is drawn: position, size, rotation, alignment (its `text` is unused).
    pub shape: Text,
    caret: usize,
    anchor: usize,
    original: String,
}

impl TextEdit {
    /// Edit `text` for `target`, drawn like `shape`; all of it selected.
    pub fn new(target: TextTarget, text: String, shape: Text) -> TextEdit {
        let n = text.chars().count();
        TextEdit { target, original: text.clone(), text, shape, caret: n, anchor: 0 }
    }

    pub fn is_multiline(&self) -> bool {
        self.target.multiline()
    }

    /// Caret and selection anchor (character indices).
    pub fn cursor(&self) -> (usize, usize) {
        (self.caret, self.anchor)
    }

    fn len(&self) -> usize {
        self.text.chars().count()
    }

    fn byte(&self, i: usize) -> usize {
        self.text.char_indices().nth(i).map_or(self.text.len(), |(b, _)| b)
    }

    fn selection(&self) -> (usize, usize) {
        (self.caret.min(self.anchor), self.caret.max(self.anchor))
    }

    pub fn selected_text(&self) -> String {
        let (a, b) = self.selection();
        self.text[self.byte(a)..self.byte(b)].to_string()
    }

    /// Replace the selection with `s` (newlines become spaces in single-line texts).
    pub fn insert(&mut self, s: &str) {
        let s = if self.is_multiline() { s.replace("\r\n", "\n") } else { s.replace(['\r', '\n'], " ") };
        let (a, b) = self.selection();
        let (ba, bb) = (self.byte(a), self.byte(b));
        self.text.replace_range(ba..bb, &s);
        self.caret = a + s.chars().count();
        self.anchor = self.caret;
    }

    /// Cut: the selected text, removed.
    pub fn cut(&mut self) -> Option<String> {
        let s = self.selected_text();
        if s.is_empty() {
            return None;
        }
        self.insert("");
        Some(s)
    }

    fn chars(&self) -> Vec<char> {
        self.text.chars().collect()
    }

    /// Line of character index `i` and the index where that line starts.
    fn line_of(&self, i: usize) -> (usize, usize) {
        let chars = self.chars();
        let mut line = 0;
        let mut start = 0;
        for (k, c) in chars.iter().enumerate().take(i) {
            if *c == '\n' {
                line += 1;
                start = k + 1;
            }
        }
        (line, start)
    }

    fn line_end(&self, i: usize) -> usize {
        let chars = self.chars();
        (i..chars.len()).find(|&k| chars[k] == '\n').unwrap_or(chars.len())
    }

    fn word_left(&self, i: usize) -> usize {
        let c = self.chars();
        let mut k = i;
        while k > 0 && !c[k - 1].is_alphanumeric() {
            k -= 1;
        }
        while k > 0 && c[k - 1].is_alphanumeric() {
            k -= 1;
        }
        k
    }

    fn word_right(&self, i: usize) -> usize {
        let c = self.chars();
        let mut k = i;
        while k < c.len() && !c[k].is_alphanumeric() {
            k += 1;
        }
        while k < c.len() && c[k].is_alphanumeric() {
            k += 1;
        }
        k
    }

    fn move_to(&mut self, i: usize, extend: bool) {
        self.caret = i.min(self.len());
        if !extend {
            self.anchor = self.caret;
        }
    }

    /// The caret one line up or down, at about the same x.
    fn vertical(&self, down: bool) -> usize {
        let (line, start) = self.line_of(self.caret);
        let lines: Vec<&str> = self.text.split('\n').collect();
        let target = if down { line + 1 } else { line.wrapping_sub(1) };
        if target >= lines.len() {
            return if down { self.len() } else { 0 };
        }
        let h = self.height();
        let x = font::caret_positions(lines[line], h)[self.caret - start];
        let origins = self.origins();
        let x = x + origins[line][0] - origins[target][0];
        let col = nearest(&font::caret_positions(lines[target], h), x);
        let target_start: usize = lines[..target].iter().map(|l| l.chars().count() + 1).sum();
        target_start + col
    }

    pub fn key(&mut self, key: EditKey, m: Mods) -> Outcome {
        let (a, b) = self.selection();
        match key {
            EditKey::Escape => return Outcome::Cancel,
            EditKey::Enter if m.shift && self.is_multiline() => self.insert("\n"),
            EditKey::Enter => return Outcome::Commit,
            EditKey::SelectAll => {
                self.anchor = 0;
                self.caret = self.len();
            }
            EditKey::Left => {
                let to = if m.line {
                    self.line_of(self.caret).1
                } else if m.word {
                    self.word_left(self.caret)
                } else if a != b && !m.shift {
                    a
                } else {
                    self.caret.saturating_sub(1)
                };
                self.move_to(to, m.shift);
            }
            EditKey::Right => {
                let to = if m.line {
                    self.line_end(self.caret)
                } else if m.word {
                    self.word_right(self.caret)
                } else if a != b && !m.shift {
                    b
                } else {
                    self.caret + 1
                };
                self.move_to(to, m.shift);
            }
            EditKey::Home => self.move_to(self.line_of(self.caret).1, m.shift),
            EditKey::End => self.move_to(self.line_end(self.caret), m.shift),
            EditKey::Up => self.move_to(self.vertical(false), m.shift),
            EditKey::Down => self.move_to(self.vertical(true), m.shift),
            EditKey::Backspace | EditKey::Delete => {
                if a == b {
                    let back = key == EditKey::Backspace;
                    let to = match (back, m.line, m.word) {
                        (true, true, _) => self.line_of(self.caret).1,
                        (true, _, true) => self.word_left(self.caret),
                        (true, _, _) => self.caret.saturating_sub(1),
                        (false, true, _) => self.line_end(self.caret),
                        (false, _, true) => self.word_right(self.caret),
                        (false, _, _) => (self.caret + 1).min(self.len()),
                    };
                    self.anchor = to;
                }
                self.insert("");
            }
        }
        Outcome::Editing
    }

    fn height(&self) -> f32 {
        (self.shape.size as f64 * 1e-6) as f32
    }

    fn aligns(&self) -> (FH, FV) {
        let h = match self.shape.halign {
            HAlign::Left => FH::Left,
            HAlign::Center => FH::Center,
            HAlign::Right => FH::Right,
        };
        let v = match self.shape.valign {
            VAlign::Bottom => FV::Bottom,
            VAlign::Center => FV::Center,
            VAlign::Top => FV::Top,
        };
        (h, v)
    }

    fn origins(&self) -> Vec<[f32; 2]> {
        let (h, v) = self.aligns();
        font::line_origins(&self.text, self.height(), h, v)
    }

    /// Local (unrotated, mm, relative to the anchor) → world.
    fn world(&self, x: f32, y: f32) -> Point {
        let (s, c) = (self.shape.rotation.radians() as f32).sin_cos();
        let (wx, wy) = (x * c - y * s, x * s + y * c);
        self.shape.pos + Point::mm(wx as f64, wy as f64)
    }

    /// World → local.
    fn local(&self, p: Point) -> (f32, f32) {
        let d = p - self.shape.pos;
        let (x, y) = ((d.x as f64 * 1e-6) as f32, (d.y as f64 * 1e-6) as f32);
        let (s, c) = (self.shape.rotation.radians() as f32).sin_cos();
        (x * c + y * s, -x * s + y * c)
    }

    /// The character index nearest to world point `p`.
    pub fn index_at(&self, p: Point) -> usize {
        let (x, y) = self.local(p);
        let h = self.height();
        let origins = self.origins();
        let lines: Vec<&str> = self.text.split('\n').collect();
        let line = (0..lines.len())
            .min_by(|&i, &j| {
                let d = |k: usize| (y - (origins[k][1] + h * 0.5)).abs();
                d(i).total_cmp(&d(j))
            })
            .unwrap_or(0);
        let col = nearest(&font::caret_positions(lines[line], h), x - origins[line][0]);
        lines[..line].iter().map(|l| l.chars().count() + 1).sum::<usize>() + col
    }

    /// A press at `p`: the caret goes there (extending the selection with `extend`).
    pub fn click(&mut self, p: Point, extend: bool) {
        let i = self.index_at(p);
        self.move_to(i, extend);
    }

    /// Dragging to `p` selects from the press.
    pub fn drag(&mut self, p: Point) {
        self.caret = self.index_at(p);
    }

    /// A double click at `p` selects the word there.
    pub fn select_word(&mut self, p: Point) {
        let i = self.index_at(p);
        let c = self.chars();
        let mut a = i;
        while a > 0 && c[a - 1].is_alphanumeric() {
            a -= 1;
        }
        let mut b = i;
        while b < c.len() && c[b].is_alphanumeric() {
            b += 1;
        }
        (self.anchor, self.caret) = (a, b);
    }

    /// Does world point `p` lie on the edited text (its frame)?
    pub fn contains(&self, p: Point) -> bool {
        let (x, y) = self.local(p);
        let (x0, y0, x1, y1) = self.local_frame();
        (x0..=x1).contains(&x) && (y0..=y1).contains(&y)
    }

    /// The text's box, local: (min x, min y, max x, max y), with a margin.
    fn local_frame(&self) -> (f32, f32, f32, f32) {
        let h = self.height();
        let origins = self.origins();
        let pad = h * 0.3;
        let mut b = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
        for (line, o) in self.text.split('\n').zip(&origins) {
            let w = font::caret_positions(line, h).last().copied().unwrap_or(0.0).max(h * 0.3);
            b = (b.0.min(o[0]), b.1.min(o[1]), b.2.max(o[0] + w), b.3.max(o[1] + h));
        }
        (b.0 - pad, b.1 - pad, b.2 + pad, b.3 + pad)
    }

    pub fn geometry(&self) -> EditGeometry {
        let h = self.height();
        let origins = self.origins();
        let lines: Vec<&str> = self.text.split('\n').collect();
        let quad = |x0: f32, y0: f32, x1: f32, y1: f32| {
            [self.world(x0, y0), self.world(x1, y0), self.world(x1, y1), self.world(x0, y1)]
        };
        let (fx0, fy0, fx1, fy1) = self.local_frame();
        let (a, b) = self.selection();
        let mut selection = Vec::new();
        let mut caret = (self.shape.pos, self.shape.pos);
        let mut start = 0;
        let (below, above) = (h * 0.25, h * 1.2);
        for (k, line) in lines.iter().enumerate() {
            let n = line.chars().count();
            let xs = font::caret_positions(line, h);
            let [ox, oy] = origins[k];
            let (sa, sb) = (a.max(start).min(start + n), b.max(start).min(start + n));
            if a != b && (sa < sb || (a <= start + n && b > start + n)) {
                let x1 = if b > start + n { xs[n] + h * 0.3 } else { xs[sb - start] };
                selection.push(quad(ox + xs[sa - start], oy - below, ox + x1, oy + above));
            }
            if (start..=start + n).contains(&self.caret) {
                let x = ox + xs[self.caret - start];
                caret = (self.world(x, oy - below), self.world(x, oy + above));
            }
            start += n + 1;
        }
        EditGeometry {
            text: Text { text: self.text.clone(), ..self.shape.clone() },
            frame: quad(fx0, fy0, fx1, fy1),
            selection,
            caret,
        }
    }

    pub fn changed(&self) -> bool {
        self.text != self.original
    }

    /// The command that applies the edit (`None`: nothing to do).
    pub fn commit(&self, project: &Project) -> Result<Option<Command>, String> {
        let text = self.text.clone();
        let empty = text.trim().is_empty();
        let unchanged =
            !self.changed() && !matches!(self.target, TextTarget::NewSchText(_) | TextTarget::NewBoardText(_));
        if unchanged {
            return Ok(None);
        }
        let need = |what: &str| if empty { Err(format!("{what} can't be empty")) } else { Ok(()) };
        let update = |o: Object| Ok(Some(Command::Update { objects: vec![o] }));
        match &self.target {
            TextTarget::SchText(i) => {
                if empty {
                    return Ok(Some(Command::Delete { ids: vec![ObjectId::SchText(*i)] }));
                }
                let mut t = project.schematic.texts.get(*i).ok_or("the text is gone")?.clone();
                t.text.text = text;
                update(Object::SchText(t))
            }
            TextTarget::BoardText(i) => {
                if empty {
                    return Ok(Some(Command::Delete { ids: vec![ObjectId::BoardText(*i)] }));
                }
                let mut t = project.board.texts.get(*i).ok_or("the text is gone")?.clone();
                t.text.text = text;
                update(Object::BoardText(t))
            }
            TextTarget::Label(i) => {
                need("A label's name")?;
                let mut l = project.schematic.labels.get(*i).ok_or("the label is gone")?.clone();
                l.name = text.trim().to_string();
                update(Object::Label(l))
            }
            TextTarget::Sheet(i) => {
                need("A sheet's name")?;
                let mut s = project.schematic.sheets.get(*i).ok_or("the sheet is gone")?.clone();
                s.name = text.trim().to_string();
                update(Object::Sheet(s))
            }
            TextTarget::Reference(c) => {
                need("A reference")?;
                Ok(Some(Command::SetComponent {
                    id: *c,
                    refdes: Some(text.trim().to_string()),
                    value: None,
                    variant: None,
                }))
            }
            TextTarget::Value(c) => {
                Ok(Some(Command::SetComponent { id: *c, refdes: None, value: Some(text), variant: None }))
            }
            TextTarget::NewSchText(t) => {
                if empty {
                    return Ok(None);
                }
                let mut t = t.clone();
                t.text.text = text;
                Ok(Some(Command::Insert { objects: vec![Object::SchText(t)] }))
            }
            TextTarget::NewBoardText(t) => {
                if empty {
                    return Ok(None);
                }
                let mut t = t.clone();
                t.text.text = text;
                Ok(Some(Command::Insert { objects: vec![Object::BoardText(t)] }))
            }
            TextTarget::ItemText { item, index } => {
                let mut content = Content::get(project, *item).ok_or("the item is gone")?;
                match &mut content {
                    Content::Symbol(s) if *index < s.texts.len() => {
                        if empty {
                            s.texts.remove(*index);
                        } else {
                            s.texts[*index].text = text;
                        }
                    }
                    Content::Footprint(f) if *index < f.texts.len() => {
                        if empty {
                            f.texts.remove(*index);
                        } else {
                            f.texts[*index].text.text = text;
                        }
                    }
                    _ => return Err("the text is gone".into()),
                }
                update(content.into_object())
            }
        }
    }
}

/// Draw an edit on `layer`: a frame covering the text's normal drawing, the selection,
/// the text and the caret.
pub(crate) fn draw(p: &mut Painter, layer: render::LayerId, g: &EditGeometry, color: render::Color) {
    use crate::draw::mmf;
    use render::Instance;
    p.fill(layer, g.frame, pal().background);
    for i in 0..4 {
        p.push(layer, Instance::line(mmf(g.frame[i]), mmf(g.frame[(i + 1) % 4]), 0.05, pal().selection));
    }
    for q in &g.selection {
        p.fill(layer, *q, pal().selection_fill);
    }
    p.text(layer, &g.text, color, &|s| s.to_string(), false);
    let w = (g.text.size as f64 * 1e-6 * 0.08).max(0.05) as f32;
    p.push(layer, Instance::line(mmf(g.caret.0), mmf(g.caret.1), w, pal().selection));
}

/// Index of the value in `xs` nearest to `x`.
fn nearest(xs: &[f32], x: f32) -> usize {
    (0..xs.len()).min_by(|&i, &j| (xs[i] - x).abs().total_cmp(&(xs[j] - x).abs())).unwrap_or(0)
}
