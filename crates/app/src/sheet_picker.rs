//! "Move to sheet…": the dialog choosing the sheet the selection moves to.

use model::{Id, ObjectId, Project};

use crate::fields::hint;
use crate::theme;

pub struct SheetPicker {
    /// The schematic objects to move.
    pub ids: Vec<ObjectId>,
    /// Sheets offered: id and display path.
    sheets: Vec<(Id, String)>,
    filter: String,
    selected: usize,
    focused: bool,
}

impl SheetPicker {
    /// A picker for moving `ids` off `from`: every sheet but `from` and, for selected sheet
    /// symbols, their own subtrees. `None` if there is nowhere to go.
    pub fn new(project: &Project, ids: Vec<ObjectId>, from: Id) -> Option<SheetPicker> {
        let sch = &project.schematic;
        let excluded: Vec<Id> = ids
            .iter()
            .filter_map(|id| if let ObjectId::Sheet(s) = id { Some(sch.sheet_and_descendants(*s)) } else { None })
            .flatten()
            .chain([from])
            .collect();
        let mut sheets: Vec<(Id, String)> =
            sch.page_order().into_iter().filter(|s| !excluded.contains(s)).map(|s| (s, sch.display_path(s))).collect();
        sheets.sort_by(|a, b| a.1.cmp(&b.1));
        (!sheets.is_empty()).then_some(SheetPicker { ids, sheets, filter: String::new(), selected: 0, focused: false })
    }

    /// Show the dialog. Returns `Some(Some(sheet))` when chosen, `Some(None)` when
    /// cancelled, `None` while open.
    pub fn ui(&mut self, ctx: &egui::Context) -> Option<Option<Id>> {
        let mut outcome = None;
        let n = self.ids.len();
        let title = if n == 1 { "Move 1 object to sheet".to_string() } else { format!("Move {n} objects to sheet") };
        let modal = theme::dialog(ctx, egui::Id::unique("sheet_picker"), &title, 360.0, |ui| {
            let edit = crate::fields::clearable(ui, &mut self.filter, |e| {
                e.hint_text("Filter sheets").desired_width(f32::INFINITY)
            });
            if !self.focused {
                edit.request_focus();
                self.focused = true;
            }
            let needle = self.filter.trim().to_lowercase();
            let shown: Vec<&(Id, String)> =
                self.sheets.iter().filter(|(_, p)| p.to_lowercase().contains(&needle)).collect();
            if edit.changed() {
                self.selected = 0;
            }
            let (down, up, enter) = ui.input(|i| {
                (
                    i.key_pressed(egui::Key::ArrowDown),
                    i.key_pressed(egui::Key::ArrowUp),
                    i.key_pressed(egui::Key::Enter),
                )
            });
            if down {
                self.selected = (self.selected + 1).min(shown.len().saturating_sub(1));
            }
            if up {
                self.selected = self.selected.saturating_sub(1);
            }
            ui.add_space(6.0);
            egui::ScrollArea::vertical().max_height(240.0).auto_shrink([false, true]).show(ui, |ui| {
                for (i, (id, path)) in shown.iter().enumerate() {
                    let r = ui.add_sized(
                        [ui.available_width(), 22.0],
                        egui::Button::selectable(i == self.selected, path.as_str()),
                    );
                    if r.clicked() {
                        self.selected = i;
                    }
                    if r.double_clicked() {
                        outcome = Some(Some(*id));
                    }
                }
                if shown.is_empty() {
                    hint(ui, "No sheet matches.");
                }
            });
            if enter && let Some((id, _)) = shown.get(self.selected) {
                outcome = Some(Some(*id));
            }
            ui.add_space(6.0);
            hint(ui, "They keep their place on the page and follow the pointer on the new sheet until you click.");
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                let chosen = shown.get(self.selected).map(|(id, _)| *id);
                if ui.add_enabled(chosen.is_some(), egui::Button::new("Move")).clicked() {
                    outcome = Some(chosen);
                }
                if ui.button("Cancel").clicked() {
                    outcome = Some(None);
                }
            });
        });
        if modal.should_close() && outcome.is_none() {
            outcome = Some(None);
        }
        outcome
    }
}
