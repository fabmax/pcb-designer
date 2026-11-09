//! User settings, saved as JSON in the configuration folder:
//! macOS `~/Library/Application Support/pcb-designer`, Windows `%APPDATA%\pcb-designer`,
//! elsewhere `$XDG_CONFIG_HOME/pcb-designer` (or `~/.config/pcb-designer`).
//! `PCB_DESIGNER_CONFIG` overrides the folder (tests, scripted screenshots).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use view::style::Theme;

use crate::keymap::{Action, Keymap, Shortcut};

const RECENT: usize = 10;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub theme: Theme,
    /// Recently opened or saved projects, newest first.
    pub recent: Vec<PathBuf>,
    /// Key bindings that differ from the defaults.
    pub keymap: BTreeMap<Action, Vec<Shortcut>>,
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

impl Settings {
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
        let mut s = Settings { theme: Theme::Light, ..Default::default() };
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
