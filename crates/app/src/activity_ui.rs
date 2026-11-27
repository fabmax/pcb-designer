//! The Agent activity panel: the agent's calls of this session, newest first. A row
//! shows when, the tool, what it did and whether it worked (and the undo step an edit
//! made); clicking it shows the arguments and the result.

use std::collections::VecDeque;
use web_time::Instant;

use egui::{RichText, Ui};

use crate::agent::ActivityEntry;
use crate::theme;

/// "12 s ago", "3 min ago", "2 h ago".
pub fn ago(seconds: u64) -> String {
    match seconds {
        0..5 => "now".into(),
        5..60 => format!("{seconds} s ago"),
        60..3600 => format!("{} min ago", seconds / 60),
        _ => format!("{} h ago", seconds / 3600),
    }
}

/// What the panel asks for.
pub enum Act {
    /// Clear the log.
    Clear,
}

/// `url`: where the server listens, while it runs (the header shows it and the number of
/// calls). `error`: why it didn't start.
pub fn ui(ui: &mut Ui, entries: &VecDeque<ActivityEntry>, url: Option<&str>, error: Option<&str>) -> Option<Act> {
    let mut act = None;
    let running = url.is_some();
    if let (false, Some(e)) = (running, error) {
        ui.label(RichText::new(e).color(theme::error()));
        ui.add_space(4.0);
    }
    if entries.is_empty() {
        return act;
    }
    let now = Instant::now();
    let cols = Columns::measure(ui, entries);
    // Column titles, past the rows' expand arrows.
    ui.horizontal(|ui| {
        ui.add_space(ui.spacing().indent);
        let title = |t: &str| RichText::new(t).color(theme::text_dim()).size(SMALL);
        cols.cell(ui, cols.when, title("When"));
        cols.cell(ui, cols.tool, title("Tool"));
        cols.cell(ui, cols.result, title("Result"));
        ui.label(title("What"));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.button("Clear").on_hover_text("Clear the list").clicked() {
                act = Some(Act::Clear);
            }
        });
    });
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        for (i, e) in entries.iter().enumerate().rev() {
            let id = ui.make_persistent_id(("activity", i, e.at));
            crate::fields::collapsing_header(
                ui,
                id,
                false,
                |ui| {
                    let when = ago(now.duration_since(e.at).as_secs());
                    cols.cell(ui, cols.when, RichText::new(when).color(theme::text_dim()).size(SMALL));
                    cols.cell(ui, cols.tool, RichText::new(&e.tool).monospace().size(TOOL));
                    let (status, color) = status(e);
                    cols.cell(ui, cols.result, RichText::new(status).color(color));
                    ui.add(egui::Label::new(what(ui, e)).truncate());
                },
                |ui| {
                    ui.label(RichText::new("Arguments").strong());
                    ui.label(RichText::new(&e.args).monospace().size(11.5));
                    match &e.result {
                        Some(Ok(text)) => {
                            ui.label(RichText::new("Result").strong());
                            ui.label(RichText::new(text).monospace().size(11.5));
                        }
                        Some(Err(err)) => {
                            ui.label(RichText::new("Error").strong());
                            ui.label(RichText::new(err).color(theme::error()));
                        }
                        None => {}
                    }
                },
            );
        }
    });
    act
}

/// Text size of the time and the column titles.
const SMALL: f32 = 12.0;

/// Text size of the tool names (monospace looks larger than the UI font at one size).
const TOOL: f32 = 12.5;

/// Widths of the fixed columns: when, tool and result; what fills the rest.
struct Columns {
    when: f32,
    tool: f32,
    result: f32,
}

impl Columns {
    /// Wide enough for every entry (and the titles), plus a gap.
    fn measure(ui: &Ui, entries: &VecDeque<ActivityEntry>) -> Columns {
        let width = |text: &str, font: egui::FontId| {
            ui.painter().layout_no_wrap(text.to_string(), font, egui::Color32::WHITE).size().x
        };
        let small = egui::FontId::proportional(SMALL);
        let body = egui::TextStyle::Body.resolve(ui.style());
        let mono = egui::FontId::monospace(TOOL);
        let gap = 12.0;
        let when = ["59 min ago", "59 s ago", "When"].iter().map(|t| width(t, small.clone())).fold(0.0, f32::max);
        let tool = entries.iter().map(|e| width(&e.tool, mono.clone())).fold(width("Tool", small.clone()), f32::max);
        let result = ["ok", "error", "later", "Result"].iter().map(|t| width(t, body.clone())).fold(0.0, f32::max);
        Columns { when: when + gap, tool: tool + gap, result: result + gap }
    }

    /// `text` in a cell `width` wide (left aligned).
    fn cell(&self, ui: &mut Ui, width: f32, text: RichText) {
        let height = ui.spacing().interact_size.y;
        let layout = egui::Layout::left_to_right(egui::Align::Center);
        ui.allocate_ui_with_layout(egui::vec2(width, height), layout, |ui| {
            ui.set_min_width(width);
            ui.add(egui::Label::new(text).truncate());
        });
    }
}

/// The result column: ok, error, or answered later.
fn status(e: &ActivityEntry) -> (&'static str, egui::Color32) {
    match &e.result {
        Some(Ok(_)) => ("ok", theme::success()),
        Some(Err(_)) => ("error", theme::error()),
        None => ("later", theme::text_dim()),
    }
}

/// The what column: the summary, an error's first line and the undo step it made.
fn what(ui: &Ui, e: &ActivityEntry) -> egui::text::LayoutJob {
    let mut job = egui::text::LayoutJob::default();
    let body = egui::TextStyle::Body.resolve(ui.style());
    let format = |color, font: egui::FontId| egui::TextFormat {
        font_id: font,
        color,
        valign: egui::Align::Center,
        ..Default::default()
    };
    job.append(&e.summary, 0.0, format(ui.visuals().text_color(), body.clone()));
    if let Some(Err(err)) = &e.result {
        job.append(&first_line(err), 8.0, format(theme::error(), body.clone()));
    }
    if let Some(undo) = &e.undo {
        job.append(&format!("undo: {undo}"), 8.0, format(theme::accent(), egui::FontId::proportional(SMALL)));
    }
    job
}

fn first_line(text: &str) -> String {
    let line = text.lines().next().unwrap_or_default();
    if line.chars().count() > 80 { format!("{}…", line.chars().take(80).collect::<String>()) } else { line.into() }
}

/// The number of calls, for the header: "No calls yet", "1 call this session", …
pub fn count(n: usize) -> String {
    match n {
        0 => "No calls yet".into(),
        1 => "1 call this session".into(),
        n => format!("{n} calls this session"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn times_ago() {
        assert_eq!(ago(2), "now");
        assert_eq!(ago(12), "12 s ago");
        assert_eq!(ago(190), "3 min ago");
        assert_eq!(ago(7300), "2 h ago");
    }

    #[test]
    fn call_counts() {
        assert_eq!(count(0), "No calls yet");
        assert_eq!(count(1), "1 call this session");
        assert_eq!(count(12), "12 calls this session");
    }
}
