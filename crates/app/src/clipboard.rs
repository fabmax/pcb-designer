//! The system clipboard. egui writes it (`ctx.copy_text`) and hands keyboard pastes over
//! as `Event::Paste`; menu picks read it here.

/// The clipboard's text, if it has any.
#[cfg(not(target_arch = "wasm32"))]
pub fn read() -> Option<String> {
    arboard::Clipboard::new().ok()?.get_text().ok()
}

/// A browser only hands the clipboard over in a paste event (see the module docs).
#[cfg(target_arch = "wasm32")]
pub fn read() -> Option<String> {
    None
}
