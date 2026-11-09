//! Application chrome themes (dark and light). The active theme is the canvas palette's
//! ([`view::style::theme`]), so chrome and canvases always match.

use egui::{Color32, CornerRadius, Frame, Margin, Painter, Rect, Stroke, Visuals, pos2, style::Selection};
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

/// Window background: shows in the gaps between panels.
pub fn window_bg() -> Color32 {
    if dark() { Color32::from_rgb(0x0d, 0x0e, 0x11) } else { Color32::from_rgb(0xd3, 0xd6, 0xdc) }
}

/// Subtle outline of panels and tab groups.
pub fn border() -> Color32 {
    if dark() { Color32::from_rgb(0x2a, 0x2e, 0x36) } else { Color32::from_rgb(0xc4, 0xc8, 0xd0) }
}

/// Gap between panels (points).
pub const GAP: i8 = 6;
/// Corner radius of panels and tab groups.
pub const RADIUS: u8 = 10;

/// A side panel: rounded, `outer` margin to its neighbours.
pub fn panel_frame(outer: Margin) -> Frame {
    Frame::NONE.fill(panel()).corner_radius(RADIUS).inner_margin(Margin::same(8)).outer_margin(outer)
}

/// A modal dialog: a title bar in the accent colour over `width` points of content with
/// generous padding. Closes on Esc or a click outside (see `ModalResponse::should_close`).
pub fn dialog<R>(
    ctx: &egui::Context,
    id: egui::Id,
    title: &str,
    width: f32,
    add_contents: impl FnOnce(&mut egui::Ui) -> R,
) -> egui::ModalResponse<R> {
    const PAD: i8 = 18;
    let r = RADIUS;
    let frame = Frame::NONE
        .fill(panel())
        .stroke(Stroke::new(1.0, border()))
        .corner_radius(r)
        .shadow(ctx.global_style().visuals.popup_shadow);
    egui::Modal::new(id).frame(frame).show(ctx, |ui| {
        ui.set_width(width + 2.0 * f32::from(PAD));
        let spacing = ui.spacing().item_spacing;
        ui.spacing_mut().item_spacing.y = 0.0;
        let bar = Frame::NONE
            .fill(panel_dark())
            .corner_radius(CornerRadius { nw: r - 1, ne: r - 1, sw: 0, se: 0 })
            .inner_margin(Margin { left: PAD, right: PAD, top: 11, bottom: 10 })
            .show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                bold_label(ui, title, 17.0, accent());
            });
        let line = bar.response.rect;
        ui.painter().hline(line.x_range(), line.bottom(), Stroke::new(1.0, border()));
        Frame::NONE
            .inner_margin(Margin { left: PAD, right: PAD, top: 14, bottom: PAD })
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing = spacing;
                add_contents(ui)
            })
            .inner
    })
}

/// Text in a bolder weight. egui ships no bold face, so the text is drawn three times,
/// smeared sideways by `BOLD` points.
pub fn bold_label(ui: &mut egui::Ui, text: &str, size: f32, color: Color32) {
    const BOLD: f32 = 0.8;
    let galley = ui.painter().layout_no_wrap(text.to_string(), egui::FontId::proportional(size), color);
    let (rect, _) = ui.allocate_exact_size(galley.size() + egui::vec2(BOLD, 0.0), egui::Sense::hover());
    for dx in [0.0, BOLD / 2.0, BOLD] {
        ui.painter().galley(rect.min + egui::vec2(dx, 0.0), galley.clone(), color);
    }
}

/// Round the corners of `rect` after its content was painted (content that can't be
/// clipped, like the GPU canvases): masks the corners with the window background and
/// smooths the rounded edge.
pub fn round_corners(painter: &Painter, rect: Rect) {
    let r = f32::from(RADIUS);
    let bg = window_bg();
    let mut mesh = egui::Mesh::default();
    let corners = [
        (rect.left_top(), pos2(rect.left() + r, rect.top() + r), std::f32::consts::PI),
        (rect.right_top(), pos2(rect.right() - r, rect.top() + r), 1.5 * std::f32::consts::PI),
        (rect.right_bottom(), pos2(rect.right() - r, rect.bottom() - r), 0.0),
        (rect.left_bottom(), pos2(rect.left() + r, rect.bottom() - r), 0.5 * std::f32::consts::PI),
    ];
    const STEPS: u32 = 8;
    for (corner, center, start) in corners {
        // A fan from the corner over the quarter circle.
        let base = mesh.vertices.len() as u32;
        mesh.colored_vertex(corner, bg);
        for i in 0..=STEPS {
            let t = start + std::f32::consts::FRAC_PI_2 * i as f32 / STEPS as f32;
            mesh.colored_vertex(center + r * egui::vec2(t.cos(), t.sin()), bg);
        }
        for i in 0..STEPS {
            mesh.add_triangle(base, base + 1 + i, base + 2 + i);
        }
    }
    painter.add(mesh);
    // Anti-aliased edge over the mask's (in the background colour: no visible outline).
    painter.rect_stroke(rect, RADIUS, Stroke::new(1.0, bg), egui::StrokeKind::Middle);
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
