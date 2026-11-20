//! The Simulation tab: the waveform plot of the analysis the Simulation panel shows (a
//! table for an operating point), its tool bar and, under the plot, the traces with
//! their values at cursors A and B.
//!
//! The logic (scales, ticks, decimation, traces) is `sim::plot`; this draws it with
//! egui's painter and handles the pointer: wheel zooms x around the pointer (Shift: y),
//! middle or right drag pans, left drag zooms into a box, click places cursor A
//! (Shift+click B), dragging a cursor moves it, double-click or V fits, Esc removes the
//! cursors. While the user hasn't zoomed y, y follows the visible part of the traces.

use std::collections::HashSet;
use std::sync::Arc;

use egui::{Align2, Color32, FontId, Pos2, Rect, RichText, Sense, Stroke, Ui, pos2, vec2};
use model::*;
use sim::plot::{self, Axis, PlotData, Scale, Signal};
use view::Document;
use view::sim_overlay::ProbeTarget;

use crate::sim_ui::SimPanel;
use crate::theme;

/// What the tab asks the app to do.
pub enum PlotAct {
    /// The plot was clicked: it takes the keys.
    Focus,
    Run(Id),
    /// Right-click → Remove probe: (analysis, probe index).
    RemoveProbe(Id, usize),
}

/// A signal's target on the schematic (hovering its row highlights it).
fn target(s: &Signal) -> ProbeTarget {
    match s {
        // The net is found by name: `at` is only used to probe.
        Signal::Voltage(net) => ProbeTarget::Net { net: net.clone(), at: model::ObjectId::Wire(Id::from_u128(0)) },
        Signal::Diff(net, reference) => {
            let at = model::ObjectId::Wire(Id::from_u128(0));
            ProbeTarget::Diff { net: net.clone(), at, reference: reference.clone(), ref_at: at }
        }
        Signal::Current(symbol, pin) => ProbeTarget::Pin { symbol: *symbol, pin: *pin },
        Signal::Power(component) => ProbeTarget::Part { component: *component },
    }
}

/// A plot's data, with what it was made of (run, signals, phase) and how many named
/// nets there are.
type Data = ((u64, Vec<Signal>, bool), Arc<PlotData>, usize);

/// A cached decimation: its key and the points of each trace in values.
type Decimated = ((u64, u64, u64, usize, bool, bool), Vec<Vec<[f64; 2]>>);

/// The view of one analysis' plot (in memory only).
#[derive(Default)]
pub struct PlotState {
    x: Option<Scale>,
    y: [Option<Scale>; 3],
    /// The user zoomed or panned y: it no longer follows x.
    y_manual: bool,
    log_x: Option<bool>,
    /// Cursor A and B, as x values of samples.
    cursors: [Option<f64>; 2],
    /// Cursor A before the last click (a double-click puts it back).
    before_click: Option<Option<f64>>,
    hidden: HashSet<String>,
    /// The data and what it was made of: (run, signals, phase).
    data: Option<Data>,
    /// The run's x range the view was fitted to (a rerun over the same range keeps
    /// the zoom).
    fitted_to: Option<(f64, f64)>,
    drag: Option<Drag>,
    decimated: Option<Decimated>,
    /// Operating point table: sort column (0: signal, 1: value) and descending.
    sort: (usize, bool),
    /// The readout table shows measurements instead of the cursor values.
    measure: bool,
    /// The measurements as last computed: (range, data) and the table.
    measured: Option<(MeasureKey, MeasureTable)>,
}

type MeasureKey = (u64, u64, usize);
type MeasureTable = (Vec<&'static str>, Vec<Vec<String>>);

#[derive(Clone, Copy)]
enum Drag {
    Pan,
    Box(Pos2),
    Cursor(usize),
}

impl PlotState {
    /// Fits x to the whole run and y to the traces.
    pub fn fit(&mut self) {
        self.x = None;
        self.y = [None; 3];
        self.y_manual = false;
    }

    pub fn clear_cursors(&mut self) {
        self.cursors = [None, None];
    }

    /// The x and y scales, filling in what a fit cleared: x over the whole run, y to the
    /// visible part of the traces (always, while the user hasn't zoomed y).
    fn scales(&mut self, data: &PlotData, log_x: bool) -> (Scale, [Scale; 3]) {
        let (x0, x1) = (data.x.first().copied().unwrap_or(0.0), data.x.last().copied().unwrap_or(1.0));
        let x = *self.x.get_or_insert_with(|| Scale::new(x0, x1, log_x));
        let mut ys = [Scale::new(-1.0, 1.0, false); 3];
        for (i, axis) in Axis::ALL.into_iter().enumerate() {
            let y = match self.y[i] {
                Some(y) if self.y_manual => y,
                _ => {
                    let (lo, hi) = plot::fit_y(data, axis, x.lo, x.hi, &self.hidden).unwrap_or((-1.0, 1.0));
                    Scale::new(lo, hi, false)
                }
            };
            self.y[i] = Some(y);
            ys[i] = y;
        }
        (x, ys)
    }
}

/// Space for the axes' labels.
const LEFT: f32 = 64.0;
const RIGHT: f32 = 64.0;
const BOTTOM: f32 = 36.0;
const TOP: f32 = 10.0;

pub fn ui(ui: &mut Ui, panel: &mut SimPanel, doc: &Document) -> Option<PlotAct> {
    // The tab has the canvas background, like the schematic; the chart sets itself off
    // from it (see `chart_bg`).
    ui.painter().rect_filled(ui.max_rect(), 0.0, theme::color(view::style::pal().background));
    let mut act = None;
    let setup = &doc.project().schematic.simulation;
    let Some(analysis) = panel.current(setup).cloned() else {
        centered_hint(ui, "Add an analysis in the Simulation panel (⌘2) to plot it.");
        return None;
    };
    let stale = panel.session.is_stale(doc.project(), doc.connectivity(), &analysis, doc.revision);
    let running = panel.session.running().is_some_and(|(id, _)| id == analysis.id);
    let ac = matches!(analysis.kind, AnalysisKind::Ac { .. });

    // The tool bar.
    let mut fit = false;
    let mut export = false;
    let mut phase = !panel.no_phase;
    let state = panel.plots.entry(analysis.id).or_default();
    let mut log_x = state.log_x.unwrap_or(ac);
    let mut picked = None;
    egui::Frame::new().inner_margin(egui::Margin::symmetric(8, 4)).show(ui, |ui| {
        ui.horizontal(|ui| {
            let options: Vec<(Id, String)> = setup.analyses.iter().map(|a| (a.id, a.name.clone())).collect();
            picked = crate::fields::combo(ui, "plot_analysis", &analysis.id, &options);
            ui.separator();
            if analysis.kind != AnalysisKind::Op {
                fit = ui.button("Fit").on_hover_text("Show the whole run (V, double-click)").clicked();
                ui.checkbox(&mut log_x, "Log x");
                if ac {
                    ui.checkbox(&mut phase, "Phase");
                }
                export = ui.button("Export CSV…").on_hover_text("The traces shown, every point of the run").clicked();
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if running {
                    ui.label(RichText::new("Running…").color(theme::text_dim()));
                    ui.spinner();
                } else if stale {
                    if ui.small_button("Run").on_hover_text("⌘R").clicked() {
                        act = Some(PlotAct::Run(analysis.id));
                    }
                    ui.label(RichText::new("Changed since the run").color(theme::warn()));
                }
            });
        });
    });
    if let Some(id) = picked {
        panel.show_tab(id);
    }
    panel.no_phase = !phase;
    let state = panel.plots.entry(analysis.id).or_default();
    if log_x != state.log_x.unwrap_or(ac) {
        state.log_x = Some(log_x);
        state.x = None;
        state.decimated = None;
    }
    if fit {
        state.fit();
    }

    let Some(done) = panel.session.result(analysis.id) else {
        let text = if running {
            "Simulating…"
        } else if analysis.probes.is_empty() && analysis.kind != AnalysisKind::Op {
            "Run the analysis (⌘R) to plot it. Pick signals with the Probe tool (O) on the schematic."
        } else {
            "Run the analysis (⌘R) to plot it."
        };
        centered_hint(ui, text);
        return act;
    };
    let dim = stale || running;
    if done.result.scale().is_none() {
        if let Some(t) = op_table(ui, &done.rows, &mut state.sort, dim) {
            panel.hover = Some(t);
        }
        return act;
    }

    // The data, rebuilt when the run, the signals or the phase choice change.
    let (signals, named) = plot::signals(&analysis, &done.result, doc.connectivity());
    let key = (done.run, signals.clone(), phase);
    if state.data.as_ref().is_none_or(|d| d.0 != key) {
        let Some(data) = plot::data(&done.result, &signals, phase, doc.connectivity()) else { return act };
        let range = (data.x.first().copied().unwrap_or(0.0), data.x.last().copied().unwrap_or(1.0));
        if state.fitted_to != Some(range) {
            state.x = None;
            state.fitted_to = Some(range);
        }
        if !state.y_manual {
            state.y = [None; 3];
        }
        state.data = Some((key, Arc::new(data), named));
        state.decimated = None;
    }
    let (_, data, named) = state.data.clone().expect("built above");
    if export {
        export_csv(&data, &state.hidden, &analysis.name);
    }

    // Plot above, readout table below (at most 40 % of the height).
    // Header, traces, missing ones and up to two hint lines.
    let table_rows = data.traces.len() + data.missing.len() + 3;
    let table_h = (table_rows as f32 * 22.0 + 20.0).min(ui.available_height() * 0.4);
    let plot_h = (ui.available_height() - table_h).max(120.0);
    let (outer, response) = ui.allocate_exact_size(vec2(ui.available_width(), plot_h), Sense::click_and_drag());
    let right = if !data.axis_unit(Axis::Right2).is_empty() {
        2.0 * RIGHT
    } else if !data.axis_unit(Axis::Right).is_empty() {
        RIGHT
    } else {
        16.0
    };
    let rect = Rect::from_min_max(
        pos2(outer.left() + LEFT, outer.top() + TOP),
        pos2(outer.right() - right, outer.bottom() - BOTTOM),
    );
    if rect.width() < 40.0 || rect.height() < 40.0 {
        return act;
    }
    if response.clicked() || response.drag_started() || response.secondary_clicked() {
        act = Some(PlotAct::Focus);
    }
    plot_view(ui, state, &data, rect, &response, log_x);
    // After the pointer: a double-click may just have cleared the view for a fit.
    let scales = state.scales(&data, log_x);
    draw(ui, state, &data, scales, rect, outer, &response, dim);

    ui.add_space(4.0);
    egui::ScrollArea::vertical().id_salt(("plot_table", analysis.id)).auto_shrink([false, false]).show(ui, |ui| {
        egui::Frame::new().inner_margin(egui::Margin { left: 12, right: 12, top: 0, bottom: 8 }).show(ui, |ui| {
            let (hover, remove) = readout(ui, state, &data, &signals, named, analysis.probes.is_empty());
            if let Some(s) = hover {
                panel.hover = Some(target(&s));
            }
            if let Some(i) = remove {
                act = Some(PlotAct::RemoveProbe(analysis.id, i));
            }
        });
    });
    act
}

fn centered_hint(ui: &mut Ui, text: &str) {
    ui.centered_and_justified(|ui| ui.label(RichText::new(text).color(theme::text_dim())));
}

/// Fits the view where needed, and applies the pointer to it.
fn plot_view(ui: &Ui, state: &mut PlotState, data: &PlotData, rect: Rect, response: &egui::Response, log_x: bool) {
    let (x, _) = state.scales(data, log_x);
    let to_frac = |p: Pos2| ((p.x - rect.left()) / rect.width(), (rect.bottom() - p.y) / rect.height());
    let input = ui.input(|i| (i.smooth_scroll_delta, i.zoom_delta(), i.modifiers, i.pointer.interact_pos()));
    let (scroll, zoom, mods, pointer) = input;
    if response.hovered()
        && let Some(p) = pointer
        && rect.contains(p)
    {
        let (fx, fy) = to_frac(p);
        let wheel = if mods.shift { scroll.x + scroll.y } else { scroll.y };
        let factor = zoom as f64 * (wheel as f64 * 0.003).exp();
        if (factor - 1.0).abs() > 1e-6 {
            if mods.shift {
                for s in state.y.iter_mut().flatten() {
                    s.zoom(fy as f64, factor);
                }
                state.y_manual = true;
            } else if let Some(s) = &mut state.x {
                s.zoom(fx as f64, factor);
            }
        }
        if !mods.shift
            && scroll.x.abs() > 0.0
            && let Some(s) = &mut state.x
        {
            s.pan(-scroll.x as f64 / rect.width() as f64);
        }
    }
    // Drags.
    let cursor_px = |c: Option<f64>| c.map(|v| rect.left() + state.x.unwrap_or(x).frac(v) as f32 * rect.width());
    if response.drag_started() {
        // Where the button went down (the drag is only detected some pixels later).
        let at = ui.input(|i| i.pointer.press_origin()).or(response.interact_pointer_pos()).unwrap_or_default();
        state.drag = if response.dragged_by(egui::PointerButton::Primary) {
            let near = (0..2).find(|&k| cursor_px(state.cursors[k]).is_some_and(|x| (x - at.x).abs() <= 5.0));
            Some(near.map_or(Drag::Box(at), Drag::Cursor))
        } else {
            Some(Drag::Pan)
        };
    }
    if response.dragged() {
        let d = response.drag_delta();
        match state.drag {
            Some(Drag::Pan) => {
                if let Some(s) = &mut state.x {
                    s.pan(-d.x as f64 / rect.width() as f64);
                }
                if d.y != 0.0 {
                    for s in state.y.iter_mut().flatten() {
                        s.pan(d.y as f64 / rect.height() as f64);
                    }
                    state.y_manual = true;
                }
            }
            Some(Drag::Cursor(k)) => {
                if let Some(p) = response.interact_pointer_pos() {
                    let v = state.x.unwrap_or(x).value(to_frac(p).0 as f64);
                    state.cursors[k] = plot::nearest(&data.x, v).map(|i| data.x[i]);
                }
            }
            _ => {}
        }
    }
    if response.drag_stopped() {
        if let (Some(Drag::Box(start)), Some(end)) = (state.drag, response.interact_pointer_pos()) {
            let b = Rect::from_two_pos(start, end).intersect(rect);
            if b.width() > 4.0 {
                let x = state.x.unwrap_or(x);
                let (a, c) = (to_frac(b.left_top()).0 as f64, to_frac(b.right_top()).0 as f64);
                state.x = Some(Scale::new(x.value(a), x.value(c), x.log));
            }
            if b.height() > 4.0 {
                let (lo, hi) = (to_frac(b.left_bottom()).1 as f64, to_frac(b.left_top()).1 as f64);
                for s in state.y.iter_mut().flatten() {
                    *s = Scale::new(s.value(lo), s.value(hi), false);
                }
                state.y_manual = true;
            }
        }
        state.drag = None;
    }
    // The second click of a double-click counts as a click too: it places no cursor.
    if response.clicked()
        && !response.double_clicked()
        && let Some(p) = response.interact_pointer_pos()
    {
        let v = state.x.unwrap_or(x).value(to_frac(p).0 as f64);
        let snapped = plot::nearest(&data.x, v).map(|i| data.x[i]);
        let k = usize::from(mods.shift);
        state.before_click = Some(state.cursors[0]);
        state.cursors[k] = snapped;
    }
    if response.double_clicked() {
        if let Some(before) = state.before_click.take() {
            state.cursors[0] = before;
        }
        state.fit();
    }
}

#[allow(clippy::too_many_arguments)]
fn draw(
    ui: &Ui,
    state: &mut PlotState,
    data: &PlotData,
    scales: (Scale, [Scale; 3]),
    rect: Rect,
    outer: Rect,
    response: &egui::Response,
    dim: bool,
) {
    let painter = ui.painter_at(outer);
    let (x, ys) = scales;
    painter.rect_filled(rect, 4.0, chart_bg());
    let grid = theme::color(view::style::pal().plot_grid);
    let text = theme::color(view::style::pal().plot_text);
    let font = FontId::proportional(11.0);
    let px = |v: f64| rect.left() + x.frac(v) as f32 * rect.width();
    let py = |s: &Scale, v: f64| rect.bottom() - s.frac(v) as f32 * rect.height();

    // Grid and tick labels.
    for v in x.ticks(((rect.width() / 90.0) as usize).max(2)) {
        let p = px(v);
        painter.vline(p, rect.y_range(), Stroke::new(1.0, grid));
        painter.text(pos2(p, rect.bottom() + 4.0), Align2::CENTER_TOP, plot::label(v, data.x_unit), font.clone(), text);
    }
    painter.text(pos2(rect.center().x, outer.bottom() - 2.0), Align2::CENTER_BOTTOM, &data.x_label, font.clone(), text);
    for (i, axis) in Axis::ALL.into_iter().enumerate() {
        let unit = data.axis_unit(axis);
        if unit.is_empty() {
            continue;
        }
        let s = &ys[i];
        for v in s.ticks(((rect.height() / 40.0) as usize).max(2)) {
            let p = py(s, v);
            if i == 0 {
                painter.hline(rect.x_range(), p, Stroke::new(1.0, grid));
                painter.text(
                    pos2(rect.left() - 6.0, p),
                    Align2::RIGHT_CENTER,
                    plot::label(v, unit),
                    font.clone(),
                    text,
                );
            } else {
                let out = if axis == Axis::Right2 { RIGHT } else { 0.0 };
                painter.text(
                    pos2(rect.right() + 6.0 + out, p),
                    Align2::LEFT_CENTER,
                    plot::label(v, unit),
                    font.clone(),
                    text,
                );
            }
        }
    }
    painter.rect_stroke(rect, 4.0, Stroke::new(1.0, grid), egui::StrokeKind::Inside);

    // Traces, decimated per pixel column (cached until the view, size or data change).
    let columns = rect.width().ceil() as usize;
    let key = (x.lo.to_bits(), x.hi.to_bits(), std::ptr::from_ref(data) as u64, columns, x.log, dim);
    if state.decimated.as_ref().is_none_or(|d| d.0 != key) {
        let col = |v: f64| (x.frac(v) * columns as f64).floor() as i64;
        let pts = data.traces.iter().map(|t| plot::decimate(&data.x, &t.y, x.lo, x.hi, columns, col)).collect();
        state.decimated = Some((key, pts));
    }
    let clip = painter.with_clip_rect(rect);
    let visible = plot::visible(&data.x, x.lo, x.hi).len();
    let dots = visible > 0 && rect.width() / visible as f32 > 8.0;
    for (t, pts) in data.traces.iter().zip(&state.decimated.as_ref().unwrap().1) {
        if state.hidden.contains(&t.label) {
            continue;
        }
        let s = &ys[t.axis.index()];
        let mut color = theme::color(view::style::trace_color(t.signal));
        if dim {
            color = color.gamma_multiply(0.35);
        }
        let points: Vec<Pos2> = pts.iter().map(|p| pos2(px(p[0]), py(s, p[1]))).collect();
        let stroke = Stroke::new(1.5, color);
        if t.dashed || t.signal >= 8 {
            clip.extend(egui::Shape::dashed_line(&points, stroke, 6.0, 4.0));
        } else {
            clip.add(egui::Shape::line(points.clone(), stroke));
        }
        if dots {
            for p in &points {
                clip.circle_filled(*p, 2.5, color);
            }
        }
    }

    // Cursors.
    for (k, c) in state.cursors.iter().enumerate() {
        let Some(v) = c else { continue };
        let p = px(*v);
        if p < rect.left() || p > rect.right() {
            continue;
        }
        let color = theme::accent();
        if k == 0 {
            painter.vline(p, rect.y_range(), Stroke::new(1.5, color));
        } else {
            painter.extend(egui::Shape::dashed_line(
                &[pos2(p, rect.top()), pos2(p, rect.bottom())],
                Stroke::new(1.5, color),
                5.0,
                3.0,
            ));
        }
        let label = if k == 0 { "A" } else { "B" };
        let r = Rect::from_center_size(pos2(p, rect.top() + 9.0), vec2(16.0, 16.0));
        painter.rect_filled(r, 3.0, color);
        painter.text(r.center(), Align2::CENTER_CENTER, label, FontId::proportional(11.0), Color32::WHITE);
    }

    // Box being dragged.
    if let (Some(Drag::Box(start)), Some(end)) = (state.drag, response.interact_pointer_pos()) {
        let b = Rect::from_two_pos(start, end).intersect(rect);
        painter.rect(
            b,
            0.0,
            theme::accent().gamma_multiply(0.12),
            Stroke::new(1.0, theme::accent()),
            egui::StrokeKind::Inside,
        );
    }

    // Hover: a guide line and the values at the pointer's x.
    if state.drag.is_none()
        && let Some(p) = response.hover_pos()
        && rect.contains(p)
    {
        painter.vline(p.x, rect.y_range(), Stroke::new(1.0, text.gamma_multiply(0.6)));
        let v = x.value(((p.x - rect.left()) / rect.width()) as f64);
        let rows: Vec<(Color32, bool, String, String)> = data
            .traces
            .iter()
            .filter(|t| !state.hidden.contains(&t.label))
            .filter_map(|t| {
                let y = plot::interpolate(&data.x, &t.y, v)?;
                let color = theme::color(view::style::trace_color(t.signal));
                Some((color, t.dashed || t.signal >= 8, t.label.clone(), plot::label(y, t.unit)))
            })
            .collect();
        egui::Tooltip::always_open(
            ui.ctx().clone(),
            ui.layer_id(),
            egui::Id::unique("plot_hover"),
            egui::PopupAnchor::Pointer,
        )
        .gap(12.0)
        .show(|ui| {
            ui.label(RichText::new(plot::label(v, data.x_unit)).strong());
            egui::Grid::new("plot_hover_grid").spacing([10.0, 2.0]).show(ui, |ui| {
                for (color, dashed, label, value) in rows {
                    ui.horizontal(|ui| {
                        swatch(ui, color, dashed);
                        ui.label(label);
                    });
                    ui.label(RichText::new(value).monospace());
                    ui.end_row();
                }
            });
        });
    }
}

/// The chart's background: the window background in the dark theme; white in the light
/// one, where the window background is a mid grey that hides the grid and leaves the
/// lighter trace colours at 1.5:1 contrast (white: 2.2:1 at worst, the readout table
/// names every trace).
fn chart_bg() -> Color32 {
    theme::color(view::style::pal().plot_bg)
}

/// A trace's colour: a short thick line, dashed like the trace.
fn swatch(ui: &mut Ui, color: Color32, dashed: bool) {
    let (r, _) = ui.allocate_exact_size(vec2(16.0, 10.0), Sense::hover());
    let stroke = Stroke::new(3.0, color);
    if dashed {
        ui.painter().extend(egui::Shape::dashed_line(&[r.left_center(), r.right_center()], stroke, 4.0, 2.5));
    } else {
        ui.painter().line_segment([r.left_center(), r.right_center()], stroke);
    }
}

/// The traces with their values at the cursors; a click on a row hides or shows it.
/// Returns the signal hovered and a probe to remove (right-click; `auto`: no probes).
fn readout(
    ui: &mut Ui,
    state: &mut PlotState,
    data: &PlotData,
    signals: &[Signal],
    named: usize,
    auto: bool,
) -> (Option<Signal>, Option<usize>) {
    let mut hover = None;
    let mut remove = None;
    let mut row_menu = |r: &egui::Response, signal: usize| {
        if r.hovered() {
            hover = signals.get(signal).cloned();
        }
        if !auto {
            r.context_menu(|ui| {
                if ui.button("Remove probe").clicked() {
                    remove = Some(signal);
                    ui.close();
                }
            });
        }
    };
    let at = |c: Option<f64>| c.and_then(|v| plot::nearest(&data.x, v));
    let (a, b) = (at(state.cursors[0]), at(state.cursors[1]));
    let mut toggle = None;
    // Measurements: between the cursors, else over the visible range.
    let (lo, hi, between) = match state.cursors {
        [Some(p), Some(q)] => (p.min(q), p.max(q), true),
        _ => state.x.map_or((f64::NEG_INFINITY, f64::INFINITY, false), |x| (x.lo, x.hi, false)),
    };
    let table = state.measure.then(|| {
        let key = (lo.to_bits(), hi.to_bits(), std::ptr::from_ref(data) as usize);
        if state.measured.as_ref().is_none_or(|m| m.0 != key) {
            state.measured = Some((key, sim::measure::table(data, lo, hi)));
        }
        state.measured.as_ref().expect("set above").1.clone()
    });
    ui.horizontal(|ui| {
        let tip = "Measure the traces between cursors A and B, else over the visible range";
        if ui.add(egui::Button::new("Measure").selected(state.measure)).on_hover_text(tip).clicked() {
            state.measure = !state.measure;
        }
        if let Some((cols, rows)) = &table {
            let (x0, x1) = (lo.max(data.x[0]), hi.min(*data.x.last().unwrap_or(&0.0)));
            let span = format!("{} … {}", plot::label(x0, data.x_unit), plot::label(x1, data.x_unit));
            let what = if between { format!("Between A and B · {span}") } else { format!("Visible range · {span}") };
            ui.label(RichText::new(what).color(theme::text_dim()));
            if ui.small_button("Copy").on_hover_text("The table as text, tab-separated").clicked() {
                let mut text = format!("Signal\t{}\n", cols.join("\t"));
                for (t, row) in data.traces.iter().zip(rows) {
                    text.push_str(&format!("{}\t{}\n", t.label, row.join("\t")));
                }
                ui.ctx().copy_text(text);
            }
        }
    });
    egui::Grid::new("plot_readout").striped(true).spacing([18.0, 3.0]).show(ui, |ui| {
        crate::fields::label(ui, "Signal");
        let head = |ui: &mut Ui, text: String| {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| crate::fields::label(ui, &text))
        };
        if let Some((cols, rows)) = &table {
            for c in cols {
                head(ui, c.to_string());
            }
            ui.end_row();
            for (k, t) in data.traces.iter().enumerate() {
                let hidden = state.hidden.contains(&t.label);
                let color = theme::color(view::style::trace_color(t.signal));
                let r = ui
                    .horizontal(|ui| {
                        swatch(ui, if hidden { color.gamma_multiply(0.25) } else { color }, t.dashed || t.signal >= 8);
                        let text = RichText::new(&t.label);
                        let text = if hidden { text.color(theme::text_dim()).strikethrough() } else { text };
                        ui.add(egui::Label::new(text).sense(Sense::click()))
                    })
                    .inner;
                row_menu(&r, t.signal);
                if r.on_hover_text(if hidden { "Show" } else { "Hide" }).clicked() {
                    toggle = Some(t.label.clone());
                }
                for v in &rows[k] {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(RichText::new(v).monospace())
                    });
                }
                ui.end_row();
            }
        } else {
            let xa = a.map(|i| data.x[i]);
            let xb = b.map(|i| data.x[i]);
            head(ui, xa.map_or("A".into(), |v| format!("A {}", plot::label(v, data.x_unit))));
            head(ui, xb.map_or("B".into(), |v| format!("B {}", plot::label(v, data.x_unit))));
            let dx = xa.zip(xb).map(|(p, q)| q - p);
            head(
                ui,
                dx.map_or("B − A".into(), |d| {
                    let inv = if d != 0.0 && data.x_unit == "s" {
                        format!(" · {}", plot::label(1.0 / d.abs(), "Hz"))
                    } else {
                        String::new()
                    };
                    format!("Δ {}{inv}", plot::label(d, data.x_unit))
                }),
            );
            ui.end_row();
            for t in &data.traces {
                let hidden = state.hidden.contains(&t.label);
                let color = theme::color(view::style::trace_color(t.signal));
                let r = ui
                    .horizontal(|ui| {
                        swatch(ui, if hidden { color.gamma_multiply(0.25) } else { color }, t.dashed || t.signal >= 8);
                        let text = RichText::new(&t.label);
                        let text = if hidden { text.color(theme::text_dim()).strikethrough() } else { text };
                        ui.add(egui::Label::new(text).sense(Sense::click()))
                    })
                    .inner;
                row_menu(&r, t.signal);
                if r.on_hover_text(if hidden { "Show" } else { "Hide" }).clicked() {
                    toggle = Some(t.label.clone());
                }
                let value = |i: Option<usize>| i.map_or(String::new(), |i| plot::label(t.y[i], t.unit));
                let delta = a.zip(b).map_or(String::new(), |(i, j)| plot::label(t.y[j] - t.y[i], t.unit));
                for v in [value(a), value(b), delta] {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(RichText::new(v).monospace())
                    });
                }
                ui.end_row();
            }
        }
        for m in &data.missing {
            let text = RichText::new(format!("{} · {}", m.label, m.reason)).color(theme::text_dim());
            let r = ui.add(egui::Label::new(text).sense(Sense::click()));
            row_menu(&r, m.signal);
            ui.end_row();
        }
    });
    if let Some(l) = toggle
        && !state.hidden.remove(&l)
    {
        state.hidden.insert(l);
    }
    if auto {
        let shown = data.traces.iter().filter(|t| !t.dashed).count();
        let text = if named > shown {
            format!("{shown} of {named} named nets; tick signals in the Simulation panel to choose.")
        } else if shown == 0 {
            "No named nets: tick signals in the Simulation panel to plot them.".to_string()
        } else {
            "The named nets; tick signals in the Simulation panel to choose.".to_string()
        };
        crate::fields::hint(ui, &text);
    }
    if state.cursors.iter().all(Option::is_none) {
        crate::fields::hint(ui, "Click the plot to place cursor A, Shift+click for B.");
    }
    (hover, remove)
}

/// The operating point's values, sortable by signal or value. Returns the hovered row's
/// target.
fn op_table(ui: &mut Ui, rows: &[sim::report::Row], sort: &mut (usize, bool), dim: bool) -> Option<ProbeTarget> {
    let mut hover = None;
    let mut rows: Vec<&sim::report::Row> = rows.iter().collect();
    let number = |r: &sim::report::Row| r.values.first().and_then(|v| si::parse(v)).unwrap_or(f64::NAN);
    match sort.0 {
        1 => rows.sort_by(|a, b| number(a).total_cmp(&number(b))),
        _ => rows.sort_by(|a, b| natural_cmp(&a.label, &b.label)),
    }
    if sort.1 {
        rows.reverse();
    }
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        egui::Frame::new().inner_margin(12).show(ui, |ui| {
            egui::Grid::new("op_table").striped(true).spacing([30.0, 4.0]).min_col_width(80.0).show(ui, |ui| {
                for (i, name) in ["Signal", "Value"].into_iter().enumerate() {
                    let arrow = if sort.0 == i { if sort.1 { " ▼" } else { " ▲" } } else { "" };
                    if ui
                        .add(egui::Button::new(RichText::new(format!("{name}{arrow}")).strong()).frame(false))
                        .clicked()
                    {
                        *sort = if sort.0 == i { (i, !sort.1) } else { (i, false) };
                    }
                }
                ui.end_row();
                for r in rows {
                    let text = |t: &str| {
                        let t = RichText::new(t);
                        if dim { t.color(theme::text_dim()) } else { t }
                    };
                    let l = ui.add(egui::Label::new(text(&r.label)).sense(Sense::hover()));
                    ui.label(text(r.values.first().map_or("", |v| v.as_str())).monospace());
                    if l.hovered() {
                        hover = match (&r.net, r.current) {
                            (Some(net), _) => Some(target(&Signal::Voltage(net.clone()))),
                            (None, Some((s, p))) => Some(target(&Signal::Current(s, p))),
                            _ => None,
                        };
                    }
                    ui.end_row();
                }
            });
        });
    });
    hover
}

fn export_csv(data: &PlotData, hidden: &HashSet<String>, name: &str) {
    let Some(path) = rfd::FileDialog::new()
        .set_title("Export CSV")
        .set_file_name(format!("{name}.csv"))
        .add_filter("CSV", &["csv"])
        .save_file()
    else {
        return;
    };
    if let Err(e) = std::fs::write(&path, plot::csv(data, hidden)) {
        log::warn!("can't write {}: {e}", path.display());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sim::plot::Trace;

    fn data() -> PlotData {
        let x: Vec<f64> = (0..=100).map(|i| i as f64 * 1e-5).collect();
        let y = x.iter().map(|t| (t * 6283.0).sin()).collect();
        let trace = Trace { label: "V(OUT)".into(), unit: "V", axis: Axis::Left, dashed: false, signal: 0, y };
        PlotData { x_label: "Time".into(), x_unit: "s", log_x: false, x, traces: vec![trace], missing: Vec::new() }
    }

    /// One frame of the plot (pointer handling, then drawing) with `events`.
    fn frame(ctx: &egui::Context, state: &mut PlotState, data: &PlotData, events: Vec<egui::Event>) {
        let input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0))),
            events,
            ..Default::default()
        };
        let mut out = ctx.run_ui(input, |ui| {
            let (outer, response) = ui.allocate_exact_size(vec2(780.0, 560.0), Sense::click_and_drag());
            let rect = outer.shrink(40.0);
            plot_view(ui, state, data, rect, &response, false);
            let scales = state.scales(data, false);
            draw(ui, state, data, scales, rect, outer, &response, false);
        });
        // No renderer takes the font textures.
        out.textures_delta.clear();
    }

    #[test]
    fn double_click_fits_without_losing_the_scales() {
        let ctx = egui::Context::default();
        let mut state = PlotState::default();
        let data = data();
        let at = pos2(400.0, 300.0);
        frame(&ctx, &mut state, &data, vec![egui::Event::PointerMoved(at)]);
        // Zoom in, then double-click: the frame of the double-click still draws.
        state.x = Some(Scale::new(2e-4, 4e-4, false));
        state.y_manual = true;
        let click = |pressed| egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        frame(&ctx, &mut state, &data, vec![click(true), click(false)]);
        frame(&ctx, &mut state, &data, vec![click(true), click(false)]);
        let x = state.x.expect("fitted again");
        assert_eq!((x.lo, x.hi), (0.0, 1e-3), "the whole run");
        assert!(!state.y_manual);
        // The double-click's first click doesn't leave a cursor behind.
        assert_eq!(state.cursors, [None, None]);
    }

    #[test]
    fn scales_fill_in_after_a_fit() {
        let mut state = PlotState::default();
        let data = data();
        state.fit();
        let (x, [left, ..]) = state.scales(&data, false);
        assert_eq!((x.lo, x.hi), (0.0, 1e-3));
        assert!(left.lo < -0.9 && left.hi > 0.9, "{left:?}");
    }
}
