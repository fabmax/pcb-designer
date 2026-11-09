//! A [`Host`] without a GUI: a document plus selections. Used by tests and by the headless
//! MCP mode.
//!
//! Libraries are kept open in memory (the starter library first). KiCad imports go into
//! the named library, else a "KiCad import" library, and their parts straight into the
//! project (there is nobody to place them from a library window).

use std::path::PathBuf;

use editor_core::Command;
use kicad::Catalog;
use kicad::download::{self, Source};
use model::{Library, ObjectId, builtin};
use render::gpu::Background;
use render::offscreen::Offscreen;
use serde_json::Value;
use view::{Document, LibraryDoc, ViewKind};

use crate::host::{
    Host, KicadImport, Reply, Responder, kicad_download_reply, kicad_import_reply, kicad_request, render_png,
};
use crate::library::{Libraries, LibraryRequest};

pub struct HeadlessHost {
    pub doc: Document,
    /// Open libraries: the starter library, then the ones agents created or opened.
    pub libraries: Vec<LibraryDoc>,
    /// Download folder of KiCad's libraries (`None`: downloads are refused).
    pub kicad_root: Option<PathBuf>,
    /// More KiCad library folders, always searched.
    pub kicad_paths: Vec<PathBuf>,
    pub kicad_source: Source,
    catalog: Option<Catalog>,
    selection: [Vec<ObjectId>; 2],
    offscreen: Option<Offscreen>,
}

impl HeadlessHost {
    pub fn new(doc: Document) -> Self {
        HeadlessHost {
            doc,
            libraries: vec![LibraryDoc::new(builtin::starter_library())],
            kicad_root: None,
            kicad_paths: Vec::new(),
            kicad_source: Source::default(),
            catalog: None,
            selection: Default::default(),
            offscreen: None,
        }
    }

    fn catalog(&mut self) -> &mut Catalog {
        let paths: Vec<PathBuf> =
            self.kicad_paths.iter().cloned().chain(download::library_paths(self.kicad_root.as_deref())).collect();
        self.catalog.get_or_insert_with(|| Catalog::scan(&paths))
    }

    /// The open library named `name`, or a new one.
    fn library_named(&mut self, name: &str) -> &mut LibraryDoc {
        let i = match self.libraries.iter().position(|l| l.name() == name) {
            Some(i) => i,
            None => {
                let mut lib = LibraryDoc::new(Library::new(name));
                lib.doc.mark_saved();
                self.libraries.push(lib);
                self.libraries.len() - 1
            }
        };
        &mut self.libraries[i]
    }

    fn import(&mut self, job: &KicadImport) -> Result<Value, String> {
        let req = kicad_request(job, |symbol| {
            let c = self.catalog();
            let sym = c.load_symbol(symbol).map_err(|e| e.to_string())?;
            Ok(kicad::import::suggested_footprints(c, &sym))
        })?;
        let imported = kicad::import(self.catalog(), &req);
        if imported.symbols.is_empty() && imported.footprints.is_empty() {
            return Err(if imported.errors.is_empty() {
                "nothing to import".into()
            } else {
                imported.errors.join("; ")
            });
        }
        let target = self.library_named(job.library.as_deref().unwrap_or("KiCad import"));
        let (added, replaced) =
            target.upsert(imported.symbols.clone(), imported.footprints.clone(), imported.parts.clone())?;
        let (library, name) = (target.library().clone(), target.name().to_string());
        let parts: Vec<model::Id> = imported.parts.iter().map(|p| p.id).collect();
        if !parts.is_empty() {
            self.doc.execute(Command::ImportParts { library, parts }).map_err(|e| e.to_string())?;
        }
        let mut reply = kicad_import_reply(&name, added, replaced, &imported, &req);
        reply["in_project"] = Value::Bool(!imported.parts.is_empty());
        Ok(reply)
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
        self.libraries.iter().map(LibraryDoc::library).collect()
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

    fn kicad_search(&mut self, query: &str, footprints: bool, limit: usize) -> Result<Value, String> {
        let c = self.catalog();
        if c.symbol_count() + c.footprint_count() == 0 {
            return Err("no KiCad libraries found; download them with download_kicad_libraries".into());
        }
        let results = if footprints { c.search_footprints(query, limit) } else { c.search_symbols(query, limit) };
        Ok(serde_json::json!({ "results": results, "truncated": results.len() >= limit }))
    }

    fn library_request(&mut self, request: LibraryRequest, reply: Responder) {
        let _ = reply.send(crate::library::respond(request, self));
    }

    fn kicad_import(&mut self, request: KicadImport, reply: Responder) {
        let _ = reply.send(self.import(&request).map(Reply::Json));
    }

    fn kicad_download(&mut self, update: bool, reply: Responder) {
        let result = match &self.kicad_root {
            None => Err("no folder to download KiCad libraries to".to_string()),
            Some(root) => download::ensure(root, update, None, &self.kicad_source, &mut |_| true)
                .map(|(installed, fresh)| Reply::Json(kicad_download_reply(&installed, fresh)))
                .map_err(|e| e.to_string()),
        };
        if result.is_ok() {
            self.catalog = None;
        }
        let _ = reply.send(result);
    }
}

impl Libraries for HeadlessHost {
    fn count(&self) -> usize {
        self.libraries.len()
    }

    fn get(&mut self, index: usize) -> &mut LibraryDoc {
        &mut self.libraries[index]
    }

    fn add(&mut self, lib: LibraryDoc) -> usize {
        self.libraries.push(lib);
        self.libraries.len() - 1
    }

    fn offscreen(&mut self) -> &mut Option<Offscreen> {
        &mut self.offscreen
    }
}
