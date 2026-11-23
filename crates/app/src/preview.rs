//! A small, view-only picture of one symbol or footprint: always fitted, no grid, no
//! origin, in its editor's palette (the schematic's or the PCB's). One preview keeps its
//! canvas (and GPU scene) while it switches between items; it redraws itself when that
//! palette changes ([`ItemView::sync`]).

use view::Document;
use view::item::{Item, ItemView};

use crate::canvas::Canvas;
use crate::theme;

/// What an empty preview says instead.
#[derive(Clone, Copy)]
pub enum Empty<'a> {
    Hint(&'a str),
    Error(&'a str),
}

pub struct Preview {
    view: ItemView,
    canvas: Canvas,
}

impl Default for Preview {
    fn default() -> Self {
        Self::new()
    }
}

impl Preview {
    pub fn new() -> Self {
        let mut view = ItemView::new();
        view.set_origin_visible(false);
        Preview { view, canvas: Canvas::new(view::style::Editor::Schematic, 1.0) }
    }

    /// Redraw on the next frame (after the shown document was replaced).
    pub fn invalidate(&mut self) {
        self.view.restyle();
    }

    /// Draw `item` of `doc` in a box of `size`, or `empty` when there is nothing to
    /// show. The response senses clicks.
    pub fn ui(
        &mut self,
        ui: &mut egui::Ui,
        doc: &Document,
        item: Option<Item>,
        size: egui::Vec2,
        empty: Empty,
    ) -> egui::Response {
        let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
        self.view.show(item);
        self.canvas.editor = self.view.editor();
        self.canvas.background = view::style::with_editor(self.view.editor(), background);
        self.view.sync(doc);
        let bounds = self.view.item().and_then(|_| self.view.content_bounds());
        let painter = ui.painter_at(rect);
        match bounds {
            Some(b) if ui.is_rect_visible(rect) => {
                let viewport = [rect.width() as f64, rect.height() as f64];
                let mm = model::to_mm;
                self.canvas.camera.fit([mm(b.min.x), mm(b.min.y), mm(b.max.x), mm(b.max.y)], viewport, 12.0);
                self.canvas.paint(ui, rect, &mut self.view.scene);
            }
            Some(_) => {}
            None => {
                painter.rect_filled(rect, 0.0, to_egui(self.canvas.background.color));
                // On the preview's background: in its theme.
                let (text, color) = view::style::with_editor(self.view.editor(), || match empty {
                    Empty::Hint(t) => (t, theme::text_dim()),
                    Empty::Error(t) => (t, theme::warn()),
                });
                let font = egui::FontId::proportional(12.0);
                let galley = painter.layout(text.to_string(), font, color, rect.width() - 16.0);
                let pos = rect.center() - galley.size() / 2.0;
                painter.galley(pos, galley, color);
            }
        }
        painter.rect_stroke(rect, 0.0, egui::Stroke::new(1.0, theme::border()), egui::StrokeKind::Inside);
        response
    }
}

fn background() -> render::gpu::Background {
    let bg = view::style::background(1.0);
    render::gpu::Background { show_grid: false, axis_color: render::Color::rgba(0, 0, 0, 0), ..bg }
}

fn to_egui(c: render::Color) -> egui::Color32 {
    let [r, g, b, a] = c.0;
    egui::Color32::from_rgba_unmultiplied(r, g, b, a)
}
