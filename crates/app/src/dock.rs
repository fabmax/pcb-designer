//! Tool windows, IntelliJ style: a narrow bar of icon buttons on the left and right
//! window edges, one button per panel. A side shows at most one panel; clicking its
//! button again hides it. Panels never open or switch on their own.
//!
//! [`Dock`] is the saved state (which panel each side shows, and the sizes). [`Unseen`]
//! counts problems that appeared while the Messages panel was hidden (its badge).

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use crate::icons::Icon;
use crate::keymap::Action;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Panel {
    Libraries,
    LibraryBrowser,
    Messages,
    Properties,
    ToolSettings,
    BoardSetup,
    Overview,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    Left,
    Right,
    Bottom,
}

impl Panel {
    /// In bar order: left bar (top group, then the bottom panels), then the right bar.
    pub const ALL: [Panel; 7] = [
        Panel::Libraries,
        Panel::LibraryBrowser,
        Panel::Messages,
        Panel::Properties,
        Panel::ToolSettings,
        Panel::BoardSetup,
        Panel::Overview,
    ];

    pub fn side(self) -> Side {
        match self {
            Panel::Libraries | Panel::LibraryBrowser => Side::Left,
            Panel::Messages => Side::Bottom,
            Panel::Properties | Panel::ToolSettings | Panel::BoardSetup | Panel::Overview => Side::Right,
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            Panel::Libraries => "Libraries",
            Panel::LibraryBrowser => "Library browser",
            Panel::Messages => "Messages",
            Panel::Properties => "Properties",
            Panel::ToolSettings => "Tool settings",
            Panel::BoardSetup => "Board setup",
            Panel::Overview => "Overview",
        }
    }

    pub fn icon(self) -> Icon {
        match self {
            Panel::Libraries => Icon::Tree,
            Panel::LibraryBrowser => Icon::Library,
            Panel::Messages => Icon::Messages,
            Panel::Properties => Icon::Properties,
            Panel::ToolSettings => Icon::Wrench,
            Panel::BoardSetup => Icon::BoardSetup,
            Panel::Overview => Icon::Info,
        }
    }

    /// The keyboard action showing or hiding it.
    pub fn action(self) -> Action {
        match self {
            Panel::Libraries => Action::PanelLibraries,
            Panel::LibraryBrowser => Action::PanelLibraryBrowser,
            Panel::Messages => Action::PanelMessages,
            Panel::Properties => Action::PanelProperties,
            Panel::ToolSettings => Action::PanelToolSettings,
            Panel::BoardSetup => Action::PanelBoardSetup,
            Panel::Overview => Action::PanelOverview,
        }
    }

    pub fn of_action(action: Action) -> Option<Panel> {
        Panel::ALL.into_iter().find(|p| p.action() == action)
    }

    /// By its saved name (`library_browser`), for `--panel`.
    pub fn parse(name: &str) -> Option<Panel> {
        serde_json::from_value(serde_json::Value::String(name.replace('-', "_"))).ok()
    }

    pub fn on(side: Side) -> impl Iterator<Item = Panel> {
        Panel::ALL.into_iter().filter(move |p| p.side() == side)
    }
}

/// Which panel each side shows, and the panel sizes (points). Saved in the settings.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Dock {
    pub left: Option<Panel>,
    pub right: Option<Panel>,
    pub bottom: Option<Panel>,
    pub left_width: f32,
    pub right_width: f32,
    pub bottom_height: f32,
}

impl Default for Dock {
    fn default() -> Self {
        Dock {
            left: Some(Panel::Libraries),
            right: Some(Panel::Properties),
            bottom: None,
            left_width: 240.0,
            right_width: 300.0,
            bottom_height: 200.0,
        }
    }
}

impl Dock {
    pub fn open(&self, side: Side) -> Option<Panel> {
        match side {
            Side::Left => self.left,
            Side::Right => self.right,
            Side::Bottom => self.bottom,
        }
    }

    fn slot(&mut self, side: Side) -> &mut Option<Panel> {
        match side {
            Side::Left => &mut self.left,
            Side::Right => &mut self.right,
            Side::Bottom => &mut self.bottom,
        }
    }

    pub fn is_open(&self, panel: Panel) -> bool {
        self.open(panel.side()) == Some(panel)
    }

    /// Show `panel` (replacing its side's panel), or hide it if it is showing.
    pub fn toggle(&mut self, panel: Panel) {
        let slot = self.slot(panel.side());
        *slot = if *slot == Some(panel) { None } else { Some(panel) };
    }

    pub fn show(&mut self, panel: Panel) {
        *self.slot(panel.side()) = Some(panel);
    }

    pub fn hide(&mut self, side: Side) {
        *self.slot(side) = None;
    }

    /// Size of a side's panel (width, or height for the bottom).
    pub fn size(&self, side: Side) -> f32 {
        match side {
            Side::Left => self.left_width,
            Side::Right => self.right_width,
            Side::Bottom => self.bottom_height,
        }
    }

    pub fn set_size(&mut self, side: Side, size: f32) {
        let size = size.round();
        match side {
            Side::Left => self.left_width = size,
            Side::Right => self.right_width = size,
            Side::Bottom => self.bottom_height = size,
        }
    }
}

/// Problems that appeared since the Messages panel was last seen. Problems are
/// identified by a key (a hash of their section and text).
#[derive(Debug, Default)]
pub struct Unseen {
    seen: HashSet<u64>,
    /// Current problems not seen yet.
    pub new: usize,
    /// All current problems.
    pub total: usize,
}

impl Unseen {
    /// Update with the current problems; while `visible` they all count as seen. A
    /// problem that goes away and comes back is new again.
    pub fn update(&mut self, keys: impl IntoIterator<Item = u64>, visible: bool) {
        let current: HashSet<u64> = keys.into_iter().collect();
        if visible {
            self.seen = current.clone();
        } else {
            self.seen.retain(|k| current.contains(k));
        }
        self.new = current.difference(&self.seen).count();
        self.total = current.len();
    }
}

/// Key of a problem for [`Unseen`].
pub fn problem_key(section: &str, text: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    (section, text).hash(&mut h);
    h.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_panel_per_side() {
        let mut d = Dock::default();
        assert_eq!((d.left, d.right, d.bottom), (Some(Panel::Libraries), Some(Panel::Properties), None));
        d.toggle(Panel::ToolSettings);
        assert_eq!(d.right, Some(Panel::ToolSettings), "switches the side's panel");
        assert_eq!(d.left, Some(Panel::Libraries), "other sides stay");
        d.toggle(Panel::ToolSettings);
        assert_eq!(d.right, None, "clicking the open panel hides it");
        d.toggle(Panel::Messages);
        assert!(d.is_open(Panel::Messages));
        assert_eq!(Panel::parse("library-browser"), Some(Panel::LibraryBrowser));
        assert_eq!(Panel::parse("board_setup"), Some(Panel::BoardSetup));
        assert_eq!(Panel::parse("nets"), None);
        for p in Panel::ALL {
            assert_eq!(Panel::of_action(p.action()), Some(p));
        }
    }

    #[test]
    fn dock_saves_and_loads() {
        let mut d = Dock::default();
        d.toggle(Panel::Messages);
        d.set_size(Side::Left, 310.4);
        let json = serde_json::to_string(&d).unwrap();
        assert!(json.contains(r#""bottom":"messages""#), "{json}");
        let back: Dock = serde_json::from_str(&json).unwrap();
        assert_eq!(back, d);
        assert_eq!(back.left_width, 310.0);
        // Missing fields fall back to the defaults.
        let partial: Dock = serde_json::from_str(r#"{"right": null}"#).unwrap();
        assert_eq!((partial.left, partial.right), (Some(Panel::Libraries), None));
    }

    #[test]
    fn unseen_problems() {
        let mut u = Unseen::default();
        let (a, b, c) = (problem_key("s", "a"), problem_key("s", "b"), problem_key("d", "a"));
        u.update([a, b], false);
        assert_eq!((u.new, u.total), (2, 2));
        u.update([a, b], true);
        assert_eq!(u.new, 0, "seen while the panel is open");
        u.update([a, b, c], false);
        assert_eq!((u.new, u.total), (1, 3), "only the new one counts");
        u.update([b, c], false);
        assert_eq!(u.new, 1);
        u.update([a, b, c], false);
        assert_eq!(u.new, 2, "a went away and came back");
        u.update([], false);
        assert_eq!((u.new, u.total), (0, 0));
    }
}
