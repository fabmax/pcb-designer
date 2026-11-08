//! PCB Designer desktop application.
//!
//! Usage: pcb-designer [--components N] [--no-vsync] [--animate] [--storm]
//!                     [--screenshot FILE.png [--frames N]]

mod app;
mod canvas;
mod theme;

fn main() -> eframe::Result {
    env_logger::init();
    let mut opts =
        app::Options { components: 2_000, screenshot: None, screenshot_frames: 30, animate: false, storm: false };
    let mut vsync = true;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--components" => opts.components = args.next().and_then(|v| v.parse().ok()).unwrap_or(opts.components),
            "--no-vsync" => vsync = false,
            "--animate" => opts.animate = true,
            "--storm" => opts.storm = true,
            "--screenshot" => opts.screenshot = args.next(),
            "--frames" => opts.screenshot_frames = args.next().and_then(|v| v.parse().ok()).unwrap_or(30),
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
