//! The Messages panel: schematic problems (ERC), board problems, design rule check
//! results and library issues, side by side. Clicking a message shows its objects.

use editor_core::copper::{CopperItem, Flag};
use editor_core::drc::ViolationKind;
use egui::RichText;
use model::*;
use view::{Document, ViewId};

use crate::dock::problem_key;
use crate::fields::hint;
use crate::library::LibraryEditor;
use crate::theme;
use crate::units;

/// What a clicked message wants shown.
pub enum Show {
    /// Select objects in a view and centre on `pos`.
    Objects(ViewId, Vec<ObjectId>, Option<Point>),
    /// Open a part in a library tab.
    LibraryPart(u64, Id),
}

/// Keys of all current messages (for the unseen-messages badge).
pub fn keys(doc: &Document, libraries: &mut [LibraryEditor]) -> Vec<u64> {
    let mut keys: Vec<u64> = doc.connectivity().problems.iter().map(|p| problem_key("sch", &p.message)).collect();
    let copper = doc.copper();
    keys.extend(copper.shorts.iter().map(|(nets, at)| problem_key("short", &format!("{nets:?}{at:?}"))));
    keys.extend(copper.flagged().map(|c| problem_key("flag", &format!("{:?}", c.item.object()))));
    if let Some(r) = doc.drc() {
        keys.extend(r.violations.iter().map(|v| problem_key("drc", &v.message)));
    }
    for l in libraries {
        let name = l.lib.name().to_string();
        keys.extend(l.issues().iter().map(|(_, m)| problem_key(&name, m)));
    }
    keys
}

/// The panel. Returns a clicked message's target.
pub fn ui(ui: &mut egui::Ui, doc: &mut Document, libraries: &mut [LibraryEditor]) -> Option<Show> {
    let mut show = None;
    ui.columns(3, |cols| {
        column(&mut cols[0], "messages_sch", |ui| show = show.take().or(schematic(ui, doc)));
        column(&mut cols[1], "messages_board", |ui| show = show.take().or(board(ui, doc)));
        column(&mut cols[2], "messages_libs", |ui| show = show.take().or(library_issues(ui, libraries)));
    });
    show
}

fn column(ui: &mut egui::Ui, id: &str, add: impl FnOnce(&mut egui::Ui)) {
    egui::ScrollArea::vertical().id_salt(id).auto_shrink([false, false]).show(ui, add);
}

fn heading(ui: &mut egui::Ui, title: &str, count: usize) {
    let color = if count > 0 { theme::warn() } else { theme::success() };
    ui.horizontal(|ui| {
        ui.label(RichText::new(title).strong());
        let text = if count > 0 { count.to_string() } else { "OK".to_string() };
        ui.label(RichText::new(text).color(color));
    });
    ui.add_space(2.0);
}

/// A clickable message line.
fn item(ui: &mut egui::Ui, text: &str) -> bool {
    let r = ui.add(egui::Button::selectable(false, text).truncate());
    r.on_hover_text(text).clicked()
}

const SHOWN: usize = 300;

fn more(ui: &mut egui::Ui, total: usize) {
    if total > SHOWN {
        hint(ui, &format!("… and {} more", total - SHOWN));
    }
}

fn schematic(ui: &mut egui::Ui, doc: &Document) -> Option<Show> {
    let problems = &doc.connectivity().problems;
    heading(ui, "Schematic", problems.len());
    if problems.is_empty() {
        hint(ui, "No problems: nets are named consistently and buses are used correctly.");
    }
    let mut show = None;
    for p in problems.iter().take(SHOWN) {
        if item(ui, &p.message) {
            // The sheet of the first object that is drawn on one; only its objects are selected.
            let s = &doc.project().schematic;
            let sheet = p.objects.iter().find_map(|&o| s.sheet_of(o)).or(s.root_sheet());
            let view = sheet.map_or(doc.root(), ViewId::Sheet);
            let objects = p.objects.iter().copied().filter(|&o| s.sheet_of(o) == sheet).collect();
            show = Some(Show::Objects(view, objects, p.pos));
        }
    }
    more(ui, problems.len());
    show
}

fn board(ui: &mut egui::Ui, doc: &mut Document) -> Option<Show> {
    let mut show = None;
    let copper = doc.copper();
    let flagged: Vec<ObjectId> = copper
        .flagged()
        .filter(|c| matches!(c.item, CopperItem::Track(_) | CopperItem::Via(_)))
        .map(|c| c.item.object())
        .collect();
    let unconnected = copper.flagged().filter(|c| c.flag == Some(Flag::Unconnected)).count();
    heading(ui, "Board", copper.shorts.len() + usize::from(!flagged.is_empty()));
    for (nets, at) in &copper.shorts {
        if item(ui, &format!("Short: {} at {}", nets.join(" ↔ "), units::fmt_point(*at))) {
            show = Some(Show::Objects(ViewId::Board, Vec::new(), Some(*at)));
        }
    }
    if !flagged.is_empty() {
        let text = if unconnected > 0 {
            format!("{} flagged track(s)/via(s), {unconnected} touching no pad", flagged.len())
        } else {
            format!("{} flagged track(s)/via(s)", flagged.len())
        };
        if item(ui, &text) {
            show = Some(Show::Objects(ViewId::Board, flagged.clone(), None));
        }
        hint(ui, "Schematic changes never delete copper; it is flagged here instead.");
    }
    if copper.shorts.is_empty() && flagged.is_empty() {
        hint(ui, "No shorts or flagged copper.");
    }

    ui.add_space(8.0);
    let count = doc.drc().map_or(0, |r| r.violations.len());
    ui.horizontal(|ui| {
        ui.label(RichText::new("Design rules").strong());
        match doc.drc() {
            Some(r) if r.violations.is_empty() => {
                ui.label(RichText::new("OK").color(theme::success()));
            }
            Some(_) => {
                ui.label(RichText::new(count.to_string()).color(theme::warn()));
            }
            None => {}
        }
        if ui.small_button("Run DRC").clicked() {
            doc.run_drc();
        }
        if doc.drc().is_some() && ui.small_button("Clear").clicked() {
            doc.clear_drc();
        }
    });
    let Some(report) = doc.drc() else {
        hint(ui, "Checks clearances, shorts, unrouted nets, the board edge, courtyards, track widths and drills.");
        return show;
    };
    let outdated = report.revision != doc.revision;
    let status = format!(
        "{} in {:.1} ms{}",
        if report.violations.is_empty() { "Clean" } else { "Found problems" },
        report.time.as_secs_f64() * 1e3,
        if outdated { " · board changed since" } else { "" }
    );
    ui.label(RichText::new(status).color(if outdated { theme::warn() } else { theme::text_dim() }).small());
    let mut kind: Option<ViolationKind> = None;
    for v in report.violations.iter().take(SHOWN) {
        if kind != Some(v.kind) {
            kind = Some(v.kind);
            let n = report.violations.iter().filter(|w| w.kind == v.kind).count();
            ui.add_space(2.0);
            ui.label(RichText::new(format!("{} ({n})", v.kind.label())).color(theme::text_dim()));
        }
        if item(ui, &v.message) {
            show = Some(Show::Objects(ViewId::Board, v.objects.clone(), Some(v.pos)));
        }
    }
    more(ui, report.violations.len());
    show
}

fn library_issues(ui: &mut egui::Ui, libraries: &mut [LibraryEditor]) -> Option<Show> {
    let mut show = None;
    let total: usize = libraries.iter_mut().map(|l| l.issues().len()).sum();
    heading(ui, "Libraries", total);
    if total == 0 {
        hint(ui, "The open libraries have no issues.");
    }
    for l in libraries {
        let (key, name) = (l.key, l.lib.name().to_string());
        let issues = l.issues();
        if issues.is_empty() {
            continue;
        }
        ui.label(RichText::new(format!("Library {name} ({})", issues.len())).color(theme::text_dim()));
        for (part, message) in issues.iter().take(SHOWN) {
            if item(ui, message) {
                show = Some(Show::LibraryPart(key, *part));
            }
        }
        more(ui, issues.len());
    }
    show
}
