//! The Parts panel: every part of the workspace's libraries that the active editor can
//! place (schematic parts on the schematic, board parts on the board), grouped by
//! library and filtered by a search field, with a preview of the selected part. Double
//! click or Enter starts the Place tool.

use std::collections::HashSet;

use egui::{Id, Key, Modifiers, RichText, Ui};
use model::{Id as PartId, Part, natural_cmp};
use view::LibraryDoc;
use view::item::Item;

use crate::icons::{self, Icon};
use crate::kicad_import::{KicadMatches, MATCH_LIMIT};
use crate::library::{Open, PROJECT_LIB};
use crate::nav_list;
use crate::preview::{Empty, Preview};
use crate::project_panel::{Act, Owner};
use crate::theme;

/// Which parts the panel lists: those the active editor places.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Schematic,
    Board,
}

impl Kind {
    fn lists(self, part: &Part) -> bool {
        part.is_board_part() == (self == Kind::Board)
    }
}

/// A library the panel lists parts of.
pub struct Source<'a> {
    pub key: u64,
    pub title: String,
    pub doc: &'a LibraryDoc,
    /// The library's first problem: placing from it is refused.
    pub issue: Option<String>,
}

/// A row of the list.
#[derive(Clone, Debug, PartialEq)]
pub enum Row {
    /// A library: its key, the number of parts listed (matching) and whether it's
    /// collapsed.
    Library { key: u64, count: usize, collapsed: bool },
    /// A category of a library: its name, the number of parts listed and whether it's
    /// collapsed.
    Category { key: u64, name: String, count: usize, collapsed: bool },
    /// A part; `nested` in a category.
    Part { key: u64, part: PartId, nested: bool },
}

impl Row {
    /// What a click on the row folds.
    fn fold(&self) -> Option<Fold> {
        match self {
            Row::Library { key, .. } => Some(Fold::Library(*key)),
            Row::Category { key, name, .. } => Some(Fold::Category(*key, name.clone())),
            Row::Part { .. } => None,
        }
    }
}

/// A library or category whose folding the user toggled: libraries start open,
/// categories closed.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Fold {
    Library(u64),
    Category(u64, String),
}

/// What the panel asks for.
pub enum PartsAct {
    /// A Project panel action (place, open, copy).
    Tree(Act),
    /// Open the KiCad import window with the panel's query.
    Kicad,
}

/// The KiCad link's text.
fn kicad_label(m: KicadMatches) -> String {
    match m {
        KicadMatches::Scanning => "Searching the KiCad libraries…".into(),
        KicadMatches::NoLibraries | KicadMatches::Available => "Import KiCad libraries".into(),
        KicadMatches::Found(0) => "No matches in the KiCad libraries".into(),
        KicadMatches::Found(1) => "1 match in the KiCad libraries".into(),
        KicadMatches::Found(n) if n >= MATCH_LIMIT => format!("{MATCH_LIMIT}+ matches in the KiCad libraries"),
        KicadMatches::Found(n) => format!("{n} matches in the KiCad libraries"),
    }
}

/// The KiCad link under the search field: a full-width row with the matches in the
/// KiCad libraries (a click opens the import window with the query).
fn kicad_link(ui: &mut Ui, kicad: KicadMatches) -> egui::Response {
    let height = nav_list::row_height(ui);
    let (rect, response) = ui.allocate_exact_size(egui::vec2(ui.available_width(), height), egui::Sense::click());
    let color = if kicad == KicadMatches::Scanning { theme::text_dim() } else { theme::accent() };
    if response.hovered() {
        ui.painter().rect_filled(rect, 4.0, ui.visuals().widgets.hovered.weak_bg_fill);
    }
    let icon = egui::Rect::from_center_size(egui::pos2(rect.left() + 10.0, rect.center().y), egui::vec2(12.0, 12.0));
    icons::paint(ui.painter(), icon, Icon::Search, color);
    let font = egui::TextStyle::Body.resolve(ui.style());
    let text = ui.painter().layout_no_wrap(kicad_label(kicad), font, color);
    ui.painter().galley(egui::pos2(icon.right() + 6.0, rect.center().y - text.size().y / 2.0), text, color);
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// Does `part` match every word of `query` (lower case), in its name, category,
/// description, default value or properties?
pub fn matches(part: &Part, words: &[String]) -> bool {
    if words.is_empty() {
        return true;
    }
    let mut hay = format!("{}\n{}\n{}\n{}", part.name, part.category, part.description, part.default_value);
    for (k, v) in &part.properties {
        hay.push('\n');
        hay.push_str(k);
        hay.push('\n');
        hay.push_str(v);
    }
    let hay = hay.to_lowercase();
    words.iter().all(|w| hay.contains(w.as_str()))
}

/// The rows: a header per library with parts of `kind` (with a query: matching ones,
/// all expanded), then its categories (in natural order, closed unless toggled) with
/// their parts, then its
/// uncategorised parts; parts in natural order. Libraries without such parts are left out.
pub fn rows(sources: &[Source], kind: Kind, query: &str, toggled: &HashSet<Fold>) -> Vec<Row> {
    let words: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
    let searching = !words.is_empty();
    let mut out = Vec::new();
    for s in sources {
        let mut parts: Vec<&Part> =
            s.doc.library().parts.iter().filter(|p| kind.lists(p) && matches(p, &words)).collect();
        if parts.is_empty() {
            continue;
        }
        parts.sort_by(|a, b| {
            (a.category.is_empty().cmp(&b.category.is_empty()))
                .then_with(|| natural_cmp(&a.category, &b.category))
                .then_with(|| a.category.cmp(&b.category))
                .then_with(|| natural_cmp(&a.name, &b.name))
        });
        let key = s.key;
        let closed = !searching && toggled.contains(&Fold::Library(key));
        out.push(Row::Library { key, count: parts.len(), collapsed: closed });
        if closed {
            continue;
        }
        for chunk in parts.chunk_by(|a, b| a.category == b.category) {
            let name = &chunk[0].category;
            if name.is_empty() {
                out.extend(chunk.iter().map(|p| Row::Part { key, part: p.id, nested: false }));
                continue;
            }
            let closed = !searching && !toggled.contains(&Fold::Category(key, name.clone()));
            out.push(Row::Category { key, name: name.clone(), count: chunk.len(), collapsed: closed });
            if !closed {
                out.extend(chunk.iter().map(|p| Row::Part { key, part: p.id, nested: true }));
            }
        }
    }
    out
}

/// What the preview shows of a part: unit A's symbol, or the default variant's footprint.
fn preview_item(part: &Part, kind: Kind) -> Option<Item> {
    match kind {
        Kind::Schematic => part.units.first().map(|u| Item::Symbol(u.symbol)),
        Kind::Board => part.variants.first().and_then(|v| v.footprint).map(Item::Footprint),
    }
}

fn owner(key: u64) -> Owner {
    if key == PROJECT_LIB { Owner::Project } else { Owner::Lib(key) }
}

const PREVIEW_HEIGHT: f32 = 200.0;
/// Indentation of a category and of its parts.
const INDENT: f32 = 12.0;

fn search_id() -> Id {
    Id::unique("parts_search")
}

fn list_id() -> Id {
    Id::unique("parts_list")
}

#[derive(Default)]
pub struct PartsPanel {
    query: String,
    selected: Option<(u64, PartId)>,
    toggled: HashSet<Fold>,
    preview: Preview,
    /// The library the preview last showed (its view is redrawn when that changes).
    previewed: Option<u64>,
    /// Focus the search field (and select its text) on the next frame.
    focus_search: bool,
}

impl PartsPanel {
    pub fn query(&self) -> &str {
        &self.query
    }

    /// P: focus the search field with its text selected.
    pub fn focus_search(&mut self) {
        self.focus_search = true;
    }

    /// The colours changed: redraw the preview.
    pub fn restyle(&mut self) {
        self.preview.invalidate();
    }

    pub fn ui(&mut self, ui: &mut Ui, kind: Kind, sources: &[Source], kicad: KicadMatches) -> Option<PartsAct> {
        // The list may overflow (and widen the ui); the preview keeps the panel's width.
        let width = ui.available_width();
        let title = match kind {
            Kind::Schematic => "Schematic parts",
            Kind::Board => "Board parts",
        };

        // Esc in the field clears the query first (a second Esc leaves the field).
        let field_focused = ui.memory(|m| m.has_focus(search_id()));
        if field_focused && !self.query.is_empty() && ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape)) {
            self.query.clear();
        }
        let field_keys = nav_list::field_keys(ui, search_id());
        crate::fields::search_field(ui, &mut self.query, |e| e.id(search_id()).hint_text("Search parts"));
        if std::mem::take(&mut self.focus_search) {
            ui.memory_mut(|m| m.request_focus(search_id()));
            if let Some(mut state) = egui::TextEdit::load_state(ui.ctx(), search_id()) {
                let all = egui::text::CCursorRange::two(
                    egui::text::CCursor::new(0),
                    egui::text::CCursor::new(self.query.chars().count()),
                );
                state.cursor.set_char_range(Some(all));
                state.store(ui.ctx(), search_id());
            }
        }
        crate::fields::summary(ui, title);
        if kicad_link(ui, kicad).clicked() {
            return Some(PartsAct::Kicad);
        }
        ui.add_space(2.0);

        let rows = rows(sources, kind, &self.query, &self.toggled);
        let no_parts = rows.is_empty();
        let source = |key: u64| sources.iter().find(|s| s.key == key);
        let part_of = |key: u64, id: PartId| source(key).and_then(|s| s.doc.library().parts.get(id));
        let selected = rows.iter().position(|r| match (r, self.selected) {
            (Row::Part { key, part, .. }, Some((k, p))) => *key == k && *part == p,
            _ => false,
        });
        let mut act: Option<Act> = None;
        let info_height = 2.0 * nav_list::row_height(ui);
        // The preview gives way to the list in a short panel.
        let preview_height = (ui.available_height() * 0.4).min(PREVIEW_HEIGHT);
        let list_height = (ui.available_height() - preview_height - info_height - 8.0).max(60.0);
        let mut toggle = None;
        if no_parts {
            let text = match kind {
                _ if !self.query.trim().is_empty() => "No parts match.",
                Kind::Schematic => "No schematic parts in the libraries.",
                Kind::Board => {
                    "No board parts in the libraries. Board parts are parts without a symbol (mounting \
                     holes, logos, fiducials); create one in a library."
                }
            };
            ui.label(RichText::new(text).color(theme::text_dim()));
            ui.add_space(4.0);
        }
        let list_height = (list_height - if no_parts { 40.0 } else { 0.0 }).max(40.0);
        let out = {
            let mut menu = |ui: &mut Ui, i: usize| {
                let Row::Part { key, part, .. } = rows[i] else {
                    ui.close();
                    return;
                };
                if ui.button("Place").clicked() {
                    act = Some(Act::Place(owner(key), part));
                }
                if ui.button("Open in library").clicked() {
                    act = Some(Act::OpenItem(key, Open::Part(part)));
                }
                if key != PROJECT_LIB && ui.button("Copy to Project Library").clicked() {
                    act = Some(Act::CopyToProject(key, part));
                }
            };
            let row_ui = |ui: &mut Ui, i: usize| match &rows[i] {
                Row::Library { key, count, collapsed } => {
                    let Some(s) = source(*key) else { return };
                    let open = !*collapsed;
                    let (r, _) = ui.allocate_exact_size(egui::vec2(12.0, 12.0), egui::Sense::hover());
                    crate::project_panel::chevron_paint(ui.painter(), r, open, theme::text_dim());
                    nav_list::name(ui, RichText::new(&s.title).strong());
                    nav_list::dim(ui, &count.to_string());
                    if let Some(issue) = &s.issue {
                        let (r, resp) = ui.allocate_exact_size(egui::vec2(14.0, 14.0), egui::Sense::hover());
                        icons::paint(ui.painter(), r, Icon::Missing, theme::warn());
                        resp.on_hover_text(format!("Placing is refused until the library is fixed: {issue}"));
                    }
                }
                Row::Category { name, count, collapsed, .. } => {
                    ui.add_space(INDENT);
                    let (r, _) = ui.allocate_exact_size(egui::vec2(12.0, 12.0), egui::Sense::hover());
                    crate::project_panel::chevron_paint(ui.painter(), r, !*collapsed, theme::text_dim());
                    nav_list::name(ui, RichText::new(name).strong());
                    nav_list::dim(ui, &count.to_string());
                }
                Row::Part { key, part, nested } => {
                    // Names line up with the library titles (after the chevron's room), or
                    // the category names.
                    ui.allocate_exact_size(egui::vec2(12.0, 12.0), egui::Sense::hover());
                    if *nested {
                        ui.add_space(INDENT);
                    }
                    if let Some(p) = part_of(*key, *part) {
                        nav_list::name(ui, &p.name);
                        if !p.description.is_empty() {
                            nav_list::dim(ui, &p.description);
                        }
                    }
                }
            };
            let fill = |i: usize| matches!(rows[i], Row::Library { .. }).then(theme::panel);
            let out = egui::Frame::NONE
                .fill(theme::panel_dark())
                .corner_radius(4)
                .inner_margin(egui::Margin::same(2))
                .show(ui, |ui| {
                    ui.set_min_height(list_height);
                    nav_list::show_with_menu(
                        ui,
                        list_id(),
                        rows.len(),
                        selected,
                        list_height,
                        false,
                        field_keys,
                        row_ui,
                        Some(&mut menu),
                        &fill,
                    )
                })
                .inner;
            // A click on a library's or category's row folds it; Enter on it too.
            if let Some(i) = out.select.or(out.activate)
                && let Some(fold) = rows[i].fold()
                && (out.activate.is_some() || ui.input(|inp| inp.pointer.any_click()))
            {
                toggle = Some(fold);
            }
            out
        };
        if let Some(fold) = toggle
            && self.query.trim().is_empty()
            && !self.toggled.remove(&fold)
        {
            self.toggled.insert(fold);
        }
        // Keys skip library and category rows (to the next part in the direction moved).
        let clicked = ui.input(|inp| inp.pointer.any_click());
        let landed = match out.select {
            Some(i) if !clicked && !matches!(rows[i], Row::Part { .. }) => {
                let up = selected.is_some_and(|s| i < s);
                let part = |j: &usize| matches!(rows[*j], Row::Part { .. });
                if up { (0..i).rev().find(part) } else { (i..rows.len()).find(part) }.or(selected)
            }
            other => other,
        };
        if let Some(Row::Part { key, part, .. }) = landed.map(|i| &rows[i]) {
            self.selected = Some((*key, *part));
        }
        if let Some(Row::Part { key, part, .. }) = out.activate.map(|i| &rows[i]) {
            self.selected = Some((*key, *part));
            act = Some(Act::Place(owner(*key), *part));
        }
        if matches!(act, Some(Act::Place(..))) {
            // The canvas takes the keys (R, X, Esc) of the part being placed.
            ui.memory_mut(|m| {
                m.surrender_focus(search_id());
                m.surrender_focus(list_id());
            });
        }

        // The preview and a line about the selected part.
        ui.add_space(4.0);
        let shown = self.selected.and_then(|(k, id)| Some((k, source(k)?, part_of(k, id)?)));
        let size = egui::vec2(width, preview_height);
        match shown {
            Some((key, s, part)) => {
                if self.previewed != Some(key) {
                    self.previewed = Some(key);
                    self.preview.invalidate();
                }
                let empty = match kind {
                    Kind::Schematic => Empty::Hint("No symbol"),
                    Kind::Board => Empty::Hint("No footprint"),
                };
                self.preview.ui(ui, &s.doc.doc, preview_item(part, kind), size, empty);
                let mut info = vec![s.title.clone()];
                if !part.description.is_empty() {
                    info.push(part.description.clone());
                }
                let footprint = part.variants.first().and_then(|v| v.footprint);
                if let Some(fp) = footprint.and_then(|f| s.doc.library().footprints.get(f))
                    && kind == Kind::Schematic
                {
                    info.push(fp.name.clone());
                }
                let info = info.join(" · ");
                let text = RichText::new(&info).color(theme::text_dim()).size(12.0);
                ui.add(egui::Label::new(text).truncate()).on_hover_text(info);
            }
            None => {
                let empty_doc = sources.first().map(|s| &s.doc.doc);
                if let Some(doc) = empty_doc {
                    self.preview.ui(ui, doc, None, size, Empty::Hint("Select a part to see it here"));
                }
            }
        }
        act.map(PartsAct::Tree)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use model::{Library, PartUnit, PartVariant};

    fn part(name: &str, board: bool) -> Part {
        let mut p = Part {
            sim: Default::default(),
            id: PartId::new(),
            name: name.into(),
            category: String::new(),
            description: String::new(),
            prefix: "R".into(),
            default_value: String::new(),
            units: Vec::new(),
            variants: Vec::new(),
            properties: Default::default(),
            source: None,
        };
        if !board {
            p.units.push(PartUnit { name: String::new(), symbol: PartId::new() });
        }
        p.variants.push(PartVariant { id: PartId::new(), name: String::new(), footprint: None, connections: vec![] });
        p
    }

    fn lib(parts: Vec<Part>) -> LibraryDoc {
        let mut l = Library::default();
        for p in parts {
            l.parts.insert(p);
        }
        LibraryDoc::new(l)
    }

    #[test]
    fn rows_follow_the_editor_the_query_and_folding() {
        let mut r10 = part("R10", false);
        r10.properties.insert("MPN".into(), "RC0603FR-0710KL".into());
        let a = lib(vec![r10, part("R2", false), part("MountingHole", true)]);
        let b = lib(vec![part("LM358", false)]);
        let sources = [
            Source { key: 1, title: "A".into(), doc: &a, issue: None },
            Source { key: 2, title: "B".into(), doc: &b, issue: None },
        ];
        let names = |rows: &[Row]| -> Vec<String> {
            rows.iter()
                .map(|r| match r {
                    Row::Library { key, count, .. } => format!("#{key}:{count}"),
                    Row::Category { name, count, collapsed, .. } => {
                        format!("{}{name}:{count}", if *collapsed { "+" } else { "-" })
                    }
                    Row::Part { key, part, nested } => {
                        let s = sources.iter().find(|s| s.key == *key).unwrap();
                        let name = s.doc.library().parts.get(*part).unwrap().name.clone();
                        if *nested { format!("  {name}") } else { name }
                    }
                })
                .collect()
        };
        let none = HashSet::new();
        assert_eq!(names(&rows(&sources, Kind::Schematic, "", &none)), ["#1:2", "R2", "R10", "#2:1", "LM358"]);
        assert_eq!(names(&rows(&sources, Kind::Board, "", &none)), ["#1:1", "MountingHole"]);
        // Words must all match, in any field; libraries without matches go.
        assert_eq!(names(&rows(&sources, Kind::Schematic, "r 0710k", &none)), ["#1:1", "R10"]);
        assert_eq!(names(&rows(&sources, Kind::Schematic, "nothing", &none)), Vec::<String>::new());
        // Folded libraries show their header only, unless searching.
        let folded = HashSet::from([Fold::Library(1)]);
        assert_eq!(names(&rows(&sources, Kind::Schematic, "", &folded)), ["#1:2", "#2:1", "LM358"]);
        assert_eq!(names(&rows(&sources, Kind::Schematic, "r2", &folded)), ["#1:1", "R2"]);
    }

    #[test]
    fn categories_group_parts_under_their_library() {
        let cat = |name: &str, category: &str| Part { category: category.into(), ..part(name, false) };
        let a = lib(vec![cat("R", "Passives"), cat("Q", "Transistors"), part("X", false), cat("C", "Passives")]);
        let sources = [Source { key: 1, title: "A".into(), doc: &a, issue: None }];
        let names = |rows: &[Row]| -> Vec<String> {
            rows.iter()
                .map(|r| match r {
                    Row::Library { count, .. } => format!("#{count}"),
                    Row::Category { name, count, collapsed, .. } => {
                        format!("{}{name}:{count}", if *collapsed { "+" } else { "-" })
                    }
                    Row::Part { part, nested, .. } => {
                        let name = a.library().parts.get(*part).unwrap().name.clone();
                        if *nested { format!("  {name}") } else { name }
                    }
                })
                .collect()
        };
        // Categories start closed; a toggle opens one.
        let none = HashSet::new();
        assert_eq!(names(&rows(&sources, Kind::Schematic, "", &none)), ["#4", "+Passives:2", "+Transistors:1", "X"]);
        let opened = HashSet::from([Fold::Category(1, "Passives".into())]);
        assert_eq!(
            names(&rows(&sources, Kind::Schematic, "", &opened)),
            ["#4", "-Passives:2", "  C", "  R", "+Transistors:1", "X"]
        );
        // The search matches categories (words ANDed) and shows them expanded.
        assert_eq!(names(&rows(&sources, Kind::Schematic, "passives", &none)), ["#2", "-Passives:2", "  C", "  R"]);
        assert_eq!(names(&rows(&sources, Kind::Schematic, "pass r", &none)), ["#1", "-Passives:1", "  R"]);
    }

    #[test]
    fn kicad_link_says_what_a_click_finds() {
        assert_eq!(kicad_label(KicadMatches::Available), "Import KiCad libraries");
        assert_eq!(kicad_label(KicadMatches::NoLibraries), "Import KiCad libraries");
        assert_eq!(kicad_label(KicadMatches::Found(1)), "1 match in the KiCad libraries");
        assert_eq!(kicad_label(KicadMatches::Found(17)), "17 matches in the KiCad libraries");
        assert_eq!(kicad_label(KicadMatches::Found(MATCH_LIMIT)), "1000+ matches in the KiCad libraries");
    }
}
