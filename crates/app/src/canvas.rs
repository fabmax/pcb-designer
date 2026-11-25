//! Canvas widget: hosts a GPU rendered [`Scene`] inside egui and handles navigation.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use egui_wgpu::{CallbackResources, CallbackTrait, ScreenDescriptor};
use render::gpu::{Background, GpuScene, Renderer, ViewUniform};
use render::{Camera, Scene, SceneUpload};

/// Ids of dropped canvases whose GPU scenes are freed by the next `prepare`.
static RELEASED: Mutex<Vec<u64>> = Mutex::new(Vec::new());
static NEXT_ID: AtomicU64 = AtomicU64::new(1);
/// Canvases draw a crosshair at the pointer (View → Crosshair).
static CROSSHAIR: AtomicBool = AtomicBool::new(false);

pub fn set_crosshair(on: bool) {
    CROSSHAIR.store(on, Ordering::Relaxed);
}

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

/// A GPU canvas. Each has its own GPU scene, freed when the canvas is dropped.
pub struct Canvas {
    id: u64,
    pub camera: Camera,
    pub background: Background,
    /// The editor whose theme it shows (the crosshair's colour; callers set the
    /// background, see [`Canvas::restyle`]).
    pub editor: view::style::Editor,
    /// Uploads not yet consumed by the GPU (applied in order during `prepare`).
    pending: Arc<Mutex<Vec<SceneUpload>>>,
    fit_requested: bool,
    pub cursor_world: Option<[f64; 2]>,
    viewport: [f64; 2],
    rect: egui::Rect,
    /// Cmd+wheel (Ctrl elsewhere) steps something (the Route tool's width) instead of
    /// zooming; read them with [`Canvas::take_command_wheel`]. Set by the caller each
    /// frame before [`Canvas::ui`].
    pub command_wheel: bool,
    wheel_steps: f64,
    /// The last mouse wheel notch counted as a step: egui's time and direction.
    last_notch: Option<(f64, f64)>,
    /// A short note next to the pointer ([`Canvas::flash`]) and when it was set.
    tip: Option<(String, std::time::Instant)>,
}

/// How long a [`Canvas::flash`] note stays.
const TIP_TIME: std::time::Duration = std::time::Duration::from_millis(1200);

impl Canvas {
    /// A canvas of `editor` with a grid of `grid` world units.
    pub fn new(editor: view::style::Editor, grid: f32) -> Self {
        let background = view::style::with_editor(editor, || view::style::background(grid));
        Self {
            id: NEXT_ID.fetch_add(1, Ordering::Relaxed),
            camera: Camera::default(),
            background,
            editor,
            pending: Default::default(),
            fit_requested: true,
            cursor_world: None,
            viewport: [1.0, 1.0],
            rect: egui::Rect::NOTHING,
            command_wheel: false,
            wheel_steps: 0.0,
            last_notch: None,
            tip: None,
        }
    }

    /// Show `editor`'s theme: its background (keeping the grid).
    pub fn restyle(&mut self, editor: view::style::Editor) {
        self.editor = editor;
        self.background = view::style::with_editor(editor, || view::style::background(self.background.grid));
    }

    /// Whole Cmd+wheel steps since the last call (up: positive), with
    /// [`Canvas::command_wheel`] on. A trackpad's small deltas add up to steps.
    pub fn take_command_wheel(&mut self) -> i32 {
        let whole = self.wheel_steps.trunc();
        self.wheel_steps -= whole;
        whole as i32
    }

    /// Show `text` next to the pointer for a moment.
    pub fn flash(&mut self, text: String) {
        self.tip = Some((text, std::time::Instant::now()));
    }

    fn tip_ui(&mut self, ui: &egui::Ui, response: &egui::Response) {
        let Some((text, since)) = &self.tip else { return };
        let left = TIP_TIME.saturating_sub(since.elapsed());
        if left.is_zero() || !response.hovered() {
            self.tip = None;
            return;
        }
        ui.ctx().request_repaint_after(left);
        egui::Tooltip::always_open(
            ui.ctx().clone(),
            ui.layer_id(),
            egui::Id::unique("canvas_flash"),
            egui::PopupAnchor::Pointer,
        )
        .gap(14.0)
        .show(|ui| ui.label(text.as_str()));
    }

    pub fn request_fit(&mut self) {
        self.fit_requested = true;
    }

    /// Show this part of the world (a restored camera): no fit to the content.
    pub fn set_camera(&mut self, camera: Camera) {
        self.camera = camera;
        self.fit_requested = false;
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
        self.paint(ui, rect, scene);
        self.tip_ui(ui, &response);
        if CROSSHAIR.load(Ordering::Relaxed)
            && let Some(at) = response.hover_pos()
        {
            let color = view::style::palette(view::style::editor_theme(self.editor)).crosshair;
            draw_crosshair(ui, rect, at, crate::theme::color(color));
        }
        response
    }

    /// Draw the scene into `rect` without handling input.
    pub fn paint(&mut self, ui: &egui::Ui, rect: egui::Rect, scene: &mut Scene) {
        self.rect = rect;
        self.viewport = [rect.width() as f64, rect.height() as f64];
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
                    if modifiers.command && self.command_wheel {
                        self.wheel_steps += match (source, unit) {
                            // macOS sends an event per notch of a mouse wheel, but with
                            // accelerated deltas (a slow notch is a fraction of a line):
                            // count the notches.
                            (egui::MouseWheelSource::Wheel, _) => {
                                let sign = delta.y.signum() as f64;
                                if delta.y != 0.0 && notch_counts(&mut self.last_notch, i.time, sign) {
                                    sign
                                } else {
                                    0.0
                                }
                            }
                            (_, egui::MouseWheelUnit::Point) => delta.y as f64 / 50.0,
                            (_, egui::MouseWheelUnit::Line) => delta.y as f64,
                            (_, egui::MouseWheelUnit::Page) => delta.y as f64 * 10.0,
                        };
                        continue;
                    }
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
            // egui counts Cmd+wheel into the zoom, smoothed over the following frames:
            // while Cmd+wheel steps instead, only pinches zoom.
            let z = if self.command_wheel {
                let pinch = i.events.iter().map(|e| if let egui::Event::Zoom(f) = e { *f } else { 1.0 }).product();
                i.multi_touch().map_or(pinch, |t| t.zoom_delta)
            } else {
                i.zoom_delta()
            };
            if z != 1.0 {
                self.camera.zoom_at(z as f64, anchor, vp);
            }
        });
    }
}

/// Some mouse drivers send a burst of events per wheel notch: in one direction, a notch
/// counts at most every [`NOTCH_GAP`] seconds (still 25 steps a second when spun fast).
/// `last` is the last counted notch's time and direction.
fn notch_counts(last: &mut Option<(f64, f64)>, time: f64, sign: f64) -> bool {
    const NOTCH_GAP: f64 = 0.04;
    let counts = last.is_none_or(|(t, s)| s != sign || time - t >= NOTCH_GAP);
    if counts {
        *last = Some((time, sign));
    }
    counts
}

impl Drop for Canvas {
    fn drop(&mut self) {
        RELEASED.lock().unwrap().push(self.id);
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
        let released = std::mem::take(&mut *RELEASED.lock().unwrap());
        if !released.is_empty() {
            res.scenes.retain(|(id, _)| !released.contains(id));
        }
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
            alt: i.modifiers.alt,
            tolerance: px_to_nm(5.0),
            drag_threshold: px_to_nm(4.0),
            screen: pos.map(|p| [p.x as f64, p.y as f64]),
            nm_per_point: 1e6 / canvas.camera.zoom,
        }
    }))
}

/// The crosshair's look (screen points): the hole around the pointer, and the dashes.
const CROSSHAIR_HOLE: f32 = 24.0;
const CROSSHAIR_DASH: f32 = 16.0;
const CROSSHAIR_GAP: f32 = 24.0;

/// Crosshair through `at` across `clip`: dashed horizontal and vertical lines, and
/// fainter 45° ones, leaving a hole around the pointer.
fn draw_crosshair(ui: &egui::Ui, clip: egui::Rect, at: egui::Pos2, color: egui::Color32) {
    use egui::{Shape, Stroke, vec2};
    let painter = ui.painter_at(clip);
    // One physical pixel wide.
    let width = 1.0 / ui.ctx().pixels_per_point();
    let stroke = Stroke::new(width, color);
    let faint = Stroke::new(width, color.gamma_multiply(0.5));
    // Long enough to cross the canvas from anywhere; the painter clips.
    let reach = clip.width() + clip.height();
    let diagonal = std::f32::consts::FRAC_1_SQRT_2;
    let rays = [
        (vec2(1.0, 0.0), stroke),
        (vec2(-1.0, 0.0), stroke),
        (vec2(0.0, 1.0), stroke),
        (vec2(0.0, -1.0), stroke),
        (vec2(diagonal, diagonal), faint),
        (vec2(-diagonal, diagonal), faint),
        (vec2(diagonal, -diagonal), faint),
        (vec2(-diagonal, -diagonal), faint),
    ];
    for (dir, stroke) in rays {
        // Each ray starts its dashes at the hole, so they don't crawl as the pointer moves.
        let line = [at + dir * CROSSHAIR_HOLE, at + dir * reach];
        painter.extend(Shape::dashed_line(&line, stroke, CROSSHAIR_DASH, CROSSHAIR_GAP));
    }
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

/// Handles of the selected drawing, zone or keepout: corners as squares, the others as
/// small circles; the one under the pointer filled.
pub fn draw_handles(
    ui: &egui::Ui,
    canvas: &Canvas,
    clip: egui::Rect,
    handles: &[view::handles::Handle],
    hot: Option<view::handles::Grip>,
) {
    let painter = ui.painter_at(clip);
    let accent = crate::theme::accent();
    let bg = crate::theme::panel();
    for h in handles {
        let c = canvas.world_to_screen([h.pos.x as f64 * 1e-6, h.pos.y as f64 * 1e-6]);
        let fill = if hot == Some(h.grip) { accent } else { bg };
        let stroke = egui::Stroke::new(1.5, accent);
        if h.is_minor() {
            painter.circle(c, 3.5, fill, stroke);
        } else {
            let r = egui::Rect::from_center_size(c, egui::vec2(8.0, 8.0));
            painter.rect(r, 1.0, fill, stroke, egui::StrokeKind::Middle);
        }
    }
}

/// The Measure tool's Clearance overlay: the picked objects' outlines (and the hovered
/// one's, thinner), the gap as a ruler between the nearest points, and its reading with
/// the rule (in the warning colour when the gap is too small).
pub fn draw_clearance(
    ui: &egui::Ui,
    canvas: &Canvas,
    clip: egui::Rect,
    picked: &[editor_core::copper::Geo],
    hover: &[editor_core::copper::Geo],
    gap: Option<&editor_core::clearance::Gap>,
) {
    use egui::{FontId, Stroke, vec2};
    let painter = ui.painter_at(clip);
    let accent = crate::theme::accent();
    let screen = |p: model::Point| canvas.world_to_screen([p.x as f64 * 1e-6, p.y as f64 * 1e-6]);
    let outline = |g: &editor_core::copper::Geo, stroke: Stroke| {
        for line in view::measure::geo_outlines(g) {
            painter.add(egui::Shape::line(line.into_iter().map(screen).collect(), stroke));
        }
    };
    for g in hover {
        outline(g, Stroke::new(1.0, accent.gamma_multiply(0.7)));
    }
    for g in picked {
        outline(g, Stroke::new(2.0, accent));
    }
    let Some(gap) = gap else { return };
    let color = if gap.violates() { crate::theme::warn() } else { accent };
    let (sa, sb) = (screen(gap.a), screen(gap.b));
    let stroke = Stroke::new(2.0, color);
    painter.line_segment([sa, sb], stroke);
    for c in [sa, sb] {
        painter.circle_filled(c, 3.0, color);
    }
    let text = view::measure::gap_text(gap, crate::units::fmt);
    let galley = painter.layout_no_wrap(text, FontId::proportional(12.0), ui.visuals().text_color());
    let centre = sa + (sb - sa) / 2.0;
    let inner = clip.shrink(8.0);
    let mut pos = centre + vec2(12.0, 10.0);
    let size = galley.size();
    if pos.x + size.x > inner.right() {
        pos.x = centre.x - 12.0 - size.x;
    }
    if pos.y + size.y > inner.bottom() {
        pos.y = centre.y - 10.0 - size.y;
    }
    pos = pos.max(inner.min);
    let bg = egui::Rect::from_min_size(pos, size).expand(5.0);
    painter.rect(bg, 4.0, crate::theme::panel(), Stroke::new(1.0, color), egui::StrokeKind::Inside);
    painter.galley(pos, galley, ui.visuals().text_color());
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

#[cfg(test)]
mod tests {
    use super::notch_counts;

    #[test]
    fn a_burst_of_wheel_events_is_one_notch() {
        let mut last = None;
        let counted: Vec<bool> =
            [(0.0, 1.0), (0.005, 1.0), (0.02, 1.0), (0.05, 1.0), (0.06, -1.0), (0.07, 1.0), (0.071, 1.0)]
                .into_iter()
                .map(|(t, s)| notch_counts(&mut last, t, s))
                .collect();
        // A burst counts once; the next notch 50 ms on does; turning back counts at once.
        assert_eq!(counted, [true, false, false, true, true, true, false]);
    }
}
