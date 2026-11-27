//! Native file dialogs that don't block the frame.
//!
//! A blocking dialog (`rfd::FileDialog`) runs a nested event loop inside winit's event
//! handler. On macOS the save panel can then deliver an event winit queued meanwhile,
//! and winit aborts ("tried to handle event while another event is currently being
//! handled"). So the app opens file dialogs with rfd's async API: on macOS a sheet on
//! the window, elsewhere a dialog on another thread. The frame goes on, and the answer
//! arrives in a later frame through [`Dialog::poll`], together with what to do with it.
//!
//! Message dialogs (`rfd::MessageDialog`) stay blocking: an alert runs in the modal
//! panel run loop mode, which winit's queued events don't run in.

use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll, Wake, Waker};

#[cfg(not(target_arch = "wasm32"))]
fn path(file: &rfd::FileHandle) -> PathBuf {
    file.path().to_path_buf()
}

/// A browser hands over the file, not a path: only its name (W2 reads files).
#[cfg(target_arch = "wasm32")]
fn path(file: &rfd::FileHandle) -> PathBuf {
    file.file_name().into()
}

/// What the dialog asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Save,
    PickFile,
    PickFolder,
}

type Answer = Pin<Box<dyn Future<Output = Option<PathBuf>>>>;

/// At most one open file dialog, and `T`: what to do with the chosen path.
pub struct Dialog<T> {
    pending: Option<(T, Answer, Waker)>,
}

impl<T> Default for Dialog<T> {
    fn default() -> Self {
        Self { pending: None }
    }
}

/// Wakes the app when the dialog is answered (macOS calls back on the main thread,
/// other systems from the dialog's thread).
struct Repaint(egui::Context);

impl Wake for Repaint {
    fn wake(self: Arc<Self>) {
        self.0.request_repaint();
    }
}

impl<T> Dialog<T> {
    pub fn is_open(&self) -> bool {
        self.pending.is_some()
    }

    /// Show `dialog`; `then` comes back with the answer. Returns false (showing
    /// nothing) while another dialog is still open.
    pub fn open(&mut self, ctx: &egui::Context, dialog: rfd::AsyncFileDialog, kind: Kind, then: T) -> bool {
        if self.is_open() {
            return false;
        }
        let file = match kind {
            Kind::Save => Box::pin(dialog.save_file()) as Pin<Box<dyn Future<Output = _>>>,
            Kind::PickFile => Box::pin(dialog.pick_file()),
            #[cfg(not(target_arch = "wasm32"))]
            Kind::PickFolder => Box::pin(dialog.pick_folder()),
            // Browsers have no folder picker in rfd.
            #[cfg(target_arch = "wasm32")]
            Kind::PickFolder => Box::pin(std::future::ready(None)),
        };
        self.wait(ctx, Box::pin(async move { file.await.map(|f| path(&f)) }), then);
        true
    }

    /// Answer at once with `path` instead of showing a dialog (scripted runs).
    pub fn answered(&mut self, ctx: &egui::Context, path: Option<PathBuf>, then: T) -> bool {
        if self.is_open() {
            return false;
        }
        self.wait(ctx, Box::pin(std::future::ready(path)), then);
        true
    }

    fn wait(&mut self, ctx: &egui::Context, answer: Answer, then: T) {
        self.pending = Some((then, answer, Waker::from(Arc::new(Repaint(ctx.clone())))));
    }

    /// The answer once the dialog closed: what to do, and the path (`None` when
    /// cancelled).
    pub fn poll(&mut self) -> Option<(T, Option<PathBuf>)> {
        let (_, answer, waker) = self.pending.as_mut()?;
        let Poll::Ready(file) = answer.as_mut().poll(&mut Context::from_waker(waker)) else { return None };
        let (then, ..) = self.pending.take()?;
        Some((then, file))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_answer_comes_back_with_what_it_is_for() {
        let ctx = egui::Context::default();
        let mut dialog = Dialog::default();
        assert!(dialog.poll().is_none());
        dialog.wait(&ctx, Box::pin(std::future::pending()), "open");
        assert!(dialog.is_open());
        assert!(dialog.poll().is_none(), "still open");
        assert!(dialog.is_open());

        dialog.wait(&ctx, Box::pin(std::future::ready(Some(PathBuf::from("/a")))), "save");
        assert_eq!(dialog.poll(), Some(("save", Some(PathBuf::from("/a")))));
        assert!(!dialog.is_open());
        assert!(dialog.poll().is_none(), "answered once");

        dialog.wait(&ctx, Box::pin(std::future::ready(None)), "cancelled");
        assert_eq!(dialog.poll(), Some(("cancelled", None)));
    }
}
