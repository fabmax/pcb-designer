//! User settings, saved as JSON in the configuration folder:
//! macOS `~/Library/Application Support/pcb-designer`, Windows `%APPDATA%\pcb-designer`,
//! elsewhere `$XDG_CONFIG_HOME/pcb-designer` (or `~/.config/pcb-designer`).
//! `PCB_DESIGNER_CONFIG` overrides the folder (tests, scripted screenshots).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use view::style::{LayerColor, LayerColors, Theme, parse_color_hex};

use crate::keymap::{Action, Keymap, Shortcut};
use crate::units::Unit;

const RECENT: usize = 10;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub theme: Theme,
    /// Display unit of lengths.
    pub unit: Unit,
    /// Recently opened or saved projects, newest first.
    pub recent: Vec<PathBuf>,
    /// Key bindings that differ from the defaults.
    pub keymap: BTreeMap<Action, Vec<Shortcut>>,
    /// Open panels and their sizes.
    pub dock: crate::dock::Dock,
    /// The main window's size and position as last closed.
    pub window: Option<WindowState>,
    pub mcp: McpSettings,
    /// The drawing tools' remembered style choices, by tool family (`schematic`,
    /// `board`, `library`, `array_pads`, `array_pins`; see `view::tool_memory`).
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub tools: BTreeMap<String, serde_json::Value>,
    /// Where KiCad's libraries are downloaded to; `None`: `<config>/kicad`.
    pub kicad_dir: Option<PathBuf>,
    /// The settings window's page and size.
    pub settings_window: crate::settings_ui::WindowMemory,
    /// Size of the user interface (egui's zoom factor; the canvases keep their zoom);
    /// `None`: 100 %.
    pub ui_scale: Option<f32>,
    /// Changed board layer colours per theme, as `#rrggbb[aa]`.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub layer_colors: BTreeMap<Theme, BTreeMap<LayerColor, String>>,
    /// The most library item tabs open at once; `None`: [`LIBRARY_TABS`].
    pub library_tabs: Option<usize>,
    /// The ngspice executable; `None`: found on `PATH` and the usual places.
    pub ngspice: Option<PathBuf>,
    /// The ngspice shared library (libngspice); `None`: found in the usual places.
    pub ngspice_library: Option<PathBuf>,
    /// Every simulation in a separate ngspice process (never the shared library).
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub simulation_process_only: bool,
}

/// The default and the range of [`Settings::library_tabs`].
pub const LIBRARY_TABS: usize = 10;
pub const LIBRARY_TABS_RANGE: std::ops::RangeInclusive<usize> = 1..=50;

/// The UI scales offered.
pub const UI_SCALES: [f32; 8] = [0.8, 0.9, 1.0, 1.1, 1.25, 1.5, 1.75, 2.0];

/// The main window as last closed (points).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct WindowState {
    pub pos: Option<[f32; 2]>,
    pub size: [f32; 2],
    #[serde(default)]
    pub maximized: bool,
    #[serde(default)]
    pub fullscreen: bool,
}

/// The MCP server (agent access).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct McpSettings {
    pub port: u16,
    /// Start the server when the app starts.
    pub start: bool,
}

impl Default for McpSettings {
    fn default() -> Self {
        McpSettings { port: crate::agent::DEFAULT_PORT, start: false }
    }
}

pub fn config_dir() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("PCB_DESIGNER_CONFIG") {
        return Some(dir.into());
    }
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let base = if cfg!(target_os = "macos") {
        home.map(|h| h.join("Library/Application Support"))
    } else if cfg!(windows) {
        std::env::var_os("APPDATA").map(PathBuf::from)
    } else {
        std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).or_else(|| home.map(|h| h.join(".config")))
    };
    base.map(|b| b.join("pcb-designer"))
}

/// Where KiCad's libraries are downloaded to by default (`<config>/kicad/<version>/`).
pub fn default_kicad_dir() -> Option<PathBuf> {
    config_dir().map(|d| d.join("kicad"))
}

impl Settings {
    /// The most library item tabs open at once.
    pub fn max_library_tabs(&self) -> usize {
        self.library_tabs.unwrap_or(LIBRARY_TABS).clamp(*LIBRARY_TABS_RANGE.start(), *LIBRARY_TABS_RANGE.end())
    }

    pub fn path() -> Option<PathBuf> {
        config_dir().map(|d| d.join("settings.json"))
    }

    /// The saved settings (defaults if there are none or they can't be read).
    pub fn load() -> Settings {
        Self::path().map_or_else(Settings::default, |p| Self::load_from(&p))
    }

    pub fn load_from(path: &Path) -> Settings {
        match std::fs::read_to_string(path) {
            Ok(json) => serde_json::from_str(&json).unwrap_or_else(|e| {
                log::warn!("ignoring settings {}: {e}", path.display());
                Settings::default()
            }),
            Err(_) => Settings::default(),
        }
    }

    pub fn save(&self) -> Result<(), String> {
        let path = Self::path().ok_or("no configuration folder")?;
        self.save_to(&path)
    }

    pub fn save_to(&self, path: &Path) -> Result<(), String> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        }
        let json = serde_json::to_string_pretty(self).expect("settings serialize");
        std::fs::write(path, json).map_err(|e| format!("{}: {e}", path.display()))
    }

    /// Remember a project as the most recent one.
    pub fn add_recent(&mut self, path: &Path) {
        let path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
        self.recent.retain(|p| p != &path);
        self.recent.insert(0, path);
        self.recent.truncate(RECENT);
    }

    /// The layer colours (unreadable ones left out).
    pub fn layer_colors(&self) -> LayerColors {
        self.layer_colors
            .iter()
            .map(|(&t, m)| (t, m.iter().filter_map(|(&l, hex)| Some((l, parse_color_hex(hex)?))).collect()))
            .collect()
    }

    /// A UI scale within the offered range.
    pub fn ui_scale(&self) -> f32 {
        self.ui_scale.filter(|s| s.is_finite()).map_or(1.0, |s| s.clamp(UI_SCALES[0], UI_SCALES[7]))
    }

    /// Where KiCad's libraries are downloaded to.
    /// Where ngspice is and how to run it.
    pub fn engine(&self) -> sim::EngineSettings {
        sim::EngineSettings {
            executable: self.ngspice.clone(),
            library: self.ngspice_library.clone(),
            process_only: self.simulation_process_only,
        }
    }

    pub fn kicad_dir(&self) -> Option<PathBuf> {
        self.kicad_dir.clone().or_else(default_kicad_dir)
    }

    /// The default key bindings with the saved changes.
    pub fn keymap(&self) -> Keymap {
        let mut map = Keymap::default();
        for (&action, keys) in &self.keymap {
            map.set(action, keys.clone());
        }
        map
    }

    /// Remember the bindings of `map` that differ from the defaults.
    pub fn set_keymap(&mut self, map: &Keymap) {
        let defaults = Keymap::default();
        self.keymap = Action::ALL
            .iter()
            .filter(|&&a| map.shortcuts(a) != defaults.shortcuts(a))
            .map(|&a| (a, map.shortcuts(a).to_vec()))
            .collect();
    }
}

#[cfg(test)]
mod tests {
    use egui::Key;

    use super::*;

    #[test]
    fn round_trip_and_recent() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sub/settings.json");
        assert_eq!(Settings::load_from(&path), Settings::default());
        let mut s = Settings { theme: Theme::Light, unit: Unit::Mil, ..Default::default() };
        for i in 0..12 {
            s.add_recent(&dir.path().join(format!("p{i}")));
        }
        s.add_recent(&dir.path().join("p5"));
        assert_eq!(s.recent.len(), RECENT);
        assert_eq!(s.recent[0], dir.path().join("p5"));
        let mut map = Keymap::default();
        map.set(Action::PcbZone, vec![Shortcut::shift(Key::Z)]);
        s.set_keymap(&map);
        assert_eq!(s.keymap.len(), 1, "only changes are stored");
        s.save_to(&path).unwrap();
        let loaded = Settings::load_from(&path);
        assert_eq!(loaded, s);
        assert_eq!(loaded.keymap().shortcuts(Action::PcbZone), &[Shortcut::shift(Key::Z)]);
        assert_eq!(loaded.keymap().shortcuts(Action::Undo), Keymap::default().shortcuts(Action::Undo));
        // Unknown or broken files fall back to the defaults.
        std::fs::write(&path, "{ not json").unwrap();
        assert_eq!(Settings::load_from(&path), Settings::default());
    }
}
