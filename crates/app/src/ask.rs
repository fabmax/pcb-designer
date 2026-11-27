//! Questions in message boxes (blocking, see `file_dialog.rs`). In the browser rfd shows
//! `window.confirm`, which has OK and Cancel only and answers `Ok`, not the custom
//! button: these helpers give the same answers on both.

/// Asks `description`, with `ok` (e.g. "Delete") or Cancel. Whether `ok` was picked.
pub fn confirm(title: &str, description: &str, ok: &str) -> bool {
    let answer = rfd::MessageDialog::new()
        .set_title(title)
        .set_description(description)
        .set_buttons(rfd::MessageButtons::OkCancelCustom(ok.into(), "Cancel".into()))
        .show();
    match answer {
        rfd::MessageDialogResult::Custom(b) => b == ok,
        rfd::MessageDialogResult::Ok => cfg!(target_arch = "wasm32"),
        _ => false,
    }
}

/// Asks whether to save unsaved changes: `Yes` saves, `No` discards, `Cancel` keeps
/// them open. In the browser two OK/Cancel questions: save? if not, discard?
pub fn save(question: &str) -> rfd::MessageDialogResult {
    let ask = |description: &str, buttons| {
        rfd::MessageDialog::new()
            .set_level(rfd::MessageLevel::Warning)
            .set_title("Unsaved changes")
            .set_description(description)
            .set_buttons(buttons)
            .show()
    };
    if !cfg!(target_arch = "wasm32") {
        return ask(question, rfd::MessageButtons::YesNoCancel);
    }
    use rfd::MessageDialogResult::{Cancel, No, Ok, Yes};
    if ask(&format!("{question}\nOK saves them."), rfd::MessageButtons::OkCancel) == Ok {
        return Yes;
    }
    match ask("Discard the changes?", rfd::MessageButtons::OkCancel) {
        Ok => No,
        _ => Cancel,
    }
}
