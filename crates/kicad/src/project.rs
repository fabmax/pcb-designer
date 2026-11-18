//! KiCad projects (`.kicad_pro` with its `.kicad_sch` and `.kicad_pcb`, KiCad 6–10) →
//! a [`Project`].
//!
//! Parts come from the schematic's embedded symbols and the footprints on the board:
//! one part per symbol, one variant per distinct board footprint used with it (board
//! footprints that differ from others of the same name become `Name (2)`), a "No
//! footprint" variant for components that aren't on the board. Footprints without a
//! symbol become board parts. KiCad UUIDs are kept as ids. What can't be represented is
//! skipped and counted in the [`Report`].

use std::collections::{BTreeMap, HashMap};
use std::path::Path;

use model::*;

use crate::common::kid;
use crate::import::{connect, make_part};
use crate::pcb::{self, PcbImport};
use crate::sch::{self, SchImport};

/// What the import did not take over, for the user.
#[derive(Clone, Debug, Default)]
pub struct Report {
    /// Things to look at (not annotated symbols, footprints missing from the board…).
    pub warnings: Vec<String>,
    /// Conversions the user should know about, once each.
    pub notes: Vec<String>,
    /// Skipped features and how often they occurred.
    pub skipped: BTreeMap<String, usize>,
}

impl Report {
    pub fn skip(&mut self, what: &str) {
        *self.skipped.entry(what.to_string()).or_default() += 1;
    }

    pub fn warn(&mut self, w: impl Into<String>) {
        self.warnings.push(w.into());
    }

    pub fn note(&mut self, n: &str) {
        if !self.notes.iter().any(|x| x == n) {
            self.notes.push(n.to_string());
        }
    }

    /// Everything as lines of text.
    pub fn lines(&self) -> Vec<String> {
        let mut out: Vec<String> = self.warnings.clone();
        out.extend(self.notes.iter().cloned());
        out.extend(self.skipped.iter().map(|(what, n)| format!("Skipped: {what} ({n}×)")));
        out
    }

    pub fn is_empty(&self) -> bool {
        self.warnings.is_empty() && self.notes.is_empty() && self.skipped.is_empty()
    }
}

/// An imported project (not saved yet) and the report.
#[derive(Clone, Debug)]
pub struct ProjectImport {
    pub project: Project,
    pub report: Report,
}

/// Import the KiCad project `pro` (a `.kicad_pro`; its schematic and board are the
/// `.kicad_sch` and `.kicad_pcb` of the same name next to it).
pub fn import_project(pro: &Path) -> Result<ProjectImport, String> {
    let dir = pro.parent().unwrap_or(Path::new("."));
    let stem = pro.file_stem().and_then(|s| s.to_str()).ok_or("not a KiCad project file")?.to_string();
    let sch_path = dir.join(format!("{stem}.kicad_sch"));
    let pcb_path = dir.join(format!("{stem}.kicad_pcb"));
    if !sch_path.exists() {
        if dir.join(format!("{stem}.sch")).exists() {
            return Err(format!("{stem} is a KiCad 5 (legacy) project; open it in KiCad 6 or newer and save it first"));
        }
        return Err(format!("{} not found", sch_path.display()));
    }
    let mut report = Report::default();
    let settings: serde_json::Value = std::fs::read_to_string(pro)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or(serde_json::Value::Null);

    let sch = sch::import(&sch_path, &mut report)?;
    if !sch.refused.is_empty() {
        return Err(format!(
            "these symbols can't be imported:\n{}\nReplace them in KiCad with symbols whose power pins are visible.",
            sch.refused.join("\n")
        ));
    }
    let pcb = if pcb_path.exists() { pcb::import(&pcb_path, &mut report)? } else { PcbImport::default() };
    let mut project = assemble(&stem, sch, pcb, &mut report);
    design_rules(&settings, &mut project.board.rules, &mut report);
    check_settings(&settings, &mut project, &mut report);
    let issues = project.validate();
    if !issues.is_empty() {
        return Err(format!("the imported project is inconsistent (a bug):\n{}", issues.join("\n")));
    }
    Ok(ProjectImport { project, report })
}

fn assemble(name: &str, mut sch: SchImport, pcb: PcbImport, report: &mut Report) -> Project {
    let mut p = Project::new(name);
    p.schematic.sheets = Store::default();
    for s in std::mem::take(&mut sch.sheets) {
        p.schematic.sheets.insert(s);
    }

    // Footprint definitions, distinct by content.
    let mut defs: Vec<(String, String, Footprint)> = Vec::new(); // (content key, lib id, def)
    let mut def_of: Vec<Id> = Vec::new(); // per board footprint
    for f in &pcb.footprints {
        let key = content_key(&f.def);
        let id = match defs.iter().find(|(k, l, _)| *k == key && *l == f.lib_id) {
            Some((_, _, d)) => d.id,
            None => {
                let same_name = defs.iter().filter(|(_, l, _)| *l == f.lib_id).count();
                let mut def = f.def.clone();
                def.id = kid(&["kicad-board-footprint", &f.lib_id, &same_name.to_string()]);
                if same_name > 0 {
                    def.name = format!("{} ({})", def.name, same_name + 1);
                    report.note("footprints edited on the KiCad board became footprints of their own (\"Name (2)\")");
                }
                let id = def.id;
                defs.push((key, f.lib_id.clone(), def));
                id
            }
        };
        def_of.push(id);
    }

    // Board footprints by reference.
    let mut fp_by_ref: HashMap<&str, usize> = HashMap::new();
    for (i, f) in pcb.footprints.iter().enumerate() {
        if !f.refdes.is_empty() && fp_by_ref.insert(f.refdes.as_str(), i).is_some() {
            report
                .warn(format!("{}: several footprints with this reference; the later ones are board parts", f.refdes));
        }
    }

    // Parts and components of the schematic.
    let mut parts: BTreeMap<String, Part> = BTreeMap::new();
    let mut used_fps: Vec<bool> = vec![false; pcb.footprints.len()];
    let mut comp_of_fp: Vec<Option<Id>> = vec![None; pcb.footprints.len()];
    for c in &sch.components {
        let Some(sym) = sch.symbols.get(&c.lib_id) else { continue };
        let part = parts.entry(c.lib_id.clone()).or_insert_with(|| {
            let mut part = make_part(sym, &[]);
            part.id = kid(&["kicad-part", &c.lib_id]);
            part.variants.clear();
            part
        });
        let board_fp = fp_by_ref.get(c.refdes.as_str()).copied().filter(|&i| !used_fps[i] && c.on_board);
        let variant = match board_fp {
            Some(i) => {
                used_fps[i] = true;
                comp_of_fp[i] = Some(c.id);
                let def_id = def_of[i];
                let def = &defs.iter().find(|(_, _, d)| d.id == def_id).unwrap().2;
                let vid = kid(&["kicad-variant", &c.lib_id, &def_id.to_string()]);
                if !part.variants.iter().any(|v| v.id == vid) {
                    part.variants.push(PartVariant {
                        id: vid,
                        name: def.name.clone(),
                        footprint: Some(def_id),
                        connections: connect(sym, def),
                    });
                }
                vid
            }
            None => {
                if c.on_board {
                    report.warn(format!("{}: not on the board; imported without footprint", c.refdes));
                }
                let vid = kid(&["kicad-variant", &c.lib_id, ""]);
                if !part.variants.iter().any(|v| v.id == vid) {
                    part.variants.push(PartVariant::no_footprint(vid));
                }
                vid
            }
        };
        p.schematic.components.insert(Component {
            id: c.id,
            part: part.id,
            variant,
            refdes: c.refdes.clone(),
            value: c.value.clone(),
            properties: c.properties.clone(),
        });
    }
    for sym in sch.symbols.values() {
        for u in &sym.units {
            if !p.library.symbols.contains(u.id) {
                p.library.symbols.insert(u.clone());
            }
        }
    }

    // Footprints without a symbol: board parts.
    let mut refs: std::collections::HashSet<String> = p.schematic.components.iter().map(|c| c.refdes.clone()).collect();
    for (i, f) in pcb.footprints.iter().enumerate() {
        if used_fps[i] {
            continue;
        }
        let def_id = def_of[i];
        let def = &defs.iter().find(|(_, _, d)| d.id == def_id).unwrap().2;
        let part_id = kid(&["kicad-board-part", &def_id.to_string()]);
        let vid = kid(&["kicad-variant", "board", &def_id.to_string()]);
        let prefix: String = f.refdes.chars().take_while(|c| c.is_ascii_alphabetic()).collect();
        let prefix = if prefix.is_empty() { "H".to_string() } else { prefix };
        parts.entry(format!("board:{def_id}")).or_insert_with(|| Part {
            id: part_id,
            name: def.name.clone(),
            description: String::new(),
            prefix: prefix.clone(),
            default_value: f.value.clone(),
            units: Vec::new(),
            variants: vec![PartVariant {
                id: vid,
                name: def.name.clone(),
                footprint: Some(def_id),
                connections: Vec::new(),
            }],
            properties: BTreeMap::new(),
            source: None,
        });
        let mut refdes = f.refdes.clone();
        if refdes.is_empty() || refdes.contains('*') || refs.contains(&refdes) {
            let mut n = 1;
            while refs.contains(&format!("{prefix}{n}")) {
                n += 1;
            }
            let new = format!("{prefix}{n}");
            if !f.refdes.is_empty() && !f.refdes.contains('*') {
                report.warn(format!("{}: reference used twice; the board-only footprint is {new}", f.refdes));
            }
            refdes = new;
        }
        refs.insert(refdes.clone());
        let id = kid(&["kicad-component", "board", &f.instance.id.to_string()]);
        comp_of_fp[i] = Some(id);
        p.schematic.components.insert(Component {
            id,
            part: part_id,
            variant: vid,
            refdes,
            value: f.value.clone(),
            properties: BTreeMap::new(),
        });
    }

    for (_, _, def) in defs {
        p.library.footprints.insert(def);
    }
    for (_, part) in parts {
        p.library.parts.insert(part);
    }

    // The schematic.
    for i in sch.instances {
        p.schematic.symbols.insert(i);
    }
    for f in sch.fields {
        p.schematic.fields.insert(f);
    }
    for w in sch.wires {
        p.schematic.wires.insert(w);
    }
    for w in sch.bus_wires {
        p.schematic.bus_wires.insert(w);
    }
    for e in sch.bus_entries {
        p.schematic.bus_entries.insert(e);
    }
    for j in sch.junctions {
        p.schematic.junctions.insert(j);
    }
    for n in sch.no_connects {
        p.schematic.no_connects.insert(n);
    }
    for f in sch.power_flags {
        p.schematic.power_flags.insert(f);
    }
    for l in sch.labels {
        p.schematic.labels.insert(l);
    }
    for t in sch.texts {
        p.schematic.texts.insert(t);
    }
    reattach_sheet_pins(&mut p, &sch.kicad_sheet_pins, report);

    // The board.
    let rules = p.board.rules.clone();
    p.board = Board::with_copper_layers(pcb.copper_layers.max(2));
    p.board.rules = rules;
    if let Some(m) = pcb.mask_expansion {
        p.board.rules.mask_expansion = m;
    }
    if let Some(m) = pcb.paste_margin {
        p.board.rules.paste_margin = m;
    }
    let mut kicad_pad_nets: Vec<(Id, String, String)> = Vec::new(); // (component, pad, net)
    for (i, f) in pcb.footprints.into_iter().enumerate() {
        let Some(component) = comp_of_fp[i] else { continue };
        let mut inst = f.instance;
        inst.component = component;
        let def = p.library.footprints.get(def_of[i]).expect("inserted above");
        // The instance's texts, in the order of the definition's templates.
        let mut fields = f.fields;
        for t in def.texts.iter().filter(|t| t.is_field()) {
            if !fields.iter().any(|x| x.text.text == t.text.text) {
                let mut x =
                    FootprintField::for_footprint(inst.id, def).into_iter().find(|x| x.text.text == t.text.text);
                if let Some(x) = x.take() {
                    fields.push(x);
                }
            }
        }
        for x in fields {
            p.board.fields.insert(x);
        }
        kicad_pad_nets.extend(f.pad_nets.into_iter().map(|(pad, net)| (component, pad, net)));
        p.board.footprints.insert(inst);
    }
    for t in pcb.tracks {
        p.board.tracks.insert(t);
    }
    for v in pcb.vias {
        p.board.vias.insert(v);
    }
    for z in pcb.zones {
        p.board.zones.insert(z);
    }
    for k in pcb.keepouts {
        p.board.keepouts.insert(k);
    }
    for g in pcb.graphics {
        p.board.graphics.insert(g);
    }
    for t in pcb.texts {
        p.board.texts.insert(t);
    }
    resolve_zone_nets(&mut p, &kicad_pad_nets, report);
    p
}

/// What makes two footprint definitions the same: pads, graphics (in any order) and
/// texts other than the reference and value (those are per footprint), turned by up to
/// 180° (KiCad keeps texts readable).
fn content_key(def: &Footprint) -> String {
    let mut graphics: Vec<String> = def.graphics.iter().map(|g| serde_json::to_string(g).unwrap()).collect();
    graphics.sort();
    let mut texts: Vec<String> = def
        .texts
        .iter()
        .filter(|t| !t.is_field())
        .map(|t| {
            let mut t = t.clone();
            t.text.rotation = Angle::deg(t.text.rotation.degrees().rem_euclid(180.0));
            serde_json::to_string(&t).unwrap()
        })
        .collect();
    texts.sort();
    let pads: Vec<String> = def.pads.iter().map(|p| serde_json::to_string(p).unwrap()).collect();
    format!("{pads:?}{graphics:?}{texts:?}")
}

/// Our sheet pins are laid out by name, KiCad's are where the user put them: wire ends
/// at KiCad's pins move to ours.
fn reattach_sheet_pins(p: &mut Project, kicad: &[(Id, String, Point)], report: &mut Report) {
    let s = &p.schematic;
    let mut ports: HashMap<Id, Vec<(&str, LabelShape)>> = HashMap::new();
    for l in s.labels.iter().filter(|l| l.kind == LabelKind::Hierarchical) {
        let v = ports.entry(l.sheet).or_default();
        if !v.iter().any(|x| x.0 == l.name) {
            v.push((l.name.as_str(), l.shape));
        }
    }
    let mut moves: Vec<(Id, Point, Point)> = Vec::new(); // (parent sheet, from, to)
    for (sheet_id, name, at) in kicad {
        let Some(sheet) = s.sheets.get(*sheet_id) else { continue };
        let Some(parent) = sheet.parent else { continue };
        let (_, pins) = sheet.layout(ports.get(sheet_id).map_or(&[][..], Vec::as_slice));
        match pins.iter().find(|pin| pin.name == *name) {
            Some(pin) if pin.pos != *at => moves.push((parent, *at, pin.pos)),
            Some(_) => {}
            None => {
                report.warn(format!("sheet {}: pin {name} has no hierarchical label in the sheet; dropped", sheet.name))
            }
        }
    }
    let mut moved = 0;
    let ids: Vec<Id> = p.schematic.wires.ids().collect();
    for id in ids {
        let w = p.schematic.wires.get(id).unwrap().clone();
        let mut nw = w.clone();
        for &(sheet, from, to) in &moves {
            if w.sheet == sheet {
                if w.a == from {
                    nw.a = to;
                }
                if w.b == from {
                    nw.b = to;
                }
            }
        }
        if nw != w {
            moved += 1;
            p.schematic.wires.replace(nw);
        }
    }
    if moved > 0 {
        report.note("sheet pins are laid out by name here: wires to them were stretched to the new positions");
    }
}

/// Zones bind to nets by name, and our generated net names differ from KiCad's
/// (`Net-(R1-2)` vs `Net-(R1-Pad2)`): a zone whose net isn't one of ours takes the net of
/// the pads that had its net in KiCad.
fn resolve_zone_nets(p: &mut Project, kicad: &[(Id, String, String)], report: &mut Report) {
    if p.board.zones.is_empty() {
        return;
    }
    let conn = editor_core::connectivity::Connectivity::build(p);
    let ours: std::collections::HashSet<&str> = conn.nets.iter().map(|n| n.name.as_str()).collect();
    let refdes: HashMap<Id, &str> = p.schematic.components.iter().map(|c| (c.id, c.refdes.as_str())).collect();
    let mut renames: Vec<(Id, String)> = Vec::new();
    for z in p.board.zones.iter().filter(|z| !z.net.is_empty() && !ours.contains(z.net.as_str())) {
        let found = kicad.iter().filter(|(_, _, net)| *net == z.net).find_map(|(comp, pad, _)| {
            let r = refdes.get(comp)?;
            conn.nets.iter().find(|n| n.pins.iter().any(|np| np.refdes == *r && np.pads.contains(pad)))
        });
        match found {
            Some(n) => renames.push((z.id, n.name.clone())),
            None => report.warn(format!("zone on net {}: no such net here; check its net", z.net)),
        }
    }
    for (id, net) in renames {
        let mut z = p.board.zones.get(id).unwrap().clone();
        z.net = net;
        p.board.zones.replace(z);
    }
}

/// KiCad's rule severity names.
fn severity(v: &serde_json::Value) -> Option<Severity> {
    match v.as_str()? {
        "error" => Some(Severity::Error),
        "warning" => Some(Severity::Warning),
        "ignore" => Some(Severity::Ignore),
        _ => None,
    }
}

/// ERC and DRC severities and the pin conflict matrix from the project file, where we
/// have an equivalent. Exclusions aren't imported (KiCad keys them by its own positions
/// and ids).
fn check_settings(settings: &serde_json::Value, project: &mut Project, report: &mut Report) {
    let erc_rules = &settings["erc"]["rule_severities"];
    let erc = &mut project.schematic.erc;
    for (kicad, rule) in [
        ("pin_not_driven", ErcRule::UndrivenInput),
        ("power_pin_not_driven", ErcRule::PowerInputUnpowered),
        ("pin_not_connected", ErcRule::UnconnectedPin),
        ("no_connect_connected", ErcRule::NoConnectMisplaced),
        ("multiple_net_names", ErcRule::NetNames),
        ("duplicate_reference", ErcRule::DuplicateReference),
        ("hier_label_mismatch", ErcRule::SheetPinUnconnected),
        ("bus_entry_needed", ErcRule::BusMisuse),
    ] {
        if let Some(s) = severity(&erc_rules[kicad]) {
            erc.set_severity(rule, s);
        }
    }
    // KiCad's pin types: input, output, bidirectional, tri-state, passive, free,
    // unspecified, power in, power out, open collector, open emitter, not connected.
    const KICAD: [Option<PinKind>; 12] = [
        Some(PinKind::Input),
        Some(PinKind::Output),
        Some(PinKind::Bidirectional),
        Some(PinKind::TriState),
        Some(PinKind::Passive),
        None,
        None,
        Some(PinKind::PowerIn),
        Some(PinKind::PowerOut),
        Some(PinKind::OpenCollector),
        None,
        Some(PinKind::NotConnected),
    ];
    let pin_to_pin_off = severity(&erc_rules["pin_to_pin"]) == Some(Severity::Ignore);
    if let Some(rows) = settings["erc"]["pin_map"].as_array() {
        for (i, row) in rows.iter().enumerate().take(KICAD.len()) {
            let Some(row) = row.as_array() else { continue };
            for (j, cell) in row.iter().enumerate().take(i + 1) {
                let (Some(a), Some(b)) = (KICAD[i], KICAD.get(j).copied().flatten()) else { continue };
                let s = match cell.as_u64() {
                    Some(1) => Severity::Warning,
                    Some(2) => Severity::Error,
                    _ => Severity::Ignore,
                };
                erc.set_pin_pair(a, b, if pin_to_pin_off { Severity::Ignore } else { s });
            }
        }
    }
    if settings["erc"]["erc_exclusions"].as_array().is_some_and(|e| !e.is_empty()) {
        report.skip("ERC exclusions (exclude the findings again in Messages)");
    }
    let drc_rules = &settings["board"]["design_settings"]["rule_severities"];
    let drc = &mut project.board.drc;
    for (kicad, kind) in [
        ("shorting_items", ViolationKind::Short),
        ("clearance", ViolationKind::Clearance),
        ("items_not_allowed", ViolationKind::Keepout),
        ("copper_edge_clearance", ViolationKind::BoardEdge),
        ("courtyards_overlap", ViolationKind::Courtyard),
        ("track_width", ViolationKind::TrackWidth),
        ("drill_out_of_range", ViolationKind::Drill),
        ("track_dangling", ViolationKind::Unconnected),
        ("unconnected_items", ViolationKind::Unrouted),
    ] {
        if let Some(s) = severity(&drc_rules[kicad]) {
            drc.set_severity(kind, s);
        }
    }
    if settings["board"]["design_settings"]["drc_exclusions"].as_array().is_some_and(|e| !e.is_empty()) {
        report.skip("DRC exclusions (exclude the findings again in Messages)");
    }
}

/// Design rules from the project file: the Default net class and the board's minimums.
fn design_rules(settings: &serde_json::Value, rules: &mut DesignRules, report: &mut Report) {
    let mm = |v: &serde_json::Value| v.as_f64().filter(|&x| x > 0.0).map(crate::common::nm);
    if let Some(classes) = settings["net_settings"]["classes"].as_array() {
        for c in classes {
            if c["name"] == "Default" {
                if let Some(v) = mm(&c["clearance"]) {
                    rules.clearance = v;
                }
                if let Some(v) = mm(&c["track_width"]) {
                    rules.track_width = v;
                }
                if let Some(v) = mm(&c["via_diameter"]) {
                    rules.via_diameter = v;
                }
                if let Some(v) = mm(&c["via_drill"]) {
                    rules.via_drill = v;
                }
            } else if let Some(name) = c["name"].as_str() {
                report.skip(&format!("net class {name} (per-net rules aren't supported; the Default class applies)"));
            }
        }
    }
    let r = &settings["board"]["design_settings"]["rules"];
    if let Some(v) = mm(&r["min_track_width"]) {
        rules.min_track_width = v;
    }
    if let Some(v) = mm(&r["min_through_hole_diameter"]) {
        rules.min_drill = v;
    }
    if rules.track_width < rules.min_track_width {
        rules.track_width = rules.min_track_width;
    }
}
