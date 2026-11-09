//! Application chrome themes (dark and light). The active theme is the canvas palette's
//! ([`view::style::theme`]), so chrome and canvases always match.

use egui::{Color32, CornerRadius, Stroke, Visuals, style::Selection};
use view::style::Theme;

fn dark() -> bool {
    view::style::theme() == Theme::Dark
}

pub fn accent() -> Color32 {
    if dark() { Color32::from_rgb(0x4c, 0x9a, 0xff) } else { Color32::from_rgb(0x1f, 0x6f, 0xe0) }
}

pub fn panel() -> Color32 {
    if dark() { Color32::from_rgb(0x1e, 0x21, 0x27) } else { Color32::from_rgb(0xf2, 0xf2, 0xf4) }
}

/// Tool bars and inset areas.
pub fn panel_dark() -> Color32 {
    if dark() { Color32::from_rgb(0x17, 0x19, 0x1e) } else { Color32::from_rgb(0xe4, 0xe5, 0xe9) }
}

pub fn warn() -> Color32 {
    if dark() { Color32::from_rgb(0xf0, 0xb4, 0x5a) } else { Color32::from_rgb(0xb0, 0x60, 0x00) }
}

pub fn success() -> Color32 {
    if dark() { Color32::from_rgb(0x5f, 0xd3, 0xa0) } else { Color32::from_rgb(0x12, 0x80, 0x50) }
}

pub fn text_dim() -> Color32 {
    if dark() { Color32::from_rgb(0x8a, 0x92, 0xa3) } else { Color32::from_rgb(0x5c, 0x63, 0x70) }
}

/// Apply the theme to egui (call after [`view::style::set_theme`]).
pub fn apply(ctx: &egui::Context) {
    let dark = dark();
    let mut v = if dark { Visuals::dark() } else { Visuals::light() };
    let (accent, panel, inset) = (accent(), panel(), panel_dark());
    v.panel_fill = panel;
    v.window_fill = panel;
    v.extreme_bg_color = inset;
    v.selection =
        Selection { bg_fill: accent.gamma_multiply(if dark { 0.45 } else { 0.3 }), stroke: Stroke::new(1.0, accent) };
    v.hyperlink_color = accent;
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
    if dark {
        v.faint_bg_color = Color32::from_rgb(0x23, 0x27, 0x2e);
        v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, Color32::from_rgb(0x2c, 0x30, 0x38));
        v.widgets.inactive.weak_bg_fill = Color32::from_rgb(0x2a, 0x2e, 0x36);
        v.widgets.hovered.weak_bg_fill = Color32::from_rgb(0x34, 0x39, 0x43);
    } else {
        v.faint_bg_color = Color32::from_rgb(0xea, 0xeb, 0xee);
        v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, Color32::from_rgb(0xd2, 0xd4, 0xda));
        v.widgets.inactive.weak_bg_fill = Color32::from_rgb(0xe2, 0xe4, 0xe9);
        v.widgets.hovered.weak_bg_fill = Color32::from_rgb(0xd6, 0xd9, 0xe0);
    }
    v.widgets.active.weak_bg_fill = accent.gamma_multiply(0.6);
    ctx.set_visuals(v);

    ctx.global_style_mut(|s| {
        s.spacing.item_spacing = egui::vec2(8.0, 6.0);
        s.spacing.button_padding = egui::vec2(10.0, 4.0);
        s.spacing.interact_size.y = 24.0;
    });
}
