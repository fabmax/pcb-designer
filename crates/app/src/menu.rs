//! The main menu, described once and drawn either by egui (a bar in the window) or, on
//! macOS, natively in the system menu bar (`native_menu.rs`).
//!
//! [`menus`] builds the description from a [`MenuState`] snapshot; picking an entry yields
//! a [`Cmd`] that `PcbApp::run_menu_cmd` carries out.

use std::path::PathBuf;

use egui::RichText;
use view::style::Theme;

use crate::keymap::{Action, Keymap, Shortcut};
use crate::theme;

/// What picking a menu entry does.
#[derive(Clone, Debug, PartialEq)]
pub enum Cmd {
    Action(Action),
    OpenRecent(PathBuf),
    ClearRecent,
    OpenDemo,
    NewLibrary,
    AddLibrary,
    ImportKicad,
    Quit,
    Layout { side_by_side: bool },
    Theme(Theme),
    Drc,
    ToggleAgent,
    Copy(String),
    ShowShortcuts,
}

/// Standard macOS entries, carried out by the system (native menu only).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum System {
    About,
    Services,
    Hide,
    HideOthers,
    ShowAll,
    Minimize,
    Zoom,
    Fullscreen,
}

/// How a [`Entry::Note`] is shown.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tone {
    Dim,
    Warn,
    Mono,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Entry {
    Item {
        label: String,
        cmd: Cmd,
        shortcut: Option<Shortcut>,
        enabled: bool,
    },
    /// A checkbox, or a radio button of a group of mutually exclusive choices.
    Check {
        label: String,
        cmd: Cmd,
        checked: bool,
        radio: bool,
        hover: Option<&'static str>,
    },
    Sub {
        label: String,
        enabled: bool,
        entries: Vec<Entry>,
    },
    /// Text that can't be picked.
    Note {
        text: String,
        tone: Tone,
    },
    System(System),
    Separator,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Menu {
    pub title: String,
    pub entries: Vec<Entry>,
}

/// Everything the menus show.
pub struct MenuState<'a> {
    /// Built for the system menu bar of macOS: adds the application and Window menus
    /// (Settings and Quit move into the application menu).
    pub native: bool,
    pub keymap: &'a Keymap,
    /// A library (not the project) has the focus: Save acts on it.
    pub library_focus: bool,
    pub recent: &'a [PathBuf],
    pub undo: Option<String>,
    pub redo: Option<String>,
    pub side_by_side: bool,
    pub theme: Theme,
    pub agent_running: bool,
    pub agent_error: Option<String>,
    pub agent_url: Option<String>,
    pub agent_port: u16,
    /// The `claude mcp add` command for the headless (stdio) mode.
    pub headless_command: String,
}

pub const APP_NAME: &str = "PCB Designer";

fn item(label: impl Into<String>, cmd: Cmd) -> Entry {
    Entry::Item { label: label.into(), cmd, shortcut: None, enabled: true }
}

fn note(text: impl Into<String>, tone: Tone) -> Entry {
    Entry::Note { text: text.into(), tone }
}

pub fn menus(s: &MenuState) -> Vec<Menu> {
    let action = |label: &str, a: Action, enabled: bool| Entry::Item {
        label: label.to_string(),
        cmd: Cmd::Action(a),
        shortcut: s.keymap.shortcuts(a).first().copied(),
        enabled,
    };
    let menu = |title: &str, entries| Menu { title: title.to_string(), entries };
    let mut out = Vec::new();

    if s.native {
        out.push(menu(
            APP_NAME,
            vec![
                Entry::System(System::About),
                Entry::Separator,
                action("Settings…", Action::Settings, true),
                Entry::Separator,
                Entry::System(System::Services),
                Entry::Separator,
                Entry::System(System::Hide),
                Entry::System(System::HideOthers),
                Entry::System(System::ShowAll),
                Entry::Separator,
                Entry::Item {
                    label: format!("Quit {APP_NAME}"),
                    cmd: Cmd::Quit,
                    shortcut: Some(Shortcut::cmd(egui::Key::Q)),
                    enabled: true,
                },
            ],
        ));
    }

    let mut recent: Vec<Entry> = s
        .recent
        .iter()
        .map(|p| {
            item(p.file_name().map_or(String::new(), |n| n.to_string_lossy().into_owned()), Cmd::OpenRecent(p.clone()))
        })
        .collect();
    recent.extend([Entry::Separator, item("Clear list", Cmd::ClearRecent)]);
    let (save, save_as) =
        if s.library_focus { ("Save library", "Save library as…") } else { ("Save project", "Save project as…") };
    let mut file = vec![
        action("New project", Action::New, true),
        action("Open project…", Action::Open, true),
        Entry::Sub { label: "Open recent".into(), enabled: !s.recent.is_empty(), entries: recent },
        item("Open demo project", Cmd::OpenDemo),
        Entry::Separator,
        item("New library…", Cmd::NewLibrary),
        item("Add library…", Cmd::AddLibrary),
        item("Import from KiCad…", Cmd::ImportKicad),
        Entry::Separator,
        action(save, Action::Save, true),
        action(save_as, Action::SaveAs, true),
    ];
    if !s.native {
        file.extend([Entry::Separator, item("Quit", Cmd::Quit)]);
    }
    out.push(menu("File", file));

    let mut edit = vec![
        action(s.undo.as_deref().unwrap_or("Undo"), Action::Undo, s.undo.is_some()),
        action(s.redo.as_deref().unwrap_or("Redo"), Action::Redo, s.redo.is_some()),
        Entry::Separator,
    ];
    for a in [Action::SelectAll, Action::Rotate, Action::Mirror, Action::Flip, Action::Delete] {
        edit.push(action(a.label(), a, true));
    }
    if !s.native {
        edit.extend([Entry::Separator, action("Settings…", Action::Settings, true)]);
    }
    out.push(menu("Edit", edit));

    let mut view = vec![
        action("Fit view", Action::FitView, true),
        Entry::Separator,
        Entry::Check {
            label: "Tabs".into(),
            cmd: Cmd::Layout { side_by_side: false },
            checked: !s.side_by_side,
            radio: true,
            hover: Some("Schematic and PCB as full-size tabs"),
        },
        Entry::Check {
            label: "Side by side".into(),
            cmd: Cmd::Layout { side_by_side: true },
            checked: s.side_by_side,
            radio: true,
            hover: Some("Schematic left, PCB right"),
        },
        Entry::Separator,
    ];
    for &t in Theme::ALL {
        view.push(Entry::Check {
            label: format!("{} theme", t.label()),
            cmd: Cmd::Theme(t),
            checked: s.theme == t,
            radio: true,
            hover: None,
        });
    }
    if s.native {
        view.extend([Entry::Separator, Entry::System(System::Fullscreen)]);
    }
    out.push(menu("View", view));

    let mut tools = vec![
        item("Run design rule check", Cmd::Drc),
        Entry::Separator,
        Entry::Check {
            label: "Agent access (MCP server)".into(),
            cmd: Cmd::ToggleAgent,
            checked: s.agent_running,
            radio: false,
            hover: None,
        },
    ];
    if let Some(e) = &s.agent_error {
        tools.push(note(e, Tone::Warn));
    }
    if let Some(url) = &s.agent_url {
        let json = format!(
            "{{\n  \"mcpServers\": {{\n    \"pcb-designer\": {{ \"type\": \"http\", \"url\": \"{url}\" }}\n  }}\n}}"
        );
        tools.extend([
            note(url, Tone::Mono),
            item("Copy URL", Cmd::Copy(url.clone())),
            item("Copy Claude Code command", Cmd::Copy(format!("claude mcp add --transport http pcb-designer {url}"))),
            item("Copy JSON config", Cmd::Copy(json)),
        ]);
    } else {
        tools.push(note(format!("Listens on 127.0.0.1:{} only", s.agent_port), Tone::Dim));
    }
    tools.extend([
        Entry::Separator,
        note("Headless (no window, MCP on stdio):", Tone::Dim),
        item("Copy Claude Code command (headless)", Cmd::Copy(s.headless_command.clone())),
    ]);
    out.push(menu("Tools", tools));

    if s.native {
        out.push(menu("Window", vec![Entry::System(System::Minimize), Entry::System(System::Zoom)]));
    }
    out.push(menu("Help", vec![item("Keyboard shortcuts", Cmd::ShowShortcuts)]));
    out
}

/// Draws the menus as an egui menu bar. Returns the picked command.
pub fn bar_ui(ui: &mut egui::Ui, menus: &[Menu]) -> Option<Cmd> {
    let mut picked = None;
    egui::MenuBar::new().ui(ui, |ui| {
        for m in menus {
            ui.menu_button(&m.title, |ui| entries_ui(ui, &m.entries, &mut picked));
        }
    });
    picked
}

fn entries_ui(ui: &mut egui::Ui, entries: &[Entry], picked: &mut Option<Cmd>) {
    for e in entries {
        match e {
            Entry::Item { label, cmd, shortcut, enabled } => {
                let mut b = egui::Button::new(label);
                if let Some(s) = shortcut {
                    b = b.shortcut_text(ui.ctx().format_shortcut(&s.to_egui()));
                }
                if ui.add_enabled(*enabled, b).clicked() {
                    *picked = Some(cmd.clone());
                }
            }
            Entry::Check { label, cmd, checked, radio, hover } => {
                let mut r = if *radio {
                    ui.radio(*checked, label)
                } else {
                    let mut on = *checked;
                    ui.checkbox(&mut on, label)
                };
                if let Some(h) = hover {
                    r = r.on_hover_text(*h);
                }
                // Picking the selected radio button changes nothing.
                if r.clicked() && !(*radio && *checked) {
                    *picked = Some(cmd.clone());
                }
            }
            Entry::Sub { label, enabled, entries } => {
                ui.add_enabled_ui(*enabled, |ui| {
                    ui.menu_button(label, |ui| entries_ui(ui, entries, picked));
                });
            }
            Entry::Note { text, tone } => {
                let t = RichText::new(text);
                ui.label(match tone {
                    Tone::Dim => t.color(theme::text_dim()),
                    Tone::Warn => t.color(theme::warn()),
                    Tone::Mono => t.monospace().color(theme::text_dim()),
                });
            }
            Entry::System(_) => {}
            Entry::Separator => {
                ui.separator();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(native: bool, keymap: &Keymap) -> MenuState<'_> {
        MenuState {
            native,
            keymap,
            library_focus: false,
            recent: &[],
            undo: Some("Undo move".into()),
            redo: None,
            side_by_side: false,
            theme: Theme::Dark,
            agent_running: false,
            agent_error: None,
            agent_url: None,
            agent_port: 4321,
            headless_command: "claude mcp add …".into(),
        }
    }

    fn find<'a>(menus: &'a [Menu], cmd: &Cmd) -> Option<&'a Entry> {
        fn walk<'a>(entries: &'a [Entry], cmd: &Cmd) -> Option<&'a Entry> {
            entries.iter().find_map(|e| match e {
                Entry::Item { cmd: c, .. } | Entry::Check { cmd: c, .. } if c == cmd => Some(e),
                Entry::Sub { entries, .. } => walk(entries, cmd),
                _ => None,
            })
        }
        menus.iter().find_map(|m| walk(&m.entries, cmd))
    }

    #[test]
    fn native_menus_follow_macos_conventions() {
        let keymap = Keymap::default();
        let titles = |m: &[Menu]| m.iter().map(|m| m.title.clone()).collect::<Vec<_>>();
        let window = menus(&state(false, &keymap));
        assert_eq!(titles(&window), ["File", "Edit", "View", "Tools", "Help"]);
        let native = menus(&state(true, &keymap));
        assert_eq!(titles(&native), [APP_NAME, "File", "Edit", "View", "Tools", "Window", "Help"]);
        // Settings and Quit live in the application menu, once.
        let settings = Cmd::Action(Action::Settings);
        assert!(find(&native[..1], &settings).is_some() && find(&native[1..], &settings).is_none());
        assert!(find(&native[..1], &Cmd::Quit).is_some() && find(&native[1..], &Cmd::Quit).is_none());
        assert!(find(&window, &settings).is_some() && find(&window, &Cmd::Quit).is_some());
    }

    #[test]
    fn entries_reflect_the_state() {
        let keymap = Keymap::default();
        let mut s = state(false, &keymap);
        let m = menus(&s);
        let Some(Entry::Item { label, enabled, shortcut, .. }) = find(&m, &Cmd::Action(Action::Undo)) else { panic!() };
        assert_eq!((label.as_str(), *enabled), ("Undo move", true));
        assert_eq!(*shortcut, Some(Shortcut::cmd(egui::Key::Z)));
        let Some(Entry::Item { enabled, .. }) = find(&m, &Cmd::Action(Action::Redo)) else { panic!() };
        assert!(!enabled);
        assert!(find(&m, &Cmd::Copy("x".into())).is_none());

        let recent = [PathBuf::from("/a/blinky")];
        s.recent = &recent;
        s.side_by_side = true;
        s.agent_url = Some("http://127.0.0.1:4321/mcp".into());
        let m = menus(&s);
        let Some(Entry::Item { label, .. }) = find(&m, &Cmd::OpenRecent(recent[0].clone())) else { panic!() };
        assert_eq!(label, "blinky");
        let Some(Entry::Check { checked, .. }) = find(&m, &Cmd::Layout { side_by_side: true }) else { panic!() };
        assert!(checked);
        assert!(find(&m, &Cmd::Copy("http://127.0.0.1:4321/mcp".into())).is_some());
    }
}
