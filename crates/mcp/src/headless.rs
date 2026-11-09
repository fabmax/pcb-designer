//! A [`Host`] without a GUI: a document plus selections. Used by tests and, later, by the
//! headless MCP mode.

use model::{Library, ObjectId, builtin};
use render::gpu::Background;
use render::offscreen::Offscreen;
use view::{Document, ViewKind};

use crate::host::{Host, render_png};

pub struct HeadlessHost {
    pub doc: Document,
    pub library: Library,
    selection: [Vec<ObjectId>; 2],
    offscreen: Option<Offscreen>,
}

impl HeadlessHost {
    pub fn new(doc: Document) -> Self {
        HeadlessHost { doc, library: builtin::starter_library(), selection: Default::default(), offscreen: None }
    }
}

pub fn background(view: ViewKind) -> Background {
    let grid = match view {
        ViewKind::Schematic => 2.54,
        ViewKind::Board => 1.0,
    };
    view::style::background(grid)
}

impl Host for HeadlessHost {
    fn document(&mut self) -> &mut Document {
        &mut self.doc
    }

    fn libraries(&self) -> Vec<&Library> {
        vec![&self.library]
    }

    fn selection(&self, view: ViewKind) -> Vec<ObjectId> {
        self.selection[view as usize].clone()
    }

    fn set_selection(&mut self, view: ViewKind, ids: Vec<ObjectId>) {
        let (project, v) = self.doc.project_and_view(view);
        v.set_highlight(project, &ids, None);
        self.selection[view as usize] = ids;
    }

    fn render_view(&mut self, view: ViewKind, _fit: bool, width: u32, height: u32) -> Result<Vec<u8>, String> {
        render_png(&mut self.offscreen, &self.doc.view(view).scene, None, &background(view), width, height)
    }
}
