//! KiCad's simulation data → ours: the `Sim.*` fields of symbols (and KiCad 5's
//! `Spice_*`, migrated as KiCad does), the SPICE model libraries they name, and the
//! SPICE directives in schematic texts.
//!
//! * A component's fields become its part's [`SimModel`] (parts of one symbol whose
//!   models differ are split by the project import) and the component's
//!   [`ComponentSim`]: source waveforms, model parameter overrides, exclusion.
//! * Values in `Sim.*` fields use SI prefixes (`M` mega), directives SPICE's (`M`
//!   milli, `Meg` mega).
//! * Models from libraries (`Sim.Library` + `Sim.Name`) are embedded: the `.model` card,
//!   or the `.subckt` with the cards and subcircuits it uses from the same file.
//! * Potentiometers become the built-in `POT` model (its `pos` is the schematic's
//!   slider); voltage and current controlled switches subcircuits around ngspice's `S`
//!   and `W`.
//! * What has no equivalent here (IBIS, transmission lines, coupled inductors) is left
//!   unsimulated with a warning; sources KiCad generates
//!   differently (AM, FM, noise, random) and KiCad's raw SPICE device become SPICE line
//!   models, as KiCad writes them.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use model::*;

use crate::project::Report;
use crate::symbol::KiSymbol;

/// Fields read here; the import doesn't keep them as properties.
pub fn is_sim_field(key: &str) -> bool {
    key.starts_with("Sim.") || key.starts_with("Sim_") || key.starts_with("Spice_")
}

/// The simulation of one component.
#[derive(Clone, Debug, PartialEq)]
pub struct Mapped {
    /// Its part's model.
    pub model: SimModel,
    pub sim: ComponentSim,
    /// The component's value, where it changes: a source's shows its waveform, KiCad's
    /// `${SIM.PARAMS}` is resolved.
    pub value: Option<String>,
    /// A name for the model, to tell parts of one symbol with different models apart
    /// (`Q2N2222`).
    pub tag: String,
}

/// A component as KiCad describes its simulation (older fields migrated).
#[derive(Clone, Debug, Default, PartialEq)]
struct Fields {
    /// `R`, `V`, `NPN`, `SUBCKT`, `SPICE`, … (upper case).
    device: String,
    /// `Sim.Type`: `SIN`, `VDMOS`, … (upper case).
    kind: String,
    /// `Sim.Params`: (lower-case key, value); bare values have an empty key.
    params: Vec<(String, String)>,
    /// The `Sim.Params` text as written (for `${SIM.PARAMS}` in values).
    params_text: String,
    /// `Sim.Pins`: (symbol pin number, model pin name).
    pins: Vec<(String, String)>,
    library: String,
    name: String,
    enabled: bool,
}

impl Fields {
    fn param(&self, key: &str) -> Option<&str> {
        self.params.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
    }

    fn number(&self, key: &str) -> Option<f64> {
        self.param(key).and_then(si_number)
    }
}

/// The libraries (`Sim.Library`, `.include`) read so far, by resolved path.
pub struct Libs {
    dir: PathBuf,
    files: HashMap<PathBuf, Result<String, String>>,
}

impl Libs {
    /// Libraries relative to `dir` (the KiCad project's folder).
    pub fn new(dir: &Path) -> Libs {
        Libs { dir: dir.to_path_buf(), files: HashMap::new() }
    }

    /// The path of a library name: `${VAR}`s expanded (`KIPRJMOD`, KiCad's symbol
    /// folders, the environment), relative to the project.
    pub fn path(&self, name: &str) -> PathBuf {
        let mut out = String::new();
        let mut rest = name.trim().trim_matches('"');
        while let Some(i) = rest.find("${") {
            out.push_str(&rest[..i]);
            let Some(end) = rest[i..].find('}') else { break };
            let var = &rest[i + 2..i + end];
            out.push_str(&self.var(var).unwrap_or_else(|| format!("${{{var}}}")));
            rest = &rest[i + end + 1..];
        }
        out.push_str(rest);
        let p = PathBuf::from(out);
        if p.is_absolute() { p } else { self.dir.join(p) }
    }

    fn var(&self, var: &str) -> Option<String> {
        if var == "KIPRJMOD" {
            return Some(self.dir.to_string_lossy().into_owned());
        }
        if let Ok(v) = std::env::var(var) {
            return Some(v);
        }
        let symbols = var.starts_with("KICAD") && var.ends_with("_SYMBOL_DIR");
        symbols.then(|| crate::default_paths().0.into_iter().next()).flatten().map(|p| p.to_string_lossy().into_owned())
    }

    /// A library's text (included files inlined).
    pub fn text(&mut self, name: &str) -> Result<String, String> {
        let path = self.path(name);
        if let Some(r) = self.files.get(&path) {
            return r.clone();
        }
        let r = read_lib(&path, 0);
        self.files.insert(path, r.clone());
        r
    }
}

/// A library file with its `.include`s inlined.
fn read_lib(path: &Path, depth: usize) -> Result<String, String> {
    let text = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let text = String::from_utf8_lossy(&text).into_owned();
    if depth > 8 {
        return Ok(text);
    }
    let dir = path.parent().unwrap_or(Path::new("."));
    let mut out = String::with_capacity(text.len());
    for line in text.lines() {
        let t = line.trim();
        let lower = t.to_ascii_lowercase();
        if lower.starts_with(".include") || lower.starts_with(".inc ") {
            let file = t.split_once(char::is_whitespace).map_or("", |x| x.1).trim().trim_matches(['"', '\'']);
            let p = dir.join(file);
            if let Ok(inner) = read_lib(&p, depth + 1) {
                out.push_str(&inner);
                out.push('\n');
                continue;
            }
        }
        out.push_str(line);
        out.push('\n');
    }
    Ok(out)
}

/// The `.model` and `.subckt` definitions of a library, by lower-case name.
struct LibItems {
    items: Vec<(String, bool, String)>, // (name, is subckt, text)
}

impl LibItems {
    fn parse(text: &str) -> LibItems {
        let mut items = Vec::new();
        let lines: Vec<&str> = text.lines().collect();
        let mut i = 0;
        while i < lines.len() {
            let t = lines[i].trim_start();
            let lower = t.to_ascii_lowercase();
            if lower.starts_with(".model") {
                let mut text = lines[i].to_string();
                while i + 1 < lines.len() && lines[i + 1].trim_start().starts_with('+') {
                    i += 1;
                    text.push('\n');
                    text.push_str(lines[i]);
                }
                if let Some(name) = t.split_whitespace().nth(1) {
                    items.push((name.to_ascii_lowercase(), false, text));
                }
            } else if lower.starts_with(".subckt") {
                let name = t.split_whitespace().nth(1).unwrap_or("").to_ascii_lowercase();
                let start = i;
                let mut depth = 0;
                while i < lines.len() {
                    let l = lines[i].trim_start().to_ascii_lowercase();
                    if l.starts_with(".subckt") {
                        depth += 1;
                    } else if l.starts_with(".ends") {
                        depth -= 1;
                        if depth == 0 {
                            break;
                        }
                    }
                    i += 1;
                }
                let end = i.min(lines.len() - 1);
                items.push((name, true, lines[start..=end].join("\n")));
            }
            i += 1;
        }
        LibItems { items }
    }

    fn get(&self, name: &str) -> Option<(bool, &str)> {
        let name = name.to_ascii_lowercase();
        self.items.iter().find(|(n, _, _)| *n == name).map(|(_, s, t)| (*s, t.as_str()))
    }

    /// The definition of `name`, followed by those it uses (by name, recursively).
    fn with_dependencies(&self, name: &str) -> Option<(bool, String)> {
        let (subckt, first) = self.get(name)?;
        let mut taken = vec![name.to_ascii_lowercase()];
        let mut texts = vec![first.to_string()];
        let mut k = 0;
        while k < texts.len() {
            let body = texts[k].clone();
            for line in logical_lines(&body).iter().skip(1) {
                if line.starts_with('.') {
                    continue;
                }
                for token in line.split(|c: char| c.is_whitespace() || "(),=".contains(c)) {
                    let t = token.to_ascii_lowercase();
                    if t.is_empty() || taken.contains(&t) {
                        continue;
                    }
                    if let Some((_, text)) = self.get(&t) {
                        taken.push(t);
                        texts.push(text.to_string());
                    }
                }
            }
            k += 1;
        }
        Some((subckt, texts.join("\n")))
    }
}

// ---------------------------------------------------------------------------------------
// Fields

/// The `Sim.Params` syntax: `key=value` pairs (spaces around `=` allowed, values may be
/// quoted with `\"` escapes), bare values with an empty key.
fn parse_params(text: &str) -> Vec<(String, String)> {
    let mut tokens: Vec<String> = Vec::new();
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c.is_whitespace() {
            i += 1;
        } else if c == '=' {
            tokens.push("=".into());
            i += 1;
        } else if c == '\'' {
            // A SPICE expression in single quotes, kept with them.
            let mut s = String::from("'");
            i += 1;
            while i < chars.len() && chars[i] != '\'' {
                s.push(chars[i]);
                i += 1;
            }
            s.push('\'');
            i += 1;
            tokens.push(s);
        } else if c == '"' {
            let mut s = String::new();
            i += 1;
            while i < chars.len() && chars[i] != '"' {
                if chars[i] == '\\' && i + 1 < chars.len() {
                    i += 1;
                }
                s.push(chars[i]);
                i += 1;
            }
            i += 1;
            tokens.push(format!("\"{s}"));
        } else {
            let mut s = String::new();
            while i < chars.len() && !chars[i].is_whitespace() && chars[i] != '=' {
                s.push(chars[i]);
                i += 1;
            }
            tokens.push(s);
        }
    }
    let unquote = |t: &str| t.strip_prefix('"').unwrap_or(t).to_string();
    let mut out = Vec::new();
    let mut k = 0;
    while k < tokens.len() {
        if tokens.get(k + 1).is_some_and(|t| t == "=") {
            let value = tokens.get(k + 2).map(|t| unquote(t)).unwrap_or_default();
            out.push((unquote(&tokens[k]).to_ascii_lowercase(), value));
            k += 3;
        } else {
            out.push((String::new(), unquote(&tokens[k])));
            k += 1;
        }
    }
    out
}

/// `Sim.Pins`: `1=+ 2=-` pairs (symbol pin number, model pin name).
fn parse_pins(text: &str) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    for pair in text.split_whitespace() {
        if let Some((s, m)) = pair.split_once('=')
            && !out.iter().any(|(a, b)| a == s && b == m)
        {
            out.push((s.to_string(), m.to_string()));
        }
    }
    out
}

fn enabled(text: &str) -> bool {
    !matches!(text.trim().chars().next().map(|c| c.to_ascii_lowercase()), Some('n' | 'f' | '0'))
}

/// The fields of a component, KiCad 5's `Spice_*` migrated (as KiCad 7 does). `None`
/// when there are none.
fn fields(props: &BTreeMap<String, String>, value: &str) -> Option<Fields> {
    let get = |k: &str| props.get(k).map(|s| s.trim().to_string()).unwrap_or_default();
    let legacy = ["Spice_Primitive", "Spice_Model", "Spice_Node_Sequence", "Spice_Lib_File", "Spice_Netlist_Enabled"]
        .iter()
        .any(|k| props.contains_key(*k));
    if legacy {
        return Some(legacy_fields(props, value));
    }
    let any = props.keys().any(|k| k.starts_with("Sim.") || k.starts_with("Sim_"));
    let or = |a: &str, b: &str| if props.contains_key(a) { get(a) } else { get(b) };
    let params_text = or("Sim.Params", "Sim_Params");
    let f = Fields {
        device: or("Sim.Device", "Sim_Device").to_ascii_uppercase(),
        kind: or("Sim.Type", "Sim_Type").to_ascii_uppercase(),
        params: parse_params(&params_text),
        params_text,
        pins: parse_pins(&get("Sim.Pins")),
        library: get("Sim.Library"),
        name: get("Sim.Name"),
        enabled: enabled(&get("Sim.Enable")),
    };
    any.then_some(f)
}

fn legacy_fields(props: &BTreeMap<String, String>, value: &str) -> Fields {
    let get = |k: &str| props.get(k).map(|s| s.trim().to_string()).unwrap_or_default();
    let device = get("Spice_Primitive").to_ascii_uppercase();
    let model = props.get("Spice_Model").map_or_else(|| value.trim().to_string(), |m| m.trim().to_string());
    let from_value = !props.contains_key("Spice_Model");
    let lib = get("Spice_Lib_File");
    // The node sequence: symbol pins in model pin order.
    let pins: Vec<(String, String)> = get("Spice_Node_Sequence")
        .split(|c: char| "{:,; }".contains(c))
        .filter(|s| !s.is_empty())
        .enumerate()
        .map(|(k, s)| (s.to_string(), (k + 1).to_string()))
        .collect();
    let mut f = Fields { pins, enabled: enabled(&get("Spice_Netlist_Enabled")), ..Default::default() };
    if !lib.is_empty() {
        let (name, rest) = model.split_once(' ').unwrap_or((&model, ""));
        f.library = lib;
        f.name = name.to_string();
        f.params = parse_params(rest);
        f.params_text = rest.to_string();
        return f;
    }
    if ["R", "L", "C", "V", "I"].contains(&device.as_str()) && from_value {
        // Inferred from the value, like a symbol without fields.
        f.device = device;
        f.params_text = String::new();
        return f;
    }
    f.device = device.clone();
    f.params_text = model.clone();
    f.params = vec![(String::new(), model)];
    f.kind = "LEGACY".into();
    f
}

// ---------------------------------------------------------------------------------------
// Numbers

/// A number in KiCad's SI notation (`4k7`, `10M` mega, `1Meg`, `100n`).
fn si_number(text: &str) -> Option<f64> {
    let t = text.trim();
    if t.is_empty() || !t.starts_with(|c: char| c.is_ascii_digit() || c == '.' || c == '-' || c == '+') {
        return None;
    }
    // `100uF;16V`, `10k/1%`: the number in front.
    if let Some(i) = t.find([';', ',', '/'])
        && i > 0
    {
        return si_number(&t[..i]);
    }
    si::parse(t).or_else(|| {
        // KiCad reads the prefixes in either case (`1UF`, `1g`), but `M` and `m`.
        let i = t.find(|c: char| !(c.is_ascii_digit() || ".+-".contains(c)))?;
        let fixed = match t[i..].chars().next()? {
            'U' => 'u',
            'N' => 'n',
            'P' => 'p',
            'g' => 'G',
            't' => 'T',
            _ => return None,
        };
        si::parse(&format!("{}{fixed}{}", &t[..i], &t[i + 1..]))
    })
}

/// A number in SPICE notation: suffixes case-insensitive, `m` milli, `meg` mega, `mil`;
/// trailing letters (units) ignored.
pub fn spice_number(text: &str) -> Option<f64> {
    let t = text.trim().to_ascii_lowercase();
    let end = t
        .char_indices()
        .find(|&(i, c)| {
            !(c.is_ascii_digit()
                || c == '.'
                || ((c == '-' || c == '+') && (i == 0 || t[..i].ends_with('e')))
                || (c == 'e' && t[i + 1..].starts_with(|d: char| d.is_ascii_digit() || d == '-' || d == '+')))
        })
        .map_or(t.len(), |(i, _)| i);
    let v: f64 = t[..end].parse().ok()?;
    let rest = &t[end..];
    if rest.starts_with("mil") {
        return Some(v * 25.4e-6);
    }
    let power = if rest.starts_with("meg") {
        6
    } else {
        match rest.chars().next() {
            Some('t') => 12,
            Some('g') => 9,
            Some('k') => 3,
            Some('m') => -3,
            Some('u' | 'µ' | 'μ') => -6,
            Some('n') => -9,
            Some('p') => -12,
            Some('f') => -15,
            _ => 0,
        }
    };
    // Divided for small prefixes: 100n is exactly 1e-7.
    Some(if power < 0 { v / 10f64.powi(-power) } else { v * 10f64.powi(power) })
}

/// A `Sim.*` value as SPICE reads it: numbers converted, anything else as written.
fn to_spice(text: &str) -> String {
    match si_number(text) {
        Some(v) => si::spice(v),
        None => text.trim().to_string(),
    }
}

// ---------------------------------------------------------------------------------------
// Components

/// A symbol's pins: (unit, pin, number), each number once.
fn symbol_pins(sym: &KiSymbol) -> Vec<(u32, Id, String)> {
    let mut out: Vec<(u32, Id, String)> = Vec::new();
    for (u, s) in sym.units.iter().enumerate() {
        for p in s.pins.iter() {
            let n = p.key().to_string();
            if !out.iter().any(|(_, _, x)| *x == n) {
                out.push((u as u32, p.id, n));
            }
        }
    }
    out
}

/// The symbol pins of a model's `ports` (KiCad's model pin names, in node order): by
/// `Sim.Pins` (a pin name, or its position for numbered pins), else the symbol's pins in
/// number order. Optional ports left unmapped are dropped.
fn map_pins(f: &Fields, ports: &[&str], optional: usize, sym: &KiSymbol) -> Result<Vec<SimPin>, String> {
    let pins = symbol_pins(sym);
    let pin = |number: &str| pins.iter().find(|(_, _, n)| n == number).map(|(u, id, _)| SimPin { unit: *u, pin: *id });
    let required = ports.len() - optional;
    let mut out = Vec::new();
    if f.pins.is_empty() {
        let mut numbers: Vec<&str> = pins.iter().map(|(_, _, n)| n.as_str()).collect();
        numbers.sort();
        if numbers.len() < required {
            return Err(format!("the model has {} pins, the symbol {}", required, numbers.len()));
        }
        for n in numbers.iter().take(ports.len()) {
            out.push(pin(n).expect("listed"));
        }
        return Ok(out);
    }
    for (k, port) in ports.iter().enumerate() {
        let by_name = f.pins.iter().find(|(_, m)| m.eq_ignore_ascii_case(port));
        let by_index = || f.pins.iter().find(|(_, m)| m.parse::<usize>().ok() == Some(k + 1));
        match by_name.or_else(by_index) {
            Some((number, _)) => out.push(
                pin(number).ok_or_else(|| format!("Sim.Pins names pin {number}, which the symbol doesn't have"))?,
            ),
            None if k >= required => break,
            None => return Err(format!("no symbol pin is assigned to the model's pin {port}")),
        }
    }
    Ok(out)
}

/// The simulation of a component from its properties: `Ok(None)` when it has none (no
/// fields, nothing to infer), `Err` when it can't be simulated here (a warning).
pub fn component(
    props: &BTreeMap<String, String>,
    value: &str,
    refdes: &str,
    exclude: bool,
    sym: &KiSymbol,
    libs: &mut Libs,
) -> Result<Option<Mapped>, String> {
    let prefix: String = refdes.chars().take_while(|c| !c.is_ascii_digit()).collect::<String>().to_ascii_uppercase();
    let mut f = match fields(props, value) {
        Some(f) => f,
        None => Fields { enabled: true, ..Default::default() },
    };
    // The value as KiCad shows it.
    let value = value.replace("${SIM.PARAMS}", &f.params_text).replace("${SIM.NAME}", &f.name);
    let exclude = exclude || !f.enabled;
    if f.device.is_empty() && f.library.is_empty() {
        // Inferred from the reference and value: R, C, L, V, I with two pins.
        let letter = prefix.chars().next().map(String::from).unwrap_or_default();
        if !["R", "C", "L", "V", "I"].contains(&letter.as_str()) || value.trim().is_empty() || !f.kind.is_empty() {
            return Ok(None);
        }
        if symbol_pins(sym).len() != 2 {
            return Ok(None);
        }
        f.device = letter;
    }
    let mut m = map(&f, &value, sym, libs)?;
    m.sim.exclude = exclude;
    m.model = joined_ports(m.model);
    Ok(Some(m))
}

/// A model whose ports share symbol pins (`Sim.Pins "3=3 3=4"`: source and bulk on one
/// pin) with each pin once: a SPICE line's nodes renumbered, a subcircuit or primitive
/// wrapped in a subcircuit that joins the ports.
fn joined_ports(model: SimModel) -> SimModel {
    let mut distinct: Vec<SimPin> = Vec::new();
    let mut node_of: Vec<usize> = Vec::new();
    for p in &model.pins {
        let k = distinct.iter().position(|d| d == p).unwrap_or_else(|| {
            distinct.push(*p);
            distinct.len() - 1
        });
        node_of.push(k);
    }
    if distinct.len() == model.pins.len() {
        return model;
    }
    let ports: Vec<String> = (1..=distinct.len()).map(|k| format!("p{k}")).collect();
    let nodes: Vec<&str> = node_of.iter().map(|&k| ports[k].as_str()).collect();
    let device = match &model.device {
        SimDevice::Spice { template } => {
            let mut t = template.clone();
            for (k, &n) in node_of.iter().enumerate().rev() {
                t = t.replace(&format!("${{N{}}}", k + 1), &format!("${{M{}}}", n + 1));
            }
            SimDevice::Spice { template: t.replace("${M", "${N") }
        }
        SimDevice::Subckt { text } => {
            let Some(h) = SubcktHeader::parse(text) else { return model };
            // Side by side: ngspice doesn't take every subcircuit nested.
            let (name, ports, nodes) = (&h.name, ports.join(" "), nodes.join(" "));
            let text = format!(".subckt {name}_J {ports}\nX1 {nodes} {name}\n.ends\n{text}");
            SimDevice::Subckt { text }
        }
        SimDevice::Primitive { kind, model: card } => {
            let Some(c) = ModelCard::parse(card) else { return model };
            let (name, ports, nodes, letter) = (&c.name, ports.join(" "), nodes.join(" "), kind.letter());
            let text = format!(".subckt {name}_J {ports}\n{letter}1 {nodes} {name}\n.ends\n{card}");
            SimDevice::Subckt { text }
        }
        _ => return model,
    };
    SimModel { device, pins: distinct, params: model.params }
}

fn mapped(model: SimModel, tag: impl Into<String>) -> Mapped {
    Mapped { model, sim: ComponentSim::default(), value: None, tag: tag.into() }
}

fn map(f: &Fields, value: &str, sym: &KiSymbol, libs: &mut Libs) -> Result<Mapped, String> {
    let device = f.device.as_str();
    match device {
        "IBIS" | "KIBIS" => return Err("IBIS models aren't supported".into()),
        "TLINE" => return Err("transmission lines aren't supported".into()),
        "K" => return Err("coupled inductors aren't supported".into()),
        _ => {}
    }
    if device == "XSPICE" {
        return xspice(f, sym, libs);
    }
    if device == "SW" {
        return switch(f, sym);
    }
    if !f.library.is_empty() && device != "SPICE" {
        return library_model(f, sym, libs);
    }
    match device {
        "R" | "C" | "L" => passive(f, value, sym),
        "V" | "I" => source(f, value, sym),
        "D" | "NPN" | "PNP" | "NJFET" | "PJFET" | "NMOS" | "PMOS" | "NMES" | "PMES" => builtin_primitive(f, sym),
        "E" | "F" | "G" | "H" => controlled(f, sym),
        "SPICE" => raw_spice(f, sym, libs),
        "SUBCKT" => Err("a subcircuit without a library (Sim.Library)".into()),
        other => Err(format!("KiCad's {other} model isn't supported")),
    }
}

/// A one-line SPICE model: `{letter}${REF} ${N1} … {rest}`.
fn spice_line(letter: &str, nodes: usize, rest: &str) -> SimDevice {
    let nodes: String = (1..=nodes).map(|k| format!(" ${{N{k}}}")).collect();
    SimDevice::Spice { template: format!("{letter}${{REF}}{nodes} {}", rest.trim()) }
}

fn passive(f: &Fields, value: &str, sym: &KiSymbol) -> Result<Mapped, String> {
    if f.device == "R" && f.kind == "POT" {
        return potentiometer(f, value, sym);
    }
    let pins = map_pins(f, &["+", "-"], 0, sym)?;
    let letter = f.device.as_str();
    let key = letter.to_ascii_lowercase();
    let given = f.param(&key).or_else(|| f.param(""));
    // Behavioral: a value that isn't a number is an expression (`'time > 1m ? 8 : 80'`).
    let behavioral = f.kind == "=" || (f.kind.is_empty() && si_number(given.unwrap_or(value)).is_none());
    if behavioral {
        let expr = given.unwrap_or(value).trim();
        let spec = if expr.to_ascii_lowercase().replace(' ', "").starts_with(&format!("{key}=")) {
            expr.to_string()
        } else {
            format!("{key}={expr}")
        };
        let model = SimModel { pins, ..SimModel::new(spice_line(letter, 2, &spec)) };
        return Ok(mapped(model, "behavioral"));
    }
    match f.kind.as_str() {
        "" | "=" => {}
        other => return Err(format!("KiCad's {letter} model {other} isn't supported")),
    }
    let device = match letter {
        "R" => SimDevice::Resistor,
        "C" => SimDevice::Capacitor,
        _ => SimDevice::Inductor,
    };
    let mut m = mapped(SimModel { pins: pins.clone(), ..SimModel::new(device) }, "");
    let Some(text) = given else {
        if si::parse(value).is_none() {
            // `1UF`: the number as we read it.
            m.value = si_number(value).map(si::value_text);
        }
        return Ok(m);
    };
    let v = si_number(text).ok_or_else(|| format!("the value {text:?} is not a number"))?;
    if si_number(value) != Some(v) {
        if value.trim().is_empty() || value.contains(&f.params_text) {
            m.value = Some(text.trim().to_string());
        } else {
            // The value field says something else: the model keeps KiCad's number.
            let model = SimModel { pins, ..SimModel::new(spice_line(letter, 2, &si::spice(v))) };
            return Ok(mapped(model, si::value_text(v)));
        }
    }
    Ok(m)
}

/// KiCad's potentiometer (`r`, `pos`; pins r0, wiper, r1): the built-in `POT`, the
/// resistance as the value, the position the component's (0 at r0, as ngspice's).
fn potentiometer(f: &Fields, value: &str, sym: &KiSymbol) -> Result<Mapped, String> {
    let pins = map_pins(f, &["r0", "wiper", "r1"], 0, sym)?;
    let params = [("r", "${VALUE}"), ("pos", "0.5")].map(|(k, v)| (k.to_string(), v.to_string())).into();
    let text = model::builtin::generate::sims::POT.to_string();
    let mut m = mapped(SimModel { device: SimDevice::Subckt { text }, pins, params }, "POT");
    if let Some(r) = f.param("r") {
        let v = si_number(r).ok_or_else(|| format!("the resistance {r:?} is not a number"))?;
        if si_number(value) != Some(v) {
            m.value = Some(si::value_text(v));
        }
    } else if si_number(value).is_none() {
        return Err("a potentiometer without its resistance (r)".into());
    }
    if let Some(pos) = f.param("pos") {
        let v = si_number(pos).ok_or_else(|| format!("the position {pos:?} is not a number"))?;
        m.sim.params.insert("pos".into(), SimControl::Position.format(v));
    }
    Ok(m)
}

/// KiCad's switches: voltage (`V`: ngspice `S`, `thr`/`his` as `vt`/`vh`) or current
/// controlled (`I`: `W`, through a 0 V source between ctrl+ and ctrl−, `it`/`ih`); pins
/// no+, no−, ctrl+, ctrl−.
fn switch(f: &Fields, sym: &KiSymbol) -> Result<Mapped, String> {
    let pins = map_pins(f, &["no+", "no-", "ctrl+", "ctrl-"], 0, sym)?;
    let (current, prefix) = match f.kind.as_str() {
        "V" | "" => (false, "v"),
        "I" => (true, "i"),
        other => return Err(format!("KiCad's switch model {other} isn't supported")),
    };
    let mut card = Vec::new();
    for (key, ours) in [("thr", "t"), ("his", "h"), ("ron", "ron"), ("roff", "roff")] {
        if let Some(v) = f.param(key) {
            let name = if ours.starts_with('r') { ours.to_string() } else { format!("{prefix}{ours}") };
            card.push(format!("{name}={}", to_spice(v)));
        }
    }
    let card = card.join(" ");
    let (name, body) = if current {
        ("KSW_I", format!("Vc cp cn 0\nW1 nop non Vc KCSW\n.model KCSW CSW({card})"))
    } else {
        ("KSW_V", format!("S1 nop non cp cn KSW\n.model KSW SW({card})"))
    };
    let text = format!(".subckt {name} nop non cp cn\n{body}\n.ends {name}");
    let model = SimModel { pins, ..SimModel::new(SimDevice::Subckt { text }) };
    Ok(mapped(model, if current { "CSW" } else { "SW" }))
}

/// The waveform of a KiCad source, or (for kinds that have no waveform here) its SPICE
/// arguments.
enum Wave {
    Source(Source),
    Line(String),
}

fn source(f: &Fields, value: &str, sym: &KiSymbol) -> Result<Mapped, String> {
    let pins = map_pins(f, &["+", "-"], 0, sym)?;
    let letter = f.device.as_str();
    let wave = if f.kind == "LEGACY" {
        legacy_source(f.param("").unwrap_or(""))?
    } else if f.kind.is_empty() && f.params.is_empty() {
        // Inferred: the value is the DC level ("5", "DC 5", "AC 1"), or a SPICE spec.
        let v = value.trim();
        let (key, rest) = if let Some(r) = v.strip_prefix("DC ") {
            ("dc", r)
        } else if let Some(r) = v.strip_prefix("AC ") {
            ("ac", r)
        } else {
            ("dc", v)
        };
        match si_number(rest) {
            Some(x) if key == "dc" => {
                Wave::Source(Source { waveform: Waveform::Dc { value: x }, ..Default::default() })
            }
            Some(x) => Wave::Source(Source { ac: x, ..Default::default() }),
            None => legacy_source(v)?,
        }
    } else {
        modern_source(f, value)?
    };
    let unit = if letter == "I" { "A" } else { "V" };
    match wave {
        Wave::Source(s) => {
            let device = if letter == "I" { SimDevice::CurrentSource } else { SimDevice::VoltageSource };
            let mut m = mapped(SimModel { pins, ..SimModel::new(device) }, "");
            m.value = Some(s.describe(unit));
            m.sim.source = Some(s);
            Ok(m)
        }
        Wave::Line(spec) => {
            let model = SimModel { pins, ..SimModel::new(spice_line(letter, 2, &spec)) };
            let tag = spec.split('(').next().unwrap_or("").split_whitespace().last().unwrap_or("").to_string();
            Ok(mapped(model, tag))
        }
    }
}

/// `Sim.Type` + `Sim.Params` of a source.
fn modern_source(f: &Fields, value: &str) -> Result<Wave, String> {
    let n = |k: &str| f.number(k).unwrap_or(0.0);
    let ac = n("ac");
    let ac_phase = n("ph");
    let with = |waveform: Waveform| Ok(Wave::Source(Source { waveform, ac, ac_phase }));
    let kind = if f.kind.is_empty() { "DC" } else { f.kind.as_str() };
    match kind {
        "DC" => {
            // Without `dc=`, the value is the level ("5", "DC 5"; "AC 1" an AC input).
            let v = value.trim();
            let given = f.number("dc").or_else(|| f.number(""));
            let (dc, ac) = match (given, v.strip_prefix("AC ")) {
                (Some(dc), _) => (dc, ac),
                (None, Some(a)) => (0.0, si_number(a).unwrap_or(ac)),
                (None, None) => (si_number(v.strip_prefix("DC ").unwrap_or(v)).unwrap_or(0.0), ac),
            };
            Ok(Wave::Source(Source { waveform: Waveform::Dc { value: dc }, ac, ac_phase }))
        }
        "SIN" => with(Waveform::Sine {
            offset: n("dc"),
            amplitude: n("ampl"),
            frequency: f.number("f").ok_or("a sine source without frequency (f)")?,
            delay: n("td"),
            damping: n("theta"),
            phase: n("phase"),
        }),
        "PULSE" => with(Waveform::Pulse {
            low: n("y1"),
            high: n("y2"),
            delay: n("td"),
            rise: n("tr"),
            fall: n("tf"),
            width: n("tw"),
            period: n("per"),
        }),
        "EXP" => with(Waveform::Exp {
            v1: n("y1"),
            v2: n("y2"),
            delay1: n("td1"),
            tau1: n("tau1"),
            delay2: n("td2"),
            tau2: n("tau2"),
        }),
        "PWL" => {
            let text = f.param("pwl").unwrap_or("");
            match pwl_points(text, si_number) {
                Some(points) => with(Waveform::Pwl { points, repeat: false }),
                None => Ok(Wave::Line(format!(
                    "PWL({})",
                    text.split_whitespace().map(to_spice).collect::<Vec<_>>().join(" ")
                ))),
            }
        }
        _ => {
            let arg = |k: &str, default: &str| f.param(k).map_or(default.to_string(), to_spice);
            let (func, args): (&str, Vec<String>) = match kind {
                "AM" => ("AM", positional(f, &["vo", "vmo", "vma", "fm", "fc", "td", "phasem", "phasec"])),
                "SFFM" => ("SFFM", positional(f, &["vo", "va", "fm", "mdi", "fc", "phasem", "phasec"])),
                "WHITENOISE" => ("TRNOISE", vec![arg("rms", "0"), arg("dt", "0"), "0 0 0 0 0".into()]),
                "PINKNOISE" => {
                    ("TRNOISE", vec!["0".into(), arg("dt", "0"), arg("slope", "0"), arg("rms", "0"), "0 0 0".into()])
                }
                "BURSTNOISE" => {
                    ("TRNOISE", vec!["0 0 0 0".into(), arg("ampl", "0"), arg("tcapt", "0"), arg("temit", "0")])
                }
                "RANDUNIFORM" => (
                    "TRRANDOM",
                    vec!["1".into(), arg("ts", "0"), arg("td", "0"), arg("range", "1"), arg("offset", "0")],
                ),
                "RANDGAUSSIAN" | "RANDNORMAL" => {
                    ("TRRANDOM", vec!["2".into(), arg("ts", "0"), arg("td", "0"), arg("stddev", "1"), arg("mean", "0")])
                }
                "RANDEXP" => {
                    ("TRRANDOM", vec!["3".into(), arg("ts", "0"), arg("td", "0"), arg("mean", "1"), arg("offset", "0")])
                }
                "RANDPOISSON" => (
                    "TRRANDOM",
                    vec!["4".into(), arg("ts", "0"), arg("td", "0"), arg("lambda", "1"), arg("offset", "0")],
                ),
                other => return Err(format!("KiCad's {other} source isn't supported")),
            };
            let mut spec = format!("DC {} {func}({})", arg("dc", "0"), args.join(" "));
            if ac != 0.0 {
                spec.push_str(&format!(" AC {} {}", si::spice(ac), si::spice(ac_phase)));
            }
            Ok(Wave::Line(spec))
        }
    }
}

/// Values of `keys` in order, up to the last one given (gaps as 0).
fn positional(f: &Fields, keys: &[&str]) -> Vec<String> {
    let last = keys.iter().rposition(|k| f.param(k).is_some()).map_or(0, |i| i + 1);
    keys[..last].iter().map(|k| f.param(k).map_or("0".to_string(), to_spice)).collect()
}

/// `t v t v …` → points (`None` when something isn't a number or times go back).
fn pwl_points(text: &str, number: fn(&str) -> Option<f64>) -> Option<Vec<(f64, f64)>> {
    let values: Vec<f64> = text.split_whitespace().map(number).collect::<Option<Vec<f64>>>()?;
    if values.len() < 2 || !values.len().is_multiple_of(2) {
        return None;
    }
    let points: Vec<(f64, f64)> = values.chunks(2).map(|c| (c[0], c[1])).collect();
    points.windows(2).all(|w| w[1].0 > w[0].0).then_some(points)
}

/// A SPICE source spec (KiCad 5's `Spice_Model`, or a value): `5`, `dc 5`, `dc(1)`,
/// `sin(0 1 1k)`, `pulse(…)`, `exp(…)`, `pwl(…)`, `ac 1`, and other functions as written.
fn legacy_source(spec: &str) -> Result<Wave, String> {
    let lower = spec.trim().to_ascii_lowercase();
    if lower.is_empty() {
        return Ok(Wave::Source(Source::default()));
    }
    let tokens: Vec<&str> = lower
        .split(|c: char| c.is_whitespace() || c == '(' || c == ')' || c == ',')
        .filter(|t| !t.is_empty())
        .collect();
    let mut s = Source::default();
    let mut i = 0;
    let num = |t: Option<&&str>| t.and_then(|t| spice_number(t));
    let mut known = true;
    while i < tokens.len() {
        let t = tokens[i];
        let args: Vec<f64> = tokens[i + 1..].iter().map_while(|t| spice_number(t)).collect();
        let a = |k: usize| args.get(k).copied().unwrap_or(0.0);
        match t {
            "dc" => {
                s.waveform = Waveform::Dc { value: num(tokens.get(i + 1)).unwrap_or(0.0) };
                i += 2;
                continue;
            }
            "ac" => {
                s.ac = a(0);
                s.ac_phase = a(1);
            }
            "sin" => {
                s.waveform = Waveform::Sine {
                    offset: a(0),
                    amplitude: a(1),
                    frequency: a(2),
                    delay: a(3),
                    damping: a(4),
                    phase: a(5),
                }
            }
            "pulse" => {
                s.waveform = Waveform::Pulse {
                    low: a(0),
                    high: a(1),
                    delay: a(2),
                    rise: a(3),
                    fall: a(4),
                    width: a(5),
                    period: a(6),
                }
            }
            "exp" => {
                s.waveform = Waveform::Exp { v1: a(0), v2: a(1), delay1: a(2), tau1: a(3), delay2: a(4), tau2: a(5) }
            }
            "pwl" => {
                let points: Vec<(f64, f64)> = args.chunks(2).filter(|c| c.len() == 2).map(|c| (c[0], c[1])).collect();
                if points.is_empty() || points.windows(2).any(|w| w[1].0 <= w[0].0) {
                    known = false;
                }
                s.waveform = Waveform::Pwl { points, repeat: false };
            }
            _ if spice_number(t).is_some() && i == 0 => {
                s.waveform = Waveform::Dc { value: spice_number(t).unwrap_or(0.0) };
            }
            _ => known = false,
        }
        i += 1 + args.len();
    }
    if known { Ok(Wave::Source(s)) } else { Ok(Wave::Line(spec.trim().to_string())) }
}

/// KiCad's ports, our kind and the `.model` type of a primitive device.
fn primitive_info(device: &str) -> (&'static [&'static str], usize, PrimitiveKind, &'static str) {
    match device {
        "D" => (&["A", "K"], 0, PrimitiveKind::Diode, "D"),
        "NPN" => (&["C", "B", "E", "S"], 1, PrimitiveKind::Npn, "NPN"),
        "PNP" => (&["C", "B", "E", "S"], 1, PrimitiveKind::Pnp, "PNP"),
        "NJFET" => (&["D", "G", "S"], 0, PrimitiveKind::Njf, "NJF"),
        "PJFET" => (&["D", "G", "S"], 0, PrimitiveKind::Pjf, "PJF"),
        "NMES" => (&["D", "G", "S"], 0, PrimitiveKind::Njf, "NMF"),
        "PMES" => (&["D", "G", "S"], 0, PrimitiveKind::Pjf, "PMF"),
        "NMOS" => (&["D", "G", "S", "B"], 1, PrimitiveKind::Nmos, "NMOS"),
        _ => (&["D", "G", "S", "B"], 1, PrimitiveKind::Pmos, "PMOS"),
    }
}

/// `level=` of KiCad's model types.
fn level(kind: &str) -> Result<Option<&'static str>, String> {
    Ok(match kind {
        "" | "GUMMELPOON" | "JFET" | "JFET1" | "MOS1" | "STATZ" | "MES" => None,
        "VBIC" => Some("4"),
        "HICUM2" => Some("8"),
        "JFET2" => Some("2"),
        "MOS2" => Some("2"),
        "MOS3" => Some("3"),
        "BSIM1" => Some("4"),
        "BSIM2" => Some("5"),
        "MOS6" => Some("6"),
        "BSIM3" => Some("8"),
        "MOS9" => Some("9"),
        "B4SOI" => Some("10"),
        "BSIM4" => Some("14"),
        "B3SOIFD" => Some("55"),
        "B3SOIDD" => Some("56"),
        "B3SOIPD" => Some("57"),
        "HISIM2" => Some("68"),
        "HISIMHV1" => Some("73"),
        "VDMOS" => None,
        other => return Err(format!("KiCad's {other} model isn't supported")),
    })
}

/// A primitive from a `.model` card: ours when it fits (MOSFET bulk on the source, no
/// BJT substrate), else wrapped in a subcircuit with all its pins.
fn primitive(device: &str, card: String, f: &Fields, sym: &KiSymbol, params: bool) -> Result<Mapped, String> {
    let (ports, optional, kind, _) = primitive_info(device);
    let pins = map_pins(f, ports, optional, sym)?;
    let parsed = ModelCard::parse(&card).ok_or("the model text has no .model line")?;
    let fits = PrimitiveKind::of_card(&parsed) == Some(kind);
    let tag = parsed.name.clone();
    let mut m = if pins.len() == 3 - usize::from(kind == PrimitiveKind::Diode) && fits {
        mapped(SimModel { pins, ..SimModel::new(SimDevice::Primitive { kind, model: card }) }, tag)
    } else {
        let names: Vec<String> = ports[..pins.len()].iter().map(|p| p.to_ascii_lowercase()).collect();
        let n = names.join(" ");
        let letter = if device.ends_with("MES") { 'Z' } else { kind.letter() };
        let text = format!(".subckt {tag}_{} {n}\n{letter}1 {n} {}\n{card}\n.ends", pins.len(), parsed.name);
        mapped(SimModel { pins, ..SimModel::new(SimDevice::Subckt { text }) }, tag)
    };
    if params {
        m.sim.params = f.params.iter().filter(|(k, _)| !k.is_empty()).map(|(k, v)| (k.clone(), to_spice(v))).collect();
    }
    Ok(m)
}

/// A primitive with its parameters in `Sim.Params` (no library).
fn builtin_primitive(f: &Fields, sym: &KiSymbol) -> Result<Mapped, String> {
    let (_, _, _, card_kind) = primitive_info(&f.device);
    let card_kind = if f.kind == "VDMOS" { "VDMOS" } else { card_kind };
    let mut params: Vec<String> = f
        .params
        .iter()
        .filter(|(k, _)| !k.is_empty())
        .map(|(k, v)| if v.is_empty() { k.clone() } else { format!("{k}={}", to_spice(v)) })
        .collect();
    if f.kind == "VDMOS" && f.device == "PMOS" && !params.iter().any(|p| p.eq_ignore_ascii_case("pchan")) {
        params.push("pchan".into());
    }
    if let Some(l) = level(&f.kind)? {
        params.push(format!("level={l}"));
    }
    let name = format!("{}_{}", f.device, if f.kind.is_empty() { "default" } else { f.kind.as_str() });
    let card = format!(".model {name} {card_kind}({})", params.join(" "));
    let mut m = primitive(&f.device, card, f, sym, false)?;
    m.tag = if params.is_empty() { String::new() } else { params.join(" ") };
    Ok(m)
}

/// A model from `Sim.Library` + `Sim.Name`: a `.model` card for a primitive, else a
/// `.subckt`.
fn library_model(f: &Fields, sym: &KiSymbol, libs: &mut Libs) -> Result<Mapped, String> {
    let text = libs.text(&f.library)?;
    let items = LibItems::parse(&text);
    let (subckt, def) =
        items.with_dependencies(&f.name).ok_or_else(|| format!("{} has no model {}", f.library, f.name))?;
    if !subckt {
        let card = ModelCard::parse(&def).ok_or_else(|| format!("{}: unreadable model {}", f.library, f.name))?;
        let device = match f.device.as_str() {
            "" | "SUBCKT" => match card.kind.to_ascii_uppercase().as_str() {
                "D" => "D",
                "NPN" => "NPN",
                "PNP" => "PNP",
                "NJF" => "NJFET",
                "PJF" => "PJFET",
                "NMOS" => "NMOS",
                "PMOS" => "PMOS",
                "VDMOS" if card.params.iter().any(|(k, _)| k.eq_ignore_ascii_case("pchan")) => "PMOS",
                "VDMOS" => "NMOS",
                other => return Err(format!("{}: {} is a {other} model, which isn't supported", f.library, f.name)),
            },
            d => d,
        };
        let card_text = def.lines().take_while(|l| !l.trim().is_empty()).collect::<Vec<_>>().join("\n");
        let card_text = logical_lines(&card_text).into_iter().next().unwrap_or(card_text);
        return primitive(device, card_text, f, sym, true);
    }
    let header = SubcktHeader::parse(&def).ok_or_else(|| format!("{}: unreadable subcircuit {}", f.library, f.name))?;
    let ports: Vec<&str> = header.ports.iter().map(String::as_str).collect();
    let pins = map_pins(f, &ports, 0, sym)?;
    let mut m = mapped(SimModel { pins, ..SimModel::new(SimDevice::Subckt { text: def }) }, header.name);
    m.sim.params = f.params.iter().filter(|(k, _)| !k.is_empty()).map(|(k, v)| (k.clone(), to_spice(v))).collect();
    Ok(m)
}

/// KiCad's XSPICE code model (`Sim.Device XSPICE`, the `.model` card from
/// `Sim.Library`): KiCad writes `A${REF} node… NAME`, a node per pin; here a subcircuit
/// around that line with the card. Vector ports of the code models whose port layout is
/// known get their brackets (ngspice needs them; [`xspice_nodes`]). Digital ports need
/// bridges to analog nets, placed by the user as in KiCad. `Sim.Params` override the
/// card's parameters.
fn xspice(f: &Fields, sym: &KiSymbol, libs: &mut Libs) -> Result<Mapped, String> {
    if f.library.is_empty() {
        return Err("an XSPICE code model without its library (Sim.Library)".into());
    }
    let text = libs.text(&f.library)?;
    let items = LibItems::parse(&text);
    let (subckt, def) =
        items.with_dependencies(&f.name).ok_or_else(|| format!("{} has no model {}", f.library, f.name))?;
    if subckt {
        return library_model(f, sym, libs);
    }
    let line = logical_lines(&def).into_iter().next().unwrap_or_default();
    let card = ModelCard::parse(&line).ok_or_else(|| format!("{}: unreadable model {}", f.library, f.name))?;
    let overrides: Vec<(String, String)> =
        f.params.iter().filter(|(k, _)| !k.is_empty()).map(|(k, v)| (k.clone(), to_spice(v))).collect();
    // Rewritten only to override: vector values (`[0.5 1]`) don't survive the round trip.
    let card_text = if overrides.is_empty() { line } else { card.clone().with(&overrides).text() };
    let count = if f.pins.is_empty() { symbol_pins(sym).len() } else { f.pins.len() };
    let numbers: Vec<String> = (1..=count).map(|k| k.to_string()).collect();
    let numbers: Vec<&str> = numbers.iter().map(String::as_str).collect();
    let pins = map_pins(f, &numbers, 0, sym)?;
    let ports: Vec<String> = (1..=pins.len()).map(|k| format!("p{k}")).collect();
    let name = &card.name;
    let text = format!(
        ".subckt {name}_A {}\nA1 {} {name}\n.ends\n{card_text}",
        ports.join(" "),
        xspice_nodes(&card.kind, &ports)
    );
    Ok(mapped(SimModel { pins, ..SimModel::new(SimDevice::Subckt { text }) }, name.clone()))
}

/// The nodes of an XSPICE instance: gates take their inputs as one vector (`[a b] y`),
/// bridges their inputs and outputs (`[a b] [x y]`, half each); others a node per port.
fn xspice_nodes(kind: &str, nodes: &[String]) -> String {
    let n = nodes.len();
    match kind.to_ascii_lowercase().as_str() {
        "d_and" | "d_nand" | "d_or" | "d_nor" | "d_xor" | "d_xnor" if n >= 2 => {
            format!("[{}] {}", nodes[..n - 1].join(" "), nodes[n - 1])
        }
        "adc_bridge" | "dac_bridge" if n >= 2 && n.is_multiple_of(2) => {
            format!("[{}] [{}]", nodes[..n / 2].join(" "), nodes[n / 2..].join(" "))
        }
        _ => nodes.join(" "),
    }
}

/// Controlled sources: E, G by a voltage (4 pins), F, H by a source's current.
fn controlled(f: &Fields, sym: &KiSymbol) -> Result<Mapped, String> {
    let gain = f.param("gain").map_or("1".to_string(), to_spice);
    let letter = f.device.as_str();
    let (pins, rest, nodes) = if letter == "E" || letter == "G" {
        (map_pins(f, &["+", "-", "C+", "C-"], 0, sym)?, gain, 4)
    } else {
        let control = f.param("control").ok_or("a current-controlled source without its control source")?;
        (map_pins(f, &["+", "-"], 0, sym)?, format!("{control} {gain}"), 2)
    };
    let model = SimModel { pins, ..SimModel::new(spice_line(letter, nodes, &rest)) };
    Ok(mapped(model, rest))
}

/// KiCad's raw SPICE device: `type` (the letter), `model` (the rest of the line), `lib`
/// (a file it needs; its definition is embedded, or its text added to the setup).
fn raw_spice(f: &Fields, sym: &KiSymbol, libs: &mut Libs) -> Result<Mapped, String> {
    let letter = f.param("type").unwrap_or("").to_ascii_uppercase();
    let model = f.param("model").or_else(|| f.param("")).unwrap_or("").trim().to_string();
    if letter.is_empty() {
        return Err("a SPICE model without its device letter (type)".into());
    }
    let lib = f.param("lib").unwrap_or("").trim();
    let count = if f.pins.is_empty() { symbol_pins(sym).len() } else { f.pins.len() };
    let ports: Vec<String> = (1..=count).map(|k| k.to_string()).collect();
    let ports: Vec<&str> = ports.iter().map(String::as_str).collect();
    let pins = map_pins(f, &ports, 0, sym)?;
    if !lib.is_empty() {
        let text = libs.text(lib)?;
        let items = LibItems::parse(&text);
        let name = model.split_whitespace().next().unwrap_or("");
        match items.with_dependencies(name) {
            Some((true, def)) if letter == "X" => {
                let mut m = mapped(SimModel { pins, ..SimModel::new(SimDevice::Subckt { text: def }) }, name);
                m.sim.params = parse_params(model.split_once(char::is_whitespace).map_or("", |x| x.1))
                    .into_iter()
                    .filter(|(k, _)| !k.is_empty())
                    .collect();
                return Ok(m);
            }
            Some((false, def)) => {
                let card = logical_lines(&def).into_iter().next().unwrap_or_default();
                let fits = ModelCard::parse(&card)
                    .and_then(|c| PrimitiveKind::of_card(&c))
                    .filter(|k| k.letter().to_string() == letter);
                if let Some(kind) = fits
                    && pins.len() == kind.ports().len()
                {
                    let m = SimModel { pins, ..SimModel::new(SimDevice::Primitive { kind, model: card }) };
                    return Ok(mapped(m, name));
                }
            }
            _ => {}
        }
        return Err(format!("the SPICE line needs {lib}, which isn't embedded"));
    }
    let n = pins.len();
    let model = SimModel { pins, ..SimModel::new(spice_line(&letter, n, &model)) };
    Ok(mapped(model, f.param("model").unwrap_or("").to_string()))
}

// ---------------------------------------------------------------------------------------
// Directives

/// Is this schematic text a SPICE directive (KiCad: a line starting with a known dot
/// command)?
pub fn is_directive(text: &str) -> bool {
    const DIRECTIVES: [&str; 37] = [
        ".ac", ".control", ".csparam", ".disto", ".dc", ".else", ".elseif", ".end", ".endc", ".endif", ".ends",
        ".four", ".func", ".global", ".ic", ".if", ".include", ".lib", ".meas", ".model", ".nodeset", ".noise", ".op",
        ".options", ".param", ".plot", ".print", ".probe", ".pz", ".save", ".sens", ".sp", ".subckt", ".temp", ".tf",
        ".title", ".tran",
    ];
    text.lines().any(|l| {
        let l = l.trim().to_ascii_lowercase();
        DIRECTIVES
            .iter()
            .any(|d| l == *d || l.starts_with(&format!("{d} ")) || (*d == ".options" && l.starts_with(".option ")))
    })
}

/// What the directives of a schematic set up.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Setup {
    pub analyses: Vec<Analysis>,
    /// Lines for the setup's extra SPICE (`.param`, `.options`, models, included files).
    pub extra: String,
    pub temperature: Option<f64>,
}

/// The directives of `texts` (each a schematic text that [`is_directive`]).
/// `source`: the component a `.dc` sweeps, by its SPICE name.
pub fn directives(
    texts: &[String],
    source: &dyn Fn(&str) -> Option<Id>,
    libs: &mut Libs,
    report: &mut Report,
) -> Setup {
    let mut out = Setup::default();
    let mut extra: Vec<String> = Vec::new();
    let add_analysis = |out: &mut Setup, kind: AnalysisKind| {
        let base = kind.label().to_string();
        let mut name = base.clone();
        let mut k = 2;
        while out.analyses.iter().any(|a| a.name == name) {
            name = format!("{base} {k}");
            k += 1;
        }
        out.analyses.push(Analysis::new(name, kind));
    };
    for text in texts {
        // Continuation lines joined, so a directive is one line.
        let mut lines: Vec<String> = Vec::new();
        for l in text.lines() {
            let t = l.trim();
            if let Some(rest) = t.strip_prefix('+')
                && let Some(last) = lines.last_mut()
            {
                last.push(' ');
                last.push_str(rest.trim());
            } else {
                lines.push(t.to_string());
            }
        }
        let mut control = false;
        let mut subckt = 0;
        for line in lines {
            let lower = line.to_ascii_lowercase();
            let word = lower.split_whitespace().next().unwrap_or("").to_string();
            let args: Vec<&str> = line.split_whitespace().skip(1).collect();
            if control {
                control = word != ".endc";
                continue;
            }
            if subckt > 0 {
                if word == ".subckt" {
                    subckt += 1;
                } else if word == ".ends" {
                    subckt -= 1;
                }
                extra.push(line);
                continue;
            }
            match word.as_str() {
                "" => {}
                w if w.starts_with('*') => {}
                ".control" => {
                    control = true;
                    report.skip("ngspice control scripts (.control … .endc)");
                }
                ".end" | ".title" | ".probe" | ".save" | ".print" | ".plot" | ".width" => {}
                ".op" => add_analysis(&mut out, AnalysisKind::Op),
                ".tran" => match tran(&args) {
                    Some(k) => add_analysis(&mut out, k),
                    None => report.warn(format!("Simulation: can't read the directive {line:?}")),
                },
                ".ac" => match ac(&args, report) {
                    Some(k) => add_analysis(&mut out, k),
                    None => report.warn(format!("Simulation: can't read the directive {line:?}")),
                },
                ".dc" => {
                    let parsed = (args.len() >= 4)
                        .then(|| {
                            Some((
                                source(args[0])?,
                                spice_number(args[1])?,
                                spice_number(args[2])?,
                                spice_number(args[3])?,
                            ))
                        })
                        .flatten();
                    match parsed {
                        Some((id, start, stop, step)) => {
                            if args.len() > 4 {
                                report.note("a nested DC sweep was imported as its first sweep");
                            }
                            add_analysis(&mut out, AnalysisKind::Dc { source: id, start, stop, step })
                        }
                        None => report
                            .warn(format!("Simulation: can't read the directive {line:?} (only sources are swept)")),
                    }
                }
                ".temp" => out.temperature = args.first().and_then(|t| spice_number(t)),
                ".noise" | ".tf" | ".pz" | ".sens" | ".disto" | ".four" | ".sp" | ".fft" => {
                    report.skip(&format!("{word} analyses"))
                }
                ".include" | ".inc" => {
                    let file = args.join(" ");
                    match libs.text(&file) {
                        Ok(t) => {
                            extra.push(format!("* {}", file.trim_matches(['"', '\''])));
                            extra.push(t.trim_end().to_string());
                        }
                        Err(e) => report.warn(format!("Simulation: can't include {e}")),
                    }
                }
                ".lib" => {
                    let file = args.first().copied().unwrap_or("");
                    match libs.text(file) {
                        Ok(t) => {
                            let section = args.get(1).map(|s| s.to_ascii_lowercase());
                            let body = match &section {
                                Some(s) => lib_section(&t, s),
                                None => Some(t.clone()),
                            };
                            match body {
                                Some(b) => {
                                    extra.push(format!("* {}", file.trim_matches(['"', '\''])));
                                    extra.push(b.trim_end().to_string());
                                }
                                None => report.warn(format!("Simulation: {file} has no section {}", args[1])),
                            }
                        }
                        Err(e) => report.warn(format!("Simulation: can't include {e}")),
                    }
                }
                ".subckt" => {
                    subckt = 1;
                    extra.push(line);
                }
                _ => extra.push(line),
            }
        }
    }
    out.extra = extra.join("\n");
    out
}

/// `.lib file section`: the lines between `.lib section` and `.endl`.
fn lib_section(text: &str, section: &str) -> Option<String> {
    let mut out = Vec::new();
    let mut inside = false;
    for l in text.lines() {
        let lower = l.trim().to_ascii_lowercase();
        if !inside {
            let mut w = lower.split_whitespace();
            inside = w.next() == Some(".lib") && w.next() == Some(section) && w.next().is_none();
        } else if lower.starts_with(".endl") {
            return Some(out.join("\n"));
        } else {
            out.push(l);
        }
    }
    None
}

/// `.tran tstep tstop [tstart [tmax]] [uic]`.
fn tran(args: &[&str]) -> Option<AnalysisKind> {
    let nums: Vec<f64> =
        args.iter().filter(|a| !a.eq_ignore_ascii_case("uic")).map(|a| spice_number(a)).collect::<Option<_>>()?;
    let (step, stop) = (*nums.first()?, *nums.get(1)?);
    Some(AnalysisKind::Tran {
        stop,
        step: Some(step),
        start: nums.get(2).copied().unwrap_or(0.0),
        max_step: nums.get(3).copied(),
    })
}

/// `.ac dec|oct|lin n fstart fstop`: octaves and linear points as points per decade.
fn ac(args: &[&str], report: &mut Report) -> Option<AnalysisKind> {
    let n = spice_number(args.get(1)?)?;
    let (start, stop) = (spice_number(args.get(2)?)?, spice_number(args.get(3)?)?);
    let ppd = match args.first()?.to_ascii_lowercase().as_str() {
        "dec" => n,
        "oct" => n * 10f64.log2(),
        "lin" => {
            report.note("linear AC sweeps were imported as logarithmic ones");
            n / (stop / start).log10().max(1.0)
        }
        _ => return None,
    };
    Some(AnalysisKind::Ac { start, stop, points_per_decade: ppd.round().max(1.0) as u32 })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn params() {
        assert_eq!(
            parse_params("dc=0 ampl=1 f=1k ac=1"),
            [("dc", "0"), ("ampl", "1"), ("f", "1k"), ("ac", "1")].map(|(a, b)| (a.to_string(), b.to_string()))
        );
        assert_eq!(
            parse_params("freq = 317k type=\"V\" model=\"sin(0 1 1k)\" pwl=\"0 1 1m 2\""),
            [("freq", "317k"), ("type", "V"), ("model", "sin(0 1 1k)"), ("pwl", "0 1 1m 2")]
                .map(|(a, b)| (a.to_string(), b.to_string()))
        );
        assert_eq!(parse_params("1k"), [(String::new(), "1k".to_string())]);
        assert_eq!(parse_params("r='TIME > 1m ? 8 : 9'"), [("r".to_string(), "'TIME > 1m ? 8 : 9'".to_string())]);
    }

    #[test]
    fn pins() {
        assert_eq!(
            parse_pins("1=C 1=C 2=B 2=B"),
            [("1", "C"), ("2", "B")].map(|(a, b)| (a.to_string(), b.to_string()))
        );
    }

    #[test]
    fn numbers() {
        assert_eq!(spice_number("1Meg"), Some(1e6));
        assert_eq!(spice_number("10m"), Some(10e-3));
        assert_eq!(spice_number("10M"), Some(10e-3));
        assert_eq!(spice_number("1e-3"), Some(1e-3));
        assert_eq!(spice_number("100nF"), Some(100e-9));
        assert_eq!(spice_number("-2.5"), Some(-2.5));
        assert_eq!(spice_number("{x}"), None);
        assert_eq!(si_number("10M"), Some(10e6));
        assert_eq!(si_number("4k7"), Some(4700.0));
        let near = |v: Option<f64>, want: f64| v.is_some_and(|v| (v - want).abs() < want * 1e-12);
        assert!(near(si_number("100uF;16V"), 100e-6));
        assert!(near(si_number("1UF"), 1e-6));
    }

    #[test]
    fn legacy_specs() {
        let Ok(Wave::Source(s)) = legacy_source("sin(0 1 1k)") else { panic!() };
        assert_eq!(
            s.waveform,
            Waveform::Sine { offset: 0.0, amplitude: 1.0, frequency: 1e3, delay: 0.0, damping: 0.0, phase: 0.0 }
        );
        let Ok(Wave::Source(s)) = legacy_source("dc(1)") else { panic!() };
        assert_eq!(s.waveform, Waveform::Dc { value: 1.0 });
        let Ok(Wave::Source(s)) = legacy_source("DC 5 AC 1") else { panic!() };
        assert_eq!((s.waveform, s.ac), (Waveform::Dc { value: 5.0 }, 1.0));
        let Ok(Wave::Line(l)) = legacy_source("am(1 0 100k 1k 1n)") else { panic!() };
        assert_eq!(l, "am(1 0 100k 1k 1n)");
    }

    #[test]
    fn directive_texts() {
        assert!(is_directive(".tran 1u 10m\n"));
        assert!(is_directive(".options abstol=1u\n.control\nrun\n.endc"));
        assert!(!is_directive("*.ac dec 10 1 1Meg"));
        assert!(!is_directive("Note: .tran is set below"));
        let mut report = Report::default();
        let mut libs = Libs::new(Path::new("."));
        let v1 = Id::new();
        let source = |n: &str| (n.eq_ignore_ascii_case("V1")).then_some(v1);
        let texts = vec![
            ".tran 1u 10m\n.ac dec 10 1 1Meg".to_string(),
            ".options abstol=1u\n.control\nrun\n.endc\n.temp 50\n.dc V1 0 5 0.1\n.param r=1k\n+ c=1n".to_string(),
        ];
        let s = directives(&texts, &source, &mut libs, &mut report);
        let kinds: Vec<&AnalysisKind> = s.analyses.iter().map(|a| &a.kind).collect();
        assert_eq!(
            kinds,
            [
                &AnalysisKind::Tran { stop: 10e-3, step: Some(1e-6), start: 0.0, max_step: None },
                &AnalysisKind::Ac { start: 1.0, stop: 1e6, points_per_decade: 10 },
                &AnalysisKind::Dc { source: v1, start: 0.0, stop: 5.0, step: 0.1 },
            ]
        );
        assert_eq!(s.extra, ".options abstol=1u\n.param r=1k c=1n");
        assert_eq!(s.temperature, Some(50.0));
        assert!(report.lines().join("\n").contains(".control"), "{:?}", report.lines());
    }

    #[test]
    fn library_dependencies() {
        let lib = "* lib\n.subckt AMP in out\nQ1 out in 0 QX\nX1 in out BUF\n.ends AMP\n.model QX NPN(BF=100)\n.subckt BUF a b\nR1 a b 1k\n.ends\n.model UNUSED D()\n";
        let items = LibItems::parse(lib);
        let (subckt, text) = items.with_dependencies("amp").unwrap();
        assert!(subckt);
        assert!(text.starts_with(".subckt AMP"), "{text}");
        assert!(text.contains(".model QX NPN(BF=100)") && text.contains(".subckt BUF"), "{text}");
        assert!(!text.contains("UNUSED"), "{text}");
    }
}
