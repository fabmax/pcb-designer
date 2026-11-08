//! MCP tools and the Streamable HTTP server (127.0.0.1 only).
//!
//! The server runs on its own thread with a small tokio runtime. Every tool call becomes
//! a [`Call`] sent to the thread that owns the document; `wake` makes that thread look at
//! its queue (e.g. requests an egui repaint).

use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::JoinHandle;
use std::time::Duration;

use base64::Engine;
use editor_core::Command;
use model::{Id, ObjectId, ObjectKind};
use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolResult, ContentBlock, Implementation, ServerCapabilities, ServerConfig};
use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
use rmcp::transport::streamable_http_server::{StreamableHttpServerConfig, StreamableHttpService};
use rmcp::{ErrorData, ServerHandler, tool, tool_handler, tool_router};
use schemars::JsonSchema;
use serde::Deserialize;
use tokio::sync::oneshot;
use tokio_util::sync::CancellationToken;
use view::ViewKind;

use crate::host::{Call, Reply, Request};

pub type Waker = Arc<dyn Fn() + Send + Sync>;

const INSTRUCTIONS: &str = "\
PCB Designer: inspect and edit the project open in the app. The user sees every change live.

Units: coordinates and sizes are integers in nanometers (1 mm = 1000000, 1 mil = 25400); \
angles are degrees, counter-clockwise; Y points up.

Typical workflow:
1. get_project for an overview (components, layers, design rules, outline).
2. list_library_parts, then import_parts to copy parts into the project.
3. execute commands (place_part, add_wire, add_label, add_track, move, rotate, ...). Several \
   commands in one call form a single undo step and are applied all-or-nothing.
4. screenshot the schematic or board to check the result visually.

Schematic and board are linked: placing a part creates its footprint on the board automatically \
(in a staging area right of the board outline), deleting the component removes it. Footprints \
cannot be created or deleted directly, only moved/rotated/flipped. Schematic connectivity is \
by wire endpoints touching pin ends and by labels with equal names.";

#[derive(Clone, Copy, Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ViewName {
    Schematic,
    Board,
}

impl From<ViewName> for ViewKind {
    fn from(v: ViewName) -> Self {
        match v {
            ViewName::Schematic => ViewKind::Schematic,
            ViewName::Board => ViewKind::Board,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ScreenshotTarget {
    Schematic,
    Board,
    /// The whole application window as the user sees it (GUI only).
    Window,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ImportPartsParams {
    /// Part ids from list_library_parts.
    pub parts: Vec<Id>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListObjectsParams {
    /// Only objects of this kind. Without it, all schematic and board objects (library
    /// contents excluded).
    #[serde(default)]
    pub kind: Option<ObjectKind>,
    #[serde(default)]
    pub offset: Option<usize>,
    /// Maximum number of objects (default 100, at most 1000).
    #[serde(default)]
    pub limit: Option<usize>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct GetObjectsParams {
    pub ids: Vec<ObjectId>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ComponentParams {
    /// Reference designator, e.g. "R1".
    pub refdes: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ExecuteParams {
    /// Commands to run in order, as one undo step. If one fails, none is applied.
    pub commands: Vec<Command>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ViewParams {
    pub view: ViewName,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SetSelectionParams {
    pub view: ViewName,
    /// Objects to select (replaces the selection; empty clears it).
    pub ids: Vec<ObjectId>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ScreenshotParams {
    pub target: ScreenshotTarget,
    /// For views: true (default) shows everything, false shows what the user currently
    /// sees in that view.
    #[serde(default)]
    pub fit: Option<bool>,
    /// Image size for views in pixels (default 1200x800, 64..4096).
    #[serde(default)]
    pub width: Option<u32>,
    #[serde(default)]
    pub height: Option<u32>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SaveParams {
    /// Project folder to save to. Defaults to the current folder.
    #[serde(default)]
    pub path: Option<String>,
}

/// The MCP service; one per session, all sharing the same call queue.
#[derive(Clone)]
pub struct PcbTools {
    calls: Sender<Call>,
    wake: Waker,
    #[allow(dead_code, reason = "read by the tool_handler macro")]
    tool_router: ToolRouter<Self>,
}

impl PcbTools {
    pub fn new(calls: Sender<Call>, wake: Waker) -> Self {
        PcbTools { calls, wake, tool_router: Self::tool_router() }
    }

    async fn call(&self, request: Request) -> Result<CallToolResult, ErrorData> {
        let (tx, rx) = oneshot::channel();
        if self.calls.send(Call { request, reply: tx }).is_err() {
            return Ok(error("the application is shutting down"));
        }
        (self.wake)();
        let result = match tokio::time::timeout(Duration::from_secs(60), rx).await {
            Ok(Ok(r)) => r,
            Ok(Err(_)) => Err("the application dropped the request".into()),
            Err(_) => Err("timed out waiting for the application (a dialog may be open)".into()),
        };
        Ok(match result {
            Ok(Reply::Json(v)) => CallToolResult::success(vec![ContentBlock::text(
                serde_json::to_string_pretty(&v).unwrap_or_else(|e| e.to_string()),
            )]),
            Ok(Reply::Png(png)) => CallToolResult::success(vec![ContentBlock::image(
                base64::engine::general_purpose::STANDARD.encode(png),
                "image/png",
            )]),
            Err(e) => error(e),
        })
    }
}

fn error(msg: impl Into<String>) -> CallToolResult {
    CallToolResult::error(vec![ContentBlock::text(msg.into())])
}

#[tool_router]
impl PcbTools {
    #[tool(description = "Overview of the open project: name, file path, unsaved changes, undo/redo, \
        object counts, sheets, components, board layers, design rules and outline bounds.")]
    async fn get_project(&self) -> Result<CallToolResult, ErrorData> {
        self.call(Request::Project).await
    }

    #[tool(description = "Parts available for import: ids, pin names per unit and footprint variants.")]
    async fn list_library_parts(&self) -> Result<CallToolResult, ErrorData> {
        self.call(Request::LibraryParts).await
    }

    #[tool(description = "Copy parts from the library into the project so they can be placed with \
        the place_part command. Already imported parts are skipped.")]
    async fn import_parts(&self, Parameters(p): Parameters<ImportPartsParams>) -> Result<CallToolResult, ErrorData> {
        self.call(Request::ImportParts { parts: p.parts }).await
    }

    #[tool(description = "List project objects (optionally of one kind), paginated. Objects are \
        returned in full JSON; ids are {\"type\", \"id\"} references used by other tools.")]
    async fn list_objects(&self, Parameters(p): Parameters<ListObjectsParams>) -> Result<CallToolResult, ErrorData> {
        let limit = p.limit.unwrap_or(100).clamp(1, 1000);
        self.call(Request::ListObjects { kind: p.kind, offset: p.offset.unwrap_or(0), limit }).await
    }

    #[tool(description = "Get objects by id (null for ids that don't exist).")]
    async fn get_objects(&self, Parameters(p): Parameters<GetObjectsParams>) -> Result<CallToolResult, ErrorData> {
        self.call(Request::GetObjects { ids: p.ids }).await
    }

    #[tool(description = "Everything about one component: part, variant, symbol placement with \
        world positions of its pin ends (where wires connect), footprint placement with pad \
        positions, and the pin to pad mapping.")]
    async fn get_component(&self, Parameters(p): Parameters<ComponentParams>) -> Result<CallToolResult, ErrorData> {
        self.call(Request::Component { refdes: p.refdes }).await
    }

    #[tool(description = "Execute editing commands (the same ones the GUI uses) as a single undo \
        step, all-or-nothing. Returns the changed object ids and the created objects.")]
    async fn execute(&self, Parameters(p): Parameters<ExecuteParams>) -> Result<CallToolResult, ErrorData> {
        self.call(Request::Execute { commands: p.commands }).await
    }

    #[tool(description = "Undo the last change (made by the user or an agent).")]
    async fn undo(&self) -> Result<CallToolResult, ErrorData> {
        self.call(Request::Undo).await
    }

    #[tool(description = "Redo the last undone change.")]
    async fn redo(&self) -> Result<CallToolResult, ErrorData> {
        self.call(Request::Redo).await
    }

    #[tool(description = "Objects the user has selected in a view.")]
    async fn get_selection(&self, Parameters(p): Parameters<ViewParams>) -> Result<CallToolResult, ErrorData> {
        self.call(Request::GetSelection { view: p.view.into() }).await
    }

    #[tool(description = "Select objects in a view (highlights them for the user).")]
    async fn set_selection(&self, Parameters(p): Parameters<SetSelectionParams>) -> Result<CallToolResult, ErrorData> {
        self.call(Request::SetSelection { view: p.view.into(), ids: p.ids }).await
    }

    #[tool(description = "PNG image of the schematic, the board, or the whole application window.")]
    async fn screenshot(&self, Parameters(p): Parameters<ScreenshotParams>) -> Result<CallToolResult, ErrorData> {
        let target = match p.target {
            ScreenshotTarget::Schematic => Some(ViewKind::Schematic),
            ScreenshotTarget::Board => Some(ViewKind::Board),
            ScreenshotTarget::Window => None,
        };
        let request = Request::Screenshot {
            target,
            fit: p.fit.unwrap_or(true),
            width: p.width.unwrap_or(1200),
            height: p.height.unwrap_or(800),
        };
        self.call(request).await
    }

    #[tool(description = "Save the project folder (to its current location or to `path`).")]
    async fn save_project(&self, Parameters(p): Parameters<SaveParams>) -> Result<CallToolResult, ErrorData> {
        self.call(Request::Save { path: p.path.map(Into::into) }).await
    }
}

#[tool_handler]
impl ServerHandler for PcbTools {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("pcb-designer", env!("CARGO_PKG_VERSION")))
            .with_instructions(INSTRUCTIONS)
    }
}

/// A running MCP server. Stops when dropped.
pub struct McpServer {
    addr: SocketAddr,
    cancel: CancellationToken,
    thread: Option<JoinHandle<()>>,
}

impl McpServer {
    /// Listen on `127.0.0.1:port` (0 picks a free port). Calls arrive on the returned
    /// receiver; `wake` is called after each one is queued.
    pub fn start(port: u16, wake: Waker) -> Result<(McpServer, Receiver<Call>), String> {
        // Bind synchronously (so errors are reported here) without needing a runtime on
        // the calling thread.
        let listener = std::net::TcpListener::bind(("127.0.0.1", port))
            .map_err(|e| format!("cannot listen on 127.0.0.1:{port}: {e}"))?;
        listener.set_nonblocking(true).map_err(|e| e.to_string())?;
        let addr = listener.local_addr().map_err(|e| e.to_string())?;
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("mcp")
            .enable_all()
            .build()
            .map_err(|e| e.to_string())?;

        let (tx, rx) = mpsc::channel();
        let cancel = CancellationToken::new();
        let service: StreamableHttpService<PcbTools, LocalSessionManager> = StreamableHttpService::new(
            move || Ok(PcbTools::new(tx.clone(), wake.clone())),
            Default::default(),
            StreamableHttpServerConfig::default()
                .with_cancellation_token(cancel.child_token())
                // Loopback `Host` only (the default) guards against DNS rebinding. Agents
                // are not browsers: reject any request that carries a web `Origin`.
                .enforce_origin_validation(),
        );
        let router = axum::Router::new().nest_service("/mcp", service);
        let token = cancel.clone();
        let thread = std::thread::Builder::new()
            .name("mcp-server".into())
            .spawn(move || {
                runtime.block_on(async move {
                    let listener = match tokio::net::TcpListener::from_std(listener) {
                        Ok(l) => l,
                        Err(e) => return log::error!("MCP server: {e}"),
                    };
                    let shutdown = async move { token.cancelled_owned().await };
                    if let Err(e) = axum::serve(listener, router).with_graceful_shutdown(shutdown).await {
                        log::error!("MCP server stopped: {e}");
                    }
                });
                // Don't wait for lingering client streams.
                runtime.shutdown_timeout(Duration::from_millis(200));
            })
            .map_err(|e| e.to_string())?;
        log::info!("MCP server listening on http://{addr}/mcp");
        Ok((McpServer { addr, cancel, thread: Some(thread) }, rx))
    }

    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    pub fn url(&self) -> String {
        format!("http://{}/mcp", self.addr)
    }

    pub fn stop(&mut self) {
        self.cancel.cancel();
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

impl Drop for McpServer {
    fn drop(&mut self) {
        self.stop();
    }
}
