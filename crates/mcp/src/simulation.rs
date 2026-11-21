//! Simulation for agents: `simulate` (a run through the host's session, answered when it
//! ends), `get_spice_netlist` and the plot as an offscreen scene (`screenshot` target
//! `simulation`). Shared by the app and the headless host.

use std::time::{Duration, Instant};

use editor_core::connectivity::Connectivity;
use model::{Analysis, AnalysisKind, Id, Probe, Project, TuneParam, si};
use render::font::{self, HAlign, VAlign};
use render::{Camera, Color, Instance, Scene};
use serde_json::{Value, json};
use sim::plot::{self, Axis, PlotData, Scale, Signal};
use sim::tune::Overrides;
use sim::{Finished, Level, Netlist, Outcome, Session};
use view::Document;

/// Longest an agent's run may take.
pub const TIMEOUT: Duration = Duration::from_secs(120);
/// Most points of a trace in a reply.
pub const MAX_POINTS: usize = 200;

/// What `simulate` asks for.
#[derive(Clone, Debug, Default)]
pub struct SimulateRequest {
    /// The analysis, by name or id (`None`: the one the host shows, else the first).
    pub analysis: Option<String>,
    /// Nets (`OUT`, `V(OUT)`) and pin currents (`R1.1`, `I(R1.1)`); `None`: the
    /// analysis' probes, else its named nets.
    pub signals: Option<Vec<String>>,
    /// x values (time, frequency, swept value) to read every signal at.
    pub points: Vec<f64>,
    /// The x range measurements cover (default: the whole run).
    pub from: Option<f64>,
    pub to: Option<f64>,
    /// Values for this run only: `R1` (a part's value) or `V1.amplitude` (a source's
    /// field) to an SI value (`4k7`, `0.2`).
    pub values: Vec<(String, String)>,
}

/// A run an agent is waiting for.
#[derive(Debug)]
pub struct SimJob {
    pub analysis: Id,
    /// The signals asked for (`None`: the defaults, chosen with the result).
    pub signals: Option<Vec<Signal>>,
    /// Currents the analysis doesn't probe: measured in this run only.
    pub extra: Vec<Probe>,
    pub points: Vec<f64>,
    pub range: (Option<f64>, Option<f64>),
    /// The request's `values`, put into its components.
    pub overrides: Overrides,
    pub started: Instant,
}

/// The analysis named `spec` (name, case-insensitive, or id), else `default`, else the
/// first.
pub fn find_analysis<'a>(
    project: &'a Project,
    spec: Option<&str>,
    default: Option<Id>,
) -> Result<&'a Analysis, String> {
    let setup = &project.schematic.simulation;
    let names = || setup.analyses.iter().map(|a| a.name.as_str()).collect::<Vec<_>>().join(", ");
    if setup.analyses.is_empty() {
        return Err("the project has no analyses: add one with the set_simulation command".into());
    }
    match spec {
        Some(s) => setup
            .analyses
            .iter()
            .find(|a| a.name.eq_ignore_ascii_case(s.trim()) || a.id.to_string() == s.trim())
            .ok_or_else(|| format!("no analysis {s:?} (analyses: {})", names())),
        None => Ok(default.and_then(|id| setup.analyses.iter().find(|a| a.id == id)).unwrap_or(&setup.analyses[0])),
    }
}

/// Parses signal names against the netlist the analysis gets now.
pub fn parse_signals(names: &[String], netlist: &Netlist, conn: &Connectivity) -> Result<Vec<Signal>, String> {
    names.iter().map(|n| parse_signal(n, netlist, conn)).collect()
}

fn parse_signal(name: &str, netlist: &Netlist, conn: &Connectivity) -> Result<Signal, String> {
    let s = name.trim();
    let inner = |prefix: char| {
        let rest = s.strip_prefix(prefix).or_else(|| s.strip_prefix(prefix.to_ascii_lowercase()))?;
        rest.strip_prefix('(')?.strip_suffix(')')
    };
    let net = |n: &str| -> Result<String, String> {
        let n = n.trim();
        conn.net(n).map(|net| net.name.clone()).ok_or_else(|| format!("no net {n:?}"))
    };
    if let Some(body) = inner('P') {
        let pin = netlist.pins.iter().find(|p| p.refdes.eq_ignore_ascii_case(body.trim()));
        return pin.map(|p| Signal::Power(p.component)).ok_or_else(|| format!("{name:?}: no simulated part {body:?}"));
    }
    let (body, current) = match (inner('V'), inner('I')) {
        (Some(v), _) => (v, Some(false)),
        (_, Some(i)) => (i, Some(true)),
        _ => (s, None),
    };
    if current == Some(false)
        && let Some((a, b)) = body.split_once(',')
    {
        return Ok(Signal::Diff(net(a)?, net(b)?));
    }
    if current != Some(false)
        && let Some((refdes, key)) = body.rsplit_once('.')
        && netlist.pins.iter().any(|p| p.refdes.eq_ignore_ascii_case(refdes))
    {
        let pins: Vec<_> = netlist.pins.iter().filter(|p| p.refdes.eq_ignore_ascii_case(refdes)).collect();
        let pin = pins.iter().find(|p| p.key.eq_ignore_ascii_case(key)).ok_or_else(|| {
            let keys: Vec<&str> = pins.iter().map(|p| p.key.as_str()).collect();
            format!("{refdes} has no simulated pin {key:?} (pins: {})", keys.join(", "))
        })?;
        let symbol = pin.symbol.ok_or_else(|| format!("the unit of pin {body} isn't placed"))?;
        return Ok(Signal::Current(symbol, pin.pin));
    }
    if current == Some(true) {
        return Err(format!("{name:?}: no such pin; name a current as REFDES.PIN (R1.1)"));
    }
    match conn.net(body) {
        Some(n) => Ok(Signal::Voltage(n.name.clone())),
        None if body.contains('.') => Err(format!("{name:?}: no such net, and no simulated part of that name")),
        None => Err(format!("no net {body:?}")),
    }
}

/// Parses `simulate`'s values: `R1` → the part's value (a DC source's level),
/// `V1.amplitude` → a field of its source (`V1.ac`: the AC magnitude), each to an SI
/// value.
pub fn parse_values(project: &Project, values: &[(String, String)]) -> Result<Overrides, String> {
    let tunable = sim::tune::tunable(project);
    let mut out = Vec::new();
    for (key, text) in values {
        let key = key.trim();
        let (refdes, field) = match key.split_once('.') {
            Some((r, f)) => (r, Some(f)),
            None => (key, None),
        };
        let c = project
            .schematic
            .components
            .iter()
            .find(|c| c.refdes.eq_ignore_ascii_case(refdes))
            .ok_or_else(|| format!("values: no component {refdes:?}"))?;
        let param = match field {
            // A source's own value is its DC level.
            None if c.sim.source.is_some() => TuneParam::Source { component: c.id, field: "value".into() },
            None => TuneParam::Value { component: c.id },
            Some(f) => TuneParam::Source { component: c.id, field: f.to_ascii_lowercase() },
        };
        if !tunable.contains(&param) {
            let fields: Vec<String> = tunable
                .iter()
                .filter(|p| p.component() == c.id)
                .map(|p| match p {
                    TuneParam::Value { .. } => c.refdes.clone(),
                    TuneParam::Source { field, .. } => format!("{}.{field}", c.refdes),
                })
                .collect();
            let hint = if fields.is_empty() {
                "nothing of it can".to_string()
            } else {
                format!("it has {}", fields.join(", "))
            };
            return Err(format!("values: {key:?} can't be set for a run ({hint})"));
        }
        let v = si::parse(text).ok_or_else(|| format!("values: {key:?}: {text:?} isn't a value"))?;
        out.push((param, v));
    }
    Ok(Overrides::new(project, &out))
}

/// Prepares an agent's run: the analysis, the signals, the currents to measure.
pub fn prepare(doc: &mut Document, req: &SimulateRequest, default: Option<Id>) -> Result<(SimJob, Analysis), String> {
    doc.update_connectivity(false);
    let project = doc.project();
    let analysis = find_analysis(project, req.analysis.as_deref(), default)?.clone();
    let conn = doc.connectivity();
    let overrides = parse_values(project, &req.values)?;
    let signals = match &req.signals {
        Some(names) if !names.is_empty() => {
            let netlist = sim::netlist::build_with(project, conn, &analysis, &overrides);
            Some(parse_signals(names, &netlist, conn)?)
        }
        _ => None,
    };
    // An operating point measures every pin.
    let sweep = !matches!(analysis.kind, AnalysisKind::Op);
    let probed: Vec<Signal> = analysis.probes.iter().map(|p| Signal::of_probe(p, conn)).collect();
    let extra = signals
        .iter()
        .flatten()
        .filter(|s| sweep && s.needs_sensing() && !probed.contains(s))
        .map(Signal::probe)
        .collect();
    let job = SimJob {
        analysis: analysis.id,
        signals,
        extra,
        points: req.points.clone(),
        range: (req.from, req.to),
        overrides,
        started: Instant::now(),
    };
    Ok((job, analysis))
}

/// Starts an agent's run in `session`.
pub fn start(session: &mut Session, doc: &Document, job: &SimJob, backend: sim::SharedBackend) -> Result<(), String> {
    let a = find_analysis(doc.project(), Some(&job.analysis.to_string()), None)?;
    if session.start_with(doc.project(), doc.connectivity(), a, &job.extra, &job.overrides, backend) {
        Ok(())
    } else {
        Err(format!("{} not started: {}", a.name, errors(session)))
    }
}

/// What a run an agent waits for has come to: `None` while it runs.
pub fn check(job: &SimJob, session: &mut Session, doc: &Document) -> Option<Result<Value, String>> {
    match session.running() {
        Some((id, _)) if id == job.analysis => {
            if job.started.elapsed() > TIMEOUT {
                session.stop();
                return Some(Err(format!("stopped after {} s", TIMEOUT.as_secs())));
            }
            return None;
        }
        Some(_) => return Some(Err("stopped: another analysis was started".into())),
        None => {}
    }
    let attempt = session.attempt().filter(|a| a.analysis == job.analysis);
    Some(match attempt.map(|a| &a.outcome) {
        Some(Outcome::Done) => {
            let done = session.result(job.analysis)?;
            let project = doc.project();
            let analysis = find_analysis(project, Some(&job.analysis.to_string()), None).ok()?;
            Ok(reply(project, analysis, job, done, doc.connectivity(), session))
        }
        Some(Outcome::Cancelled) | None => Err("the run was stopped".into()),
        Some(Outcome::Failed(_) | Outcome::NetlistErrors) => Err(format!("the run failed: {}", errors(session))),
        Some(Outcome::Running) => return None,
    })
}

/// The errors of the last attempt.
fn errors(session: &Session) -> String {
    let lines: Vec<&str> = session
        .attempt()
        .iter()
        .flat_map(|a| &a.problems)
        .filter(|p| p.level == Level::Error)
        .map(|p| p.message.as_str())
        .collect();
    if lines.is_empty() { "unknown error".into() } else { lines.join("; ") }
}

/// Six significant digits: enough for an agent, short in JSON.
fn round(v: f64) -> Value {
    if !v.is_finite() {
        return Value::Null;
    }
    json!(format!("{v:.5e}").parse::<f64>().unwrap_or(v))
}

/// The answer to `simulate`.
fn reply(
    project: &Project,
    analysis: &Analysis,
    job: &SimJob,
    done: &Finished,
    conn: &Connectivity,
    session: &Session,
) -> Value {
    let r = &done.result;
    let steps = sim::tune::steps(project, done);
    let warnings: Vec<&str> = session
        .attempt()
        .iter()
        .flat_map(|a| &a.problems)
        .filter(|p| p.level == Level::Warning)
        .map(|p| p.message.as_str())
        .collect();
    let mut out = json!({
        "analysis": analysis.name,
        "kind": analysis.kind.label(),
        "title": sim::report::title(r),
        "seconds": round(done.duration.as_secs_f64()),
        "warnings": warnings,
    });
    if let Some((param, values)) = &done.step {
        out["step"] = json!({ "parameter": sim::tune::short_label(project, param), "values": values.iter().map(|v| round(*v)).collect::<Vec<_>>() });
    }
    let signals = job.signals.clone().unwrap_or_else(|| plot::signals(analysis, r, conn).0);
    let Some(data) = plot::data_steps(&steps, &signals, true, conn) else {
        // An operating point: the table, or the signals asked for (a sweep: per step).
        let row = |label: String, v: f64, unit: &str| json!({ "signal": label, "value": round(v), "unit": unit, "text": si::format(v, unit) });
        let rows = |r: &sim::SimResult| -> Vec<Value> {
            match &job.signals {
                None => sim::report::signals(r)
                    .into_iter()
                    .map(|s| row(s.label, s.vector.re.first().copied().unwrap_or(f64::NAN), s.unit))
                    .collect(),
                Some(asked) => asked
                    .iter()
                    .map(|a| {
                        let v = plot::vector(r, a).and_then(|v| v.re.first().copied()).unwrap_or(f64::NAN);
                        row(plot::signal_label(r, a), v, a.unit())
                    })
                    .collect(),
            }
        };
        match &done.step {
            None => out["values"] = json!(rows(r)),
            Some(_) => {
                out["steps"] = json!(
                    done.results()
                        .into_iter()
                        .zip(&steps)
                        .map(|((v, r), (label, _))| json!({ "step": label, "value": v.map_or(Value::Null, round), "values": rows(r) }))
                        .collect::<Vec<_>>()
                );
            }
        }
        return out;
    };
    let (lo, hi) = (data.x.first().copied().unwrap_or(0.0), data.x.last().copied().unwrap_or(0.0));
    out["x"] = json!({ "label": data.x_label, "unit": data.x_unit, "from": round(lo), "to": round(hi), "samples": data.x.len() });
    let range = (job.range.0.unwrap_or(lo), job.range.1.unwrap_or(hi));
    let values = done.step.as_ref().map(|(_, v)| v.as_slice());
    out["signals"] = json!(
        data.traces
            .iter()
            .map(|t| {
                let mut v = trace_json(&data, t, &job.points, range);
                if let Some(value) = values.and_then(|v| v.get(t.step)) {
                    v["step"] = json!(data.steps[t.step]);
                    v["step_value"] = round(*value);
                }
                v
            })
            .collect::<Vec<_>>()
    );
    if !data.missing.is_empty() {
        out["missing"] =
            json!(data.missing.iter().map(|m| json!({"signal": m.label, "reason": m.reason})).collect::<Vec<_>>());
    }
    out
}

fn trace_json(data: &PlotData, t: &plot::Trace, points: &[f64], (from, to): (f64, f64)) -> Value {
    let x = &data.x;
    let (lo, hi) = (x.first().copied().unwrap_or(0.0), x.last().copied().unwrap_or(0.0));
    let scale = Scale::new(lo, hi, data.log_x);
    let columns = MAX_POINTS / 4 - 1;
    let pts = plot::decimate(x, &t.y, lo, hi, columns, |v| (scale.frac(v) * columns as f64).floor() as i64);
    let mut v = json!({
        "signal": t.label,
        "unit": t.unit,
        "end": round(t.y.last().copied().unwrap_or(f64::NAN)),
        "points": pts.iter().map(|p| json!([round(p[0]), round(p[1])])).collect::<Vec<_>>(),
    });
    let opt = |v: Option<f64>| v.map_or(Value::Null, round);
    if let Some(m) = sim::measure::measure(x, &t.y, from, to) {
        v["min"] = round(m.min);
        v["max"] = round(m.max);
        if !data.log_x {
            v["average"] = round(m.average);
            v["rms"] = round(m.rms);
            v["peak_to_peak"] = round(m.peak_to_peak);
            v["frequency"] = opt(m.frequency);
            v["rise"] = opt(m.rise);
            v["fall"] = opt(m.fall);
        }
    }
    if data.log_x
        && t.unit == "dB"
        && let Some(m) = sim::measure::measure_ac(x, &t.y, from, to)
    {
        v["peak"] = json!({ "db": round(m.peak_db), "frequency": round(m.peak_frequency) });
        v["minus_3db"] = json!([opt(m.minus_3db.0), opt(m.minus_3db.1)]);
    }
    if t.unit == "°"
        && let Some(mag) = data.traces.iter().find(|m| m.signal == t.signal && m.step == t.step && m.unit == "dB")
    {
        v["phase_margin"] = opt(sim::measure::phase_margin(x, &mag.y, &t.y, from, to));
    }
    if !points.is_empty() {
        v["at"] = json!(
            points
                .iter()
                .map(|&p| json!([p, round(plot::interpolate(x, &t.y, p).unwrap_or(f64::NAN))]))
                .collect::<Vec<_>>()
        );
    }
    v
}

/// The answer to `get_spice_netlist`: the netlist an analysis would run now.
pub fn netlist_reply(project: &Project, conn: &Connectivity, analysis: &Analysis) -> Value {
    let nl = sim::netlist::build(project, conn, analysis);
    let mut devices: Vec<(&str, &str)> = nl
        .devices
        .iter()
        .filter_map(|(id, dev)| Some((project.schematic.components.get(*id)?.refdes.as_str(), dev.as_str())))
        .collect();
    devices.sort_by(|a, b| model::natural_cmp(a.0, b.0));
    let problems: Vec<Value> = nl
        .problems
        .iter()
        .map(
            |p| json!({ "level": format!("{:?}", p.level).to_lowercase(), "message": p.message, "objects": p.objects }),
        )
        .collect();
    json!({
        "analysis": analysis.name,
        "netlist": nl.text,
        "nodes": nl.nodes,
        "devices": devices.into_iter().map(|(r, d)| json!({"refdes": r, "device": d})).collect::<Vec<_>>(),
        "problems": problems,
    })
}

/// The plot of a finished run as a scene in pixels (y up, origin at the lower left)
/// and the camera that shows exactly `width` × `height`: the whole run, the given
/// signals (AC with phase).
pub fn plot_scene(
    project: &Project,
    done: &Finished,
    signals: &[Signal],
    conn: &Connectivity,
    width: u32,
    height: u32,
) -> Option<(Scene, Camera)> {
    let data = plot::data_steps(&sim::tune::steps(project, done), signals, true, conn)?;
    let pal = view::style::pal();
    let (w, h) = (width as f32, height as f32);
    let mut prims: Vec<Instance> = Vec::new();
    let text = |prims: &mut Vec<Instance>, s: &str, at: [f32; 2], ha: HAlign, va: VAlign, color: Color| {
        let p = font::Placement {
            origin: at,
            height: 8.0,
            rotation: 0.0,
            halign: ha,
            valign: va,
            bold: false,
            mirror: false,
        };
        font::text_triangles(s, &p, |([a, b, c], e)| prims.push(Instance::triangle(a, b, c, e, color)));
    };
    prims.push(Instance::rect([w / 2.0, h / 2.0], [w, h], 0.0, 0.0, pal.background));
    let right = !data.axis_unit(Axis::Right).is_empty();
    let legend_h = 18.0;
    let (l, r, b, t) = (72.0, if right { w - 64.0 } else { w - 20.0 }, 44.0, h - 16.0 - legend_h);
    let (cw, ch) = (r - l, t - b);
    prims.push(Instance::rect([(l + r) / 2.0, (b + t) / 2.0], [cw, ch], 4.0, 0.0, pal.plot_bg));
    let (lo, hi) = (data.x.first().copied().unwrap_or(0.0), data.x.last().copied().unwrap_or(1.0));
    let x = Scale::new(lo, hi, data.log_x);
    let none = Default::default();
    let ys = [Axis::Left, Axis::Right].map(|a| {
        let (lo, hi) = plot::fit_y(&data, a, lo, hi, &none).unwrap_or((-1.0, 1.0));
        Scale::new(lo, hi, false)
    });
    let px = |v: f64| l + x.frac(v) as f32 * cw;
    let py = |s: &Scale, v: f64| b + s.frac(v) as f32 * ch;
    for v in x.ticks(((cw / 90.0) as usize).max(2)) {
        let p = px(v);
        prims.push(Instance::line([p, b], [p, t], 1.0, pal.plot_grid));
        text(&mut prims, &plot::label(v, data.x_unit), [p, b - 6.0], HAlign::Center, VAlign::Top, pal.plot_text);
    }
    text(&mut prims, &data.x_label, [(l + r) / 2.0, 6.0], HAlign::Center, VAlign::Bottom, pal.plot_text);
    for (i, axis) in [Axis::Left, Axis::Right].into_iter().enumerate() {
        let unit = data.axis_unit(axis);
        if unit.is_empty() {
            continue;
        }
        for v in ys[i].ticks(((ch / 40.0) as usize).max(2)) {
            let p = py(&ys[i], v);
            if i == 0 {
                prims.push(Instance::line([l, p], [r, p], 1.0, pal.plot_grid));
                text(&mut prims, &plot::label(v, unit), [l - 6.0, p], HAlign::Right, VAlign::Center, pal.plot_text);
            } else {
                text(&mut prims, &plot::label(v, unit), [r + 6.0, p], HAlign::Left, VAlign::Center, pal.plot_text);
            }
        }
    }
    for (a, b2) in [([l, b], [r, b]), ([r, b], [r, t]), ([r, t], [l, t]), ([l, t], [l, b])] {
        prims.push(Instance::line(a, b2, 1.0, pal.plot_grid));
    }
    // Traces, decimated per pixel column.
    let columns = cw.ceil() as usize;
    // A trace's colour: a sweep's steps fade towards the chart.
    let style = |tr: &plot::Trace| {
        let (index, fade) = data.color(tr);
        let (c, bg) = (view::style::trace_color(index).0, pal.plot_bg.0);
        let mix = |k: usize| (c[k] as f32 + (bg[k] as f32 - c[k] as f32) * fade).round() as u8;
        (Color([mix(0), mix(1), mix(2), c[3]]), tr.dashed || index >= 8)
    };
    for tr in &data.traces {
        let s = &ys[if tr.axis == Axis::Left { 0 } else { 1 }];
        let (color, dashed) = style(tr);
        let pts = plot::decimate(&data.x, &tr.y, lo, hi, columns, |v| (x.frac(v) * columns as f64).floor() as i64);
        let pts: Vec<[f32; 2]> =
            pts.iter().filter(|p| p[1].is_finite()).map(|p| [px(p[0]), py(s, p[1]).clamp(b, t)]).collect();
        polyline(&mut prims, &pts, 1.5, color, dashed);
    }
    // The legend above the chart.
    let mut lx = l;
    let ly = t + legend_h / 2.0 + 4.0;
    for tr in &data.traces {
        let (color, dashed) = style(tr);
        polyline(&mut prims, &[[lx, ly], [lx + 16.0, ly]], 3.0, color, dashed);
        text(&mut prims, &tr.label, [lx + 22.0, ly], HAlign::Left, VAlign::Center, pal.plot_text);
        lx += 22.0 + font::text_width(&tr.label, 8.0) + 18.0;
    }
    let mut scene = Scene::new();
    let layer = scene.add_layer("plot");
    scene.set_item(1, prims.into_iter().map(|p| (layer, p)));
    let camera = Camera { center: [w as f64 / 2.0, h as f64 / 2.0], zoom: 1.0 };
    Some((scene, camera))
}

/// Line segments through `pts`, dashed 6 on, 4 off.
fn polyline(out: &mut Vec<Instance>, pts: &[[f32; 2]], width: f32, color: Color, dashed: bool) {
    if !dashed {
        out.extend(pts.windows(2).map(|w| Instance::line(w[0], w[1], width, color)));
        return;
    }
    let (on, off) = (6.0, 4.0);
    let mut phase = 0.0f32;
    for w in pts.windows(2) {
        let (a, b) = (w[0], w[1]);
        let len = (b[0] - a[0]).hypot(b[1] - a[1]);
        let at = |d: f32| [a[0] + (b[0] - a[0]) * d / len, a[1] + (b[1] - a[1]) * d / len];
        let mut d = 0.0;
        while d < len {
            let in_on = phase < on;
            let step = if in_on { on - phase } else { on + off - phase }.min(len - d);
            if in_on {
                out.push(Instance::line(at(d), at(d + step), width, color));
            }
            d += step;
            phase = (phase + step) % (on + off);
        }
    }
}
