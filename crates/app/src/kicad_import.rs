//! "Import from KiCad" window: search KiCad's symbol and footprint libraries, pick symbols
//! (each with a footprint, making a part) and lone footprints, and import them into a
//! library.
//!
//! The libraries searched are KiCad's downloaded ones (see [`kicad::download`]) or else an
//! installed KiCad's, plus folders added by hand or with `--kicad`. Downloads and update
//! checks run on a background thread, polled every frame by [`KicadImport::poll`].

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::thread::JoinHandle;

use egui::RichText;
use kicad::download::{self, DownloadError, Installed, Source};
use kicad::import::{Request, pin_numbers, suggested_footprints};
use kicad::{Catalog, Imported};
use mcp::host::{Reply, Responder};

use crate::fields::{self, hint};
use crate::theme;

const RESULT_LIMIT: usize = 400;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Symbols,
    Footprints,
}

/// A symbol picked for import.
struct Pick {
    symbol: String,
    description: String,
    units: usize,
    pins: usize,
    /// Chosen footprint (`None`: import the symbol only).
    footprint: Option<String>,
    /// Footprints offered in the picker: the default, then filter matches.
    candidates: Vec<String>,
    power: bool,
}

/// Outcome of the last import, shown in the window.
struct Report {
    summary: String,
    warnings: Vec<String>,
    errors: Vec<String>,
}

/// What the application should do after the window was drawn.
pub enum ImportAction {
    /// Import into the library with this key, or into a new library (`None`).
    Import(Option<u64>, Request),
}

/// A download running in the background.
struct DownloadJob {
    bytes: Arc<AtomicU64>,
    cancel: Arc<AtomicBool>,
    handle: JoinHandle<Result<(Installed, bool), DownloadError>>,
    /// Agents waiting for the result.
    replies: Vec<Responder>,
}

/// State of the downloaded libraries shown in the window.
enum Remote {
    Idle,
    Failed(String),
    Checking(JoinHandle<Result<String, DownloadError>>),
    UpToDate,
    Available(String),
}

pub struct KicadImport {
    pub open: bool,
    /// Folders added by hand or with `--kicad`, searched besides the downloaded or
    /// installed libraries.
    pub extra: Vec<PathBuf>,
    /// Download folder (`None`: no configuration folder, downloads unavailable).
    root: Option<PathBuf>,
    download: Option<DownloadJob>,
    remote: Remote,
    catalog: Option<Catalog>,
    query: String,
    kind: Kind,
    results: Vec<String>,
    results_for: Option<(String, Kind)>,
    picks: Vec<Pick>,
    footprints: Vec<String>,
    /// Pick that receives footprints chosen in the footprint search.
    active: Option<usize>,
    /// Target library key; `None` = a new library.
    pub target: Option<u64>,
    report: Option<Report>,
    error: Option<String>,
}

impl KicadImport {
    pub fn new(extra: Vec<PathBuf>, root: Option<PathBuf>) -> Self {
        KicadImport {
            open: false,
            extra,
            root,
            download: None,
            remote: Remote::Idle,
            catalog: None,
            query: String::new(),
            kind: Kind::Symbols,
            results: Vec::new(),
            results_for: None,
            picks: Vec::new(),
            footprints: Vec::new(),
            active: None,
            target: None,
            report: None,
            error: None,
        }
    }

    pub fn set_query(&mut self, query: &str, footprints: bool) {
        self.query = query.to_string();
        self.kind = if footprints { Kind::Footprints } else { Kind::Symbols };
    }

    /// Picked symbols and footprints (for startup automation).
    pub fn has_picks(&self) -> bool {
        !self.picks.is_empty() || !self.footprints.is_empty()
    }

    pub fn pending_request(&self) -> Request {
        self.request()
    }

    /// Every folder searched: the added ones, then the downloaded or installed libraries.
    fn paths(&self) -> Vec<PathBuf> {
        self.extra.iter().cloned().chain(download::library_paths(self.root.as_deref())).collect()
    }

    fn catalog(&mut self) -> &mut Catalog {
        let paths = self.paths();
        self.catalog.get_or_insert_with(|| Catalog::scan(&paths))
    }

    fn installed(&self) -> Option<Installed> {
        self.root.as_deref().and_then(download::installed)
    }

    /// Start downloading (`version`: that release; else the newest unless already
    /// downloaded and not `update`). An agent's `reply` is answered when it finishes.
    pub fn start_download(
        &mut self,
        ctx: &egui::Context,
        update: bool,
        version: Option<String>,
        reply: Option<Responder>,
    ) {
        let Some(root) = self.root.clone() else {
            if let Some(r) = reply {
                let _ = r.send(Err("no configuration folder to download KiCad libraries to".into()));
            }
            return;
        };
        if let Some(job) = &mut self.download {
            job.replies.extend(reply);
            return;
        }
        let bytes = Arc::new(AtomicU64::new(0));
        let cancel = Arc::new(AtomicBool::new(false));
        let (b, c, ctx) = (bytes.clone(), cancel.clone(), ctx.clone());
        let handle = std::thread::spawn(move || {
            let mut progress = |n| {
                b.store(n, Ordering::Relaxed);
                ctx.request_repaint();
                !c.load(Ordering::Relaxed)
            };
            let r = download::ensure(&root, update, version.as_deref(), &Source::default(), &mut progress);
            ctx.request_repaint();
            r
        });
        self.remote = Remote::Idle;
        self.download = Some(DownloadJob { bytes, cancel, handle, replies: reply.into_iter().collect() });
    }

    fn check_updates(&mut self, ctx: &egui::Context) {
        let ctx = ctx.clone();
        self.remote = Remote::Checking(std::thread::spawn(move || {
            let r = download::latest_version(&Source::default());
            ctx.request_repaint();
            r
        }));
    }

    /// Collect finished background work (call every frame).
    pub fn poll(&mut self) {
        if self.download.as_ref().is_some_and(|j| j.handle.is_finished()) {
            let job = self.download.take().unwrap();
            let result = job.handle.join().unwrap_or_else(|_| Err(DownloadError::Http("download crashed".into())));
            match &result {
                Ok(_) => self.rescan(),
                Err(DownloadError::Cancelled) => {}
                Err(e) => self.remote = Remote::Failed(e.to_string()),
            }
            for r in job.replies {
                let _ = r.send(match &result {
                    Ok((installed, fresh)) => Ok(Reply::Json(mcp::host::kicad_download_reply(installed, *fresh))),
                    Err(e) => Err(e.to_string()),
                });
            }
        }
        if let Remote::Checking(h) = &self.remote
            && h.is_finished()
        {
            let Remote::Checking(h) = std::mem::replace(&mut self.remote, Remote::Idle) else { unreachable!() };
            let current = self.installed().map(|i| i.version);
            self.remote = match h.join().unwrap_or_else(|_| Err(DownloadError::Http("check crashed".into()))) {
                Ok(v) if Some(&v) == current.as_ref() => Remote::UpToDate,
                Ok(v) => Remote::Available(v),
                Err(e) => Remote::Failed(e.to_string()),
            };
        }
    }

    fn rescan(&mut self) {
        self.catalog = None;
        self.results_for = None;
    }

    /// Add a symbol to the selection with its suggested footprint.
    pub fn pick_symbol(&mut self, full: &str) {
        if self.picks.iter().any(|p| p.symbol == full) {
            return;
        }
        let catalog = self.catalog();
        match catalog.load_symbol(full) {
            Ok(sym) => {
                let suggested = suggested_footprints(catalog, &sym);
                let mut candidates = suggested.clone();
                for (_, c) in catalog.footprint_candidates(&sym.fp_filters, 60) {
                    if !candidates.contains(&c) {
                        candidates.push(c);
                    }
                }
                self.picks.push(Pick {
                    symbol: full.to_string(),
                    description: sym.description.clone(),
                    units: sym.units.len(),
                    pins: pin_numbers(&sym).len(),
                    footprint: suggested.first().cloned(),
                    candidates,
                    power: sym.power,
                });
                self.active = Some(self.picks.len() - 1);
                self.error = None;
            }
            Err(e) => self.error = Some(e.to_string()),
        }
    }

    pub fn pick_footprint(&mut self, full: &str) {
        if !self.footprints.iter().any(|f| f == full) {
            self.footprints.push(full.to_string());
        }
    }

    fn request(&self) -> Request {
        Request {
            symbols: self
                .picks
                .iter()
                .filter(|p| !p.power)
                .map(|p| (p.symbol.clone(), p.footprint.iter().cloned().collect()))
                .collect(),
            footprints: self.footprints.clone(),
        }
    }

    /// Search the catalog; also returns the symbol and footprint counts.
    pub fn search(&mut self, query: &str, footprints: bool, limit: usize) -> (Vec<String>, usize, usize) {
        let c = self.catalog();
        let r = if footprints { c.search_footprints(query, limit) } else { c.search_symbols(query, limit) };
        (r, c.symbol_count(), c.footprint_count())
    }

    /// Suggested footprints of a symbol.
    pub fn suggested(&mut self, symbol: &str) -> Result<Vec<String>, String> {
        let c = self.catalog();
        let sym = c.load_symbol(symbol).map_err(|e| e.to_string())?;
        Ok(suggested_footprints(c, &sym))
    }

    /// Run an import request against the catalog.
    pub fn import(&mut self, req: &Request) -> Imported {
        kicad::import(self.catalog(), req)
    }

    /// Show the result of an import and clear the selection.
    pub fn finish(&mut self, imported: &Imported, target: &str, added: usize, replaced: usize) {
        let summary = format!(
            "{} part(s), {} symbol(s), {} footprint(s) into {target}: {added} added, {replaced} updated",
            imported.parts.len(),
            imported.symbols.len(),
            imported.footprints.len()
        );
        self.report = Some(Report { summary, warnings: imported.warnings.clone(), errors: imported.errors.clone() });
        self.picks.clear();
        self.footprints.clear();
        self.active = None;
    }

    pub fn fail(&mut self, error: String) {
        self.report = Some(Report { summary: "Import failed".into(), warnings: Vec::new(), errors: vec![error] });
    }

    pub fn ui(&mut self, ctx: &egui::Context, libraries: &[(u64, String)]) -> Option<ImportAction> {
        if !self.open {
            return None;
        }
        let mut open = self.open;
        let mut action = None;
        egui::Window::new("Import from KiCad")
            .open(&mut open)
            .default_size([980.0, 620.0])
            .resizable(true)
            .collapsible(false)
            .show(ctx, |ui| action = self.contents(ui, libraries));
        self.open = open;
        action
    }

    fn contents(&mut self, ui: &mut egui::Ui, libraries: &[(u64, String)]) -> Option<ImportAction> {
        let mut action = None;
        // ---- Sources
        let (syms, sym_libs, fps, fp_libs) = {
            let c = self.catalog();
            (c.symbol_count(), c.symbol_libs.len(), c.footprint_count(), c.footprint_libs.len())
        };
        ui.horizontal_wrapped(|ui| {
            let text = format!("{syms} symbols in {sym_libs} libraries, {fps} footprints in {fp_libs} libraries");
            ui.label(RichText::new(text).color(if syms + fps == 0 { theme::warn() } else { theme::text_dim() }));
            if ui
                .button("Add folder…")
                .on_hover_text("A KiCad symbols or footprints folder, or a single library")
                .clicked()
                && let Some(dir) = rfd::FileDialog::new().set_title("KiCad library folder").pick_folder()
            {
                self.extra.push(dir);
                self.rescan();
            }
            if ui.button("Rescan").clicked() {
                self.rescan();
            }
        });
        self.download_ui(ui, syms + fps == 0);
        egui::CollapsingHeader::new(RichText::new("Library folders").color(theme::text_dim()))
            .id_salt("kicad_paths")
            .show(ui, |ui| {
                let added = self.extra.len();
                let mut remove = None;
                for (i, p) in self.paths().iter().enumerate() {
                    ui.horizontal(|ui| {
                        if i < added && ui.small_button("Remove").clicked() {
                            remove = Some(i);
                        }
                        ui.label(RichText::new(p.display().to_string()).monospace());
                    });
                }
                if let Some(i) = remove {
                    self.extra.remove(i);
                    self.rescan();
                }
            });
        ui.separator();

        // ---- Search (left) and selection (right)
        let bottom_height = 120.0;
        let height = (ui.available_height() - bottom_height).max(200.0);
        ui.columns(2, |cols| {
            cols[0].set_min_height(height);
            self.search_ui(&mut cols[0], height);
            self.selection_ui(&mut cols[1], height);
        });
        ui.separator();

        // ---- Target and import
        ui.horizontal_wrapped(|ui| {
            ui.label("Import into");
            let mut options: Vec<(Option<u64>, String)> = vec![(None, "New library".into())];
            options.extend(libraries.iter().map(|(k, n)| (Some(*k), n.clone())));
            if self.target.is_some_and(|t| !libraries.iter().any(|l| l.0 == t)) {
                self.target = None;
            }
            if let Some(t) = fields::combo(ui, "kicad_target", &self.target, &options) {
                self.target = t;
            }
            let req = self.request();
            let n = req.symbols.len() + req.footprints.len();
            let label = format!("Import {n} item{}", if n == 1 { "" } else { "s" });
            if ui.add_enabled(n > 0, egui::Button::new(RichText::new(label).strong())).clicked() {
                action = Some(ImportAction::Import(self.target, req));
            }
            if (!self.picks.is_empty() || !self.footprints.is_empty()) && ui.button("Clear").clicked() {
                self.picks.clear();
                self.footprints.clear();
                self.active = None;
            }
        });
        if let Some(r) = &self.report {
            ui.label(RichText::new(&r.summary).color(if r.errors.is_empty() {
                theme::success()
            } else {
                theme::warn()
            }));
            for e in &r.errors {
                ui.label(RichText::new(e).color(theme::warn()));
            }
            if !r.warnings.is_empty() {
                egui::CollapsingHeader::new(format!("{} note(s)", r.warnings.len())).id_salt("kicad_warnings").show(
                    ui,
                    |ui| {
                        egui::ScrollArea::vertical().id_salt("kicad_warn_scroll").max_height(80.0).show(ui, |ui| {
                            for w in &r.warnings {
                                ui.label(RichText::new(w).color(theme::text_dim()));
                            }
                        });
                    },
                );
            }
        }
        action
    }

    /// Download progress, the download offer when nothing was found, and updates.
    fn download_ui(&mut self, ui: &mut egui::Ui, nothing_found: bool) {
        if self.root.is_none() {
            return;
        }
        let mb = |b: u64| b as f64 / 1e6;
        if let Some(job) = &self.download {
            ui.horizontal(|ui| {
                let done = job.bytes.load(Ordering::Relaxed);
                let total = download::APPROX_DOWNLOAD_BYTES;
                let bar = egui::ProgressBar::new((done as f32 / total as f32).min(0.99))
                    .desired_width(240.0)
                    .text(format!("{:.1} MB of about {:.0} MB", mb(done), mb(total)));
                ui.label("Downloading KiCad libraries…");
                ui.add(bar);
                if ui.button("Cancel").clicked() {
                    job.cancel.store(true, Ordering::Relaxed);
                }
            });
            return;
        }
        let ctx = ui.ctx().clone();
        let offer = format!(
            "Download KiCad {}.x libraries ({:.0} MB download, {:.0} MB on disk)",
            download::SUPPORTED_MAJOR,
            mb(download::APPROX_DOWNLOAD_BYTES),
            mb(download::APPROX_DISK_BYTES)
        );
        let installed = self.installed();
        ui.horizontal_wrapped(|ui| {
            match &installed {
                Some(i) => {
                    ui.label(
                        RichText::new(format!("Downloaded KiCad libraries {}", i.version)).color(theme::text_dim()),
                    );
                }
                None if nothing_found
                    && ui
                        .button(RichText::new(&offer).strong())
                        .on_hover_text("KiCad's official symbols and footprints, without 3D models")
                        .clicked() =>
                {
                    self.start_download(&ctx, false, None, None);
                }
                None => {}
            }
            match &self.remote {
                Remote::Idle if installed.is_some() => {
                    if ui.link("Check for updates").clicked() {
                        self.check_updates(&ctx);
                    }
                }
                Remote::Idle => {}
                Remote::Checking(_) => {
                    ui.spinner();
                    ui.label(RichText::new("Checking…").color(theme::text_dim()));
                }
                Remote::UpToDate => {
                    ui.label(RichText::new("Up to date").color(theme::success()));
                }
                Remote::Available(v) => {
                    let v = v.clone();
                    if ui.button(format!("Update to {v}")).clicked() {
                        self.start_download(&ctx, true, Some(v), None);
                    }
                }
                Remote::Failed(e) => {
                    ui.label(RichText::new(e).color(theme::warn()));
                    if ui.button("Retry").clicked() {
                        if installed.is_some() {
                            self.check_updates(&ctx);
                        } else {
                            self.start_download(&ctx, false, None, None);
                        }
                    }
                }
            }
        });
    }

    fn search_ui(&mut self, ui: &mut egui::Ui, height: f32) {
        ui.horizontal(|ui| {
            ui.selectable_value(&mut self.kind, Kind::Symbols, "Symbols");
            ui.selectable_value(&mut self.kind, Kind::Footprints, "Footprints");
        });
        let hint_text = match self.kind {
            Kind::Symbols => "Search symbols, e.g. \"device r\", \"lm358\", \"stm32f103\"",
            Kind::Footprints => "Search footprints, e.g. \"soic-8\", \"0603\"",
        };
        ui.add(egui::TextEdit::singleline(&mut self.query).hint_text(hint_text).desired_width(f32::INFINITY));
        let key = (self.query.clone(), self.kind);
        if self.results_for.as_ref() != Some(&key) {
            let q = self.query.trim().to_string();
            let kind = self.kind;
            let c = self.catalog();
            self.results = match kind {
                _ if q.is_empty() => Vec::new(),
                Kind::Symbols => c.search_symbols(&q, RESULT_LIMIT),
                Kind::Footprints => c.search_footprints(&q, RESULT_LIMIT),
            };
            self.results_for = Some(key);
        }
        if let Some(e) = &self.error {
            ui.label(RichText::new(e).color(theme::warn()));
        }
        if self.results.len() >= RESULT_LIMIT {
            hint(ui, &format!("First {RESULT_LIMIT} matches; refine the search."));
        }
        let mut add = None;
        let mut assign = None;
        let active = self.active.and_then(|i| self.picks.get(i)).map(|p| p.symbol.clone());
        egui::ScrollArea::vertical()
            .id_salt("kicad_results")
            .max_height(height - 60.0)
            .auto_shrink([false, false])
            .show(ui, |ui| {
                for r in &self.results {
                    ui.horizontal(|ui| {
                        if ui.small_button("Add").clicked() {
                            add = Some(r.clone());
                        }
                        if self.kind == Kind::Footprints
                            && let Some(sym) = &active
                            && ui.small_button("Use").on_hover_text(format!("Footprint of {sym}")).clicked()
                        {
                            assign = Some(r.clone());
                        }
                        let (lib, name) = r.split_once(':').unwrap_or(("", r));
                        ui.label(RichText::new(lib).color(theme::text_dim()));
                        ui.label(name);
                    });
                }
            });
        match (add, self.kind) {
            (Some(r), Kind::Symbols) => self.pick_symbol(&r),
            (Some(r), Kind::Footprints) => self.pick_footprint(&r),
            _ => {}
        }
        if let (Some(f), Some(i)) = (assign, self.active)
            && let Some(p) = self.picks.get_mut(i)
        {
            if !p.candidates.contains(&f) {
                p.candidates.insert(0, f.clone());
            }
            p.footprint = Some(f);
        }
    }

    fn selection_ui(&mut self, ui: &mut egui::Ui, height: f32) {
        ui.label(RichText::new("To import").strong());
        if self.picks.is_empty() && self.footprints.is_empty() {
            hint(ui, "Add symbols (they become parts with the chosen footprint) or lone footprints.");
            return;
        }
        let mut remove = None;
        let mut remove_fp = None;
        egui::ScrollArea::vertical().id_salt("kicad_picks").max_height(height - 30.0).auto_shrink([false, false]).show(
            ui,
            |ui| {
                for (i, p) in self.picks.iter_mut().enumerate() {
                    let active = self.active == Some(i);
                    let frame = egui::Frame::group(ui.style())
                        .fill(if active { theme::panel() } else { theme::panel_dark() })
                        .inner_margin(egui::Margin::same(6));
                    let r = frame.show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.horizontal(|ui| {
                            if ui.small_button("Remove").clicked() {
                                remove = Some(i);
                            }
                            ui.label(RichText::new(&p.symbol).strong());
                            let units = if p.units > 1 { format!(", {} units", p.units) } else { String::new() };
                            let pins = if p.pins == 1 { "1 pin".to_string() } else { format!("{} pins", p.pins) };
                            ui.label(RichText::new(format!("{pins}{units}")).color(theme::text_dim()));
                        });
                        if !p.description.is_empty() {
                            ui.label(RichText::new(&p.description).color(theme::text_dim()).small());
                        }
                        if p.power {
                            ui.label(
                                RichText::new("Power symbol: use a net label instead; it will not be imported.")
                                    .color(theme::warn()),
                            );
                            return;
                        }
                        ui.horizontal(|ui| {
                            ui.label("Footprint");
                            let mut options: Vec<(Option<String>, String)> =
                                vec![(None, "— symbol only —".to_string())];
                            options.extend(p.candidates.iter().map(|c| (Some(c.clone()), c.clone())));
                            if let Some(f) = fields::combo(ui, ("kicad_fp", i), &p.footprint, &options) {
                                p.footprint = f;
                            }
                        });
                        if p.candidates.is_empty() {
                            hint(ui, "No matching footprint; search one under Footprints and press Use.");
                        }
                    });
                    if r.response.interact(egui::Sense::click()).clicked() {
                        self.active = Some(i);
                    }
                }
                for (i, f) in self.footprints.iter().enumerate() {
                    ui.horizontal(|ui| {
                        if ui.small_button("Remove").clicked() {
                            remove_fp = Some(i);
                        }
                        ui.label(RichText::new("Footprint").color(theme::text_dim()));
                        ui.label(f);
                    });
                }
            },
        );
        if let Some(i) = remove {
            self.picks.remove(i);
            self.active = self.active.and_then(|a| {
                if a == i {
                    None
                } else if a > i {
                    Some(a - 1)
                } else {
                    Some(a)
                }
            });
        }
        if let Some(i) = remove_fp {
            self.footprints.remove(i);
        }
    }
}
