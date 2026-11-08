//! Dark theme for the application chrome.

use egui::{Color32, CornerRadius, Stroke, Visuals, style::Selection};

pub const ACCENT: Color32 = Color32::from_rgb(0x4c, 0x9a, 0xff);
pub const PANEL: Color32 = Color32::from_rgb(0x1e, 0x21, 0x27);
pub const PANEL_DARK: Color32 = Color32::from_rgb(0x17, 0x19, 0x1e);
pub const WARN: Color32 = Color32::from_rgb(0xf0, 0xb4, 0x5a);
pub const SUCCESS: Color32 = Color32::from_rgb(0x5f, 0xd3, 0xa0);
pub const TEXT_DIM: Color32 = Color32::from_rgb(0x8a, 0x92, 0xa3);

pub fn apply(ctx: &egui::Context) {
    let mut v = Visuals::dark();
    v.panel_fill = PANEL;
    v.window_fill = PANEL;
    v.extreme_bg_color = PANEL_DARK;
    v.faint_bg_color = Color32::from_rgb(0x23, 0x27, 0x2e);
    v.selection = Selection { bg_fill: ACCENT.gamma_multiply(0.45), stroke: Stroke::new(1.0, ACCENT) };
    v.hyperlink_color = ACCENT;
    v.window_corner_radius = CornerRadius::same(8);
    v.menu_corner_radius = CornerRadius::same(6);
    for w in [
        &mut v.widgets.noninteractive,
        &mut v.widgets.inactive,
        &mut v.widgets.hovered,
        &mut v.widgets.active,
        &mut v.widgets.open,
    ] {
        w.corner_radius = CornerRadius::same(5);
    }
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, Color32::from_rgb(0x2c, 0x30, 0x38));
    v.widgets.inactive.weak_bg_fill = Color32::from_rgb(0x2a, 0x2e, 0x36);
    v.widgets.hovered.weak_bg_fill = Color32::from_rgb(0x34, 0x39, 0x43);
    v.widgets.active.weak_bg_fill = ACCENT.gamma_multiply(0.6);
    ctx.set_visuals(v);

    ctx.global_style_mut(|s| {
        s.spacing.item_spacing = egui::vec2(8.0, 6.0);
        s.spacing.button_padding = egui::vec2(10.0, 4.0);
        s.spacing.interact_size.y = 24.0;
    });
}
