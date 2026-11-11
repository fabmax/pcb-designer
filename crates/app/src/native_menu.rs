//! The main menu in the macOS system menu bar (muda), built from [`menu::menus`].
//!
//! Shortcuts: macOS menus grab their key equivalents before the window sees them. Only
//! shortcuts with ⌘ become key equivalents (plain keys like R would stop typing in
//! text fields), and an entry fired that way is fed back to egui as its key press, so it
//! goes through the keymap like any other key: ⌘Z in a text field still undoes typing.

use std::sync::mpsc::{Receiver, channel};

use muda::accelerator::Accelerator;
use muda::{CheckMenuItem, IsMenuItem, MenuEvent, MenuItem, PredefinedMenuItem, Submenu};

use crate::keymap::Shortcut;
use crate::menu::{self, Cmd, Entry, Menu, System};

/// Native widget of an entry, in [`flatten`] order.
enum Handle {
    Item(MenuItem),
    Check(CheckMenuItem),
    Sub(Submenu),
    Other,
}

pub struct NativeMenu {
    bar: muda::Menu,
    handles: Vec<Handle>,
    /// Command of each handle (the menu id is the index).
    cmds: Vec<Option<(Cmd, Option<Shortcut>)>>,
    shown: Vec<Menu>,
    events: Receiver<MenuEvent>,
    /// Picked since the last sync: macOS toggles check items itself, so state is reset.
    stale: bool,
}

/// Picking a native entry.
pub enum Picked {
    Cmd(Cmd),
    /// Replay this shortcut as a key press.
    Key(Shortcut),
}

impl NativeMenu {
    pub fn new(ctx: &egui::Context) -> Self {
        let (tx, events) = channel();
        let ctx = ctx.clone();
        MenuEvent::set_event_handler(Some(move |e| {
            let _ = tx.send(e);
            ctx.request_repaint();
        }));
        NativeMenu {
            bar: muda::Menu::new(),
            handles: Vec::new(),
            cmds: Vec::new(),
            shown: Vec::new(),
            events,
            stale: false,
        }
    }

    /// Shows `menus`, updating the native menus in place when only texts and states changed.
    pub fn sync(&mut self, menus: Vec<Menu>) {
        if menus == self.shown && !self.stale {
            return;
        }
        let flat = flatten(&menus);
        if flat.len() == self.handles.len() && flat.iter().zip(&self.handles).all(|(e, h)| same_kind(*e, h)) {
            for (i, (e, h)) in flat.iter().zip(&self.handles).enumerate() {
                update(*e, h);
                self.cmds[i] = cmd_of(*e);
            }
        } else {
            self.rebuild(&menus);
        }
        self.shown = menus;
        self.stale = false;
    }

    fn rebuild(&mut self, menus: &[Menu]) {
        let bar = muda::Menu::new();
        self.handles.clear();
        self.cmds.clear();
        for m in menus {
            let sub = Submenu::new(&m.title, true);
            self.handles.push(Handle::Sub(sub.clone()));
            self.cmds.push(None);
            self.append(&sub, &m.entries);
            let _ = bar.append(&sub);
            match m.title.as_str() {
                "Window" => sub.set_as_windows_menu_for_nsapp(),
                "Help" => sub.set_as_help_menu_for_nsapp(),
                _ => {}
            }
        }
        bar.init_for_nsapp();
        self.bar = bar;
    }

    fn append(&mut self, parent: &Submenu, entries: &[Entry]) {
        for e in entries {
            let id = self.handles.len().to_string();
            let (handle, native): (Handle, Box<dyn IsMenuItem>) = match e {
                Entry::Item { label, shortcut, enabled, .. } => {
                    let i = MenuItem::with_id(id, label, *enabled, shortcut.and_then(accelerator));
                    (Handle::Item(i.clone()), Box::new(i))
                }
                Entry::Check { label, checked, .. } => {
                    let c = CheckMenuItem::with_id(id, label, true, *checked, None);
                    (Handle::Check(c.clone()), Box::new(c))
                }
                Entry::Sub { label, enabled, entries } => {
                    let s = Submenu::with_id(id, label, *enabled);
                    self.handles.push(Handle::Sub(s.clone()));
                    self.cmds.push(None);
                    self.append(&s, entries);
                    let _ = parent.append(&s);
                    continue;
                }
                Entry::Note { text, .. } => {
                    let i = MenuItem::with_id(id, text, false, None);
                    (Handle::Item(i.clone()), Box::new(i))
                }
                Entry::System(s) => (Handle::Other, Box::new(system(*s))),
                Entry::Separator => (Handle::Other, Box::new(PredefinedMenuItem::separator())),
            };
            let _ = parent.append(native.as_ref());
            self.handles.push(handle);
            self.cmds.push(cmd_of(Some(e)));
        }
    }

    /// Entries picked since the last call.
    pub fn picked(&mut self) -> Vec<Picked> {
        let mut out = Vec::new();
        while let Ok(e) = self.events.try_recv() {
            self.stale = true;
            let Some(Some((cmd, shortcut))) = e.id.as_ref().parse::<usize>().ok().and_then(|i| self.cmds.get(i)) else {
                continue;
            };
            out.push(match (cmd, shortcut) {
                (Cmd::Action(_), Some(s)) if accelerator(*s).is_some() => Picked::Key(*s),
                _ => Picked::Cmd(cmd.clone()),
            });
        }
        out
    }
}

/// The key press egui would have seen for `s`.
pub fn key_event(s: Shortcut) -> egui::Event {
    let modifiers = egui::Modifiers { alt: s.alt, shift: s.shift, mac_cmd: s.command, command: s.command, ctrl: false };
    egui::Event::Key { key: s.key, physical_key: None, pressed: true, repeat: false, modifiers }
}

/// Entries in creation order; a menu or submenu comes before its entries. Top-level
/// menus are `None` (their titles never change).
fn flatten(menus: &[Menu]) -> Vec<Option<&Entry>> {
    fn walk<'a>(entries: &'a [Entry], out: &mut Vec<Option<&'a Entry>>) {
        for e in entries {
            out.push(Some(e));
            if let Entry::Sub { entries, .. } = e {
                walk(entries, out);
            }
        }
    }
    let mut out = Vec::new();
    for m in menus {
        out.push(None);
        walk(&m.entries, &mut out);
    }
    out
}

fn same_kind(e: Option<&Entry>, h: &Handle) -> bool {
    let Some(e) = e else { return matches!(h, Handle::Sub(_)) };
    match (e, h) {
        (Entry::Item { .. } | Entry::Note { .. }, Handle::Item(_)) => true,
        (Entry::Check { .. }, Handle::Check(_)) => true,
        (Entry::Sub { .. }, Handle::Sub(_)) => true,
        // A system entry or separator: rebuild when one turns into the other.
        (Entry::System(_) | Entry::Separator, Handle::Other) => true,
        _ => false,
    }
}

fn update(e: Option<&Entry>, h: &Handle) {
    let Some(e) = e else { return };
    match (e, h) {
        (Entry::Item { label, shortcut, enabled, .. }, Handle::Item(i)) => {
            i.set_text(label);
            i.set_enabled(*enabled);
            let _ = i.set_accelerator(shortcut.and_then(accelerator));
        }
        (Entry::Note { text, .. }, Handle::Item(i)) => i.set_text(text),
        (Entry::Check { label, checked, .. }, Handle::Check(c)) => {
            c.set_text(label);
            c.set_checked(*checked);
        }
        (Entry::Sub { label, enabled, .. }, Handle::Sub(s)) => {
            s.set_text(label);
            s.set_enabled(*enabled);
        }
        _ => {}
    }
}

fn cmd_of(e: Option<&Entry>) -> Option<(Cmd, Option<Shortcut>)> {
    match e? {
        Entry::Item { cmd, shortcut, .. } => Some((cmd.clone(), *shortcut)),
        Entry::Check { cmd, .. } => Some((cmd.clone(), None)),
        _ => None,
    }
}

/// Key equivalent for shortcuts with ⌘ (others would fire while typing).
fn accelerator(s: Shortcut) -> Option<Accelerator> {
    if !s.command {
        return None;
    }
    let mut mods = String::from("super+");
    if s.shift {
        mods.push_str("shift+");
    }
    if s.alt {
        mods.push_str("alt+");
    }
    // muda takes key names like "Z", "1", "Comma", "ArrowUp".
    let name = match s.key {
        egui::Key::Comma => "Comma",
        egui::Key::Period => "Period",
        egui::Key::Slash => "Slash",
        egui::Key::Minus => "Minus",
        egui::Key::Equals => "Equal",
        k => k.name(),
    };
    format!("{mods}{name}").parse().ok()
}

fn system(s: System) -> PredefinedMenuItem {
    match s {
        System::About => PredefinedMenuItem::about(Some(&format!("About {}", menu::APP_NAME)), None),
        System::Services => PredefinedMenuItem::services(None),
        System::Hide => PredefinedMenuItem::hide(Some(&format!("Hide {}", menu::APP_NAME))),
        System::HideOthers => PredefinedMenuItem::hide_others(None),
        System::ShowAll => PredefinedMenuItem::show_all(None),
        System::Minimize => PredefinedMenuItem::minimize(None),
        System::Zoom => PredefinedMenuItem::maximize(None),
        System::Fullscreen => PredefinedMenuItem::fullscreen(None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keymap::Action;

    #[test]
    fn only_command_shortcuts_become_key_equivalents() {
        let default = crate::keymap::Keymap::default();
        let first = |a: Action| default.shortcuts(a)[0];
        assert!(accelerator(first(Action::Undo)).is_some());
        assert!(accelerator(first(Action::SaveAs)).is_some());
        assert!(accelerator(first(Action::Settings)).is_some());
        assert!(accelerator(first(Action::PanelProject)).is_some());
        assert!(accelerator(first(Action::Rotate)).is_none());
        assert!(accelerator(first(Action::Delete)).is_none());
    }
}
