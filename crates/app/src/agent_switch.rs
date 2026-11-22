//! The agent access switch in the title bar (the menu bar off macOS): turns the MCP
//! server on and off and, while it runs, shows the URL agents connect to (selectable,
//! with a copy button).

use egui::{Color32, Response, RichText, Sense, Ui, vec2};

use crate::icons::{self, Icon};
use crate::theme;

/// Draws the switch (right to left: lay it out in a right-to-left `ui`). Returns true if
/// it was clicked (toggle the server). `error`: why the server didn't start.
pub fn ui(ui: &mut Ui, running: bool, url: Option<&str>, port: u16, error: Option<&str>) -> bool {
    ui.spacing_mut().item_spacing.x = 6.0;
    let hover = match (running, error) {
        (true, _) => "Agents can edit the open project over MCP. Click to turn agent access off.".to_string(),
        (false, Some(e)) => format!("The MCP server didn't start: {e}"),
        (false, None) => format!("Let agents edit the open project over MCP (port {port})."),
    };
    let clicked = switch(ui, running).on_hover_text(&hover).clicked();
    let color = if error.is_some() && !running { theme::error() } else { ui.visuals().text_color() };
    let label = ui.add(egui::Label::new(RichText::new("Agent access").size(12.5).color(color)).sense(Sense::click()));
    let clicked = clicked || label.on_hover_text(&hover).clicked();
    if let Some(url) = url {
        if copy_button(ui).on_hover_text("Copy the URL").clicked() {
            ui.ctx().copy_text(url.to_string());
        }
        url_field(ui, url);
    }
    clicked
}

/// The URL as a read-only text field, as wide as its text.
fn url_field(ui: &mut Ui, url: &str) {
    let font = egui::FontId::monospace(11.5);
    let width = ui.painter().layout_no_wrap(url.to_string(), font.clone(), Color32::WHITE).size().x;
    let mut text = url.to_string();
    ui.add(
        egui::TextEdit::singleline(&mut text)
            .font(font)
            .desired_width(width + 8.0)
            .clip_text(false)
            .margin(vec2(6.0, 2.0))
            .text_color(theme::text_dim()),
    )
    .on_hover_text("The URL agents connect to (settings: Agent access)");
}

fn copy_button(ui: &mut Ui) -> Response {
    let (rect, response) = ui.allocate_exact_size(vec2(18.0, 18.0), Sense::click());
    if response.hovered() {
        ui.painter().rect_filled(rect, 4.0, ui.visuals().widgets.hovered.weak_bg_fill);
    }
    let color = if response.hovered() { ui.visuals().text_color() } else { theme::text_dim() };
    icons::paint(ui.painter(), rect.shrink(2.0), Icon::Copy, color);
    response
}

/// An on/off switch.
fn switch(ui: &mut Ui, on: bool) -> Response {
    let size = vec2(28.0, 16.0);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    response.widget_info(|| egui::WidgetInfo::selected(egui::Role::CheckBox, ui.is_enabled(), on, "Agent access"));
    let t = ui.ctx().animate_bool_responsive(response.id, on);
    let radius = rect.height() / 2.0;
    let off = ui.visuals().widgets.inactive.bg_fill;
    let fill = lerp_color(off, theme::accent(), t);
    let fill = if response.hovered() { fill.gamma_multiply(1.15) } else { fill };
    let painter = ui.painter();
    painter.rect_filled(rect, radius, fill);
    let x = egui::lerp((rect.left() + radius)..=(rect.right() - radius), t);
    painter.circle_filled(egui::pos2(x, rect.center().y), radius - 2.5, Color32::WHITE);
    response
}

fn lerp_color(a: Color32, b: Color32, t: f32) -> Color32 {
    let l = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgba_unmultiplied(l(a.r(), b.r()), l(a.g(), b.g()), l(a.b(), b.b()), l(a.a(), b.a()))
}
