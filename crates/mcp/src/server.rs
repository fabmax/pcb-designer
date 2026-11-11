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

use crate::host::{Call, Reply, Request, ViewSpec};
use crate::library::LibraryRequest;

pub type Waker = Arc<dyn Fn() + Send + Sync>;

const INSTRUCTIONS: &str = "\
PCB Designer: inspect and edit the project open in the app. The user sees every change live.
";

const HEADLESS_INSTRUCTIONS: &str = "\
PCB Designer (headless): inspect and edit the project this server was started with. Nothing is \
shown on screen; screenshot renders views offscreen. save_project writes the project folder \
(pass a path if it was started without one).
";

const COMMON_INSTRUCTIONS: &str = "

Units: coordinates and sizes are integers in nanometers (1 mm = 1000000, 1 mil = 25400); \
angles are degrees, counter-clockwise; Y points up.

Typical workflow:
1. get_project for an overview (components, layers, design rules, outline).
2. list_library_parts, then import_parts to copy parts into the project. Parts nobody has \
   (e.g. from a datasheet): create_library (or open_library), edit_library to add the symbol, \
   footprints and part, render_library_item to check them, save_library, then import_parts.
3. execute commands (place_part, add_wire, add_label, add_track, move, rotate, ...). Several \
   commands in one call form a single undo step and are applied all-or-nothing.
4. screenshot the schematic or board to check the result visually.

Sheets: the schematic is a tree of sheets (get_project lists their paths, e.g. /Power). Commands that add schematic objects take a sheet (id, name or path; default the root sheet). add_sheet adds a sub-sheet, placed on its parent as a sheet symbol; delete removes it with its contents; update renames or resizes it. screenshot, get_selection and set_selection take a sheet too. Reference designators are unique across all sheets.

Schematic and board are linked: placing a part creates its footprint on the board automatically \
(in a staging area right of the board outline), deleting the component removes it. Footprints \
cannot be created or deleted directly, only moved/rotated/flipped. Schematic connectivity is \
by wire endpoints touching pin ends and by labels with equal names, scoped as in KiCad: add_label \
kind local (default; same sheet, net named /NAME, or /Sheet/NAME below the root), global (all \
sheets, plain name; shape input/output/bidirectional/tri_state/passive), hierarchical (to the \
sheet's pin on the parent sheet) or power (global, plain name). Buses (add_bus, \
add_bus_entry) only group signals: name a bus with a label whose name is a bus name \
(D[0..7], {SDA SCL}, I2C{SDA SCL} with members I2C.SDA, I2C.SCL) placed on a bus wire; signals \
leave through bus entries on wires labelled with a member name. Bus wires never connect \
to wires or pins.";

#[derive(Clone, Copy, Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ViewName {
    Schematic,
    Board,
}

impl ViewName {
    fn spec(self, sheet: Option<String>) -> ViewSpec {
        let kind = match self {
            ViewName::Schematic => ViewKind::Schematic,
            ViewName::Board => ViewKind::Board,
        };
        ViewSpec { kind, sheet }
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
pub struct SearchKicadParams {
    /// Words that must all occur in "Library:Name", case-insensitive (e.g. "lm358",
    /// "device r", "soic-8 3.9x4.9").
    pub query: String,
    /// Search footprints instead of symbols.
    #[serde(default)]
    pub footprints: bool,
    /// Maximum number of results (default 50, at most 500).
    #[serde(default)]
    pub limit: Option<usize>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct KicadSymbolPick {
    /// "Library:Name" from search_kicad, e.g. "Amplifier_Operational:LM358".
    pub symbol: String,
    /// Footprints ("Library:Name"), one part variant each. Omit for the symbol's default
    /// (or best matching) footprint; an empty list imports the symbol only.
    #[serde(default)]
    pub footprints: Option<Vec<String>>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct DownloadKicadParams {
    /// Look for a newer release and download it if there is one (default: keep an
    /// existing download as it is).
    #[serde(default)]
    pub update: bool,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ImportKicadParams {
    #[serde(default)]
    pub symbols: Vec<KicadSymbolPick>,
    /// Lone footprints ("Library:Name").
    #[serde(default)]
    pub footprints: Vec<String>,
    /// Name of the open library to import into (default: a library named "KiCad import",
    /// created if needed).
    #[serde(default)]
    pub library: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct UpdatePartsParams {
    /// Project part ids; omit to update all parts whose library changed.
    #[serde(default)]
    pub parts: Option<Vec<Id>>,
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
pub struct DrcParams {
    /// Maximum number of violations to list (default 200; the counts cover all).
    pub limit: Option<usize>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ComponentParams {
    /// Reference designator, e.g. "R1".
    pub refdes: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ExecuteParams {
    /// Commands to run in order, as one undo step. If one fails, none is applied. A
    /// `sheet` or `parent` field takes a sheet id, name or path ("/Power/Regulator").
    #[schemars(with = "Vec<Command>")]
    pub commands: Vec<serde_json::Value>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ViewParams {
    pub view: ViewName,
    /// Schematic sheet: id, name or path ("/Power/Regulator"). Default: the root sheet.
    #[serde(default)]
    pub sheet: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SetSelectionParams {
    pub view: ViewName,
    /// Schematic sheet: id, name or path. Default: the root sheet.
    #[serde(default)]
    pub sheet: Option<String>,
    /// Objects to select (replaces the selection; empty clears it).
    pub ids: Vec<ObjectId>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ScreenshotParams {
    pub target: ScreenshotTarget,
    /// Schematic sheet: id, name or path ("/Power/Regulator"). Default: the root sheet.
    #[serde(default)]
    pub sheet: Option<String>,
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

#[derive(Debug, Deserialize, JsonSchema)]
pub struct CreateLibraryParams {
    /// Library name, e.g. the part or the manufacturer.
    pub name: String,
    /// File to save it to (`….pcblib`, relative to the project folder; the library takes
    /// the file's name). Default: `NAME.pcblib` in the project folder. A project that was
    /// never saved has no folder: then the library stays unsaved (and out of the project)
    /// until save_library gets a path.
    #[serde(default)]
    pub path: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct OpenLibraryParams {
    /// A `.pcblib` file (relative paths are relative to the project folder).
    pub path: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct RemoveLibraryParams {
    /// Name or file of a library (see list_libraries).
    pub library: String,
    /// Drop unsaved changes to the library (otherwise removing it is refused).
    #[serde(default)]
    pub discard: bool,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct LibraryItemsParams {
    /// Name of an open library (see list_libraries).
    pub library: String,
    /// Items as `[part:|symbol:|footprint:]NAME` or ids, e.g. "OpAmp" (part and symbol),
    /// "footprint:SOIC-8".
    pub items: Vec<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct EditLibraryParams {
    /// Name of an open library.
    pub library: String,
    /// Symbols, footprints and parts to add or update (see the tool description).
    #[serde(default)]
    pub upsert: Vec<serde_json::Value>,
    /// Items to delete: `[part:|symbol:|footprint:]NAME` or ids.
    #[serde(default)]
    pub delete: Vec<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SaveLibraryParams {
    pub library: String,
    /// File to save to (`….pcblib`; the library takes the file's name). Defaults to its
    /// current file.
    #[serde(default)]
    pub path: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct RenderLibraryItemParams {
    pub library: String,
    /// A symbol or footprint: `[symbol:|footprint:]NAME` or id (a part shows its symbol).
    pub item: String,
    /// Image size in pixels (default 800x800, 64..4096).
    #[serde(default)]
    pub width: Option<u32>,
    #[serde(default)]
    pub height: Option<u32>,
}

/// The MCP service; one per session, all sharing the same call queue.
#[derive(Clone)]
pub struct PcbTools {
    calls: Sender<Call>,
    wake: Waker,
    headless: bool,
    #[allow(dead_code, reason = "read by the tool_handler macro")]
    tool_router: ToolRouter<Self>,
}

impl PcbTools {
    pub fn new(calls: Sender<Call>, wake: Waker) -> Self {
        PcbTools { calls, wake, headless: false, tool_router: Self::tool_router() }
    }

    /// Tools for a host without a window (the instructions say so).
    pub fn headless(calls: Sender<Call>) -> Self {
        PcbTools { headless: true, ..Self::new(calls, Arc::new(|| {})) }
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

    #[tool(description = "Parts of all open libraries (ids, pin names per unit, footprint variants), \
        plus the parts already in the project and whether their library has a newer version \
        (state LibraryChanged: use update_parts).")]
    async fn list_library_parts(&self) -> Result<CallToolResult, ErrorData> {
        self.call(Request::LibraryParts).await
    }

    #[tool(description = "Copy parts from the library into the project so they can be placed with \
        the place_part command. Already imported parts are skipped.")]
    async fn import_parts(&self, Parameters(p): Parameters<ImportPartsParams>) -> Result<CallToolResult, ErrorData> {
        self.call(Request::ImportParts { parts: p.parts }).await
    }

    #[tool(description = "Replace project parts by their current library version (symbols, \
        footprints, pin-pad mapping). Components keep their variant when it still exists. \
        Without part ids, every part whose library changed is updated.")]
    async fn update_parts(&self, Parameters(p): Parameters<UpdatePartsParams>) -> Result<CallToolResult, ErrorData> {
        self.call(Request::UpdateParts { parts: p.parts.unwrap_or_default() }).await
    }

    #[tool(description = "Search the KiCad symbol (or footprint) libraries by name: the ones \
        downloaded with download_kicad_libraries, else an installed KiCad's. Returns \
        \"Library:Name\" entries for import_kicad.")]
    async fn search_kicad(&self, Parameters(p): Parameters<SearchKicadParams>) -> Result<CallToolResult, ErrorData> {
        let limit = p.limit.unwrap_or(50).clamp(1, 500);
        self.call(Request::KicadSearch { query: p.query, footprints: p.footprints, limit }).await
    }

    #[tool(description = "Import KiCad symbols and footprints into an open library. Each symbol \
        becomes a part with one variant per footprint (pads mapped to pins by number; power \
        symbols like GND are skipped, use net labels). Returns the new part ids (for \
        import_parts), the chosen footprints and conversion notes. Re-importing updates \
        the library objects in place. In headless mode the parts are also added to the \
        project, ready to place.")]
    async fn import_kicad(&self, Parameters(p): Parameters<ImportKicadParams>) -> Result<CallToolResult, ErrorData> {
        let request = crate::host::KicadImport {
            symbols: p.symbols.into_iter().map(|s| (s.symbol, s.footprints)).collect(),
            footprints: p.footprints,
            library: p.library,
        };
        self.call(Request::KicadImport(request)).await
    }

    #[tool(description = "Download KiCad's official symbol and footprint libraries (about 24 MB, \
        450 MB on disk; no 3D models) so search_kicad and import_kicad work without a KiCad \
        installation. Returns at once if they are already downloaded; with update, checks for \
        a newer release first. Returns the version, folder and library counts.")]
    async fn download_kicad_libraries(
        &self,
        Parameters(p): Parameters<DownloadKicadParams>,
    ) -> Result<CallToolResult, ErrorData> {
        self.call(Request::KicadDownload { update: p.update }).await
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

    #[tool(description = "Schematic netlist: every net with at least two pins or a label, with \
        its pins (reference, pin name, footprint pads, position), plus unconnected pins and \
        connectivity problems (e.g. two different labels on one net, misused buses; each with \
        message, objects and position). Pins connect where wire ends, pins, labels and \
        junctions coincide, where a wire ends on another wire, and through same-named labels \
        (local: same sheet; global and power: all sheets); \
        crossing wires connect only with a junction. Also lists buses with their members and \
        the net and pins of each member (null if no such net).")]
    async fn get_netlist(&self) -> Result<CallToolResult, ErrorData> {
        self.call(Request::Netlist).await
    }

    #[tool(description = "Board routing state: unrouted connections (ratsnest lines between the \
        nearest pads, vias or track ends of each net, in nm), shorts between nets, flagged copper \
        (tracks/vias in a short or touching no pad, kept after schematic changes), copper layers \
        and design rules. Route with `execute` add_track (points on pad centers; tracks get their \
        net from the pads they touch) and add_via (through, connects all layers).")]
    async fn get_routing(&self) -> Result<CallToolResult, ErrorData> {
        self.call(Request::Routing).await
    }

    #[tool(description = "Run the design rule check on the board: shorts, clearance between \
        copper of different nets, copper at or outside the board edge, overlapping courtyards, \
        track width and drill below the minimums, copper touching no pad, unrouted connections. \
        Violations are sorted most severe first, with a message, position (nm) and the objects \
        involved (footprints for pads). The GUI marks them too.")]
    async fn run_drc(&self, Parameters(p): Parameters<DrcParams>) -> Result<CallToolResult, ErrorData> {
        self.call(Request::Drc { limit: p.limit.unwrap_or(200) }).await
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
        self.call(Request::GetSelection { view: p.view.spec(p.sheet) }).await
    }

    #[tool(description = "Select objects in a view (highlights them for the user).")]
    async fn set_selection(&self, Parameters(p): Parameters<SetSelectionParams>) -> Result<CallToolResult, ErrorData> {
        self.call(Request::SetSelection { view: p.view.spec(p.sheet), ids: p.ids }).await
    }

    #[tool(description = "PNG image of the schematic, the board, or the whole application window.")]
    async fn screenshot(&self, Parameters(p): Parameters<ScreenshotParams>) -> Result<CallToolResult, ErrorData> {
        let target = match p.target {
            ScreenshotTarget::Schematic => Some(ViewName::Schematic.spec(p.sheet)),
            ScreenshotTarget::Board => Some(ViewName::Board.spec(p.sheet)),
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

    #[tool(description = "Libraries: name, kind (project: the project's own copies of the \
        parts it uses, changed with execute; built_in; external: one of the project's \
        library files; unsaved; missing: a project library file that could not be read), file, \
        unsaved changes, and their symbols (pin counts), footprints (pad counts) and parts \
        (units, variants). Parts can be put into the project with import_parts.")]
    async fn list_libraries(&self) -> Result<CallToolResult, ErrorData> {
        self.call(Request::Library(LibraryRequest::List)).await
    }

    #[tool(description = "Create a new, empty library file and add it to the project's \
        libraries (the user sees it in a tab). Fill it with edit_library.")]
    async fn create_library(
        &self,
        Parameters(p): Parameters<CreateLibraryParams>,
    ) -> Result<CallToolResult, ErrorData> {
        self.call(Request::Library(LibraryRequest::Create { name: p.name, path: p.path.map(Into::into) })).await
    }

    #[tool(description = "Add a library file (.pcblib) to the project's libraries, so its parts \
        can be edited or imported into the project. Refused for a copy of a library that is \
        already there (same library id).")]
    async fn open_library(&self, Parameters(p): Parameters<OpenLibraryParams>) -> Result<CallToolResult, ErrorData> {
        self.call(Request::Library(LibraryRequest::Open { path: p.path.into() })).await
    }

    #[tool(description = "Remove a library from the project's libraries (the file stays). Parts \
        already copied into the project stay too.")]
    async fn remove_library(
        &self,
        Parameters(p): Parameters<RemoveLibraryParams>,
    ) -> Result<CallToolResult, ErrorData> {
        self.call(Request::Library(LibraryRequest::Remove { library: p.library, discard: p.discard })).await
    }

    #[tool(description = "Symbols, footprints and parts of an open library in full JSON: the \
        format edit_library takes. The Built-in library's parts (e.g. \"OpAmp\", \"NPN\", \
        \"footprint:SOIC-8\") are good examples.")]
    async fn get_library_items(
        &self,
        Parameters(p): Parameters<LibraryItemsParams>,
    ) -> Result<CallToolResult, ErrorData> {
        self.call(Request::Library(LibraryRequest::Get { library: p.library, items: p.items })).await
    }

    #[tool(description = "Add or change symbols, footprints and parts of an open library (e.g. \
        a part from a datasheet). One undo step, all-or-nothing: refused if it would leave \
        dangling references. `upsert` takes objects in the format get_library_items returns \
        (\"type\": lib_symbol, lib_footprint or lib_part), with conveniences:\n\
        - Matched by id or name: a match is updated, otherwise added. Fields left out keep \
        their value (new items get defaults). Ids are optional everywhere: pins keep theirs \
        by number (or name), pads by name, variants by name.\n\
        - lib_symbol: name, pins, graphics, texts. Pin: name, number (datasheet pin \
        number), pos (the outer end, where wires connect; put it on the 2.54 mm grid), \
        direction (degrees from pos toward the body: 0 for a pin on the left side, 180 \
        right, 270 top, 90 bottom), length (default 2.54 mm), kind (input, output, \
        bidirectional, tri_state, passive, power_in, power_out, open_collector, \
        not_connected). Graphics are shapes ({\"type\": \"rect\", \"a\": [x, y], \"b\": [x, y], \
        \"width\": 254000, \"fill\": true}, line a/b, circle center/radius, arc, polygon \
        points). Texts: text (${REF}, ${VALUE} show the component's), pos, size, halign, \
        valign; width defaults to size/7.\n\
        - lib_footprint: name, description, pads, graphics, texts. Pad: name (as in the \
        datasheet), pos (centre), size [w, h], shape ({\"type\": \"rect\"}, round_rect with \
        ratio, oval, circle), kind ({\"type\": \"smd\"}, through_hole or non_plated_hole \
        with drill). Origin at the package centre, pad 1 top left, Y up. Graphics \
        {\"layer\": silkscreen, fabrication or courtyard, \"shape\": …}: body outline on \
        fabrication, courtyard around everything, silkscreen clear of the pads.\n\
        - lib_part: name, description, prefix (default U), default_value (default the \
        name), properties, units [{\"symbol\": SYMBOL NAME}] (named A, B, …), variants \
        [{\"footprint\": FOOTPRINT NAME, \"name\"?, \"connections\"?}] (one per package). \
        Without connections, pads connect to pins with the same number (else name); or \
        \"in_order\"; or a list of {\"pad\": PAD NAME, \"pin\": PIN NUMBER or NAME, \"unit\"?}.\n\
        Coordinates and sizes in nm (1 mm = 1000000). Returns what changed and each variant's \
        mapping (unconnected pads, pins without a pad). Check with render_library_item, then \
        save_library.")]
    async fn edit_library(&self, Parameters(p): Parameters<EditLibraryParams>) -> Result<CallToolResult, ErrorData> {
        let request = LibraryRequest::Edit { library: p.library, upsert: p.upsert, delete: p.delete };
        self.call(Request::Library(request)).await
    }

    #[tool(description = "Save a library to its file, or to `path` (the project then refers to \
        the new file).")]
    async fn save_library(&self, Parameters(p): Parameters<SaveLibraryParams>) -> Result<CallToolResult, ErrorData> {
        self.call(Request::Library(LibraryRequest::Save { library: p.library, path: p.path.map(Into::into) })).await
    }

    #[tool(description = "PNG of a library symbol or footprint as the editor draws it (pins with \
        names and numbers; pads with their names, silkscreen, fabrication and courtyard \
        layers). Use it to check a new symbol or footprint.")]
    async fn render_library_item(
        &self,
        Parameters(p): Parameters<RenderLibraryItemParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let request = LibraryRequest::Render {
            library: p.library,
            item: p.item,
            width: p.width.unwrap_or(800),
            height: p.height.unwrap_or(800),
        };
        self.call(Request::Library(request)).await
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
            .with_instructions(format!(
                "{}{COMMON_INSTRUCTIONS}",
                if self.headless { HEADLESS_INSTRUCTIONS } else { INSTRUCTIONS }
            ))
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
