//! The system clipboard. egui writes it (`ctx.copy_text`) and hands keyboard pastes over
//! as `Event::Paste`; menu picks read it here.

/// The clipboard's text, if it has any.
pub fn read() -> Option<String> {
    arboard::Clipboard::new().ok()?.get_text().ok()
}
