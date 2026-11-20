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
use crate::simulation::{self, SimulateRequest};

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

Sheets: the schematic is a tree of sheets (get_project lists their paths, e.g. /Power). \
Commands that add schematic objects take a sheet (id, name or path; default the root \
sheet). add_sheet adds a sub-sheet, placed on its parent as a sheet symbol (pos = bottom \
left corner); delete removes it with its contents; update renames or resizes it. A \
sub-sheet's hierarchical labels become pins of its symbol (one per name: inputs on the \
left edge, the others on the right, from the top down in name order, 2.54 mm apart; \
get_project lists them with their positions); wire them on the parent sheet like \
component pins. screenshot, \
get_selection and set_selection take a sheet too. Reference designators are unique \
across all sheets. Each sheet has a page (A4 landscape by default; size none for no frame) \
whose lower left corner is the origin: keep objects inside page_rect. update a sheet's page \
to change it; set_title_block edits the title block shown on every sheet. move_to_sheet {ids, sheet} moves \
schematic objects to another sheet keeping their ids, positions and references (symbols take \
their reference/value texts and no-connect flags along; wires not listed stay, so check the \
nets afterwards); moving a sheet symbol makes its sheet (with its sub-sheets) a sub-sheet of \
the target, renamed with a number on a name clash. duplicate {ids, delta} copies objects \
moved by delta: copied components get the next free references, a copied sheet symbol copies \
its whole sub-sheet tree, and on the board board parts are copied (footprints of schematic \
parts are not: duplicate their symbols).

Schematic and board are linked: placing a part creates its footprint on the board automatically \
(in a staging area right of the board outline), deleting the component removes it. Footprints \
cannot be created or deleted directly, only moved/rotated/flipped. The exception are board \
parts: parts without symbol (units: []; mounting holes, logos, fiducials) are placed with \
place_board_part (part, pos, rotation, side) and deleted on the board; their pads have no net, so \
copper of a net touching them is a short. Each footprint has its own reference/value texts \
(kind footprint_field: text ${REF} or ${VALUE} in footprint coordinates, layer, hidden); \
move or rotate them, or update them (size, hidden); they move with their footprint and can't \
be deleted. Schematic symbols have them too (kind symbol_field: text ${REF} or ${VALUE} in \
symbol coordinates, hidden); they are drawn upright whatever the symbol's rotation. \
add_text adds a free text: on a schematic sheet, or with layer on the board (silkscreen, fab, \
copper without a net, …); newlines break lines. Schematic connectivity is \
by wire endpoints touching pin ends and by labels with equal names, scoped as in KiCad: add_label \
kind local (default; same sheet, net named /NAME, or /Sheet/NAME below the root), global (all \
sheets, plain name; shape input/output/bidirectional/tri_state/passive), hierarchical (to the \
sheet's pin on the parent sheet) or power (global, plain name; style bar/arrow/circle/ground/\
signal_ground/chassis/protective_earth picks the glyph, default from the name: GND → ground). Buses (add_bus, \
add_bus_entry) only group signals: name a bus with a label whose name is a bus name \
(D[0..7], {SDA SCL}, I2C{SDA SCL} with members I2C.SDA, I2C.SCL) placed on a bus wire; signals \
leave through bus entries on wires labelled with a member name. Bus wires never connect \
to wires or pins. Pins left open on purpose get a no-connect flag (add_no_connect at the pin \
end; it moves and is deleted with the symbol); other unconnected pins are reported as problems. \
A net powered from outside the schematic (a connector, a supply) gets a power flag \
(add_power_flag on a wire or pin end; KiCad's PWR_FLAG): its power inputs and inputs count as \
driven. \
move, rotate, mirror and flip_side take keep_connected (default false), as a drag in the GUI: \
wires attached to what moves follow (keeping horizontal and vertical wires so), ends of moved \
wires stay on what stays put (joined by a short wire), and on the board tracks ending on moved \
pads and vias move along, sliding along the next track to keep their angles.

Copper zones and keepouts are inserted with execute (command insert, object zone or keepout). \
A zone fills its outline minus its holes on one layer with its net (empty net: isolated \
copper); connection thermal/thermal_through_hole/solid/none says how its own pads join, \
islands always/never/below_area which unconnected fill pieces go. add_cutout {id, outline} cuts a polygon out of a \
zone or keepout: inside, it becomes a hole (merged with holes it overlaps), across the edge it \
trims the outline; cuts that miss, remove all or split the area are refused. Edit corners by \
update with a new outline or holes. measure_clearance gives the gap between two pads, tracks, \
vias, zones, drill holes or the board edge, with the rule that applies. A keepout (layers: copper \
layers, empty for all) forbids tracks, vias, pads, zone fills (no_pour) or footprints \
(courtyards) inside its outline; fills leave it out and DRC reports the rest. Arc tracks have \
a mid point (any point on the arc between a and b): add_track takes mids (one per segment, \
null for straight ones). round_corners {tracks, radius} rounds the corners between straight \
tracks with tangent arcs. add_via takes span [from, to] (two copper layers) for blind and \
buried vias; without it a via goes through all layers.";

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
    /// The last Gerber export (files read back from <project>/output/gerber/).
    Gerber,
    /// The whole application window as the user sees it (GUI only).
    Window,
    /// The plot of an analysis' last run (`analysis`): the whole run, its probes (else
    /// its named nets).
    Simulation,
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
    /// (or best matching) footprint; an empty list makes a part without footprint
    /// (kept off the board).
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
    /// List excluded violations too (marked excluded).
    pub include_excluded: Option<bool>,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct NetlistParams {
    /// List excluded problems too (marked excluded).
    pub include_excluded: Option<bool>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ExcludeParams {
    /// Keys of findings (`key` of get_netlist problems or run_drc violations).
    pub keys: Vec<String>,
    /// Why they are fine (shown next to them).
    pub comment: Option<String>,
    /// Include them again instead.
    pub include: Option<bool>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ClearanceParams {
    pub a: editor_core::clearance::Feature,
    pub b: editor_core::clearance::Feature,
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
pub struct SimulateParams {
    /// The analysis, by name. Default: the one the Simulation panel shows, else the first.
    #[serde(default)]
    pub analysis: Option<String>,
    /// Signals: net names ("OUT" or "V(OUT)") and pin currents ("R1.1" or "I(R1.1)",
    /// reference and pin number or name). Default: the analysis' probes, else its named
    /// nets (up to 8). Currents not probed are measured for this run only.
    #[serde(default)]
    pub signals: Option<Vec<String>>,
    /// Sweeps: x values (seconds, hertz, or the swept source's volts or amperes) to read
    /// each signal at.
    #[serde(default)]
    pub points: Option<Vec<f64>>,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct SpiceNetlistParams {
    /// The analysis, by name. Default: the one the Simulation panel shows, else the first.
    #[serde(default)]
    pub analysis: Option<String>,
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
    /// Gerber: only these files (file names or labels like "Top Copper"). Default: all.
    #[serde(default)]
    pub layers: Option<Vec<String>>,
    /// Simulation: the analysis (name). Default: the one the Simulation panel shows,
    /// else the first.
    #[serde(default)]
    pub analysis: Option<String>,
    /// Image size for views in pixels (default 1200x800, 64..4096).
    #[serde(default)]
    pub width: Option<u32>,
    #[serde(default)]
    pub height: Option<u32>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ImportProjectParams {
    /// The KiCad project file (`.kicad_pro`); its `.kicad_sch` and `.kicad_pcb` sit next to it.
    pub path: String,
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
    /// "footprint:SOT-23".
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
        // A simulation may run for its own limit, plus building its netlist.
        let limit = if matches!(request, Request::Simulate(_)) { simulation::TIMEOUT.as_secs() + 10 } else { 60 };
        let (tx, rx) = oneshot::channel();
        if self.calls.send(Call { request, reply: tx }).is_err() {
            return Ok(error("the application is shutting down"));
        }
        (self.wake)();
        let result = match tokio::time::timeout(Duration::from_secs(limit), rx).await {
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
        symbols like GND are skipped, use power labels; symbols with hidden power pins are refused). Returns the new part ids (for \
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
        its pins (reference, pin name, footprint pads, position), plus unconnected pins (without \
        those under a no-connect flag or of kind not_connected) and \
        connectivity and ERC problems (e.g. two different labels on one net, misused buses, \
        pin kinds that conflict per the pin conflict matrix, undriven inputs, power inputs \
        without a power output, power port or power flag, reused references; each with rule, \
        severity (error/warning), message, objects, position and key; errors first; excluded \
        ones only with include_excluded). Pins connect where wire ends, pins, labels and \
        junctions coincide, where a wire ends on another wire, and through same-named labels \
        (local: same sheet; global and power: all sheets); \
        crossing wires connect only with a junction. Also lists buses with their members and \
        the net and pins of each member (null if no such net).")]
    async fn get_netlist(&self, Parameters(p): Parameters<NetlistParams>) -> Result<CallToolResult, ErrorData> {
        self.call(Request::Netlist { include_excluded: p.include_excluded.unwrap_or(false) }).await
    }

    #[tool(description = "Exclude ERC or DRC findings the design means (e.g. an intended \
        courtyard overlap): by their key from get_netlist problems or run_drc violations, with an \
        optional comment. Excluded findings are kept in the project, listed apart and not \
        counted; a finding whose pins or objects change gets a new key and shows again. \
        include: true includes them again. One undo step.")]
    async fn exclude_findings(&self, Parameters(p): Parameters<ExcludeParams>) -> Result<CallToolResult, ErrorData> {
        self.call(Request::ExcludeFindings {
            keys: p.keys,
            comment: p.comment.unwrap_or_default(),
            include: p.include.unwrap_or(false),
        })
        .await
    }

    #[tool(description = "Change how the checks rate findings: erc {rule: severity} for \
        schematic rules (undriven_input, power_input_unpowered, unconnected_pin, \
        no_connect_misplaced, power_flag_misplaced, net_names, duplicate_reference, \
        sheet_pin_unconnected, hier_label_on_root, bus_misuse), drc {kind: severity} for design \
        rule check kinds (short, clearance, keepout, board_edge, courtyard, track_width, drill, \
        via_span, zone_net, unconnected, unrouted), matrix [{a, b, severity}] for pairs of pin kinds \
        on one net (input, output, bidirectional, tri_state, passive, power_in, power_out, \
        open_collector, not_connected; ignore = fine). Severity: error, warning or ignore (not \
        reported). reset: true starts from the defaults (KiCad's). Returns the resulting \
        severities. One undo step.")]
    async fn set_check_settings(
        &self,
        Parameters(p): Parameters<crate::host::CheckSettings>,
    ) -> Result<CallToolResult, ErrorData> {
        self.call(Request::SetCheckSettings(p)).await
    }

    #[tool(description = "Board routing state: unrouted connections (ratsnest lines between the \
        nearest pads, vias or track ends of each net, in nm), shorts between nets, flagged copper \
        (tracks/vias in a short or touching no pad, kept after schematic changes), copper layers \
        and design rules. Route with `execute` add_track (points on pad centers; tracks get their \
        net from the pads they touch; mids for arc segments) and add_via (through all layers, or \
        span [from, to] for blind and buried vias). Without a width or size they take the net \
        class's (rules.net_classes: per class clearance, track_width, via_diameter, via_drill \
        and nets, exact names or * ? patterns; change them with `execute` set_design_rules).")]
    async fn get_routing(&self) -> Result<CallToolResult, ErrorData> {
        self.call(Request::Routing).await
    }

    #[tool(description = "Run the design rule check on the board: shorts, clearance between \
        copper of different nets, copper at or outside the board edge, overlapping courtyards, \
        track width and drill below the minimums, copper touching no pad, unrouted connections. \
        Violations are sorted errors first (by kind), then warnings, with kind, severity, message, \
        position (nm), the objects involved (footprints for pads) and key (for \
        exclude_findings); excluded ones only with include_excluded. Severities: \
        set_check_settings. The GUI marks them too.")]
    async fn run_drc(&self, Parameters(p): Parameters<DrcParams>) -> Result<CallToolResult, ErrorData> {
        self.call(Request::Drc { limit: p.limit.unwrap_or(200), include_excluded: p.include_excluded.unwrap_or(false) })
            .await
    }

    #[tool(description = "Measure the gap between two things on the board: a pad {type: pad, \
        footprint, pad}, track {type: track, id}, via, zone (its fill), pad_hole {footprint, pad}, \
        via_hole {id} or the board edge {type: edge}. Returns the distance (nm, 0 when touching), \
        the nearest points a and b, the design rule clearance that applies (copper of different \
        nets on a common layer, copper to the edge; null otherwise) and whether it's violated.")]
    async fn measure_clearance(&self, Parameters(p): Parameters<ClearanceParams>) -> Result<CallToolResult, ErrorData> {
        self.call(Request::MeasureClearance { a: p.a, b: p.b }).await
    }

    #[tool(description = "Export the board for manufacturing: Gerber X2 files (copper, solder \
        mask, paste, silkscreen, board outline; copper carries net, pad and component \
        attributes), Excellon drill files (plated, non-plated) and a Gerber job file \
        (.gbrjob) into <project folder>/output/gerber/, replacing the previous export, plus a \
        zip next to that folder; and for assembly a bill of materials and component positions \
        (pick and place, CSV) into <project folder>/output/assembly/. Zones are refilled and the design rules checked first; violations are \
        reported but don't stop the export. Fails without a board outline or when the project \
        was never saved.")]
    async fn export_gerber(&self) -> Result<CallToolResult, ErrorData> {
        self.call(Request::ExportGerber).await
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

    #[tool(description = "PNG image of the schematic, the board, the last Gerber export (read \
        back from the files, to check them; mask layers drawn inverted, drills as holes), the \
        plot of a simulation's last run, or the whole application window.")]
    async fn screenshot(&self, Parameters(p): Parameters<ScreenshotParams>) -> Result<CallToolResult, ErrorData> {
        let (width, height) = (p.width.unwrap_or(1200), p.height.unwrap_or(800));
        match p.target {
            ScreenshotTarget::Gerber => {
                return self.call(Request::GerberScreenshot { layers: p.layers, width, height }).await;
            }
            ScreenshotTarget::Simulation => {
                return self.call(Request::SimulationScreenshot { analysis: p.analysis, width, height }).await;
            }
            _ => {}
        }
        let target = match p.target {
            ScreenshotTarget::Schematic => Some(ViewName::Schematic.spec(p.sheet)),
            ScreenshotTarget::Board => Some(ViewName::Board.spec(p.sheet)),
            ScreenshotTarget::Window | ScreenshotTarget::Gerber | ScreenshotTarget::Simulation => None,
        };
        let request = Request::Screenshot { target, fit: p.fit.unwrap_or(true), width, height };
        self.call(request).await
    }

    #[tool(description = "Run a simulation analysis of the schematic with ngspice and answer \
        when it ends (at most 120 s; the user sees the run in the Simulation panel and plot, and \
        a run of theirs is stopped). An operating point answers each net's voltage, each pin's \
        current (into the pin) and each device's power. A sweep (transient, AC, DC sweep) \
        answers per signal its min, max, average, end value and up to 200 points (x, y; dense \
        runs keep their envelope), plus its values at `points`; AC signals in dB, with their \
        phase in degrees as separate signals. Set analyses up with the set_simulation command \
        (execute); give parts models in the library (edit_library) and sources their waveform \
        through their component's sim field.")]
    async fn simulate(&self, Parameters(p): Parameters<SimulateParams>) -> Result<CallToolResult, ErrorData> {
        let points = p.points.unwrap_or_default();
        self.call(Request::Simulate(SimulateRequest { analysis: p.analysis, signals: p.signals, points })).await
    }

    #[tool(description = "The SPICE netlist an analysis would run now, with the SPICE node of \
        each net, the device name of each component, and problems (errors stop a run; each with \
        the objects involved).")]
    async fn get_spice_netlist(
        &self,
        Parameters(p): Parameters<SpiceNetlistParams>,
    ) -> Result<CallToolResult, ErrorData> {
        self.call(Request::SpiceNetlist { analysis: p.analysis }).await
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
        \"footprint:SOT-23\") are good examples.")]
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
        - All three take an optional category: plain text that groups items in library trees         (e.g. \"Passives\", \"Connectors\"; footprints \"SMD\" or \"Through hole\"). Parts and         symbols share categories; list_libraries shows the ones in use.\n\
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
        with drill [w, h]: equal for a round hole, else an oval slot), optional back (SMD pad on \
        the other side), mask and paste (false: no mask opening / no solder paste), optional \
        mask_margin and paste_margin (nm, overriding the board's mask expansion / paste margin \
        rules for this pad; negative shrinks the opening). Origin at the \
        package centre, pad 1 top left, Y up. Graphics \
        {\"layer\": silkscreen, fabrication or courtyard, \"shape\": …, \"back\"?}: body outline on \
        fabrication, courtyard around everything, silkscreen clear of the pads.\n\
        - lib_part: name, description, prefix (default U), default_value (default the \
        name), properties, units [{\"symbol\": SYMBOL NAME}] (named A, B, …; [] for a board part \
        placed on the board only), variants \
        [{\"footprint\": FOOTPRINT NAME, \"name\"?, \"connections\"?}] (one per package; one \
        without footprint, e.g. {\"name\": \"No footprint\"}, keeps components using it off the board: \
        logos, power flags). \
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

    #[tool(description = "Replace the open project by a KiCad 6+ project (schematic with its \
        sheets, board with footprints, tracks, vias, zones and outline, design rules). The \
        result is unsaved: save_project with a path writes it. In the app the user is asked to \
        save unsaved changes first and may cancel. Returns the report: warnings, notes and \
        what was skipped (e.g. dimensions, title block).")]
    async fn import_kicad_project(
        &self,
        Parameters(p): Parameters<ImportProjectParams>,
    ) -> Result<CallToolResult, ErrorData> {
        self.call(Request::ImportKicadProject { path: p.path.into() }).await
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
