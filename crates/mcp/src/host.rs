//! The application side of the MCP bridge.
//!
//! Tool calls arrive as [`Call`]s on the thread that owns the document (the GUI thread, or
//! the headless host). [`handle`] answers them using a [`Host`], which provides the
//! document plus the bits of UI state agents may look at (selection, cameras, window
//! screenshots).

use std::path::PathBuf;

use editor_core::query::{PartState, pad_positions, part_statuses, pin_pad_names, pin_positions};
use editor_core::{ChangeSet, Command};
use model::*;
use render::gpu::Background;
use render::offscreen::Offscreen;
use render::{Camera, Scene};
use serde_json::{Value, json};
use tokio::sync::oneshot;
use view::interaction::new_group;
use view::{Document, ViewId, ViewKind};

use crate::library::LibraryRequest;
use crate::simulation::{SimJob, SimulateRequest};

/// What a tool asks the application to do.
#[derive(Debug)]
pub enum Request {
    Project,
    LibraryParts,
    ImportParts {
        parts: Vec<Id>,
    },
    /// Update project parts from their libraries (empty: all that changed).
    UpdateParts {
        parts: Vec<Id>,
    },
    ListObjects {
        kind: Option<ObjectKind>,
        offset: usize,
        limit: usize,
    },
    GetObjects {
        ids: Vec<ObjectId>,
    },
    Component {
        refdes: String,
    },
    /// Nets of the schematic.
    Netlist {
        include_excluded: bool,
    },
    /// Board routing state: ratsnest, shorts, layers, rules.
    Routing,
    /// Design rule check (also marks the violations in the GUI).
    Drc {
        limit: usize,
        include_excluded: bool,
    },
    /// Exclude ERC or DRC findings by key (or include them again).
    ExcludeFindings {
        keys: Vec<String>,
        comment: String,
        include: bool,
    },
    /// Change ERC and DRC severities and pin conflict matrix cells.
    SetCheckSettings(CheckSettings),
    /// A track laid by the router (pushing or going round what's in the way).
    Route {
        layer: LayerId,
        points: Vec<Point>,
        width: Option<Coord>,
        mode: editor_core::router::RouteMode,
    },
    /// The gap between two board features.
    MeasureClearance {
        a: editor_core::clearance::Feature,
        b: editor_core::clearance::Feature,
    },
    /// Gerber and drill files into the project folder.
    /// The schematic as a PDF into the project's output folder.
    ExportSchematicPdf,
    ExportGerber {
        /// Lay the BOM and position files out for this service (and keep it as the
        /// board's preset).
        assembly: Option<model::AssemblyPreset>,
    },
    /// Commands as JSON: `sheet` and `parent` may name a sheet (see [`resolve_sheets`]).
    Execute {
        commands: Vec<Value>,
    },
    Undo,
    Redo,
    GetSelection {
        view: ViewSpec,
    },
    SetSelection {
        view: ViewSpec,
        ids: Vec<ObjectId>,
    },
    /// Render a view (`target = Some(view)`) or capture the whole window (`None`).
    /// The last Gerber export, read back from disk; `layers`: file names or labels.
    GerberScreenshot {
        layers: Option<Vec<String>>,
        width: u32,
        height: u32,
    },
    Screenshot {
        target: Option<ViewSpec>,
        fit: bool,
        width: u32,
        height: u32,
    },
    Save {
        path: Option<PathBuf>,
    },
    /// Search KiCad's symbol (or footprint) libraries by name.
    KicadSearch {
        query: String,
        footprints: bool,
        limit: usize,
    },
    /// Import KiCad symbols (as parts) and footprints into an open library.
    KicadImport(KicadImport),
    /// Download KiCad's libraries (or, with `update`, a newer release).
    KicadDownload {
        update: bool,
    },
    /// Create, open, read, edit, save or render libraries.
    Library(LibraryRequest),
    /// Replace the open project by an imported KiCad project (`.kicad_pro`).
    ImportKicadProject {
        path: PathBuf,
    },
    /// Write the open project as a KiCad 9 project into folder `dir`, its files named
    /// `name` (default: the project's name).
    ExportKicadProject {
        dir: PathBuf,
        name: Option<String>,
    },
    /// Run an analysis and answer with its values (see [`crate::simulation`]).
    Simulate(SimulateRequest),
    /// The SPICE netlist an analysis would run now.
    SpiceNetlist {
        analysis: Option<String>,
    },
    /// The plot of an analysis' last run.
    SimulationScreenshot {
        analysis: Option<String>,
        /// A transient's spectrum instead of its waveform.
        spectrum: bool,
        width: u32,
        height: u32,
    },
}

/// A view as an agent names it: the board, or a schematic sheet by id, name or path
/// (`None`: the root sheet).
#[derive(Clone, Debug)]
pub struct ViewSpec {
    pub kind: ViewKind,
    pub sheet: Option<String>,
}

impl ViewSpec {
    pub fn resolve(&self, project: &Project) -> Result<ViewId, String> {
        match (self.kind, &self.sheet) {
            (ViewKind::Board, None) => Ok(ViewId::Board),
            (ViewKind::Board, Some(_)) => Err("the board has no sheets".into()),
            (ViewKind::Schematic, None) => Ok(ViewId::root(project)),
            (ViewKind::Schematic, Some(s)) => find_sheet(project, s).map(ViewId::Sheet),
        }
    }
}

fn find_sheet(project: &Project, spec: &str) -> Result<Id, String> {
    project.schematic.find_sheet(spec).ok_or_else(|| {
        let paths: Vec<String> = project.schematic.sheets.ids().map(|s| project.schematic.display_path(s)).collect();
        format!("no sheet {spec:?} (sheets: {})", paths.join(", "))
    })
}

/// Parse a command, with sheets in `sheet` and `parent` fields given by id, name or path.
pub fn resolve_sheets(project: &Project, mut command: Value) -> Result<Command, String> {
    if let Some(obj) = command.as_object_mut() {
        for key in ["sheet", "parent"] {
            if let Some(Value::String(spec)) = obj.get(key)
                && spec.parse::<Id>().is_err()
            {
                let id = find_sheet(project, spec)?;
                obj.insert(key.into(), json!(id));
            }
        }
    }
    serde_json::from_value(command).map_err(|e| e.to_string())
}

/// A KiCad import requested by an agent.
#[derive(Clone, Debug, Default)]
pub struct KicadImport {
    /// `Lib:Name` of each symbol, with the footprints to make a part with (`None`: the
    /// symbol's default or best matching footprint; empty: a part with a "no footprint"
    /// variant, kept off the board).
    pub symbols: Vec<(String, Option<Vec<String>>)>,
    /// Lone footprints (`Lib:Name`).
    pub footprints: Vec<String>,
    /// Name of the open library to import into; a new library if none is given or found.
    pub library: Option<String>,
}

impl Request {
    /// Short description for activity displays.
    pub fn describe(&self) -> String {
        match self {
            Request::Project => "read project".into(),
            Request::LibraryParts => "list library parts".into(),
            Request::ImportParts { parts } => format!("import {} part(s)", parts.len()),
            Request::UpdateParts { parts } if parts.is_empty() => "update parts".into(),
            Request::UpdateParts { parts } => format!("update {} part(s)", parts.len()),
            Request::ListObjects { .. } => "list objects".into(),
            Request::GetObjects { ids } => format!("read {} object(s)", ids.len()),
            Request::Component { refdes } => format!("read {refdes}"),
            Request::Netlist { .. } => "read netlist".into(),
            Request::ExcludeFindings { keys, include: false, .. } => format!("exclude {} finding(s)", keys.len()),
            Request::ExcludeFindings { keys, include: true, .. } => format!("include {} finding(s)", keys.len()),
            Request::SetCheckSettings(_) => "change check settings".into(),
            Request::Routing => "read routing state".into(),
            Request::Drc { .. } => "run the design rule check".into(),
            Request::MeasureClearance { .. } => "measure a clearance".into(),
            Request::Route { .. } => "route a track".into(),
            Request::ExportGerber { .. } => "export Gerber files".into(),
            Request::ExportSchematicPdf => "export the schematic as PDF".into(),
            Request::Execute { commands } => match &commands[..] {
                [one] => one.get("command").and_then(Value::as_str).unwrap_or("command").replace('_', " "),
                many => format!("{} commands", many.len()),
            },
            Request::Undo => "undo".into(),
            Request::Redo => "redo".into(),
            Request::GetSelection { .. } => "read selection".into(),
            Request::SetSelection { ids, .. } => format!("select {} object(s)", ids.len()),
            Request::Screenshot { target: Some(v), .. } => match &v.sheet {
                Some(s) => format!("screenshot sheet {s}"),
                None => format!("screenshot {:?}", v.kind),
            },
            Request::Screenshot { target: None, .. } => "screenshot window".into(),
            Request::GerberScreenshot { .. } => "screenshot the Gerber export".into(),
            Request::Save { .. } => "save".into(),
            Request::KicadSearch { query, .. } => format!("search KiCad for {query:?}"),
            Request::KicadImport(k) => format!("import {} item(s) from KiCad", k.symbols.len() + k.footprints.len()),
            Request::KicadDownload { update: false } => "download KiCad libraries".into(),
            Request::KicadDownload { update: true } => "update KiCad libraries".into(),
            Request::Library(r) => r.describe(),
            Request::ImportKicadProject { path } => format!("import KiCad project {}", path.display()),
            Request::ExportKicadProject { dir, .. } => format!("export a KiCad project to {}", dir.display()),
            Request::Simulate(r) => match &r.analysis {
                Some(a) => format!("simulate {a}"),
                None => "simulate".into(),
            },
            Request::SpiceNetlist { .. } => "read the SPICE netlist".into(),
            Request::SimulationScreenshot { .. } => "screenshot the simulation plot".into(),
        }
    }

    /// Does this request change the document?
    pub fn modifies(&self) -> bool {
        matches!(
            self,
            Request::ImportParts { .. }
                | Request::UpdateParts { .. }
                | Request::Execute { .. }
                | Request::ExcludeFindings { .. }
                | Request::SetCheckSettings(_)
                | Request::Undo
                | Request::Redo
                | Request::Save { .. }
                | Request::KicadImport(_)
                | Request::ImportKicadProject { .. }
                | Request::ExportGerber { assembly: Some(_) }
        ) || matches!(self, Request::Library(r) if r.modifies())
    }
}

#[derive(Debug)]
pub enum Reply {
    Json(Value),
    Png(Vec<u8>),
}

pub type Responder = oneshot::Sender<Result<Reply, String>>;

/// A request plus the channel its answer goes to.
#[derive(Debug)]
pub struct Call {
    pub request: Request,
    pub reply: Responder,
}

/// What [`handle`] needs from the application.
pub trait Host {
    fn document(&self) -> &Document;
    fn document_mut(&mut self) -> &mut Document;
    /// Open libraries that `import_parts` / `list_library_parts` / `update_parts` use.
    fn libraries(&self) -> Vec<&Library>;
    fn selection(&self, view: ViewId) -> Vec<ObjectId>;
    fn set_selection(&mut self, view: ViewId, ids: Vec<ObjectId>);
    /// PNG of a view: everything (`fit`) or what the user currently sees.
    fn render_view(&mut self, view: ViewId, fit: bool, width: u32, height: u32) -> Result<Vec<u8>, String>;
    /// Render any scene offscreen, on `editor`'s background, seen by `camera` (`None`:
    /// fitted).
    fn render_scene(
        &mut self,
        scene: &Scene,
        editor: view::style::Editor,
        camera: Option<Camera>,
        width: u32,
        height: u32,
    ) -> Result<Vec<u8>, String>;
    /// The analysis the user looks at (the Simulation panel's).
    fn shown_analysis(&self) -> Option<Id> {
        None
    }
    /// The last finished run of an analysis.
    fn sim_result(&self, _analysis: Id) -> Option<&sim::Finished> {
        None
    }
    /// Run an agent's simulation and answer when it ends ([`crate::simulation::start`],
    /// then [`crate::simulation::check`] after each poll of the session).
    fn simulate(&mut self, _job: SimJob, reply: Responder) {
        let _ = reply.send(Err("this host can't simulate".into()));
    }
    /// Capture the application window. Hosts without a window refuse.
    fn window_screenshot(&mut self, reply: Responder) {
        let _ = reply.send(Err("this host has no window; screenshot a view instead".into()));
    }
    /// `Lib:Name`s of KiCad symbols or footprints matching `query`.
    fn kicad_search(&mut self, _query: &str, _footprints: bool, _limit: usize) -> Result<Value, String> {
        Err("KiCad libraries are not available in this host".into())
    }
    /// Import from KiCad; the host may answer later (it may need to open a library).
    fn kicad_import(&mut self, _request: KicadImport, reply: Responder) {
        let _ = reply.send(Err("KiCad libraries are not available in this host".into()));
    }
    /// Download KiCad's libraries; the host may answer later (it may download in the
    /// background).
    fn kicad_download(&mut self, _update: bool, reply: Responder) {
        let _ = reply.send(Err("KiCad libraries can't be downloaded in this host".into()));
    }
    /// Bring the loaded libraries in line with the project's library files (they change
    /// with undo and redo). Called before every request.
    fn sync_libraries(&mut self) {}
    /// Library tools (see [`crate::library`]); the host may answer later (opening a
    /// library may need the UI). Answer with [`crate::library::respond`].
    fn library_request(&mut self, _request: LibraryRequest, reply: Responder) {
        let _ = reply.send(Err("libraries can't be edited in this host".into()));
    }
    /// Replace the open project by an imported one. The app asks about unsaved changes
    /// first (and may answer later); this default replaces it right away.
    fn replace_project(&mut self, import: kicad::ProjectImport, reply: Responder) {
        *self.document_mut() = Document::new(import.project);
        let _ = reply.send(Ok(Reply::Json(project_import_reply(&import.report))));
    }
}

/// Reply to a project import: the report.
pub fn project_import_reply(report: &kicad::Report) -> Value {
    json!({
        "imported": true,
        "saved": false,
        "warnings": report.warnings,
        "notes": report.notes,
        "skipped": report.skipped,
    })
}

/// Resolve an agent's import: symbols without footprints get their suggested ones.
pub fn kicad_request(
    job: &KicadImport,
    mut suggested: impl FnMut(&str) -> Result<Vec<String>, String>,
) -> Result<kicad::Request, String> {
    let mut req = kicad::Request { symbols: Vec::new(), footprints: job.footprints.clone() };
    for (symbol, fps) in &job.symbols {
        let fps = match fps {
            Some(f) => f.clone(),
            None => suggested(symbol)?,
        };
        req.symbols.push((symbol.clone(), fps));
    }
    Ok(req)
}

/// Reply to an import: what was imported into `library`, with notes.
pub fn kicad_import_reply(
    library: &str,
    added: usize,
    replaced: usize,
    imported: &kicad::Imported,
    req: &kicad::Request,
) -> Value {
    let parts: Vec<Value> = imported
        .parts
        .iter()
        .map(|p| {
            let variants: Vec<Value> = p
                .variants
                .iter()
                .map(|v| json!({ "id": v.id, "footprint": v.name, "connected_pads": v.connections.len() }))
                .collect();
            json!({ "id": p.id, "name": p.name, "units": p.units.len(), "variants": variants })
        })
        .collect();
    let footprints: Vec<&str> = imported.footprints.iter().map(|f| f.name.as_str()).collect();
    let without_footprint: Vec<&str> = req.symbols.iter().filter(|s| s.1.is_empty()).map(|s| s.0.as_str()).collect();
    json!({
        "library": library,
        "added": added,
        "updated": replaced,
        "parts": parts,
        "footprints": footprints,
        "parts_without_footprint": without_footprint,
        "notes": imported.warnings,
        "errors": imported.errors,
    })
}

/// Reply to a download: the version, where it is and what it holds.
pub fn kicad_download_reply(installed: &kicad::download::Installed, downloaded: bool) -> Value {
    let catalog = kicad::Catalog::scan(&installed.paths());
    json!({
        "version": installed.version,
        "path": installed.dir,
        "downloaded": downloaded,
        "symbol_libraries": catalog.symbol_libs.len(),
        "footprint_libraries": catalog.footprint_libs.len(),
        "symbols": catalog.symbol_count(),
        "footprints": catalog.footprint_count(),
    })
}

/// Answer one call. Window screenshots may be answered later by the host.
/// What became of a call (for activity logs): answered now, with a short text of the
/// result or the error, or answered later by the host.
#[derive(Clone, Debug, PartialEq)]
pub enum Outcome {
    Answered(Result<String, String>),
    Deferred,
}

/// Longest result text an [`Outcome`] keeps.
const OUTCOME_TEXT: usize = 4000;

fn outcome_text(reply: &Reply) -> String {
    match reply {
        Reply::Json(v) => {
            let mut text = serde_json::to_string_pretty(v).unwrap_or_default();
            if text.len() > OUTCOME_TEXT {
                let mut end = OUTCOME_TEXT;
                while !text.is_char_boundary(end) {
                    end -= 1;
                }
                text.truncate(end);
                text.push_str("\n…");
            }
            text
        }
        Reply::Png(png) => format!("PNG image, {} KB", png.len().div_ceil(1024)),
    }
}

pub fn handle(call: Call, host: &mut dyn Host) -> Outcome {
    let Call { request, reply } = call;
    host.sync_libraries();
    let request = match request {
        Request::Screenshot { target: None, .. } => {
            host.window_screenshot(reply);
            return Outcome::Deferred;
        }
        Request::KicadImport(k) => {
            host.kicad_import(k, reply);
            return Outcome::Deferred;
        }
        Request::KicadDownload { update } => {
            host.kicad_download(update, reply);
            return Outcome::Deferred;
        }
        Request::Library(r) => {
            host.library_request(r, reply);
            return Outcome::Deferred;
        }
        Request::Simulate(req) => {
            let default = host.shown_analysis();
            return match crate::simulation::prepare(host.document_mut(), &req, default) {
                Ok((job, _)) => {
                    host.simulate(job, reply);
                    Outcome::Deferred
                }
                Err(e) => {
                    let _ = reply.send(Err(e.clone()));
                    Outcome::Answered(Err(e))
                }
            };
        }
        Request::ImportKicadProject { path } => {
            return match kicad::import_project(&path) {
                Ok(import) => {
                    host.replace_project(import, reply);
                    Outcome::Deferred
                }
                Err(e) => {
                    let _ = reply.send(Err(e.clone()));
                    Outcome::Answered(Err(e))
                }
            };
        }
        r => r,
    };
    let result = respond(request, host);
    let outcome = Outcome::Answered(result.as_ref().map(outcome_text).map_err(Clone::clone));
    let _ = reply.send(result);
    outcome
}

fn respond(request: Request, host: &mut dyn Host) -> Result<Reply, String> {
    let json = match request {
        Request::Project => project_summary(host.document()),
        Request::LibraryParts => {
            let refs = host.libraries();
            let libraries: Vec<Value> = refs.iter().copied().map(library_parts).collect();
            let statuses = part_statuses(host.document().project(), &refs);
            let project_parts: Vec<Value> = statuses
                .iter()
                .map(|s| json!({ "id": s.part, "name": s.name, "library": s.library_name, "state": format!("{:?}", s.state) }))
                .collect();
            json!({ "libraries": libraries, "project_parts": project_parts })
        }
        Request::ImportParts { parts } => {
            let mut commands: Vec<Command> = Vec::new();
            let libraries = host.libraries();
            let mut by_lib: Vec<(usize, Vec<Id>)> = Vec::new();
            for p in parts {
                let i = libraries
                    .iter()
                    .position(|l| l.parts.contains(p))
                    .ok_or(format!("part {p} is in no open library"))?;
                match by_lib.iter_mut().find(|(l, _)| *l == i) {
                    Some((_, v)) => v.push(p),
                    None => by_lib.push((i, vec![p])),
                }
            }
            for (i, parts) in by_lib {
                commands.push(Command::import_parts(libraries[i], &parts).map_err(|e| e.to_string())?);
            }
            execute(host.document_mut(), commands)?
        }
        Request::UpdateParts { parts } => {
            let libraries = host.libraries();
            let statuses = part_statuses(host.document().project(), &libraries);
            let mut by_lib: Vec<(usize, Vec<Id>)> = Vec::new();
            for s in statuses {
                let wanted =
                    if parts.is_empty() { s.state == PartState::LibraryChanged } else { parts.contains(&s.part) };
                if !wanted {
                    continue;
                }
                let i = s.library.ok_or(format!("the library of part {:?} is not open", s.name))?;
                match by_lib.iter_mut().find(|(l, _)| *l == i) {
                    Some((_, v)) => v.push(s.part),
                    None => by_lib.push((i, vec![s.part])),
                }
            }
            if by_lib.is_empty() {
                return Ok(Reply::Json(json!({ "updated": 0 })));
            }
            let commands = by_lib
                .into_iter()
                .map(|(i, parts)| Command::update_parts(libraries[i], &parts).map_err(|e| e.to_string()))
                .collect::<Result<_, _>>()?;
            execute(host.document_mut(), commands)?
        }
        Request::ListObjects { kind, offset, limit } => list_objects(host.document().project(), kind, offset, limit),
        Request::GetObjects { ids } => {
            let p = host.document().project();
            json!(ids.iter().map(|&id| p.get(id)).collect::<Vec<_>>())
        }
        Request::Component { refdes } => component(host.document().project(), &refdes)?,
        Request::Netlist { include_excluded } => {
            let doc = host.document_mut();
            doc.update_connectivity(false);
            netlist(doc.project(), doc.connectivity(), include_excluded)
        }
        Request::Routing => {
            let doc = host.document_mut();
            doc.update_connectivity(false);
            routing(doc.project(), doc.copper())
        }
        Request::Drc { limit, include_excluded } => {
            let report = host.document_mut().run_drc();
            let excluded = report.violations.iter().filter(|v| v.excluded).count();
            let v: Vec<&editor_core::drc::Violation> =
                report.violations.iter().filter(|v| include_excluded || !v.excluded).collect();
            let count = |s: Severity| v.iter().filter(|x| !x.excluded && x.severity == s).count();
            let mut by_kind: std::collections::BTreeMap<&str, usize> = Default::default();
            for x in &v {
                *by_kind.entry(x.kind.label()).or_default() += 1;
            }
            json!({
                "clean": v.iter().all(|x| x.excluded),
                "count": v.len(),
                "errors": count(Severity::Error),
                "warnings": count(Severity::Warning),
                "excluded": excluded,
                "by_kind": by_kind,
                "violations": &v[..v.len().min(limit)],
                "truncated": v.len() > limit,
            })
        }
        Request::Route { layer, points, width, mode } => route(host.document_mut(), layer, points, width, mode)?,
        Request::MeasureClearance { a, b } => {
            let doc = host.document_mut();
            doc.update_connectivity(false);
            let (project, copper) = (doc.project(), doc.copper());
            let gap =
                editor_core::clearance::gap(project, copper, a, b).ok_or("no such object (or it has no copper)")?;
            json!({
                "distance": gap.distance.round() as i64,
                "a": [gap.a.x, gap.a.y],
                "b": [gap.b.x, gap.b.y],
                "rule": gap.rule,
                "violates": gap.violates(),
                "a_name": editor_core::clearance::name(project, copper, a),
                "b_name": editor_core::clearance::name(project, copper, b),
            })
        }
        Request::ExportSchematicPdf => {
            let file = host.document_mut().export_schematic_pdf()?;
            let project = host.document().project();
            let pages: Vec<String> = view::print::sheets_in_order(project)
                .into_iter()
                .map(|id| match project.schematic.sheet_path(id) {
                    p if p.is_empty() => "/".to_string(),
                    p => p,
                })
                .collect();
            json!({ "file": file, "pages": pages })
        }
        Request::ExportGerber { assembly } => {
            if let Some(preset) = assembly
                && preset != host.document().project().board.assembly
            {
                host.document_mut().execute(Command::SetAssemblyPreset { preset }).map_err(|e| e.to_string())?;
            }
            let export = host.document_mut().export_gerber().map_err(|e| e.to_string())?;
            let mut by_kind: std::collections::BTreeMap<&str, usize> = Default::default();
            for x in &export.warnings {
                *by_kind.entry(x.kind.label()).or_default() += 1;
            }
            json!({
                "dir": export.dir,
                "zip": export.dir.parent().map(|d| d.join(&export.manifest.zip)),
                "files": export.manifest.files.iter().map(|f| json!({"name": f.name, "label": f.label})).collect::<Vec<_>>(),
                "assembly_dir": export.dir.parent().map(|d| d.join("assembly")),
                "assembly_preset": host.document().project().board.assembly.label(),
                "assembly": export.manifest.assembly.iter().map(|f| json!({"name": f.name, "label": f.label})).collect::<Vec<_>>(),
                "drc_violations": export.warnings.len(),
                "drc_by_kind": by_kind,
            })
        }
        Request::KicadSearch { query, footprints, limit } => host.kicad_search(&query, footprints, limit)?,
        Request::ExportKicadProject { dir, name } => {
            let project = host.document().project();
            let name = name.unwrap_or_else(|| project.name.clone());
            let export = kicad::export::export_project(project, &dir, &name)?;
            json!({
                "dir": dir,
                "project": dir.join(format!("{}.kicad_pro", kicad::export::item_name(&name))),
                "files": export.files.iter().map(|(f, _)| f.to_string_lossy().replace('\\', "/")).collect::<Vec<_>>(),
                "warnings": export.report.warnings,
                "notes": export.report.notes,
                "skipped": export.report.skipped,
            })
        }
        Request::KicadImport(_)
        | Request::KicadDownload { .. }
        | Request::Library(_)
        | Request::Simulate(_)
        | Request::ImportKicadProject { .. } => {
            unreachable!("answered by the host")
        }
        Request::SpiceNetlist { analysis } => {
            let default = host.shown_analysis();
            let doc = host.document_mut();
            doc.update_connectivity(false);
            let a = crate::simulation::find_analysis(doc.project(), analysis.as_deref(), default)?;
            crate::simulation::netlist_reply(doc.project(), doc.connectivity(), a)
        }
        Request::SimulationScreenshot { analysis, spectrum, width, height } => {
            let default = host.shown_analysis();
            host.document_mut().update_connectivity(false);
            let doc = host.document();
            let a = crate::simulation::find_analysis(doc.project(), analysis.as_deref(), default)?;
            if spectrum && !matches!(a.kind, model::AnalysisKind::Tran { .. }) {
                return Err(format!("{} isn't a transient: only transients have a spectrum", a.name));
            }
            let done =
                host.sim_result(a.id).ok_or_else(|| format!("{} has no results yet: run it with simulate", a.name))?;
            let conn = doc.connectivity();
            let signals = sim::plot::signals(a, &done.result, conn).0;
            let (w, h) = (width.clamp(64, 4096), height.clamp(64, 4096));
            let editor = view::style::Editor::Simulation;
            let (scene, camera) = view::style::with_editor(editor, || {
                crate::simulation::plot_scene(doc.project(), done, &signals, conn, spectrum, w, h)
            })
            .ok_or_else(|| format!("{} is an operating point: it has no plot; simulate answers its values", a.name))?;
            return host.render_scene(&scene, editor, Some(camera), w, h).map(Reply::Png);
        }
        Request::Execute { commands } => execute_json(host.document_mut(), commands)?,
        Request::ExcludeFindings { keys, comment, include } => {
            exclude_findings(host.document_mut(), &keys, &comment, include)?
        }
        Request::SetCheckSettings(c) => set_check_settings(host.document_mut(), c)?,
        Request::Undo => {
            let doc = host.document_mut();
            let label = doc.editor().history().undo_label().map(str::to_string).ok_or("nothing to undo")?;
            let changes = doc.undo().unwrap_or_default();
            json!({ "undone": label, "changes": summarize(&changes) })
        }
        Request::Redo => {
            let doc = host.document_mut();
            let label = doc.editor().history().redo_label().map(str::to_string).ok_or("nothing to redo")?;
            let changes = doc.redo().unwrap_or_default();
            json!({ "redone": label, "changes": summarize(&changes) })
        }
        Request::GetSelection { view } => {
            let view = view.resolve(host.document().project())?;
            let ids = host.selection(view);
            let p = host.document().project();
            json!(ids.iter().filter_map(|&id| p.get(id)).collect::<Vec<_>>())
        }
        Request::SetSelection { view, ids } => {
            let p = host.document().project();
            let view = view.resolve(p)?;
            if let Some(missing) = ids.iter().find(|&&id| !p.contains(id)) {
                return Err(format!("no such object: {}", serde_json::to_string(missing).unwrap_or_default()));
            }
            if let Some(wrong) = ids.iter().find(|id| !view.kind().shows(id.kind())) {
                return Err(format!("{:?} objects are not shown in the {:?} view", wrong.kind(), view.kind()));
            }
            if let Some(sheet) = view.sheet()
                && let Some(&other) = ids.iter().find(|&&id| p.schematic.sheet_of(id) != Some(sheet))
            {
                let on = p.schematic.sheet_of(other).map(|s| p.schematic.display_path(s)).unwrap_or_default();
                let other = serde_json::to_string(&other).unwrap_or_default();
                return Err(format!("{other} is on sheet {on}, not {}", p.schematic.display_path(sheet)));
            }
            host.set_selection(view, ids);
            json!({ "selected": host.selection(view).len() })
        }
        Request::Screenshot { target: Some(view), fit, width, height } => {
            let view = view.resolve(host.document().project())?;
            let (w, h) = (width.clamp(64, 4096), height.clamp(64, 4096));
            return host.render_view(view, fit, w, h).map(Reply::Png);
        }
        Request::Screenshot { target: None, .. } => unreachable!("handled by the host"),
        Request::GerberScreenshot { layers, width, height } => {
            let dir = host.document().path.clone().ok_or("the project has no folder, so no export")?;
            let dir = view::gerber::gerber_dir(&dir);
            if !dir.is_dir() {
                return Err("no Gerber export yet: run export_gerber first".into());
            }
            let mut gv = view::GerberView::load(&dir).map_err(|e| e.to_string())?;
            if let Some(names) = layers {
                for f in &gv.files {
                    let shown =
                        names.iter().any(|n| n.eq_ignore_ascii_case(&f.name) || n.eq_ignore_ascii_case(&f.label));
                    gv.scene.layer_mut(f.render_layer).visible = shown;
                }
                if !gv.files.iter().any(|f| gv.scene.layer(f.render_layer).visible) {
                    let all: Vec<String> = gv.files.iter().map(|f| format!("{} ({})", f.name, f.label)).collect();
                    return Err(format!("no such file; the export has: {}", all.join(", ")));
                }
            }
            let (w, h) = (width.clamp(64, 4096), height.clamp(64, 4096));
            return host.render_scene(&gv.scene, view::style::Editor::Pcb, None, w, h).map(Reply::Png);
        }
        Request::Save { path } => {
            let doc = host.document_mut();
            match path {
                Some(p) => doc.save_as(&p),
                None if doc.path.is_some() => doc.save(),
                None => return Err("the project has never been saved: pass a folder path".into()),
            }
            .map_err(|e| e.to_string())?;
            json!({ "saved": doc.path })
        }
    };
    Ok(Reply::Json(json))
}

/// A track along `points` laid by the router in `mode`, with what it pushed, as one undo
/// step.
fn route(
    doc: &mut Document,
    layer: LayerId,
    points: Vec<Point>,
    width: Option<Coord>,
    mode: editor_core::router::RouteMode,
) -> Result<Value, String> {
    let &start = points.first().ok_or("no points given")?;
    if points.len() < 2 {
        return Err("a track needs two points".into());
    }
    let copper = doc.current_copper();
    let net = copper.at(start, layer).and_then(|c| copper.net_of(c.item)).map(str::to_string);
    let width = width.unwrap_or_else(|| doc.project().board.rules.for_net(net.as_deref()).track_width);
    let (project, copper) = (doc.project(), doc.copper());
    let routed = editor_core::router::route(project, copper, mode, layer, width, net.as_deref(), &points, None);
    if routed.points.len() < 2 {
        let why = routed.blocked.map_or("no room".to_string(), |c| c.message);
        return Err(format!("no track added: {why}"));
    }
    let mut commands =
        vec![Command::AddTrack { layer, points: routed.points.clone(), width: Some(width), mids: Vec::new() }];
    commands.extend(routed.shoved.commands(project));
    let pushed: Vec<Value> =
        routed.shoved.lines.iter().map(|p| json!({ "tracks": p.line.tracks, "path": p.path })).collect();
    let vias: Vec<Value> = routed.shoved.vias.iter().map(|m| json!({ "id": m.id, "to": m.to })).collect();
    let mut out = execute(doc, commands)?;
    out["path"] = json!(routed.points);
    out["reached"] = json!(routed.reached);
    out["blocked"] = json!(routed.blocked.map(|c| json!({ "at": c.at, "message": c.message })));
    out["pushed"] = json!(pushed);
    out["moved_vias"] = json!(vias);
    Ok(out)
}

/// Run commands as one undo step. If any fails, the earlier ones are undone.
fn execute(doc: &mut Document, commands: Vec<Command>) -> Result<Value, String> {
    run_commands(doc, commands.into_iter().map(Ok).collect())
}

/// [`execute`] for an agent's commands: each is parsed just before it runs, so it can name
/// a sheet an earlier one added.
fn execute_json(doc: &mut Document, commands: Vec<Value>) -> Result<Value, String> {
    run_commands(doc, commands.into_iter().map(Err).collect())
}

/// Commands, parsed (`Ok`) or still JSON (`Err`).
fn run_commands(doc: &mut Document, commands: Vec<Result<Command, Value>>) -> Result<Value, String> {
    if commands.is_empty() {
        return Err("no commands given".into());
    }
    let group = new_group();
    let mut all = ChangeSet::default();
    let mut labels = Vec::new();
    let n = commands.len();
    for (i, cmd) in commands.into_iter().enumerate() {
        let result = cmd.or_else(|json| resolve_sheets(doc.project(), json)).and_then(|cmd| {
            labels.push(cmd.label());
            doc.execute_grouped(cmd, Some(group)).map_err(|e| e.to_string())
        });
        match result {
            Ok(changes) => all = all.then(changes),
            Err(e) => {
                doc.close_group();
                if !all.is_empty() {
                    doc.undo();
                }
                let which = if n > 1 { format!("command {} of {n} failed", i + 1) } else { "command failed".into() };
                return Err(format!("{which}: {e}. Nothing was changed."));
            }
        }
    }
    doc.close_group();
    let project = doc.project();
    // Return created objects in full (agents need their ids and generated refdes).
    let created: Vec<Object> =
        all.changes.iter().filter(|c| c.is_insert()).filter_map(|c| project.get(c.id)).take(50).collect();
    Ok(json!({ "executed": labels, "changes": summarize(&all), "created": created }))
}

fn summarize(changes: &ChangeSet) -> Value {
    let pick = |f: fn(&editor_core::Change) -> bool| -> Vec<ObjectId> {
        changes.changes.iter().filter(|c| f(c)).map(|c| c.id).collect()
    };
    json!({
        "inserted": pick(|c| c.is_insert()),
        "updated": pick(|c| !c.is_insert() && !c.is_remove()),
        "removed": pick(|c| c.is_remove()),
    })
}

fn project_summary(doc: &Document) -> Value {
    let p = doc.project();
    let h = doc.editor().history();
    let counts: serde_json::Map<String, Value> = ObjectKind::ALL
        .iter()
        .map(|&k| (serde_json::to_value(k).unwrap().as_str().unwrap().to_string(), json!(p.count(k))))
        .collect();
    let part_name = |c: &Component| p.library.parts.get(c.part).map(|part| part.name.clone());
    let components: Vec<Value> = p
        .schematic
        .components
        .iter()
        .take(500)
        .map(|c| json!({ "id": c.id, "refdes": c.refdes, "value": c.value, "part": part_name(c) }))
        .collect();
    let outline =
        Rect::union_all(p.board.graphics.iter().filter(|g| g.layer == LayerId::OUTLINE).map(|g| g.shape.bounds()));
    json!({
        "name": p.name,
        "path": doc.path,
        "modified": doc.is_modified(),
        "undo": h.undo_label(),
        "redo": h.redo_label(),
        "units": "Coordinates and sizes are integers in nanometers (1 mm = 1000000, 1 mil = 25400). \
                  Angles are degrees, counter-clockwise. Y points up in both views.",
        "counts": counts,
        "sheets": p.schematic.sheets.iter().map(|s| sheet_summary(p, s)).collect::<Vec<_>>(),
        "components": components,
        "board": {
            "layers": p.board.layers.iter().map(|l| json!({ "id": l.id, "name": l.name, "function": l.function })).collect::<Vec<_>>(),
            "rules": p.board.rules,
            "outline_bounds": outline.map(|r| json!({ "min": r.min, "max": r.max })),
        },
        "library_parts": p.library.parts.iter().map(|part| json!({ "id": part.id, "name": part.name })).collect::<Vec<_>>(),
    })
}

/// A sheet for `get_project`: path, symbol count, and for sub-sheets the symbol's
/// rectangle and pins on the parent sheet.
fn sheet_summary(p: &Project, s: &Sheet) -> Value {
    let mut v = json!({
        "id": s.id,
        "path": p.schematic.display_path(s.id),
        "symbols": p.schematic.symbols.iter().filter(|y| y.sheet == s.id).count(),
        "page": s.page,
        "page_rect": s.page.rect().map(|r| json!({ "min": r.min, "max": r.max })),
    });
    if s.is_root() {
        v["title_block"] = json!(p.schematic.title_block());
    } else {
        let (rect, pins) = s.layout(&p.schematic.hier_ports(s.id));
        v["symbol"] = json!({ "min": rect.min, "max": rect.max });
        v["pins"] = pins
            .iter()
            .map(|pin| json!({ "name": pin.name, "shape": pin.shape, "pos": pin.pos }))
            .collect::<Vec<_>>()
            .into();
    }
    v
}

fn library_parts(lib: &Library) -> Value {
    let parts: Vec<Value> = lib
        .parts
        .iter()
        .map(|part| {
            let units: Vec<Value> = part
                .units
                .iter()
                .map(|u| {
                    let pins: Vec<&str> = lib
                        .symbols
                        .get(u.symbol)
                        .map(|s| s.pins.iter().map(|p| p.name.as_str()).collect())
                        .unwrap_or_default();
                    json!({ "name": u.name, "pins": pins })
                })
                .collect();
            let variants: Vec<Value> = part
                .variants
                .iter()
                .map(|v| {
                    let fp = v.footprint.and_then(|f| lib.footprints.get(f)).map(|f| f.name.clone());
                    json!({ "id": v.id, "name": v.name, "footprint": fp })
                })
                .collect();
            json!({
                "id": part.id,
                "name": part.name,
                "description": part.description,
                "prefix": part.prefix,
                "default_value": part.default_value,
                "units": units,
                "variants": variants,
            })
        })
        .collect();
    json!({ "library": lib.name, "parts": parts })
}

fn list_objects(p: &Project, kind: Option<ObjectKind>, offset: usize, limit: usize) -> Value {
    // Library objects are large; list them only when asked for explicitly.
    let kinds: Vec<ObjectKind> = match kind {
        Some(k) => vec![k],
        None => ObjectKind::ALL.iter().copied().filter(|k| !k.is_library()).collect(),
    };
    let total: usize = kinds.iter().map(|&k| p.count(k)).sum();
    let objects: Vec<Object> = kinds.iter().flat_map(|&k| p.objects(k)).skip(offset).take(limit).collect();
    let next = (offset + objects.len() < total).then_some(offset + objects.len());
    json!({ "total": total, "offset": offset, "next_offset": next, "objects": objects })
}

fn component(p: &Project, refdes: &str) -> Result<Value, String> {
    let c = p.schematic.component_by_refdes(refdes).ok_or_else(|| format!("no component {refdes:?}"))?;
    let part = p.library.parts.get(c.part);
    let symbols: Vec<Value> = p
        .schematic
        .symbols_of(c.id)
        .map(|s| {
            let flagged = |pos: Point| p.schematic.no_connects.iter().any(|n| n.sheet == s.sheet && n.pos == pos);
            let pins: Vec<Value> = pin_positions(p, s.id)
                .into_iter()
                .map(|(id, name, pos)| {
                    let mut pin = json!({ "id": id, "name": name, "pos": pos });
                    if flagged(pos) {
                        pin["no_connect"] = true.into();
                    }
                    pin
                })
                .collect();
            json!({ "symbol": s, "pins": pins })
        })
        .collect();
    let footprint = p.board.footprint_of(c.id).map(|f| {
        let pads: Vec<Value> = pad_positions(p, f.id)
            .into_iter()
            .map(|(id, name, pos)| json!({ "id": id, "name": name, "pos": pos }))
            .collect();
        json!({ "footprint": f, "pads": pads })
    });
    let pin_pads: Vec<Value> =
        pin_pad_names(p, c.id).into_iter().map(|(pin, pads)| json!({ "pin": pin, "pads": pads })).collect();
    Ok(json!({
        "component": c,
        "part": part.map(|p| json!({ "name": p.name, "description": p.description, "board_part": p.is_board_part() })),
        "variant": part.and_then(|p| p.variant(c.variant)).map(|v| v.name.clone()),
        "symbols": symbols,
        "footprint": footprint,
        "pin_pads": pin_pads,
    }))
}

/// Shared helper for hosts: render a scene to PNG with a lazily created offscreen device.
pub fn render_png(
    offscreen: &mut Option<Offscreen>,
    scene: &Scene,
    camera: Option<Camera>,
    background: &Background,
    width: u32,
    height: u32,
) -> Result<Vec<u8>, String> {
    if offscreen.is_none() {
        *offscreen = Some(Offscreen::new()?);
    }
    let renderer = offscreen.as_ref().expect("created above");
    let camera = camera.unwrap_or_else(|| {
        let mut cam = Camera::default();
        if let Some(b) = scene.bounds() {
            cam.fit(b, [width as f64, height as f64], 24.0);
        }
        cam
    });
    Ok(renderer.render(scene, &camera, background, width, height).encode_png())
}

/// ERC and DRC severities and pin conflict matrix cells to change.
#[derive(Clone, Debug, Default, serde::Deserialize, schemars::JsonSchema)]
pub struct CheckSettings {
    /// Severity per schematic rule (pin_conflict takes its from the matrix).
    #[serde(default)]
    pub erc: std::collections::BTreeMap<ErcRule, Severity>,
    /// Severity per design rule check kind.
    #[serde(default)]
    pub drc: std::collections::BTreeMap<ViolationKind, Severity>,
    /// Pin conflict matrix cells (symmetric; severity ignore = the pair is fine).
    #[serde(default)]
    pub matrix: Vec<MatrixCell>,
    /// Start from the defaults (exclusions stay).
    #[serde(default)]
    pub reset: bool,
}

fn set_check_settings(doc: &mut Document, c: CheckSettings) -> Result<Value, String> {
    let p = doc.project();
    let (mut erc, mut drc) = (p.schematic.erc.clone(), p.board.drc.clone());
    if c.reset {
        erc.severities.clear();
        erc.matrix.clear();
        drc.severities.clear();
    }
    for (rule, s) in c.erc {
        erc.set_severity(rule, s);
    }
    for cell in c.matrix {
        erc.set_pin_pair(cell.a, cell.b, cell.severity);
    }
    for (kind, s) in c.drc {
        drc.set_severity(kind, s);
    }
    let group = Some(new_group());
    let p = doc.project();
    let (erc_changed, drc_changed) = (erc != p.schematic.erc, drc != p.board.drc);
    if erc_changed {
        doc.execute_grouped(Command::SetErcSettings { erc: erc.clone() }, group).map_err(|e| e.to_string())?;
    }
    if drc_changed {
        doc.execute_grouped(Command::SetDrcSettings { drc: drc.clone() }, group).map_err(|e| e.to_string())?;
    }
    doc.close_group();
    let erc_severities: std::collections::BTreeMap<ErcRule, Severity> =
        ErcRule::ALL.iter().map(|&r| (r, erc.severity(r))).collect();
    let drc_severities: std::collections::BTreeMap<ViolationKind, Severity> =
        ViolationKind::ALL.iter().map(|&k| (k, drc.severity(k))).collect();
    Ok(json!({
        "changed": erc_changed || drc_changed,
        "erc": erc_severities,
        "drc": drc_severities,
        "matrix_changes": erc.matrix,
    }))
}

/// Exclude findings (or include them again) by key: schematic findings first, else the
/// design rule check's (run now).
fn exclude_findings(doc: &mut Document, keys: &[String], comment: &str, include: bool) -> Result<Value, String> {
    doc.update_connectivity(false);
    let mut erc = doc.project().schematic.erc.clone();
    let mut drc = doc.project().board.drc.clone();
    let sch: Vec<(String, String)> =
        doc.connectivity().problems.iter().map(|p| (p.key.clone(), p.message.clone())).collect();
    let board: Vec<(String, String)> =
        doc.run_drc().violations.iter().map(|v| (v.key.clone(), v.message.clone())).collect();
    for key in keys {
        let (list, found) = if let Some(f) = sch.iter().find(|f| &f.0 == key) {
            (&mut erc.exclusions, Some(f))
        } else if let Some(f) = board.iter().find(|f| &f.0 == key) {
            (&mut drc.exclusions, Some(f))
        } else if erc.exclusions.iter().any(|e| &e.key == key) {
            (&mut erc.exclusions, None)
        } else if drc.exclusions.iter().any(|e| &e.key == key) {
            (&mut drc.exclusions, None)
        } else {
            return Err(format!("no finding with key {key} (see get_netlist problems and run_drc)"));
        };
        list.retain(|e| &e.key != key);
        if !include {
            let message = found.map_or(String::new(), |f| f.1.clone());
            list.push(Exclusion { key: key.clone(), message, comment: comment.to_string() });
        }
    }
    let group = Some(new_group());
    let p = doc.project();
    let (erc_changed, drc_changed) = (erc != p.schematic.erc, drc != p.board.drc);
    if erc_changed {
        doc.execute_grouped(Command::SetErcSettings { erc }, group).map_err(|e| e.to_string())?;
    }
    if drc_changed {
        doc.execute_grouped(Command::SetDrcSettings { drc }, group).map_err(|e| e.to_string())?;
    }
    doc.close_group();
    Ok(json!({ "changed": erc_changed || drc_changed, "keys": keys, "excluded": !include }))
}

fn netlist(project: &Project, c: &editor_core::connectivity::Connectivity, include_excluded: bool) -> Value {
    let nets: Vec<Value> = c
        .netlist()
        .filter(|n| n.pins.len() > 1 || n.named)
        .map(|n| {
            let pins: Vec<Value> = n
                .pins
                .iter()
                .map(|p| json!({ "ref": p.refdes, "pin": p.pin_name, "pads": p.pads, "pos": p.pos }))
                .collect();
            let mut net = json!({ "name": n.name, "pins": pins });
            if !n.sheet_pins.is_empty() {
                let sheet_pins: Vec<String> = n
                    .sheet_pins
                    .iter()
                    .map(|(sheet, name)| format!("{}:{name}", project.schematic.display_path(*sheet)))
                    .collect();
                net["sheet_pins"] = json!(sheet_pins);
            }
            net
        })
        .collect();
    let open: std::collections::HashSet<(Id, Id)> = c.unconnected.iter().map(|&(s, p, _)| (s, p)).collect();
    let unconnected: Vec<String> =
        c.nets.iter().flat_map(|n| &n.pins).filter(|p| open.contains(&(p.symbol, p.pin))).map(|p| p.label()).collect();
    let buses: Vec<Value> = c
        .buses
        .iter()
        .map(|b| {
            let members: Vec<Value> = b
                .members
                .iter()
                .map(|m| {
                    let net = c.member_net(project, b, m);
                    let pins: Vec<String> = net.map(|n| n.pins.iter().map(|p| p.label()).collect()).unwrap_or_default();
                    json!({ "name": m, "net": net.map(|n| n.name.as_str()), "pins": pins })
                })
                .collect();
            json!({
                "name": b.name,
                "members": members,
                "bus_wires": b.wires,
                "entries": b.entries.len(),
            })
        })
        .collect();
    let problems: Vec<&editor_core::connectivity::Problem> =
        c.problems.iter().filter(|p| include_excluded || !p.excluded).collect();
    let count = |s: Severity| c.problems.iter().filter(|p| !p.excluded && p.severity == s).count();
    json!({
        "nets": nets,
        "buses": buses,
        "unconnected_pins": unconnected,
        "problems": problems,
        "errors": count(Severity::Error),
        "warnings": count(Severity::Warning),
        "excluded": c.problems.iter().filter(|p| p.excluded).count(),
    })
}

fn routing(p: &Project, c: &editor_core::copper::BoardConnectivity) -> Value {
    let unrouted: Vec<Value> =
        c.ratsnest.iter().map(|r| json!({ "net": c.nets[r.net as usize], "from": r.a, "to": r.b })).collect();
    let shorts: Vec<Value> = c.shorts.iter().map(|(nets, at)| json!({ "nets": nets, "at": at })).collect();
    let layers: Vec<Value> = p.board.copper_layers().map(|l| json!({ "id": l.id, "name": l.name })).collect();
    use editor_core::copper::{CopperItem, Flag};
    let flagged: Vec<Value> = c
        .flagged()
        .filter_map(|item| {
            if matches!(item.item, CopperItem::Pad { .. }) {
                return None;
            }
            let id = item.item.object();
            let problem = match item.flag {
                Some(Flag::Short) => "short",
                _ => "unconnected",
            };
            Some(json!({ "object": id, "problem": problem }))
        })
        .collect();
    json!({
        "fully_routed": c.ratsnest.is_empty() && c.shorts.is_empty(),
        "unrouted": unrouted,
        "shorts": shorts,
        "flagged": flagged,
        "copper_layers": layers,
        "rules": p.board.rules,
    })
}
