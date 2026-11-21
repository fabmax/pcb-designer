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
use crate::units;

#[derive(Default)]
pub struct Properties {
    group: Option<u64>,
    /// A grouped widget is being dragged or edited this frame.
    active: bool,
    texts: FxHashMap<(ObjectId, &'static str), String>,
    /// Text fields being edited: their buffers aren't reset to the model's value. Usually
    /// one; two in the frame where focus moves from one to another.
    editing: Vec<(ObjectId, &'static str)>,
    sim: crate::sim_form::ComponentForm,
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
            [] => {
                crate::fields::hint(
                    ui,
                    "Nothing selected. Click an object to see and edit its properties; drag a box to select several.",
                );
            }
            [id] => self.single(ui, &mut cx, *id),
            ids => {
                multiple(ui, ids);
                dnp_many(ui, &mut cx, ids);
                exclude_many(ui, &mut cx, ids);
            }
        }
        if !self.active && self.group.take().is_some() {
            cx.doc.close_group();
        }
        let live: Vec<ObjectId> = selection.to_vec();
        self.texts.retain(|(id, _), _| live.contains(id));
        self.editing.retain(|(id, _)| live.contains(id));
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
        self.edit(ui, target, label, current, false)
    }

    /// Multi-line text field (Enter adds a line), committed on focus loss.
    fn multiline(&mut self, ui: &mut egui::Ui, target: ObjectId, label: &'static str, current: &str) -> Option<String> {
        self.edit(ui, target, label, current, true)
    }

    /// A text field showing `current` until it is edited. The buffer follows the model
    /// except while this field is being edited: focus can move to a field drawn earlier
    /// in the same frame, so "not focused" alone would reset the buffer before the field
    /// reports losing focus, and the edit would be lost.
    fn edit(
        &mut self,
        ui: &mut egui::Ui,
        target: ObjectId,
        label: &'static str,
        current: &str,
        multiline: bool,
    ) -> Option<String> {
        let key = (target, label);
        let id = ui.make_persistent_id(key);
        let editing = self.editing.contains(&key);
        let buf = self.texts.entry(key).or_insert_with(|| current.to_string());
        if !editing {
            buf.clear();
            buf.push_str(current);
        }
        let edit =
            if multiline { egui::TextEdit::multiline(buf).desired_rows(3) } else { egui::TextEdit::singleline(buf) };
        let r = ui.add(edit.id(id).desired_width(f32::INFINITY));
        let buf = &self.texts[&key];
        if r.has_focus() && !editing {
            self.editing.push(key);
        } else if !r.has_focus() && editing {
            self.editing.retain(|k| *k != key);
        }
        (r.lost_focus() && buf != current).then(|| buf.clone())
    }

    /// Length in the display unit, edited by dragging or typing; at least `min` mm.
    /// Returns the new value in nm.
    fn length(&mut self, ui: &mut egui::Ui, value: Coord, min: f64) -> Option<Coord> {
        // Lengths are below a kilometre.
        let limit = mm(1e6);
        let min = if min > -1e6 { mm(min) } else { -limit };
        let (r, v) = crate::units::drag(ui, value, min, limit);
        self.active |= r.dragged() || r.has_focus();
        v
    }

    fn point(&mut self, ui: &mut egui::Ui, p: Point) -> Option<Point> {
        crate::fields::pair(ui, |ui, w| {
            let x = crate::fields::sized(ui, w, |ui| self.length(ui, p.x, f64::MIN));
            let y = crate::fields::sized(ui, w, |ui| self.length(ui, p.y, f64::MIN));
            (x.is_some() || y.is_some()).then(|| Point::new(x.unwrap_or(p.x), y.unwrap_or(p.y)))
        })
    }

    /// A sheet's drawing page: size, orientation, custom dimensions.
    fn page(&mut self, ui: &mut egui::Ui, cx: &mut Ctx, mut sh: Sheet) {
        crate::fields::section(ui, "Page", None);
        let page = sh.page;
        grid(ui, |ui| {
            label(ui, "Size");
            let mut size = page.size;
            let custom = match page.size {
                PageSize::Custom { .. } => page.size,
                _ => {
                    let d = page.dimensions().unwrap_or(Size::new(mm(297.0), mm(210.0)));
                    PageSize::Custom { w: d.w, h: d.h }
                }
            };
            crate::fields::combo_box(ui, "page_size").selected_text(size.label()).show_ui(ui, |ui| {
                ui.selectable_value(&mut size, PageSize::None, PageSize::None.label());
                for p in PageSize::NAMED {
                    ui.selectable_value(&mut size, p, p.label());
                }
                ui.selectable_value(&mut size, custom, PageSize::Custom { w: 0, h: 0 }.label());
            });
            if size != page.size {
                sh.page.size = size;
                self.update(cx, Object::Sheet(sh.clone()), false);
            }
            ui.end_row();
            match sh.page.size {
                PageSize::None => {}
                PageSize::Custom { w, h } => {
                    label(ui, "Width");
                    if let Some(w) = self.length(ui, w, 50.0) {
                        sh.page.size = PageSize::Custom { w, h };
                        self.update(cx, Object::Sheet(sh.clone()), true);
                    }
                    ui.end_row();
                    label(ui, "Height");
                    if let Some(h) = self.length(ui, h, 50.0) {
                        sh.page.size = PageSize::Custom { w, h };
                        self.update(cx, Object::Sheet(sh.clone()), true);
                    }
                    ui.end_row();
                }
                _ => {
                    label(ui, "Orientation");
                    let mut portrait = sh.page.portrait;
                    let text = if portrait { "Portrait" } else { "Landscape" };
                    crate::fields::combo_box(ui, "page_orientation").selected_text(text).show_ui(ui, |ui| {
                        ui.selectable_value(&mut portrait, false, "Landscape");
                        ui.selectable_value(&mut portrait, true, "Portrait");
                    });
                    if portrait != sh.page.portrait {
                        sh.page.portrait = portrait;
                        self.update(cx, Object::Sheet(sh.clone()), false);
                    }
                    ui.end_row();
                }
            }
        });
    }

    /// The project's title block (the same on every sheet). `target` keys the text fields.
    fn title_block(&mut self, ui: &mut egui::Ui, cx: &mut Ctx, target: ObjectId) {
        crate::fields::section_note(ui, "Title block", "all sheets");
        let mut tb = cx.doc.project().schematic.title_block();
        let mut changed = false;
        grid(ui, |ui| {
            for (name, field) in [
                ("Title", &mut tb.title),
                ("Date", &mut tb.date),
                ("Revision", &mut tb.revision),
                ("Company", &mut tb.company),
            ] {
                label(ui, name);
                if let Some(v) = self.text(ui, target, name, field) {
                    *field = v;
                    changed = true;
                }
                ui.end_row();
            }
            // Soft-wrapped here; only its line breaks start lines in the frame.
            crate::fields::label_top(ui, "Comment");
            if let Some(v) = self.multiline(ui, target, "Comment", &tb.comment.clone()) {
                tb.comment = v;
                changed = true;
            }
            ui.end_row();
        });
        if changed {
            self.run(cx, Command::SetTitleBlock { title_block: tb }, false);
        }
    }

    fn single(&mut self, ui: &mut egui::Ui, cx: &mut Ctx, id: ObjectId) {
        let Some(object) = cx.doc.project().get(id) else { return };
        match object {
            Object::Symbol(s) => self.symbol(ui, cx, s),
            Object::Footprint(f) => self.footprint(ui, cx, f),
            Object::Sheet(mut sh) => {
                // Sub-sheets show their path; the root's ("/") says nothing.
                let path = if sh.is_root() { String::new() } else { cx.doc.project().schematic.display_path(sh.id) };
                heading(ui, "Sheet", &path);
                grid(ui, |ui| {
                    label(ui, "Name");
                    if let Some(name) = self.text(ui, id, "name", &sh.name) {
                        sh.name = name.trim().to_string();
                        self.update(cx, Object::Sheet(sh.clone()), false);
                    }
                    ui.end_row();
                    if !sh.is_root() {
                        label(ui, "Width");
                        if let Some(w) = self.length(ui, sh.size.w, 2.54) {
                            sh.size.w = w;
                            self.update(cx, Object::Sheet(sh.clone()), true);
                        }
                        ui.end_row();
                        label(ui, "Height");
                        // The top edge stays (the pins hang from it): the symbol grows
                        // downwards, in whole pin pitches.
                        if let Some(h) = self.length(ui, sh.size.h, 2.54) {
                            let h = Sheet::snap_size(Size { w: sh.size.w, h }).h;
                            sh.pos.y += sh.size.h - h;
                            sh.size.h = h;
                            self.update(cx, Object::Sheet(sh.clone()), true);
                        }
                        ui.end_row();
                        row(ui, "Position", fmt_point(sh.pos));
                    }
                    let s = &cx.doc.project().schematic;
                    let symbols = s.symbols.iter().filter(|y| y.sheet == sh.id).count();
                    row(ui, "Symbols", symbols.to_string());
                    row(ui, "Sub-sheets", s.children(sh.id).count().to_string());
                });
                self.page(ui, cx, sh.clone());
                self.title_block(ui, cx, id);
            }
            Object::Wire(w) => {
                heading(ui, "Wire", "");
                grid(ui, |ui| {
                    row(ui, "Start", fmt_point(w.a));
                    row(ui, "End", fmt_point(w.b));
                    row(ui, "Length", units::fmt((w.b - w.a).length().round() as Coord));
                    net_rows(ui, cx.doc, id);
                });
            }
            Object::BusWire(w) => {
                heading(ui, "Bus", "");
                grid(ui, |ui| {
                    row(ui, "Start", fmt_point(w.a));
                    row(ui, "End", fmt_point(w.b));
                    row(ui, "Length", units::fmt((w.b - w.a).length().round() as Coord));
                    bus_rows(ui, cx.doc, id);
                });
            }
            Object::BusEntry(mut e) => {
                heading(ui, "Bus entry", "");
                grid(ui, |ui| {
                    row(ui, "Position", fmt_point(e.pos));
                    label(ui, "Direction");
                    if let Some(dir) = crate::sch_ui::entry_dir_picker(ui, e.dir) {
                        e.dir = dir;
                        self.update(cx, Object::BusEntry(e.clone()), false);
                    }
                    ui.end_row();
                    bus_rows(ui, cx.doc, id);
                    net_rows(ui, cx.doc, id);
                });
            }
            Object::NoConnect(n) => {
                heading(ui, "No-connect flag", "");
                grid(ui, |ui| {
                    row(ui, "Position", fmt_point(n.pos));
                });
            }
            Object::PowerFlag(mut f) => {
                crate::fields::subject(
                    ui,
                    "Power flag",
                    "",
                    Some("Its net counts as powered: power inputs and inputs on it are driven."),
                );
                grid(ui, |ui| {
                    row(ui, "Position", fmt_point(f.pos));
                    label(ui, "Rotation");
                    if let Some(a) =
                        crate::fields::rotation_combo(ui, "flag_rot", f.rotation, ["Up", "Left", "Down", "Right"])
                    {
                        f.rotation = a;
                        self.update(cx, Object::PowerFlag(f.clone()), false);
                    }
                    ui.end_row();
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
                let bus = editor_core::bus::is_bus_name(&l.name);
                heading(ui, if bus { "Bus label" } else { "Label" }, "");
                grid(ui, |ui| {
                    label(ui, "Name");
                    if let Some(name) = self.text(ui, id, "name", &l.name) {
                        l.name = name;
                        self.update(cx, Object::Label(l.clone()), false);
                    }
                    ui.end_row();
                    label(ui, "Kind");
                    let mut kind = l.kind;
                    crate::fields::combo_box(ui, "label_kind").selected_text(kind.label()).show_ui(ui, |ui| {
                        for k in LabelKind::ALL {
                            ui.selectable_value(&mut kind, k, k.label());
                        }
                    });
                    if kind != l.kind {
                        l.kind = kind;
                        self.update(cx, Object::Label(l.clone()), false);
                    }
                    ui.end_row();
                    if l.kind.has_shape() {
                        label(ui, "Shape");
                        let mut shape = l.shape;
                        crate::fields::combo_box(ui, "label_shape").selected_text(shape.label()).show_ui(ui, |ui| {
                            for s in LabelShape::ALL {
                                ui.selectable_value(&mut shape, s, s.label());
                            }
                        });
                        if shape != l.shape {
                            l.shape = shape;
                            self.update(cx, Object::Label(l.clone()), false);
                        }
                        ui.end_row();
                    }
                    if l.kind == LabelKind::Power {
                        label(ui, "Style");
                        if let Some(style) = crate::fields::power_style_combo(ui, "power_style", l.style, &l.name) {
                            l.style = style;
                            self.update(cx, Object::Label(l.clone()), false);
                        }
                        ui.end_row();
                    }
                    row(ui, "Position", fmt_point(l.pos));
                    if bus {
                        bus_rows(ui, cx.doc, id);
                    } else {
                        net_rows(ui, cx.doc, id);
                    }
                });
            }
            Object::SchText(mut t) => {
                heading(ui, "Text", "");
                grid(ui, |ui| {
                    label(ui, "Text");
                    if let Some(s) = self.multiline(ui, id, "text", &t.text.text) {
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
                    label(ui, "Bold");
                    if ui.checkbox(&mut t.text.bold, "").changed() {
                        self.update(cx, Object::SchText(t.clone()), false);
                    }
                    ui.end_row();
                });
            }
            Object::Track(mut t) => {
                let arc = t.arc();
                heading(ui, if arc.is_some() { "Arc track" } else { "Track" }, "");
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
                    if let Some(Shape::Arc { radius, sweep, .. }) = arc {
                        row(ui, "Radius", units::fmt(radius));
                        row(ui, "Angle", format!("{:.1}°", sweep.abs()));
                    }
                    row(ui, "Length", units::fmt(t.length().round() as Coord));
                    label(ui, "Locked");
                    if ui.checkbox(&mut t.locked, "").on_hover_text("The router never pushes it aside").changed() {
                        self.update(cx, Object::Track(t.clone()), false);
                    }
                    ui.end_row();
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
                    // Blind and buried vias: only with inner layers.
                    if cx.doc.project().board.copper_layers().count() > 2 {
                        for (name, is_from) in [("From layer", true), ("To layer", false)] {
                            label(ui, name);
                            let cur = if is_from { v.from } else { v.to };
                            if let Some(l) = copper_layer_combo_salt(ui, cx.doc.project(), cur, name) {
                                let other = if is_from { v.to } else { v.from };
                                if l != other {
                                    (v.from, v.to) = if l.0 < other.0 { (l, other) } else { (other, l) };
                                    self.update(cx, Object::Via(v.clone()), true);
                                }
                            }
                            ui.end_row();
                        }
                        row(ui, "Kind", v.kind().to_string());
                    }
                    label(ui, "Locked");
                    if ui.checkbox(&mut v.locked, "").on_hover_text("The router never pushes it aside").changed() {
                        self.update(cx, Object::Via(v.clone()), false);
                    }
                    ui.end_row();
                    copper_net_row(ui, cx.doc, CopperItem::Via(v.id));
                });
            }
            Object::Zone(mut z) => {
                let rules = cx.doc.project().board.rules.clone();
                crate::fields::subject(
                    ui,
                    "Copper zone",
                    if z.net.is_empty() { "no net" } else { &z.net },
                    Some("An empty net makes isolated copper: it keeps the clearance to all pads and tracks."),
                );
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
                    if crate::fields::fill(ui, egui::DragValue::new(&mut prio).range(0..=100)).changed() {
                        z.priority = prio;
                        self.update(cx, Object::Zone(z.clone()), true);
                    }
                    ui.end_row();
                    label(ui, "Pads");
                    let mut connection = z.connection;
                    crate::fields::combo_box(ui, "zone_connection").selected_text(connection.label()).show_ui(
                        ui,
                        |ui| {
                            for c in ZoneConnection::ALL {
                                ui.selectable_value(&mut connection, c, c.label());
                            }
                        },
                    );
                    if connection != z.connection {
                        z.connection = connection;
                        self.update(cx, Object::Zone(z.clone()), false);
                    }
                    ui.end_row();
                    // Overrides of the design rules: the rule's value until edited.
                    let overrides: [(&str, ZoneField, Coord, f64); 3] = [
                        ("Clearance", |z| &mut z.clearance, rules.clearance, 0.0),
                        ("Thermal gap", |z| &mut z.thermal_gap, rules.thermal_gap, 0.01),
                        ("Spoke width", |z| &mut z.thermal_spoke, rules.thermal_spoke, 0.01),
                    ];
                    for (name, field, rule, min) in overrides {
                        label(ui, name);
                        ui.horizontal(|ui| {
                            let current = *field(&mut z);
                            // Room for the "Rule" button or label after the field.
                            let w = ui.available_width() - 52.0;
                            let v = crate::fields::sized(ui, w, |ui| self.length(ui, current.unwrap_or(rule), min));
                            if let Some(v) = v {
                                *field(&mut z) = Some(v);
                                self.update(cx, Object::Zone(z.clone()), true);
                            }
                            if current.is_some() {
                                if ui.small_button("Rule").on_hover_text("Use the design rule").clicked() {
                                    *field(&mut z) = None;
                                    self.update(cx, Object::Zone(z.clone()), false);
                                }
                            } else {
                                ui.label(RichText::new("rule").color(theme::text_dim()));
                            }
                        });
                        ui.end_row();
                    }
                    label(ui, "Remove islands");
                    let names = |i: IslandRemoval| match i {
                        IslandRemoval::Always => "Always",
                        IslandRemoval::Never => "Never",
                        IslandRemoval::BelowArea(_) => "Below an area",
                    };
                    let mut islands = z.islands;
                    crate::fields::combo_box(ui, "zone_islands").selected_text(names(islands)).show_ui(ui, |ui| {
                        ui.selectable_value(&mut islands, IslandRemoval::Always, "Always");
                        ui.selectable_value(&mut islands, IslandRemoval::Never, "Never");
                        let below = match z.islands {
                            b @ IslandRemoval::BelowArea(_) => b,
                            _ => IslandRemoval::BelowArea(10_000_000_000_000),
                        };
                        ui.selectable_value(&mut islands, below, "Below an area");
                    });
                    if islands != z.islands {
                        z.islands = islands;
                        self.update(cx, Object::Zone(z.clone()), false);
                    }
                    ui.end_row();
                    if let IslandRemoval::BelowArea(area) = z.islands {
                        label(ui, "Minimum area");
                        let mut mm2 = area as f64 * 1e-12;
                        let drag = egui::DragValue::new(&mut mm2).range(0.0..=1e6).speed(0.1).suffix(" mm²");
                        if ui.add(drag).changed() {
                            z.islands = IslandRemoval::BelowArea((mm2 * 1e12).round() as i64);
                            self.update(cx, Object::Zone(z.clone()), true);
                        }
                        ui.end_row();
                    }
                    row(ui, "Corners", z.outline.len().to_string());
                    if !z.holes.is_empty() {
                        row(ui, "Holes", z.holes.len().to_string());
                    }
                });
            }
            Object::Keepout(mut k) => {
                crate::fields::subject(
                    ui,
                    "Keepout",
                    "",
                    Some(
                        "Zone fills leave the area out; DRC reports tracks, vias, pads and footprints (courtyards) in it.",
                    ),
                );
                grid(ui, |ui| {
                    label(ui, "Layers");
                    let mut all = k.layers.is_empty();
                    if ui.checkbox(&mut all, "All copper layers").changed() {
                        k.layers = if all { Vec::new() } else { vec![LayerId::TOP_COPPER] };
                        self.update(cx, Object::Keepout(k.clone()), false);
                    }
                    ui.end_row();
                    if !all {
                        let layers: Vec<(LayerId, String)> =
                            cx.doc.project().board.copper_layers().map(|l| (l.id, l.name.clone())).collect();
                        for (layer, name) in layers {
                            ui.label("");
                            let mut on = k.layers.contains(&layer);
                            if ui.checkbox(&mut on, name).changed() {
                                k.layers.retain(|&l| l != layer);
                                if on {
                                    k.layers.push(layer);
                                    k.layers.sort();
                                }
                                // No layer left means all of them: keep at least one.
                                if !k.layers.is_empty() {
                                    self.update(cx, Object::Keepout(k.clone()), false);
                                }
                            }
                            ui.end_row();
                        }
                    }
                    let mut changed = false;
                    let flags = [
                        (&mut k.no_tracks, "Tracks"),
                        (&mut k.no_vias, "Vias"),
                        (&mut k.no_pads, "Pads"),
                        (&mut k.no_pour, "Zone fills"),
                        (&mut k.no_footprints, "Footprints"),
                    ];
                    for (i, (flag, name)) in flags.into_iter().enumerate() {
                        label(ui, if i == 0 { "Keep out" } else { "" });
                        changed |= ui.checkbox(flag, name).changed();
                        ui.end_row();
                    }
                    if changed {
                        self.update(cx, Object::Keepout(k.clone()), false);
                    }
                    row(ui, "Corners", k.outline.len().to_string());
                    if !k.holes.is_empty() {
                        row(ui, "Holes", k.holes.len().to_string());
                    }
                });
            }
            Object::BoardGraphic(g) => {
                heading(ui, "Graphic", "");
                grid(ui, |ui| {
                    row(ui, "Layer", layer_name(cx.doc.project(), g.layer));
                    row(ui, "Shape", shape_name(&g.shape).into());
                });
            }
            Object::FootprintField(mut f) => {
                let project = cx.doc.project();
                let Some(fp) = project.board.footprints.get(f.footprint).cloned() else { return };
                let c = project.schematic.components.get(fp.component);
                let (refdes, value) =
                    c.map_or(("?".to_string(), String::new()), |c| (c.refdes.clone(), c.value.clone()));
                heading(ui, &format!("{} text", f.label()), &refdes);
                ui.label(RichText::new("Moves with its footprint. Drag it, or R to turn it.").color(theme::text_dim()));
                ui.add_space(4.0);
                grid(ui, |ui| {
                    row(ui, "Text", f.text.text.replace("${REF}", &refdes).replace("${VALUE}", &value));
                    label(ui, "Layer");
                    let mut layer = f.layer;
                    crate::fields::combo_box(ui, "field_layer").selected_text(format!("{layer:?}")).show_ui(ui, |ui| {
                        ui.selectable_value(&mut layer, FootprintLayer::Silkscreen, "Silkscreen");
                        ui.selectable_value(&mut layer, FootprintLayer::Fabrication, "Fabrication");
                    });
                    if layer != f.layer {
                        f.layer = layer;
                        self.update(cx, Object::FootprintField(f.clone()), false);
                    }
                    ui.end_row();
                    label(ui, "Size");
                    if let Some(v) = self.length(ui, f.text.size, 0.1) {
                        f.text.size = v;
                        self.update(cx, Object::FootprintField(f.clone()), true);
                    }
                    ui.end_row();
                    label(ui, "Bold");
                    if ui.checkbox(&mut f.text.bold, "").changed() {
                        self.update(cx, Object::FootprintField(f.clone()), false);
                    }
                    ui.end_row();
                    label(ui, "Offset");
                    if let Some(p) = self.point(ui, f.text.pos) {
                        f.text.pos = p;
                        self.update(cx, Object::FootprintField(f.clone()), true);
                    }
                    ui.end_row();
                    label(ui, "Rotation");
                    if let Some(r) = rotation_combo(ui, "field_rot", f.text.rotation) {
                        f.text.rotation = r;
                        self.update(cx, Object::FootprintField(f.clone()), false);
                    }
                    ui.end_row();
                    label(ui, "Visible");
                    let mut visible = !f.hidden;
                    if ui.checkbox(&mut visible, "").changed() {
                        f.hidden = !visible;
                        self.update(cx, Object::FootprintField(f.clone()), false);
                    }
                    ui.end_row();
                });
                let project = cx.doc.project();
                let template = editor_core::query::footprint_of_instance(project, &fp).and_then(|def| {
                    FootprintField::for_footprint(fp.id, def).into_iter().find(|t| t.text.text == f.text.text)
                });
                if let Some(t) = template
                    && (t.text != f.text || t.layer != f.layer || t.back != f.back)
                    && ui.button("Reset to library position").clicked()
                {
                    (f.text, f.layer, f.back) = (t.text, t.layer, t.back);
                    self.update(cx, Object::FootprintField(f), false);
                }
            }
            Object::SymbolField(mut f) => {
                let project = cx.doc.project();
                let Some(sym) = project.schematic.symbols.get(f.symbol).cloned() else { return };
                let c = project.schematic.components.get(sym.component);
                let (refdes, value) =
                    c.map_or(("?".to_string(), String::new()), |c| (c.refdes.clone(), c.value.clone()));
                heading(ui, &format!("{} text", f.label()), &refdes);
                ui.label(
                    RichText::new("Moves with its symbol. Drag it, R to turn it, double-click to edit.")
                        .color(theme::text_dim()),
                );
                ui.add_space(4.0);
                grid(ui, |ui| {
                    row(ui, "Text", f.text.text.replace("${REF}", &refdes).replace("${VALUE}", &value));
                    label(ui, "Size");
                    if let Some(v) = self.length(ui, f.text.size, 0.1) {
                        f.text.size = v;
                        self.update(cx, Object::SymbolField(f.clone()), true);
                    }
                    ui.end_row();
                    label(ui, "Bold");
                    if ui.checkbox(&mut f.text.bold, "").changed() {
                        self.update(cx, Object::SymbolField(f.clone()), false);
                    }
                    ui.end_row();
                    label(ui, "Offset");
                    if let Some(p) = self.point(ui, f.text.pos) {
                        f.text.pos = p;
                        self.update(cx, Object::SymbolField(f.clone()), true);
                    }
                    ui.end_row();
                    label(ui, "Rotation");
                    if let Some(r) = rotation_combo(ui, "sym_field_rot", f.text.rotation) {
                        f.text.rotation = r;
                        self.update(cx, Object::SymbolField(f.clone()), false);
                    }
                    ui.end_row();
                    label(ui, "Visible");
                    let mut visible = !f.hidden;
                    if ui.checkbox(&mut visible, "").changed() {
                        f.hidden = !visible;
                        self.update(cx, Object::SymbolField(f.clone()), false);
                    }
                    ui.end_row();
                });
                let project = cx.doc.project();
                let template = editor_core::query::symbol_of_instance(project, &sym).and_then(|def| {
                    SymbolField::for_symbol(sym.id, def).into_iter().find(|t| t.text.text == f.text.text)
                });
                if let Some(t) = template
                    && t.text != f.text
                    && ui.button("Reset to library position").clicked()
                {
                    f.text = t.text;
                    self.update(cx, Object::SymbolField(f), false);
                }
            }
            Object::BoardText(mut t) => {
                heading(ui, "Board text", "");
                grid(ui, |ui| {
                    label(ui, "Text");
                    if let Some(s) = self.multiline(ui, id, "text", &t.text.text) {
                        t.text.text = s;
                        self.update(cx, Object::BoardText(t.clone()), false);
                    }
                    ui.end_row();
                    row(ui, "Layer", layer_name(cx.doc.project(), t.layer));
                    label(ui, "Size");
                    if let Some(v) = self.length(ui, t.text.size, 0.1) {
                        t.text.size = v;
                        self.update(cx, Object::BoardText(t.clone()), true);
                    }
                    ui.end_row();
                    label(ui, "Bold");
                    if ui.checkbox(&mut t.text.bold, "").changed() {
                        self.update(cx, Object::BoardText(t.clone()), false);
                    }
                    ui.end_row();
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
        // Text buffers are kept for selected objects only: key them by the symbol.
        let target = ObjectId::Symbol(s.id);
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
            self.variant_combo(ui, cx, &c, &part, false);
            ui.end_row();
            self.dnp_row(ui, cx, &c);
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
            // Show or hide its reference and value texts (hidden ones can't be clicked).
            let fields: Vec<SymbolField> =
                cx.doc.project().schematic.fields.iter().filter(|x| x.symbol == s.id).cloned().collect();
            for mut field in fields {
                label(ui, &format!("{} text", field.label()));
                let mut visible = !field.hidden;
                if ui.checkbox(&mut visible, "").on_hover_text("Shown on the canvas").changed() {
                    field.hidden = !visible;
                    self.update(cx, Object::SymbolField(field), false);
                }
                ui.end_row();
            }
        });
        if let Some(new) = self.sim.ui(ui, cx.doc.project(), &c) {
            self.update(cx, Object::Component(new), false);
        }
        pin_table(ui, cx.doc, Some(s.id), c.id);
    }

    /// "Do not populate": left out of the BOM and the position file.
    fn dnp_row(&mut self, ui: &mut egui::Ui, cx: &mut Ctx, c: &Component) {
        let mut dnp = c.dnp;
        if crate::fields::check_row(ui, "Do not populate", &mut dnp).on_hover_text(DNP_TIP).changed() {
            self.update(cx, Object::Component(Component { dnp, ..c.clone() }), false);
        }
    }

    /// The component's footprint variant. `on_board`: only variants with a footprint (on
    /// the board, a "no footprint" variant would delete the selected footprint).
    fn variant_combo(&mut self, ui: &mut egui::Ui, cx: &mut Ctx, c: &Component, part: &Part, on_board: bool) {
        let current = part.variant(c.variant).map_or("?".to_string(), |v| v.name.clone());
        let mut variant = c.variant;
        crate::fields::combo_box(ui, "variant").selected_text(current).show_ui(ui, |ui| {
            for v in part.variants.iter().filter(|v| !on_board || v.footprint.is_some()) {
                ui.selectable_value(&mut variant, v.id, &v.name);
            }
        });
        if variant != c.variant {
            let cmd = Command::SetComponent { id: c.id, refdes: None, value: None, variant: Some(variant) };
            self.run(cx, cmd, false);
        }
    }

    fn footprint(&mut self, ui: &mut egui::Ui, cx: &mut Ctx, mut f: FootprintInstance) {
        let project = cx.doc.project();
        let Some(c) = project.schematic.components.get(f.component).cloned() else { return };
        let Some(part) = project.library.parts.get(c.part).cloned() else { return };
        let fp_name = project.library.footprint_of(c.part, c.variant).map_or("?".into(), |fp| fp.name.clone());
        heading(ui, &c.refdes, &fp_name);
        if part.is_board_part() {
            let note = "Board part: placed on the board only, no symbol. Its pads have no net.";
            ui.label(RichText::new(note).color(theme::text_dim()));
        }
        ui.add_space(4.0);
        // Text buffers are kept for selected objects only: key them by the footprint.
        let target = ObjectId::Footprint(f.id);
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
            self.variant_combo(ui, cx, &c, &part, true);
            ui.end_row();
            if !part.is_board_part() {
                self.dnp_row(ui, cx, &c);
            }
            label(ui, "Side");
            let mut side = f.side;
            crate::fields::combo_box(ui, "side").selected_text(format!("{side:?}")).show_ui(ui, |ui| {
                ui.selectable_value(&mut side, Side::Top, "Top");
                ui.selectable_value(&mut side, Side::Bottom, "Bottom");
            });
            if side != f.side {
                let cmd =
                    Command::FlipSide { ids: vec![ObjectId::Footprint(f.id)], axis_x: f.pos.x, keep_connected: true };
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
            // Show or hide its reference and value texts (hidden ones can't be clicked).
            let fields: Vec<FootprintField> =
                cx.doc.project().board.fields.iter().filter(|x| x.footprint == f.id).cloned().collect();
            for mut field in fields {
                let layer = if field.layer == FootprintLayer::Fabrication { " (fab)" } else { "" };
                label(ui, &format!("{} text{layer}", field.label()));
                let mut visible = !field.hidden;
                if ui.checkbox(&mut visible, "").on_hover_text("Shown on the canvas").changed() {
                    field.hidden = !visible;
                    self.update(cx, Object::FootprintField(field), false);
                }
                ui.end_row();
            }
        });
        if !part.is_board_part() {
            pin_table(ui, cx.doc, None, c.id);
        }
    }
}

/// The selected object's subject line.
fn heading(ui: &mut egui::Ui, title: &str, subtitle: &str) {
    crate::fields::subject(ui, title, subtitle, None);
}

fn grid(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui)) {
    crate::fields::grid(ui, ui.next_auto_id(), add);
}

use crate::fields::{label, row};

fn fmt_point(p: Point) -> String {
    units::fmt_point(p)
}

fn rotation_combo(ui: &mut egui::Ui, salt: &str, current: Angle) -> Option<Angle> {
    let mut deg = current.degrees();
    crate::fields::combo_box(ui, salt).selected_text(format!("{deg:.0}°")).show_ui(ui, |ui| {
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
    copper_layer_combo_salt(ui, project, current, "copper_layer")
}

fn copper_layer_combo_salt(ui: &mut egui::Ui, project: &Project, current: LayerId, salt: &str) -> Option<LayerId> {
    let mut layer = current;
    crate::fields::combo_box(ui, salt).selected_text(layer_name(project, current)).show_ui(ui, |ui| {
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

/// Name and members of the bus of a bus wire, entry or label.
fn bus_rows(ui: &mut egui::Ui, doc: &Document, id: ObjectId) {
    let Some(bus) = doc.connectivity().bus_of(id) else {
        if matches!(id, ObjectId::BusEntry(_) | ObjectId::Label(_)) {
            label(ui, "Problem");
            ui.label(RichText::new("not on a bus").color(theme::warn()));
            ui.end_row();
        }
        return;
    };
    if bus.name.is_empty() {
        label(ui, "Bus");
        ui.label(RichText::new("unnamed (add a label like D[0..7])").color(theme::warn()));
        ui.end_row();
        return;
    }
    row(ui, "Bus", bus.name.clone());
    const MAX: usize = 64;
    let mut members: Vec<&str> = bus.members.iter().take(MAX).map(String::as_str).collect();
    let more = format!("… {} more", bus.members.len().saturating_sub(MAX));
    if bus.members.len() > MAX {
        members.push(&more);
    }
    label(ui, &format!("Members ({})", bus.members.len()));
    ui.add(egui::Label::new(RichText::new(members.join(" ")).monospace()).wrap());
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
    crate::fields::collapsing(ui, &format!("Pins ({})", pins.len()), "pins", |ui| {
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
    });
}

fn simple_pin_table(ui: &mut egui::Ui, pins: &[(String, Vec<String>)]) {
    if pins.is_empty() {
        return;
    }
    ui.add_space(6.0);
    crate::fields::collapsing(ui, &format!("Pins ({})", pins.len()), "pins", |ui| {
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
    });
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

const DNP_TIP: &str = "On the board (pads, copper) but not assembled: left out of the BOM and the position file";

/// "Do not populate" for the components of several selected symbols and footprints
/// (board parts aside): ticked when all are; one command sets them all.
fn dnp_many(ui: &mut egui::Ui, cx: &mut Ctx, ids: &[ObjectId]) {
    let p = cx.doc.project();
    let mut comps: Vec<Component> = Vec::new();
    for id in ids {
        let component = match id {
            ObjectId::Symbol(s) => p.schematic.symbols.get(*s).map(|s| s.component),
            ObjectId::Footprint(f) => p.board.footprints.get(*f).map(|f| f.component),
            _ => None,
        };
        let Some(c) = component.and_then(|c| p.schematic.components.get(c)) else { continue };
        let board_part = p.library.parts.get(c.part).is_some_and(|part| part.is_board_part());
        if !board_part && !comps.iter().any(|x| x.id == c.id) {
            comps.push(c.clone());
        }
    }
    if comps.is_empty() {
        return;
    }
    let mut all = comps.iter().all(|c| c.dnp);
    crate::fields::section(ui, "Assembly", None);
    let mut changed = false;
    grid(ui, |ui| {
        changed = crate::fields::check_row(ui, "Do not populate", &mut all).on_hover_text(DNP_TIP).changed();
    });
    if changed {
        let objects = comps
            .into_iter()
            .map(|mut c| {
                c.dnp = all;
                Object::Component(c)
            })
            .collect();
        if let Err(e) = cx.doc.execute(Command::Update { objects }) {
            *cx.message = Some(e.to_string());
        }
    }
}

/// "Exclude from simulation" for several selected symbols (all of them simulated):
/// ticked when all are excluded; one command sets them all.
fn exclude_many(ui: &mut egui::Ui, cx: &mut Ctx, ids: &[ObjectId]) {
    let p = cx.doc.project();
    let mut comps: Vec<Component> = Vec::new();
    for id in ids {
        let ObjectId::Symbol(s) = id else { return };
        let Some(c) = p.schematic.symbols.get(*s).and_then(|s| p.schematic.components.get(s.component)) else { return };
        let simulated = p
            .library
            .parts
            .get(c.part)
            .and_then(|part| part.sim.as_ref())
            .is_some_and(|m| m.device != SimDevice::Ignore);
        if !simulated {
            return;
        }
        if !comps.iter().any(|x| x.id == c.id) {
            comps.push(c.clone());
        }
    }
    let mut all = comps.iter().all(|c| c.sim.exclude);
    crate::fields::section(ui, "Simulation", None);
    let mut changed = false;
    grid(ui, |ui| {
        changed = crate::fields::check_row(ui, "Exclude", &mut all)
            .on_hover_text("Left out of the netlist, as if they weren't there")
            .changed();
    });
    if changed {
        let objects = comps
            .into_iter()
            .map(|mut c| {
                c.sim.exclude = all;
                Object::Component(c)
            })
            .collect();
        if let Err(e) = cx.doc.execute(Command::Update { objects }) {
            *cx.message = Some(e.to_string());
        }
    }
}

/// Project statistics (Overview panel), by area. Counts of problems show in the warning
/// colour when not zero.
pub fn project_summary(ui: &mut egui::Ui, doc: &Document) {
    use crate::fields::{problem_row, section};
    let p = doc.project();
    let conn = doc.connectivity();
    section(ui, "Schematic", None);
    grid(ui, |ui| {
        row(ui, "Components", p.schematic.components.len().to_string());
        row(ui, "Wires", p.schematic.wires.len().to_string());
        row(ui, "Labels", p.schematic.labels.len().to_string());
        if !p.schematic.bus_wires.is_empty() {
            row(ui, "Buses", conn.buses.len().to_string());
        }
        row(ui, "Nets", conn.netlist().filter(|n| n.pins.len() > 1 || n.named).count().to_string());
        problem_row(ui, "Unconnected pins", conn.unconnected.len(), None);
        let severities = conn.problems.iter().filter(|p| !p.excluded).map(|p| p.severity);
        checks_row(ui, "Electrical rules", Some(severities));
    });
    section(ui, "Board", None);
    grid(ui, |ui| {
        row(ui, "Copper layers", p.board.copper_layers().count().to_string());
        row(ui, "Footprints", p.board.footprints.len().to_string());
        row(ui, "Tracks", p.board.tracks.len().to_string());
        row(ui, "Vias", p.board.vias.len().to_string());
        problem_row(ui, "Unrouted", doc.copper().unrouted(), Some("connections"));
        let drc = doc.drc().map(|r| r.violations.iter().filter(|v| !v.excluded).map(|v| v.severity));
        checks_row(ui, "Design rules", drc);
    });
    section(ui, "Library", None);
    grid(ui, |ui| {
        row(ui, "Parts", p.library.parts.len().to_string());
        row(ui, "Symbols", p.library.symbols.len().to_string());
        row(ui, "Footprints", p.library.footprints.len().to_string());
    });
}

/// Errors and warnings of a check ("not run" without findings to count).
fn checks_row(ui: &mut egui::Ui, name: &str, findings: Option<impl Iterator<Item = Severity>>) {
    let Some(findings) = findings else {
        return row(ui, name, "not run".into());
    };
    let (mut errors, mut warnings) = (0, 0);
    for s in findings {
        match s {
            Severity::Error => errors += 1,
            Severity::Warning => warnings += 1,
            Severity::Ignore => {}
        }
    }
    let plural = |n: usize, what: &str| format!("{n} {what}{}", if n == 1 { "" } else { "s" });
    let text = match (errors, warnings) {
        (0, 0) => "OK".to_string(),
        (e, 0) => plural(e, "error"),
        (0, w) => plural(w, "warning"),
        (e, w) => format!("{}, {}", plural(e, "error"), plural(w, "warning")),
    };
    label(ui, name);
    let color = if errors > 0 {
        theme::error()
    } else if warnings > 0 {
        theme::warn()
    } else {
        ui.visuals().text_color()
    };
    ui.label(RichText::new(text).color(color));
    ui.end_row();
}

/// A zone setting that overrides a design rule.
type ZoneField = fn(&mut Zone) -> &mut Option<Coord>;
