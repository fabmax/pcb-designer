//! Properties panel: shows and edits the selected object(s).
//!
//! Every edit is an editor command. Continuous edits (dragging a value) are grouped into
//! one undo step; text fields are committed on Enter or when they lose focus.

use editor_core::Command;
use editor_core::copper::{CopperItem, Flag};
use editor_core::query::pin_pad_names;
use egui::RichText;
use model::*;
use rustc_hash::FxHashMap;
use view::Document;
use view::interaction::new_group;

use crate::theme;

#[derive(Default)]
pub struct Properties {
    group: Option<u64>,
    /// A grouped widget is being dragged or edited this frame.
    active: bool,
    texts: FxHashMap<(ObjectId, &'static str), String>,
}

struct Ctx<'a> {
    doc: &'a mut Document,
    message: &'a mut Option<String>,
}

impl Properties {
    pub fn ui(&mut self, ui: &mut egui::Ui, doc: &mut Document, selection: &[ObjectId], message: &mut Option<String>) {
        self.active = false;
        let mut cx = Ctx { doc, message };
        match selection {
            [] => project_summary(ui, cx.doc),
            [id] => self.single(ui, &mut cx, *id),
            ids => multiple(ui, ids),
        }
        if !self.active && self.group.take().is_some() {
            cx.doc.close_group();
        }
        let live: Vec<ObjectId> = selection.to_vec();
        self.texts.retain(|(id, _), _| live.contains(id));
    }

    fn run(&mut self, cx: &mut Ctx, cmd: Command, grouped: bool) {
        let group = if grouped { Some(*self.group.get_or_insert_with(new_group)) } else { None };
        if let Err(e) = cx.doc.execute_grouped(cmd, group) {
            *cx.message = Some(e.to_string());
        }
    }

    fn update(&mut self, cx: &mut Ctx, object: Object, grouped: bool) {
        self.run(cx, Command::Update { objects: vec![object] }, grouped);
    }

    /// Text field committed on Enter / focus loss. Returns the new text if it changed.
    fn text(&mut self, ui: &mut egui::Ui, target: ObjectId, label: &'static str, current: &str) -> Option<String> {
        let id = ui.make_persistent_id((target, label));
        let focused = ui.memory(|m| m.has_focus(id));
        let buf = self.texts.entry((target, label)).or_insert_with(|| current.to_string());
        if !focused {
            buf.clear();
            buf.push_str(current);
        }
        let r = ui.add(egui::TextEdit::singleline(buf).id(id).desired_width(f32::INFINITY));
        (r.lost_focus() && buf != current).then(|| buf.clone())
    }

    /// Length in mm, edited by dragging or typing. Returns the new value in nm.
    fn length(&mut self, ui: &mut egui::Ui, value: Coord, min: f64) -> Option<Coord> {
        let mut v = to_mm(value);
        let r = ui.add(egui::DragValue::new(&mut v).speed(0.05).range(min..=f64::MAX).suffix(" mm").max_decimals(4));
        self.active |= r.dragged() || r.has_focus();
        r.changed().then(|| mm(v))
    }

    fn point(&mut self, ui: &mut egui::Ui, p: Point) -> Option<Point> {
        let mut out = None;
        ui.horizontal(|ui| {
            let x = self.length(ui, p.x, f64::MIN);
            let y = self.length(ui, p.y, f64::MIN);
            if x.is_some() || y.is_some() {
                out = Some(Point::new(x.unwrap_or(p.x), y.unwrap_or(p.y)));
            }
        });
        out
    }

    fn single(&mut self, ui: &mut egui::Ui, cx: &mut Ctx, id: ObjectId) {
        let Some(object) = cx.doc.project().get(id) else { return };
        match object {
            Object::Symbol(s) => self.symbol(ui, cx, s),
            Object::Footprint(f) => self.footprint(ui, cx, f),
            Object::Wire(w) => {
                heading(ui, "Wire", "");
                grid(ui, |ui| {
                    row(ui, "Start", fmt_point(w.a));
                    row(ui, "End", fmt_point(w.b));
                    row(ui, "Length", format!("{:.3} mm", (w.b - w.a).length() * 1e-6));
                    net_rows(ui, cx.doc, id);
                });
            }
            Object::Junction(j) => {
                heading(ui, "Junction", "");
                grid(ui, |ui| {
                    row(ui, "Position", fmt_point(j.pos));
                    net_rows(ui, cx.doc, id);
                });
            }
            Object::Label(mut l) => {
                heading(ui, "Net label", "");
                grid(ui, |ui| {
                    label(ui, "Name");
                    if let Some(name) = self.text(ui, id, "name", &l.name) {
                        l.name = name;
                        self.update(cx, Object::Label(l.clone()), false);
                    }
                    ui.end_row();
                    label(ui, "Kind");
                    let mut kind = l.kind;
                    egui::ComboBox::from_id_salt("label_kind").selected_text(format!("{kind:?}")).show_ui(ui, |ui| {
                        ui.selectable_value(&mut kind, LabelKind::Net, "Net");
                        ui.selectable_value(&mut kind, LabelKind::Power, "Power");
                    });
                    if kind != l.kind {
                        l.kind = kind;
                        self.update(cx, Object::Label(l.clone()), false);
                    }
                    ui.end_row();
                    row(ui, "Position", fmt_point(l.pos));
                    net_rows(ui, cx.doc, id);
                });
            }
            Object::SchText(mut t) => {
                heading(ui, "Text", "");
                grid(ui, |ui| {
                    label(ui, "Text");
                    if let Some(s) = self.text(ui, id, "text", &t.text.text) {
                        t.text.text = s;
                        self.update(cx, Object::SchText(t.clone()), false);
                    }
                    ui.end_row();
                    label(ui, "Size");
                    if let Some(v) = self.length(ui, t.text.size, 0.1) {
                        t.text.size = v;
                        self.update(cx, Object::SchText(t.clone()), true);
                    }
                    ui.end_row();
                });
            }
            Object::Track(mut t) => {
                heading(ui, "Track", "");
                grid(ui, |ui| {
                    label(ui, "Layer");
                    if let Some(layer) = copper_layer_combo(ui, cx.doc.project(), t.layer) {
                        t.layer = layer;
                        self.update(cx, Object::Track(t.clone()), false);
                    }
                    ui.end_row();
                    label(ui, "Width");
                    let min = to_mm(cx.doc.project().board.rules.min_track_width);
                    if let Some(w) = self.length(ui, t.width, min) {
                        t.width = w;
                        self.update(cx, Object::Track(t.clone()), true);
                    }
                    ui.end_row();
                    row(ui, "Start", fmt_point(t.a));
                    row(ui, "End", fmt_point(t.b));
                    copper_net_row(ui, cx.doc, CopperItem::Track(t.id));
                });
            }
            Object::Via(mut v) => {
                heading(ui, "Via", "");
                grid(ui, |ui| {
                    label(ui, "Position");
                    if let Some(p) = self.point(ui, v.pos) {
                        v.pos = p;
                        self.update(cx, Object::Via(v.clone()), true);
                    }
                    ui.end_row();
                    label(ui, "Diameter");
                    if let Some(d) = self.length(ui, v.diameter, to_mm(v.drill) + 0.05) {
                        v.diameter = d;
                        self.update(cx, Object::Via(v.clone()), true);
                    }
                    ui.end_row();
                    label(ui, "Drill");
                    let min = to_mm(cx.doc.project().board.rules.min_drill);
                    if let Some(d) = self.length(ui, v.drill, min) {
                        v.drill = d.min(v.diameter - mm(0.05));
                        self.update(cx, Object::Via(v.clone()), true);
                    }
                    ui.end_row();
                    copper_net_row(ui, cx.doc, CopperItem::Via(v.id));
                });
            }
            Object::Zone(mut z) => {
                heading(ui, "Copper zone", "Filling arrives with DRC (milestone 9)");
                grid(ui, |ui| {
                    label(ui, "Net");
                    if let Some(net) = self.text(ui, id, "net", &z.net) {
                        z.net = net;
                        self.update(cx, Object::Zone(z.clone()), false);
                    }
                    ui.end_row();
                    label(ui, "Layer");
                    if let Some(layer) = copper_layer_combo(ui, cx.doc.project(), z.layer) {
                        z.layer = layer;
                        self.update(cx, Object::Zone(z.clone()), false);
                    }
                    ui.end_row();
                    label(ui, "Priority");
                    let mut prio = z.priority;
                    if ui.add(egui::DragValue::new(&mut prio).range(0..=100)).changed() {
                        z.priority = prio;
                        self.update(cx, Object::Zone(z.clone()), true);
                    }
                    ui.end_row();
                    row(ui, "Corners", z.outline.len().to_string());
                });
            }
            Object::BoardGraphic(g) => {
                heading(ui, "Graphic", "");
                grid(ui, |ui| {
                    row(ui, "Layer", layer_name(cx.doc.project(), g.layer));
                    row(ui, "Shape", shape_name(&g.shape).into());
                });
            }
            Object::BoardText(mut t) => {
                heading(ui, "Board text", "");
                grid(ui, |ui| {
                    label(ui, "Text");
                    if let Some(s) = self.text(ui, id, "text", &t.text.text) {
                        t.text.text = s;
                        self.update(cx, Object::BoardText(t.clone()), false);
                    }
                    ui.end_row();
                    row(ui, "Layer", layer_name(cx.doc.project(), t.layer));
                });
            }
            other => {
                heading(ui, &format!("{:?}", other.id().kind()), "");
            }
        }
    }

    fn symbol(&mut self, ui: &mut egui::Ui, cx: &mut Ctx, mut s: SymbolInstance) {
        let project = cx.doc.project();
        let Some(c) = project.schematic.components.get(s.component).cloned() else { return };
        let Some(part) = project.library.parts.get(c.part).cloned() else { return };
        let target = ObjectId::Component(c.id);
        heading(ui, &c.refdes, &part.name);
        if !part.description.is_empty() {
            ui.label(RichText::new(&part.description).color(theme::text_dim()));
        }
        ui.add_space(4.0);
        grid(ui, |ui| {
            label(ui, "Reference");
            if let Some(refdes) = self.text(ui, target, "refdes", &c.refdes) {
                let cmd = Command::SetComponent { id: c.id, refdes: Some(refdes), value: None, variant: None };
                self.run(cx, cmd, false);
            }
            ui.end_row();
            label(ui, "Value");
            if let Some(value) = self.text(ui, target, "value", &c.value) {
                let cmd = Command::SetComponent { id: c.id, refdes: None, value: Some(value), variant: None };
                self.run(cx, cmd, false);
            }
            ui.end_row();
            label(ui, "Footprint");
            let current = part.variant(c.variant).map_or("?".to_string(), |v| v.name.clone());
            let mut variant = c.variant;
            egui::ComboBox::from_id_salt("variant").selected_text(current).show_ui(ui, |ui| {
                for v in &part.variants {
                    ui.selectable_value(&mut variant, v.id, &v.name);
                }
            });
            if variant != c.variant {
                let cmd = Command::SetComponent { id: c.id, refdes: None, value: None, variant: Some(variant) };
                self.run(cx, cmd, false);
            }
            ui.end_row();
            label(ui, "Position");
            if let Some(p) = self.point(ui, s.pos) {
                s.pos = p;
                self.update(cx, Object::Symbol(s.clone()), true);
            }
            ui.end_row();
            label(ui, "Rotation");
            if let Some(r) = rotation_combo(ui, "sym_rot", s.rotation) {
                s.rotation = r;
                self.update(cx, Object::Symbol(s.clone()), false);
            }
            ui.end_row();
            label(ui, "Mirrored");
            let mut mirror = s.mirror;
            if ui.checkbox(&mut mirror, "").changed() {
                s.mirror = mirror;
                self.update(cx, Object::Symbol(s.clone()), false);
            }
            ui.end_row();
        });
        pin_table(ui, cx.doc, Some(s.id), c.id);
    }

    fn footprint(&mut self, ui: &mut egui::Ui, cx: &mut Ctx, mut f: FootprintInstance) {
        let project = cx.doc.project();
        let Some(c) = project.schematic.components.get(f.component).cloned() else { return };
        let fp_name = project.library.footprint_of(c.part, c.variant).map_or("?".into(), |fp| fp.name.clone());
        heading(ui, &c.refdes, &fp_name);
        ui.label(RichText::new("Reference, value and footprint are edited in the schematic.").color(theme::text_dim()));
        ui.add_space(4.0);
        grid(ui, |ui| {
            row(ui, "Value", c.value.clone());
            label(ui, "Side");
            let mut side = f.side;
            egui::ComboBox::from_id_salt("side").selected_text(format!("{side:?}")).show_ui(ui, |ui| {
                ui.selectable_value(&mut side, Side::Top, "Top");
                ui.selectable_value(&mut side, Side::Bottom, "Bottom");
            });
            if side != f.side {
                let cmd = Command::FlipSide { ids: vec![ObjectId::Footprint(f.id)], axis_x: f.pos.x };
                self.run(cx, cmd, false);
                return;
            }
            ui.end_row();
            label(ui, "Position");
            ui.add_enabled_ui(!f.locked, |ui| {
                if let Some(p) = self.point(ui, f.pos) {
                    f.pos = p;
                    self.update(cx, Object::Footprint(f.clone()), true);
                }
            });
            ui.end_row();
            label(ui, "Rotation");
            ui.add_enabled_ui(!f.locked, |ui| {
                if let Some(r) = rotation_combo(ui, "fp_rot", f.rotation) {
                    f.rotation = r;
                    self.update(cx, Object::Footprint(f.clone()), false);
                }
            });
            ui.end_row();
            label(ui, "Locked");
            let mut locked = f.locked;
            if ui.checkbox(&mut locked, "").changed() {
                f.locked = locked;
                self.update(cx, Object::Footprint(f.clone()), false);
            }
            ui.end_row();
        });
        pin_table(ui, cx.doc, None, c.id);
    }
}

fn heading(ui: &mut egui::Ui, title: &str, subtitle: &str) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(title).strong().size(16.0));
        if !subtitle.is_empty() {
            ui.label(RichText::new(subtitle).color(theme::text_dim()));
        }
    });
}

fn grid(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui)) {
    egui::Grid::new(ui.next_auto_id()).num_columns(2).spacing([12.0, 6.0]).show(ui, add);
}

fn label(ui: &mut egui::Ui, text: &str) {
    ui.label(RichText::new(text).color(theme::text_dim()));
}

fn row(ui: &mut egui::Ui, k: &str, v: String) {
    label(ui, k);
    ui.label(v);
    ui.end_row();
}

fn fmt_point(p: Point) -> String {
    format!("{:.3}, {:.3} mm", to_mm(p.x), to_mm(p.y))
}

fn rotation_combo(ui: &mut egui::Ui, salt: &str, current: Angle) -> Option<Angle> {
    let mut deg = current.degrees();
    egui::ComboBox::from_id_salt(salt).selected_text(format!("{deg:.0}°")).show_ui(ui, |ui| {
        for d in [0.0, 90.0, 180.0, 270.0] {
            ui.selectable_value(&mut deg, d, format!("{d:.0}°"));
        }
    });
    (deg != current.degrees()).then(|| Angle::deg(deg))
}

fn layer_name(project: &Project, id: LayerId) -> String {
    project.board.layers.iter().find(|l| l.id == id).map_or(format!("{id:?}"), |l| l.name.clone())
}

fn copper_layer_combo(ui: &mut egui::Ui, project: &Project, current: LayerId) -> Option<LayerId> {
    let mut layer = current;
    egui::ComboBox::from_id_salt("copper_layer").selected_text(layer_name(project, current)).show_ui(ui, |ui| {
        for l in project.board.copper_layers() {
            ui.selectable_value(&mut layer, l.id, &l.name);
        }
    });
    (layer != current).then_some(layer)
}

fn shape_name(s: &Shape) -> &'static str {
    match s {
        Shape::Line { .. } => "Line",
        Shape::Arc { .. } => "Arc",
        Shape::Circle { .. } => "Circle",
        Shape::Rect { .. } => "Rectangle",
        Shape::Polygon { .. } => "Polygon",
    }
}

/// Net of board copper (from the pads it touches).
fn copper_net_row(ui: &mut egui::Ui, doc: &Document, item: CopperItem) {
    let copper = doc.copper();
    let net = copper.net_of(item).unwrap_or("none (not connected to a pad)");
    row(ui, "Net", net.to_string());
    let flag = match copper.get(item).and_then(|c| c.flag) {
        Some(Flag::Short) => "connects different nets (short)",
        Some(Flag::Unconnected) => "touches no pad",
        None => return,
    };
    label(ui, "Problem");
    ui.label(RichText::new(flag).color(theme::warn()));
    ui.end_row();
}

/// Net name and the pins on it.
fn net_rows(ui: &mut egui::Ui, doc: &Document, id: ObjectId) {
    let Some(net) = doc.connectivity().net_of(id) else { return };
    row(ui, "Net", net.name.clone());
    const MAX: usize = 12;
    let mut pins: Vec<String> = net.pins.iter().take(MAX).map(|p| p.label()).collect();
    if net.pins.len() > MAX {
        pins.push(format!("… {} more", net.pins.len() - MAX));
    }
    label(ui, "Pins");
    let text = if pins.is_empty() { "none".to_string() } else { pins.join(" ") };
    ui.add(egui::Label::new(RichText::new(text).monospace()).wrap());
    ui.end_row();
}

/// Pins of a placed symbol (or all pins of the component) with their pads and nets.
fn pin_table(ui: &mut egui::Ui, doc: &Document, symbol: Option<Id>, component: Id) {
    let conn = doc.connectivity();
    let mut pins: Vec<(&str, String, &str)> = conn
        .nets
        .iter()
        .flat_map(|n| n.pins.iter().map(move |p| (n, p)))
        .filter(|(_, p)| p.component == component && symbol.is_none_or(|s| p.symbol == s))
        .map(|(n, p)| (p.pin_name.as_str(), p.pads.join(", "), n.name.as_str()))
        .collect();
    if pins.is_empty() {
        // Connectivity not computed yet: pads only.
        let names = pin_pad_names(doc.project(), component);
        return simple_pin_table(ui, &names);
    }
    pins.sort_by(|a, b| natural_cmp(&a.1, &b.1).then_with(|| natural_cmp(a.0, b.0)));
    ui.add_space(6.0);
    egui::CollapsingHeader::new(RichText::new(format!("Pins ({})", pins.len())).strong()).default_open(false).show(
        ui,
        |ui| {
            egui::Grid::new("pins").num_columns(3).striped(true).spacing([12.0, 2.0]).show(ui, |ui| {
                label(ui, "Pin");
                label(ui, "Pad");
                label(ui, "Net");
                ui.end_row();
                for (pin, pads, net) in pins {
                    ui.monospace(pin);
                    ui.monospace(pads);
                    ui.monospace(net);
                    ui.end_row();
                }
            });
        },
    );
}

fn simple_pin_table(ui: &mut egui::Ui, pins: &[(String, Vec<String>)]) {
    if pins.is_empty() {
        return;
    }
    ui.add_space(6.0);
    egui::CollapsingHeader::new(RichText::new(format!("Pins ({})", pins.len())).strong()).default_open(false).show(
        ui,
        |ui| {
            egui::Grid::new("pins").num_columns(2).striped(true).spacing([12.0, 2.0]).show(ui, |ui| {
                label(ui, "Pin");
                label(ui, "Pad");
                ui.end_row();
                for (pin, pads) in pins {
                    ui.monospace(pin);
                    ui.monospace(pads.join(", "));
                    ui.end_row();
                }
            });
        },
    );
}

fn multiple(ui: &mut egui::Ui, ids: &[ObjectId]) {
    heading(ui, &format!("{} objects", ids.len()), "");
    let mut counts: Vec<(ObjectKind, usize)> = Vec::new();
    for id in ids {
        match counts.iter_mut().find(|(k, _)| *k == id.kind()) {
            Some((_, n)) => *n += 1,
            None => counts.push((id.kind(), 1)),
        }
    }
    grid(ui, |ui| {
        for (kind, n) in counts {
            row(ui, &format!("{kind:?}"), n.to_string());
        }
    });
    ui.add_space(6.0);
    ui.label(RichText::new("R rotate · X mirror · F flip · Del delete").color(theme::text_dim()));
}

fn project_summary(ui: &mut egui::Ui, doc: &Document) {
    let p = doc.project();
    heading(ui, &p.name, "");
    if let Some(path) = &doc.path {
        ui.label(RichText::new(path.display().to_string()).color(theme::text_dim()).small());
    }
    ui.add_space(4.0);
    grid(ui, |ui| {
        row(ui, "Components", p.schematic.components.len().to_string());
        row(ui, "Wires", p.schematic.wires.len().to_string());
        row(ui, "Labels", p.schematic.labels.len().to_string());
        let conn = doc.connectivity();
        row(ui, "Nets", conn.netlist().filter(|n| n.pins.len() > 1 || n.named).count().to_string());
        row(ui, "Unconnected pins", conn.unconnected.len().to_string());
        row(ui, "Copper layers", p.board.copper_layers().count().to_string());
        row(ui, "Tracks", p.board.tracks.len().to_string());
        row(ui, "Vias", p.board.vias.len().to_string());
        row(ui, "Library parts", p.library.parts.len().to_string());
    });
    ui.add_space(8.0);
    ui.label(RichText::new("Nothing selected").color(theme::text_dim()));
}
