//! PCB Designer desktop application.
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
//! `--example led`; the examples are `led` and `amp` (a discrete audio amplifier).

mod activity_ui;
mod agent;
mod agent_switch;
mod app;
mod array_form;
mod canvas;
mod checks_ui;
mod clipboard;
mod color_edit;
mod dock;
mod fields;
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
mod workspace;

use app::{Options, Startup};
use view::demo::Example;

fn main() -> eframe::Result {
    env_logger::init();
    let mut opts = Options {
        startup: Startup::Empty,
        screenshot: None,
        screenshot_frames: 30,
        select: None,
        keymap: None,
        mcp: None,
        library: Vec::new(),
        show: Vec::new(),
        kicad_paths: Vec::new(),
        kicad_pick: Vec::new(),
        kicad_import: false,
        kicad_query: None,
        panels: Vec::new(),
        script: None,
        side_by_side: false,
    };
    let mut vsync = true;
    let mut stdio = false;
    let mut args = std::env::args().skip(1);
    if std::env::args().nth(1).as_deref() == Some("export-gerber") {
        let usage = || {
            eprintln!("usage: pcb-designer export-gerber PROJECT_DIR [--assembly generic|jlcpcb|pcbway]");
            std::process::exit(2);
        };
        let rest: Vec<String> = std::env::args().skip(2).collect();
        let (dir, preset) = match rest.as_slice() {
            [dir] => (dir, None),
            [dir, flag, name] if flag == "--assembly" => match model::AssemblyPreset::parse(name) {
                Some(p) => (dir, Some(p)),
                None => usage(),
            },
            _ => usage(),
        };
        if let Err(e) = export_gerber(dir.as_ref(), preset) {
            eprintln!("pcb-designer: {e}");
            std::process::exit(1);
        }
        return Ok(());
    }
    if std::env::args().nth(1).as_deref() == Some("export-schematic-pdf") {
        let Some(dir) = std::env::args().nth(2) else {
            eprintln!("usage: pcb-designer export-schematic-pdf PROJECT_DIR");
            std::process::exit(2);
        };
        let pdf = view::Document::open(std::path::Path::new(&dir))
            .map_err(|e| format!("cannot open {dir}: {e}"))
            .and_then(|mut doc| doc.export_schematic_pdf());
        match pdf {
            Ok(file) => println!("{}", file.display()),
            Err(e) => {
                eprintln!("pcb-designer: {e}");
                std::process::exit(1);
            }
        }
        return Ok(());
    }
    if std::env::args().nth(1).as_deref() == Some("simulate") {
        if let Err(e) = simulate(std::env::args().skip(2).collect()) {
            eprintln!("pcb-designer: {e}");
            std::process::exit(1);
        }
        return Ok(());
    }
    if std::env::args().nth(1).as_deref() == Some("import-kicad") {
        let (Some(pro), Some(out)) = (std::env::args().nth(2), std::env::args().nth(3)) else {
            eprintln!("usage: pcb-designer import-kicad PROJECT.kicad_pro OUT_DIR");
            std::process::exit(2);
        };
        if let Err(e) = import_kicad(pro.as_ref(), out.as_ref()) {
            eprintln!("pcb-designer: {e}");
            std::process::exit(1);
        }
        return Ok(());
    }
    if std::env::args().nth(1).as_deref() == Some("export-kicad") {
        let (Some(dir), Some(out)) = (std::env::args().nth(2), std::env::args().nth(3)) else {
            eprintln!("usage: pcb-designer export-kicad PROJECT_DIR OUT_DIR");
            std::process::exit(2);
        };
        if let Err(e) = export_kicad(dir.as_ref(), out.as_ref()) {
            eprintln!("pcb-designer: {e}");
            std::process::exit(1);
        }
        return Ok(());
    }
    while let Some(a) = args.next() {
        match a.as_str() {
            "--demo" => opts.startup = Startup::Example(Example::Led),
            "--example" => {
                let name = args.next().unwrap_or_default();
                let Some(example) = Example::parse(&name) else {
                    let names: Vec<_> = Example::ALL.iter().map(|e| e.name()).collect();
                    eprintln!("pcb-designer: no example {name:?} (examples: {})", names.join(", "));
                    std::process::exit(2);
                };
                opts.startup = Startup::Example(example);
            }
            "--mcp-stdio" => stdio = true,
            "--stress" => opts.startup = Startup::Stress(args.next().and_then(|v| v.parse().ok()).unwrap_or(2_000)),
            "--no-vsync" => vsync = false,
            "--side-by-side" => opts.side_by_side = true,
            "--screenshot" => opts.screenshot = args.next(),
            "--frames" => opts.screenshot_frames = args.next().and_then(|v| v.parse().ok()).unwrap_or(30),
            "--select" => opts.select = args.next(),
            "--script" => opts.script = args.next(),
            "--library" => opts.library.extend(args.next().map(Into::into)),
            "--show" => opts.show.extend(args.next()),
            "--panel" => opts.panels.extend(args.next()),
            "--kicad" => opts.kicad_paths.extend(args.next().map(Into::into)),
            "--kicad-pick" => opts.kicad_pick.extend(args.next()),
            "--kicad-import" => opts.kicad_import = true,
            "--kicad-search" => opts.kicad_query = args.next(),
            "--mcp" => opts.mcp = Some(opts.mcp.unwrap_or(agent::DEFAULT_PORT)),
            "--mcp-port" => opts.mcp = args.next().and_then(|v| v.parse().ok()).or(Some(agent::DEFAULT_PORT)),
            "--keymap" => {
                let path = args.next().unwrap_or_default();
                match std::fs::read_to_string(&path)
                    .map_err(|e| e.to_string())
                    .and_then(|json| keymap::Keymap::from_json(&json).map_err(|e| e.to_string()))
                {
                    Ok(map) => {
                        for (shortcut, actions) in map.conflicts() {
                            eprintln!("keymap: {shortcut} is bound to several actions: {actions:?}");
                        }
                        opts.keymap = Some(map);
                    }
                    Err(e) => eprintln!("keymap {path:?} not loaded: {e}"),
                }
            }
            other if other.ends_with(".kicad_pro") => opts.startup = Startup::ImportKicad(other.into()),
            other if !other.starts_with("--") => opts.startup = Startup::Open(other.into()),
            other => eprintln!("ignoring unknown argument {other:?}"),
        }
    }

    if stdio {
        if let Err(e) = run_stdio(opts.startup, opts.kicad_paths.clone()) {
            eprintln!("pcb-designer: {e}");
            std::process::exit(1);
        }
        return Ok(());
    }

    let mut wgpu_options = egui_wgpu::WgpuConfiguration::default();
    if !vsync {
        wgpu_options.surface.present_mode = wgpu::PresentMode::AutoNoVsync;
    }
    let mut viewport = egui::ViewportBuilder::default()
        .with_title("PCB Designer")
        .with_inner_size([1600.0, 1000.0])
        .with_min_inner_size([800.0, 500.0]);
    match eframe::icon_data::from_png_bytes(icons::APP_ICON_PNG) {
        Ok(icon) => viewport = viewport.with_icon(std::sync::Arc::new(icon)),
        Err(e) => log::warn!("app icon: {e}"),
    }
    // macOS: the content extends under a transparent title bar; `title_bar.rs` draws it.
    // The window as last closed (scripted runs keep the fixed size).
    if opts.screenshot.is_none()
        && opts.script.is_none()
        && let Some(w) = settings::Settings::load().window
    {
        viewport = viewport.with_inner_size([w.size[0].max(800.0), w.size[1].max(500.0)]);
        if let Some(pos) = w.pos {
            viewport = viewport.with_position(pos);
        }
        viewport = viewport.with_maximized(w.maximized).with_fullscreen(w.fullscreen);
    }
    if cfg!(target_os = "macos") {
        viewport = viewport.with_fullsize_content_view(true).with_titlebar_shown(false).with_title_shown(false);
    }
    let options = eframe::NativeOptions {
        viewport,
        renderer: eframe::Renderer::Wgpu,
        wgpu_options,
        // The app sets its own menu bar (`native_menu.rs`).
        #[cfg(target_os = "macos")]
        event_loop_builder: Some(Box::new(|b| {
            use winit::platform::macos::EventLoopBuilderExtMacOS;
            b.with_default_menu(false);
        })),
        ..Default::default()
    };
    eframe::run_native("PCB Designer", options, Box::new(move |cc| Ok(Box::new(app::PcbApp::new(cc, opts)))))
}

/// `export-gerber`: Gerber and drill files of a project, without a window.
fn export_gerber(dir: &std::path::Path, preset: Option<model::AssemblyPreset>) -> Result<(), String> {
    let mut doc = view::Document::open(dir).map_err(|e| format!("cannot open {}: {e}", dir.display()))?;
    if let Some(preset) = preset {
        doc.execute(editor_core::Command::SetAssemblyPreset { preset }).map_err(|e| e.to_string())?;
    }
    let export = doc.export_gerber().map_err(|e| e.to_string())?;
    for v in &export.warnings {
        eprintln!("warning: {}: {}", v.kind.label(), v.message);
    }
    for f in &export.manifest.files {
        println!("{}", export.dir.join(&f.name).display());
    }
    let assembly = view::gerber::assembly_dir(dir);
    for f in &export.manifest.assembly {
        println!("{}", assembly.join(&f.name).display());
    }
    Ok(())
}

/// `import-kicad`: a KiCad project saved as a project folder, without a window.
fn import_kicad(pro: &std::path::Path, out: &std::path::Path) -> Result<(), String> {
    let import = kicad::import_project(pro)?;
    for line in import.report.lines() {
        eprintln!("{line}");
    }
    let mut doc = view::Document::new(import.project);
    doc.save_as(out).map_err(|e| format!("cannot save {}: {e}", out.display()))?;
    println!("{}", out.display());
    Ok(())
}

/// `export-kicad`: a project as a KiCad project, named after the project.
fn export_kicad(dir: &std::path::Path, out: &std::path::Path) -> Result<(), String> {
    let project = model::io::load_project(dir).map_err(|e| e.to_string())?;
    let export = kicad::export::export_project(&project, out, &project.name)?;
    for line in export.report.lines() {
        eprintln!("{line}");
    }
    println!("{}", out.join(format!("{}.kicad_pro", kicad::export::item_name(&project.name))).display());
    Ok(())
}

/// `simulate`: an analysis of a project run by ngspice, without a window.
fn simulate(args: Vec<String>) -> Result<(), String> {
    const USAGE: &str = "usage: pcb-designer simulate PROJECT_DIR [--analysis NAME] [--csv FILE] [--netlist]";
    let (mut dir, mut name, mut csv, mut netlist_only) = (None, None, None, false);
    let mut args = args.into_iter();
    while let Some(a) = args.next() {
        match a.as_str() {
            "--analysis" => name = Some(args.next().ok_or(USAGE)?),
            "--csv" => csv = Some(args.next().ok_or(USAGE)?),
            "--netlist" => netlist_only = true,
            _ if dir.is_none() && !a.starts_with("--") => dir = Some(a),
            _ => return Err(USAGE.into()),
        }
    }
    let dir = dir.ok_or(USAGE)?;
    let project = model::io::load_project(&dir).map_err(|e| e.to_string())?;
    let setup = &project.schematic.simulation;
    let analysis = match &name {
        Some(n) => setup.find(n).cloned().ok_or_else(|| {
            let names: Vec<&str> = setup.analyses.iter().map(|a| a.name.as_str()).collect();
            format!("no analysis {n:?} (the project has: {})", names.join(", "))
        })?,
        None => setup
            .analyses
            .first()
            .cloned()
            .unwrap_or_else(|| model::Analysis::new("Operating point", model::AnalysisKind::Op)),
    };
    let conn = editor_core::connectivity::Connectivity::build(&project);
    let netlist = sim::netlist::build(&project, &conn, &analysis);
    for p in &netlist.problems {
        let level = match p.level {
            sim::Level::Error => "error",
            sim::Level::Warning => "warning",
            sim::Level::Note => "note",
        };
        eprintln!("{level}: {}", p.message);
    }
    if netlist_only {
        print!("{}", netlist.text);
        return Ok(());
    }
    let engine = sim::Engine::find(&settings::Settings::load().engine());
    let backend = engine.backend(&analysis)?;
    let result =
        sim::simulate(&project, &conn, &analysis, &*backend, &sim::Cancel::default()).map_err(|e| e.to_string())?;
    for w in &result.warnings {
        eprintln!("ngspice: {w}");
    }
    match csv {
        Some(path) => {
            std::fs::write(&path, sim::report::csv(&result)).map_err(|e| format!("cannot write {path}: {e}"))?;
            println!("{path}");
        }
        None => print!("{}", sim::report::text(&result)),
    }
    Ok(())
}

/// Headless MCP server on stdin/stdout.
fn run_stdio(startup: Startup, kicad_paths: Vec<std::path::PathBuf>) -> Result<(), String> {
    use model::Project;
    use view::Document;
    let doc = match startup {
        Startup::Open(path) if path.exists() => {
            Document::open(&path).map_err(|e| format!("cannot open {}: {e}", path.display()))?
        }
        Startup::Open(path) => {
            let name = path.file_stem().map_or("Untitled".into(), |n| n.to_string_lossy().into_owned());
            let mut doc = Document::new(Project::new(&name));
            doc.path = Some(path);
            doc
        }
        Startup::ImportKicad(path) => {
            let import = kicad::import_project(&path)?;
            for line in import.report.lines() {
                log::info!("{line}");
            }
            Document::new(import.project)
        }
        Startup::Example(example) => Document::new(example.project()),
        Startup::Empty => Document::new(Project::new("Untitled")),
        Startup::Stress(n) => Document::new(view::demo::stress_project(n)),
    };
    log::info!("MCP on stdio: {}", doc.title());
    let mut host = mcp::headless::HeadlessHost::new(doc);
    let settings = settings::Settings::load();
    host.kicad_root = settings.kicad_dir();
    host.engine = settings.engine();
    host.kicad_paths = kicad_paths;
    mcp::stdio::serve_stdio(&mut host)
}
