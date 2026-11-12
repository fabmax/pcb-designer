//! MCP integration: runs the server and answers agent requests on the GUI thread.

use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use mcp::host::{KicadImport as KicadJob, Responder, render_png};
use mcp::library::LibraryRequest;
use mcp::{Call, Host, McpServer, Reply};
use model::{Library, ObjectId};
use render::offscreen::Offscreen;
use serde_json::{Value, json};
use view::{Document, ViewId};

use crate::app::Panes;
use crate::kicad_import::KicadImport;

pub const DEFAULT_PORT: u16 = 7340;

/// Requests the app answers itself after [`Agent::process`]: they open library tabs.
pub enum Job {
    Kicad(KicadJob, Responder),
    Library(LibraryRequest, Responder),
}

/// Server state owned by the app.
pub struct Agent {
    server: Option<(McpServer, Receiver<Call>)>,
    pub port: u16,
    pub offscreen: Option<Offscreen>,
    pub calls: usize,
    pub last: Option<(String, Instant)>,
    pub error: Option<String>,
}

impl Agent {
    pub fn new(port: u16) -> Self {
        Agent { server: None, port, offscreen: None, calls: 0, last: None, error: None }
    }

    pub fn is_running(&self) -> bool {
        self.server.is_some()
    }

    pub fn url(&self) -> Option<String> {
        self.server.as_ref().map(|(s, _)| s.url())
    }

    pub fn start(&mut self, ctx: &egui::Context) {
        if self.server.is_some() {
            return;
        }
        let ctx = ctx.clone();
        match McpServer::start(self.port, Arc::new(move || ctx.request_repaint())) {
            Ok(s) => {
                self.error = None;
                self.server = Some(s);
            }
            Err(e) => self.error = Some(e),
        }
    }

    pub fn stop(&mut self) {
        self.server = None;
    }

    /// Answer queued requests. Call once per frame when no interaction is in progress.
    /// KiCad imports and library requests are returned for the app to run (they change
    /// the open libraries).
    pub fn process(
        &mut self,
        ctx: &egui::Context,
        doc: &mut Document,
        panes: &mut Panes,
        libraries: &[&Library],
        kicad: &mut KicadImport,
    ) -> Vec<Job> {
        let Some((_, rx)) = &self.server else { return Vec::new() };
        let mut jobs = Vec::new();
        let mut host = AppHost { ctx, doc, panes, libraries, offscreen: &mut self.offscreen, kicad, jobs: &mut jobs };
        while let Ok(call) = rx.try_recv() {
            self.calls += 1;
            let description = call.request.describe();
            log::info!("MCP: {description}");
            if call.request.modifies() {
                self.last = Some((description, Instant::now()));
            }
            mcp::handle(call, &mut host);
        }
        jobs
    }
}

struct AppHost<'a> {
    ctx: &'a egui::Context,
    doc: &'a mut Document,
    panes: &'a mut Panes,
    libraries: &'a [&'a Library],
    offscreen: &'a mut Option<Offscreen>,
    kicad: &'a mut KicadImport,
    jobs: &'a mut Vec<Job>,
}

impl Host for AppHost<'_> {
    fn document(&self) -> &Document {
        self.doc
    }

    fn document_mut(&mut self) -> &mut Document {
        self.doc
    }

    fn libraries(&self) -> Vec<&Library> {
        self.libraries.to_vec()
    }

    fn selection(&self, view: ViewId) -> Vec<ObjectId> {
        self.panes.selection(view)
    }

    fn set_selection(&mut self, view: ViewId, ids: Vec<ObjectId>) {
        self.panes.get(view).it.set_selection(&mut self.doc.target(view), ids);
    }

    fn render_view(&mut self, view: ViewId, fit: bool, width: u32, height: u32) -> Result<Vec<u8>, String> {
        let canvas = &self.panes.get(view).canvas;
        // The user's camera, scaled so the image shows the same region as the canvas.
        let camera = (!fit).then(|| {
            let mut cam = canvas.camera;
            let [vw, vh] = canvas.viewport();
            cam.zoom *= (width as f64 / vw.max(1.0)).min(height as f64 / vh.max(1.0));
            cam
        });
        render_png(self.offscreen, &self.doc.view(view).scene, camera, &canvas.background, width, height)
    }

    fn render_scene(&mut self, scene: &render::Scene, width: u32, height: u32) -> Result<Vec<u8>, String> {
        let background = mcp::headless::background(view::ViewKind::Board);
        render_png(self.offscreen, scene, None, &background, width, height)
    }

    fn kicad_search(&mut self, query: &str, footprints: bool, limit: usize) -> Result<Value, String> {
        let (results, symbols, fps) = self.kicad.search(query, footprints, limit);
        if symbols + fps == 0 {
            return Err("no KiCad libraries found; download them with download_kicad_libraries".into());
        }
        Ok(json!({ "results": results, "truncated": results.len() >= limit }))
    }

    fn kicad_import(&mut self, request: KicadJob, reply: Responder) {
        self.jobs.push(Job::Kicad(request, reply));
    }

    fn library_request(&mut self, request: LibraryRequest, reply: Responder) {
        self.jobs.push(Job::Library(request, reply));
    }

    fn kicad_download(&mut self, update: bool, reply: Responder) {
        self.kicad.start_download(self.ctx, update, None, Some(reply));
    }

    fn window_screenshot(&mut self, reply: Responder) {
        let reply = Mutex::new(Some(reply));
        self.ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::ScreenshotCallback::new(move |img| {
            let rgba = img.pixels.iter().flat_map(|c| c.to_srgba_unmultiplied()).collect();
            let image = render::offscreen::Image { width: img.size[0] as u32, height: img.size[1] as u32, rgba };
            if let Some(reply) = reply.lock().unwrap().take() {
                let _ = reply.send(Ok(Reply::Png(image.encode_png())));
            }
        })));
        self.ctx.request_repaint();
    }
}
