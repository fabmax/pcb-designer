//! Floating tool bars over the canvases (like Blender's): a vertical strip of icon
//! buttons in rounded groups at the top left, an optional header with the tool's
//! options next to it, view buttons (layers, grid, fit) at the bottom left and the usage
//! hint next to them.
//!
//! They are drawn in the canvas' layer after the canvas, so they win egui's hit test:
//! clicks and drags on them never reach the canvas, and the canvas isn't hovered
//! underneath them.

use egui::{
    Align, Color32, CornerRadius, Id, Layout, Popup, PopupCloseBehavior, Pos2, Rect, RectAlign, RichText, Sense,
    Stroke, Ui, UiBuilder, vec2,
};
use model::Coord;
use render::Scene;
use render::gpu::Background;
use view::LayerEntry;

use crate::icons::{self, Icon};
use crate::keymap::{Action, Keymap};
use crate::theme;
use crate::units::{self, GridKind, Unit};

/// Button size (points).
const BUTTON: f32 = 32.0;
/// Padding of a group around its buttons.
const PAD: f32 = 3.0;
/// Distance from the canvas edges.
const INSET: f32 = 8.0;
/// Gap between groups.
const GAP: f32 = 6.0;
const RADIUS: u8 = 8;
const SHADOW: egui::Shadow = egui::Shadow { offset: [0, 2], blur: 10, spread: 0, color: Color32::from_black_alpha(70) };

/// A tool button.
pub struct ToolButton<T> {
    pub tool: T,
    pub icon: Icon,
    pub name: &'static str,
    /// What the tool does (tooltip).
    pub about: &'static str,
    /// Shortcut shown in the tooltip.
    pub action: Option<Action>,
}

/// The vertical tool strip at the top left of `canvas`. Returns the clicked tool and
/// the strip's right edge.
pub fn strip<T: Copy + PartialEq>(
    ui: &mut Ui,
    canvas: Rect,
    id: Id,
    groups: &[&[ToolButton<T>]],
    current: T,
    keymap: &Keymap,
) -> (Option<T>, f32) {
    let mut chosen = None;
    let mut top = canvas.top() + INSET;
    let left = canvas.left() + INSET;
    let painter = ui.painter_at(canvas);
    for (g, group) in groups.iter().enumerate() {
        if group.is_empty() {
            continue;
        }
        let n = group.len() as f32;
        let rect = Rect::from_min_size(Pos2::new(left, top), vec2(BUTTON + 2.0 * PAD, n * BUTTON + 2.0 * PAD));
        // Background: catches clicks between the buttons.
        ui.interact(rect, id.with(("group", g)), Sense::click_and_drag());
        paint_block(&painter, rect);
        for (i, b) in group.iter().enumerate() {
            let r = Rect::from_min_size(rect.min + vec2(PAD, PAD + i as f32 * BUTTON), vec2(BUTTON, BUTTON));
            let response = icon_button(ui, &painter, r, id.with((g, i)), b.icon, b.tool == current);
            // Tooltip to the right, like Blender's (not over the buttons below).
            let mut tip = egui::Tooltip::for_enabled(&response);
            tip.popup = tip.popup.align(RectAlign::RIGHT_START).gap(GAP + PAD);
            tip.show(|ui| tooltip(ui, b.name, b.about, b.action, keymap));
            if response.clicked() {
                chosen = Some(b.tool);
            }
        }
        top = rect.bottom() + GAP;
    }
    (chosen, left + BUTTON + 2.0 * PAD)
}

/// A square icon button: accent fill when `active`, highlighted on hover.
fn icon_button(ui: &Ui, painter: &egui::Painter, r: Rect, id: Id, icon: Icon, active: bool) -> egui::Response {
    let response = ui.interact(r, id, Sense::click_and_drag());
    let v = ui.visuals();
    let (fill, color) = if active {
        (theme::accent(), Color32::WHITE)
    } else if response.hovered() {
        (v.widgets.hovered.weak_bg_fill, v.strong_text_color())
    } else {
        (Color32::TRANSPARENT, v.text_color())
    };
    painter.rect_filled(r.shrink(1.0), CornerRadius::same(6), fill);
    icons::paint(painter, r.shrink(7.0), icon, color);
    response
}

fn tooltip(ui: &mut Ui, name: &str, about: &str, action: Option<Action>, keymap: &Keymap) {
    ui.set_max_width(280.0);
    ui.horizontal(|ui| {
        ui.label(RichText::new(name).strong());
        if let Some(action) = action {
            let keys = keymap.menu_text(ui.ctx(), action);
            if !keys.is_empty() {
                ui.label(RichText::new(keys).color(theme::text_dim()));
            }
        }
    });
    ui.label(about);
}

/// Background of a floating block: panel colour, a soft shadow and a faint outline so it
/// stands out against any canvas.
fn paint_block(painter: &egui::Painter, rect: Rect) {
    painter.add(SHADOW.as_shape(rect, CornerRadius::same(RADIUS)));
    painter.rect(rect, RADIUS, theme::panel(), Stroke::new(1.0, theme::border()), egui::StrokeKind::Inside);
}

/// A floating header at the top of `canvas`, starting at `left`, holding `add` (combo
/// boxes, labels). Its height matches a tool button group.
pub fn header<R>(ui: &mut Ui, canvas: Rect, left: f32, id: Id, add: impl FnOnce(&mut Ui) -> R) -> R {
    // The size is only known after the contents: block clicks on last frame's rect.
    let last: Option<Rect> = ui.data(|d| d.get_temp(id));
    if let Some(r) = last {
        ui.interact(r, id.with("bg"), Sense::click_and_drag());
    }
    let max = Rect::from_min_max(Pos2::new(left + GAP, canvas.top() + INSET), canvas.max - vec2(INSET, INSET));
    let mut child = ui.new_child(UiBuilder::new().max_rect(max).layout(Layout::left_to_right(Align::Min)));
    child.set_clip_rect(canvas.intersect(ui.clip_rect()));
    let frame = egui::Frame::NONE
        .fill(theme::panel())
        .stroke(Stroke::new(1.0, theme::border()))
        .shadow(SHADOW)
        .corner_radius(RADIUS)
        .inner_margin(egui::Margin::symmetric(10, 0));
    let shown = frame.show(&mut child, |ui| {
        let height = BUTTON + 2.0 * PAD - 2.0;
        ui.allocate_ui_with_layout(vec2(ui.available_width(), height), Layout::left_to_right(Align::Center), |ui| {
            ui.set_min_height(height);
            add(ui)
        })
        .inner
    });
    let rect = shown.response.rect;
    if last != Some(rect) {
        ui.data_mut(|d| d.insert_temp(id, rect));
        ui.ctx().request_repaint();
    }
    shown.inner
}

/// Width of the view buttons block at the bottom left.
const VIEW_BUTTONS_WIDTH: f32 = 3.0 * BUTTON + 2.0 * PAD;

/// The view buttons at the bottom left of `canvas`: the layer and grid buttons open
/// `layers` and `grid` in popups, fit view is returned as clicked.
pub fn view_buttons(
    ui: &mut Ui,
    canvas: Rect,
    id: Id,
    keymap: &Keymap,
    layers: impl FnOnce(&mut Ui),
    grid: impl FnOnce(&mut Ui),
) -> bool {
    let height = BUTTON + 2.0 * PAD;
    let rect = Rect::from_min_size(
        Pos2::new(canvas.left() + INSET, canvas.bottom() - INSET - height),
        vec2(VIEW_BUTTONS_WIDTH, height),
    );
    if canvas.width() < VIEW_BUTTONS_WIDTH + 2.0 * INSET || canvas.height() < 3.0 * height {
        return false;
    }
    ui.interact(rect, id.with("bg"), Sense::click_and_drag());
    let painter = ui.painter_at(canvas);
    paint_block(&painter, rect);
    let button = |i: usize| Rect::from_min_size(rect.min + vec2(PAD + i as f32 * BUTTON, PAD), vec2(BUTTON, BUTTON));

    let popup_button =
        |ui: &mut Ui, i: usize, icon: Icon, name: &str, about: &str, content: &mut dyn FnMut(&mut Ui)| {
            let bid = id.with(name);
            let was_open = Popup::is_id_open(ui.ctx(), bid.with("popup"));
            // Clicks toggle the popup this frame; show the state it will have.
            let clicked = ui.ctx().read_response(bid).is_some_and(|r| r.clicked());
            let response = icon_button(ui, &painter, button(i), bid, icon, was_open != clicked);
            if !was_open {
                let mut tip = egui::Tooltip::for_enabled(&response);
                tip.popup = tip.popup.align(RectAlign::TOP_START).gap(GAP + PAD);
                tip.show(|ui| tooltip(ui, name, about, None, keymap));
            }
            Popup::menu(&response)
                .id(bid.with("popup"))
                .align(RectAlign::TOP_START)
                .gap(GAP + PAD)
                .close_behavior(PopupCloseBehavior::CloseOnClickOutside)
                .show(|ui| {
                    ui.set_min_width(180.0);
                    ui.set_max_width(220.0);
                    content(ui)
                });
        };
    let mut layers = Some(layers);
    let mut grid = Some(grid);
    popup_button(ui, 0, Icon::Layers, "Layers", "Show or hide layers.", &mut |ui| {
        if let Some(f) = layers.take() {
            f(ui)
        }
    });
    popup_button(ui, 1, Icon::Grid, "Grid", "Grid size, units and visibility.", &mut |ui| {
        if let Some(f) = grid.take() {
            f(ui)
        }
    });
    let fit = icon_button(ui, &painter, button(2), id.with("fit"), Icon::Fit, false);
    let mut tip = egui::Tooltip::for_enabled(&fit);
    tip.popup = tip.popup.align(RectAlign::TOP_START).gap(GAP + PAD);
    tip.show(|ui| tooltip(ui, "Fit view", "Zoom to show everything.", Some(Action::FitView), keymap));
    fit.clicked()
}

/// Layer menu: a checkbox per layer with its colour, groups separated, and buttons to
/// show or hide all.
pub fn layer_menu(ui: &mut Ui, scene: &mut Scene, entries: &[LayerEntry]) {
    ui.horizontal(|ui| {
        ui.label(RichText::new("Layers").strong());
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            for (label, visible) in [("None", false), ("All", true)] {
                if ui.small_button(label).clicked() {
                    for e in entries {
                        scene.layer_mut(e.layer).visible = visible;
                    }
                }
            }
        });
    });
    ui.separator();
    let mut group = entries.first().map_or(0, |e| e.group);
    for e in entries {
        if e.group != group {
            group = e.group;
            ui.separator();
        }
        let layer = scene.layer_mut(e.layer);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            ui.checkbox(&mut layer.visible, "");
            let (r, swatch) = ui.allocate_exact_size(vec2(12.0, 12.0), Sense::click());
            if let Some(c) = e.color {
                let [red, g, b, _] = c.0;
                let color = Color32::from_rgb(red, g, b);
                let color = if layer.visible { color } else { color.gamma_multiply(0.35) };
                ui.painter().rect_filled(r, 3.0, color);
            }
            let text = RichText::new(&layer.name);
            let text = if layer.visible { text } else { text.color(theme::text_dim()) };
            let name = ui.add(egui::Label::new(text).sense(Sense::click()).selectable(false));
            if swatch.clicked() || name.clicked() {
                layer.visible = !layer.visible;
            }
        });
    }
}

/// Grid menu: display unit, grid size (presets for `kind` or any value) and visibility.
pub fn grid_menu(ui: &mut Ui, grid: &mut Coord, background: &mut Background, kind: GridKind) {
    let mut set = |grid: &mut Coord, g: Coord| {
        *grid = g;
        background.grid = Unit::Mm.value(g) as f32;
    };
    let dim = |ui: &mut Ui, text: &str| ui.label(RichText::new(text).color(theme::text_dim()));
    ui.horizontal(|ui| {
        dim(ui, "Units");
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            // Menus use tight padding; the unit chips and the size field get more room.
            ui.spacing_mut().button_padding = vec2(8.0, 3.0);
            ui.spacing_mut().item_spacing.x = 4.0;
            let mut unit = units::unit();
            for u in Unit::ALL.into_iter().rev() {
                ui.selectable_value(&mut unit, u, u.label());
            }
            if unit != units::unit() {
                units::set_unit(unit);
                // A round grid in the new unit (0.5 mm → 25 mil, not 19.69 mil).
                let g = kind.nearest_preset(*grid, unit);
                set(grid, g);
            }
        });
    });
    ui.separator();
    ui.horizontal(|ui| {
        dim(ui, "Grid size");
        // Main align Max: the value is right-aligned in its field (shown and edited).
        ui.with_layout(Layout::right_to_left(Align::Center).with_main_align(Align::Max), |ui| {
            ui.spacing_mut().button_padding = vec2(8.0, 3.0);
            ui.spacing_mut().interact_size.x = 84.0;
            // Menus draw widgets without a frame: give the field one.
            let v = &mut ui.visuals_mut().widgets;
            for w in [&mut v.inactive, &mut v.hovered, &mut v.active] {
                w.weak_bg_fill = theme::panel_dark();
                w.bg_stroke = Stroke::new(1.0, theme::border());
            }
            if let (_, Some(g)) = units::drag(ui, *grid, 1_000, 100_000_000) {
                set(grid, g);
            }
        });
    });
    for g in kind.presets(units::unit()) {
        if ui.selectable_label(*grid == g, units::fmt(g)).clicked() {
            set(grid, g);
        }
    }
    ui.separator();
    ui.checkbox(&mut background.show_grid, "Show grid");
}

/// The usage hint at the bottom of `canvas`, right of the view buttons (not interactive).
pub fn hint(ui: &Ui, canvas: Rect, text: &str) {
    let left = canvas.left() + INSET + VIEW_BUTTONS_WIDTH + GAP;
    let max_width = canvas.right() - INSET - left - 20.0;
    if max_width < 40.0 {
        return;
    }
    let galley = egui::WidgetText::from(RichText::new(text).color(theme::text_dim())).into_galley(
        ui,
        Some(egui::TextWrapMode::Truncate),
        max_width,
        egui::TextStyle::Body,
    );
    let size = galley.size() + vec2(20.0, 10.0);
    // Centred on the view buttons.
    let center_y = canvas.bottom() - INSET - (BUTTON + 2.0 * PAD) / 2.0;
    let rect = Rect::from_min_size(Pos2::new(left, center_y - size.y / 2.0), size);
    let painter = ui.painter_at(canvas);
    painter.rect_filled(rect, RADIUS, theme::panel().gamma_multiply(0.85));
    painter.galley(rect.min + vec2(10.0, 5.0), galley, theme::text_dim());
}
