//! macOS: our own title strip. The window's title bar is transparent and the content
//! extends under it (see `main.rs`); this strip fills the title bar area in the window
//! background colour, with the app icon and the project name, while the native window
//! buttons stay on its left.
//!
//! The AppKit bits winit doesn't expose (title bar metrics, the "unsaved" dot, the
//! title bar double-click setting) are read from the `NSWindow` here.

use egui::{Color32, Sense, TextureHandle, pos2, vec2};
use objc2::rc::Retained;
use objc2_app_kit::{NSView, NSWindow, NSWindowButton};
use objc2_foundation::{NSString, NSUserDefaults};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

use crate::theme;

/// Size of the icon in the strip (points).
const ICON: f32 = 16.0;

/// Title bar geometry of the window (points).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Chrome {
    /// Height of the title bar (0 in full screen).
    pub height: f32,
    /// Right edge of the native window buttons.
    pub buttons_right: f32,
}

impl Chrome {
    /// Used until the window can be asked (and in screenshots of a window-less frame).
    pub const DEFAULT: Chrome = Chrome { height: 28.0, buttons_right: 78.0 };
}

/// What a double-click on the title bar does (System Settings → Desktop & Dock).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DoubleClick {
    Zoom,
    Minimize,
    Nothing,
}

impl DoubleClick {
    /// From the global `AppleActionOnDoubleClick` default ("Maximize", "Fill", "Minimize",
    /// "None"; unset means zoom). "Fill" (tile to fill the screen) has no public API, so
    /// it zooms.
    pub fn from_setting(value: Option<&str>) -> DoubleClick {
        match value {
            Some("Minimize") => DoubleClick::Minimize,
            Some("None") => DoubleClick::Nothing,
            _ => DoubleClick::Zoom,
        }
    }
}

fn ns_window(frame: &eframe::Frame) -> Option<Retained<NSWindow>> {
    let RawWindowHandle::AppKit(h) = frame.window_handle().ok()?.as_raw() else { return None };
    // SAFETY: the handle's view lives as long as the window, and `ui` runs on the main
    // thread (AppKit's requirement).
    let view: &NSView = unsafe { h.ns_view.cast().as_ref() };
    view.window()
}

/// The window's title bar geometry, if there is a window.
pub fn chrome(frame: &eframe::Frame) -> Option<Chrome> {
    let window = ns_window(frame)?;
    let height = (window.frame().size.height - window.contentLayoutRect().size.height).max(0.0) as f32;
    let buttons_right =
        window.standardWindowButton(NSWindowButton::ZoomButton).map_or(Chrome::DEFAULT.buttons_right, |b| {
            let f = b.frame();
            (f.origin.x + f.size.width) as f32
        });
    Some(Chrome { height, buttons_right })
}

/// Shows (or clears) the dot in the close button for unsaved changes.
pub fn set_document_edited(frame: &eframe::Frame, edited: bool) {
    if let Some(window) = ns_window(frame)
        && window.isDocumentEdited() != edited
    {
        window.setDocumentEdited(edited);
    }
}

fn double_click(frame: &eframe::Frame) {
    let key = NSString::from_str("AppleActionOnDoubleClick");
    let value = NSUserDefaults::standardUserDefaults().stringForKey(&key).map(|s| s.to_string());
    let Some(window) = ns_window(frame) else { return };
    match DoubleClick::from_setting(value.as_deref()) {
        DoubleClick::Zoom => window.zoom(None),
        DoubleClick::Minimize => window.miniaturize(None),
        DoubleClick::Nothing => {}
    }
}

/// The strip's text: the project name, plus "— Edited" with unsaved changes.
pub fn title_parts(name: &str, edited: bool) -> (String, Option<&'static str>) {
    (name.to_string(), edited.then_some(" — Edited"))
}

#[derive(Default)]
pub struct TitleBar {
    icon: Option<TextureHandle>,
}

impl TitleBar {
    /// Draws the strip as a top panel `chrome.height` high.
    pub fn show(&mut self, ui: &mut egui::Ui, frame: &eframe::Frame, chrome: Chrome, name: &str, edited: bool) {
        let icon = self.icon(ui.ctx()).id();
        egui::Panel::top("title bar")
            .frame(egui::Frame::NONE.fill(theme::window_bg()))
            .show_separator_line(false)
            .exact_size(chrome.height)
            .show(ui, |ui| {
                let rect = ui.max_rect();
                let response = ui.interact(rect, ui.scope_id().with("drag"), Sense::click_and_drag());
                if response.drag_started_by(egui::PointerButton::Primary) {
                    ui.ctx().send_viewport_cmd(egui::ViewportCommand::StartDrag);
                }
                if response.double_clicked() {
                    double_click(frame);
                }

                let (name, edited) = title_parts(name, edited);
                let font = egui::FontId::proportional(13.0);
                let text = ui.visuals().text_color();
                let painter = ui.painter();
                let name = painter.layout_no_wrap(name, font.clone(), text);
                let edited = edited.map(|e| painter.layout_no_wrap(e.to_string(), font, theme::text_dim()));
                let gap = 6.0;
                let width = ICON + gap + name.size().x + edited.as_ref().map_or(0.0, |g| g.size().x);
                // Centred in the window like a native title, but clear of the buttons.
                let left = (rect.center().x - width / 2.0).max(rect.left() + chrome.buttons_right + 12.0);
                let cy = rect.center().y;
                let icon_rect = egui::Rect::from_min_size(pos2(left, cy - ICON / 2.0), vec2(ICON, ICON));
                let uv = egui::Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
                painter.image(icon, icon_rect, uv, Color32::WHITE);
                let x = left + ICON + gap;
                let text_y = cy - name.size().y / 2.0;
                let name_w = name.size().x;
                painter.galley(pos2(x, text_y), name, text);
                if let Some(e) = edited {
                    painter.galley(pos2(x + name_w, text_y), e, theme::text_dim());
                }
            });
    }

    fn icon(&mut self, ctx: &egui::Context) -> &TextureHandle {
        self.icon.get_or_insert_with(|| {
            // 64² (4× the drawn size): sharp on Retina with linear filtering.
            crate::icons::app_icon(ctx, 64)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn double_click_follows_the_system_setting() {
        assert_eq!(DoubleClick::from_setting(None), DoubleClick::Zoom);
        assert_eq!(DoubleClick::from_setting(Some("Maximize")), DoubleClick::Zoom);
        assert_eq!(DoubleClick::from_setting(Some("Fill")), DoubleClick::Zoom);
        assert_eq!(DoubleClick::from_setting(Some("Minimize")), DoubleClick::Minimize);
        assert_eq!(DoubleClick::from_setting(Some("None")), DoubleClick::Nothing);
    }

    #[test]
    fn title_marks_unsaved_changes() {
        assert_eq!(title_parts("Demo", false), ("Demo".to_string(), None));
        assert_eq!(title_parts("Demo", true).1, Some(" — Edited"));
    }
}
