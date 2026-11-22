//! Agent access in the Agent activity panel's header: "Access", a switch that turns the
//! MCP server on and off, and a read-only text field with the URL it listens on (or
//! "Access is disabled").

use egui::{Color32, Response, RichText, Sense, Ui, vec2};

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
    ui.add(
        egui::TextEdit::singleline(&mut text)
            .font(egui::FontId::monospace(11.5))
            .text_color(color)
            .desired_width(ui.available_width())
            .margin(vec2(6.0, 1.0))
            .align(egui::Align2::LEFT_CENTER),
    )
    .on_hover_text(tip);
    clicked
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
