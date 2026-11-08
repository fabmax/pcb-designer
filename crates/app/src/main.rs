//! PCB Designer desktop application.
//!
//! Usage: pcb-designer [PROJECT_DIR | --demo | --new | --stress N] [--no-vsync]
//!                     [--screenshot FILE.png [--frames N] [--select-all]]
//!
//! Without arguments the demo project is opened.

mod app;
mod canvas;
mod properties;
mod theme;

use app::{Options, Startup};

fn main() -> eframe::Result {
    env_logger::init();
    let mut opts = Options { startup: Startup::Demo, screenshot: None, screenshot_frames: 30, select_all: false };
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
            "--select-all" => opts.select_all = true,
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
