//! Application chrome themes (dark and light). The colours follow the theme in effect
//! ([`view::style::theme`]): the app's, or an editor's inside its
//! [`view::style::scope`] (the simulation tabs draw their widgets in the Simulation
//! theme).

use egui::{Color32, CornerRadius, Frame, Margin, Painter, Rect, Stroke, Visuals, pos2, style::Selection};
use view::style::Theme;

fn dark() -> bool {
    view::style::theme() == Theme::Dark
}

/// A canvas palette colour (`view::style`) for egui.
pub fn color(c: render::Color) -> Color32 {
    let [r, g, b, a] = c.0;
    Color32::from_rgba_unmultiplied(r, g, b, a)
}

pub fn accent() -> Color32 {
    if dark() { Color32::from_rgb(0x4c, 0x9a, 0xff) } else { Color32::from_rgb(0x1f, 0x6f, 0xe0) }
}

pub fn panel() -> Color32 {
    if dark() { Color32::from_rgb(0x1e, 0x21, 0x27) } else { Color32::from_rgb(0xf2, 0xf2, 0xf4) }
}

/// The title row of a panel: a shade off the panel.
pub fn panel_header() -> Color32 {
    if dark() { Color32::from_rgb(0x25, 0x29, 0x30) } else { Color32::from_rgb(0xe9, 0xea, 0xee) }
}

/// Height of a panel's title row (its shade) and of the tab bars.
pub const TITLE_ROW_HEIGHT: f32 = 37.0;

/// Size of panel titles, tab titles and section titles.
pub const TITLE_SIZE: f32 = 14.0;

/// Tool bars and inset areas.
pub fn panel_dark() -> Color32 {
    if dark() { Color32::from_rgb(0x17, 0x19, 0x1e) } else { Color32::from_rgb(0xe4, 0xe5, 0xe9) }
}

pub fn warn() -> Color32 {
    if dark() { Color32::from_rgb(0xf0, 0xb4, 0x5a) } else { Color32::from_rgb(0xb0, 0x60, 0x00) }
}

/// Errors (ERC and DRC findings).
pub fn error() -> Color32 {
    if dark() { Color32::from_rgb(0xff, 0x6b, 0x7f) } else { Color32::from_rgb(0xc8, 0x14, 0x3c) }
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

/// Padding of dialogs and windows.
const PAD: i8 = 18;

/// Frame of dialogs and windows: no padding (the title bar spans the full width).
fn floating_frame(ctx: &egui::Context) -> Frame {
    Frame::NONE
        .fill(panel())
        .stroke(Stroke::new(1.0, border()))
        .corner_radius(RADIUS)
        .shadow(ctx.global_style().visuals.popup_shadow)
}

/// The title bar of dialogs and windows, with a close button when `close` is given.
/// Then the content with generous padding.
fn floating_contents<R>(
    ui: &mut egui::Ui,
    title: &str,
    close: Option<&mut bool>,
    add_contents: impl FnOnce(&mut egui::Ui) -> R,
) -> R {
    let r = RADIUS;
    let spacing = ui.spacing().item_spacing;
    ui.spacing_mut().item_spacing.y = 0.0;
    let bar = Frame::NONE
        .fill(panel_dark())
        .corner_radius(CornerRadius { nw: r - 1, ne: r - 1, sw: 0, se: 0 })
        .inner_margin(Margin { left: PAD, right: PAD, top: 11, bottom: 10 })
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.horizontal(|ui| {
                bold_label(ui, title, 17.0, accent());
                if let Some(close) = close {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let b = egui::Button::new(egui::RichText::new("×").size(18.0).color(text_dim())).frame(false);
                        if ui.add(b).on_hover_text("Close").clicked() {
                            *close = true;
                        }
                    });
                }
            });
        });
    let line = bar.response.rect;
    ui.painter().hline(line.x_range(), line.bottom(), Stroke::new(1.0, border()));
    Frame::NONE
        .inner_margin(Margin { left: PAD, right: PAD, top: 14, bottom: PAD })
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing = spacing;
            // Its form rows share one label column, like a panel's.
            crate::fields::form_scope(ui, ("window", title), add_contents)
        })
        .inner
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
    egui::Modal::new(id).frame(floating_frame(ctx)).show(ctx, |ui| {
        ui.set_width(width + 2.0 * f32::from(PAD));
        floating_contents(ui, title, None, add_contents)
    })
}

/// A resizable, non-modal window that looks like [`dialog`], with a close button, first
/// shown centred. Shown while `open`; returns the contents' result when shown.
pub fn window<R>(
    ctx: &egui::Context,
    title: &str,
    open: &mut bool,
    default_size: [f32; 2],
    add_contents: impl FnOnce(&mut egui::Ui) -> R,
) -> Option<R> {
    window_sized(ctx, title, open, default_size, [0.0, 0.0], add_contents)
}

/// The id of a [`window`]'s area (for its size).
pub fn window_id(title: &str) -> egui::Id {
    egui::Id::unique(("theme_window", title))
}

/// A [`window`] with a minimum size.
pub fn window_sized<R>(
    ctx: &egui::Context,
    title: &str,
    open: &mut bool,
    default_size: [f32; 2],
    min_size: [f32; 2],
    add_contents: impl FnOnce(&mut egui::Ui) -> R,
) -> Option<R> {
    if !*open {
        return None;
    }
    let mut close = false;
    let r = egui::Window::new(title)
        .id(window_id(title))
        .min_size(min_size)
        .title_bar(false)
        .frame(floating_frame(ctx))
        .default_size(default_size)
        // Opens centred in the app window; egui remembers where it was moved to.
        .pivot(egui::Align2::CENTER_CENTER)
        .default_pos(ctx.content_rect().center())
        .resizable(true)
        .collapsible(false)
        .show(ctx, |ui| floating_contents(ui, title, Some(&mut close), add_contents))
        .and_then(|r| r.inner);
    if close {
        *open = false;
    }
    r
}

/// Adjust `ui` for widgets on an inset area (`panel_dark`): in the light theme buttons
/// would otherwise have about the inset's colour.
pub fn on_inset(ui: &mut egui::Ui) {
    if !dark() {
        ui.visuals_mut().widgets.inactive.weak_bg_fill = panel();
    }
}

/// How far bold text is smeared sideways (points).
pub const BOLD: f32 = 0.8;

/// Text in a bolder weight. egui ships no bold face, so the text is drawn three times,
/// smeared sideways by `BOLD` points.
pub fn bold_label(ui: &mut egui::Ui, text: &str, size: f32, color: Color32) {
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

/// egui's fonts plus JetBrains Mono (embedded for canvas text anyway) as the last
/// fallback of both families: egui's defaults lack arrows and the key symbols ⌘ ⇧ ⌥.
pub fn install_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    let name = "JetBrainsMono";
    fonts.font_data.insert(name.into(), std::sync::Arc::new(egui::FontData::from_static(render::font::REGULAR)));
    for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
        fonts.families.entry(family).or_default().push(name.into());
    }
    ctx.set_fonts(fonts);
}

/// Apply the app's theme to egui.
pub fn apply(ctx: &egui::Context) {
    let dark = dark();
    // Pin egui's theme: it would otherwise follow the system (or window) appearance and
    // swap these visuals for its defaults.
    ctx.set_theme(if dark { egui::Theme::Dark } else { egui::Theme::Light });
    ctx.set_visuals(visuals());

    ctx.global_style_mut(|s| {
        s.spacing.item_spacing = egui::vec2(8.0, 6.0);
        s.spacing.button_padding = egui::vec2(10.0, 4.0);
        s.spacing.interact_size.y = 24.0;
    });
}

/// egui's visuals in the theme in effect (an editor's inside its scope).
pub fn visuals() -> Visuals {
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
        // Check boxes and radio buttons: visible on inset areas too (egui's default is
        // about the inset colour).
        v.widgets.inactive.bg_fill = Color32::from_rgb(0xcc, 0xcf, 0xd6);
        v.widgets.hovered.weak_bg_fill = Color32::from_rgb(0xd6, 0xd9, 0xe0);
    }
    v.widgets.active.weak_bg_fill = accent.gamma_multiply(0.6);
    v
}

/// macOS: the window's appearance (native buttons, edge highlight) follows the app theme,
/// since our title strip replaces the system's.
pub fn apply_to_window(ctx: &egui::Context) {
    if cfg!(target_os = "macos") {
        let t = if dark() { egui::SystemTheme::Dark } else { egui::SystemTheme::Light };
        ctx.send_viewport_cmd(egui::ViewportCommand::SetTheme(t));
    }
}

#[cfg(test)]
mod tests {
    /// Every character in the string literals of the UI code has a glyph in the UI fonts
    /// (with the fallback of [`super::install_fonts`]), so none shows as a box. Test
    /// modules, comments and doc comments are skipped.
    #[test]
    fn ui_strings_have_glyphs() {
        let ctx = egui::Context::default();
        super::install_fonts(&ctx);
        let mut out = ctx.run_ui(Default::default(), |_| {});
        out.textures_delta.clear();
        let font = egui::FontId::proportional(14.0);
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
        let mut missing = Vec::new();
        for dir in ["app/src", "view/src"] {
            for entry in std::fs::read_dir(root.join(dir)).unwrap() {
                let path = entry.unwrap().path();
                if path.extension().is_none_or(|e| e != "rs") {
                    continue;
                }
                let text = std::fs::read_to_string(&path).unwrap();
                let code = text.split("#[cfg(test)]").next().unwrap_or_default();
                for (n, line) in code.lines().enumerate() {
                    if line.trim_start().starts_with("//") {
                        continue;
                    }
                    // Characters between double quotes (good enough for this code base).
                    let mut inside = false;
                    for c in line.chars() {
                        if c == '"' {
                            inside = !inside;
                        } else if inside && !c.is_ascii() && !ctx.fonts_mut(|f| f.has_glyph(&font, c)) {
                            missing.push(format!("{}:{}: {c}", path.display(), n + 1));
                        }
                    }
                }
            }
        }
        assert!(missing.is_empty(), "characters the UI font lacks:\n{}", missing.join("\n"));
    }
}
