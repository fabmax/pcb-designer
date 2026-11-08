//! Main window: menu, side panel with milestone-0 spike controls, docked schematic and PCB
//! canvases, status bar.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use egui::{RichText, WidgetText};
use egui_tiles::{Behavior, SimplificationOptions, TileId, Tiles, Tree, UiResponse};
use render::gpu::Background;
use spike::{GenStats, Project, SyncReport};

use crate::canvas::{Canvas, CanvasResources};
use crate::theme;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Pane {
    Schematic,
    Pcb,
}

pub struct Options {
    pub components: usize,
    /// Save a screenshot of the whole window to this file and exit (development aid).
    pub screenshot: Option<String>,
    /// Frames to render before taking the screenshot.
    pub screenshot_frames: u32,
    /// Start with animated cameras / continuous random edits (benchmarking).
    pub animate: bool,
    pub storm: bool,
}

pub struct PcbApp {
    tree: Tree<Pane>,
    screenshot: Option<(String, u32)>,
    project: Project,
    stats: GenStats,
    components: usize,
    sch: Canvas,
    pcb: Canvas,
    hovered: Option<Pane>,
    // Live sync
    storm: bool,
    storm_per_frame: usize,
    reports: VecDeque<SyncReport>,
    // Rendering performance
    animate: bool,
    anim_t: f64,
    frame_times: VecDeque<f32>,
    last_frame: Option<Instant>,
    regen_time: Duration,
    last_log: Option<Instant>,
    /// Scene bounds at generation time (for the camera animation).
    bounds: [Option<[f64; 4]>; 2],
}

fn ms(d: Duration) -> f32 {
    d.as_secs_f32() * 1000.0
}

impl PcbApp {
    pub fn new(cc: &eframe::CreationContext<'_>, opts: Options) -> Self {
        let components = opts.components;
        theme::apply(&cc.egui_ctx);
        let rs = cc.wgpu_render_state.as_ref().expect("wgpu renderer");
        CanvasResources::install(rs);

        let mut tiles = Tiles::default();
        let sch = tiles.insert_pane(Pane::Schematic);
        let pcb = tiles.insert_pane(Pane::Pcb);
        let root = tiles.insert_horizontal_tile(vec![sch, pcb]);
        let tree = Tree::new(egui::Id::unique("main_tree"), root, tiles);

        let t = Instant::now();
        let (project, stats) = Project::generate(components, 1);
        let regen_time = t.elapsed();

        Self {
            tree,
            screenshot: opts.screenshot.map(|p| (p, opts.screenshot_frames)),
            project,
            stats,
            components,
            sch: Canvas::new(1, Background { grid: 2.54, ..Default::default() }),
            pcb: Canvas::new(2, Background { grid: 0.5, ..Default::default() }),
            hovered: None,
            storm: opts.storm,
            storm_per_frame: 1,
            reports: VecDeque::new(),
            animate: opts.animate,
            anim_t: 0.0,
            frame_times: VecDeque::new(),
            last_frame: None,
            regen_time,
            last_log: None,
            bounds: [None; 2],
        }
    }

    fn regenerate(&mut self) {
        let t = Instant::now();
        let (project, stats) = Project::generate(self.components, 1);
        self.regen_time = t.elapsed();
        self.project = project;
        self.stats = stats;
        self.reports.clear();
        self.bounds = [None; 2];
        self.sch.request_fit();
        self.pcb.request_fit();
    }

    fn apply_random_edits(&mut self, n: usize) {
        for _ in 0..n {
            let e = self.project.random_edit();
            let r = self.project.apply(e);
            self.reports.push_back(r);
            if self.reports.len() > 240 {
                self.reports.pop_front();
            }
        }
    }

    fn animate_cameras(&mut self, dt: f64) {
        self.anim_t += dt;
        let t = self.anim_t;
        let canvases = [(&mut self.sch, &self.project.sch_scene), (&mut self.pcb, &self.project.pcb_scene)];
        for (i, (canvas, scene)) in canvases.into_iter().enumerate() {
            let b = *self.bounds[i].get_or_insert_with(|| scene.bounds().unwrap_or([0.0, 0.0, 1.0, 1.0]));
            canvas.camera.fit(b, canvas.viewport(), 24.0);
            let zoom = canvas.camera.zoom * 2f64.powf((1.0 - (t * 0.6).cos()) * 2.5);
            canvas.camera.zoom = zoom;
            canvas.camera.center = [
                b[0] + (b[2] - b[0]) * (0.5 + 0.4 * (t * 0.23).sin()),
                b[1] + (b[3] - b[1]) * (0.5 + 0.4 * (t * 0.17).cos()),
            ];
        }
    }

    fn record_frame(&mut self) -> f64 {
        let now = Instant::now();
        let dt = self.last_frame.map(|l| (now - l).as_secs_f64()).unwrap_or(0.0);
        self.last_frame = Some(now);
        if dt > 0.0 {
            self.frame_times.push_back(dt as f32);
            if self.frame_times.len() > 120 {
                self.frame_times.pop_front();
            }
        }
        dt
    }

    fn side_panel(&mut self, ui: &mut egui::Ui, cpu: Option<f32>) {
        ui.add_space(6.0);
        ui.label(RichText::new("Milestone 0 · performance spike").strong().size(15.0));
        ui.label(RichText::new("Synthetic project, live schematic -> PCB sync").color(theme::TEXT_DIM));
        ui.separator();

        section(ui, "Synthetic project", |ui| {
            ui.horizontal(|ui| {
                ui.label("Components");
                ui.add(egui::DragValue::new(&mut self.components).range(10..=100_000).speed(50));
            });
            ui.horizontal(|ui| {
                for n in [1_000, 10_000, 20_000] {
                    if ui.button(format!("{}k", n / 1000)).clicked() {
                        self.components = n;
                        self.regenerate();
                    }
                }
                if ui.button("Regenerate").clicked() {
                    self.regenerate();
                }
            });
            let s = &self.stats;
            egui::Grid::new("stats").num_columns(2).spacing([16.0, 2.0]).show(ui, |ui| {
                let row = |ui: &mut egui::Ui, k: &str, v: String| {
                    ui.label(RichText::new(k).color(theme::TEXT_DIM));
                    ui.label(v);
                    ui.end_row();
                };
                row(ui, "Components", self.project.sch.components.len().to_string());
                row(ui, "Pins / wires", format!("{} / {}", s.pins, self.project.sch.wires.len()));
                row(ui, "Labels", self.project.sch.labels.len().to_string());
                row(ui, "Nets", self.project.conn.net_count().to_string());
                row(ui, "Tracks / vias", format!("{} / {}", s.tracks, s.vias));
                row(ui, "Schematic prims", fmt_k(self.project.sch_scene.instance_count()));
                row(ui, "PCB prims", fmt_k(self.project.pcb_scene.instance_count()));
                row(ui, "Generated in", format!("{:.0} ms", ms(self.regen_time)));
            });
        });

        section(ui, "Live sync", |ui| {
            ui.horizontal(|ui| {
                if ui.button("Random edit").clicked() {
                    self.apply_random_edits(1);
                }
                ui.toggle_value(&mut self.storm, "Edit storm");
                ui.add_enabled(
                    self.storm,
                    egui::DragValue::new(&mut self.storm_per_frame).range(1..=50).suffix(" /frame"),
                );
            });
            if let Some(r) = self.reports.back() {
                ui.label(format!("Last: {} — {:.2} ms", r.kind, ms(r.total)));
            }
            if !self.reports.is_empty() {
                let n = self.reports.len() as f32;
                let avg = |f: fn(&SyncReport) -> Duration| self.reports.iter().map(|r| ms(f(r))).sum::<f32>() / n;
                let max = self.reports.iter().map(|r| ms(r.total)).fold(0.0, f32::max);
                egui::Grid::new("sync").num_columns(2).spacing([16.0, 2.0]).show(ui, |ui| {
                    let row = |ui: &mut egui::Ui, k: &str, v: f32| {
                        ui.label(RichText::new(k).color(theme::TEXT_DIM));
                        ui.label(format!("{v:.3} ms"));
                        ui.end_row();
                    };
                    row(ui, "Total (avg)", avg(|r| r.total));
                    row(ui, "Total (max)", max);
                    row(ui, "Schematic", avg(|r| r.schematic));
                    row(ui, "Connectivity", avg(|r| r.connectivity));
                    row(ui, "Board", avg(|r| r.board));
                    row(ui, "Ratsnest", avg(|r| r.ratsnest));
                });
                ui.label(RichText::new(format!("over last {} edits", self.reports.len())).color(theme::TEXT_DIM));
            }
        });

        section(ui, "Rendering", |ui| {
            ui.toggle_value(&mut self.animate, "Animate cameras (benchmark)");
            let (avg, max) = frame_stats(&self.frame_times);
            ui.label(format!("{:.0} fps · frame {:.1} ms avg / {:.1} ms max", 1000.0 / avg.max(0.001), avg, max));
            if let Some(cpu) = cpu {
                ui.label(format!("CPU per frame: {:.2} ms", cpu * 1000.0));
            }
            ui.label(RichText::new("Run with --no-vsync to measure uncapped fps.").color(theme::TEXT_DIM));
        });

        section(ui, "PCB layers", |ui| layer_toggles(ui, &mut self.project.pcb_scene));
        section(ui, "Schematic layers", |ui| layer_toggles(ui, &mut self.project.sch_scene));
    }

    fn status_bar(&self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            let (name, canvas) = match self.hovered {
                Some(Pane::Schematic) => ("Schematic", Some(&self.sch)),
                Some(Pane::Pcb) => ("PCB", Some(&self.pcb)),
                None => ("", None),
            };
            if let Some(c) = canvas {
                if let Some([x, y]) = c.cursor_world {
                    ui.monospace(format!("{name}  X {x:>9.3} mm  Y {y:>9.3} mm"));
                }
                ui.separator();
                ui.monospace(format!("zoom {:.2} px/mm", c.camera.zoom));
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let (avg, _) = frame_stats(&self.frame_times);
                ui.monospace(format!("{:.0} fps", 1000.0 / avg.max(0.001)));
                ui.separator();
                ui.label(RichText::new("Drag: pan · Wheel/pinch: zoom · Double-click/F: fit").color(theme::TEXT_DIM));
            });
        });
    }
}

impl PcbApp {
    /// Print frame and sync statistics every two seconds while benchmarking.
    fn log_perf(&mut self) {
        if !(self.animate || self.storm) {
            return;
        }
        let now = Instant::now();
        if self.last_log.is_some_and(|t| now - t < Duration::from_secs(2)) {
            return;
        }
        self.last_log = Some(now);
        let (avg, max) = frame_stats(&self.frame_times);
        let sync_max = self.reports.iter().map(|r| ms(r.total)).fold(0.0, f32::max);
        println!(
            "fps {:.0}  frame avg {:.2} ms  max {:.2} ms  | sync max {:.2} ms (last {} edits)",
            1000.0 / avg.max(0.001),
            avg,
            max,
            sync_max,
            self.reports.len()
        );
    }

    fn screenshot_tick(&mut self, ctx: &egui::Context) {
        let Some((path, frames)) = &mut self.screenshot else {
            return;
        };
        ctx.request_repaint();
        if *frames > 0 {
            *frames -= 1;
            return;
        }
        let path = path.clone();
        self.screenshot = None;
        ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::ScreenshotCallback::new(move |img| {
            let rgba = img.pixels.iter().flat_map(|c| c.to_srgba_unmultiplied()).collect();
            let image = render::offscreen::Image { width: img.size[0] as u32, height: img.size[1] as u32, rgba };
            match image.save_png(&path) {
                Ok(()) => println!("screenshot saved to {path}"),
                Err(e) => eprintln!("screenshot failed: {e}"),
            }
            std::process::exit(0);
        })));
    }
}

fn fmt_k(n: usize) -> String {
    if n >= 1_000_000 {
        format!("{:.2} M", n as f64 / 1e6)
    } else if n >= 1_000 {
        format!("{:.1} k", n as f64 / 1e3)
    } else {
        n.to_string()
    }
}

fn frame_stats(times: &VecDeque<f32>) -> (f32, f32) {
    if times.is_empty() {
        return (0.0, 0.0);
    }
    let avg = times.iter().sum::<f32>() / times.len() as f32 * 1000.0;
    let max = times.iter().copied().fold(0.0, f32::max) * 1000.0;
    (avg, max)
}

fn section(ui: &mut egui::Ui, title: &str, add: impl FnOnce(&mut egui::Ui)) {
    egui::CollapsingHeader::new(RichText::new(title).strong()).default_open(true).show(ui, add);
    ui.add_space(4.0);
}

fn layer_toggles(ui: &mut egui::Ui, scene: &mut render::Scene) {
    for id in 0..scene.layer_count() as u16 {
        let layer = scene.layer_mut(id);
        let name = layer.name.clone();
        ui.checkbox(&mut layer.visible, name);
    }
}

struct TreeBehavior<'a> {
    project: &'a mut Project,
    sch: &'a mut Canvas,
    pcb: &'a mut Canvas,
    hovered: &'a mut Option<Pane>,
}

impl Behavior<Pane> for TreeBehavior<'_> {
    fn tab_title_for_pane(&mut self, pane: &Pane) -> WidgetText {
        match pane {
            Pane::Schematic => "Schematic".into(),
            Pane::Pcb => "PCB".into(),
        }
    }

    fn pane_ui(&mut self, ui: &mut egui::Ui, _tile_id: TileId, pane: &mut Pane) -> UiResponse {
        let response = match pane {
            Pane::Schematic => self.sch.ui(ui, &mut self.project.sch_scene),
            Pane::Pcb => self.pcb.ui(ui, &mut self.project.pcb_scene),
        };
        if response.hovered() {
            *self.hovered = Some(*pane);
        }
        UiResponse::None
    }

    fn simplification_options(&self) -> SimplificationOptions {
        SimplificationOptions { all_panes_must_have_tabs: true, ..Default::default() }
    }

    fn gap_width(&self, _style: &egui::Style) -> f32 {
        2.0
    }

    fn tab_bar_color(&self, _visuals: &egui::Visuals) -> egui::Color32 {
        theme::PANEL_DARK
    }
}

impl eframe::App for PcbApp {
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let dt = self.record_frame();
        let ctx = ui.ctx().clone();

        if self.storm {
            self.apply_random_edits(self.storm_per_frame);
        }
        if self.animate {
            self.animate_cameras(dt.min(0.1));
        }
        if ctx.input(|i| i.key_pressed(egui::Key::F)) && !ctx.egui_wants_keyboard_input() {
            self.sch.request_fit();
            self.pcb.request_fit();
        }

        egui::Panel::top("menu").show(ui, |ui| {
            egui::MenuBar::new().ui(ui, |ui| {
                ui.menu_button("File", |ui| {
                    if ui.button("Quit").clicked() {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                });
                ui.menu_button("View", |ui| {
                    if ui.button("Fit all (F)").clicked() {
                        self.sch.request_fit();
                        self.pcb.request_fit();
                    }
                });
            });
        });

        let cpu = frame.info().cpu_usage;
        egui::Panel::bottom("status").show(ui, |ui| self.status_bar(ui));
        egui::Panel::left("side").resizable(true).default_size(290.0).show(ui, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| self.side_panel(ui, cpu));
        });

        self.hovered = None;
        egui::CentralPanel::default().frame(egui::Frame::NONE).show(ui, |ui| {
            let mut behavior = TreeBehavior {
                project: &mut self.project,
                sch: &mut self.sch,
                pcb: &mut self.pcb,
                hovered: &mut self.hovered,
            };
            self.tree.ui(&mut behavior, ui);
        });

        if self.storm || self.animate {
            ctx.request_repaint();
        }
        self.screenshot_tick(&ctx);
        self.log_perf();
    }
}
