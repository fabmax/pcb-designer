//! "Import KiCad parts" window: search KiCad's symbol and footprint libraries, pick symbols
//! (each with the footprints of its variants, making a part) and lone footprints, and
//! import them into a library.
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
use kicad::preview::PreviewCache;
use kicad::{Catalog, Fit, Imported};
use mcp::host::{Reply, Responder};
use view::LibraryDoc;
use view::item::Item;

use crate::fields::{self, hint};
use crate::nav_list;
use crate::preview::{Empty, Preview};
use crate::theme;

const RESULT_LIMIT: usize = 400;
/// Matches counted for the Parts panel (more show as "1000+").
pub const MATCH_LIMIT: usize = 1000;

/// What the KiCad libraries have for a Parts panel search.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KicadMatches {
    /// The libraries are being scanned.
    Scanning,
    /// None installed or downloaded.
    NoLibraries,
    /// No query: the libraries are there.
    Available,
    /// Matches of the query (at most [`MATCH_LIMIT`]).
    Found(usize),
}
/// Height of the preview boxes under the selection.
const PREVIEW_HEIGHT: f32 = 180.0;
/// Height of the names over the preview boxes.
const CAPTION_HEIGHT: f32 = 18.0;

/// `Lib:Name` of the symbol and footprint previewed.
type PreviewTarget = (Option<String>, Option<String>);

/// The selected row of the window's lists; it is what the previews show.
#[derive(Clone, PartialEq, Eq)]
enum Selected {
    /// A symbol search result, with its suggested footprint.
    SymbolResult {
        name: String,
        footprint: Option<String>,
    },
    FootprintResult(String),
    /// A footprint in a pick's list.
    Candidate {
        pick: String,
        footprint: String,
    },
    /// A lone footprint to import.
    Lone(String),
}

/// Previews of the selection (else the active pick): KiCad items converted (and
/// cached), shown from a scratch library.
#[derive(Default)]
struct Previews {
    cache: PreviewCache,
    /// What `doc` holds.
    shown: Option<PreviewTarget>,
    doc: Option<LibraryDoc>,
    symbol: (Option<Item>, Option<String>),
    footprint: (Option<Item>, Option<String>),
    symbol_view: Preview,
    footprint_view: Preview,
}

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
    /// Footprints offered: the default and those used by hand, then the filter matches
    /// best first. Each checked one becomes a variant; none checked: a part without
    /// footprint, kept off the board.
    candidates: Vec<Candidate>,
    /// Also list footprints missing pins (always when none fits).
    show_missing: bool,
    /// Why this symbol won't be imported (power symbols, hidden power pins).
    refused: Option<String>,
}

struct Candidate {
    name: String,
    /// How its pads fit the pins (`None`: not readable).
    fit: Option<Fit>,
    checked: bool,
}

impl Candidate {
    /// Listed unless it misses pins and those are hidden (checked ones always are).
    fn listed(&self, show_missing: bool) -> bool {
        self.checked || show_missing || self.fit.is_none_or(Fit::fits)
    }
}

impl Pick {
    /// The footprints of the variants, in list order.
    fn chosen(&self) -> impl Iterator<Item = &String> {
        self.candidates.iter().filter(|c| c.checked).map(|c| &c.name)
    }

    /// Add a footprint (chosen in the footprint search) at the top, checked.
    fn use_footprint(&mut self, name: String, fit: Option<Fit>) {
        match self.candidates.iter_mut().find(|c| c.name == name) {
            Some(c) => c.checked = true,
            None => self.candidates.insert(0, Candidate { name, fit, checked: true }),
        }
    }
}

/// Short note on a footprint's pad fit.
fn fit_note(fit: Option<Fit>) -> Option<String> {
    match fit? {
        Fit::Exact => None,
        Fit::Extra(1) => Some("+1 pad".into()),
        Fit::Extra(n) => Some(format!("+{n} pads")),
        Fit::Missing(1) => Some("1 pin without pad".into()),
        Fit::Missing(n) => Some(format!("{n} pins without pad")),
    }
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
    /// A catalog being scanned in the background (for the Parts panel's match count),
    /// with the folders it scans.
    scanning: Option<(Vec<PathBuf>, std::thread::JoinHandle<Catalog>)>,
    /// The Parts panel's last count: query, footprints, matches.
    counted: Option<(String, bool, KicadMatches)>,
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
    error: Option<String>,
    /// The selected row (`None`: the previews show the active pick).
    selected: Option<Selected>,
    /// Scroll the "To import" list to this item next frame (just added).
    reveal: Option<Reveal>,
    previews: Previews,
}

/// An item of the "To import" list to scroll into view.
#[derive(Clone, PartialEq, Eq)]
enum Reveal {
    Pick(String),
    Lone(String),
}

/// The search field (lists hand the focus back to it on Esc).
fn search_id() -> egui::Id {
    egui::Id::unique("kicad_search")
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
            scanning: None,
            counted: None,
            query: String::new(),
            kind: Kind::Symbols,
            results: Vec::new(),
            results_for: None,
            picks: Vec::new(),
            footprints: Vec::new(),
            active: None,
            target: None,
            error: None,
            selected: None,
            reveal: None,
            previews: Previews::default(),
        }
    }

    /// Where libraries are downloaded to (the settings changed it).
    pub fn set_root(&mut self, root: Option<PathBuf>) {
        if root != self.root {
            self.root = root;
            self.catalog = None;
            self.counted = None;
            self.results_for = None;
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
        // A background scan of the same folders is waited for rather than repeated.
        if self.catalog.is_none()
            && let Some((scanned, job)) = self.scanning.take()
            && let Ok(c) = job.join()
            && scanned == paths
        {
            self.catalog = Some(c);
        }
        self.catalog.get_or_insert_with(|| Catalog::scan(&paths))
    }

    /// How many KiCad symbols (`footprints`: footprints) match `query`, without blocking:
    /// the first call scans the libraries in the background.
    pub fn match_count(&mut self, ctx: &egui::Context, query: &str, footprints: bool) -> KicadMatches {
        if self.catalog.is_none() {
            let paths = self.paths();
            match self.scanning.take() {
                Some((scanned, job)) if job.is_finished() => {
                    if let Ok(c) = job.join()
                        && scanned == paths
                    {
                        self.catalog = Some(c);
                    }
                }
                Some(running) => {
                    self.scanning = Some(running);
                    ctx.request_repaint_after(std::time::Duration::from_millis(100));
                    return KicadMatches::Scanning;
                }
                None => {
                    let scan = paths.clone();
                    self.scanning = Some((paths, std::thread::spawn(move || Catalog::scan(&scan))));
                    ctx.request_repaint_after(std::time::Duration::from_millis(100));
                    return KicadMatches::Scanning;
                }
            }
            if self.catalog.is_none() {
                // Scanned other folders (the settings changed meanwhile): again.
                return self.match_count(ctx, query, footprints);
            }
        }
        let query = query.trim();
        if let Some((q, f, m)) = &self.counted
            && q == query
            && *f == footprints
        {
            return *m;
        }
        let c = self.catalog.as_ref().expect("scanned");
        let total = if footprints { c.footprint_count() } else { c.symbol_count() };
        let m = if total == 0 {
            KicadMatches::NoLibraries
        } else if query.is_empty() {
            KicadMatches::Available
        } else {
            let found =
                if footprints { c.search_footprints(query, MATCH_LIMIT) } else { c.search_symbols(query, MATCH_LIMIT) };
            KicadMatches::Found(found.len())
        };
        self.counted = Some((query.to_string(), footprints, m));
        m
    }

    /// Open the window searching `query` (symbols, or `footprints`).
    pub fn open_with(&mut self, query: &str, footprints: bool) {
        self.set_query(query.trim(), footprints);
        self.open = true;
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
        self.counted = None;
        self.results_for = None;
        self.forget_previews();
    }

    /// Drop the converted items (the libraries changed, or the window closed).
    fn forget_previews(&mut self) {
        let p = &mut self.previews;
        p.cache.clear();
        p.shown = None;
        p.doc = None;
    }

    /// What the previews show: the selected row, else the active pick.
    fn preview_target(&self) -> PreviewTarget {
        match &self.selected {
            Some(Selected::SymbolResult { name, footprint }) => return (Some(name.clone()), footprint.clone()),
            Some(Selected::FootprintResult(f) | Selected::Lone(f)) => return (None, Some(f.clone())),
            Some(Selected::Candidate { pick, footprint }) => return (Some(pick.clone()), Some(footprint.clone())),
            None => {}
        }
        match self.active.and_then(|i| self.picks.get(i)) {
            Some(p) => (Some(p.symbol.clone()), p.chosen().next().cloned()),
            None => (None, None),
        }
    }

    /// Convert the previewed items when the target changed.
    fn update_previews(&mut self) {
        let target = self.preview_target();
        if self.previews.shown.as_ref() == Some(&target) {
            return;
        }
        self.catalog();
        let catalog = self.catalog.as_mut().expect("catalog scanned");
        let p = &mut self.previews;
        let mut lib = model::Library::new("KiCad preview");
        p.symbol = (None, None);
        p.footprint = (None, None);
        if let Some(name) = &target.0 {
            match p.cache.symbol(catalog, name) {
                Ok(s) => {
                    p.symbol.0 = Some(Item::Symbol(s.id));
                    lib.symbols.insert(s);
                }
                Err(e) => p.symbol.1 = Some(e),
            }
        }
        if let Some(name) = &target.1 {
            match p.cache.footprint(catalog, name) {
                Ok(f) => {
                    p.footprint.0 = Some(Item::Footprint(f.id));
                    lib.footprints.insert(f);
                }
                Err(e) => p.footprint.1 = Some(e),
            }
        }
        p.doc = Some(LibraryDoc::new(lib));
        p.symbol_view.invalidate();
        p.footprint_view.invalidate();
        p.shown = Some(target);
    }

    /// Symbol and footprint previews side by side.
    fn preview_ui(&mut self, ui: &mut egui::Ui) {
        self.update_previews();
        let p = &mut self.previews;
        let Some(doc) = &p.doc else { return };
        let gap = ui.spacing().item_spacing.x;
        let size = egui::vec2(((ui.available_width() - gap) / 2.0).max(60.0), PREVIEW_HEIGHT);
        let (symbol_hint, footprint_hint) = match &p.shown {
            Some((None, None)) | None => ("Click a search result or a pick to preview it", ""),
            Some((None, Some(_))) => ("No symbol", ""),
            Some((Some(_), None)) => ("", "No footprint"),
            Some(_) => ("", ""),
        };
        fn empty<'a>(error: &'a Option<String>, hint: &'a str) -> Empty<'a> {
            error.as_deref().map_or(Empty::Hint(hint), Empty::Error)
        }
        let names = p.shown.clone().unwrap_or_default();
        let caption = |ui: &mut egui::Ui, name: &Option<String>| {
            // A line even when empty, so the boxes don't move.
            ui.allocate_ui(egui::vec2(size.x, CAPTION_HEIGHT), |ui| {
                ui.set_min_size(egui::vec2(size.x, CAPTION_HEIGHT));
                if let Some(n) = name {
                    ui.add(egui::Label::new(RichText::new(n).color(theme::text_dim())).truncate());
                }
            });
        };
        ui.horizontal(|ui| {
            caption(ui, &names.0);
            caption(ui, &names.1);
        });
        ui.horizontal(|ui| {
            p.symbol_view.ui(ui, &doc.doc, p.symbol.0, size, empty(&p.symbol.1, symbol_hint));
            p.footprint_view.ui(ui, &doc.doc, p.footprint.0, size, empty(&p.footprint.1, footprint_hint));
        });
    }

    /// Add a symbol to the selection with its suggested footprint.
    pub fn pick_symbol(&mut self, full: &str) {
        // Picked already: make it the active one.
        self.reveal = Some(Reveal::Pick(full.to_string()));
        if let Some(i) = self.picks.iter().position(|p| p.symbol == full) {
            self.active = Some(i);
            return;
        }
        let catalog = self.catalog();
        match catalog.load_symbol(full) {
            Ok(sym) => {
                let pins = pin_numbers(&sym);
                let mut candidates: Vec<Candidate> = suggested_footprints(catalog, &sym)
                    .into_iter()
                    .map(|name| Candidate { fit: catalog.footprint_fit(&name, &pins), name, checked: true })
                    .collect();
                for (name, fit) in catalog.ranked_footprints(&sym.fp_filters, &pins) {
                    if !candidates.iter().any(|c| c.name == name) {
                        candidates.push(Candidate { name, fit: Some(fit), checked: false });
                    }
                }
                let show_missing = !candidates.iter().any(|c| c.fit.is_none_or(Fit::fits));
                self.picks.push(Pick {
                    symbol: full.to_string(),
                    description: sym.description.clone(),
                    units: sym.units.len(),
                    pins: pins.len(),
                    candidates,
                    show_missing,
                    refused: if sym.power {
                        Some("Power symbol: place it with the Power tool (P); it will not be imported.".into())
                    } else {
                        sym.hidden_power_error().map(|e| format!("Not imported: {e}."))
                    },
                });
                self.active = Some(self.picks.len() - 1);
                self.error = None;
            }
            Err(e) => self.error = Some(e.to_string()),
        }
    }

    pub fn pick_footprint(&mut self, full: &str) {
        self.reveal = Some(Reveal::Lone(full.to_string()));
        if !self.footprints.iter().any(|f| f == full) {
            self.footprints.push(full.to_string());
        }
    }

    fn request(&self) -> Request {
        Request {
            symbols: self
                .picks
                .iter()
                .filter(|p| p.refused.is_none())
                .map(|p| (p.symbol.clone(), p.chosen().cloned().collect()))
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

    /// Clear the selection after an import (the application shows the result).
    pub fn finish(&mut self) {
        self.picks.clear();
        self.footprints.clear();
        self.active = None;
    }

    pub fn ui(&mut self, ctx: &egui::Context, libraries: &[(u64, String)]) -> Option<ImportAction> {
        if !self.open {
            if self.previews.doc.is_some() {
                self.forget_previews();
            }
            return None;
        }
        let mut open = self.open;
        let action =
            theme::window(ctx, "Import KiCad parts", &mut open, [980.0, 780.0], |ui| self.contents(ui, libraries));
        self.open = open;
        action.flatten()
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
                let paths = self.paths();
                egui::ScrollArea::vertical().id_salt("kicad_paths_scroll").max_height(100.0).show(ui, |ui| {
                    for (i, p) in paths.iter().enumerate() {
                        ui.horizontal(|ui| {
                            if i < added && ui.small_button("Remove").clicked() {
                                remove = Some(i);
                            }
                            ui.label(RichText::new(p.display().to_string()).monospace());
                        });
                    }
                });
                if let Some(i) = remove {
                    self.extra.remove(i);
                    self.rescan();
                }
            });
        ui.separator();

        // ---- Search (left) and selection (right): they fill the height left over by the
        // rows above and below, so the window never grows by itself.
        let bottom_height = ui.spacing().interact_size.y + 3.0 * ui.spacing().item_spacing.y + 6.0;
        let height = (ui.available_height() - bottom_height).max(PREVIEW_HEIGHT + 120.0);
        let size = egui::vec2(ui.available_width(), height);
        ui.allocate_ui_with_layout(size, egui::Layout::top_down(egui::Align::Min), |ui| {
            ui.set_min_height(height);
            ui.columns(2, |cols| {
                self.search_ui(&mut cols[0]);
                self.selection_ui(&mut cols[1]);
            });
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

    fn search_ui(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.selectable_value(&mut self.kind, Kind::Symbols, "Symbols");
            ui.selectable_value(&mut self.kind, Kind::Footprints, "Footprints");
        });
        let hint_text = match self.kind {
            Kind::Symbols => "Search symbols, e.g. \"device r\", \"lm358\", \"stm32f103\"",
            Kind::Footprints => "Search footprints, e.g. \"soic-8\", \"0603\"",
        };
        // Arrows and Enter in the field move and add in the results.
        let field_keys = nav_list::field_keys(ui, search_id());
        let field = egui::TextEdit::singleline(&mut self.query).id(search_id());
        crate::fields::search_field(ui, field.hint_text(hint_text));
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
        let kind = self.kind;
        let selected = self.results.iter().position(|r| match (&self.selected, kind) {
            (Some(Selected::SymbolResult { name, .. }), Kind::Symbols) => name == r,
            (Some(Selected::FootprintResult(f)), Kind::Footprints) => f == r,
            _ => false,
        });
        let active = self.active.and_then(|i| self.picks.get(i)).map(|p| p.symbol.clone());
        let results = &self.results;
        // The results on an inset filling the column's height.
        const MARGIN: i8 = 2;
        let height = ui.available_height() - 2.0 * f32::from(MARGIN);
        let out = egui::Frame::NONE
            .fill(theme::panel_dark())
            .corner_radius(4)
            .inner_margin(egui::Margin::same(MARGIN))
            .show(ui, |ui| {
                ui.set_min_height(height);
                nav_list::show(
                    ui,
                    egui::Id::unique("kicad_results"),
                    results.len(),
                    selected,
                    height,
                    false,
                    field_keys,
                    |ui, i| {
                        let r = &results[i];
                        if ui.small_button("Add").clicked() {
                            add = Some(r.clone());
                        }
                        if kind == Kind::Footprints
                            && let Some(sym) = &active
                            && ui
                                .small_button("Use")
                                .on_hover_text(format!("Add as a footprint variant of {sym}"))
                                .clicked()
                        {
                            assign = Some(r.clone());
                        }
                        let (lib, name) = r.split_once(':').unwrap_or(("", r));
                        nav_list::dim(ui, lib);
                        nav_list::name(ui, name);
                    },
                )
            })
            .inner;
        if out.escape {
            ui.memory_mut(|m| m.request_focus(search_id()));
        }
        if let Some(i) = out.select {
            let r = self.results[i].clone();
            self.selected = Some(match kind {
                Kind::Symbols => {
                    let footprint = self.suggested(&r).ok().and_then(|f| f.into_iter().next());
                    Selected::SymbolResult { name: r, footprint }
                }
                Kind::Footprints => Selected::FootprintResult(r),
            });
        }
        if let Some(i) = out.activate {
            add = Some(self.results[i].clone());
        }
        match (add, self.kind) {
            (Some(r), Kind::Symbols) => self.pick_symbol(&r),
            (Some(r), Kind::Footprints) => self.pick_footprint(&r),
            _ => {}
        }
        if let (Some(f), Some(i)) = (assign, self.active)
            && i < self.picks.len()
        {
            let symbol = self.picks[i].symbol.clone();
            let catalog = self.catalog();
            let fit = catalog.load_symbol(&symbol).ok().and_then(|sym| catalog.footprint_fit(&f, &pin_numbers(&sym)));
            self.picks[i].use_footprint(f, fit);
        }
    }

    /// The picks above the preview strip.
    fn selection_ui(&mut self, ui: &mut egui::Ui) {
        let list_height = ui.available_height() - PREVIEW_HEIGHT - CAPTION_HEIGHT - 2.0 * ui.spacing().item_spacing.y;
        ui.allocate_ui(egui::vec2(ui.available_width(), list_height), |ui| {
            ui.set_min_height(list_height);
            self.picks_ui(ui);
        });
        self.preview_ui(ui);
    }

    fn picks_ui(&mut self, ui: &mut egui::Ui) {
        crate::fields::section(ui, "To import", None);
        if self.picks.is_empty() && self.footprints.is_empty() {
            hint(ui, "Add symbols (they become parts, a variant per checked footprint) or lone footprints.");
            return;
        }
        let mut remove = None;
        let mut remove_fp = None;
        let mut select = None;
        let mut escape = false;
        let selected = self.selected.clone();
        let reveal = self.reveal.take();
        // Scrolls so the whole item shows, its top if it is taller than the list.
        let scroll_to = |ui: &mut egui::Ui, rect: egui::Rect| {
            let fits = rect.height() <= ui.clip_rect().height();
            ui.scroll_to_rect(rect, (!fits).then_some(egui::Align::Min));
        };
        egui::ScrollArea::vertical()
            .id_salt("kicad_picks")
            .max_height(ui.available_height())
            .auto_shrink([false, false])
            .show(ui, |ui| {
                for (i, p) in self.picks.iter_mut().enumerate() {
                    let active = self.active == Some(i);
                    let frame = egui::Frame::group(ui.style()).fill(theme::panel()).inner_margin(egui::Margin::same(6));
                    // The card senses clicks before its children are added, so its buttons
                    // and combo stay on top (the widget added last wins).
                    let sense = egui::UiBuilder::new().sense(egui::Sense::click());
                    let r = ui.scope_builder(sense, |ui| {
                        frame.show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.horizontal(|ui| {
                                if ui.small_button("Remove").clicked() {
                                    remove = Some(i);
                                }
                                let units = if p.units > 1 { format!(", {} units", p.units) } else { String::new() };
                                let pins = if p.pins == 1 { "1 pin".to_string() } else { format!("{} pins", p.pins) };
                                ui.label(RichText::new(format!("{pins}{units}")).color(theme::text_dim()));
                                ui.add(egui::Label::new(RichText::new(&p.symbol).strong()).truncate());
                            });
                            if !p.description.is_empty() {
                                ui.label(RichText::new(&p.description).color(theme::text_dim()).small());
                            }
                            if let Some(why) = &p.refused {
                                ui.label(RichText::new(why).color(theme::warn()));
                                return;
                            }
                            let in_pick = match &selected {
                                Some(Selected::Candidate { pick, footprint }) if *pick == p.symbol => {
                                    Some(footprint.as_str())
                                }
                                _ => None,
                            };
                            let (s, esc) = footprints_ui(ui, p, in_pick);
                            escape |= esc;
                            if let Some(f) = s {
                                select = Some(Selected::Candidate { pick: p.symbol.clone(), footprint: f });
                            }
                        })
                    });
                    if reveal.as_ref() == Some(&Reveal::Pick(p.symbol.clone())) {
                        scroll_to(ui, r.response.rect);
                    }
                    if active {
                        // Drawn over the card's own 1 pt border, so the layout doesn't move.
                        let radius = ui.visuals().widgets.noninteractive.corner_radius;
                        let stroke = egui::Stroke::new(2.0, theme::accent());
                        ui.painter().rect_stroke(r.response.rect, radius, stroke, egui::StrokeKind::Inside);
                    }
                    // Any click inside the card (also on its widgets) makes it the active pick.
                    let pressed = ui.input(|i| i.pointer.primary_clicked().then(|| i.pointer.interact_pos()).flatten());
                    if r.response.clicked() || pressed.is_some_and(|p| r.response.rect.contains(p)) {
                        self.active = Some(i);
                        self.selected = None;
                    }
                }
                for (i, f) in self.footprints.iter().enumerate() {
                    let row = ui.horizontal(|ui| {
                        if ui.small_button("Remove").clicked() {
                            remove_fp = Some(i);
                        }
                        ui.label(RichText::new("Footprint").color(theme::text_dim()));
                        let is_selected = selected == Some(Selected::Lone(f.clone()));
                        let b = egui::Button::selectable(is_selected, f.as_str()).truncate();
                        if ui.add(b).on_hover_text("Click to preview").clicked() {
                            select = Some(Selected::Lone(f.clone()));
                        }
                    });
                    if reveal.as_ref() == Some(&Reveal::Lone(f.clone())) {
                        scroll_to(ui, row.response.rect);
                    }
                }
            });
        if let Some(i) = remove {
            self.picks.remove(i);
            // The active pick moves with the list; removing it activates its neighbour.
            let n = self.picks.len();
            self.active = self.active.map(|a| if a > i { a - 1 } else { a }).filter(|&a| a < n);
        }
        // After the cards' own clicks, which select their pick.
        if select.is_some() {
            self.selected = select;
        }
        if escape {
            ui.memory_mut(|m| m.request_focus(search_id()));
        }
        if let Some(i) = remove_fp {
            self.footprints.remove(i);
        }
    }
}

/// Height of a pick's footprint list.
const FOOTPRINT_LIST_HEIGHT: f32 = 132.0;

/// A pick's footprints with a check box each (a variant per checked one), best fitting
/// first. A double click, Enter or Space toggles one. Returns a footprint to select and
/// whether Esc was pressed in the list.
fn footprints_ui(ui: &mut egui::Ui, p: &mut Pick, selected: Option<&str>) -> (Option<String>, bool) {
    let chosen = p.chosen().count();
    let missing = p.candidates.iter().filter(|c| !c.listed(false)).count();
    ui.horizontal(|ui| {
        ui.label("Footprints");
        let text = match chosen {
            0 => "none checked: its components stay off the board".to_string(),
            1 => "1 variant".to_string(),
            n => format!("{n} variants"),
        };
        ui.label(RichText::new(text).color(if chosen == 0 { theme::warn() } else { theme::text_dim() }));
    });
    if p.candidates.is_empty() {
        hint(ui, "No footprint matches; search one under Footprints and press Use.");
        return (None, false);
    }
    let show_missing = p.show_missing;
    let listed: Vec<usize> = (0..p.candidates.len()).filter(|&i| p.candidates[i].listed(show_missing)).collect();
    let current = listed.iter().position(|&i| selected == Some(p.candidates[i].name.as_str()));
    let id = egui::Id::unique(("kicad_fps", &p.symbol));
    let candidates = &mut p.candidates;
    let out = egui::Frame::NONE
        .fill(theme::panel_dark())
        .corner_radius(4)
        .inner_margin(egui::Margin::same(2))
        .show(ui, |ui| {
            nav_list::show(ui, id, listed.len(), current, FOOTPRINT_LIST_HEIGHT, true, Default::default(), |ui, row| {
                let c = &mut candidates[listed[row]];
                ui.checkbox(&mut c.checked, "");
                if let Some(note) = fit_note(c.fit) {
                    nav_list::dim(ui, &note);
                }
                let color = if c.fit.is_none_or(Fit::fits) { ui.visuals().text_color() } else { theme::warn() };
                nav_list::name(ui, RichText::new(&c.name).color(color));
            })
        })
        .inner;
    if let Some(row) = out.activate {
        let c = &mut p.candidates[listed[row]];
        c.checked = !c.checked;
    }
    if missing > 0 && missing < p.candidates.len() {
        ui.checkbox(&mut p.show_missing, format!("Show {missing} footprint(s) missing pins"));
    } else if missing > 0 {
        hint(ui, "No footprint has a pad for every pin.");
    }
    (out.select.map(|row| p.candidates[listed[row]].name.clone()), out.escape)
}
