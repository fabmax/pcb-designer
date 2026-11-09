//! The dialog asking for a bus's name after drawing it, or for the signal of a wire
//! leaving a bus entry ([`SchPrompt`]). Confirming adds the label.

use egui::RichText;
use view::sch_tools::SchPrompt;

use crate::fields::hint;
use crate::theme;

pub struct NamePrompt {
    pub prompt: SchPrompt,
    text: String,
    focused: bool,
    /// The suggested signal was scrolled into view.
    scrolled: bool,
}

impl NamePrompt {
    pub fn new(prompt: SchPrompt) -> Self {
        // Suggest the first member that no label names yet.
        let text = match &prompt {
            SchPrompt::BusName { .. } => String::new(),
            SchPrompt::EntryNet { members, uses, .. } => {
                members.iter().zip(uses).find(|(_, n)| **n == 0).map(|(m, _)| m.clone()).unwrap_or_default()
            }
        };
        NamePrompt { prompt, text, focused: false, scrolled: false }
    }

    /// Show the dialog. Returns `Some(Some(name))` when confirmed, `Some(None)` when
    /// skipped, `None` while open.
    pub fn ui(&mut self, ctx: &egui::Context) -> Option<Option<String>> {
        let mut outcome = None;
        let title = match self.prompt {
            SchPrompt::BusName { .. } => "Name the bus",
            SchPrompt::EntryNet { .. } => "Name the signal",
        };
        let modal = theme::dialog(ctx, egui::Id::unique("name_prompt"), title, 380.0, |ui| {
            let valid = self.prompt.check(&self.text);
            match &self.prompt {
                SchPrompt::BusName { .. } => {
                    ui.label("A bus groups signals. Its name lists them:");
                    ui.add_space(2.0);
                    egui::Grid::new("bus_rules").num_columns(2).spacing([12.0, 2.0]).show(ui, |ui| {
                        for (syntax, members) in [
                            ("D[0..7]", "D0, D1, … D7"),
                            ("{SDA SCL}", "SDA, SCL"),
                            ("I2C{SDA SCL}", "I2C.SDA, I2C.SCL"),
                            ("{A[0..3] WE}", "A0 … A3, WE"),
                        ] {
                            ui.monospace(syntax);
                            ui.label(RichText::new(members).color(theme::text_dim()));
                            ui.end_row();
                        }
                    });
                    ui.add_space(4.0);
                    hint(ui, "Signals leave the bus through bus entries and connect by labels with these names.");
                }
                SchPrompt::EntryNet { bus, members, uses, .. } => {
                    ui.label(format!("This wire leaves bus {bus}. Label it with one of its signals:"));
                    ui.add_space(6.0);
                    // Wide buses scroll instead of growing the dialog; at most MAX_SHOWN
                    // buttons (bus names can list far more), the rest can be typed.
                    const MAX_SHOWN: usize = 1024;
                    egui::Frame::NONE
                        .fill(theme::panel_dark())
                        .corner_radius(6)
                        .inner_margin(egui::Margin::same(6))
                        .show(ui, |ui| {
                            egui::ScrollArea::vertical().max_height(150.0).auto_shrink([false, true]).show(ui, |ui| {
                                ui.horizontal_wrapped(|ui| {
                                    for (m, &n) in members.iter().zip(uses).take(MAX_SHOWN) {
                                        let selected = self.text.trim() == m;
                                        let r = ui.selectable_label(selected, signal_text(ui, m, n)).on_hover_text(
                                            match n {
                                                0 => "No wire has this name yet".to_string(),
                                                1 => "1 labelled wire has this name".to_string(),
                                                n => format!("{n} labelled wires have this name"),
                                            },
                                        );
                                        if selected && !self.scrolled {
                                            r.scroll_to_me(Some(egui::Align::Center));
                                            self.scrolled = true;
                                        }
                                        if r.clicked() {
                                            self.text = m.clone();
                                        }
                                        if r.double_clicked() {
                                            outcome = Some(Some(m.clone()));
                                        }
                                    }
                                });
                            });
                        });
                    ui.add_space(4.0);
                    let more = members.len().saturating_sub(MAX_SHOWN);
                    let text = if more > 0 {
                        format!(
                            "{} signals ({more} not shown): ×N counts the wires already labelled with \
                                 a signal; type a name to use any of them.",
                            members.len()
                        )
                    } else {
                        "×N counts the wires already labelled with a signal.".to_string()
                    };
                    hint(ui, &text);
                }
            }
            ui.add_space(8.0);
            let edit =
                ui.add(egui::TextEdit::singleline(&mut self.text).hint_text("Name").desired_width(f32::INFINITY));
            if !self.focused {
                edit.request_focus();
                self.focused = true;
            }
            let entered = edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            match &valid {
                Ok(()) => {
                    if let SchPrompt::BusName { .. } = self.prompt
                        && let Some(m) = editor_core::bus::members(&self.text)
                    {
                        let shown: Vec<&str> = m.iter().take(12).map(String::as_str).collect();
                        let more = if m.len() > 12 { format!(" … ({} in all)", m.len()) } else { String::new() };
                        hint(ui, &format!("Signals: {}{more}", shown.join(", ")));
                    }
                }
                Err(e) if !self.text.trim().is_empty() => {
                    ui.label(RichText::new(e).color(theme::warn()));
                }
                Err(_) => {}
            }
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                let add = ui.add_enabled(valid.is_ok(), egui::Button::new("Add label"));
                if add.clicked() || (entered && valid.is_ok()) {
                    outcome = Some(Some(self.text.trim().to_string()));
                }
                if ui.button("Skip").clicked() {
                    outcome = Some(None);
                }
            });
        });
        if outcome.is_none() && modal.should_close() {
            outcome = Some(None);
        }
        outcome
    }
}

/// A signal name with a small `×n` badge: how many labelled wires use it.
fn signal_text(ui: &egui::Ui, name: &str, uses: usize) -> egui::text::LayoutJob {
    let mut job = egui::text::LayoutJob::default();
    let color = ui.visuals().text_color();
    let mono = egui::TextStyle::Monospace.resolve(ui.style());
    job.append(name, 0.0, egui::TextFormat::simple(mono, color));
    let badge = egui::FontId::proportional(10.0);
    let dim = if uses == 0 { theme::text_dim().gamma_multiply(0.7) } else { theme::text_dim() };
    let format = egui::TextFormat { valign: egui::Align::Center, ..egui::TextFormat::simple(badge, dim) };
    job.append(&format!("×{uses}"), 3.0, format);
    job
}
