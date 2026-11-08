//! MCP integration: runs the server and answers agent requests on the GUI thread.

use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use mcp::host::{Responder, render_png};
use mcp::{Call, Host, McpServer, Reply};
use model::{Library, ObjectId};
use render::offscreen::Offscreen;
use view::{Document, ViewKind};

use crate::app::Pane;

pub const DEFAULT_PORT: u16 = 7340;

/// Server state owned by the app.
pub struct Agent {
    server: Option<(McpServer, Receiver<Call>)>,
    pub port: u16,
    offscreen: Option<Offscreen>,
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
    pub fn process(&mut self, ctx: &egui::Context, doc: &mut Document, panes: &mut [Pane; 2], library: &Library) {
        let Some((_, rx)) = &self.server else { return };
        let mut host = AppHost { ctx, doc, panes, library, offscreen: &mut self.offscreen };
        while let Ok(call) = rx.try_recv() {
            self.calls += 1;
            let description = call.request.describe();
            log::info!("MCP: {description}");
            if call.request.modifies() {
                self.last = Some((description, Instant::now()));
            }
            mcp::handle(call, &mut host);
        }
    }
}

struct AppHost<'a> {
    ctx: &'a egui::Context,
    doc: &'a mut Document,
    panes: &'a mut [Pane; 2],
    library: &'a Library,
    offscreen: &'a mut Option<Offscreen>,
}

impl Host for AppHost<'_> {
    fn document(&mut self) -> &mut Document {
        self.doc
    }

    fn library(&self) -> &Library {
        self.library
    }

    fn selection(&self, view: ViewKind) -> Vec<ObjectId> {
        self.panes[view as usize].it.selection.clone()
    }

    fn set_selection(&mut self, view: ViewKind, ids: Vec<ObjectId>) {
        self.panes[view as usize].it.set_selection(&mut self.doc.target(view), ids);
    }

    fn render_view(&mut self, view: ViewKind, fit: bool, width: u32, height: u32) -> Result<Vec<u8>, String> {
        let canvas = &self.panes[view as usize].canvas;
        // The user's camera, scaled so the image shows the same region as the canvas.
        let camera = (!fit).then(|| {
            let mut cam = canvas.camera;
            let [vw, vh] = canvas.viewport();
            cam.zoom *= (width as f64 / vw.max(1.0)).min(height as f64 / vh.max(1.0));
            cam
        });
        render_png(self.offscreen, &self.doc.view(view).scene, camera, &canvas.background, width, height)
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
