//! The Messages panel: schematic problems (ERC), board problems with the design rule
//! check results, library issues, and the last simulation attempt's problems, one tab each (the tabs sit in the panel's title
//! bar, with a badge of how each category is doing). Clicking a message shows its
//! objects.

use editor_core::Command;
use editor_core::copper::{CopperItem, Flag};
use editor_core::drc::short_key;
use egui::RichText;
use model::*;
use view::{Document, ViewId};

use crate::checks_ui;
use crate::dock::problem_key;
use crate::fields::hint;
use crate::icons::{self, Icon};
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

/// A tab of the panel.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(u8)]
pub enum Category {
    #[default]
    Schematic,
    Board,
    Libraries,
    Simulation,
}

impl Category {
    pub const ALL: [Category; 4] = [Category::Schematic, Category::Board, Category::Libraries, Category::Simulation];

    pub fn label(self) -> &'static str {
        match self {
            Category::Schematic => "Schematic",
            Category::Board => "Board",
            Category::Libraries => "Libraries",
            Category::Simulation => "Simulation",
        }
    }
}

/// How a category is doing: its findings that aren't excluded, by severity.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Status {
    pub errors: usize,
    pub warnings: usize,
}

impl Status {
    pub fn is_ok(self) -> bool {
        self.errors == 0 && self.warnings == 0
    }

    fn add(&mut self, severity: Severity) {
        match severity {
            Severity::Error => self.errors += 1,
            Severity::Warning => self.warnings += 1,
            Severity::Ignore => {}
        }
    }
}

/// The status of each category, in [`Category::ALL`] order. Board: live shorts (in the
/// short rule's severity), flagged copper (one warning) and the last DRC's findings;
/// libraries: their issues, as warnings; simulation: the last attempt's errors and warnings.
pub fn status(doc: &Document, libraries: &mut [LibraryEditor], sim: Option<&sim::Attempt>) -> [Status; 4] {
    let mut sch = Status::default();
    for p in doc.connectivity().problems.iter().filter(|p| !p.excluded) {
        sch.add(p.severity);
    }
    let mut board = Status::default();
    let short = doc.project().board.drc.severity(ViolationKind::Short);
    for _ in live_shorts(doc) {
        board.add(short);
    }
    if doc.copper().flagged().any(|c| matches!(c.item, CopperItem::Track(_) | CopperItem::Via(_))) {
        board.warnings += 1;
    }
    if let Some(r) = doc.drc() {
        for v in r.violations.iter().filter(|v| !v.excluded) {
            board.add(v.severity);
        }
    }
    let warnings = libraries.iter_mut().map(|l| l.issues().len()).sum();
    let mut simulation = Status::default();
    for p in sim.iter().flat_map(|a| &a.problems) {
        match p.level {
            sim::Level::Error => simulation.errors += 1,
            sim::Level::Warning => simulation.warnings += 1,
            sim::Level::Note => {}
        }
    }
    [sch, board, Status { errors: 0, warnings }, simulation]
}

/// The tab to show when the panel opens: `current`, unless it's fine and another
/// category has problems (errors first).
pub fn tab_to_open(current: Category, status: &[Status; 4]) -> Category {
    let of = |c: Category| status[c as usize];
    if !of(current).is_ok() {
        return current;
    }
    let with = |f: fn(Status) -> bool| Category::ALL.into_iter().find(|&c| f(of(c)));
    with(|s| s.errors > 0).or(with(|s| !s.is_ok())).unwrap_or(current)
}

/// About the width of the tab headers.
pub const TABS_WIDTH: f32 = 390.0;

/// The tab headers (in the title bar): each with its badge. Clicking one shows it.
pub fn tabs_ui(ui: &mut egui::Ui, current: &mut Category, status: &[Status; 4]) {
    ui.spacing_mut().item_spacing.x = 2.0;
    for (c, s) in Category::ALL.into_iter().zip(status) {
        if tab(ui, c.label(), *s, *current == c).clicked() {
            *current = c;
        }
    }
}

/// A tab header: a pill (filled when active) with the label and the badge.
fn tab(ui: &mut egui::Ui, label: &str, status: Status, active: bool) -> egui::Response {
    let font = egui::TextStyle::Button.resolve(ui.style());
    let text_color = if active { ui.visuals().strong_text_color() } else { theme::text_dim() };
    let galley = ui.painter().layout_no_wrap(label.to_string(), font, text_color);
    let badge_font = egui::FontId::proportional(11.0);
    let (badge_text, badge_color) = match status {
        s if s.is_ok() => (None, theme::success()),
        s if s.errors > 0 => (Some((s.errors + s.warnings).to_string()), theme::error()),
        s => (Some(s.warnings.to_string()), theme::warn()),
    };
    let badge_galley = badge_text.map(|t| ui.painter().layout_no_wrap(t, badge_font, badge_color));
    let badge_w = badge_galley.as_ref().map_or(12.0, |g| g.size().x.max(8.0) + 10.0);
    let (pad, gap, h) = (8.0, 6.0, 22.0);
    let size = egui::vec2(pad + galley.size().x + gap + badge_w + pad, h);
    let (rect, r) = ui.allocate_exact_size(size, egui::Sense::click());
    if ui.is_rect_visible(rect) {
        let v = ui.visuals();
        let fill = if active {
            v.widgets.inactive.weak_bg_fill
        } else if r.hovered() {
            v.widgets.inactive.weak_bg_fill.gamma_multiply(0.5)
        } else {
            egui::Color32::TRANSPARENT
        };
        ui.painter().rect_filled(rect, 6, fill);
        let text_pos = egui::pos2(rect.left() + pad, rect.center().y - galley.size().y / 2.0);
        ui.painter().galley(text_pos, galley.clone(), text_color);
        let badge = egui::Rect::from_min_size(
            egui::pos2(text_pos.x + galley.size().x + gap, rect.center().y - 8.0),
            egui::vec2(badge_w, 16.0),
        );
        match badge_galley {
            None => icons::paint(ui.painter(), badge.shrink(1.0), Icon::Check, badge_color),
            Some(g) => {
                ui.painter().rect_filled(badge, 8, badge_color.gamma_multiply(0.18));
                let pos = badge.center() - g.size() / 2.0;
                ui.painter().galley(pos, g, badge_color);
            }
        }
    }
    let tip = match status {
        s if s.is_ok() => "No problems".to_string(),
        s => [(s.errors, "error"), (s.warnings, "warning")]
            .into_iter()
            .filter(|(n, _)| *n > 0)
            .map(|(n, what)| format!("{n} {what}{}", if n == 1 { "" } else { "s" }))
            .collect::<Vec<_>>()
            .join(", "),
    };
    r.on_hover_text(tip)
}

/// Keys of all current messages (for the unseen-messages badge). Excluded findings don't
/// count.
pub fn keys(doc: &Document, libraries: &mut [LibraryEditor], sim: Option<&sim::Attempt>) -> Vec<u64> {
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
    let sim_problems = sim.iter().flat_map(|a| &a.problems).filter(|p| p.level != sim::Level::Note);
    keys.extend(sim_problems.map(|p| problem_key("sim", &p.message)));
    keys
}

/// The live board shorts the DRC settings don't ignore or exclude.
fn live_shorts(doc: &Document) -> impl Iterator<Item = &(Vec<String>, Point)> {
    let drc = &doc.project().board.drc;
    let shown = drc.severity(ViolationKind::Short) != Severity::Ignore;
    doc.copper().shorts.iter().filter(move |(nets, _)| shown && drc.excluded(&short_key(nets)).is_none())
}

/// The panel's tab `category`. Returns a clicked message's target.
pub fn ui(
    ui: &mut egui::Ui,
    category: Category,
    doc: &mut Document,
    libraries: &mut [LibraryEditor],
    sim: Option<&sim::Attempt>,
) -> Option<Show> {
    let scroll = egui::ScrollArea::vertical().id_salt(("messages", category as u8)).auto_shrink([false, false]);
    scroll
        .show(ui, |ui| match category {
            Category::Schematic => schematic(ui, doc),
            Category::Board => board(ui, doc),
            Category::Libraries => library_issues(ui, libraries),
            Category::Simulation => simulation(ui, doc, sim),
        })
        .inner
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
        .show_unindented(ui, |ui| {
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
    ui.label(RichText::new("Copper").strong());
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

/// The last simulation attempt's problems: the netlist's (clickable when they name
/// objects), then ngspice's errors and warnings.
fn simulation(ui: &mut egui::Ui, doc: &Document, attempt: Option<&sim::Attempt>) -> Option<Show> {
    let Some(a) = attempt else {
        hint(ui, "Problems of the last simulation run show here (Simulation panel, ⌘R).");
        return None;
    };
    let mut show = None;
    let outcome = match &a.outcome {
        sim::Outcome::Running => "running",
        sim::Outcome::Done => "done",
        sim::Outcome::NetlistErrors => "not started: the netlist has errors",
        sim::Outcome::Failed(_) => "ngspice failed",
        sim::Outcome::Cancelled => "stopped",
    };
    ui.label(RichText::new(format!("Run of {}: {outcome}", a.name)).color(theme::text_dim()));
    if a.problems.is_empty() {
        hint(ui, "No problems.");
    }
    for p in a.problems.iter().take(SHOWN) {
        let severity = match p.level {
            sim::Level::Error => Severity::Error,
            sim::Level::Warning => Severity::Warning,
            sim::Level::Note => Severity::Ignore,
        };
        let clicked = ui
            .horizontal(|ui| {
                checks_ui::icon(ui, severity, p.level == sim::Level::Note);
                item(ui, &p.message)
            })
            .inner;
        if clicked && !p.objects.is_empty() {
            let s = &doc.project().schematic;
            let sheet = p.objects.iter().find_map(|&o| s.sheet_of(o)).or(s.root_sheet());
            let view = sheet.map_or(doc.root(), ViewId::Sheet);
            let objects = p.objects.iter().copied().filter(|&o| s.sheet_of(o) == sheet).collect();
            show = Some(Show::Objects(view, objects, None));
        }
    }
    more(ui, a.problems.len());
    show
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_counts_findings_by_category() {
        let mut doc = Document::new(view::demo::demo_project());
        let mut libs = [];
        assert_eq!(status(&doc, &mut libs, None), [Status::default(); 4], "the demo is clean");
        doc.run_drc();
        let unrouted = doc.drc().unwrap().violations.len();
        assert_eq!(status(&doc, &mut libs, None)[1], Status { errors: unrouted, warnings: 0 }, "the DRC's findings");
        doc.clear_drc();

        // Without wires: unwired pins, and the tracks now join different nets.
        let ids = doc.project().schematic.wires.iter().map(|w| ObjectId::Wire(w.id)).collect();
        doc.execute(Command::Delete { ids }).unwrap();
        doc.update_connectivity(false);
        let [sch, board, lib, _] = status(&doc, &mut libs, None);
        assert!(!sch.is_ok() && !board.is_ok() && lib.is_ok());
    }

    #[test]
    fn opening_shows_a_category_with_problems() {
        let ok = Status::default();
        let warn = Status { errors: 0, warnings: 2 };
        let err = Status { errors: 1, warnings: 0 };
        assert_eq!(tab_to_open(Category::Schematic, &[ok, warn, err, ok]), Category::Libraries, "errors first");
        assert_eq!(tab_to_open(Category::Schematic, &[ok, warn, ok, ok]), Category::Board);
        assert_eq!(tab_to_open(Category::Board, &[err, warn, ok, ok]), Category::Board, "it has problems itself");
        assert_eq!(tab_to_open(Category::Board, &[ok, ok, ok, err]), Category::Simulation);
        assert_eq!(tab_to_open(Category::Board, &[ok, ok, ok, ok]), Category::Board);
    }
}
