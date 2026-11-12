//! Canvas widget: hosts a GPU rendered [`Scene`] inside egui and handles navigation.

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
    rect: egui::Rect,
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
            rect: egui::Rect::NOTHING,
        }
    }

    pub fn request_fit(&mut self) {
        self.fit_requested = true;
    }

    /// Canvas size in points.
    pub fn viewport(&self) -> [f64; 2] {
        self.viewport
    }

    pub fn fit(&mut self, scene: &Scene) {
        if let Some(b) = scene.bounds() {
            self.camera.fit(b, self.viewport, 32.0);
        }
    }

    pub fn screen_to_world(&self, p: egui::Pos2) -> [f64; 2] {
        self.camera.screen_to_world([(p.x - self.rect.min.x) as f64, (p.y - self.rect.min.y) as f64], self.viewport)
    }

    pub fn world_to_screen(&self, w: [f64; 2]) -> egui::Pos2 {
        let [x, y] = self.camera.world_to_screen(w, self.viewport);
        egui::pos2(self.rect.min.x + x as f32, self.rect.min.y + y as f32)
    }

    /// Draw the scene and handle navigation: middle/right drag and trackpad scroll pan,
    /// mouse wheel and pinch zoom. The primary button is left to the editing tools.
    pub fn ui(&mut self, ui: &mut egui::Ui, scene: &mut Scene) -> egui::Response {
        let (rect, response) = ui.allocate_exact_size(ui.available_size(), egui::Sense::click_and_drag());
        self.rect = rect;
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

        if response.dragged_by(egui::PointerButton::Middle) || response.dragged_by(egui::PointerButton::Secondary) {
            let d = response.drag_delta();
            self.camera.pan_pixels([d.x as f64, d.y as f64]);
        }
        if response.double_clicked_by(egui::PointerButton::Middle) {
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

/// Primary-button pointer state over a canvas, in world coordinates, for the editing
/// interaction. `engaged`: an interaction is in progress (keeps receiving input when the
/// pointer leaves the canvas). `None` when there is nothing to report.
pub fn pointer_input(
    ui: &egui::Ui,
    response: &egui::Response,
    canvas: &Canvas,
    engaged: bool,
    hovering: bool,
) -> Option<view::PointerInput> {
    use model::{Coord, Point};
    if !(response.hovered() || engaged || hovering) {
        return None;
    }
    let px_to_nm = |px: f64| (px / canvas.camera.zoom * 1e6) as Coord;
    Some(ui.input(|i| {
        let pos = i.pointer.latest_pos().filter(|_| response.hovered() || engaged);
        view::PointerInput {
            world: pos.map(|p| {
                let [x, y] = canvas.screen_to_world(p);
                Point::mm(x, y)
            }),
            pressed: response.hovered() && i.pointer.primary_pressed(),
            released: i.pointer.primary_released(),
            down: i.pointer.primary_down(),
            shift: i.modifiers.shift,
            command: i.modifiers.command,
            tolerance: px_to_nm(5.0),
            drag_threshold: px_to_nm(4.0),
        }
    }))
}

/// Draw the interaction's selection box: solid blue for "inside", dashed green for
/// "crossing".
pub fn draw_box(ui: &egui::Ui, canvas: &Canvas, clip: egui::Rect, r: model::Rect, crossing: bool) {
    use crate::theme;
    let a = canvas.world_to_screen(r.min.to_mm());
    let b = canvas.world_to_screen(r.max.to_mm());
    let rect = egui::Rect::from_two_pos(a, b);
    let color = if crossing { theme::success() } else { theme::accent() };
    let painter = ui.painter_at(clip);
    painter.rect_filled(rect, 0.0, color.gamma_multiply(0.12));
    if crossing {
        let pts = [rect.left_top(), rect.right_top(), rect.right_bottom(), rect.left_bottom(), rect.left_top()];
        painter.extend(egui::Shape::dashed_line(&pts, egui::Stroke::new(1.0, color), 6.0, 4.0));
    } else {
        painter.rect_stroke(rect, 0.0, egui::Stroke::new(1.0, color), egui::StrokeKind::Inside);
    }
}

/// The measure tool's overlay: the snap target ring, the ruler with crosses at its ends
/// and a label with length, ΔX, ΔY and angle in the current unit.
pub fn draw_measure(ui: &egui::Ui, canvas: &Canvas, clip: egui::Rect, m: &view::measure::Measure) {
    use egui::{FontId, Stroke, vec2};
    let painter = ui.painter_at(clip);
    let accent = crate::theme::accent();
    let screen = |p: model::Point| canvas.world_to_screen([p.x as f64 * 1e-6, p.y as f64 * 1e-6]);
    if let Some(t) = m.target {
        painter.circle_stroke(screen(t), 6.0, Stroke::new(1.5, accent));
    }
    let (Some((a, b)), Some(r)) = (m.segment(), m.reading()) else { return };
    let (sa, sb) = (screen(a), screen(b));
    let stroke = Stroke::new(2.0, accent);
    painter.line_segment([sa, sb], stroke);
    for c in [sa, sb] {
        painter.line_segment([c - vec2(5.0, 5.0), c + vec2(5.0, 5.0)], stroke);
        painter.line_segment([c - vec2(5.0, -5.0), c + vec2(5.0, -5.0)], stroke);
    }
    let text = format!(
        "{}\nΔX {}   ΔY {}   {:.1}°",
        crate::units::fmt(r.length.round() as model::Coord),
        crate::units::fmt(r.dx),
        crate::units::fmt(r.dy),
        r.angle
    );
    let font = FontId::proportional(12.0);
    let galley = painter.layout_no_wrap(text, font, ui.visuals().text_color());
    // Beside the end, away from the ruler.
    let away = if sb.x >= sa.x { 14.0 } else { -14.0 - galley.size().x };
    let mut pos = sb + vec2(away, 10.0);
    // Kept inside the canvas.
    let size = galley.size();
    let inner = clip.shrink(8.0);
    if pos.x + size.x > inner.right() {
        pos.x = sb.x - 14.0 - size.x;
    }
    if pos.y + size.y > inner.bottom() {
        pos.y = sb.y - 10.0 - size.y;
    }
    pos = pos.max(inner.min);
    let bg = egui::Rect::from_min_size(pos, galley.size()).expand(5.0);
    painter.rect(bg, 4.0, crate::theme::panel(), Stroke::new(1.0, accent), egui::StrokeKind::Inside);
    painter.galley(pos, galley, ui.visuals().text_color());
}
