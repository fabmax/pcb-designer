//! A list with a keyboard-movable selection (the KiCad import window's search results
//! and footprint lists): fixed-height rows selected by a click anywhere in them,
//! activated by a double click or Enter.
//!
//! The list takes keys while it has focus (a click on it gives it focus), like the
//! Project tree. A search field above it can pass its arrow keys and Enter on
//! ([`field_keys`]) so that typing, ↓ and Enter work without the mouse.

use egui::{Id, Key, Modifiers, Rect, Sense, Ui, vec2};
use view::list_nav::{ListKey, step};

use crate::theme;

/// Keys for a list, read from the list itself or from a search field.
#[derive(Default)]
pub struct Keys {
    pub moves: Vec<ListKey>,
    /// Enter (and Space where the list allows it): activate the selected row.
    pub activate: bool,
    pub escape: bool,
    /// The keys come from a focused search field: show the selection as focused.
    pub field_focused: bool,
}

/// What happened in a list this frame.
#[derive(Default)]
pub struct Output {
    /// A row to select (clicked or moved to).
    pub select: Option<usize>,
    /// A row to activate (double click or Enter).
    pub activate: Option<usize>,
    /// Esc while the list had focus.
    pub escape: bool,
}

/// Arrow keys and Enter typed into the search field `field` (consumed before the field
/// sees them; call before drawing it). Home and End stay with the field.
pub fn field_keys(ui: &mut Ui, field: Id) -> Keys {
    let mut keys = Keys::default();
    if !ui.memory(|m| m.has_focus(field)) {
        return keys;
    }
    keys.field_focused = true;
    let take = |key| ui.input_mut(|i| i.consume_key(Modifiers::NONE, key));
    for (key, k) in [
        (Key::ArrowUp, ListKey::Up),
        (Key::ArrowDown, ListKey::Down),
        (Key::PageUp, ListKey::PageUp),
        (Key::PageDown, ListKey::PageDown),
    ] {
        if take(key) {
            keys.moves.push(k);
        }
    }
    keys.activate = take(Key::Enter);
    keys
}

fn list_keys(ui: &mut Ui, space_activates: bool) -> Keys {
    let mut keys = Keys::default();
    let take = |key| ui.input_mut(|i| i.consume_key(Modifiers::NONE, key));
    for (key, k) in [
        (Key::ArrowUp, ListKey::Up),
        (Key::ArrowDown, ListKey::Down),
        (Key::Home, ListKey::Home),
        (Key::End, ListKey::End),
        (Key::PageUp, ListKey::PageUp),
        (Key::PageDown, ListKey::PageDown),
    ] {
        if take(key) {
            keys.moves.push(k);
        }
    }
    keys.activate = take(Key::Enter) | (space_activates && take(Key::Space));
    keys.escape = take(Key::Escape);
    keys
}

/// Row height of lists.
pub fn row_height(ui: &Ui) -> f32 {
    ui.spacing().interact_size.y + 4.0
}

/// A list of `len` rows in a scroll area at most `max_height` tall; `row` draws row `i`
/// into its rect (widgets in it, like buttons and check boxes, keep their clicks).
/// `extra` are keys from a search field.
#[allow(clippy::too_many_arguments)]
pub fn show(
    ui: &mut Ui,
    id: Id,
    len: usize,
    selected: Option<usize>,
    max_height: f32,
    space_activates: bool,
    extra: Keys,
    row: impl FnMut(&mut Ui, usize),
) -> Output {
    show_with_menu(ui, id, len, selected, max_height, space_activates, extra, row, None, &|_| None)
}

/// Fills the context menu of row `i`.
pub type RowMenu<'a> = dyn FnMut(&mut Ui, usize) + 'a;

/// [`show`] with a context menu on the rows and a background colour per row (`fill`) (`menu` fills it for row `i`).
#[allow(clippy::too_many_arguments)]
pub fn show_with_menu(
    ui: &mut Ui,
    id: Id,
    len: usize,
    selected: Option<usize>,
    max_height: f32,
    space_activates: bool,
    extra: Keys,
    mut row: impl FnMut(&mut Ui, usize),
    mut menu: Option<&mut RowMenu>,
    fill: &dyn Fn(usize) -> Option<egui::Color32>,
) -> Output {
    let mut out = Output::default();
    let focused = ui.memory(|m| m.has_focus(id));
    let mut keys = if focused { list_keys(ui, space_activates) } else { Keys::default() };
    keys.moves.extend(extra.moves);
    keys.activate |= extra.activate;
    let shown_focused = focused || extra.field_focused;
    out.escape = keys.escape;
    if focused {
        let filter = egui::EventFilter { horizontal_arrows: true, vertical_arrows: true, escape: true, tab: false };
        ui.memory_mut(|m| m.set_focus_lock_filter(id, filter));
    }

    let h = row_height(ui);
    let page = ((max_height / h) as usize).saturating_sub(1);
    let mut current = selected;
    for k in &keys.moves {
        current = step(len, current, *k, page);
    }
    let moved = current != selected;
    if moved {
        out.select = current;
    }
    if keys.activate {
        out.activate = current;
    }

    egui::ScrollArea::vertical().id_salt(id).max_height(max_height).auto_shrink([false, true]).show(ui, |ui| {
        let width = ui.available_width();
        let (area, _) = ui.allocate_exact_size(vec2(width, len as f32 * h), Sense::hover());
        // The list's keyboard focus: a widget under the rows.
        if ui.interact(area, id, Sense::click()).clicked() {
            ui.memory_mut(|m| m.request_focus(id));
        }
        let clip = ui.clip_rect();
        let visuals = ui.visuals().clone();
        for i in 0..len {
            let rect = Rect::from_min_size(area.min + vec2(0.0, i as f32 * h), vec2(width, h));
            let is_selected = current == Some(i);
            if is_selected && moved {
                ui.scroll_to_rect(rect, None);
            }
            if !clip.intersects(rect) {
                continue;
            }
            let r = ui.interact(rect, id.with(i), Sense::click());
            if let Some(c) = fill(i) {
                ui.painter().rect_filled(rect, 4.0, c);
            }
            if is_selected {
                let fill = visuals.selection.bg_fill;
                let fill = if shown_focused { fill } else { fill.gamma_multiply(0.5) };
                ui.painter().rect_filled(rect, 4.0, fill);
            } else if r.hovered() {
                ui.painter().rect_filled(rect, 4.0, visuals.widgets.hovered.weak_bg_fill);
            }
            if r.clicked() || r.double_clicked() {
                out.select = Some(i);
                ui.memory_mut(|m| m.request_focus(id));
            }
            if r.double_clicked() {
                out.activate = Some(i);
            }
            if let Some(menu) = menu.as_mut() {
                if r.secondary_clicked() {
                    out.select = Some(i);
                }
                r.context_menu(|ui| menu(ui, i));
            }
            let inner = rect.shrink2(vec2(4.0, 0.0));
            let layout = egui::Layout::left_to_right(egui::Align::Center);
            ui.scope_builder(egui::UiBuilder::new().max_rect(inner).layout(layout), |ui| {
                ui.set_clip_rect(inner.intersect(clip));
                theme::on_inset(ui);
                row(ui, i)
            });
        }
    });
    out
}

/// A row's name: cut with "…" when long, so lists never widen the window.
pub fn name(ui: &mut Ui, text: impl Into<egui::WidgetText>) {
    ui.add(egui::Label::new(text).truncate().selectable(false));
}

/// Dim text in a row (library names, notes).
pub fn dim(ui: &mut Ui, text: &str) {
    ui.add(egui::Label::new(egui::RichText::new(text).color(theme::text_dim())).selectable(false));
}
