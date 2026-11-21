//! The Settings window: categories on the left (filtered by a search field), the chosen
//! category's settings on the right. Changes apply at once; each page has its own
//! "Reset to defaults".
//!
//! Search: every setting row is listed in [`ENTRIES`] with keywords (keymap rows match by
//! action name and shortcut). A search keeps the categories with matches (with their
//! count) and, on the page, only the matching rows with the matched words highlighted.

use egui::text::LayoutJob;
use egui::{Color32, FontId, Key, RichText, TextFormat, Ui};
use serde::{Deserialize, Serialize};
use view::style::{LayerColor, Theme};

use crate::keymap::{Action, Keymap};
use crate::settings::{Settings, UI_SCALES};
use crate::theme;
use crate::units::{self, Unit};

const TITLE: &str = "Settings";

/// The window's smallest size. Wide enough for every page (the widest is Appearance with
/// its UI scale buttons), so switching pages never widens the window by itself.
const MIN_SIZE: [f32; 2] = [900.0, 420.0];

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Page {
    #[default]
    Appearance,
    LayerColors,
    Keymap,
    Tools,
    Agent,
    Libraries,
    Simulation,
}

impl Page {
    pub const ALL: [Page; 7] = [
        Page::Appearance,
        Page::LayerColors,
        Page::Keymap,
        Page::Tools,
        Page::Agent,
        Page::Libraries,
        Page::Simulation,
    ];

    /// By its saved name (`agent`), for `--show settings:agent`.
    pub fn parse(name: &str) -> Option<Page> {
        serde_json::from_value(serde_json::Value::String(name.to_string())).ok()
    }

    pub fn title(self) -> &'static str {
        match self {
            Page::Appearance => "Appearance",
            Page::LayerColors => "Layer colours",
            Page::Keymap => "Keymap",
            Page::Tools => "Tools",
            Page::Agent => "Agent access",
            Page::Libraries => "Libraries",
            Page::Simulation => "Simulation",
        }
    }
}

/// The page shown last and the window's size, kept in the settings.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct WindowMemory {
    pub page: Page,
    pub size: Option<[f32; 2]>,
}

/// A searchable setting row.
struct Entry {
    page: Page,
    label: &'static str,
    keywords: &'static str,
}

const fn entry(page: Page, label: &'static str, keywords: &'static str) -> Entry {
    Entry { page, label, keywords }
}

/// Keywords of every layer colour row.
const COLOR_KEYWORDS: &str = "colour color layer board pcb";

const ENTRIES: &[Entry] = &[
    entry(Page::Appearance, "Theme", "dark light mode colour color"),
    entry(Page::Appearance, "Length units", "mm mil inch metric imperial unit"),
    entry(Page::Appearance, "UI scale", "size zoom font text bigger smaller interface percent"),
    entry(Page::LayerColors, "Theme", "dark light mode"),
    entry(Page::Tools, "Remembered tool settings", "corner width size layer forget reset drawing style"),
    entry(Page::Agent, "Server", "mcp status start stop running url"),
    entry(Page::Agent, "Port", "mcp server network tcp"),
    entry(Page::Agent, "Start on launch", "mcp startup automatic"),
    entry(Page::Agent, "Connect a client", "claude code json config copy command url"),
    entry(Page::Agent, "Headless mode", "stdio claude code command copy window"),
    entry(Page::Libraries, "Most library tabs", "tabs preview limit maximum open items parts symbols footprints"),
    entry(Page::Libraries, "KiCad library folder", "download path directory kicad"),
    entry(Page::Simulation, "Run simulations", "spice ngspice shared library process backend crash in use version"),
    entry(Page::Simulation, "ngspice", "spice simulator path executable install program version"),
    entry(Page::Simulation, "ngspice library", "spice simulator path libngspice dll dylib shared install"),
];

/// The search words (lower case; every one has to match).
#[derive(Default)]
pub struct Filter {
    words: Vec<String>,
}

impl Filter {
    pub fn new(query: &str) -> Filter {
        Filter { words: query.split_whitespace().map(str::to_lowercase).collect() }
    }

    pub fn is_empty(&self) -> bool {
        self.words.is_empty()
    }

    /// Every word is in the label or the keywords.
    pub fn matches(&self, label: &str, keywords: &str) -> bool {
        let text = format!("{label} {keywords}").to_lowercase();
        self.words.iter().all(|w| text.contains(w.as_str()))
    }

    fn entry(&self, page: Page, label: &str) -> bool {
        let keywords = ENTRIES.iter().find(|e| e.page == page && e.label == label).map_or("", |e| e.keywords);
        self.matches(label, keywords)
    }

    /// Matching rows on `page`. Keymap rows are an action's name and its shortcuts.
    pub fn count(&self, page: Page, keymap_rows: &[(String, String)]) -> usize {
        let rows = ENTRIES.iter().filter(|e| e.page == page && self.matches(e.label, e.keywords)).count();
        match page {
            Page::Keymap => rows + keymap_rows.iter().filter(|(l, k)| self.matches(l, k)).count(),
            Page::LayerColors => {
                rows + LayerColor::ALL.iter().filter(|c| self.matches(c.label(), COLOR_KEYWORDS)).count()
            }
            _ => rows,
        }
    }

    /// `text` with the matched words highlighted.
    fn highlight(&self, text: &str, color: Color32) -> LayoutJob {
        let lower = text.to_lowercase();
        // Lower-casing can change byte lengths (rare outside ASCII): highlight nothing.
        let marks: Vec<(usize, usize)> = if lower.len() == text.len() {
            let mut m: Vec<(usize, usize)> = self
                .words
                .iter()
                .flat_map(|w| lower.match_indices(w.as_str()).map(|(i, s)| (i, i + s.len())).collect::<Vec<_>>())
                .collect();
            m.sort();
            m
        } else {
            Vec::new()
        };
        let font = FontId::proportional(14.0);
        let plain = TextFormat { font_id: font.clone(), color, ..Default::default() };
        let marked = TextFormat {
            font_id: font,
            color: theme::accent(),
            background: theme::accent().gamma_multiply(0.18),
            ..Default::default()
        };
        let mut job = LayoutJob::default();
        let mut at = 0;
        for (a, b) in marks {
            let a = a.max(at);
            if a >= b {
                continue;
            }
            job.append(&text[at..a], 0.0, plain.clone());
            job.append(&text[a..b], 0.0, marked.clone());
            at = b;
        }
        job.append(&text[at..], 0.0, plain);
        job
    }
}

/// What a frame of the window asks the app to do.
#[derive(Default)]
pub struct Changes {
    /// Settings changed: save them.
    pub save: bool,
    pub theme: Option<Theme>,
    pub ui_scale: bool,
    pub layer_colors: bool,
    /// The key bindings changed (`Settings::keymap` is up to date).
    pub keymap: bool,
    pub agent: Option<AgentCmd>,
    /// Forget the remembered tool settings (and reset the tools).
    pub forget_tools: bool,
    /// The KiCad download folder changed.
    pub kicad_dir: bool,
    /// The most library item tabs changed.
    pub library_tabs: bool,
    /// The ngspice path changed: look for it again.
    pub ngspice: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AgentCmd {
    Start,
    Stop,
    /// The port changed while running.
    Restart,
}

/// The agent server's state, for the Agent access page.
pub struct AgentStatus {
    pub url: Option<String>,
    pub error: Option<String>,
    pub headless_command: String,
}

#[derive(Default)]
pub struct SettingsWindow {
    pub open: bool,
    query: String,
    /// Focus the search field next frame (just opened, ⌘F).
    focus_search: bool,
    /// The action whose new shortcut is being recorded.
    pub recording: Option<Action>,
    /// The port being typed (applied when valid).
    port_text: Option<String>,
    /// The KiCad folder being typed.
    kicad_text: Option<String>,
    /// The ngspice path being typed.
    ngspice_text: Option<String>,
    /// The ngspice library path being typed.
    ngspice_lib_text: Option<String>,
    /// What was found for the simulation settings (the settings it was for, the text).
    ngspice_found: Option<(sim::EngineSettings, Result<String, String>)>,
}

fn search_id() -> egui::Id {
    egui::Id::unique("settings_search")
}

impl SettingsWindow {
    pub fn show(&mut self, page: Option<Page>, settings: &mut Settings) {
        self.open = true;
        self.focus_search = true;
        if let Some(p) = page {
            settings.settings_window.page = p;
        }
    }

    pub fn ui(
        &mut self,
        ctx: &egui::Context,
        settings: &mut Settings,
        keymap: &Keymap,
        agent: &AgentStatus,
    ) -> Changes {
        let mut changes = Changes::default();
        let mut open = self.open;
        let size = settings.settings_window.size.unwrap_or([MIN_SIZE[0], 600.0]);
        let keymap_rows: Vec<(String, String)> = Action::ALL
            .iter()
            .map(|&a| {
                let keys: Vec<String> = keymap.shortcuts(a).iter().map(|k| ctx.format_shortcut(&k.to_egui())).collect();
                (a.label().to_string(), keys.join(" "))
            })
            .collect();
        theme::window_sized(ctx, TITLE, &mut open, size, MIN_SIZE, |ui| {
            if ui.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, Key::F)) {
                self.focus_search = true;
            }
            let filter = Filter::new(&self.query);
            let height = ui.available_height().max(360.0);
            ui.horizontal_top(|ui| {
                ui.vertical(|ui| {
                    ui.set_width(190.0);
                    let field = egui::TextEdit::singleline(&mut self.query)
                        .id(search_id())
                        .hint_text("Search settings")
                        .desired_width(f32::INFINITY);
                    let r = ui.add(field);
                    if std::mem::take(&mut self.focus_search) {
                        r.request_focus();
                    }
                    ui.add_space(8.0);
                    let mut first = None;
                    for page in Page::ALL {
                        let count = filter.count(page, &keymap_rows);
                        if !filter.is_empty() && count == 0 {
                            continue;
                        }
                        first.get_or_insert(page);
                        let current = settings.settings_window.page == page;
                        let text = if filter.is_empty() {
                            RichText::new(page.title())
                        } else {
                            RichText::new(format!("{}  ({count})", page.title()))
                        };
                        let item = egui::Button::selectable(current, text.size(14.0))
                            .min_size(egui::vec2(ui.available_width(), 26.0));
                        if ui.add(item).clicked() {
                            settings.settings_window.page = page;
                        }
                    }
                    // The page shown has no matches: show the first that has.
                    if !filter.is_empty()
                        && filter.count(settings.settings_window.page, &keymap_rows) == 0
                        && let Some(p) = first
                    {
                        settings.settings_window.page = p;
                    }
                    if first.is_none() {
                        ui.label(RichText::new("No settings match").color(theme::text_dim()));
                    }
                });
                ui.separator();
                ui.vertical(|ui| {
                    let page = settings.settings_window.page;
                    theme::bold_label(ui, page.title(), 16.0, ui.visuals().text_color());
                    ui.add_space(10.0);
                    egui::ScrollArea::vertical().max_height(height - 30.0).auto_shrink([false, false]).show(ui, |ui| {
                        let mut rows = Rows { filter: &filter, page };
                        match page {
                            Page::Appearance => self.appearance(ui, &mut rows, settings, &mut changes),
                            Page::LayerColors => self.layer_colors(ui, &mut rows, settings, &mut changes),
                            Page::Keymap => self.keymap(ui, &filter, settings, keymap, &mut changes),
                            Page::Tools => tools(ui, &mut rows, settings, &mut changes),
                            Page::Agent => self.agent(ui, &mut rows, settings, agent, &mut changes),
                            Page::Libraries => self.libraries(ui, &mut rows, settings, &mut changes),
                            Page::Simulation => self.simulation(ui, &mut rows, settings, &mut changes),
                        }
                    });
                });
            });
        });
        if !open {
            self.recording = None;
        }
        self.open = open;
        if let Some(rect) = ctx.memory(|m| m.area_rect(theme::window_id(TITLE))) {
            let s = [rect.width().round(), rect.height().round()];
            if settings.settings_window.size != Some(s) && !ctx.input(|i| i.pointer.any_down()) {
                settings.settings_window.size = Some(s);
                changes.save = true;
            }
        }
        changes
    }

    fn appearance(&mut self, ui: &mut Ui, rows: &mut Rows, settings: &mut Settings, changes: &mut Changes) {
        rows.row(ui, "Theme", None, |ui| {
            let current = view::style::theme();
            for &t in Theme::ALL {
                if ui.selectable_label(current == t, t.label()).clicked() && current != t {
                    changes.theme = Some(t);
                }
            }
        });
        rows.row(ui, "UI scale", Some("Panels, menus and tool bars; the canvases keep their zoom."), |ui| {
            let current = settings.ui_scale();
            for s in UI_SCALES {
                let text = format!("{}%", (s * 100.0).round());
                if ui.selectable_label((current - s).abs() < 0.001, text).clicked() && (current - s).abs() >= 0.001 {
                    settings.ui_scale = (s != 1.0).then_some(s);
                    changes.ui_scale = true;
                    changes.save = true;
                }
            }
        });
        rows.row(ui, "Length units", Some("Lengths shown and typed in the panels and the status bar."), |ui| {
            let mut unit = units::unit();
            for u in Unit::ALL {
                ui.selectable_value(&mut unit, u, u.label());
            }
            if unit != units::unit() {
                units::set_unit(unit);
                settings.unit = unit;
                changes.save = true;
            }
        });
        reset(ui, rows, || {
            changes.theme = Some(Theme::default());
            units::set_unit(Unit::default());
            settings.unit = Unit::default();
            settings.ui_scale = None;
            changes.ui_scale = true;
            changes.save = true;
        });
    }

    fn layer_colors(&mut self, ui: &mut Ui, rows: &mut Rows, settings: &mut Settings, changes: &mut Changes) {
        let current = view::style::theme();
        rows.row(ui, "Theme", Some("Each theme has its own layer colours; these are the shown theme's."), |ui| {
            for &t in Theme::ALL {
                if ui.selectable_label(current == t, t.label()).clicked() && current != t {
                    changes.theme = Some(t);
                }
            }
        });
        let palette = view::style::pal();
        for c in LayerColor::ALL {
            let default = c.default_in(palette);
            let mut color = view::style::layer_color(c);
            let changed = color != default;
            rows.row_with(ui, c.label(), COLOR_KEYWORDS, None, |ui| {
                crate::color_edit::color_edit(ui, &mut color.0);
                if changed && ui.small_button("Reset").on_hover_text("Back to the theme's colour").clicked() {
                    color = default;
                }
            });
            if color != view::style::layer_color(c) {
                let map = settings.layer_colors.entry(current).or_default();
                if color == default {
                    map.remove(&c);
                } else {
                    map.insert(c, view::style::color_hex(color));
                }
                if map.is_empty() {
                    settings.layer_colors.remove(&current);
                }
                changes.layer_colors = true;
                changes.save = true;
            }
        }
        reset(ui, rows, || {
            settings.layer_colors.remove(&current);
            changes.layer_colors = true;
            changes.save = true;
        });
    }

    fn keymap(
        &mut self,
        ui: &mut Ui,
        filter: &Filter,
        settings: &mut Settings,
        keymap: &Keymap,
        changes: &mut Changes,
    ) {
        let mut map = keymap.clone();
        let conflicts = map.conflicts();
        if !conflicts.is_empty() {
            let text: Vec<String> = conflicts
                .iter()
                .map(|(s, a)| {
                    let names: Vec<&str> = a.iter().map(|a| a.label()).collect();
                    format!("{} is bound to {}", ui.ctx().format_shortcut(&s.to_egui()), names.join(" and "))
                })
                .collect();
            ui.label(RichText::new(text.join("\n")).color(theme::warn()));
            ui.add_space(6.0);
        }
        let mut changed = false;
        egui::Grid::new("keymap").num_columns(2).striped(true).spacing([24.0, 4.0]).show(ui, |ui| {
            for &action in Action::ALL {
                let keys = map.shortcuts(action).to_vec();
                let key_text: Vec<String> = keys.iter().map(|k| ui.ctx().format_shortcut(&k.to_egui())).collect();
                if !filter.matches(action.label(), &key_text.join(" ")) {
                    continue;
                }
                ui.label(filter.highlight(action.label(), ui.visuals().text_color()));
                ui.horizontal(|ui| {
                    for (i, text) in key_text.iter().enumerate() {
                        let b = egui::Button::new(filter.highlight(&format!("{text}  ×"), ui.visuals().text_color()))
                            .small();
                        if ui.add(b).on_hover_text("Remove").clicked() {
                            let mut keys = keys.clone();
                            keys.remove(i);
                            map.set(action, keys);
                            changed = true;
                        }
                    }
                    if self.recording == Some(action) {
                        ui.label(RichText::new("Press a key… (Esc cancels)").color(theme::accent()));
                    } else if ui.small_button("+").on_hover_text("Add a shortcut").clicked() {
                        self.recording = Some(action);
                    }
                });
                ui.end_row();
            }
        });
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            if filter.is_empty() && ui.button("Reset to defaults").clicked() {
                map = Keymap::default();
                changed = true;
            }
            if ui.button("Copy as JSON").clicked() {
                ui.ctx().copy_text(map.to_json());
            }
        });
        if changed {
            settings.set_keymap(&map);
            changes.keymap = true;
            changes.save = true;
        }
    }

    fn agent(
        &mut self,
        ui: &mut Ui,
        rows: &mut Rows,
        settings: &mut Settings,
        agent: &AgentStatus,
        changes: &mut Changes,
    ) {
        rows.row(ui, "Server", Some("Lets AI agents (MCP clients) read and edit the open project."), |ui| match &agent
            .url
        {
            Some(url) => {
                ui.label(RichText::new(format!("Running at {url}")).color(theme::success()));
                if ui.button("Stop").clicked() {
                    changes.agent = Some(AgentCmd::Stop);
                }
            }
            None => {
                ui.label(RichText::new("Stopped").color(theme::text_dim()));
                if ui.button("Start").clicked() {
                    changes.agent = Some(AgentCmd::Start);
                }
            }
        });
        if let Some(e) = &agent.error {
            ui.label(RichText::new(e).color(theme::warn()));
        }
        rows.row(ui, "Port", Some("Listens on 127.0.0.1 only. --mcp-port overrides it for one run."), |ui| {
            let text = self.port_text.get_or_insert_with(|| settings.mcp.port.to_string());
            let r = ui.add(egui::TextEdit::singleline(text).desired_width(70.0));
            let valid = text.trim().parse::<u16>().ok().filter(|&p| p > 0);
            if valid.is_none() {
                ui.label(RichText::new("1 – 65535").color(theme::warn()));
            }
            if (r.lost_focus() || !r.has_focus())
                && let Some(port) = valid
                && port != settings.mcp.port
            {
                settings.mcp.port = port;
                changes.save = true;
                if agent.url.is_some() {
                    changes.agent = Some(AgentCmd::Restart);
                }
            }
            if !r.has_focus() {
                self.port_text = None;
            }
        });
        rows.row(ui, "Start on launch", None, |ui| {
            if ui.checkbox(&mut settings.mcp.start, "Start the MCP server when the app starts").changed() {
                changes.save = true;
            }
        });
        rows.row(ui, "Connect a client", Some("Copy what an MCP client needs (the server has to run)."), |ui| {
            ui.add_enabled_ui(agent.url.is_some(), |ui| {
                let url = agent.url.clone().unwrap_or_default();
                if ui.button("Copy URL").clicked() {
                    ui.ctx().copy_text(url.clone());
                }
                if ui.button("Copy Claude Code command").clicked() {
                    ui.ctx().copy_text(format!("claude mcp add --transport http pcb-designer {url}"));
                }
                if ui.button("Copy JSON config").clicked() {
                    ui.ctx().copy_text(format!(
                        "{{\n  \"mcpServers\": {{\n    \"pcb-designer\": {{ \"type\": \"http\", \"url\": \"{url}\" }}\n  }}\n}}"
                    ));
                }
            });
        });
        rows.row(ui, "Headless mode", Some("No window: the agent runs the app itself, MCP on stdio."), |ui| {
            if ui.button("Copy Claude Code command").clicked() {
                ui.ctx().copy_text(agent.headless_command.clone());
            }
        });
        reset(ui, rows, || {
            settings.mcp = Default::default();
            self.port_text = None;
            changes.save = true;
        });
    }

    fn libraries(&mut self, ui: &mut Ui, rows: &mut Rows, settings: &mut Settings, changes: &mut Changes) {
        let help = "Parts, symbols and footprints open in tabs of their own; past this many, the tabs used \
                    longest ago close (nothing is lost).";
        rows.row(ui, "Most library tabs", Some(help), |ui| {
            let mut n = settings.max_library_tabs();
            if ui.add(egui::DragValue::new(&mut n).range(crate::settings::LIBRARY_TABS_RANGE)).changed() {
                settings.library_tabs = (n != crate::settings::LIBRARY_TABS).then_some(n);
                changes.library_tabs = true;
                changes.save = true;
            }
        });
        let shown = settings.kicad_dir().map(|p| p.display().to_string()).unwrap_or_default();
        rows.row(ui, "KiCad library folder", Some("Where File → Download KiCad libraries puts them."), |ui| {
            let text = self.kicad_text.get_or_insert_with(|| shown.clone());
            let r = ui.add(egui::TextEdit::singleline(text).desired_width(320.0));
            if r.lost_focus() && *text != shown {
                settings.kicad_dir = (!text.trim().is_empty()).then(|| text.trim().into());
                changes.kicad_dir = true;
                changes.save = true;
            }
            if !r.has_focus() {
                self.kicad_text = None;
            }
            if ui.button("Choose…").clicked()
                && let Some(dir) = rfd::FileDialog::new().set_title("KiCad library folder").pick_folder()
            {
                settings.kicad_dir = Some(dir);
                self.kicad_text = None;
                changes.kicad_dir = true;
                changes.save = true;
            }
        });
        reset(ui, rows, || {
            settings.kicad_dir = None;
            self.kicad_text = None;
            settings.library_tabs = None;
            changes.kicad_dir = true;
            changes.library_tabs = true;
            changes.save = true;
        });
    }
}

impl SettingsWindow {
    fn simulation(&mut self, ui: &mut Ui, rows: &mut Rows, settings: &mut Settings, changes: &mut Changes) {
        let help = "The shared library starts runs at once (the tuner reacts quicker) and draws traces while \
                    they run. A model that crashes ngspice takes the app down with it, though: a separate \
                    process is safe. Parameter sweeps always run in processes, several at once.";
        rows.row(ui, "Run simulations", Some(help), |ui| {
            let mut process_only = settings.simulation_process_only;
            ui.selectable_value(&mut process_only, false, "Shared library when found");
            ui.selectable_value(&mut process_only, true, "Separate process only");
            if process_only != settings.simulation_process_only {
                settings.simulation_process_only = process_only;
                changes.ngspice = true;
                changes.save = true;
            }
        });
        let help = "Empty: found on PATH and in the usual install folders (Homebrew, /usr/bin, Program Files).";
        if let Some(path) = path_row(ui, rows, "ngspice", help, &settings.ngspice, &mut self.ngspice_text) {
            settings.ngspice = path;
            changes.ngspice = true;
            changes.save = true;
        }
        if !settings.simulation_process_only {
            let help = "libngspice (ngspice.dll on Windows). Empty: the usual install folders (Homebrew, \
                        /usr/lib, Program Files\\Spice64, KiCad's).";
            let (label, text) = ("ngspice library", &mut self.ngspice_lib_text);
            if let Some(path) = path_row(ui, rows, label, help, &settings.ngspice_library, text) {
                settings.ngspice_library = path;
                changes.ngspice = true;
                changes.save = true;
            }
        }
        let engine = settings.engine();
        if self.ngspice_found.as_ref().is_none_or(|(e, _)| *e != engine) {
            use sim::Backend;
            let found = sim::Engine::find(&engine);
            let text = match (&found.error, &found.library, &found.process) {
                (Some(e), ..) => Err(format!("{e}.")),
                (None, Some(l), p) => {
                    let mut t = format!("In use: {} · {}", l.describe(), l.path.display());
                    if let Some(p) = p {
                        t.push_str(&format!("\nSweeps: {} · {}", p.describe(), p.path.display()));
                    }
                    Ok(t)
                }
                (None, None, Some(p)) => Ok(format!("In use: {} · {}", p.describe(), p.path.display())),
                (None, None, None) => Err("ngspice is not installed.".to_string()),
            };
            let text = text.map_err(|e| {
                format!(
                    "{e} Install it (macOS: brew install ngspice; Linux: your package manager; Windows: \
                     ngspice.sourceforge.io) or choose where it is."
                )
            });
            self.ngspice_found = Some((engine, text));
        }
        if let Some((_, found)) = &self.ngspice_found
            && rows.filter.entry(Page::Simulation, "Run simulations")
        {
            ui.horizontal(|ui| {
                ui.add_space(LABEL_WIDTH + ui.spacing().item_spacing.x);
                match found {
                    Ok(t) => ui.add(egui::Label::new(RichText::new(t).color(theme::success())).wrap()),
                    Err(e) => ui.add(egui::Label::new(RichText::new(e).color(theme::warn())).wrap()),
                };
            });
        }
        reset(ui, rows, || {
            settings.ngspice = None;
            settings.ngspice_library = None;
            settings.simulation_process_only = false;
            self.ngspice_text = None;
            self.ngspice_lib_text = None;
            changes.ngspice = true;
            changes.save = true;
        });
    }
}

/// A file path row: typed (applied when the field loses focus) or chosen. Returns the
/// new path when it changed (`None` inside: empty, found automatically).
fn path_row(
    ui: &mut Ui,
    rows: &mut Rows,
    label: &str,
    help: &str,
    path: &Option<std::path::PathBuf>,
    typing: &mut Option<String>,
) -> Option<Option<std::path::PathBuf>> {
    let shown = path.as_ref().map(|p| p.display().to_string()).unwrap_or_default();
    let mut out = None;
    rows.row(ui, label, Some(help), |ui| {
        let text = typing.get_or_insert_with(|| shown.clone());
        let r = ui.add(egui::TextEdit::singleline(text).hint_text("automatic").desired_width(320.0));
        if r.lost_focus() && *text != shown {
            out = Some((!text.trim().is_empty()).then(|| text.trim().into()));
        }
        if !r.has_focus() {
            *typing = None;
        }
        if ui.button("Choose…").clicked()
            && let Some(file) = rfd::FileDialog::new().set_title(label).pick_file()
        {
            *typing = None;
            out = Some(Some(file));
        }
    });
    out
}

fn tools(ui: &mut Ui, rows: &mut Rows, settings: &mut Settings, changes: &mut Changes) {
    let help = "Corner styles, widths, sizes, layers and other choices in Tool settings are remembered \
                across sessions; rotation, the next name and the routing layer are not.";
    rows.row(ui, "Remembered tool settings", Some(help), |ui| {
        let families: Vec<&str> = settings.tools.keys().map(|k| k.as_str()).collect();
        let text = if families.is_empty() { "Nothing remembered yet".into() } else { families.join(", ") };
        ui.add(egui::Label::new(RichText::new(text).color(theme::text_dim())).wrap());
    });
    reset(ui, rows, || {
        changes.forget_tools = true;
        changes.save = true;
    });
}

/// The rows of a page, shown when they match the search.
struct Rows<'a> {
    filter: &'a Filter,
    page: Page,
}

const LABEL_WIDTH: f32 = 170.0;

impl Rows<'_> {
    /// A setting: its label, its widgets and an optional help text below.
    fn row(&mut self, ui: &mut Ui, label: &str, help: Option<&str>, add: impl FnOnce(&mut Ui)) {
        if self.filter.entry(self.page, label) {
            self.draw(ui, label, help, add);
        }
    }

    /// A row not in [`ENTRIES`], with its keywords.
    fn row_with(&mut self, ui: &mut Ui, label: &str, keywords: &str, help: Option<&str>, add: impl FnOnce(&mut Ui)) {
        if self.filter.matches(label, keywords) {
            self.draw(ui, label, help, add);
        }
    }

    fn draw(&mut self, ui: &mut Ui, label: &str, help: Option<&str>, add: impl FnOnce(&mut Ui)) {
        ui.horizontal(|ui| {
            let (rect, _) = ui.allocate_exact_size(egui::vec2(LABEL_WIDTH, 22.0), egui::Sense::hover());
            let galley = ui.painter().layout_job(self.filter.highlight(label, ui.visuals().text_color()));
            ui.painter().galley(rect.left_center() - egui::vec2(0.0, galley.size().y / 2.0), galley, Color32::WHITE);
            add(ui);
        });
        if let Some(help) = help {
            ui.horizontal(|ui| {
                ui.add_space(LABEL_WIDTH + ui.spacing().item_spacing.x);
                ui.add(egui::Label::new(RichText::new(help).color(theme::text_dim()).size(12.5)).wrap());
            });
        }
        ui.add_space(10.0);
    }
}

/// A page's "Reset to defaults" (hidden while searching).
fn reset(ui: &mut Ui, rows: &Rows, apply: impl FnOnce()) {
    if !rows.filter.is_empty() {
        return;
    }
    ui.add_space(6.0);
    if ui.button("Reset to defaults").clicked() {
        apply();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_matches_labels_and_keywords() {
        let f = Filter::new("MCP  port");
        assert!(f.entry(Page::Agent, "Port"));
        assert!(!f.entry(Page::Agent, "Server"), "every word has to match");
        assert_eq!(f.count(Page::Agent, &[]), 1);
        assert_eq!(f.count(Page::Appearance, &[]), 0);

        let keys = vec![("Undo".to_string(), "⌘Z".to_string()), ("Zone tool".to_string(), "⇧Z".to_string())];
        assert_eq!(Filter::new("z").count(Page::Keymap, &keys), 2, "by action name or shortcut");
        assert_eq!(Filter::new("undo").count(Page::Keymap, &keys), 1);
        assert!(Filter::new("").is_empty() && Filter::new("").matches("anything", ""));
    }

    #[test]
    fn every_entry_page_has_a_label_once() {
        for (i, a) in ENTRIES.iter().enumerate() {
            assert!(ENTRIES[i + 1..].iter().all(|b| a.page != b.page || a.label != b.label), "{}", a.label);
        }
    }

    #[test]
    fn highlight_marks_matches() {
        let job = Filter::new("unit").highlight("Length units", Color32::WHITE);
        let marked: Vec<&str> = job
            .sections
            .iter()
            .filter(|s| s.format.color != Color32::WHITE)
            .map(|s| &job.text[s.byte_range.start.0..s.byte_range.end.0])
            .collect();
        assert_eq!(marked, ["unit"]);
    }
}
