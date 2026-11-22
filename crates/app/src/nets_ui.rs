//! The Nets panel: every net with its net class, pin count, the sheets it is on and its
//! routing state, filtered by a search field. A click selects the net (its wires and
//! labels in the schematic; the board highlights its copper), a double click also zooms
//! to it, and the context menu puts it into a net class.

use std::collections::HashMap;

use egui::{RichText, Ui};
use view::Document;

use crate::theme;

/// What happened in the panel.
pub enum NetsAct {
    Select(String),
    /// Select and zoom to it.
    Zoom(String),
    /// Put the net into a class (`None`: Default) by its name.
    SetClass(String, Option<String>),
}

/// How far a net is routed on the board.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Routing {
    /// The net has no pads on the board.
    NoPads,
    Routed,
    Unrouted(usize),
}

#[derive(Clone, Debug, PartialEq)]
pub struct NetRow {
    pub name: String,
    pub pins: usize,
    pub sheets: usize,
    pub routing: Routing,
    /// Its net class (`None`: Default), and whether by its exact name (else a pattern).
    pub class: Option<(String, bool)>,
}

/// The rows, sorted by name (natural order: `D2` before `D10`).
pub fn rows(doc: &Document) -> Vec<NetRow> {
    let conn = doc.connectivity();
    let copper = doc.copper();
    let mut unrouted: HashMap<u32, usize> = HashMap::new();
    for line in &copper.ratsnest {
        *unrouted.entry(line.net).or_default() += 1;
    }
    let project = doc.project();
    let mut rows: Vec<NetRow> = conn
        .netlist()
        .map(|n| {
            let mut sheets: Vec<model::Id> = n
                .wires
                .iter()
                .filter_map(|w| project.schematic.wires.get(*w).map(|w| w.sheet))
                .chain(n.labels.iter().filter_map(|l| project.schematic.labels.get(*l).map(|l| l.sheet)))
                .chain(n.pins.iter().filter_map(|p| project.schematic.symbols.get(p.symbol).map(|s| s.sheet)))
                .collect();
            sheets.sort();
            sheets.dedup();
            let routing = match copper.net_index(&n.name) {
                None => Routing::NoPads,
                Some(i) => match unrouted.get(&i) {
                    Some(&k) => Routing::Unrouted(k),
                    None => Routing::Routed,
                },
            };
            let class = project.board.rules.class_of(&n.name).map(|c| (c.name.clone(), c.lists(&n.name)));
            NetRow { name: n.name.clone(), pins: n.pins.len(), sheets: sheets.len(), routing, class }
        })
        .collect();
    rows.sort_by(|a, b| natural_cmp(&a.name, &b.name));
    rows
}

/// Compare with digit runs as numbers.
fn natural_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    let key = |s: &str| {
        let mut parts: Vec<(String, u64)> = Vec::new();
        let mut text = String::new();
        let mut num: Option<u64> = None;
        for c in s.chars() {
            match c.to_digit(10) {
                Some(d) => num = Some(num.unwrap_or(0).saturating_mul(10).saturating_add(d as u64)),
                None => {
                    if let Some(n) = num.take() {
                        parts.push((std::mem::take(&mut text), n));
                    }
                    text.extend(c.to_lowercase());
                }
            }
        }
        parts.push((text, num.unwrap_or(0)));
        parts
    };
    key(a).cmp(&key(b)).then_with(|| a.cmp(b))
}

/// The net class submenu of a row.
fn class_menu(ui: &mut Ui, row: &NetRow, rules: &model::DesignRules) -> Option<NetsAct> {
    let mut act = None;
    ui.label(RichText::new("Net class").color(theme::text_dim()).size(12.0));
    let current = row.class.as_ref().map(|(c, _)| c.as_str());
    let by_pattern = row.class.as_ref().is_some_and(|(_, exact)| !exact);
    let default = ui.radio(current.is_none(), model::DEFAULT_NET_CLASS);
    if default.clicked() && current.is_some() {
        act = Some(NetsAct::SetClass(row.name.clone(), None));
        ui.close();
    }
    for c in &rules.net_classes {
        let mut r = ui.radio(current == Some(c.name.as_str()), &c.name);
        if by_pattern && current == Some(c.name.as_str()) {
            r = r.on_hover_text("By a pattern of the class (Board setup)");
        }
        if r.clicked() && (current != Some(c.name.as_str()) || by_pattern) {
            act = Some(NetsAct::SetClass(row.name.clone(), Some(c.name.clone())));
            ui.close();
        }
    }
    if rules.net_classes.is_empty() {
        ui.label(RichText::new("Add net classes in Board setup").color(theme::text_dim()).size(12.0));
    }
    act
}

#[derive(Default)]
pub struct NetsPanel {
    query: String,
    pub selected: Option<String>,
    /// The rows and the document revision they are of.
    cache: Option<(u64, Vec<NetRow>)>,
}

const ROW: f32 = 22.0;

impl NetsPanel {
    pub fn ui(&mut self, ui: &mut Ui, doc: &Document) -> Option<NetsAct> {
        if self.cache.as_ref().is_none_or(|(r, _)| *r != doc.revision) {
            self.cache = Some((doc.revision, rows(doc)));
        }
        let all = &self.cache.as_ref().expect("filled").1;
        crate::fields::search_field(ui, &mut self.query, |e| e.hint_text("Filter nets"));
        let q = self.query.to_lowercase();
        let shown: Vec<&NetRow> = all.iter().filter(|r| q.is_empty() || r.name.to_lowercase().contains(&q)).collect();
        let unrouted: usize = all.iter().filter(|r| matches!(r.routing, Routing::Unrouted(_))).count();
        let summary = format!("{} nets, {} not fully routed", all.len(), unrouted);
        crate::fields::summary(ui, &summary);
        let mut act = None;
        let width = ui.available_width();
        egui::ScrollArea::vertical().auto_shrink([false, false]).show_rows(ui, ROW, shown.len(), |ui, range| {
            for row in &shown[range] {
                let selected = self.selected.as_deref() == Some(row.name.as_str());
                let (rect, r) = ui.allocate_exact_size(egui::vec2(width, ROW), egui::Sense::click());
                let painter = ui.painter();
                if selected {
                    painter.rect_filled(rect, 4.0, theme::accent().gamma_multiply(0.28));
                } else if r.hovered() {
                    painter.rect_filled(rect, 4.0, ui.visuals().widgets.hovered.weak_bg_fill);
                }
                let text = ui.visuals().text_color();
                let font = egui::FontId::proportional(13.5);
                let small = egui::FontId::proportional(12.0);
                let y = rect.center().y;
                let (state, color) = match row.routing {
                    Routing::NoPads => ("no pads".to_string(), theme::text_dim()),
                    Routing::Routed => ("routed".to_string(), theme::success()),
                    Routing::Unrouted(n) => (format!("{n} unrouted"), theme::warn()),
                };
                let right = painter.text(
                    egui::pos2(rect.right() - 6.0, y),
                    egui::Align2::RIGHT_CENTER,
                    state,
                    small.clone(),
                    color,
                );
                let pins = if row.pins == 1 { "1 pin".to_string() } else { format!("{} pins", row.pins) };
                let pins = match &row.class {
                    Some((class, _)) => format!("{class} · {pins}"),
                    None => pins,
                };
                let sheets = if row.sheets > 1 { format!(" · {} sheets", row.sheets) } else { String::new() };
                let mid = painter.text(
                    egui::pos2(right.left() - 10.0, y),
                    egui::Align2::RIGHT_CENTER,
                    format!("{pins}{sheets}"),
                    small,
                    theme::text_dim(),
                );
                let name_rect = egui::Rect::from_min_max(rect.left_top(), egui::pos2(mid.left() - 8.0, rect.bottom()));
                painter.with_clip_rect(name_rect).text(
                    egui::pos2(rect.left() + 6.0, y),
                    egui::Align2::LEFT_CENTER,
                    &row.name,
                    font,
                    text,
                );
                if r.double_clicked() {
                    act = Some(NetsAct::Zoom(row.name.clone()));
                } else if r.clicked() {
                    act = Some(NetsAct::Select(row.name.clone()));
                }
                r.context_menu(|ui| {
                    if let Some(a) = class_menu(ui, row, &doc.project().board.rules) {
                        act = Some(a);
                    }
                });
            }
        });
        if let Some(NetsAct::Select(n) | NetsAct::Zoom(n)) = &act {
            self.selected = Some(n.clone());
        }
        act
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn natural_order() {
        let mut v = vec!["D10", "d2", "GND", "D1", "/A"];
        v.sort_by(|a, b| natural_cmp(a, b));
        assert_eq!(v, ["/A", "D1", "d2", "D10", "GND"]);
    }

    #[test]
    fn demo_rows() {
        let doc = Document::new(view::demo::demo_project());
        let rows = rows(&doc);
        assert!(!rows.is_empty());
        let gnd = rows.iter().find(|r| r.name == "GND").expect("GND");
        assert!(gnd.pins >= 2);
        // The demo board isn't fully routed.
        assert!(rows.iter().any(|r| matches!(r.routing, Routing::Unrouted(_))), "{rows:?}");
    }
}
