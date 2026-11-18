//! The Messages panel: schematic problems (ERC), board problems, design rule check
//! results and library issues, side by side. Clicking a message shows its objects.

use editor_core::Command;
use editor_core::copper::{CopperItem, Flag};
use editor_core::drc::short_key;
use egui::RichText;
use model::*;
use view::{Document, ViewId};

use crate::checks_ui;
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

/// Keys of all current messages (for the unseen-messages badge). Excluded findings don't
/// count.
pub fn keys(doc: &Document, libraries: &mut [LibraryEditor]) -> Vec<u64> {
    let mut keys: Vec<u64> =
        doc.connectivity().problems.iter().filter(|p| !p.excluded).map(|p| problem_key("sch", &p.key)).collect();
    keys.extend(live_shorts(doc).map(|(nets, at)| problem_key("short", &format!("{nets:?}{at:?}"))));
    keys.extend(doc.copper().flagged().map(|c| problem_key("flag", &format!("{:?}", c.item.object()))));
    if let Some(r) = doc.drc() {
        keys.extend(r.violations.iter().filter(|v| !v.excluded).map(|v| problem_key("drc", &v.key)));
    }
    for l in libraries {
        let name = l.lib.name().to_string();
        keys.extend(l.issues().iter().map(|(_, m)| problem_key(&name, m)));
    }
    keys
}

/// The live board shorts the DRC settings don't ignore or exclude.
fn live_shorts(doc: &Document) -> impl Iterator<Item = &(Vec<String>, Point)> {
    let drc = &doc.project().board.drc;
    let shown = drc.severity(ViolationKind::Short) != Severity::Ignore;
    doc.copper().shorts.iter().filter(move |(nets, _)| shown && drc.excluded(&short_key(nets)).is_none())
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

/// "2 errors, 3 warnings" or "OK", in the colour of the worst.
fn counts_label(ui: &mut egui::Ui, errors: usize, warnings: usize) {
    let plural = |n: usize, what: &str| format!("{n} {what}{}", if n == 1 { "" } else { "s" });
    let (text, color) = match (errors, warnings) {
        (0, 0) => ("OK".to_string(), theme::success()),
        (e, 0) => (plural(e, "error"), theme::error()),
        (0, w) => (plural(w, "warning"), theme::warn()),
        (e, w) => (format!("{}, {}", plural(e, "error"), plural(w, "warning")), theme::error()),
    };
    ui.label(RichText::new(text).color(color));
}

fn severity_heading(ui: &mut egui::Ui, title: &str, errors: usize, warnings: usize) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(title).strong());
        counts_label(ui, errors, warnings);
    });
    ui.add_space(2.0);
}

/// A clickable message line.
fn item(ui: &mut egui::Ui, text: &str) -> bool {
    let r = ui.add(egui::Button::selectable(false, text).truncate());
    r.on_hover_text(text).clicked()
}

/// What a finding's row asks for.
enum RowAction {
    Show,
    Exclude(String),
    Include,
    Comment(String),
}

/// A finding: severity icon and message, clickable; right-click to exclude it (with an
/// optional comment) or, when excluded, to include it again or edit the comment.
fn finding(
    ui: &mut egui::Ui,
    severity: Severity,
    excluded: Option<&Exclusion>,
    message: &str,
    key: &str,
) -> Option<RowAction> {
    let mut out = None;
    let r = ui
        .horizontal(|ui| {
            checks_ui::icon(ui, severity, excluded.is_some());
            let text = match excluded {
                Some(e) if !e.comment.is_empty() => format!("{message} · {}", e.comment),
                _ => message.to_string(),
            };
            let r = ui.add(egui::Button::selectable(false, &text).truncate());
            r.on_hover_text(text)
        })
        .inner;
    if r.clicked() {
        out = Some(RowAction::Show);
    }
    let buf = egui::Id::unique(("exclusion_comment", key));
    r.context_menu(|ui| {
        let mut comment: String = ui.data_mut(|d| {
            d.get_temp_mut_or_insert_with(buf, || excluded.map_or(String::new(), |e| e.comment.clone())).clone()
        });
        ui.set_min_width(240.0);
        let edit = ui.add(egui::TextEdit::singleline(&mut comment).hint_text("Comment (optional)"));
        let enter = edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        ui.data_mut(|d| d.insert_temp(buf, comment.clone()));
        let done = |ui: &mut egui::Ui| {
            ui.data_mut(|d| d.remove_temp::<String>(buf));
            ui.close();
        };
        match excluded {
            None => {
                if ui.button("Exclude").clicked() || enter {
                    out = Some(RowAction::Exclude(comment.trim().to_string()));
                    done(ui);
                }
            }
            Some(_) => {
                if ui.button("Save comment").clicked() || enter {
                    out = Some(RowAction::Comment(comment.trim().to_string()));
                    done(ui);
                }
                if ui.button("Include again").clicked() {
                    out = Some(RowAction::Include);
                    done(ui);
                }
            }
        }
    });
    out
}

/// Apply a row's exclusion edit to `exclusions`. False if nothing changed.
fn edit_exclusions(exclusions: &mut Vec<Exclusion>, action: &RowAction, key: &str, message: &str) -> bool {
    match action {
        RowAction::Show => return false,
        RowAction::Exclude(comment) => {
            exclusions.push(Exclusion { key: key.to_string(), message: message.to_string(), comment: comment.clone() })
        }
        RowAction::Include => exclusions.retain(|e| e.key != key),
        RowAction::Comment(c) => {
            if let Some(e) = exclusions.iter_mut().find(|e| e.key == key) {
                e.comment = c.clone();
            }
        }
    }
    true
}

/// The collapsed list of excluded findings (`rows`) with the unused exclusions' count.
/// Returns true when "Remove unused" was clicked.
fn excluded_section(
    ui: &mut egui::Ui,
    id: &str,
    excluded: usize,
    unused: usize,
    rows: impl FnOnce(&mut egui::Ui),
) -> bool {
    let mut remove_unused = false;
    if excluded == 0 && unused == 0 {
        return false;
    }
    ui.add_space(4.0);
    egui::CollapsingHeader::new(RichText::new(format!("Excluded ({excluded})")).color(theme::text_dim()))
        .id_salt(id)
        .default_open(false)
        .show(ui, |ui| {
            rows(ui);
            if unused > 0 {
                ui.horizontal(|ui| {
                    let text = format!("{unused} unused (the finding no longer occurs)");
                    ui.label(RichText::new(text).color(theme::text_dim()));
                    remove_unused = ui.small_button("Remove unused").clicked();
                });
            }
        });
    remove_unused
}

const SHOWN: usize = 300;

fn more(ui: &mut egui::Ui, total: usize) {
    if total > SHOWN {
        hint(ui, &format!("… and {} more", total - SHOWN));
    }
}

fn schematic(ui: &mut egui::Ui, doc: &mut Document) -> Option<Show> {
    let problems = &doc.connectivity().problems;
    let count = |s: Severity| problems.iter().filter(|p| !p.excluded && p.severity == s).count();
    severity_heading(ui, "Schematic", count(Severity::Error), count(Severity::Warning));
    let shown: Vec<&editor_core::connectivity::Problem> = problems.iter().filter(|p| !p.excluded).collect();
    if shown.is_empty() {
        hint(
            ui,
            "No problems: every pin is connected or flagged, pin kinds go together, inputs are driven, references \
             are unique, nets are named consistently and buses are used correctly.",
        );
    }
    let erc = &doc.project().schematic.erc;
    let mut show = None;
    let mut edit: Option<(RowAction, String, String)> = None;
    let mut row = |ui: &mut egui::Ui, p: &editor_core::connectivity::Problem| match finding(
        ui,
        p.severity,
        erc.excluded(&p.key),
        &p.message,
        &p.key,
    ) {
        Some(RowAction::Show) => {
            // The sheet of the first object that is drawn on one; only its objects are selected.
            let s = &doc.project().schematic;
            let sheet = p.objects.iter().find_map(|&o| s.sheet_of(o)).or(s.root_sheet());
            let view = sheet.map_or(doc.root(), ViewId::Sheet);
            let objects = p.objects.iter().copied().filter(|&o| s.sheet_of(o) == sheet).collect();
            show = Some(Show::Objects(view, objects, p.pos));
        }
        Some(a) => edit = Some((a, p.key.clone(), p.message.clone())),
        None => {}
    };
    for p in shown.iter().take(SHOWN) {
        row(ui, p);
    }
    more(ui, shown.len());
    let excluded: Vec<&editor_core::connectivity::Problem> = problems.iter().filter(|p| p.excluded).collect();
    let unused = erc.exclusions.iter().filter(|e| !problems.iter().any(|p| p.key == e.key)).count();
    let remove_unused = excluded_section(ui, "erc_excluded", excluded.len(), unused, |ui| {
        for p in excluded.iter().take(SHOWN) {
            row(ui, p);
        }
    });
    let mut erc = erc.clone();
    let changed = match edit {
        Some((a, key, message)) => edit_exclusions(&mut erc.exclusions, &a, &key, &message),
        None if remove_unused => {
            erc.exclusions.retain(|e| problems.iter().any(|p| p.key == e.key));
            true
        }
        None => false,
    };
    if changed && let Err(e) = doc.execute(Command::SetErcSettings { erc }) {
        hint(ui, &e.to_string());
    }
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
    let shorts: Vec<&(Vec<String>, Point)> = live_shorts(doc).collect();
    heading(ui, "Board", shorts.len() + usize::from(!flagged.is_empty()));
    for (nets, at) in &shorts {
        if item(ui, &format!("Short: {} at {}", nets.join(", "), units::fmt_point(*at))) {
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
    if shorts.is_empty() && flagged.is_empty() {
        hint(ui, "No shorts or flagged copper.");
    }

    ui.add_space(8.0);
    ui.horizontal(|ui| {
        ui.label(RichText::new("Design rules").strong());
        if let Some(r) = doc.drc() {
            let count = |s: Severity| r.violations.iter().filter(|v| !v.excluded && v.severity == s).count();
            counts_label(ui, count(Severity::Error), count(Severity::Warning));
        }
        if ui.small_button("Run DRC").clicked() {
            doc.run_drc();
        }
        if doc.drc().is_some() && ui.small_button("Clear").clicked() {
            doc.clear_drc();
        }
    });
    let Some(report) = doc.drc() else {
        hint(
            ui,
            "Checks clearances, shorts, unrouted nets, keepouts, the board edge, courtyards, track widths and drills.",
        );
        return show;
    };
    let outdated = report.revision != doc.revision;
    let shown: Vec<&editor_core::drc::Violation> = report.violations.iter().filter(|v| !v.excluded).collect();
    let status = format!(
        "{} in {:.1} ms{}",
        if shown.is_empty() { "Clean" } else { "Found problems" },
        report.time.as_secs_f64() * 1e3,
        if outdated { " · board changed since" } else { "" }
    );
    ui.label(RichText::new(status).color(if outdated { theme::warn() } else { theme::text_dim() }).small());
    let drc = &doc.project().board.drc;
    let mut edit: Option<(RowAction, String, String)> = None;
    let mut row = |ui: &mut egui::Ui, v: &editor_core::drc::Violation| match finding(
        ui,
        v.severity,
        drc.excluded(&v.key),
        &v.message,
        &v.key,
    ) {
        Some(RowAction::Show) => show = Some(Show::Objects(ViewId::Board, v.objects.clone(), Some(v.pos))),
        Some(a) => edit = Some((a, v.key.clone(), v.message.clone())),
        None => {}
    };
    let mut kind: Option<(Severity, ViolationKind)> = None;
    for v in shown.iter().take(SHOWN) {
        if kind != Some((v.severity, v.kind)) {
            kind = Some((v.severity, v.kind));
            let n = shown.iter().filter(|w| (w.severity, w.kind) == (v.severity, v.kind)).count();
            ui.add_space(2.0);
            ui.label(RichText::new(format!("{} ({n})", v.kind.label())).color(theme::text_dim()));
        }
        row(ui, v);
    }
    more(ui, shown.len());
    let excluded: Vec<&editor_core::drc::Violation> = report.violations.iter().filter(|v| v.excluded).collect();
    let unused = drc.exclusions.iter().filter(|e| !report.violations.iter().any(|v| v.key == e.key)).count();
    let remove_unused = excluded_section(ui, "drc_excluded", excluded.len(), unused, |ui| {
        for v in excluded.iter().take(SHOWN) {
            row(ui, v);
        }
    });
    let mut drc = drc.clone();
    let changed = match edit {
        Some((a, key, message)) => edit_exclusions(&mut drc.exclusions, &a, &key, &message),
        None if remove_unused => {
            let report = doc.drc().expect("checked above");
            drc.exclusions.retain(|e| report.violations.iter().any(|v| v.key == e.key));
            true
        }
        None => false,
    };
    if changed && let Err(e) = doc.execute(Command::SetDrcSettings { drc }) {
        hint(ui, &e.to_string());
    }
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
