//! KiCad schematics (`.kicad_sch`, KiCad 6–10) → our schematic.
//!
//! KiCad schematic coordinates are y-down; ours are y-up, so y is negated. Symbol
//! libraries are y-up in both. Each sheet *instance* becomes a sheet here: a sheet file
//! used twice is imported twice (ids of later copies are derived from the instance
//! path). Components are collected here and given their part and variant by the project
//! import once the board's footprints are known.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use model::*;

use crate::common::*;
use crate::project::Report;
use crate::sexpr::{self, Node};
use crate::symbol::KiSymbol;

/// Oldest supported schematic format (KiCad 6.0).
const MIN_VERSION: u64 = 20211014;

/// A component (all units of one reference) found in the schematic.
#[derive(Clone, Debug)]
pub struct SchComponent {
    pub id: Id,
    pub refdes: String,
    /// `Lib:Name` of its symbol.
    pub lib_id: String,
    pub value: String,
    /// `Lib:Name` of its footprint, if set.
    pub footprint: Option<String>,
    /// Not placed on the board (KiCad: "exclude from board").
    pub on_board: bool,
    /// KiCad's "exclude from simulation".
    pub exclude_from_sim: bool,
    /// KiCad's "do not populate".
    pub dnp: bool,
    pub properties: BTreeMap<String, String>,
}

#[derive(Default)]
pub struct SchImport {
    pub sheets: Vec<Sheet>,
    /// Converted symbols of the components (power symbols excluded), by `Lib:Name`.
    pub symbols: BTreeMap<String, KiSymbol>,
    pub components: Vec<SchComponent>,
    /// Symbol instances; their `component` is the [`SchComponent::id`].
    pub instances: Vec<SymbolInstance>,
    /// Reference and value texts of the instances, where KiCad has them.
    pub fields: Vec<SymbolField>,
    pub wires: Vec<Wire>,
    pub bus_wires: Vec<BusWire>,
    pub bus_entries: Vec<BusEntry>,
    pub junctions: Vec<Junction>,
    pub no_connects: Vec<NoConnect>,
    /// From PWR_FLAG symbols.
    pub power_flags: Vec<PowerFlag>,
    pub labels: Vec<NetLabel>,
    pub texts: Vec<SchText>,
    /// Symbols refused because of hidden power pins: `Lib:Name: reason`.
    pub refused: Vec<String>,
    /// Where KiCad has the pins of sub-sheets: (sheet, pin name, position). Our pins
    /// are laid out by name, so wires to these get extended to ours.
    pub kicad_sheet_pins: Vec<(Id, String, Point)>,
}

struct Loader<'a> {
    dir: PathBuf,
    files: HashMap<String, Node>,
    /// How often each sheet file is used (reused files get derived ids).
    uses: HashMap<String, usize>,
    /// Embedded library symbols of all files, by `Lib:Name`.
    lib_symbols: HashMap<String, Node>,
    /// KiCad 6: references by instance path (`/sheet…/symbol`), from the root file.
    legacy_refs: HashMap<String, (String, u32)>,
    out: SchImport,
    by_refdes: HashMap<String, usize>,
    report: &'a mut Report,
}

pub fn import(root: &Path, report: &mut Report) -> Result<SchImport, String> {
    let dir = root.parent().unwrap_or(Path::new(".")).to_path_buf();
    let file = root.file_name().and_then(|f| f.to_str()).unwrap_or_default().to_string();
    let mut l = Loader {
        dir,
        files: HashMap::new(),
        uses: HashMap::new(),
        lib_symbols: HashMap::new(),
        legacy_refs: HashMap::new(),
        out: SchImport::default(),
        by_refdes: HashMap::new(),
        report,
    };
    l.count_uses(&file, 0)?;
    let root_node = l.files[&file].clone();
    if let Some(si) = root_node.child("symbol_instances") {
        for p in si.children("path") {
            let reference = p.child("reference").and_then(|r| r.arg(0)).unwrap_or("").to_string();
            let unit = p.child("unit").and_then(|u| u.num(0)).unwrap_or(1.0) as u32;
            l.legacy_refs.insert(p.arg(0).unwrap_or("").to_string(), (reference, unit));
        }
    }
    let root_uuid = root_node.child("uuid").and_then(|u| u.arg(0)).unwrap_or("").to_string();
    let root_sheet = Sheet {
        id: parse_id(&root_uuid).unwrap_or_else(|| kid(&["kicad-sheet", "/"])),
        name: "Schematic".into(),
        parent: None,
        pos: Point::ZERO,
        size: Size::default(),
        page: Page::default(),
        title_block: Some(title_block(&root_node)),
    };
    let root_id = root_sheet.id;
    l.out.sheets.push(root_sheet);
    l.sheet(&file, root_id, &format!("/{root_uuid}"), 0)?;
    let mut out = l.out;
    to_pages(&mut out);
    // Bus entries: `pos` is the end on the bus.
    let on_bus =
        |sheet: Id, p: Point, buses: &[BusWire]| buses.iter().any(|b| b.sheet == sheet && on_segment(p, b.a, b.b));
    for e in &mut out.bus_entries {
        let other = e.wire_end();
        if !on_bus(e.sheet, e.pos, &out.bus_wires) && on_bus(e.sheet, other, &out.bus_wires) {
            e.dir = entry_dir(e.pos - other).unwrap_or(e.dir);
            e.pos = other;
        }
    }
    Ok(out)
}

impl Loader<'_> {
    /// The page of a sheet file: `(paper "A4" [portrait])` or `(paper "User" W H)`.
    fn page(&mut self, node: &Node, file: &str) -> Page {
        let Some(paper) = node.child("paper") else { return Page::default() };
        let portrait = paper.flag("portrait");
        let size = match paper.arg(0).unwrap_or("A4") {
            "A5" => PageSize::A5,
            "A4" => PageSize::A4,
            "A3" => PageSize::A3,
            "A2" => PageSize::A2,
            "A1" => PageSize::A1,
            "A0" => PageSize::A0,
            "A" | "USLetter" => PageSize::Letter,
            "USLegal" => PageSize::Legal,
            "B" | "USLedger" => PageSize::Tabloid,
            "C" => PageSize::AnsiC,
            "D" => PageSize::AnsiD,
            "E" => PageSize::AnsiE,
            "User" => match (paper.num(1), paper.num(2)) {
                (Some(w), Some(h)) => PageSize::Custom { w: nm(w), h: nm(h) },
                _ => PageSize::A4,
            },
            other => {
                self.report.warn(format!("{file}: paper size {other} is unknown; A4 is used"));
                PageSize::A4
            }
        };
        Page { size, portrait }
    }
}

/// A file's `(title_block …)`.
fn title_block(node: &Node) -> TitleBlock {
    let Some(tb) = node.child("title_block") else { return TitleBlock::default() };
    let text = |key: &str| tb.child(key).and_then(|c| c.arg(0)).unwrap_or("").to_string();
    // KiCad's numbered comments (1–9) become lines, gaps kept as empty lines.
    let mut comments: Vec<String> = Vec::new();
    for c in tb.children("comment") {
        let n = c.num(0).unwrap_or(0.0) as usize;
        if (1..=9).contains(&n) {
            if comments.len() < n {
                comments.resize(n, String::new());
            }
            comments[n - 1] = c.arg(1).unwrap_or("").to_string();
        }
    }
    while comments.last().is_some_and(String::is_empty) {
        comments.pop();
    }
    TitleBlock {
        title: text("title"),
        date: text("date"),
        revision: text("rev"),
        company: text("company"),
        comment: comments.join("\n"),
    }
}

/// KiCad pages hang down from the origin (y down, so negative y here); ours stand on it.
/// Shift each sheet's content up by its page height (on the grid, [`page_lift`]).
fn to_pages(out: &mut SchImport) {
    let heights: HashMap<Id, Coord> = out.sheets.iter().map(|s| (s.id, page_lift(&s.page))).collect();
    let up = |sheet: Id| Point::new(0, heights.get(&sheet).copied().unwrap_or(0));
    for s in &mut out.sheets {
        if let Some(parent) = s.parent {
            s.pos = s.pos + up(parent);
        }
    }
    for i in &mut out.instances {
        i.pos = i.pos + up(i.sheet);
    }
    for w in &mut out.wires {
        (w.a, w.b) = (w.a + up(w.sheet), w.b + up(w.sheet));
    }
    for w in &mut out.bus_wires {
        (w.a, w.b) = (w.a + up(w.sheet), w.b + up(w.sheet));
    }
    for e in &mut out.bus_entries {
        e.pos = e.pos + up(e.sheet);
    }
    for j in &mut out.junctions {
        j.pos = j.pos + up(j.sheet);
    }
    for n in &mut out.no_connects {
        n.pos = n.pos + up(n.sheet);
    }
    for f in &mut out.power_flags {
        f.pos = f.pos + up(f.sheet);
    }
    for l in &mut out.labels {
        l.pos = l.pos + up(l.sheet);
    }
    for t in &mut out.texts {
        t.text.pos = t.text.pos + up(t.sheet);
    }
    // Pins of a sub-sheet's symbol are on its parent.
    let parents: HashMap<Id, Id> = out.sheets.iter().filter_map(|s| Some((s.id, s.parent?))).collect();
    for (sheet, _, at) in &mut out.kicad_sheet_pins {
        if let Some(&parent) = parents.get(sheet) {
            *at = *at + up(parent);
        }
    }
}

/// A KiCad UUID as our id.
pub fn parse_id(uuid: &str) -> Option<Id> {
    uuid.parse().ok()
}

fn read_schematic(path: &Path) -> Result<Node, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let root = sexpr::parse(&text).map_err(|e| format!("{}: {e}", path.display()))?;
    if root.head() != "kicad_sch" {
        return Err(format!("{}: not a KiCad schematic", path.display()));
    }
    let version = root.child("version").and_then(|v| v.num(0)).unwrap_or(0.0) as u64;
    if version < MIN_VERSION {
        return Err(format!(
            "{}: schematic format {version} is older than KiCad 6; open and save it in KiCad 6 or newer first",
            path.display()
        ));
    }
    Ok(root)
}

impl Loader<'_> {
    /// Load `file` and the files of its sub-sheets, counting how often each is used.
    fn count_uses(&mut self, file: &str, depth: usize) -> Result<(), String> {
        if depth > 32 {
            return Err(format!("{file}: sheets nested too deeply (recursive sheets?)"));
        }
        *self.uses.entry(file.to_string()).or_default() += 1;
        if !self.files.contains_key(file) {
            let node = read_schematic(&self.dir.join(file))?;
            if let Some(ls) = node.child("lib_symbols") {
                for s in ls.children("symbol") {
                    if let Some(name) = s.arg(0) {
                        self.lib_symbols.entry(name.to_string()).or_insert_with(|| s.clone());
                    }
                }
            }
            self.files.insert(file.to_string(), node);
        }
        let subs: Vec<String> = self.files[file].children("sheet").filter_map(sheet_file).collect();
        for s in subs {
            self.count_uses(&s, depth + 1)?;
        }
        Ok(())
    }

    /// Import the content of `file` as sheet `sheet` with KiCad instance path `path`.
    fn sheet(&mut self, file: &str, sheet: Id, path: &str, depth: usize) -> Result<(), String> {
        let node = self.files[file].clone();
        let reused = self.uses[file] > 1;
        let id = |uuid: Option<&str>| -> Id {
            match uuid {
                Some(u) if !reused => parse_id(u).unwrap_or_else(|| kid(&["kicad", path, u])),
                Some(u) => kid(&["kicad", path, u]),
                None => Id::new(),
            }
        };
        let uuid_of = |n: &Node| n.child("uuid").and_then(|u| u.arg(0)).map(str::to_string);
        let page = self.page(&node, file);
        if let Some(s) = self.out.sheets.iter_mut().find(|s| s.id == sheet) {
            s.page = page;
        }
        if depth > 0 && node.child("title_block").is_some_and(|t| t.lists().next().is_some()) {
            let root = self.out.sheets.first().and_then(|s| s.title_block.clone()).unwrap_or_default();
            if title_block(&node) != root {
                self.report.note("sub-sheets have title blocks of their own; the root sheet's is used");
            }
        }
        for item in node.lists() {
            let uuid = uuid_of(item);
            let oid = id(uuid.as_deref());
            match item.head() {
                "symbol" => self.symbol(item, sheet, path, oid),
                "wire" | "bus" => {
                    let p = pts(item, true);
                    for w in p.windows(2) {
                        let wid = if p.len() == 2 { oid } else { Id::new() };
                        if item.head() == "wire" {
                            self.out.wires.push(Wire { id: wid, sheet, a: w[0], b: w[1] });
                        } else {
                            self.out.bus_wires.push(BusWire { id: wid, sheet, a: w[0], b: w[1] });
                        }
                    }
                }
                "bus_entry" => {
                    let (Some(at), Some((dx, dy))) = (child_point(item, "at", true), item.xy("size")) else { continue };
                    let other = at + point(dx, dy, true);
                    self.out.bus_entries.push(BusEntry { id: oid, sheet, pos: at, dir: EntryDir::UpRight });
                    // Resolved against the bus wires once the sheet is read.
                    self.out.bus_entries.last_mut().unwrap().dir = entry_dir(other - at).unwrap_or_default();
                    if entry_dir(other - at).is_none() {
                        self.report.skip("bus entries of an unusual size (made 2.54 mm)");
                    }
                }
                "junction" => {
                    if let Some(at) = child_point(item, "at", true) {
                        self.out.junctions.push(Junction { id: oid, sheet, pos: at });
                    }
                }
                "no_connect" => {
                    if let Some(at) = child_point(item, "at", true) {
                        self.out.no_connects.push(NoConnect { id: oid, sheet, pos: at });
                    }
                }
                "label" | "global_label" | "hierarchical_label" => {
                    let Some(at) = item.child("at") else { continue };
                    let Some(pos) = child_point(item, "at", true) else { continue };
                    let kind = match item.head() {
                        "label" => LabelKind::Local,
                        "global_label" => LabelKind::Global,
                        _ => LabelKind::Hierarchical,
                    };
                    let shape = match item.child("shape").and_then(|s| s.arg(0)) {
                        Some("output") => LabelShape::Output,
                        Some("bidirectional") => LabelShape::Bidirectional,
                        Some("tri_state") => LabelShape::TriState,
                        Some("passive") => LabelShape::Passive,
                        _ => LabelShape::Input,
                    };
                    self.out.labels.push(NetLabel {
                        id: oid,
                        sheet,
                        pos,
                        name: plain(item.arg(0).unwrap_or("")),
                        kind,
                        shape,
                        style: None,
                        rotation: Angle::deg(at.num(2).unwrap_or(0.0)),
                    });
                }
                "text" => {
                    let content = plain(item.arg(0).unwrap_or(""));
                    if let Some(at) = item.child("at")
                        && let Some(text) = text(content, at, item.child("effects"), true, 1.0)
                    {
                        self.out.texts.push(SchText { id: oid, sheet, text });
                    }
                }
                "sheet" => self.sub_sheet(item, sheet, path, oid, depth)?,
                "polyline" | "rectangle" | "circle" | "arc" | "bezier" => {
                    self.report.skip("schematic graphics (lines, rectangles, circles, arcs)")
                }
                "image" => self.report.skip("schematic images"),
                "text_box" => self.report.skip("schematic text boxes"),
                "table" => self.report.skip("schematic tables"),
                "netclass_flag" | "directive_label" => self.report.skip("net class flags (directive labels)"),
                "bus_alias" => self.report.skip("bus aliases"),
                "rule_area" => self.report.skip("schematic rule areas"),
                _ => {}
            }
        }
        Ok(())
    }

    fn sub_sheet(&mut self, item: &Node, parent: Id, path: &str, id: Id, depth: usize) -> Result<(), String> {
        let Some(file) = sheet_file(item) else { return Ok(()) };
        let name = sheet_prop(item, &["Sheetname", "Sheet name"]).unwrap_or_else(|| file.clone());
        let (Some((x, y)), Some((w, h))) = (item.xy("at"), item.xy("size")) else { return Ok(()) };
        // KiCad gives the top left corner (y down); ours is the lower left (y up).
        let size = Size::new(nm(w), nm(h));
        let pos = point(x, y + h, true);
        let mut sheet_name = name.clone();
        let mut n = 2;
        while self.out.sheets.iter().any(|s| s.parent == Some(parent) && s.name == sheet_name) {
            sheet_name = format!("{name} ({n})");
            n += 1;
        }
        self.out.sheets.push(Sheet {
            id,
            name: sheet_name,
            parent: Some(parent),
            pos,
            size,
            page: Page::default(),
            title_block: None,
        });
        // Pins: remembered where KiCad has them, wired to ours later.
        for pin in item.children("pin") {
            if let Some(at) = child_point(pin, "at", true) {
                self.out.kicad_sheet_pins.push((id, plain(pin.arg(0).unwrap_or("")), at));
            }
        }
        let uuid = item.child("uuid").and_then(|u| u.arg(0)).unwrap_or("");
        self.sheet(&file, id, &format!("{path}/{uuid}"), depth + 1)
    }

    fn symbol(&mut self, item: &Node, sheet: Id, path: &str, id: Id) {
        let Some(lib_id) = item.child("lib_id").and_then(|l| l.arg(0)).map(str::to_string) else { return };
        let Some((lib_node, parent)) = self.lib_symbol(item, &lib_id) else {
            self.report.warn(format!("symbol {lib_id} is missing from the schematic's library cache; skipped"));
            return;
        };
        let mut props: BTreeMap<String, String> = item
            .children("property")
            .filter_map(|p| Some((p.arg(0)?.to_string(), p.arg(1).unwrap_or("").to_string())))
            .collect();
        // `Sim.*` fields the instance doesn't override come from the library symbol (as
        // in KiCad; its KiCad 5 `Spice_*` fields don't).
        for p in lib_node.children("property") {
            if let (Some(k), Some(v)) = (p.arg(0), p.arg(1))
                && k.starts_with("Sim.")
                && !props.contains_key(k)
            {
                props.insert(k.to_string(), v.to_string());
            }
        }
        let Some(at) = item.child("at") else { return };
        let pos = point(at.num(0).unwrap_or(0.0), at.num(1).unwrap_or(0.0), true);
        let mirror = item.child("mirror").and_then(|m| m.arg(0)).map(str::to_string);
        let (rotation, mirrored) = orientation(at.num(2).unwrap_or(0.0), mirror.as_deref());

        let (lib, name) = lib_id.split_once(':').unwrap_or(("", lib_id.as_str()));
        let parent = parent.map(|p| {
            let pname = p.arg(0).unwrap_or("").rsplit(':').next().unwrap_or("").to_string();
            renamed(&p, &pname)
        });
        let sym = crate::symbol::convert(lib, &renamed(&lib_node, name), parent.as_ref());
        if sym.power {
            self.power(&sym, &props, sheet, id, pos, rotation, mirrored, &lib_id);
            return;
        }
        if let Some(e) = sym.hidden_power_error() {
            let msg = format!("{lib_id}: {e}");
            if !self.out.refused.contains(&msg) {
                self.out.refused.push(msg);
            }
            return;
        }

        // Reference and unit of this instance.
        let uuid = item.child("uuid").and_then(|u| u.arg(0)).unwrap_or("");
        let mut reference = props.get("Reference").cloned().unwrap_or_default();
        let mut unit = item.child("unit").and_then(|u| u.num(0)).unwrap_or(1.0) as u32;
        if let Some((r, u)) = self.legacy_refs.get(&format!("{path}/{uuid}")) {
            (reference, unit) = (r.clone(), *u);
        }
        if let Some(inst) = item.child("instances") {
            for proj in inst.children("project") {
                for p in proj.children("path") {
                    if p.arg(0) == Some(path) {
                        if let Some(r) = p.child("reference").and_then(|r| r.arg(0)) {
                            reference = r.to_string();
                        }
                        if let Some(u) = p.child("unit").and_then(|u| u.num(0)) {
                            unit = u as u32;
                        }
                    }
                }
            }
        }
        if reference.is_empty() || reference.ends_with('?') {
            // Where KiCad shows it: its own coordinates, in mm.
            let at = item.child("at");
            let (x, y) = (at.and_then(|a| a.num(0)).unwrap_or(0.0), at.and_then(|a| a.num(1)).unwrap_or(0.0));
            let on = self.out.sheets.iter().find(|s| s.id == sheet).map_or("", |s| s.name.as_str());
            self.report.warn(format!(
                "{lib_id} at ({x}, {y}) mm on sheet {on} has no reference designator (not annotated); skipped"
            ));
            return;
        }

        let index = match self.by_refdes.get(&reference) {
            Some(&i) => i,
            None => {
                let mut properties = BTreeMap::new();
                for (k, v) in &props {
                    if !v.is_empty()
                        && !["Reference", "Value", "Footprint", "Description", "Datasheet"].contains(&k.as_str())
                    {
                        properties.insert(k.clone(), v.clone());
                    }
                }
                if item.child("in_bom").and_then(|n| n.arg(0)) == Some("no") {
                    properties.insert("In BOM".into(), "no".into());
                }
                let footprint = props.get("Footprint").map(|f| f.trim().to_string()).filter(|f| !f.is_empty());
                self.out.components.push(SchComponent {
                    id: kid(&["kicad-component", &reference]),
                    refdes: reference.clone(),
                    lib_id: lib_id.clone(),
                    value: props.get("Value").cloned().unwrap_or_default(),
                    footprint,
                    on_board: item.child("on_board").and_then(|n| n.arg(0)) != Some("no"),
                    exclude_from_sim: item.child("exclude_from_sim").and_then(|n| n.arg(0)) == Some("yes"),
                    dnp: item.child("dnp").and_then(|n| n.arg(0)) == Some("yes"),
                    properties,
                });
                self.out.symbols.entry(lib_id.clone()).or_insert(sym);
                self.by_refdes.insert(reference.clone(), self.out.components.len() - 1);
                self.out.components.len() - 1
            }
        };
        let component = self.out.components[index].id;
        let unit = unit.saturating_sub(1);
        if self.out.instances.iter().any(|i| i.component == component && i.unit == unit) {
            self.report.warn(format!("{reference}: unit {} placed twice; the copy is skipped", unit + 1));
            return;
        }
        let inst = SymbolInstance { id, sheet, component, unit, pos, rotation, mirror: mirrored };
        // Reference and value where this instance shows them: absolute sheet positions and
        // angles in KiCad, symbol coordinates here.
        let t = inst.transform();
        for (key, placeholder) in [("Reference", "${REF}"), ("Value", "${VALUE}")] {
            let Some(p) = item.children("property").find(|p| p.arg(0) == Some(key)) else { continue };
            let Some(at) = p.child("at") else { continue };
            let effects = p.child("effects");
            let hidden = p.flag("hide") || effects.is_some_and(|e| e.flag("hide"));
            let visible_effects = effects.filter(|_| !hidden);
            let Some(mut text) = text(placeholder.into(), at, visible_effects, true, 1.0) else { continue };
            if hidden && let Some(size) = effects.and_then(|e| e.child("font")).and_then(|f| f.child("size")) {
                text.size = nm(size.num(0).unwrap_or(1.27));
            }
            // Positions are absolute, angles relative to the symbol's (KiCad draws a
            // field at 0° along a symbol turned by 90° vertically).
            text.pos = t.inverse_point(text.pos);
            text.rotation = t.inverse_angle(text.rotation.rotated_by(rotation));
            self.out.fields.push(SymbolField {
                id: kid(&["kicad-field", &id.to_string(), key]),
                symbol: id,
                text,
                hidden,
            });
        }
        self.out.instances.push(inst);
    }

    /// A power symbol: a power label at its pin, or a power flag for PWR_FLAG (its pin is
    /// a power output).
    #[allow(clippy::too_many_arguments)]
    fn power(
        &mut self,
        sym: &KiSymbol,
        props: &BTreeMap<String, String>,
        sheet: Id,
        id: Id,
        pos: Point,
        rotation: Angle,
        mirror: bool,
        lib_id: &str,
    ) {
        let Some(unit) = sym.units.first() else { return };
        let pins: Vec<&Pin> = unit.pins.iter().collect();
        let flag = pins.len() == 1 && pins[0].kind == PinKind::PowerOut;
        if pins.len() != 1 || !(pins[0].kind == PinKind::PowerIn || flag) {
            self.report.skip("power symbols with several pins or other pin kinds");
            return;
        }
        let t = Transform::new(pos, rotation, mirror);
        let at = t.point(pins[0].pos);
        // Unrotated, KiCad's power symbols draw their glyph above the pin (ground: below),
        // as ours do: our rotation is where the symbol's "up" ends up.
        let up = Transform::new(Point::ZERO, rotation, mirror).point(Point::new(0, 1_000_000));
        let rotation = Angle::deg((up.y as f64).atan2(up.x as f64).to_degrees() - 90.0);
        if flag {
            self.out.power_flags.push(PowerFlag { id, sheet, pos: at, rotation });
            return;
        }
        let name = plain(props.get("Value").map_or("", String::as_str));
        let style = power_style(lib_id, &name);
        self.out.labels.push(NetLabel {
            id,
            sheet,
            pos: at,
            name,
            kind: LabelKind::Power,
            shape: LabelShape::default(),
            style: Some(style),
            rotation,
        });
    }

    /// The embedded library symbol of an instance (`lib_name` overrides `lib_id` when
    /// the cached copy was renamed).
    /// The embedded library symbol of an instance (`lib_name` overrides `lib_id` when
    /// the cached copy was renamed), and the symbol it extends, if any (derived symbols
    /// are usually flattened in the cache).
    fn lib_symbol(&self, item: &Node, lib_id: &str) -> Option<(Node, Option<Node>)> {
        let key = item.child("lib_name").and_then(|l| l.arg(0)).unwrap_or(lib_id);
        let node = self.lib_symbols.get(key).or_else(|| self.lib_symbols.get(lib_id))?;
        let parent = node.child("extends").and_then(|e| e.arg(0)).and_then(|parent| {
            let lib = key.split_once(':').map_or("", |(l, _)| l);
            self.lib_symbols.get(&format!("{lib}:{parent}")).or_else(|| self.lib_symbols.get(parent)).cloned()
        });
        Some((node.clone(), parent))
    }
}

/// Our rotation and mirror of a KiCad symbol orientation: `angle` (counterclockwise on
/// screen) and `(mirror x)` (top to bottom) or `(mirror y)` (left to right). KiCad
/// mirrors after rotating; ours mirrors (left to right) first.
pub fn orientation(angle: f64, mirror: Option<&str>) -> (Angle, bool) {
    match mirror {
        Some("y") => (Angle::deg(-angle), true),
        Some("x") => (Angle::deg(180.0 - angle), true),
        _ => (Angle::deg(angle), false),
    }
}

fn power_style(lib_id: &str, name: &str) -> PowerStyle {
    let sym = lib_id.split_once(':').map_or(lib_id, |(_, n)| n).to_ascii_uppercase();
    match sym.as_str() {
        "EARTH" => PowerStyle::Ground,
        "GNDREF" | "GNDS" => PowerStyle::SignalGround,
        "CHASSIS" | "GNDPWR" => PowerStyle::Chassis,
        "PE" | "EARTH_PROTECTIVE" | "PROTECTIVE_EARTH" => PowerStyle::ProtectiveEarth,
        s if s.contains("GND") => PowerStyle::Ground,
        s if s.starts_with("VEE") || s.starts_with("-") => PowerStyle::Arrow,
        _ => PowerStyle::for_name(name),
    }
}

/// The direction of a bus entry from its bus end to its wire end.
fn entry_dir(offset: Point) -> Option<EntryDir> {
    EntryDir::ALL.into_iter().find(|d| d.offset() == offset)
}

fn sheet_prop(item: &Node, keys: &[&str]) -> Option<String> {
    item.children("property")
        .find(|p| p.arg(0).is_some_and(|k| keys.contains(&k)))
        .and_then(|p| p.arg(1))
        .map(str::to_string)
}

fn sheet_file(item: &Node) -> Option<String> {
    sheet_prop(item, &["Sheetfile", "Sheet file"]).filter(|f| !f.is_empty())
}

/// `node` with its name (first argument) replaced.
pub fn renamed(node: &Node, name: &str) -> Node {
    match node {
        Node::List(items) => {
            let mut items = items.clone();
            if items.len() > 1 {
                items[1] = Node::Atom(name.to_string());
            }
            Node::List(items)
        }
        other => other.clone(),
    }
}

/// Is `p` on the segment `a`–`b` (exactly, in nm)?
pub fn on_segment(p: Point, a: Point, b: Point) -> bool {
    let (d, q) = (b - a, p - a);
    let cross = d.x as i128 * q.y as i128 - d.y as i128 * q.x as i128;
    cross == 0 && p.x >= a.x.min(b.x) && p.x <= a.x.max(b.x) && p.y >= a.y.min(b.y) && p.y <= a.y.max(b.y)
}
