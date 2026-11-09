//! Customizable key bindings.
//!
//! Every keyboard-triggerable action is an [`Action`]. A [`Keymap`] maps actions to any
//! number of [`Shortcut`]s and is (de)serializable, so a settings UI or config file can
//! change it. Shortcuts are written like `"Cmd+Shift+Z"`; `Cmd` is the platform command
//! key (⌘ on macOS, Ctrl elsewhere).

use std::collections::BTreeMap;
use std::fmt;
use std::str::FromStr;

use egui::{Key, KeyboardShortcut, Modifiers};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    New,
    Open,
    Save,
    SaveAs,
    Undo,
    Redo,
    SelectAll,
    Rotate,
    Mirror,
    Flip,
    Delete,
    Cancel,
    FitView,
    ToolSelect,
    ToolPin,
    ToolLine,
    ToolRect,
    ToolCircle,
    ToolText,
    NavigateBack,
    NavigateForward,
    SchWire,
    SchLabel,
    SchPower,
    SchJunction,
    Posture,
    PcbRoute,
    PcbVia,
    PcbOutline,
    PcbNextLayer,
}

/// Where an action's shortcut applies. Actions of different contexts may share a key.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Context {
    Library,
    Schematic,
    Board,
}

impl Action {
    pub const ALL: &[Action] = &[
        Action::New,
        Action::Open,
        Action::Save,
        Action::SaveAs,
        Action::Undo,
        Action::Redo,
        Action::SelectAll,
        Action::Rotate,
        Action::Mirror,
        Action::Flip,
        Action::Delete,
        Action::Cancel,
        Action::FitView,
        Action::ToolSelect,
        Action::ToolPin,
        Action::ToolLine,
        Action::ToolRect,
        Action::ToolCircle,
        Action::ToolText,
        Action::NavigateBack,
        Action::NavigateForward,
        Action::SchWire,
        Action::SchLabel,
        Action::SchPower,
        Action::SchJunction,
        Action::Posture,
        Action::PcbRoute,
        Action::PcbVia,
        Action::PcbOutline,
        Action::PcbNextLayer,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Action::New => "New project",
            Action::Open => "Open project",
            Action::Save => "Save",
            Action::SaveAs => "Save as",
            Action::Undo => "Undo",
            Action::Redo => "Redo",
            Action::SelectAll => "Select all",
            Action::Rotate => "Rotate",
            Action::Mirror => "Mirror",
            Action::Flip => "Flip side",
            Action::Delete => "Delete",
            Action::Cancel => "Cancel / clear selection",
            Action::FitView => "Fit view",
            Action::ToolSelect => "Select tool",
            Action::ToolPin => "Pin / pad tool",
            Action::ToolLine => "Line tool",
            Action::ToolRect => "Rectangle tool",
            Action::ToolCircle => "Circle tool",
            Action::ToolText => "Text tool",
            Action::NavigateBack => "Back (library editor)",
            Action::NavigateForward => "Forward (library editor)",
            Action::SchWire => "Wire tool (schematic)",
            Action::SchLabel => "Net label tool (schematic)",
            Action::SchPower => "Power port tool (schematic)",
            Action::SchJunction => "Junction tool (schematic)",
            Action::Posture => "Corner direction (wires, tracks)",
            Action::PcbRoute => "Route tool (PCB)",
            Action::PcbVia => "Via tool (PCB)",
            Action::PcbOutline => "Board outline tool (PCB)",
            Action::PcbNextLayer => "Next copper layer; adds a via while routing (PCB)",
        }
    }

    /// The only context the action works in (`None`: everywhere).
    pub fn context(self) -> Option<Context> {
        match self {
            Action::ToolPin
            | Action::ToolLine
            | Action::ToolRect
            | Action::ToolCircle
            | Action::ToolText
            | Action::NavigateBack
            | Action::NavigateForward => Some(Context::Library),
            Action::SchWire | Action::SchLabel | Action::SchPower | Action::SchJunction => Some(Context::Schematic),
            Action::PcbRoute | Action::PcbVia | Action::PcbOutline | Action::PcbNextLayer => Some(Context::Board),
            _ => None,
        }
    }

    fn applies(self, context: Context) -> bool {
        self.context().is_none_or(|c| c == context)
    }

    /// Could both actions be triggered in the same context?
    fn overlaps(self, other: Action) -> bool {
        match (self.context(), other.context()) {
            (Some(a), Some(b)) => a == b,
            _ => true,
        }
    }

    /// Still triggers while a text field has keyboard focus.
    fn works_while_typing(self) -> bool {
        matches!(self, Action::New | Action::Open | Action::Save | Action::SaveAs)
    }

    /// Fires repeatedly while the key is held.
    fn repeats(self) -> bool {
        matches!(self, Action::Undo | Action::Redo)
    }
}

/// A key plus exact modifiers.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Shortcut {
    pub command: bool,
    pub shift: bool,
    pub alt: bool,
    pub key: Key,
}

impl Shortcut {
    pub const fn key(key: Key) -> Self {
        Shortcut { command: false, shift: false, alt: false, key }
    }

    pub const fn cmd(key: Key) -> Self {
        Shortcut { command: true, shift: false, alt: false, key }
    }

    pub const fn alt(key: Key) -> Self {
        Shortcut { command: false, shift: false, alt: true, key }
    }

    pub const fn shift(key: Key) -> Self {
        Shortcut { command: false, shift: true, alt: false, key }
    }

    pub const fn cmd_shift(key: Key) -> Self {
        Shortcut { command: true, shift: true, alt: false, key }
    }

    /// Exact match: extra modifiers prevent a match (so Cmd+Z doesn't fire on Cmd+Shift+Z).
    pub fn matches(&self, key: Key, m: Modifiers) -> bool {
        key == self.key && m.command == self.command && m.shift == self.shift && m.alt == self.alt
    }

    pub fn to_egui(self) -> KeyboardShortcut {
        let mut m = Modifiers::NONE;
        if self.command {
            m |= Modifiers::COMMAND;
        }
        if self.shift {
            m |= Modifiers::SHIFT;
        }
        if self.alt {
            m |= Modifiers::ALT;
        }
        KeyboardShortcut::new(m, self.key)
    }
}

impl fmt::Display for Shortcut {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (on, name) in [(self.command, "Cmd+"), (self.shift, "Shift+"), (self.alt, "Alt+")] {
            if on {
                f.write_str(name)?;
            }
        }
        f.write_str(self.key.name())
    }
}

impl FromStr for Shortcut {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, String> {
        let mut out = Shortcut::key(Key::Escape);
        let parts: Vec<&str> = s.split('+').map(str::trim).collect();
        let (key, mods) = parts.split_last().ok_or("empty shortcut")?;
        for m in mods {
            match m.to_ascii_lowercase().as_str() {
                "cmd" | "command" | "ctrl" | "control" => out.command = true,
                "shift" => out.shift = true,
                "alt" | "option" | "opt" => out.alt = true,
                other => return Err(format!("unknown modifier {other:?} in {s:?}")),
            }
        }
        out.key = Key::from_name(key).ok_or_else(|| format!("unknown key {key:?} in {s:?}"))?;
        Ok(out)
    }
}

impl Serialize for Shortcut {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for Shortcut {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        String::deserialize(d)?.parse().map_err(serde::de::Error::custom)
    }
}

/// Action → shortcuts. Actions missing from a loaded keymap keep their defaults.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Keymap {
    bindings: BTreeMap<Action, Vec<Shortcut>>,
}

impl Default for Keymap {
    fn default() -> Self {
        use Shortcut as S;
        let bindings = [
            (Action::New, vec![S::cmd(Key::N)]),
            (Action::Open, vec![S::cmd(Key::O)]),
            (Action::Save, vec![S::cmd(Key::S)]),
            (Action::SaveAs, vec![S::cmd_shift(Key::S)]),
            (Action::Undo, vec![S::cmd(Key::Z)]),
            (Action::Redo, vec![S::cmd_shift(Key::Z), S::cmd(Key::Y)]),
            (Action::SelectAll, vec![S::cmd(Key::A)]),
            (Action::Rotate, vec![S::key(Key::R)]),
            (Action::Mirror, vec![S::key(Key::X)]),
            (Action::Flip, vec![S::key(Key::F)]),
            (Action::Delete, vec![S::key(Key::Delete), S::key(Key::Backspace)]),
            (Action::Cancel, vec![S::key(Key::Escape)]),
            (Action::FitView, vec![S::key(Key::V), S::key(Key::Home)]),
            (Action::ToolSelect, vec![S::key(Key::S)]),
            (Action::ToolPin, vec![S::key(Key::P)]),
            (Action::ToolLine, vec![S::key(Key::L)]),
            (Action::ToolRect, vec![S::key(Key::B)]),
            (Action::ToolCircle, vec![S::key(Key::C)]),
            (Action::ToolText, vec![S::key(Key::T)]),
            (Action::NavigateBack, vec![S::alt(Key::ArrowLeft), S::cmd(Key::OpenBracket)]),
            (Action::NavigateForward, vec![S::alt(Key::ArrowRight), S::cmd(Key::CloseBracket)]),
            (Action::SchWire, vec![S::key(Key::W)]),
            (Action::SchLabel, vec![S::key(Key::L)]),
            (Action::SchPower, vec![S::key(Key::P)]),
            (Action::SchJunction, vec![S::key(Key::J)]),
            (Action::Posture, vec![S::key(Key::Slash)]),
            (Action::PcbRoute, vec![S::key(Key::T)]),
            (Action::PcbVia, vec![S::shift(Key::V)]),
            (Action::PcbOutline, vec![S::key(Key::O)]),
            (Action::PcbNextLayer, vec![S::key(Key::L)]),
        ];
        Keymap { bindings: bindings.into_iter().collect() }
    }
}

impl Keymap {
    /// Parse a keymap (JSON object `{"undo": ["Cmd+Z"], ...}`) on top of the defaults.
    pub fn from_json(json: &str) -> Result<Self, serde_json::Error> {
        let custom: Keymap = serde_json::from_str(json)?;
        let mut map = Keymap::default();
        map.bindings.extend(custom.bindings);
        Ok(map)
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).expect("keymap serializes")
    }

    pub fn shortcuts(&self, action: Action) -> &[Shortcut] {
        self.bindings.get(&action).map_or(&[], Vec::as_slice)
    }

    /// The action bound to `key` + `modifiers` in `context`, if any.
    pub fn lookup(&self, key: Key, modifiers: Modifiers, context: Context) -> Option<Action> {
        self.bindings
            .iter()
            .find(|(a, s)| a.applies(context) && s.iter().any(|s| s.matches(key, modifiers)))
            .map(|(a, _)| *a)
    }

    /// Shortcuts bound to more than one action of the same context.
    pub fn conflicts(&self) -> Vec<(Shortcut, Vec<Action>)> {
        let mut by_shortcut: BTreeMap<String, (Shortcut, Vec<Action>)> = BTreeMap::new();
        for (&action, shortcuts) in &self.bindings {
            for &s in shortcuts {
                by_shortcut.entry(s.to_string()).or_insert((s, Vec::new())).1.push(action);
            }
        }
        by_shortcut
            .into_values()
            .filter(|(_, a)| a.iter().enumerate().any(|(i, x)| a[i + 1..].iter().any(|y| x.overlaps(*y))))
            .collect()
    }

    /// Text for menus, e.g. "⌘⇧Z" on macOS.
    pub fn menu_text(&self, ctx: &egui::Context, action: Action) -> String {
        self.shortcuts(action).first().map_or(String::new(), |s| ctx.format_shortcut(&s.to_egui()))
    }

    /// Actions triggered by this frame's key presses. Matched key events are consumed so
    /// egui widgets don't also react to them.
    pub fn triggered(&self, ctx: &egui::Context, context: Context) -> Vec<Action> {
        let typing = ctx.egui_wants_keyboard_input();
        let mut actions = Vec::new();
        ctx.input_mut(|i| {
            i.events.retain(|e| {
                let egui::Event::Key { key, pressed: true, repeat, modifiers, .. } = e else { return true };
                let Some(action) = self.lookup(*key, *modifiers, context) else { return true };
                if (typing && !action.works_while_typing()) || (*repeat && !action.repeats()) {
                    return true;
                }
                actions.push(action);
                false
            });
        });
        actions
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_have_no_conflicts_and_cover_all_actions() {
        let map = Keymap::default();
        assert!(map.conflicts().is_empty());
        for &a in Action::ALL {
            assert!(!map.shortcuts(a).is_empty(), "{a:?} has no default shortcut");
        }
    }

    #[test]
    fn exact_modifier_matching() {
        let map = Keymap::default();
        let sch = Context::Schematic;
        assert_eq!(map.lookup(Key::Z, Modifiers::COMMAND, sch), Some(Action::Undo));
        assert_eq!(map.lookup(Key::Z, Modifiers::COMMAND | Modifiers::SHIFT, sch), Some(Action::Redo));
        assert_eq!(map.lookup(Key::R, Modifiers::NONE, sch), Some(Action::Rotate));
        assert_eq!(map.lookup(Key::R, Modifiers::COMMAND, sch), None);
    }

    #[test]
    fn contexts_share_keys() {
        let map = Keymap::default();
        assert_eq!(map.lookup(Key::L, Modifiers::NONE, Context::Library), Some(Action::ToolLine));
        assert_eq!(map.lookup(Key::L, Modifiers::NONE, Context::Schematic), Some(Action::SchLabel));
        assert_eq!(map.lookup(Key::L, Modifiers::NONE, Context::Board), Some(Action::PcbNextLayer));
        // Same context: a conflict.
        let map = Keymap::from_json(r#"{"sch_wire": ["L"]}"#).unwrap();
        assert_eq!(map.conflicts().len(), 1);
        // A global action clashes with every context.
        let map = Keymap::from_json(r#"{"rotate": ["W"]}"#).unwrap();
        assert_eq!(map.conflicts()[0].1, vec![Action::Rotate, Action::SchWire]);
    }

    #[test]
    fn parse_and_print() {
        let s: Shortcut = "Cmd+Shift+Z".parse().unwrap();
        assert_eq!(s, Shortcut::cmd_shift(Key::Z));
        assert_eq!(s.to_string(), "Cmd+Shift+Z");
        assert_eq!("ctrl+alt+Delete".parse::<Shortcut>().unwrap().to_string(), "Cmd+Alt+Delete");
        assert!("Hyper+Z".parse::<Shortcut>().is_err());
        assert!("Cmd+NoSuchKey".parse::<Shortcut>().is_err());
    }

    #[test]
    fn json_overrides_keep_other_defaults() {
        let map = Keymap::from_json(r#"{"rotate": ["Space"], "delete": []}"#).unwrap();
        let b = Context::Board;
        assert_eq!(map.lookup(Key::Space, Modifiers::NONE, b), Some(Action::Rotate));
        assert_eq!(map.lookup(Key::R, Modifiers::NONE, b), None);
        assert_eq!(map.lookup(Key::Delete, Modifiers::NONE, b), None);
        assert_eq!(map.lookup(Key::Z, Modifiers::COMMAND, b), Some(Action::Undo));
        let round = Keymap::from_json(&map.to_json()).unwrap();
        assert_eq!(round, map);
    }
}
