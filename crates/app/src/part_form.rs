//! Part form: name and defaults, units (symbols), footprint variants and the pin-pad
//! mapping of each variant, the simulation model (`sim_form`). Every change is one undoable `update` of the part.

use egui::RichText;
use model::*;
use view::LibraryDoc;
use view::item::{Item, PartPreview};

use crate::fields::{self, Fields, hint, label};
use crate::library::{Open, issues_ui};
use crate::preview::{Empty, Preview};
use crate::theme;

#[derive(Default)]
pub struct PartForm {
    fields: Fields,
    new_property: String,
    new_sim_param: String,
    /// The part whose unit and variant `shown` chooses (reset when another opens).
    shown_part: Option<Id>,
    shown: PartPreview,
    symbol_preview: Preview,
    footprint_preview: Preview,
}

/// Height of the symbol and footprint previews.
const PREVIEW_HEIGHT: f32 = 200.0;

impl PartForm {
    /// Symbol (with a unit choice) and footprint (with a variant choice) side by side.
    /// A click on a preview opens the item.
    fn preview_ui(&mut self, ui: &mut egui::Ui, lib: &LibraryDoc, part: &Part) -> Option<Open> {
        if self.shown_part != Some(part.id) {
            self.shown_part = Some(part.id);
            self.shown = PartPreview::default();
        }
        let (symbol, footprint) = self.shown.items(lib.library(), part);
        let gap = ui.spacing().item_spacing.x * 2.0;
        let width = ((ui.available_width() - gap) / 2.0).max(120.0);
        let size = egui::vec2(width, PREVIEW_HEIGHT);
        let mut open = None;
        let mut clicked = |r: egui::Response, item: Option<Item>| {
            if let Some(item) = item
                && r.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text("Click to edit").clicked()
            {
                open = Some(Open::Item(item));
            }
        };
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            ui.vertical(|ui| {
                ui.set_width(width);
                caption_row(ui, "Symbol", |ui| {
                    if part.units.len() > 1 {
                        let units: Vec<(usize, String)> =
                            part.units.iter().enumerate().map(|(i, u)| (i, format!("Unit {}", u.name))).collect();
                        if let Some(u) =
                            fields::combo(ui, ("preview_unit", part.id), &self.shown.unit_index(part), &units)
                        {
                            self.shown.unit = u;
                        }
                    }
                });
                let placeholder = if part.units.is_empty() { "Board part: no symbol" } else { "Symbol missing" };
                let r = self.symbol_preview.ui(ui, &lib.doc, symbol, size, Empty::Hint(placeholder));
                clicked(r, symbol);
            });
            ui.vertical(|ui| {
                ui.set_width(width);
                let variant = self.shown.variant(part);
                caption_row(ui, "Footprint", |ui| {
                    if part.variants.len() > 1 {
                        let variants: Vec<(Option<Id>, String)> =
                            part.variants.iter().map(|v| (Some(v.id), v.name.clone())).collect();
                        if let Some(v) =
                            fields::combo(ui, ("preview_variant", part.id), &variant.map(|v| v.id), &variants)
                        {
                            self.shown.variant = v;
                        }
                    }
                });
                let placeholder =
                    if variant.is_some_and(|v| v.footprint.is_some()) { "Footprint missing" } else { "No footprint" };
                let r = self.footprint_preview.ui(ui, &lib.doc, footprint, size, Empty::Hint(placeholder));
                clicked(r, footprint);
            });
        });
        open
    }
}

/// A dim caption over a preview, with its selector (if any) on the right.
fn caption_row(ui: &mut egui::Ui, caption: &str, selector: impl FnOnce(&mut egui::Ui)) {
    ui.horizontal(|ui| {
        ui.set_min_height(24.0);
        ui.label(RichText::new(caption).color(theme::text_dim()));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), selector);
    });
}

/// Pin choices of a part: (unit, pin id, label).
fn pin_options(lib: &Library, part: &Part) -> Vec<(Option<(u32, Id)>, String)> {
    let multi = part.units.len() > 1;
    let mut out = vec![(None, "— not connected —".to_string())];
    for (u, unit) in part.units.iter().enumerate() {
        let Some(sym) = lib.symbols.get(unit.symbol) else { continue };
        let mut pins: Vec<&Pin> = sym.pins.iter().collect();
        pins.sort_by(|a, b| natural_cmp(&a.name, &b.name));
        for pin in pins {
            let pin_label = match pin.number.as_str() {
                "" => pin.name.clone(),
                n if n == pin.name => pin.name.clone(),
                n => format!("{n} {}", pin.name),
            };
            let label = if multi { format!("{}: {pin_label}", unit.name) } else { pin_label };
            out.push((Some((u as u32, pin.id)), label));
        }
    }
    out
}

impl PartForm {
    /// Returns an item to open when the user asks to edit a symbol or footprint.
    pub fn ui(
        &mut self,
        ui: &mut egui::Ui,
        lib: &mut LibraryDoc,
        id: Id,
        message: &mut Option<String>,
    ) -> Option<Open> {
        let original = lib.library().parts.get(id).cloned()?;
        let mut part = original.clone();
        let mut open = None;
        self.fields.begin_frame();
        ui.set_max_width(ui.available_width().min(760.0));

        fields::subject(ui, &part.name, "Part", None);
        ui.add_space(6.0);
        let categories = fields::categories(lib.library(), false);
        egui::Grid::new(("part_fields", id)).num_columns(2).spacing([16.0, 6.0]).show(ui, |ui| {
            let f = &mut self.fields;
            label(ui, "Name");
            if let Some(v) = f.text_sized(ui, ("p_name", id), &part.name, 300.0) {
                part.name = v;
            }
            ui.end_row();
            label(ui, "Category");
            if let Some(v) = f.category(ui, ("p_category", id), &part.category, &categories) {
                part.category = v;
            }
            ui.end_row();
            label(ui, "Reference prefix");
            if let Some(v) = f.text_sized(ui, ("p_prefix", id), &part.prefix, 80.0) {
                part.prefix = v.trim().to_string();
            }
            ui.end_row();
            label(ui, "Default value");
            if let Some(v) = f.text_sized(ui, ("p_value", id), &part.default_value, 300.0) {
                part.default_value = v;
            }
            ui.end_row();
            label(ui, "Description");
            if let Some(v) = f.text_sized(ui, ("p_desc", id), &part.description, 500.0) {
                part.description = v;
            }
            ui.end_row();
        });

        ui.add_space(12.0);
        if let Some(o) = self.preview_ui(ui, lib, &part) {
            open = Some(o);
        }

        // ---- Units
        fields::section(ui, "Symbol units", None);
        hint(ui, "One symbol per unit (several units: e.g. the four gates of a 74HC00).");
        let lib_ref = lib.library();
        let symbols: Vec<(Id, String)> = {
            let mut v: Vec<(Id, String)> = lib_ref.symbols.iter().map(|s| (s.id, s.name.clone())).collect();
            v.sort_by(|a, b| natural_cmp(&a.1, &b.1));
            v
        };
        let footprints: Vec<(Id, String)> = {
            let mut v: Vec<(Id, String)> = lib_ref.footprints.iter().map(|f| (f.id, f.name.clone())).collect();
            v.sort_by(|a, b| natural_cmp(&a.1, &b.1));
            v
        };
        let mut remove_unit = None;
        egui::Grid::new(("units", id)).num_columns(4).spacing([12.0, 6.0]).show(ui, |ui| {
            for (i, unit) in part.units.iter_mut().enumerate() {
                if let Some(v) = self.fields.text_sized(ui, ("unit_name", id, i), &unit.name, 60.0) {
                    unit.name = v;
                }
                if let Some(s) = fields::combo(ui, ("unit_sym", id, i), &unit.symbol, &symbols) {
                    unit.symbol = s;
                }
                if ui.button("Edit symbol").clicked() {
                    open = Some(Open::Item(Item::Symbol(unit.symbol)));
                }
                if ui.button("Remove").clicked() {
                    remove_unit = Some(i);
                }
                ui.end_row();
            }
        });
        if let Some(i) = remove_unit {
            part.units.remove(i);
            for v in &mut part.variants {
                v.connections.retain(|c| c.unit != i as u32);
                for c in &mut v.connections {
                    if c.unit > i as u32 {
                        c.unit -= 1;
                    }
                }
            }
        }
        if part.units.is_empty() {
            hint(ui, "Board part: placed on the PCB only, no symbol. Its pads have no net.");
        }
        if ui.add_enabled(!symbols.is_empty(), egui::Button::new("Add unit")).clicked() {
            let symbol = part.units.last().map_or(symbols[0].0, |u| u.symbol);
            let name = ((b'A' + part.units.len().min(25) as u8) as char).to_string();
            part.units.push(PartUnit { name, symbol });
        }
        // Units whose symbol changed lose their connections (pin ids differ).
        for (i, (old, new)) in original.units.iter().zip(&part.units).enumerate() {
            if old.symbol != new.symbol {
                for v in &mut part.variants {
                    v.connections.retain(|c| c.unit != i as u32);
                }
            }
        }

        // ---- Variants
        fields::section(
            ui,
            "Footprint variants",
            Some(
                "Each variant is a footprint with its own pin-pad mapping (e.g. 0603 / 0805). A variant \
                 without footprint keeps its components off the board.",
            ),
        );
        // Footprint choices of a variant: none, or one of the library's.
        // Board parts always have a footprint.
        let none = (!part.is_board_part()).then(|| (None, "None (not on board)".to_string()));
        let fp_options: Vec<(Option<Id>, String)> =
            none.into_iter().chain(footprints.iter().map(|(id, n)| (Some(*id), n.clone()))).collect();
        // A variant named after its footprint (or "No footprint") follows a footprint change.
        let auto_name = |fp: Option<Id>| {
            fp.map_or(PartVariant::NO_FOOTPRINT.to_string(), |f| {
                lib_ref.footprints.get(f).map_or("Variant".into(), |f| f.name.clone())
            })
        };
        let options = pin_options(lib_ref, &part);
        let mut remove_variant = None;
        let nv = part.variants.len();
        for (vi, variant) in part.variants.iter_mut().enumerate() {
            ui.add_space(6.0);
            egui::Frame::group(ui.style()).fill(theme::panel_dark()).inner_margin(egui::Margin::same(10)).show(
                ui,
                |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal_wrapped(|ui| {
                        if let Some(v) = self.fields.text_sized(ui, ("var_name", variant.id), &variant.name, 160.0) {
                            variant.name = v;
                        }
                        if let Some(f) = fields::combo(ui, ("var_fp", variant.id), &variant.footprint, &fp_options) {
                            if variant.name == auto_name(variant.footprint) {
                                variant.name = auto_name(f);
                            }
                            variant.footprint = f;
                            variant.connections = f.map_or(Vec::new(), |f| lib_ref.map_by_name(&part.units, f));
                        }
                        if ui.add_enabled(variant.footprint.is_some(), egui::Button::new("Edit footprint")).clicked()
                            && let Some(f) = variant.footprint
                        {
                            open = Some(Open::Item(Item::Footprint(f)));
                        }
                        if ui.add_enabled(nv > 1, egui::Button::new("Remove")).clicked() {
                            remove_variant = Some(vi);
                        }
                    });
                    mapping_ui(ui, lib_ref, &part.units, variant, &options);
                },
            );
        }
        if let Some(i) = remove_variant {
            part.variants.remove(i);
        }
        if ui.button("Add variant").clicked() {
            // The last variant's footprint, else the library's first; none without footprints.
            let fp = part.variants.iter().rev().find_map(|v| v.footprint).or(footprints.first().map(|f| f.0));
            part.variants.push(PartVariant {
                id: Id::new(),
                name: auto_name(fp),
                footprint: fp,
                connections: fp.map_or(Vec::new(), |f| lib_ref.map_by_name(&part.units, f)),
            });
        }

        // ---- Simulation
        crate::sim_form::model_section(ui, &mut self.fields, &mut self.new_sim_param, lib_ref, &mut part);

        // ---- Properties
        fields::section(ui, "Properties", None);
        hint(ui, "Free key/value data copied to components (manufacturer, MPN, datasheet, …).");
        let mut remove_prop = None;
        egui::Grid::new(("props", id)).num_columns(3).spacing([12.0, 6.0]).show(ui, |ui| {
            for (k, v) in part.properties.iter_mut() {
                label(ui, k);
                if let Some(new) = self.fields.text_sized(ui, ("prop", id, k.as_str()), v, 360.0) {
                    *v = new;
                }
                if ui.small_button("Remove").clicked() {
                    remove_prop = Some(k.clone());
                }
                ui.end_row();
            }
        });
        if let Some(k) = remove_prop {
            part.properties.remove(&k);
        }
        ui.horizontal(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut self.new_property).hint_text("New property name").desired_width(200.0),
            );
            let key = self.new_property.trim().to_string();
            if ui
                .add_enabled(!key.is_empty() && !part.properties.contains_key(&key), egui::Button::new("Add"))
                .clicked()
            {
                part.properties.insert(key, String::new());
                self.new_property.clear();
            }
        });

        // ---- Problems
        let prefix = format!("part {:?}", original.name);
        let issues: Vec<String> = lib_ref
            .validate()
            .into_iter()
            .filter(|i| i.starts_with(&prefix))
            .map(|i| i[prefix.len()..].trim_start_matches([':', ' ']).to_string())
            // Shown in the Simulation section.
            .filter(|i| !i.starts_with("simulation model:"))
            .collect();
        issues_ui(ui, &issues);

        if part != original
            && let Err(e) = lib.update_part(part)
        {
            *message = Some(e);
        }
        if self.fields.end_frame() {
            lib.doc.close_group();
        }
        open
    }
}

/// Mapping table of one variant: every pad with a pin picker, plus unmapped pins.
fn mapping_ui(
    ui: &mut egui::Ui,
    lib: &Library,
    units: &[PartUnit],
    variant: &mut PartVariant,
    options: &[(Option<(u32, Id)>, String)],
) {
    let Some(id) = variant.footprint else {
        hint(ui, "No footprint: components with this variant aren't placed on the board.");
        return;
    };
    let Some(fp) = lib.footprints.get(id) else {
        ui.label(RichText::new("Footprint missing").color(theme::warn()));
        return;
    };
    if units.is_empty() {
        hint(ui, "Board part: no pins to map, its pads have no net.");
        return;
    }
    let mut pads: Vec<&Pad> = fp.pads.iter().collect();
    pads.sort_by(|a, b| natural_cmp(&a.name, &b.name));
    let connected = variant.connections.len();
    ui.horizontal_wrapped(|ui| {
        ui.label(RichText::new(format!("{connected} of {} pads connected", pads.len())).color(theme::text_dim()));
        {
            if ui
                .button("Map by name")
                .on_hover_text("Pad \"1\" to pin number 1 (or a pin named \"1\"), pad \"GND\" to pin \"GND\"")
                .clicked()
            {
                variant.connections = lib.map_by_name(units, id);
            }
            if ui.button("Map in order").on_hover_text("Pins in drawing order to pads 1, 2, 3, …").clicked() {
                variant.connections = lib.map_in_order(units, id);
            }
            if ui.button("Clear").clicked() {
                variant.connections.clear();
            }
        }
    });
    if pads.is_empty() {
        hint(ui, "The footprint has no pads yet.");
        return;
    }
    let cols = if pads.len() > 12 { 2 } else { 1 };
    egui::Grid::new(("map", variant.id)).num_columns(2 * cols).striped(true).spacing([16.0, 4.0]).show(ui, |ui| {
        let rows = pads.len().div_ceil(cols);
        for r in 0..rows {
            for c in 0..cols {
                let Some(pad) = pads.get(c * rows + r) else { continue };
                ui.label(RichText::new(format!("Pad {}", pad.name)).monospace());
                let current = variant.connections.iter().find(|x| x.pad == pad.id).map(|x| (x.unit, x.pin));
                if let Some(choice) = fields::combo(ui, ("map", variant.id, pad.id), &current, options) {
                    variant.connections.retain(|x| x.pad != pad.id);
                    if let Some((unit, pin)) = choice {
                        variant.connections.push(PadConnection { pad: pad.id, unit, pin });
                    }
                }
            }
            ui.end_row();
        }
    });
    let unmapped: Vec<&str> = options
        .iter()
        .filter_map(|(o, label)| {
            let (u, p) = (*o)?;
            (!variant.connections.iter().any(|c| c.unit == u && c.pin == p)).then_some(label.as_str())
        })
        .collect();
    if !unmapped.is_empty() {
        ui.label(RichText::new(format!("Pins without a pad: {}", unmapped.join(", "))).color(theme::warn()));
    }
}
