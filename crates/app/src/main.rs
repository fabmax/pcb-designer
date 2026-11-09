//! PCB Designer desktop application.
//!
//! Usage: pcb-designer [PROJECT_DIR | --demo | --new | --stress N] [--no-vsync]
//!                     [--keymap FILE.json] [--mcp] [--mcp-port PORT] [--side-by-side]
//!                     [--library FILE.pcblib]... [--show [part:|symbol:|footprint:]NAME]...
//!                     [--kicad DIR]... [--kicad-pick [footprint:]LIB:NAME]... [--kicad-import]
//!                     [--kicad-search [footprint:]TEXT]
//!                     [--screenshot FILE.png [--frames N] [--select all|REFDES]]
//!        pcb-designer --mcp-stdio [PROJECT_DIR | --demo]
//!
//! `--mcp-stdio` runs without a window: an MCP server on stdin/stdout for agents. A
//! project folder that doesn't exist yet becomes a new project saved there.
//!
//! `--show NAME` opens a part, symbol or footprint of the last opened (or starter)
//! library in its editor tab (repeated: the last one is shown).
//!
//! `--kicad DIR` sets the KiCad library folders (default: the KiCad installation);
//! `--kicad-pick` adds symbols/footprints to the KiCad import window, `--kicad-import`
//! imports them right away into a new library.
//!
//! Without arguments the demo project is opened.

mod agent;
mod app;
mod canvas;
mod fields;
mod icons;
mod keymap;
mod kicad_import;
mod library;
mod part_form;
mod pcb_ui;
mod properties;
mod sch_ui;
mod script;
mod settings;
mod theme;
mod toolbar;
mod units;

use app::{Options, Startup};

fn main() -> eframe::Result {
    env_logger::init();
    let mut opts = Options {
        startup: Startup::Demo,
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
        script: None,
        side_by_side: false,
    };
    let mut vsync = true;
    let mut stdio = false;
    let mut startup_given = false;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--demo" => (opts.startup, startup_given) = (Startup::Demo, true),
            "--new" => (opts.startup, startup_given) = (Startup::Empty, true),
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
            other if !other.starts_with("--") => (opts.startup, startup_given) = (Startup::Open(other.into()), true),
            other => eprintln!("ignoring unknown argument {other:?}"),
        }
    }

    if stdio {
        let startup = if startup_given { opts.startup } else { Startup::Empty };
        if let Err(e) = run_stdio(startup) {
            eprintln!("pcb-designer: {e}");
            std::process::exit(1);
        }
        return Ok(());
    }

    let mut wgpu_options = egui_wgpu::WgpuConfiguration::default();
    if !vsync {
        wgpu_options.surface.present_mode = wgpu::PresentMode::AutoNoVsync;
    }
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("PCB Designer")
            .with_inner_size([1600.0, 1000.0])
            .with_min_inner_size([800.0, 500.0]),
        renderer: eframe::Renderer::Wgpu,
        wgpu_options,
        ..Default::default()
    };
    eframe::run_native("PCB Designer", options, Box::new(move |cc| Ok(Box::new(app::PcbApp::new(cc, opts)))))
}

/// Headless MCP server on stdin/stdout.
fn run_stdio(startup: Startup) -> Result<(), String> {
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
        Startup::Demo => Document::new(view::demo::demo_project()),
        Startup::Empty => Document::new(Project::new("Untitled")),
        Startup::Stress(n) => Document::new(view::demo::stress_project(n)),
    };
    log::info!("MCP on stdio: {}", doc.title());
    let mut host = mcp::headless::HeadlessHost::new(doc);
    mcp::stdio::serve_stdio(&mut host)
}
