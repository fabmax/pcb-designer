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
}

impl NamePrompt {
    pub fn new(prompt: SchPrompt) -> Self {
        // Suggest the first member that isn't a net yet.
        let text = match &prompt {
            SchPrompt::BusName { .. } => String::new(),
            SchPrompt::EntryNet { members, used, .. } => {
                members.iter().find(|m| !used.contains(m)).cloned().unwrap_or_default()
            }
        };
        NamePrompt { prompt, text, focused: false }
    }

    /// Show the dialog. Returns `Some(Some(name))` when confirmed, `Some(None)` when
    /// skipped, `None` while open.
    pub fn ui(&mut self, ctx: &egui::Context) -> Option<Option<String>> {
        let mut outcome = None;
        let modal = egui::Modal::new(egui::Id::unique("name_prompt")).show(ctx, |ui| {
            ui.set_width(380.0);
            let valid = self.prompt.check(&self.text);
            match &self.prompt {
                SchPrompt::BusName { .. } => {
                    ui.heading("Name the bus");
                    ui.add_space(4.0);
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
                SchPrompt::EntryNet { bus, members, used, .. } => {
                    ui.heading("Name the signal");
                    ui.add_space(4.0);
                    ui.label(format!("This wire leaves bus {bus}. Label it with one of its signals:"));
                    ui.add_space(4.0);
                    ui.horizontal_wrapped(|ui| {
                        for m in members.iter().take(256) {
                            let text = if used.contains(m) {
                                RichText::new(m).monospace().color(theme::text_dim())
                            } else {
                                RichText::new(m).monospace()
                            };
                            let r = ui.selectable_label(self.text.trim() == m, text);
                            let r = if used.contains(m) { r.on_hover_text("Already used") } else { r };
                            if r.clicked() {
                                self.text = m.clone();
                            }
                            if r.double_clicked() {
                                outcome = Some(Some(m.clone()));
                            }
                        }
                    });
                    if !used.is_empty() {
                        hint(ui, "Dimmed signals are already used elsewhere.");
                    }
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
