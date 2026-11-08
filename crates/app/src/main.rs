//! PCB Designer desktop application.
//!
//! Usage: pcb-designer [PROJECT_DIR | --demo | --new | --stress N] [--no-vsync]
//!                     [--keymap FILE.json] [--mcp] [--mcp-port PORT]
//!                     [--library FILE.pcblib]... [--show [part:|symbol:|footprint:]NAME]...
//!                     [--screenshot FILE.png [--frames N] [--select all|REFDES]]
//!
//! `--show NAME` opens a part, symbol or footprint of the last opened (or starter)
//! library in its editor tab; repeat it to navigate several steps.
//!
//! Without arguments the demo project is opened.

mod agent;
mod app;
mod canvas;
mod fields;
mod keymap;
mod library;
mod part_form;
mod properties;
mod theme;

use app::{Options, Startup};

fn main() -> eframe::Result {
    env_logger::init();
    let mut opts = Options {
        startup: Startup::Demo,
        screenshot: None,
        screenshot_frames: 30,
        select: None,
        keymap: keymap::Keymap::default(),
        mcp: None,
        library: Vec::new(),
        show: Vec::new(),
    };
    let mut vsync = true;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--demo" => opts.startup = Startup::Demo,
            "--new" => opts.startup = Startup::Empty,
            "--stress" => opts.startup = Startup::Stress(args.next().and_then(|v| v.parse().ok()).unwrap_or(2_000)),
            "--no-vsync" => vsync = false,
            "--screenshot" => opts.screenshot = args.next(),
            "--frames" => opts.screenshot_frames = args.next().and_then(|v| v.parse().ok()).unwrap_or(30),
            "--select" => opts.select = args.next(),
            "--library" => opts.library.extend(args.next().map(Into::into)),
            "--show" => opts.show.extend(args.next()),
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
                        opts.keymap = map;
                    }
                    Err(e) => eprintln!("keymap {path:?} not loaded: {e}"),
                }
            }
            other if !other.starts_with("--") => opts.startup = Startup::Open(other.into()),
            other => eprintln!("ignoring unknown argument {other:?}"),
        }
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
