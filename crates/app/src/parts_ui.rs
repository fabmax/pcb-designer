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
    Library {
        key: u64,
        count: usize,
        collapsed: bool,
    },
    Part {
        key: u64,
        part: PartId,
    },
    /// The last row: the KiCad libraries' matches (opens the import window).
    Kicad,
}

/// What the panel asks for.
pub enum PartsAct {
    /// A Project panel action (place, open, copy).
    Tree(Act),
    /// Open the KiCad import window with the panel's query.
    Kicad,
}

/// The KiCad row's text.
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

/// Does `part` match every word of `query` (lower case), in its name, description,
/// default value or properties?
pub fn matches(part: &Part, words: &[String]) -> bool {
    if words.is_empty() {
        return true;
    }
    let mut hay = format!("{}\n{}\n{}", part.name, part.description, part.default_value);
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
/// all expanded), its parts in natural order. Libraries without such parts are left out.
pub fn rows(sources: &[Source], kind: Kind, query: &str, collapsed: &HashSet<u64>) -> Vec<Row> {
    let words: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
    let mut out = Vec::new();
    for s in sources {
        let mut parts: Vec<&Part> =
            s.doc.library().parts.iter().filter(|p| kind.lists(p) && matches(p, &words)).collect();
        if parts.is_empty() {
            continue;
        }
        parts.sort_by(|a, b| natural_cmp(&a.name, &b.name));
        let folded = words.is_empty() && collapsed.contains(&s.key);
        out.push(Row::Library { key: s.key, count: parts.len(), collapsed: folded });
        if !folded {
            out.extend(parts.into_iter().map(|p| Row::Part { key: s.key, part: p.id }));
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
    collapsed: HashSet<u64>,
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
        let title = match kind {
            Kind::Schematic => "Schematic parts",
            Kind::Board => "Board parts",
        };
        ui.label(RichText::new(title).color(theme::text_dim()).size(12.0));
        ui.add_space(2.0);

        // Esc in the field clears the query first (a second Esc leaves the field).
        let field_focused = ui.memory(|m| m.has_focus(search_id()));
        if field_focused && !self.query.is_empty() && ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape)) {
            self.query.clear();
        }
        let field_keys = nav_list::field_keys(ui, search_id());
        let field = egui::TextEdit::singleline(&mut self.query).id(search_id()).hint_text("Search parts");
        ui.add(field.desired_width(f32::INFINITY));
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
        ui.add_space(4.0);

        let mut rows = rows(sources, kind, &self.query, &self.collapsed);
        let no_parts = rows.is_empty();
        rows.push(Row::Kicad);
        let source = |key: u64| sources.iter().find(|s| s.key == key);
        let part_of = |key: u64, id: PartId| source(key).and_then(|s| s.doc.library().parts.get(id));
        let selected = rows.iter().position(|r| match (r, self.selected) {
            (Row::Part { key, part }, Some((k, p))) => *key == k && *part == p,
            _ => false,
        });
        let mut act: Option<Act> = None;
        let mut kicad_clicked = false;
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
                let Row::Part { key, part } = rows[i] else {
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
                Row::Part { key, part } => {
                    // Names line up with the library titles (after the chevron's room).
                    ui.allocate_exact_size(egui::vec2(12.0, 12.0), egui::Sense::hover());
                    if let Some(p) = part_of(*key, *part) {
                        nav_list::name(ui, &p.name);
                        if !p.description.is_empty() {
                            nav_list::dim(ui, &p.description);
                        }
                    }
                }
                Row::Kicad => {
                    let (r, _) = ui.allocate_exact_size(egui::vec2(12.0, 12.0), egui::Sense::hover());
                    let color = if kicad == KicadMatches::Scanning { theme::text_dim() } else { theme::accent() };
                    icons::paint(ui.painter(), r, Icon::Search, color);
                    nav_list::name(ui, RichText::new(kicad_label(kicad)).color(color));
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
            // A click on a library's row folds it; Enter on it too.
            if let Some(i) = out.select.or(out.activate)
                && let Row::Library { key, .. } = rows[i]
                && (out.activate.is_some() || ui.input(|inp| inp.pointer.any_click()))
            {
                toggle = Some(key);
            }
            // The KiCad row is a button: a click opens the import window.
            if let Some(i) = out.select.or(out.activate)
                && rows[i] == Row::Kicad
                && (out.activate.is_some() || ui.input(|inp| inp.pointer.any_click()))
            {
                kicad_clicked = true;
            }
            out
        };
        if let Some(key) = toggle
            && self.query.trim().is_empty()
            && !self.collapsed.remove(&key)
        {
            self.collapsed.insert(key);
        }
        // Keys skip library rows (to the next part in the direction moved).
        let clicked = ui.input(|inp| inp.pointer.any_click());
        let landed = match out.select {
            Some(i) if !clicked && matches!(rows[i], Row::Library { .. }) => {
                let up = selected.is_some_and(|s| i < s);
                let part = |j: &usize| !matches!(rows[*j], Row::Library { .. });
                if up { (0..i).rev().find(part) } else { (i..rows.len()).find(part) }.or(selected)
            }
            other => other,
        };
        if let Some(Row::Part { key, part }) = landed.map(|i| &rows[i]) {
            self.selected = Some((*key, *part));
        }
        if kicad_clicked {
            return Some(PartsAct::Kicad);
        }
        if let Some(Row::Part { key, part }) = out.activate.map(|i| &rows[i]) {
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
        let size = egui::vec2(ui.available_width(), preview_height);
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
            id: PartId::new(),
            name: name.into(),
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
                    Row::Kicad => "kicad".into(),
                    Row::Part { key, part } => {
                        let s = sources.iter().find(|s| s.key == *key).unwrap();
                        s.doc.library().parts.get(*part).unwrap().name.clone()
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
        let folded = HashSet::from([1]);
        assert_eq!(names(&rows(&sources, Kind::Schematic, "", &folded)), ["#1:2", "#2:1", "LM358"]);
        assert_eq!(names(&rows(&sources, Kind::Schematic, "r2", &folded)), ["#1:1", "R2"]);
    }

    #[test]
    fn kicad_row_says_what_a_click_finds() {
        assert_eq!(kicad_label(KicadMatches::Available), "Import KiCad libraries");
        assert_eq!(kicad_label(KicadMatches::NoLibraries), "Import KiCad libraries");
        assert_eq!(kicad_label(KicadMatches::Found(1)), "1 match in the KiCad libraries");
        assert_eq!(kicad_label(KicadMatches::Found(17)), "17 matches in the KiCad libraries");
        assert_eq!(kicad_label(KicadMatches::Found(MATCH_LIMIT)), "1000+ matches in the KiCad libraries");
    }
}
