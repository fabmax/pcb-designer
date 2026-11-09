//! Floating tool bars over the canvases (like Blender's): a vertical strip of icon
//! buttons in rounded groups at the top left, an optional header with the tool's
//! options next to it, and the usage hint at the bottom left.
//!
//! They are drawn in the canvas' layer after the canvas, so they win egui's hit test:
//! clicks and drags on them never reach the canvas, and the canvas isn't hovered
//! underneath them.

use egui::{Align, Color32, CornerRadius, Id, Layout, Pos2, Rect, RichText, Sense, Stroke, Ui, UiBuilder, vec2};

use crate::icons::{self, Icon};
use crate::keymap::{Action, Keymap};
use crate::theme;

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
            let response = ui.interact(r, id.with((g, i)), Sense::click_and_drag());
            let active = b.tool == current;
            let v = ui.visuals();
            let (fill, color) = if active {
                (theme::accent(), Color32::WHITE)
            } else if response.hovered() {
                (v.widgets.hovered.weak_bg_fill, v.strong_text_color())
            } else {
                (Color32::TRANSPARENT, v.text_color())
            };
            painter.rect_filled(r.shrink(1.0), CornerRadius::same(6), fill);
            icons::paint(&painter, r.shrink(7.0), b.icon, color);
            // Tooltip to the right, like Blender's (not over the buttons below).
            let mut tip = egui::Tooltip::for_enabled(&response);
            tip.popup = tip.popup.align(egui::RectAlign::RIGHT_START).gap(GAP + PAD);
            tip.show(|ui| tooltip(ui, b, keymap));
            if response.clicked() {
                chosen = Some(b.tool);
            }
        }
        top = rect.bottom() + GAP;
    }
    (chosen, left + BUTTON + 2.0 * PAD)
}

fn tooltip<T>(ui: &mut Ui, b: &ToolButton<T>, keymap: &Keymap) {
    ui.set_max_width(280.0);
    ui.horizontal(|ui| {
        ui.label(RichText::new(b.name).strong());
        if let Some(action) = b.action {
            let keys = keymap.menu_text(ui.ctx(), action);
            if !keys.is_empty() {
                ui.label(RichText::new(keys).color(theme::text_dim()));
            }
        }
    });
    ui.label(b.about);
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

/// The usage hint at the bottom left of `canvas` (not interactive).
pub fn hint(ui: &Ui, canvas: Rect, text: &str) {
    let max_width = canvas.width() - 2.0 * INSET - 20.0;
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
    let rect = Rect::from_min_size(Pos2::new(canvas.left() + INSET, canvas.bottom() - INSET - size.y), size);
    let painter = ui.painter_at(canvas);
    painter.rect_filled(rect, RADIUS, theme::panel().gamma_multiply(0.85));
    painter.galley(rect.min + vec2(10.0, 5.0), galley, theme::text_dim());
}
