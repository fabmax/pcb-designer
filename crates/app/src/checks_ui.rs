//! Settings of the checks: the Schematic setup panel (ERC rule severities and the pin
//! conflict matrix) and the DRC severities section of Board setup. Severity icons for the
//! Messages panel.

use editor_core::Command;
use egui::{RichText, Sense, vec2};
use model::*;
use view::Document;

use crate::fields::{grid, hint, label};
use crate::theme;

/// The colour of a severity.
pub fn color(s: Severity) -> egui::Color32 {
    match s {
        Severity::Error => theme::error(),
        Severity::Warning => theme::warn(),
        Severity::Ignore => theme::success(),
    }
}

/// A small severity icon: a dot for errors, a triangle for warnings, a hollow dot for
/// excluded findings.
pub fn icon(ui: &mut egui::Ui, s: Severity, excluded: bool) {
    let (rect, _) = ui.allocate_exact_size(vec2(10.0, 14.0), Sense::hover());
    let c = rect.center();
    let col = if excluded { theme::text_dim() } else { color(s) };
    let p = ui.painter();
    if excluded {
        p.circle_stroke(c, 3.5, egui::Stroke::new(1.2, col));
    } else if s == Severity::Error {
        p.circle_filled(c, 4.0, col);
    } else {
        let pts = vec![c + vec2(0.0, -4.5), c + vec2(4.5, 3.5), c + vec2(-4.5, 3.5)];
        p.add(egui::Shape::convex_polygon(pts, col, egui::Stroke::NONE));
    }
}

fn severity_combo(ui: &mut egui::Ui, salt: &str, current: Severity) -> Option<Severity> {
    let mut out = None;
    egui::ComboBox::from_id_salt(salt).width(70.0).selected_text(current.label()).show_ui(ui, |ui| {
        for s in Severity::ALL {
            if ui.selectable_label(s == current, s.label()).clicked() && s != current {
                out = Some(s);
            }
        }
    });
    out
}

fn run(doc: &mut Document, cmd: Command, message: &mut Option<String>) {
    if let Err(e) = doc.execute(cmd) {
        *message = Some(e.to_string());
    }
}

/// Short column names of the pin kinds in the matrix.
fn short(k: PinKind) -> &'static str {
    match k {
        PinKind::Input => "In",
        PinKind::Output => "Out",
        PinKind::Bidirectional => "Bi",
        PinKind::TriState => "3S",
        PinKind::Passive => "Pas",
        PinKind::PowerIn => "PwI",
        PinKind::PowerOut => "PwO",
        PinKind::OpenCollector => "OC",
        PinKind::NotConnected => "NC",
    }
}

/// The Schematic setup panel.
pub fn schematic_setup(ui: &mut egui::Ui, doc: &mut Document, message: &mut Option<String>) {
    let mut erc = doc.project().schematic.erc.clone();
    let before = erc.clone();
    ui.label(RichText::new("Electrical rules").strong());
    ui.add_space(2.0);
    grid(ui, "erc_rules", |ui| {
        for rule in ErcRule::ALL {
            label(ui, rule.label());
            if rule == ErcRule::PinConflict {
                ui.label(RichText::new("matrix").color(theme::text_dim()));
            } else if let Some(s) = severity_combo(ui, rule.code(), erc.severity(rule)) {
                erc.set_severity(rule, s);
            }
            ui.end_row();
        }
    });
    ui.add_space(10.0);
    ui.label(RichText::new("Pin conflict matrix").strong());
    hint(ui, "What two pin kinds on one net are: click a cell to go from OK to warning to error.");
    ui.add_space(4.0);
    matrix(ui, &mut erc);
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        let defaults = erc.severities.is_empty() && erc.matrix.is_empty();
        if ui.add_enabled(!defaults, egui::Button::new("Reset to defaults")).clicked() {
            erc.severities.clear();
            erc.matrix.clear();
        }
    });
    if erc != before {
        run(doc, Command::SetErcSettings { erc }, message);
    }
    ui.add_space(6.0);
    hint(
        ui,
        "The defaults follow KiCad. Ignored rules aren't reported; excluded findings (right-click one in \
         Messages) stay listed apart.",
    );
}

/// The lower triangle of the matrix, cells cycling OK → warning → error.
fn matrix(ui: &mut egui::Ui, erc: &mut ErcSettings) {
    let cell = 25.0;
    let label_w = 32.0;
    let n = PIN_KINDS.len();
    let (rect, _) = ui.allocate_exact_size(vec2(label_w + cell * n as f32, cell * (n as f32 + 1.0)), Sense::hover());
    let origin = rect.min + vec2(label_w, cell);
    let font = egui::FontId::proportional(10.0);
    for (j, &b) in PIN_KINDS.iter().enumerate() {
        let c = origin + vec2(cell * (j as f32 + 0.5), -cell * 0.5);
        ui.painter().text(c, egui::Align2::CENTER_CENTER, short(b), font.clone(), theme::text_dim());
    }
    for (i, &a) in PIN_KINDS.iter().enumerate() {
        let y = origin.y + cell * i as f32;
        ui.painter().text(
            egui::pos2(rect.min.x + label_w - 4.0, y + cell * 0.5),
            egui::Align2::RIGHT_CENTER,
            short(a),
            font.clone(),
            theme::text_dim(),
        );
        for (j, &b) in PIN_KINDS.iter().enumerate().take(i + 1) {
            let r = egui::Rect::from_min_size(egui::pos2(origin.x + cell * j as f32, y), vec2(cell, cell)).shrink(1.5);
            let id = ui.make_persistent_id(("pin_matrix", i, j));
            let resp = ui.interact(r, id, Sense::click());
            let s = erc.pin_pair(a, b);
            let fill = match s {
                Severity::Ignore => theme::panel_dark(),
                s => color(s).gamma_multiply(0.45),
            };
            let fill = if resp.hovered() { fill.gamma_multiply(1.4) } else { fill };
            ui.painter().rect_filled(r, 3.0, fill);
            let text = match s {
                Severity::Error => "E",
                Severity::Warning => "W",
                Severity::Ignore => "",
            };
            ui.painter().text(r.center(), egui::Align2::CENTER_CENTER, text, font.clone(), ui.visuals().text_color());
            if s != default_pin_pair(a, b) {
                ui.painter().rect_stroke(r, 3.0, egui::Stroke::new(1.0, theme::accent()), egui::StrokeKind::Inside);
            }
            let resp = resp.on_hover_text(format!(
                "{} – {}: {}{}",
                a.label(),
                b.label(),
                if s == Severity::Ignore { "OK" } else { s.label() },
                if s != default_pin_pair(a, b) { " (changed)" } else { "" }
            ));
            if resp.clicked() {
                let next = match s {
                    Severity::Ignore => Severity::Warning,
                    Severity::Warning => Severity::Error,
                    Severity::Error => Severity::Ignore,
                };
                erc.set_pin_pair(a, b, next);
            }
        }
    }
}

/// The DRC severities (a section of Board setup).
pub fn drc_severities(ui: &mut egui::Ui, doc: &mut Document, message: &mut Option<String>) {
    let mut drc = doc.project().board.drc.clone();
    let before = drc.clone();
    ui.label(RichText::new("Design rule check").strong());
    ui.add_space(2.0);
    grid(ui, "drc_rules", |ui| {
        for kind in ViolationKind::ALL {
            label(ui, kind.label());
            if let Some(s) = severity_combo(ui, kind.code(), drc.severity(kind)) {
                drc.set_severity(kind, s);
            }
            ui.end_row();
        }
    });
    if drc != before {
        run(doc, Command::SetDrcSettings { drc }, message);
    }
}
