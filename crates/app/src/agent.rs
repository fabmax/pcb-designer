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
use crate::sim_ui::SimPanel;

pub const DEFAULT_PORT: u16 = 7340;

/// Requests the app answers itself after [`Agent::process`]: they open library tabs.
pub enum Job {
    Kicad(KicadJob, Responder),
    Library(LibraryRequest, Responder),
    /// An imported KiCad project to replace the open one (after asking about unsaved
    /// changes).
    ReplaceProject(Box<kicad::ProjectImport>, Responder),
}

/// One agent call in the Agent activity panel.
#[derive(Clone, Debug)]
pub struct ActivityEntry {
    pub at: Instant,
    /// The tool (the request's name in snake case).
    pub tool: String,
    pub summary: String,
    /// The request's arguments (truncated).
    pub args: String,
    /// The result or the error; `None`: answered later (library tabs, screenshots).
    pub result: Option<Result<String, String>>,
    /// The undo step the call made.
    pub undo: Option<String>,
}

/// Most entries the activity log keeps.
pub const ACTIVITY: usize = 500;

/// `SetCheckSettings { … }` → `set_check_settings`.
fn tool_name(debug: &str) -> String {
    let name: String = debug.chars().take_while(|c| c.is_alphanumeric()).collect();
    let mut out = String::new();
    for (i, c) in name.chars().enumerate() {
        if c.is_uppercase() && i > 0 {
            out.push('_');
        }
        out.extend(c.to_lowercase());
    }
    out
}

fn truncated(mut text: String, max: usize) -> String {
    if text.len() > max {
        let mut end = max;
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        text.truncate(end);
        text.push('…');
    }
    text
}

/// Server state owned by the app.
pub struct Agent {
    server: Option<(McpServer, Receiver<Call>)>,
    pub port: u16,
    pub offscreen: Option<Offscreen>,
    pub calls: usize,
    pub last: Option<(String, Instant)>,
    pub error: Option<String>,
    /// Calls of this session, oldest first.
    pub activity: std::collections::VecDeque<ActivityEntry>,
}

impl Agent {
    pub fn new(port: u16) -> Self {
        Agent { server: None, port, offscreen: None, calls: 0, last: None, error: None, activity: Default::default() }
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
        sim: (&mut SimPanel, Option<std::path::PathBuf>),
    ) -> Vec<Job> {
        let Some((_, rx)) = &self.server else { return Vec::new() };
        let mut jobs = Vec::new();
        let (sim, ngspice) = sim;
        let offscreen = &mut self.offscreen;
        let mut host = AppHost { ctx, doc, panes, libraries, offscreen, kicad, jobs: &mut jobs, sim, ngspice };
        while let Ok(call) = rx.try_recv() {
            self.calls += 1;
            let description = call.request.describe();
            log::info!("MCP: {description}");
            let debug = format!("{:#?}", call.request);
            if call.request.modifies() {
                self.last = Some((description.clone(), Instant::now()));
            }
            let revision = host.doc.revision;
            let outcome = mcp::handle(call, &mut host);
            let undo = (host.doc.revision != revision)
                .then(|| host.doc.editor().history().undo_label().map(str::to_string))
                .flatten();
            if self.activity.len() == ACTIVITY {
                self.activity.pop_front();
            }
            self.activity.push_back(ActivityEntry {
                at: Instant::now(),
                tool: tool_name(&debug),
                summary: description,
                args: truncated(debug, 4000),
                result: match outcome {
                    mcp::Outcome::Answered(r) => Some(r),
                    mcp::Outcome::Deferred => None,
                },
                undo,
            });
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
    sim: &'a mut SimPanel,
    /// The ngspice setting.
    ngspice: Option<std::path::PathBuf>,
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

    fn render_scene(
        &mut self,
        scene: &render::Scene,
        camera: Option<render::Camera>,
        width: u32,
        height: u32,
    ) -> Result<Vec<u8>, String> {
        let background = mcp::headless::background(view::ViewKind::Board);
        render_png(self.offscreen, scene, camera, &background, width, height)
    }

    fn shown_analysis(&self) -> Option<model::Id> {
        self.sim.current(&self.doc.project().schematic.simulation).map(|a| a.id)
    }

    fn sim_result(&self, analysis: model::Id) -> Option<&sim::Finished> {
        self.sim.session.result(analysis)
    }

    fn simulate(&mut self, job: mcp::simulation::SimJob, reply: Responder) {
        self.sim.start_agent(self.doc, job, reply, self.ngspice.clone());
        self.ctx.request_repaint();
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

    fn replace_project(&mut self, import: kicad::ProjectImport, reply: Responder) {
        self.jobs.push(Job::ReplaceProject(Box::new(import), reply));
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_names_and_truncation() {
        assert_eq!(tool_name("SetCheckSettings {\n x: 1 }"), "set_check_settings");
        assert_eq!(tool_name("Project"), "project");
        assert_eq!(truncated("äöü".into(), 3), "ä…");
        assert_eq!(truncated("ok".into(), 3), "ok");
    }
}
