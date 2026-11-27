//! PCB Designer: the desktop application, and its browser build (`web.rs`).
//!
//! Usage: pcb-designer [PROJECT_DIR | PROJECT.kicad_pro | --demo | --example NAME | --stress N] [--no-vsync]
//!                     [--keymap FILE.json] [--mcp] [--mcp-port PORT] [--side-by-side]
//!                     [--library FILE.pcblib]... [--show [part:|symbol:|footprint:]NAME]...
//!                     [--kicad DIR]... [--kicad-pick [footprint:]LIB:NAME]... [--kicad-import]
//!                     [--kicad-search [footprint:]TEXT]
//!                     [--screenshot FILE.png [--frames N] [--select all|REFDES]]
//!                     [--panel NAME]... [--script STEPS]
//!        pcb-designer --mcp-stdio [PROJECT_DIR | --demo | --example NAME]
//!        pcb-designer export-gerber PROJECT_DIR [--assembly generic|jlcpcb|pcbway]
//!        pcb-designer export-schematic-pdf PROJECT_DIR
//!        pcb-designer import-kicad PROJECT.kicad_pro OUT_DIR
//!        pcb-designer export-kicad PROJECT_DIR OUT_DIR
//!        pcb-designer simulate PROJECT_DIR [--analysis NAME] [--csv FILE] [--netlist]
//!
//! `export-gerber` writes the Gerber, drill and job files into `PROJECT_DIR/output/gerber/`
//! and the assembly files (BOM, positions) into `PROJECT_DIR/output/assembly/` without a
//! window, listing design rule violations as warnings. `--assembly` lays the assembly
//! files out for a service for this export (the project's preset otherwise; it isn't
//! saved).
//!
//! `export-schematic-pdf` writes the schematic as `PROJECT_DIR/output/<name>-schematic.pdf`
//! (a page per sheet).
//!
//! `import-kicad` imports a KiCad project (schematic and board) and saves it as a project
//! folder, printing the import report. A `.kicad_pro` given to the app opens its import
//! as an unsaved project.
//!
//! `export-kicad` writes a project as a KiCad 9 project into `OUT_DIR` (new, empty, or an
//! earlier export), printing the export report.
//!
//! `simulate` runs an analysis of the project's simulation setup (by name, default the
//! first; an operating point when there is none) with ngspice and prints a summary, or
//! writes every point to a CSV file; `--netlist` prints the SPICE netlist instead.
//!
//! `--mcp-stdio` runs without a window: an MCP server on stdin/stdout for agents. A
//! project folder that doesn't exist yet becomes a new project saved there.
//!
//! `--show NAME` opens a part, symbol or footprint of the last opened (or built-in)
//! library in its editor tab (repeated: the last one is shown).
//!
//! `--kicad DIR` sets the KiCad library folders (default: the KiCad installation);
//! `--kicad-pick` adds symbols/footprints to the KiCad import window, `--kicad-import`
//! imports them right away into a new library.
//!
//! Without a project or an example an empty, unsaved project is opened. `--demo` is
//! `--example amp`; the examples are `led` and `amp` (a discrete audio amplifier).

// Desktop-only code (command line, files, MCP server) is unused in the browser build;
// the desktop build still checks for dead code.
#![cfg_attr(target_arch = "wasm32", allow(dead_code))]

mod about;
mod activity_ui;
mod agent;
mod agent_switch;
mod app;
mod array_form;
mod ask;
mod background;
mod canvas;
mod checks_ui;
mod clipboard;
mod color_edit;
#[cfg(not(target_arch = "wasm32"))]
mod desktop;
mod dock;
mod fields;
#[cfg(any(target_arch = "wasm32", test))]
mod file_browser;
mod file_dialog;
mod icons;
mod item_tabs;
mod keymap;
mod kicad_import;
mod libraries;
mod library;
mod menu;
mod messages;
mod name_prompt;
#[cfg(target_os = "macos")]
mod native_menu;
mod nav_list;
mod nets_ui;
mod outputs;
mod part_form;
mod parts_ui;
mod pcb_ui;
mod plot_ui;
mod pointer_filter;
mod preview;
mod project_panel;
mod properties;
mod recovery;
mod sch_ui;
mod script;
mod settings;
mod settings_ui;
mod sheet_picker;
mod sim_form;
mod sim_ui;
mod theme;
#[cfg(target_os = "macos")]
mod title_bar;
mod toolbar;
mod tune_ui;
mod units;
#[cfg(target_arch = "wasm32")]
mod web;
#[cfg(target_arch = "wasm32")]
mod web_store;
#[cfg(any(target_arch = "wasm32", test))]
mod wheel;
mod workspace;

#[cfg(not(target_arch = "wasm32"))]
fn main() -> eframe::Result {
    desktop::main()
}

#[cfg(target_arch = "wasm32")]
fn main() {
    web::start();
}
