//! Simulation forms: a part's model (the part form's Simulation section) and a
//! component's simulation settings in Properties (exclude, parameter overrides, a
//! source's waveform).

use egui::RichText;
use model::*;

use crate::fields::{self, Fields, hint, label};
use crate::theme;

/// The device type picked in the part form.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Choice {
    None,
    Ignore,
    Resistor,
    Capacitor,
    Inductor,
    Primitive(PrimitiveKind),
    Subckt,
    VoltageSource,
    CurrentSource,
    Spice,
}

impl Choice {
    fn of(sim: &Option<SimModel>) -> Choice {
        match sim.as_ref().map(|m| &m.device) {
            None => Choice::None,
            Some(SimDevice::Ignore) => Choice::Ignore,
            Some(SimDevice::Resistor) => Choice::Resistor,
            Some(SimDevice::Capacitor) => Choice::Capacitor,
            Some(SimDevice::Inductor) => Choice::Inductor,
            Some(SimDevice::Primitive { kind, .. }) => Choice::Primitive(*kind),
            Some(SimDevice::Subckt { .. }) => Choice::Subckt,
            Some(SimDevice::VoltageSource) => Choice::VoltageSource,
            Some(SimDevice::CurrentSource) => Choice::CurrentSource,
            Some(SimDevice::Spice { .. }) => Choice::Spice,
        }
    }

    fn options() -> Vec<(Choice, String)> {
        let mut out = vec![
            (Choice::None, "No model".to_string()),
            (Choice::Ignore, "Not simulated".to_string()),
            (Choice::Resistor, "Resistor".to_string()),
            (Choice::Capacitor, "Capacitor".to_string()),
            (Choice::Inductor, "Inductor".to_string()),
        ];
        out.extend(PrimitiveKind::ALL.map(|k| (Choice::Primitive(k), k.label().to_string())));
        out.extend([
            (Choice::Subckt, "Subcircuit".to_string()),
            (Choice::VoltageSource, "Voltage source".to_string()),
            (Choice::CurrentSource, "Current source".to_string()),
            (Choice::Spice, "SPICE line".to_string()),
        ]);
        out
    }

    /// The device, keeping `text` (of the previous device) where the new one takes text;
    /// a primitive without text gets a generic card.
    fn device(self, text: Option<&str>, part: &Part, pins: usize) -> Option<SimDevice> {
        let text = text.filter(|t| !t.trim().is_empty()).map(str::to_string);
        Some(match self {
            Choice::None => return None,
            Choice::Ignore => SimDevice::Ignore,
            Choice::Resistor => SimDevice::Resistor,
            Choice::Capacitor => SimDevice::Capacitor,
            Choice::Inductor => SimDevice::Inductor,
            Choice::Primitive(kind) => {
                SimDevice::Primitive { kind, model: text.unwrap_or_else(|| kind.generic_card(&part.name)) }
            }
            Choice::Subckt => SimDevice::Subckt {
                text: text.unwrap_or_else(|| {
                    let ports: Vec<String> = (1..=pins.max(1)).map(|k| format!("p{k}")).collect();
                    let name: String =
                        part.name.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }).collect();
                    format!(".subckt {name} {}\n* …\n.ends", ports.join(" "))
                }),
            },
            Choice::VoltageSource => SimDevice::VoltageSource,
            Choice::CurrentSource => SimDevice::CurrentSource,
            Choice::Spice => {
                SimDevice::Spice { template: text.unwrap_or_else(|| "R${REF} ${N1} ${N2} ${VALUE}".to_string()) }
            }
        })
    }
}

/// A pin of a part as the pin combos show it: "3 IN+" (with the unit: "A: 3 IN+").
fn pin_label(lib: &Library, part: &Part, p: SimPin) -> String {
    let Some(pin) =
        part.units.get(p.unit as usize).and_then(|u| lib.symbols.get(u.symbol)).and_then(|s| s.pins.get(p.pin))
    else {
        return "?".into();
    };
    let l = match pin.number.as_str() {
        "" => pin.name.clone(),
        n if n == pin.name => pin.name.clone(),
        n => format!("{n} {}", pin.name),
    };
    if part.units.len() > 1 { format!("{}: {l}", part.units[p.unit as usize].name) } else { l }
}

/// The part form's Simulation section: device, model text with what it parsed to, the
/// SPICE port → pin table and the parameters. Edits `part` in place.
pub fn model_section(ui: &mut egui::Ui, f: &mut Fields, new_param: &mut String, lib: &Library, part: &mut Part) {
    let choice = Choice::of(&part.sim);
    let summary = match &part.sim {
        None => "No model".to_string(),
        Some(m) => match &m.device {
            SimDevice::Primitive { kind, model } => {
                ModelCard::parse(model).map_or(kind.label().to_string(), |c| format!("{} {}", kind.label(), c.name))
            }
            SimDevice::Subckt { text } => {
                SubcktHeader::parse(text).map_or("Subcircuit".to_string(), |h| format!("Subcircuit {}", h.name))
            }
            d => d.label().to_string(),
        },
    };
    fields::section_note(ui, "Simulation", &summary);
    if part.is_board_part() {
        hint(ui, "Board part: not in the schematic, so not simulated.");
        return;
    }
    let pins = lib.sim_pins(part).len();
    ui.horizontal(|ui| {
        label(ui, "Device");
        if let Some(c) = fields::combo(ui, ("sim_device", part.id), &choice, &Choice::options()) {
            let text = part.sim.as_ref().and_then(|m| m.device.text()).map(str::to_string);
            match c.device(text.as_deref(), part, pins) {
                None => part.sim = None,
                Some(device) => match &mut part.sim {
                    Some(m) => m.device = device,
                    None => part.sim = Some(SimModel::new(device)),
                },
            }
        }
    });
    let Some(mut m) = part.sim.clone() else {
        hint(ui, "Without a model the part is left out of simulations, with a warning.");
        return;
    };
    let about = match &m.device {
        SimDevice::Ignore => "Left out of simulations on purpose (connectors, test points), without a warning.",
        SimDevice::Resistor | SimDevice::Capacitor | SimDevice::Inductor => {
            "The component's value is the resistance, capacitance or inductance (10k, 100n; M is mega)."
        }
        SimDevice::VoltageSource | SimDevice::CurrentSource => "The waveform is set on each component, in Properties.",
        SimDevice::Primitive { .. } => "A .model card; the parameters below override its own.",
        SimDevice::Subckt { .. } => "A .subckt … .ends definition; its ports map to the pins below.",
        SimDevice::Spice { .. } => "One SPICE line: ${REF} is the device name, ${VALUE} the value, ${N1}… the nodes.",
    };
    hint(ui, about);
    if m.device != SimDevice::Ignore {
        text_ui(ui, f, lib, part, &mut m);
        ui.add_space(4.0);
        pins_ui(ui, lib, part, &mut m);
        if matches!(m.device, SimDevice::Primitive { .. } | SimDevice::Subckt { .. } | SimDevice::Spice { .. }) {
            ui.add_space(4.0);
            params_ui(ui, f, new_param, part.id, &mut m);
        }
    }
    let changed = part.sim.as_ref() != Some(&m);
    part.sim = Some(m);
    if !changed {
        for issue in lib.sim_issues(part) {
            ui.label(RichText::new(issue).color(theme::warn()));
        }
    }
}

/// The model text, and what it parses to.
fn text_ui(ui: &mut egui::Ui, f: &mut Fields, lib: &Library, part: &Part, m: &mut SimModel) {
    let Some(text) = m.device.text().map(str::to_string) else { return };
    let rows = text.lines().count().clamp(1, 12);
    if let Some(new) = f.multiline(ui, ("sim_text", part.id), &text, rows) {
        let ports_before = m.device.ports();
        match &mut m.device {
            SimDevice::Primitive { model, .. } => *model = new,
            SimDevice::Subckt { text } => *text = new,
            SimDevice::Spice { template } => *template = new,
            _ => {}
        }
        // A pasted subcircuit with other ports: map them again.
        let ports = m.device.ports();
        if matches!(m.device, SimDevice::Subckt { .. }) && ports != ports_before {
            m.pins = lib.auto_map_pins(part, &ports);
        }
    }
    let parsed = match &m.device {
        SimDevice::Primitive { model, .. } => ModelCard::parse(model).map(|c| {
            let n = c.params.len();
            format!(".model {} {} · {n} parameter{}", c.name, c.kind, plural(n))
        }),
        SimDevice::Subckt { text } => SubcktHeader::parse(text).map(|h| {
            format!(".subckt {} · {} port{}: {}", h.name, h.ports.len(), plural(h.ports.len()), h.ports.join(" "))
        }),
        SimDevice::Spice { template } => {
            let n = spice_nodes(template);
            Some(format!("{n} node{}", plural(n)))
        }
        _ => None,
    };
    if let Some(p) = parsed {
        ui.label(RichText::new(p).monospace().color(theme::text_dim()));
    }
}

/// SPICE port → part pin. Without explicit pins the automatic ones (symbol order) show
/// greyed; changing a row writes the whole list.
fn pins_ui(ui: &mut egui::Ui, lib: &Library, part: &Part, m: &mut SimModel) {
    let ports = m.device.ports();
    let auto = m.pins.is_empty();
    let current = lib.sim_pins(part);
    let spice = matches!(m.device, SimDevice::Spice { .. });
    let rows = ports.len().max(current.len());
    let mut options: Vec<(Option<SimPin>, String)> = Vec::new();
    for (u, unit) in part.units.iter().enumerate() {
        let Some(sym) = lib.symbols.get(unit.symbol) else { continue };
        let mut pins: Vec<&Pin> = sym.pins.iter().collect();
        pins.sort_by(|a, b| natural_cmp(a.key(), b.key()));
        for pin in pins {
            let sp = SimPin { unit: u as u32, pin: pin.id };
            options.push((Some(sp), pin_label(lib, part, sp)));
        }
    }
    ui.horizontal(|ui| {
        label(ui, "Pins");
        if auto {
            ui.label(RichText::new("automatic: symbol order").color(theme::text_dim()));
        }
        if ui
            .small_button("Auto-map")
            .on_hover_text("Ports to pins by name and number (A/K, C/B/E, D/G/S, +/−)")
            .clicked()
        {
            m.pins = lib.auto_map_pins(part, &ports);
        }
        if spice {
            if ui.small_button("+").on_hover_text("Add a node").clicked() {
                let mut pins = current.clone();
                if let Some(next) = options.iter().filter_map(|o| o.0).find(|p| !pins.contains(p)) {
                    pins.push(next);
                }
                m.pins = pins;
            }
            if ui
                .add_enabled(!current.is_empty(), egui::Button::new("−").small())
                .on_hover_text("Remove the last node")
                .clicked()
            {
                let mut pins = current.clone();
                pins.pop();
                m.pins = pins;
            }
        }
    });
    if rows == 0 {
        return;
    }
    let mut set = None;
    let mut remove = None;
    egui::Grid::new(("sim_pins", part.id)).num_columns(3).striped(true).spacing([16.0, 4.0]).show(ui, |ui| {
        for i in 0..rows {
            let port = ports.get(i).map_or("—".to_string(), |p| p.clone());
            ui.label(RichText::new(port).monospace());
            let now = current.get(i).copied();
            ui.scope(|ui| {
                if auto {
                    ui.visuals_mut().override_text_color = Some(theme::text_dim());
                }
                if let Some(Some(p)) = fields::combo(ui, ("sim_pin", part.id, i), &now, &options) {
                    set = Some((i, p));
                }
            });
            if i >= ports.len() && !spice {
                if ui.small_button("✕").on_hover_text("Not a port of the model: remove").clicked() {
                    remove = Some(i);
                }
            } else {
                ui.label("");
            }
            ui.end_row();
        }
    });
    if let Some((i, p)) = set {
        let mut pins = current.clone();
        if let Some(old) = pins.iter().position(|q| *q == p) {
            // Swap with the row that had it.
            if let Some(prev) = pins.get(i).copied() {
                pins[old] = prev;
            }
        }
        if i < pins.len() {
            pins[i] = p;
        } else {
            pins.push(p);
        }
        m.pins = pins;
    }
    if let Some(i) = remove {
        let mut pins = current;
        pins.remove(i);
        m.pins = pins;
    }
}

/// Parameter name/value rows (`${VALUE}` takes the component's value).
fn params_ui(ui: &mut egui::Ui, f: &mut Fields, new_param: &mut String, id: Id, m: &mut SimModel) {
    label(ui, "Parameters");
    let mut remove = None;
    if !m.params.is_empty() {
        egui::Grid::new(("sim_params", id)).num_columns(3).spacing([12.0, 4.0]).show(ui, |ui| {
            for (k, v) in m.params.iter_mut() {
                ui.label(RichText::new(k).monospace());
                if let Some(new) = f.text_sized(ui, ("sim_param", id, k.as_str()), v, 200.0) {
                    *v = new;
                }
                if ui.small_button("✕").on_hover_text("Remove").clicked() {
                    remove = Some(k.clone());
                }
                ui.end_row();
            }
        });
    }
    if let Some(k) = remove {
        m.params.remove(&k);
    }
    ui.horizontal(|ui| {
        ui.add(egui::TextEdit::singleline(new_param).hint_text("Parameter name (BV, RS, …)").desired_width(200.0));
        let key = new_param.trim().to_string();
        if ui.add_enabled(!key.is_empty() && !m.params.contains_key(&key), egui::Button::new("Add")).clicked() {
            m.params.insert(key, String::new());
            new_param.clear();
        }
    });
}

fn plural(n: usize) -> &'static str {
    if n == 1 { "" } else { "s" }
}

/// The Simulation rows of a component in Properties.
#[derive(Default)]
pub struct ComponentForm {
    fields: Fields,
    si: fields::SiFields,
    new_param: String,
    /// The PWL text typed last didn't parse.
    bad_pwl: bool,
}

impl ComponentForm {
    /// Exclude, parameter overrides and a source's waveform. Returns the changed
    /// component.
    pub fn ui(&mut self, ui: &mut egui::Ui, project: &Project, c: &Component) -> Option<Component> {
        let part = project.library.parts.get(c.part)?;
        let unit = |m: &SimModel| if m.device == SimDevice::VoltageSource { "V" } else { "A" };
        // What's inside, for the header of the (usually closed) section.
        let note = match &part.sim {
            None => "no model".to_string(),
            Some(m) if m.device == SimDevice::Ignore => "not simulated".to_string(),
            _ if c.sim.exclude => "excluded".to_string(),
            Some(m) if matches!(m.device, SimDevice::VoltageSource | SimDevice::CurrentSource) => {
                c.sim.source.clone().unwrap_or_default().describe(unit(m))
            }
            _ => match c.sim.params.len() {
                0 => String::new(),
                1 => "1 override".to_string(),
                n => format!("{n} overrides"),
            },
        };
        let mut out = None;
        fields::collapsing_note(ui, "Simulation", &note, "sim_component", false, |ui| {
            out = self.body(ui, part, c, unit);
        });
        out
    }

    fn body(
        &mut self,
        ui: &mut egui::Ui,
        part: &Part,
        c: &Component,
        unit: impl Fn(&SimModel) -> &'static str,
    ) -> Option<Component> {
        let mut new = c.clone();
        let Some(model) = &part.sim else {
            hint(ui, "No model: edit the part to add one. Left out of simulations, with a warning.");
            return None;
        };
        if model.device == SimDevice::Ignore {
            hint(ui, "Not simulated (the part's model says so).");
            return None;
        }
        fields::grid(ui, ("sim_exclude", c.id), |ui| {
            fields::check_row(ui, "Exclude", &mut new.sim.exclude)
                .on_hover_text("Left out of the netlist, as if it weren't there");
        });
        match &model.device {
            SimDevice::VoltageSource | SimDevice::CurrentSource => {
                let source = new.sim.source.get_or_insert_with(Source::default);
                self.source_ui(ui, c.id, source, unit(model));
                if new.sim.source == Some(Source::default()) {
                    new.sim.source = None;
                }
            }
            SimDevice::Primitive { .. } | SimDevice::Subckt { .. } | SimDevice::Spice { .. } => {
                self.overrides_ui(ui, c.id, model, &mut new.sim.params);
            }
            _ => {}
        }
        (new != *c).then_some(new)
    }

    /// Overrides of the model's parameters: a row per parameter of the part's model (and
    /// of a primitive's card) with the part's value as the hint; empty is no override.
    fn overrides_ui(
        &mut self,
        ui: &mut egui::Ui,
        id: Id,
        model: &SimModel,
        params: &mut std::collections::BTreeMap<String, String>,
    ) {
        let mut names: Vec<(String, String)> = Vec::new();
        let mut add = |k: &str, v: &str| {
            if !names.iter().any(|(n, _)| n.eq_ignore_ascii_case(k)) {
                names.push((k.to_string(), v.to_string()));
            }
        };
        for (k, v) in &model.params {
            add(k, v);
        }
        if let SimDevice::Primitive { model, .. } = &model.device
            && let Some(card) = ModelCard::parse(model)
        {
            for (k, v) in &card.params {
                if let Some(v) = v {
                    add(k, v);
                }
            }
        }
        for k in params.keys() {
            add(k, "");
        }
        if !names.is_empty() {
            hint(ui, "Parameters (empty: the part's value)");
            fields::grid(ui, ("sim_overrides", id), |ui| {
                for (k, part_value) in &names {
                    label(ui, k);
                    let current = params.get(k).cloned().unwrap_or_default();
                    if let Some(v) =
                        self.fields.text_hint(ui, ("sim_override", id, k.as_str()), &current, 160.0, part_value)
                    {
                        if v.trim().is_empty() {
                            params.remove(k);
                        } else {
                            params.insert(k.clone(), v.trim().to_string());
                        }
                    }
                    ui.end_row();
                }
            });
        }
        // The button first (right to left), the field filling what's left.
        let row = egui::vec2(ui.available_width(), ui.spacing().interact_size.y);
        ui.allocate_ui_with_layout(row, egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let key = self.new_param.trim().to_string();
            let taken = names.iter().any(|(n, _)| n.eq_ignore_ascii_case(&key));
            if ui.add_enabled(!key.is_empty() && !taken, egui::Button::new("Add")).clicked() {
                params.insert(key, "0".into());
                self.new_param.clear();
            }
            ui.add(
                egui::TextEdit::singleline(&mut self.new_param)
                    .hint_text("Override another parameter")
                    .desired_width(ui.available_width()),
            );
        });
    }

    /// A source's waveform: its kind, the fields of that kind, AC magnitude and phase,
    /// and a preview.
    fn source_ui(&mut self, ui: &mut egui::Ui, id: Id, s: &mut Source, unit: &str) {
        let kinds: Vec<(&str, String)> = Waveform::KINDS.iter().map(|k| (*k, k.to_string())).collect();
        fields::grid(ui, ("sim_source", id), |ui| {
            label(ui, "Waveform");
            if let Some(k) = fields::combo(ui, ("sim_wave", id), &s.waveform.label(), &kinds) {
                s.waveform = s.waveform.with_kind(k);
            }
            ui.end_row();
            let mut num = |ui: &mut egui::Ui, name: &str, v: &mut f64, u: &str| {
                label(ui, name);
                if let Some(Some(n)) = self.si.field(ui, ("sim_wave_field", id, name), Some(*v), u, "") {
                    *v = n;
                }
                ui.end_row();
            };
            match &mut s.waveform {
                Waveform::Dc { value } => num(ui, "Value", value, unit),
                Waveform::Sine { offset, amplitude, frequency, delay, damping, phase } => {
                    num(ui, "Offset", offset, unit);
                    num(ui, "Amplitude", amplitude, unit);
                    num(ui, "Frequency", frequency, "Hz");
                    num(ui, "Delay", delay, "s");
                    num(ui, "Damping (1/s)", damping, "");
                    num(ui, "Phase", phase, "°");
                }
                Waveform::Pulse { low, high, delay, rise, fall, width, period } => {
                    num(ui, "Low", low, unit);
                    num(ui, "High", high, unit);
                    num(ui, "Delay", delay, "s");
                    num(ui, "Rise time", rise, "s");
                    num(ui, "Fall time", fall, "s");
                    num(ui, "Width", width, "s");
                    num(ui, "Period", period, "s");
                }
                Waveform::Exp { v1, v2, delay1, tau1, delay2, tau2 } => {
                    num(ui, "Initial", v1, unit);
                    num(ui, "Pulsed", v2, unit);
                    num(ui, "Rise delay", delay1, "s");
                    num(ui, "Rise τ", tau1, "s");
                    num(ui, "Fall delay", delay2, "s");
                    num(ui, "Fall τ", tau2, "s");
                }
                Waveform::Pwl { points, repeat } => {
                    label(ui, "Points");
                    let text = pwl_text(points);
                    if let Some(t) = self.fields.text_hint(ui, ("sim_pwl", id), &text, 240.0, "0 0, 1m 5, 2m 5") {
                        match parse_pwl(&t) {
                            Some(p) => {
                                *points = p;
                                self.bad_pwl = false;
                            }
                            None => self.bad_pwl = true,
                        }
                    }
                    ui.end_row();
                    label(ui, "");
                    ui.checkbox(repeat, "Repeat");
                    ui.end_row();
                }
            }
            label(ui, "AC magnitude");
            if let Some(Some(v)) = self.si.field(ui, ("sim_ac", id), Some(s.ac), unit, "") {
                s.ac = v;
            }
            ui.end_row();
            if s.ac != 0.0 {
                label(ui, "AC phase");
                if let Some(Some(v)) = self.si.field(ui, ("sim_ac_phase", id), Some(s.ac_phase), "°", "") {
                    s.ac_phase = v;
                }
                ui.end_row();
            }
        });
        if let Waveform::Pwl { .. } = s.waveform {
            if self.bad_pwl {
                ui.label(RichText::new("Not applied: pairs of time and value, times increasing").color(theme::error()));
            }
            hint(ui, "Time and value pairs, separated by commas: 0 0, 1m 5, 2m 5.");
        }
        preview(ui, &s.waveform, unit);
    }
}

/// PWL points as "t v, t v" with SI prefixes.
fn pwl_text(points: &[(f64, f64)]) -> String {
    points.iter().map(|(t, v)| format!("{} {}", si_short(*t), si_short(*v))).collect::<Vec<_>>().join(", ")
}

/// A number with an SI prefix and no unit or space ("1m", "2.5k", "0").
fn si_short(v: f64) -> String {
    si::format(v, "").replace(' ', "")
}

/// "0 0, 1m 5, 2m 5" → points; `None` if a pair doesn't parse or times go back.
pub fn parse_pwl(text: &str) -> Option<Vec<(f64, f64)>> {
    let mut out = Vec::new();
    for pair in text.split([',', ';']).map(str::trim).filter(|p| !p.is_empty()) {
        let mut it = pair.split_whitespace();
        let (t, v) = (si::parse(it.next()?)?, si::parse(it.next()?)?);
        if it.next().is_some() || out.last().is_some_and(|&(last, _)| t <= last) {
            return None;
        }
        out.push((t, v));
    }
    (!out.is_empty()).then_some(out)
}

/// A small plot of the waveform over its preview span.
fn preview(ui: &mut egui::Ui, w: &Waveform, unit: &str) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 64.0), egui::Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 4.0, theme::panel_dark());
    let span = w.preview_span();
    let n = 200;
    let ys: Vec<f64> = (0..=n).map(|i| w.at(span * i as f64 / n as f64)).collect();
    let (lo, hi) = ys.iter().fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), &y| (a.min(y), b.max(y)));
    let (lo, hi) = if hi - lo < 1e-12 { (lo - 1.0, hi + 1.0) } else { (lo, hi) };
    let inner = rect.shrink2(egui::vec2(6.0, 8.0));
    let pts: Vec<egui::Pos2> = ys
        .iter()
        .enumerate()
        .map(|(i, y)| {
            let x = inner.left() + inner.width() * i as f32 / n as f32;
            let y = inner.bottom() - inner.height() * ((y - lo) / (hi - lo)) as f32;
            egui::pos2(x, y)
        })
        .collect();
    painter.add(egui::Shape::line(pts, egui::Stroke::new(1.5, theme::color(view::style::trace_color(0)))));
    let font = egui::FontId::proportional(10.0);
    let dim = theme::text_dim();
    painter.text(
        rect.left_top() + egui::vec2(4.0, 2.0),
        egui::Align2::LEFT_TOP,
        si::format(hi, unit),
        font.clone(),
        dim,
    );
    painter.text(
        rect.left_bottom() + egui::vec2(4.0, -2.0),
        egui::Align2::LEFT_BOTTOM,
        si::format(lo, unit),
        font.clone(),
        dim,
    );
    painter.text(
        rect.right_bottom() + egui::vec2(-4.0, -2.0),
        egui::Align2::RIGHT_BOTTOM,
        si::format(span, "s"),
        font,
        dim,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pwl_points_as_text() {
        let p = parse_pwl("0 0, 1m 5; 2.5m 5").unwrap();
        assert_eq!(p, vec![(0.0, 0.0), (1e-3, 5.0), (2.5e-3, 5.0)]);
        assert_eq!(parse_pwl(&pwl_text(&p)), Some(p));
        assert_eq!(parse_pwl("0 0, 0 1"), None, "times must increase");
        assert_eq!(parse_pwl("0"), None);
        assert_eq!(parse_pwl(""), None);
    }
}
