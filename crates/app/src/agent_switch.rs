//! Agent access in the Agent activity panel's header: "Access", a switch that turns the
//! MCP server on and off, a read-only text field with the URL it listens on (or "Access
//! is disabled") and a button that copies the URL.

use egui::{Color32, Response, RichText, Sense, Ui, vec2};

use crate::icons::{self, Icon};
use crate::theme;

/// The header's access controls, laid out left to right in the space after the title.
/// `url`: where the server listens, while it runs; `error`: why it didn't start.
/// Returns true if the switch was clicked (toggle the server).
pub fn header_ui(ui: &mut Ui, url: Option<&str>, port: u16, error: Option<&str>) -> bool {
    ui.spacing_mut().item_spacing.x = 6.0;
    let running = url.is_some();
    let hover = match (running, error) {
        (true, _) => "Agents can edit the open project over MCP. Click to turn access off.".to_string(),
        (false, Some(e)) => format!("The MCP server didn't start: {e}"),
        (false, None) => format!("Let agents edit the open project over MCP (port {port})."),
    };
    let label = ui.add(egui::Label::new(RichText::new("Access").color(theme::text_dim())).sense(Sense::click()));
    let r = switch(ui, running);
    if error.is_some() && !running {
        ui.painter().circle_filled(r.rect.right_top() + vec2(-1.0, 1.0), 3.0, theme::error());
    }
    let clicked = r.on_hover_text(&hover).clicked() || label.on_hover_text(&hover).clicked();
    ui.add_space(4.0);
    let (mut text, color, tip) = match url {
        Some(u) => (u.to_string(), ui.visuals().text_color(), "The URL agents connect to"),
        None => ("Access is disabled".to_string(), theme::text_dim(), "Turn access on with the switch"),
    };
    // As wide as the URL (the same in both states), or what's left of the header.
    let font = egui::TextStyle::Body.resolve(ui.style());
    let width = [url.map_or_else(|| format!("http://127.0.0.1:{port}/mcp"), str::to_string), text.clone()]
        .into_iter()
        .map(|t| ui.painter().layout_no_wrap(t, font.clone(), Color32::WHITE).size().x)
        .fold(0.0, f32::max);
    ui.add(
        egui::TextEdit::singleline(&mut text)
            .font(font)
            .text_color(color)
            .desired_width((width + 32.0).min(ui.available_width()))
            .margin(vec2(6.0, 1.0))
            .align(egui::Align2::LEFT_CENTER),
    )
    .on_hover_text(tip);
    if copy_button(ui, url.is_some()).clicked()
        && let Some(url) = url
    {
        ui.ctx().copy_text(url.to_string());
    }
    clicked
}

/// Copies the URL (as the panel headers' icon buttons; disabled while the server is off).
fn copy_button(ui: &mut Ui, enabled: bool) -> Response {
    let sense = if enabled { Sense::click() } else { Sense::hover() };
    let (rect, r) = ui.allocate_exact_size(vec2(20.0, 20.0), sense);
    if enabled && r.hovered() {
        ui.painter().rect_filled(rect, 4.0, ui.visuals().widgets.hovered.weak_bg_fill);
    }
    let color = if enabled { theme::text_dim() } else { theme::text_dim().gamma_multiply(0.4) };
    icons::paint(ui.painter(), rect.shrink(3.5), Icon::Copy, color);
    r.on_hover_text(if enabled { "Copy the URL" } else { "Copy the URL (access is off)" })
}

/// An on/off switch.
fn switch(ui: &mut Ui, on: bool) -> Response {
    let size = vec2(28.0, 16.0);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    response.widget_info(|| egui::WidgetInfo::selected(egui::Role::CheckBox, ui.is_enabled(), on, "Agent access"));
    let t = ui.ctx().animate_bool_responsive(response.id, on);
    let radius = rect.height() / 2.0;
    let off = lerp_color(ui.visuals().widgets.inactive.bg_fill, theme::text_dim(), 0.3);
    let fill = lerp_color(off, theme::accent(), t);
    let fill = if response.hovered() { lerp_color(fill, ui.visuals().text_color(), 0.15) } else { fill };
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
