//! Values the tuner and parameter sweeps change: which parameters can change, their
//! current values, and the overrides a run's netlist uses instead of the document's
//! components (the document doesn't change until the user applies them).

use std::collections::HashMap;

use model::*;

/// Components as a run sees them: the document's, with tuned values put in.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Overrides {
    components: HashMap<Id, Component>,
}

impl Overrides {
    /// `values` applied to copies of their components (parameters of missing
    /// components, or that don't apply, are left out).
    pub fn new(project: &Project, values: &[(TuneParam, f64)]) -> Overrides {
        let mut components: HashMap<Id, Component> = HashMap::new();
        for (param, v) in values {
            let id = param.component();
            let Some(c) = components.get(&id).cloned().or_else(|| project.schematic.components.get(id).cloned()) else {
                continue;
            };
            let mut c = c;
            if apply(&mut c, param, *v) {
                components.insert(id, c);
            }
        }
        Overrides { components }
    }

    pub fn is_empty(&self) -> bool {
        self.components.is_empty()
    }

    /// The component a run uses for `c`.
    pub fn component<'a>(&'a self, c: &'a Component) -> &'a Component {
        self.components.get(&c.id).unwrap_or(c)
    }

    /// The changed components (to apply them as one edit).
    pub fn components(&self) -> impl Iterator<Item = &Component> {
        self.components.values()
    }
}

/// Puts `v` into `c` for `param`. False when it doesn't apply (not a source, no such
/// field).
pub fn apply(c: &mut Component, param: &TuneParam, v: f64) -> bool {
    match param {
        TuneParam::Value { .. } => {
            c.value = si::value_text(v);
            true
        }
        TuneParam::Source { field, .. } => c.sim.source.as_mut().is_some_and(|s| s.set_field(field, v)),
    }
}

/// The document's value of `param`.
pub fn current(project: &Project, param: &TuneParam) -> Option<f64> {
    let c = project.schematic.components.get(param.component())?;
    match param {
        TuneParam::Value { .. } => si::parse(&c.value),
        TuneParam::Source { field, .. } => c.sim.source.as_ref()?.field(field),
    }
}

/// "R1 value", "V1 amplitude", "V1 AC".
pub fn label(project: &Project, param: &TuneParam) -> String {
    let refdes = project.schematic.components.get(param.component()).map_or("?", |c| c.refdes.as_str());
    match param {
        TuneParam::Value { .. } => format!("{refdes} value"),
        TuneParam::Source { field, .. } if field == "ac" => format!("{refdes} AC"),
        TuneParam::Source { field, .. } => format!("{refdes} {field}"),
    }
}

/// The model of a component's part.
fn device(project: &Project, component: Id) -> Option<&SimDevice> {
    let c = project.schematic.components.get(component)?;
    Some(&project.library.parts.get(c.part)?.sim.as_ref()?.device)
}

/// The unit of `param`'s values: Ω, F, H for R, C, L values, V or A for source levels.
pub fn unit(project: &Project, param: &TuneParam) -> &'static str {
    let device = device(project, param.component());
    match param {
        TuneParam::Value { .. } => match device {
            Some(SimDevice::Resistor) => "Ω",
            Some(SimDevice::Capacitor) => "F",
            Some(SimDevice::Inductor) => "H",
            _ => "",
        },
        TuneParam::Source { field, .. } => {
            let level = if matches!(device, Some(SimDevice::CurrentSource)) { "A" } else { "V" };
            Source::field_unit(field, level)
        }
    }
}

/// Whether a part's model takes the component's value.
fn takes_value(model: &SimModel) -> bool {
    match &model.device {
        SimDevice::Resistor | SimDevice::Capacitor | SimDevice::Inductor => true,
        SimDevice::Spice { template } => template.contains("${VALUE}"),
        _ => model.params.values().any(|v| v.contains("${VALUE}")),
    }
}

/// Every parameter that can be tuned, by component in reference order: values that
/// are numbers of parts whose model takes them, and sources' numeric fields.
pub fn tunable(project: &Project) -> Vec<TuneParam> {
    let mut components: Vec<&Component> = project.schematic.components.iter().collect();
    components.sort_by(|a, b| natural_cmp(&a.refdes, &b.refdes));
    components.into_iter().flat_map(|c| tunable_of(project, c)).collect()
}

/// The parameters of one component that can be tuned.
pub fn tunable_of(project: &Project, c: &Component) -> Vec<TuneParam> {
    if c.sim.exclude {
        return Vec::new();
    }
    let Some(model) = project.library.parts.get(c.part).and_then(|p| p.sim.as_ref()) else { return Vec::new() };
    if let Some(source) = &c.sim.source {
        let mut s = source.clone();
        s.fields_mut()
            .into_iter()
            .map(|(field, _)| TuneParam::Source { component: c.id, field: field.to_string() })
            .collect()
    } else if takes_value(model) && si::parse(&c.value).is_some() {
        vec![TuneParam::Value { component: c.id }]
    } else {
        Vec::new()
    }
}

/// A new tuner row for `param` around its current value: a decade either way for
/// values (E24 snapping for resistors, E12 for capacitors and inductors), 0 to twice
/// the value for source fields (−2× to 2× when negative; ±1 around 0).
pub fn default_row(project: &Project, param: &TuneParam) -> Option<TuneRow> {
    let v = current(project, param)?;
    let (min, max, snap) = match param {
        TuneParam::Value { .. } => {
            if v <= 0.0 {
                return None;
            }
            let snap = match device(project, param.component()) {
                Some(SimDevice::Resistor) => Snap::E24,
                Some(SimDevice::Capacitor | SimDevice::Inductor) => Snap::E12,
                _ => Snap::None,
            };
            (v / 10.0, v * 10.0, snap)
        }
        TuneParam::Source { .. } if v > 0.0 => (0.0, 2.0 * v, Snap::None),
        TuneParam::Source { .. } if v < 0.0 => (2.0 * v, -2.0 * v, Snap::None),
        TuneParam::Source { .. } => (-1.0, 1.0, Snap::None),
    };
    Some(TuneRow { param: param.clone(), min, max, snap })
}

/// The value at slider position `t` (0…1) of a row: log for values, linear for
/// source fields, snapped.
pub fn at(row: &TuneRow, t: f64) -> f64 {
    let t = t.clamp(0.0, 1.0);
    let v = if row.param.is_log() {
        (row.min.ln() + t * (row.max.ln() - row.min.ln())).exp()
    } else {
        row.min + t * (row.max - row.min)
    };
    row.snap.apply(v)
}

/// The slider position (0…1) of `v` in a row.
pub fn position(row: &TuneRow, v: f64) -> f64 {
    let t = if row.param.is_log() {
        if v <= 0.0 {
            return 0.0;
        }
        (v.ln() - row.min.ln()) / (row.max.ln() - row.min.ln())
    } else {
        (v - row.min) / (row.max - row.min)
    };
    if t.is_finite() { t.clamp(0.0, 1.0) } else { 0.0 }
}

/// Runs that take less than this re-run while a slider is dragged (else on release).
pub const LIVE: std::time::Duration = std::time::Duration::from_millis(150);

/// The tuner's values: what its sliders are at (in memory only; the rows are the
/// setup's [`Simulation::tune`]). Parameters without a value are at the document's.
#[derive(Clone, Debug, Default)]
pub struct Tuner {
    values: HashMap<TuneParam, f64>,
}

impl Tuner {
    /// The value of a row: tuned, else the document's.
    pub fn value(&self, project: &Project, param: &TuneParam) -> Option<f64> {
        self.values.get(param).copied().or_else(|| current(project, param))
    }

    /// Sets a value (a part's as the value text it becomes reads it back, so applying
    /// it leaves nothing tuned).
    pub fn set(&mut self, param: &TuneParam, v: f64) {
        let v = match param {
            TuneParam::Value { .. } => si::parse(&si::value_text(v)).unwrap_or(v),
            TuneParam::Source { .. } => v,
        };
        self.values.insert(param.clone(), v);
    }

    /// Back to the document's value of one parameter.
    pub fn forget(&mut self, param: &TuneParam) {
        self.values.remove(param);
    }

    /// Back to the document's values.
    pub fn reset(&mut self) {
        self.values.clear();
    }

    /// The tuned values of `rows` that differ from the document's, in row order;
    /// `except` (the parameter a sweep steps) left out.
    pub fn changed(&self, project: &Project, rows: &[TuneRow], except: Option<&TuneParam>) -> Vec<(TuneParam, f64)> {
        rows.iter()
            .filter(|r| Some(&r.param) != except)
            .filter_map(|r| {
                let v = *self.values.get(&r.param)?;
                (current(project, &r.param) != Some(v)).then(|| (r.param.clone(), v))
            })
            .collect()
    }

    /// The components a run uses ([`changed`](Self::changed) put in).
    pub fn overrides(&self, project: &Project, rows: &[TuneRow], except: Option<&TuneParam>) -> Overrides {
        Overrides::new(project, &self.changed(project, rows, except))
    }

    /// Forgets values of parameters no longer in `rows`.
    pub fn retain(&mut self, rows: &[TuneRow]) {
        self.values.retain(|p, _| rows.iter().any(|r| &r.param == p));
    }
}
