//! Form widgets that commit edits the way the properties panels do: text fields on Enter
//! or focus loss, dragged values continuously but as one undo group.

use egui::RichText;
use model::*;
use rustc_hash::FxHashMap;
use view::interaction::new_group;

use crate::theme;

#[derive(Default)]
pub struct Fields {
    group: Option<u64>,
    /// A grouped widget was dragged or edited this frame.
    active: bool,
    texts: FxHashMap<egui::Id, String>,
}

impl Fields {
    pub fn begin_frame(&mut self) {
        self.active = false;
    }

    /// Returns true when an edit group just ended (the caller closes it in its document).
    pub fn end_frame(&mut self) -> bool {
        !self.active && self.group.take().is_some()
    }

    /// Undo group for continuous edits.
    pub fn group(&mut self) -> u64 {
        *self.group.get_or_insert_with(new_group)
    }

    pub fn forget(&mut self) {
        self.texts.clear();
    }

    /// Text field committed on Enter / focus loss. Returns the new text if it changed.
    pub fn text(
        &mut self,
        ui: &mut egui::Ui,
        salt: impl std::hash::Hash + std::fmt::Debug,
        current: &str,
    ) -> Option<String> {
        self.text_sized(ui, salt, current, f32::INFINITY)
    }

    pub fn text_sized(
        &mut self,
        ui: &mut egui::Ui,
        salt: impl std::hash::Hash + std::fmt::Debug,
        current: &str,
        width: f32,
    ) -> Option<String> {
        self.text_hint(ui, salt, current, width, "")
    }

    /// A text field with a greyed hint while empty, `width` points wide (in a [`grid`]:
    /// filling the value column).
    pub fn text_hint(
        &mut self,
        ui: &mut egui::Ui,
        salt: impl std::hash::Hash + std::fmt::Debug,
        current: &str,
        width: f32,
        hint: &str,
    ) -> Option<String> {
        let id = ui.make_persistent_id(salt);
        let focused = ui.memory(|m| m.has_focus(id));
        let buf = self.texts.entry(id).or_insert_with(|| current.to_string());
        if !focused {
            buf.clear();
            buf.push_str(current);
        }
        let width = value_width(ui).unwrap_or(width.min(ui.available_width()));
        let r = ui.add(egui::TextEdit::singleline(buf).id(id).desired_width(width).hint_text(hint));
        (r.lost_focus() && buf != current).then(|| buf.clone())
    }

    /// Monospace multi-line text, committed on focus loss or ⌘Enter. Returns the new
    /// text if it changed.
    pub fn multiline(
        &mut self,
        ui: &mut egui::Ui,
        salt: impl std::hash::Hash + std::fmt::Debug,
        current: &str,
        rows: usize,
    ) -> Option<String> {
        let id = ui.make_persistent_id(salt);
        let focused = ui.memory(|m| m.has_focus(id));
        let buf = self.texts.entry(id).or_insert_with(|| current.to_string());
        if !focused {
            buf.clear();
            buf.push_str(current);
        }
        let r =
            ui.add(egui::TextEdit::multiline(buf).id(id).code_editor().desired_rows(rows).desired_width(f32::INFINITY));
        let submit = r.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter) && i.modifiers.command);
        if submit {
            ui.memory_mut(|m| m.surrender_focus(id));
        }
        ((r.lost_focus() || submit) && buf != current).then(|| buf.clone())
    }

    /// A category: a text field with a menu of `suggestions` (from [`categories`]).
    /// Returns the new category if it changed.
    pub fn category(
        &mut self,
        ui: &mut egui::Ui,
        salt: impl std::hash::Hash + std::fmt::Debug,
        current: &str,
        suggestions: &[String],
    ) -> Option<String> {
        let mut out = None;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            // The menu button after the field; in a grid the two fill the value column.
            let cap = if value_width(ui).is_some() { f32::INFINITY } else { 240.0 };
            let width = (ui.available_width() - 34.0).min(cap);
            out = self.text_sized(ui, salt, current, width);
            ui.add_enabled_ui(!suggestions.is_empty(), |ui| {
                ui.menu_button("▼", |ui| {
                    for s in suggestions {
                        if ui.selectable_label(s == current, s).clicked() {
                            out = Some(s.clone());
                        }
                    }
                    if !current.is_empty() {
                        ui.separator();
                        if ui.button("None").clicked() {
                            out = Some(String::new());
                        }
                    }
                })
                .response
                .on_hover_text("Categories in use");
            });
        });
        out.map(|c| c.trim().to_string()).filter(|c| c != current)
    }

    /// Length in the display unit, edited by dragging or typing; at least `min` mm.
    /// Returns the new value in nm.
    pub fn length(&mut self, ui: &mut egui::Ui, value: Coord, min: f64) -> Option<Coord> {
        // Lengths are below a kilometre.
        let limit = mm(1e6);
        let min = if min > -1e6 { mm(min) } else { -limit };
        let (r, v) = crate::units::drag(ui, value, min, limit);
        self.active |= r.dragged() || r.has_focus();
        v
    }

    /// [`length`](Self::length) `width` points wide (NaN: its own width).
    fn length_sized(&mut self, ui: &mut egui::Ui, value: Coord, min: f64, width: f32) -> Option<Coord> {
        sized(ui, width, |ui| self.length(ui, value, min))
    }

    pub fn number(
        &mut self,
        ui: &mut egui::Ui,
        value: f64,
        range: std::ops::RangeInclusive<f64>,
        speed: f64,
    ) -> Option<f64> {
        let mut v = value;
        let r = fill(ui, egui::DragValue::new(&mut v).speed(speed).range(range).max_decimals(3));
        self.active |= r.dragged() || r.has_focus();
        r.changed().then_some(v)
    }

    pub fn point(&mut self, ui: &mut egui::Ui, p: Point) -> Option<Point> {
        pair(ui, |ui, w| {
            let x = self.length_sized(ui, p.x, f64::MIN, w);
            let y = self.length_sized(ui, p.y, f64::MIN, w);
            (x.is_some() || y.is_some()).then(|| Point::new(x.unwrap_or(p.x), y.unwrap_or(p.y)))
        })
    }

    pub fn size(&mut self, ui: &mut egui::Ui, s: Size) -> Option<Size> {
        pair(ui, |ui, half| {
            let w = self.length_sized(ui, s.w, 0.01, half);
            let h = self.length_sized(ui, s.h, 0.01, half);
            (w.is_some() || h.is_some()).then(|| Size { w: w.unwrap_or(s.w), h: h.unwrap_or(s.h) })
        })
    }
}

/// Size of a section title (bold).
const SECTION_SIZE: f32 = theme::TITLE_SIZE;
/// Size of the subject line (the selection, the tool): a section title's.
const SUBJECT_SIZE: f32 = SECTION_SIZE;
/// Most of a panel's width its label column takes.
const LABEL_SHARE: f32 = 0.45;

/// The form layout of the panel being drawn: one label column for all its grids.
struct Scope {
    /// Label column width (from the last frame's widest label; 0: not known yet).
    width: f32,
    /// Widest label this frame.
    widest: f32,
    cap: f32,
}

thread_local! {
    static SCOPE: std::cell::RefCell<Option<Scope>> = const { std::cell::RefCell::new(None) };
    /// Inside a [`grid`]: value widgets fill the value column.
    static IN_GRID: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Draw a panel (or dialog) whose grids share one label column: as wide as its widest
/// label, at most [`LABEL_SHARE`] of the width (longer labels wrap).
pub fn form_scope<R>(
    ui: &mut egui::Ui,
    salt: impl std::hash::Hash + std::fmt::Debug,
    add: impl FnOnce(&mut egui::Ui) -> R,
) -> R {
    let key = egui::Id::unique(("form_scope", salt));
    let last: Option<f32> = ui.data(|d| d.get_temp(key));
    let cap = (ui.available_width() * LABEL_SHARE).max(60.0);
    let outer = SCOPE.replace(Some(Scope { width: last.unwrap_or(0.0).min(cap), widest: 0.0, cap }));
    let r = add(ui);
    let scope = SCOPE.replace(outer).expect("set above");
    let widest = scope.widest.min(scope.cap).ceil();
    if last != Some(widest) {
        ui.data_mut(|d| d.insert_temp(key, widest));
        ui.ctx().request_repaint();
    }
    r
}

/// The selection or tool a panel is about (Properties, Tool settings): a section title
/// with a dim `subtitle` and help behind an ⓘ.
pub fn subject(ui: &mut egui::Ui, title: &str, subtitle: &str, info: Option<&str>) {
    ui.horizontal_wrapped(|ui| {
        theme::bold_label(ui, title, SUBJECT_SIZE, ui.visuals().strong_text_color());
        if !subtitle.is_empty() {
            ui.label(RichText::new(subtitle).color(theme::text_dim()));
        }
        if let Some(t) = info {
            info_button(ui, t);
        }
    });
    ui.add_space(4.0);
}

/// Room for a [`subject`] line drawn after what follows it ([`draw_subject`]), when
/// that decides its help.
pub fn reserve_subject(ui: &mut egui::Ui) -> egui::Rect {
    let h = SUBJECT_SIZE.max(ui.spacing().interact_size.y);
    let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), h), egui::Sense::hover());
    ui.add_space(4.0 + ui.spacing().item_spacing.y);
    rect
}

/// The [`subject`] line in the room [`reserve_subject`] kept.
pub fn draw_subject(ui: &mut egui::Ui, rect: egui::Rect, title: &str, info: Option<&str>) {
    let layout = egui::Layout::left_to_right(egui::Align::Center);
    ui.scope_builder(egui::UiBuilder::new().max_rect(rect).layout(layout), |ui| {
        theme::bold_label(ui, title, SUBJECT_SIZE, ui.visuals().strong_text_color());
        if let Some(t) = info {
            info_button(ui, t);
        }
    });
}

/// Space between a section and what's above it (none at the top of a panel).
fn space_above_section(ui: &mut egui::Ui) {
    if ui.min_rect().height() > 1.0 {
        ui.add_space(10.0);
    }
}

/// A section title, with help behind an ⓘ.
pub fn section(ui: &mut egui::Ui, title: &str, info: Option<&str>) {
    space_above_section(ui);
    ui.horizontal(|ui| {
        theme::bold_label(ui, title, SECTION_SIZE, ui.visuals().strong_text_color());
        if let Some(t) = info {
            info_button(ui, t);
        }
    });
    ui.add_space(2.0);
}

/// A section title with a dim note after it ("Sheet  /sub", "Title block  all sheets").
pub fn section_note(ui: &mut egui::Ui, title: &str, note: &str) {
    space_above_section(ui);
    ui.horizontal(|ui| {
        theme::bold_label(ui, title, SECTION_SIZE, ui.visuals().strong_text_color());
        if !note.is_empty() {
            ui.label(RichText::new(note).color(theme::text_dim()));
        }
    });
    ui.add_space(2.0);
}

/// An ⓘ showing `text` while hovered; a click keeps it open until a click elsewhere.
pub fn info_button(ui: &mut egui::Ui, text: &str) {
    use crate::icons::{self, Icon};
    let (rect, r) = ui.allocate_exact_size(egui::vec2(18.0, 18.0), egui::Sense::click());
    let id = r.id.with("info");
    let open = egui::Popup::is_id_open(ui.ctx(), id);
    let color = if open || r.hovered() { ui.visuals().strong_text_color() } else { theme::text_dim() };
    icons::paint(ui.painter(), rect.shrink(1.0), Icon::Info, color);
    let show = |ui: &mut egui::Ui| {
        ui.set_max_width(280.0);
        ui.label(text);
    };
    let r = if open { r } else { r.on_hover_ui(show) };
    egui::Popup::menu(&r).id(id).close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside).show(show);
}

/// A two-column form: labels ([`label`]) and values, rows ended with `ui.end_row()`.
/// Values fill their column ([`value_width`]).
pub fn grid(ui: &mut egui::Ui, salt: impl std::hash::Hash + std::fmt::Debug, add: impl FnOnce(&mut egui::Ui)) {
    let outer = IN_GRID.replace(true);
    egui::Grid::new(salt).num_columns(2).spacing([12.0, 6.0]).show(ui, add);
    IN_GRID.set(outer);
}

/// Width a value widget takes: its column's in a [`grid`] (`None` elsewhere: its own).
pub fn value_width(ui: &egui::Ui) -> Option<f32> {
    IN_GRID.get().then(|| ui.available_width())
}

/// Add `widget` as wide as the value column in a [`grid`] (else at its own size).
pub fn fill(ui: &mut egui::Ui, widget: impl egui::Widget) -> egui::Response {
    match value_width(ui) {
        Some(w) => ui.add_sized([w, ui.spacing().interact_size.y], widget),
        None => ui.add(widget),
    }
}

/// A combo box filling the value column in a [`grid`].
pub fn combo_box(ui: &egui::Ui, salt: impl std::hash::Hash + std::fmt::Debug) -> egui::ComboBox<'static> {
    let c = egui::ComboBox::from_id_salt(salt);
    match value_width(ui) {
        Some(w) => c.width(w),
        None => c,
    }
}

/// Two values side by side, splitting the value column.
pub fn pair<R>(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui, f32) -> R) -> R {
    let gap = ui.spacing().item_spacing.x;
    let half = value_width(ui).map_or(f32::NAN, |w| ((w - gap) / 2.0).floor());
    ui.horizontal(|ui| {
        let outer = IN_GRID.replace(false);
        let r = add(ui, half);
        IN_GRID.set(outer);
        r
    })
    .inner
}

/// Value widgets in `add` fill `width` points ([`pair`]'s halves; NaN: their own width).
pub fn sized<R>(ui: &mut egui::Ui, width: f32, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    if width.is_nan() {
        return add(ui);
    }
    // A grid's first frame doesn't know its column widths yet.
    let width = width.max(ui.spacing().interact_size.x);
    ui.allocate_ui(egui::vec2(width, ui.spacing().interact_size.y), |ui| {
        ui.set_width(width);
        let outer = IN_GRID.replace(true);
        let r = add(ui);
        IN_GRID.set(outer);
        r
    })
    .inner
}

/// A label in a [`grid`]'s first column: dim, as wide as the panel's label column,
/// wrapped when longer.
pub fn label(ui: &mut egui::Ui, text: &str) {
    label_at(ui, text, false);
}

/// A [`label`] at the top of its row (beside a multi-line field), aligned with the
/// field's first line.
pub fn label_top(ui: &mut egui::Ui, text: &str) {
    label_at(ui, text, true);
}

fn label_at(ui: &mut egui::Ui, text: &str, top: bool) {
    let color = theme::text_dim();
    let font = egui::TextStyle::Body.resolve(ui.style());
    let natural = ui.painter().layout_no_wrap(text.to_string(), font.clone(), color).size().x;
    let width = SCOPE.with_borrow_mut(|s| match s {
        Some(s) => {
            s.widest = s.widest.max(natural);
            if s.width > 0.0 { s.width } else { natural.min(s.cap) }
        }
        None => natural,
    });
    let galley = ui.painter().layout(text.to_string(), font, color, width.max(1.0));
    let line = ui.spacing().interact_size.y;
    let h = galley.size().y.max(line);
    let size = egui::vec2(width, h);
    let (rect, _) = if top {
        ui.allocate_ui_with_layout(size, egui::Layout::top_down(egui::Align::Min), |ui| {
            ui.allocate_exact_size(size, egui::Sense::hover())
        })
        .inner
    } else {
        ui.allocate_exact_size(size, egui::Sense::hover())
    };
    // One line is centred like a field's text; more lines start at the top.
    let y = if top {
        rect.top() + ((line - galley.size().y) / 2.0).max(0.0)
    } else {
        rect.center().y - galley.size().y / 2.0
    };
    ui.painter().galley(egui::pos2(rect.left(), y), galley, color);
}

/// A collapsible section (long lists), closed at first: its header looks like a
/// [`section`] title.
pub fn collapsing(
    ui: &mut egui::Ui,
    title: &str,
    salt: impl std::hash::Hash + std::fmt::Debug,
    add: impl FnOnce(&mut egui::Ui),
) {
    collapsing_note(ui, title, "", salt, false, add);
}

/// A collapsible section whose header looks like a [`section_note`] title (the note says
/// what's inside while it's closed).
pub fn collapsing_note(
    ui: &mut egui::Ui,
    title: &str,
    note: &str,
    salt: impl std::hash::Hash + std::fmt::Debug,
    open: bool,
    add: impl FnOnce(&mut egui::Ui),
) {
    space_above_section(ui);
    let id = ui.make_persistent_id(salt);
    collapsing_header(
        ui,
        id,
        open,
        |ui| {
            theme::bold_label(ui, title, SECTION_SIZE, ui.visuals().strong_text_color());
            if !note.is_empty() {
                ui.label(RichText::new(note).color(theme::text_dim()));
            }
        },
        add,
    );
}

/// A collapsing header and body that opens and closes on a click anywhere on the header
/// (its arrow, title or whatever `header` adds), not just on the arrow. The body isn't
/// indented (no line on its left): open, it looks like a plain section.
pub fn collapsing_header<R>(
    ui: &mut egui::Ui,
    id: egui::Id,
    open: bool,
    header: impl FnOnce(&mut egui::Ui) -> R,
    body: impl FnOnce(&mut egui::Ui),
) {
    use egui::collapsing_header::CollapsingState;
    let (_, head, _) =
        CollapsingState::load_with_default_open(ui.ctx(), id, open).show_header(ui, header).body_unindented(body);
    // Registered after the header's widgets, so on top of them (the arrow included): a
    // click toggles once wherever it lands.
    let r = ui.interact(head.response.rect, id.with("header"), egui::Sense::click());
    if r.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    if r.clicked() {
        let mut state = CollapsingState::load_with_default_open(ui.ctx(), id, open);
        state.toggle(ui);
        state.store(ui.ctx());
    }
}

/// A read-only row.
pub fn row(ui: &mut egui::Ui, k: &str, v: String) {
    label(ui, k);
    ui.label(v);
    ui.end_row();
}

/// A read-only count of problems: in the warning colour when not zero, with an optional
/// unit ("3 connections").
pub fn problem_row(ui: &mut egui::Ui, k: &str, n: usize, unit: Option<&str>) {
    label(ui, k);
    let text = match unit {
        Some(u) if n > 0 => format!("{n} {u}"),
        _ => n.to_string(),
    };
    let color = if n > 0 { theme::warn() } else { ui.visuals().text_color() };
    ui.label(RichText::new(text).color(color));
    ui.end_row();
}

/// A row of a check box (the label on the left).
pub fn check_row(ui: &mut egui::Ui, k: &str, value: &mut bool) -> egui::Response {
    label(ui, k);
    let r = ui.checkbox(value, "");
    ui.end_row();
    r
}

/// Size of a list panel's summary line.
const SUMMARY_SIZE: f32 = 12.0;

/// A list panel's search field: a search icon and a [`clearable`] field over `text`
/// (full width), set up by `edit` (id, hint, keys).
pub fn search_field(
    ui: &mut egui::Ui,
    text: &mut String,
    edit: impl FnOnce(egui::TextEdit<'_>) -> egui::TextEdit<'_>,
) -> egui::Response {
    use crate::icons::{self, Icon};
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        let (r, _) = ui.allocate_exact_size(egui::vec2(16.0, 16.0), egui::Sense::hover());
        icons::paint(ui.painter(), r, Icon::Search, theme::text_dim());
        clearable(ui, text, |e| edit(e).desired_width(f32::INFINITY))
    })
    .inner
}

/// Size of the clear button in a [`clearable`] field.
const CLEAR: f32 = 14.0;

/// A single-line field over `text` (set up by `edit`) with a round × at its right end
/// while it has text: a click clears it, keeps the focus in the field and counts as a
/// change of the field's response.
pub fn clearable(
    ui: &mut egui::Ui,
    text: &mut String,
    edit: impl FnOnce(egui::TextEdit<'_>) -> egui::TextEdit<'_>,
) -> egui::Response {
    let pad = ui.spacing().button_padding;
    // Room for the button at the right, so text never runs under it.
    let margin =
        egui::Margin { left: pad.x as i8, right: (CLEAR + 2.0 * 5.0) as i8, top: pad.y as i8, bottom: pad.y as i8 };
    let mut r = ui.add(edit(egui::TextEdit::singleline(text)).margin(margin));
    if text.is_empty() {
        return r;
    }
    // Added after the field: on top of it, so it gets the click.
    let center = egui::pos2(r.rect.right() - 5.0 - CLEAR / 2.0, r.rect.center().y);
    let rect = egui::Rect::from_center_size(center, egui::vec2(CLEAR, CLEAR));
    let button = ui.interact(rect, r.id.with("clear"), egui::Sense::click());
    let painter = ui.painter();
    let fill = if button.hovered() { ui.visuals().text_color() } else { theme::text_dim() };
    painter.circle_filled(center, CLEAR / 2.0, fill.gamma_multiply(0.8));
    let d = CLEAR * 0.18;
    let stroke = egui::Stroke::new(1.5, ui.visuals().extreme_bg_color);
    painter.line_segment([center + egui::vec2(-d, -d), center + egui::vec2(d, d)], stroke);
    painter.line_segment([center + egui::vec2(-d, d), center + egui::vec2(d, -d)], stroke);
    if button.on_hover_text("Clear").on_hover_cursor(egui::CursorIcon::Default).clicked() {
        text.clear();
        r.request_focus();
        r.mark_changed();
    }
    r
}

/// A list panel's summary line ("5 nets, 2 not fully routed"), below its search field.
pub fn summary(ui: &mut egui::Ui, text: &str) {
    ui.label(RichText::new(text).color(theme::text_dim()).size(SUMMARY_SIZE));
}

/// A one-line hint, dim.
pub fn hint(ui: &mut egui::Ui, text: &str) {
    ui.label(RichText::new(text).color(theme::text_dim()));
}

/// A row of choice buttons (selectable labels), tight enough for the value column.
pub fn choices<R>(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        ui.spacing_mut().button_padding.x = 7.0;
        add(ui)
    })
    .inner
}

/// Left / center / right buttons for a text alignment.
pub fn halign_row(ui: &mut egui::Ui, align: &mut HAlign) {
    choices(ui, |ui| {
        ui.selectable_value(align, HAlign::Left, "Left");
        ui.selectable_value(align, HAlign::Center, "Center");
        ui.selectable_value(align, HAlign::Right, "Right");
    });
}

/// Quarter-turn picker. Returns the new angle if changed.
pub fn rotation_combo(ui: &mut egui::Ui, salt: &str, current: Angle, names: [&str; 4]) -> Option<Angle> {
    let deg = current.degrees();
    let name = |d: f64| match current.quarter_turns() {
        Some(q) if (d - deg).abs() < 1e-9 => names[q as usize].to_string(),
        _ => format!("{d:.1}°"),
    };
    let mut out = None;
    combo_box(ui, salt).selected_text(name(deg)).show_ui(ui, |ui| {
        for (q, n) in names.iter().enumerate() {
            let d = q as f64 * 90.0;
            if ui.selectable_label((d - deg).abs() < 1e-9, *n).clicked() && (d - deg).abs() > 1e-9 {
                out = Some(Angle::deg(d));
            }
        }
    });
    out
}

/// The glyph of a power port named `name`: "Auto" (what the name suggests) or a fixed
/// style. Returns the newly chosen value.
pub fn power_style_combo(
    ui: &mut egui::Ui,
    salt: &str,
    current: Option<model::PowerStyle>,
    name: &str,
) -> Option<Option<model::PowerStyle>> {
    let auto = format!("Auto ({})", model::PowerStyle::for_name(name).label().to_lowercase());
    let options: Vec<_> = std::iter::once((None, auto))
        .chain(model::PowerStyle::ALL.into_iter().map(|s| (Some(s), s.label().to_string())))
        .collect();
    combo(ui, salt, &current, &options)
}

/// Value fields with SI prefixes ("10m", "1.5k"), committed on Enter or focus loss.
/// Text that isn't a value shows red and isn't committed.
#[derive(Default)]
pub struct SiFields {
    texts: FxHashMap<egui::Id, String>,
    invalid: std::collections::HashSet<egui::Id>,
}

impl SiFields {
    pub fn clear(&mut self) {
        self.texts.clear();
        self.invalid.clear();
    }

    /// `empty`: the hint of an optional value's empty field (empty: the value is
    /// required). Returns the new value when it changed.
    pub fn field(
        &mut self,
        ui: &mut egui::Ui,
        salt: impl std::hash::Hash + std::fmt::Debug,
        value: Option<f64>,
        unit: &str,
        empty: &str,
    ) -> Option<Option<f64>> {
        self.field_sized(ui, salt, value, unit, empty, 110.0)
    }

    /// [`field`](Self::field) `width` points wide (in a [`grid`]: filling the value column).
    pub fn field_sized(
        &mut self,
        ui: &mut egui::Ui,
        salt: impl std::hash::Hash + std::fmt::Debug,
        value: Option<f64>,
        unit: &str,
        empty: &str,
        width: f32,
    ) -> Option<Option<f64>> {
        let id = egui::Id::unique(salt);
        let shown = value.map_or(String::new(), |v| si::format(v, unit));
        let focused = ui.memory(|m| m.has_focus(id));
        let bad = self.invalid.contains(&id);
        let buf = self.texts.entry(id).or_insert_with(|| shown.clone());
        if !focused && !bad {
            buf.clone_from(&shown);
        }
        let width = value_width(ui).unwrap_or(width);
        let mut edit = egui::TextEdit::singleline(buf).id(id).desired_width(width).hint_text(empty);
        if bad {
            edit = edit.text_color(theme::error());
        }
        let r = ui.add(edit);
        if bad {
            r.clone()
                .on_hover_text(format!("Not a value: type a number with an optional prefix, like 10m or 1.5k{unit}"));
        }
        if !r.lost_focus() {
            return None;
        }
        let text = buf.trim().to_string();
        let new = if text.is_empty() && !empty.is_empty() {
            None
        } else if let Some(v) = si::parse(&text) {
            Some(v)
        } else {
            self.invalid.insert(id);
            return None;
        };
        self.invalid.remove(&id);
        (new != value).then_some(new)
    }
}

/// Generic combo over `options`; returns the newly chosen value.
pub fn combo<T: PartialEq + Clone>(
    ui: &mut egui::Ui,
    salt: impl std::hash::Hash + std::fmt::Debug,
    current: &T,
    options: &[(T, String)],
) -> Option<T> {
    let text = options.iter().find(|(v, _)| v == current).map_or("—".to_string(), |(_, n)| n.clone());
    let mut out = None;
    combo_box(ui, salt).selected_text(text).show_ui(ui, |ui| {
        for (v, n) in options {
            if ui.selectable_label(v == current, n).clicked() && v != current {
                out = Some(v.clone());
            }
        }
    });
    out
}

/// Rows of a drawing tool's corner style (icon buttons) and posture (which leg first:
/// automatic or fixed), for the styles `styles`.
pub fn corner_rows(
    ui: &mut egui::Ui,
    corners: &mut view::bend::Corners,
    bend: &mut view::bend::Bend,
    styles: &[view::bend::Corners],
) {
    use crate::icons::{self, Icon};
    use view::bend::{Bend, Corners};
    label(ui, "Corners");
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        for &style in styles {
            let icon = match style {
                Corners::Deg90 => Icon::Corner90,
                Corners::Deg45 => Icon::Corner45,
                Corners::Free => Icon::CornerFree,
                Corners::Arc45 => Icon::CornerArc45,
                Corners::Arc90 => Icon::CornerArc90,
            };
            let active = *corners == style;
            let (rect, r) = ui.allocate_exact_size(egui::vec2(26.0, 22.0), egui::Sense::click());
            let v = ui.visuals();
            let (fill, color) = if active {
                (theme::accent(), egui::Color32::WHITE)
            } else if r.hovered() {
                (v.widgets.hovered.weak_bg_fill, v.strong_text_color())
            } else {
                (v.widgets.inactive.weak_bg_fill, v.text_color())
            };
            ui.painter().rect_filled(rect, 4.0, fill);
            icons::paint(ui.painter(), rect.shrink(4.0), icon, color);
            if r.on_hover_text(style.label()).clicked() {
                *corners = style;
            }
        }
    });
    ui.end_row();
    if *corners != Corners::Free {
        label(ui, "First leg");
        choices(ui, |ui| {
            let (first, second) = corners.postures();
            ui.selectable_value(bend, Bend::Auto, "Auto")
                .on_hover_text("Follows the way the cursor leaves the start point; / flips it");
            ui.selectable_value(bend, Bend::First, first).on_hover_text(format!("{first} leg first"));
            ui.selectable_value(bend, Bend::Second, second).on_hover_text(format!("{second} leg first"));
        });
        ui.end_row();
    }
}

/// Category suggestions: those of `lib` and of the Built-in library. Parts and symbols
/// share theirs; footprints have their own.
pub fn categories(lib: &Library, footprints: bool) -> Vec<String> {
    static BUILTIN: std::sync::OnceLock<[Vec<String>; 2]> = std::sync::OnceLock::new();
    let builtin = BUILTIN.get_or_init(|| {
        let lib = builtin::builtin_library();
        [lib.categories(false), lib.categories(true)]
    });
    let mut out = lib.categories(footprints);
    out.extend(builtin[footprints as usize].iter().cloned());
    out.sort_by(|a, b| natural_cmp(a, b).then_with(|| a.cmp(b)));
    out.dedup();
    out
}
