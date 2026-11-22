//! Agent access in the Agent activity panel: a switch in its header turns the MCP server
//! on and off; while it runs, the panel shows the URL agents connect to (selectable, with
//! a copy button).

use egui::{Color32, Response, RichText, Sense, Ui, vec2};

use crate::icons::{self, Icon};
use crate::theme;

/// The switch in the Agent activity panel's header (laid out right to left). Returns
/// true if it was clicked (toggle the server). `error`: why the server didn't start.
pub fn switch_ui(ui: &mut Ui, running: bool, port: u16, error: Option<&str>) -> bool {
    let hover = match (running, error) {
        (true, _) => "Agents can edit the open project over MCP. Click to turn agent access off.".to_string(),
        (false, Some(e)) => format!("The MCP server didn't start: {e}"),
        (false, None) => format!("Let agents edit the open project over MCP (port {port})."),
    };
    let r = switch(ui, running);
    if error.is_some() && !running {
        ui.painter().circle_filled(r.rect.right_top() + vec2(-1.0, 1.0), 3.0, theme::error());
    }
    r.on_hover_text(hover).clicked()
}

/// The URL the server listens on, as wide as the panel, with a copy button.
pub fn url_ui(ui: &mut Ui, url: &str) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        ui.label(RichText::new("Listening on").color(theme::text_dim()));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if copy_button(ui).on_hover_text("Copy the URL").clicked() {
                ui.ctx().copy_text(url.to_string());
            }
            url_field(ui, url);
        });
    });
}

/// The URL as a read-only text field filling the row.
fn url_field(ui: &mut Ui, url: &str) {
    let mut text = url.to_string();
    ui.add(
        egui::TextEdit::singleline(&mut text)
            .font(egui::FontId::monospace(11.5))
            .desired_width(ui.available_width())
            .margin(vec2(6.0, 3.0))
            .align(egui::Align2::LEFT_CENTER),
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
