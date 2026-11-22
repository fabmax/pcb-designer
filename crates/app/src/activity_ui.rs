//! The Agent activity panel: the agent's calls of this session, newest first. A row
//! shows when, the tool, what it did and whether it worked (and the undo step an edit
//! made); clicking it shows the arguments and the result.

use std::collections::VecDeque;
use std::time::Instant;

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

/// `url`: where the server listens, while it runs. `error`: why it didn't start.
pub fn ui(ui: &mut Ui, entries: &VecDeque<ActivityEntry>, url: Option<&str>, error: Option<&str>) -> Option<Act> {
    let mut act = None;
    let running = url.is_some();
    if let Some(url) = url {
        crate::agent_switch::url_ui(ui, url);
        ui.add_space(4.0);
    } else {
        crate::fields::summary(ui, "Agent access is off: turn it on with the switch above.");
        if let Some(e) = error {
            ui.label(RichText::new(e).color(theme::error()));
        }
        ui.add_space(4.0);
    }
    ui.horizontal(|ui| {
        let text = match entries.len() {
            0 if running => "No agent calls yet.".to_string(),
            0 => return,
            n => format!("{n} call(s) this session"),
        };
        crate::fields::summary(ui, &text);
        if !entries.is_empty() {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.small_button("Clear").clicked() {
                    act = Some(Act::Clear);
                }
            });
        }
    });
    ui.add_space(4.0);
    let now = Instant::now();
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        for (i, e) in entries.iter().enumerate().rev() {
            let id = ui.make_persistent_id(("activity", i, e.at));
            let state = egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id, false);
            let (_, header, _) = state
                .show_header(ui, |ui| {
                    let when = ago(now.duration_since(e.at).as_secs());
                    ui.label(RichText::new(when).color(theme::text_dim()).size(12.0));
                    ui.label(RichText::new(&e.tool).monospace());
                    ui.label(&e.summary);
                    match &e.result {
                        Some(Ok(_)) => ui.label(RichText::new("ok").color(theme::success())),
                        Some(Err(err)) => ui.label(RichText::new(first_line(err)).color(theme::error())),
                        None => ui.label(RichText::new("answered later").color(theme::text_dim())),
                    };
                    if let Some(undo) = &e.undo {
                        ui.label(RichText::new(format!("undo: {undo}")).color(theme::accent()).size(12.0));
                    }
                })
                .body(|ui| {
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
                });
            let _ = header;
        }
    });
    act
}

fn first_line(text: &str) -> String {
    let line = text.lines().next().unwrap_or_default();
    if line.chars().count() > 80 { format!("{}…", line.chars().take(80).collect::<String>()) } else { line.into() }
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
}
