//! Canvas widget: hosts a GPU rendered [`Scene`] inside egui and handles pan/zoom.

use std::sync::{Arc, Mutex};

use egui_wgpu::{CallbackResources, CallbackTrait, ScreenDescriptor};
use render::gpu::{Background, GpuScene, Renderer, ViewUniform};
use render::{Camera, Scene, SceneUpload};

/// Shared GPU state, stored in egui-wgpu's callback resources.
pub struct CanvasResources {
    renderer: Renderer,
    scenes: Vec<(u64, GpuScene)>,
}

impl CanvasResources {
    pub fn install(rs: &egui_wgpu::RenderState) {
        let renderer = Renderer::new(&rs.device, rs.target_format);
        rs.renderer.write().callback_resources.insert(CanvasResources { renderer, scenes: Vec::new() });
    }
}

pub struct Canvas {
    id: u64,
    pub camera: Camera,
    pub background: Background,
    /// Uploads not yet consumed by the GPU (applied in order during `prepare`).
    pending: Arc<Mutex<Vec<SceneUpload>>>,
    fit_requested: bool,
    pub cursor_world: Option<[f64; 2]>,
    viewport: [f64; 2],
}

impl Canvas {
    pub fn new(id: u64, background: Background) -> Self {
        Self {
            id,
            camera: Camera::default(),
            background,
            pending: Default::default(),
            fit_requested: true,
            cursor_world: None,
            viewport: [1.0, 1.0],
        }
    }

    pub fn request_fit(&mut self) {
        self.fit_requested = true;
    }

    pub fn viewport(&self) -> [f64; 2] {
        self.viewport
    }

    pub fn fit(&mut self, scene: &Scene) {
        if let Some(b) = scene.bounds() {
            self.camera.fit(b, self.viewport, 24.0);
        }
    }

    pub fn ui(&mut self, ui: &mut egui::Ui, scene: &mut Scene) -> egui::Response {
        let (rect, response) = ui.allocate_exact_size(ui.available_size(), egui::Sense::click_and_drag());
        self.viewport = [rect.width() as f64, rect.height() as f64];
        if self.fit_requested && rect.width() > 1.0 {
            self.fit(scene);
            self.fit_requested = false;
        }
        self.handle_input(ui, &response, rect);

        let upload = scene.take_upload();
        if !upload.is_empty() {
            self.pending.lock().unwrap().push(upload);
        }
        let ppp = ui.ctx().pixels_per_point();
        let view = ViewUniform::new(&self.camera, [rect.width(), rect.height()], ppp, &self.background);
        let callback = CanvasCallback {
            id: self.id,
            view,
            pending: self.pending.clone(),
            visible: scene.visible_layers(self.camera.zoom),
        };
        ui.painter().add(egui_wgpu::Callback::new_paint_callback(rect, callback));
        response
    }

    fn handle_input(&mut self, ui: &egui::Ui, response: &egui::Response, rect: egui::Rect) {
        let vp = self.viewport;
        let local = |p: egui::Pos2| [(p.x - rect.min.x) as f64, (p.y - rect.min.y) as f64];

        // Drag with any button pans (there are no editing tools yet).
        if response.dragged() {
            let d = response.drag_delta();
            self.camera.pan_pixels([d.x as f64, d.y as f64]);
        }
        if response.double_clicked() {
            self.fit_requested = true;
        }

        let hover = response.hover_pos();
        self.cursor_world = hover.map(|p| self.camera.screen_to_world(local(p), vp));
        let Some(hover) = hover else { return };
        let anchor = local(hover);

        ui.input(|i| {
            for e in &i.events {
                if let egui::Event::MouseWheel { unit, delta, source, modifiers, .. } = e {
                    if modifiers.command || modifiers.ctrl {
                        continue; // handled as zoom_delta by egui
                    }
                    let is_wheel = match source {
                        egui::MouseWheelSource::Wheel => true,
                        egui::MouseWheelSource::Trackpad | egui::MouseWheelSource::Momentum => false,
                        egui::MouseWheelSource::Unknown => *unit != egui::MouseWheelUnit::Point,
                    };
                    if is_wheel {
                        // Mouse wheel zooms around the cursor, like most CAD tools.
                        let steps = match unit {
                            egui::MouseWheelUnit::Point => delta.y as f64 / 50.0,
                            egui::MouseWheelUnit::Line => delta.y as f64,
                            egui::MouseWheelUnit::Page => delta.y as f64 * 10.0,
                        };
                        self.camera.zoom_at(1.18f64.powf(steps), anchor, vp);
                    } else {
                        // Two-finger trackpad scroll pans.
                        self.camera.pan_pixels([delta.x as f64, delta.y as f64]);
                    }
                }
            }
            let z = i.zoom_delta();
            if z != 1.0 {
                self.camera.zoom_at(z as f64, anchor, vp);
            }
        });
    }
}

struct CanvasCallback {
    id: u64,
    view: ViewUniform,
    pending: Arc<Mutex<Vec<SceneUpload>>>,
    visible: Vec<bool>,
}

impl CallbackTrait for CanvasCallback {
    fn prepare(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        _screen: &ScreenDescriptor,
        _encoder: &mut wgpu::CommandEncoder,
        resources: &mut CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        let res: &mut CanvasResources = resources.get_mut().expect("canvas resources installed");
        let idx = match res.scenes.iter().position(|(id, _)| *id == self.id) {
            Some(i) => i,
            None => {
                res.scenes.push((self.id, GpuScene::new(device, &res.renderer)));
                res.scenes.len() - 1
            }
        };
        let gpu = &mut res.scenes[idx].1;
        for upload in self.pending.lock().unwrap().drain(..) {
            gpu.upload(device, queue, &upload);
        }
        gpu.set_view(queue, &self.view);
        Vec::new()
    }

    fn paint(
        &self,
        _info: egui::PaintCallbackInfo,
        pass: &mut wgpu::RenderPass<'static>,
        resources: &CallbackResources,
    ) {
        let res: &CanvasResources = resources.get().expect("canvas resources installed");
        if let Some((_, gpu)) = res.scenes.iter().find(|(id, _)| *id == self.id) {
            gpu.draw(&res.renderer, pass, &self.visible);
        }
    }
}
