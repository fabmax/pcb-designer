//! Our schematic → KiCad schematic files: the root sheet as `NAME.kicad_sch`, each
//! sub-sheet as a file of its own.
//!
//! KiCad pages hang down from the origin with y down; ours stand on it with y up, so a
//! point is `(x, H − y)` with `H` the height of its sheet's page, rounded down to the
//! connection grid ([`crate::common::page_lift`]). KiCad's schematic
//! resolution is 100 nm: other coordinates are rounded (and reported).

use std::collections::{BTreeMap, BTreeSet, HashMap};

use model::*;

use super::lib::{Names, SymbolDef, effects, item_name, property, symbol};
use super::sx::{X, l};
use crate::project::Report;

/// Where each sheet ended up: what the board needs to link footprints to symbols.
#[derive(Clone, Debug, Default)]
pub struct SheetInfo {
    /// The KiCad file of the sheet.
    pub file: String,
    /// `/uuid/uuid…` of the sheet instance below the root (empty for the root).
    pub uuid_path: String,
    /// `/Name/Name/` (`/` for the root).
    pub name_path: String,
}

pub struct SchExport {
    /// (file name, content).
    pub files: Vec<(String, String)>,
    pub sheets: HashMap<Id, SheetInfo>,
    /// Component → the symbol instance its footprint links to (its lowest unit).
    pub anchor: HashMap<Id, Id>,
    /// Power symbols made for power ports and flags (for the project library).
    pub power_symbols: Vec<X>,
}

struct Ctx<'a> {
    p: &'a Project,
    names: &'a Names,
    report: &'a mut Report,
    project: String,
    root_uuid: String,
    /// Page height per sheet.
    heights: HashMap<Id, Coord>,
    sheets: HashMap<Id, SheetInfo>,
    pages: HashMap<Id, usize>,
    snapped: bool,
    /// Power symbol name per (style, net name), and their definitions.
    power: BTreeMap<(String, String), String>,
    power_defs: Vec<(String, X)>,
    power_refs: usize,
    flag_refs: usize,
}

pub fn export(p: &Project, names: &Names, project_name: &str, report: &mut Report) -> SchExport {
    let s = &p.schematic;
    let root = s.root_sheet();
    let heights = s.sheets.iter().map(|sh| (sh.id, crate::common::page_lift(&sh.page))).collect();
    let mut cx = Ctx {
        p,
        names,
        report,
        project: project_name.to_string(),
        root_uuid: root.map(|r| r.to_string()).unwrap_or_default(),
        heights,
        sheets: HashMap::new(),
        pages: s.page_order().into_iter().enumerate().map(|(i, id)| (id, i + 1)).collect(),
        snapped: false,
        power: BTreeMap::new(),
        power_defs: Vec::new(),
        power_refs: 0,
        flag_refs: 0,
    };

    // File names and paths, parents first.
    let mut taken: BTreeSet<String> = BTreeSet::new();
    let root_file = format!("{project_name}.kicad_sch");
    taken.insert(root_file.to_ascii_lowercase());
    for id in s.page_order() {
        let Some(sheet) = s.sheets.get(id) else { continue };
        let info = match sheet.parent.and_then(|pid| cx.sheets.get(&pid).cloned()) {
            None => SheetInfo { file: root_file.clone(), uuid_path: String::new(), name_path: "/".into() },
            Some(parent) => {
                let base = item_name(&sheet.name);
                let mut file = format!("{base}.kicad_sch");
                let mut n = 2;
                while !taken.insert(file.to_ascii_lowercase()) {
                    file = format!("{base}_{n}.kicad_sch");
                    n += 1;
                }
                SheetInfo {
                    file,
                    uuid_path: format!("{}/{}", parent.uuid_path, sheet.id),
                    name_path: format!("{}{}/", parent.name_path, sheet.name),
                }
            }
        };
        cx.sheets.insert(id, info);
    }

    // Each component links its footprint to its lowest placed unit.
    let mut anchor: HashMap<Id, (u32, Id)> = HashMap::new();
    for i in s.symbols.iter() {
        let e = anchor.entry(i.component).or_insert((i.unit, i.id));
        if i.unit < e.0 {
            *e = (i.unit, i.id);
        }
    }

    let mut files = Vec::new();
    for id in s.page_order() {
        if let Some(sheet) = s.sheets.get(id) {
            let text = cx.sheet_file(sheet);
            files.push((cx.sheets[&id].file.clone(), text));
        }
    }
    if cx.snapped {
        cx.report.note("schematic coordinates were rounded to KiCad's 0.1 µm resolution");
    }
    SchExport {
        files,
        sheets: cx.sheets,
        anchor: anchor.into_iter().map(|(c, (_, i))| (c, i)).collect(),
        power_symbols: cx.power_defs.into_iter().map(|(_, x)| x).collect(),
    }
}

fn paper(page: &Page) -> X {
    let named = |n: &str| {
        let x = l("paper").str(n);
        if page.portrait { x.sym("portrait") } else { x }
    };
    match page.size {
        PageSize::None | PageSize::A4 => named("A4"),
        PageSize::A5 => named("A5"),
        PageSize::A3 => named("A3"),
        PageSize::A2 => named("A2"),
        PageSize::A1 => named("A1"),
        PageSize::A0 => named("A0"),
        PageSize::Letter => named("USLetter"),
        PageSize::Legal => named("USLegal"),
        PageSize::Tabloid => named("USLedger"),
        PageSize::AnsiC => named("C"),
        PageSize::AnsiD => named("D"),
        PageSize::AnsiE => named("E"),
        PageSize::Custom { w, h } => l("paper").str("User").mm(w).mm(h),
    }
}

fn title_block(tb: &TitleBlock) -> X {
    let mut x = l("title_block");
    for (k, v) in [("title", &tb.title), ("date", &tb.date), ("rev", &tb.revision), ("company", &tb.company)] {
        if !v.is_empty() {
            x = x.push(l(k).str(v));
        }
    }
    for (i, line) in tb.comment.lines().enumerate().take(9) {
        if !line.is_empty() {
            x = x.push(l("comment").sym((i + 1).to_string()).str(line));
        }
    }
    x
}

fn uuid(id: Id) -> X {
    l("uuid").str(id.to_string())
}

/// A derived id for KiCad objects that have no object of their own here.
fn sub_uuid(id: Id, what: &str) -> X {
    l("uuid").str(crate::common::kid(&["kicad-export", &id.to_string(), what]).to_string())
}

fn default_stroke() -> X {
    l("stroke").push(l("width").num(0.0)).push(l("type").sym("default"))
}

fn font(size: f64) -> X {
    l("effects").push(l("font").push(l("size").num(size).num(size)))
}

/// The middle of a text as we draw it (JetBrains Mono: an advance of about 0.82 cap
/// heights per character).
fn text_center(t: &Text) -> Point {
    let lines: Vec<&str> = t.text.split('\n').collect();
    let w = lines.iter().map(|l| l.chars().count()).max().unwrap_or(0) as f64 * t.size as f64 * 0.82;
    let h = t.size as f64 * (1.0 + 1.6 * (lines.len().max(1) - 1) as f64);
    let dx = match t.halign {
        HAlign::Left => w / 2.0,
        HAlign::Center => 0.0,
        HAlign::Right => -w / 2.0,
    };
    let dy = match t.valign {
        VAlign::Bottom => h / 2.0,
        VAlign::Center => 0.0,
        VAlign::Top => -h / 2.0,
    };
    t.pos + Point::new(dx.round() as Coord, dy.round() as Coord).rotated(t.rotation)
}

/// A KiCad angle of 0, 90, 180 or 270 nearest to `a`.
fn quarter(a: f64) -> f64 {
    ((a / 90.0).round() * 90.0).rem_euclid(360.0)
}

impl Ctx<'_> {
    /// mm of a schematic coordinate, rounded to 100 nm.
    fn c(&mut self, v: Coord) -> String {
        let r = (v as f64 / 100.0).round() as Coord * 100;
        if r != v {
            self.snapped = true;
        }
        super::sx::mm(r)
    }

    /// `(head x y)` of a point on `sheet`.
    fn pt(&mut self, head: &str, sheet: Id, p: Point) -> X {
        let h = self.heights.get(&sheet).copied().unwrap_or(0);
        let (x, y) = (self.c(p.x), self.c(h - p.y));
        l(head).sym(x).sym(y)
    }

    fn at(&mut self, sheet: Id, p: Point, angle: f64) -> X {
        self.pt("at", sheet, p).num(angle)
    }

    fn instance_path(&self, sheet: Id) -> String {
        let below = self.sheets.get(&sheet).map_or("", |s| s.uuid_path.as_str());
        format!("/{}{below}", self.root_uuid)
    }

    fn sheet_file(&mut self, sheet: &Sheet) -> String {
        let p = self.p;
        let s = &p.schematic;
        let id = sheet.id;
        let mut x = l("kicad_sch")
            .push(l("version").sym("20250114"))
            .push(l("generator").str("pcb-designer"))
            .push(l("generator_version").str("9.0"))
            // The root file's uuid starts the instance paths; a sub-sheet file's must
            // differ from its sheet symbol's.
            .push(if sheet.is_root() { uuid(id) } else { sub_uuid(id, "file") })
            .push(paper(&sheet.page));
        let tb = s.title_block();
        x = x.push(title_block(&tb));

        // Library symbols used on this sheet, then the items.
        let mut items: Vec<X> = Vec::new();
        let mut used: BTreeMap<String, X> = BTreeMap::new();
        for inst in s.symbols.iter().filter(|i| i.sheet == id) {
            if let Some(item) = self.symbol_instance(inst, &mut used) {
                items.push(item);
            }
        }
        for label in s.labels.iter().filter(|l| l.sheet == id) {
            items.push(self.label(label, &mut used));
        }
        for f in s.power_flags.iter().filter(|f| f.sheet == id) {
            items.push(self.power_flag(f, &mut used));
        }
        for w in s.wires.iter().filter(|w| w.sheet == id) {
            let pts = l("pts").push(self.pt("xy", id, w.a)).push(self.pt("xy", id, w.b));
            items.push(l("wire").push(pts).push(default_stroke()).push(uuid(w.id)));
        }
        for w in s.bus_wires.iter().filter(|w| w.sheet == id) {
            let pts = l("pts").push(self.pt("xy", id, w.a)).push(self.pt("xy", id, w.b));
            items.push(l("bus").push(pts).push(default_stroke()).push(uuid(w.id)));
        }
        for e in s.bus_entries.iter().filter(|e| e.sheet == id) {
            let off = e.dir.offset();
            let at = self.pt("at", id, e.pos);
            items.push(
                l("bus_entry").push(at).push(l("size").mm(off.x).mm(-off.y)).push(default_stroke()).push(uuid(e.id)),
            );
        }
        for j in s.junctions.iter().filter(|j| j.sheet == id) {
            let at = self.pt("at", id, j.pos);
            items.push(
                l("junction")
                    .push(at)
                    .push(l("diameter").num(0.0))
                    .push(l("color").num(0.0).num(0.0).num(0.0).num(0.0))
                    .push(uuid(j.id)),
            );
        }
        for n in s.no_connects.iter().filter(|n| n.sheet == id) {
            let at = self.pt("at", id, n.pos);
            items.push(l("no_connect").push(at).push(uuid(n.id)));
        }
        for t in s.texts.iter().filter(|t| t.sheet == id) {
            let a = t.text.rotation.degrees();
            let q = if (a.rem_euclid(180.0) - 90.0).abs() < 45.0 { 90.0 } else { 0.0 };
            if (a - q).abs() > 1e-6 && (a - q - 180.0).abs() > 1e-6 {
                self.report.skip("schematic texts at angles other than 0° and 90° (turned to the nearest)");
            }
            let at = self.at(id, t.text.pos, q);
            items.push(
                l("text")
                    .str(super::lib::field_vars(&t.text.text))
                    .yes_no("exclude_from_sim", false)
                    .push(at)
                    .push(effects(&t.text, false, None))
                    .push(uuid(t.id)),
            );
        }
        for child in s.children(id) {
            items.push(self.sheet_symbol(child));
        }

        x = x.push(l("lib_symbols").extend(used.into_values()));
        x = x.extend(items);
        if sheet.is_root() {
            x = x.push(l("sheet_instances").push(l("path").str("/").push(l("page").str("1"))));
        }
        x.yes_no("embedded_fonts", false).to_text()
    }

    /// A placed unit of a component.
    fn symbol_instance(&mut self, inst: &SymbolInstance, used: &mut BTreeMap<String, X>) -> Option<X> {
        let p = self.p;
        let c = p.schematic.components.get(inst.component)?;
        let part = p.library.parts.get(c.part)?;
        let variant = part.variant(c.variant)?;
        let lib_id = self.names.symbol_id(part.id, variant.id)?;
        let footprint = variant.footprint.and_then(|f| self.names.footprint_id(f));
        if !used.contains_key(&lib_id) {
            let d =
                SymbolDef { library: &p.library, part, variant, name: lib_id.clone(), footprint: footprint.clone() };
            used.insert(lib_id.clone(), symbol(&d, self.report));
        }
        let sheet = inst.sheet;
        let rot = inst.rotation.degrees();
        // KiCad mirrors after rotating, left to right as `(mirror y)`.
        let angle = quarter(if inst.mirror { -rot } else { rot });
        if (quarter(rot) - rot.rem_euclid(360.0)).abs() > 1e-6 {
            self.report.skip("symbols at angles other than multiples of 90° (turned to the nearest)");
        }
        let mut x = l("symbol").push(l("lib_id").str(&lib_id)).push(self.at(sheet, inst.pos, angle));
        if inst.mirror {
            x = x.push(l("mirror").sym("y"));
        }
        let in_bom = c.properties.get("In BOM").is_none_or(|v| v != "no");
        x = x
            .push(l("unit").sym((inst.unit + 1).to_string()))
            .yes_no("exclude_from_sim", c.sim.exclude)
            .yes_no("in_bom", in_bom)
            .yes_no("on_board", variant.footprint.is_some())
            .yes_no("dnp", c.dnp)
            .push(uuid(inst.id));

        // Reference and value where this symbol shows them.
        let t = Transform::new(inst.pos, Angle::deg(quarter(rot)), inst.mirror);
        let fields: Vec<&SymbolField> = p.schematic.fields.iter().filter(|f| f.symbol == inst.id).collect();
        let value = if c.value.is_empty() { part.default_value.clone() } else { c.value.clone() };
        for (key, placeholder, v) in [("Reference", "${REF}", c.refdes.clone()), ("Value", "${VALUE}", value)] {
            let (at, hidden, fx) = match fields.iter().find(|f| f.text.text == placeholder) {
                Some(f) => {
                    // KiCad turns a field's justification with its symbol, we keep fields
                    // upright where they are: centred on the text as we draw it, they land
                    // in the same place.
                    let shown = Text { text: v.clone(), ..f.text.transformed(&t).upright() };
                    // KiCad draws a field at its angle plus its symbol's (kept readable).
                    let a = (shown.rotation.degrees() - quarter(rot)).rem_euclid(180.0);
                    let a = if (a - 90.0).abs() < 45.0 { 90.0 } else { 0.0 };
                    let centred = Text { halign: HAlign::Center, valign: VAlign::Center, ..shown.clone() };
                    (self.at(sheet, text_center(&shown), a), f.hidden, effects(&centred, false, None))
                }
                None => (self.at(sheet, inst.pos, 0.0), true, font(1.27)),
            };
            x = x.push(property(key, &v, at, hidden, fx));
        }
        let hidden_at = self.at(sheet, inst.pos, 0.0);
        x = x.push(property("Footprint", footprint.as_deref().unwrap_or(""), hidden_at.clone(), true, font(1.27)));
        let datasheet = c.properties.get("Datasheet").or(part.properties.get("Datasheet"));
        x = x.push(property("Datasheet", datasheet.map_or("", String::as_str), hidden_at.clone(), true, font(1.27)));
        x = x.push(property("Description", &part.description, hidden_at.clone(), true, font(1.27)));
        for (k, v) in &c.properties {
            if !["Datasheet", "In BOM", "Reference", "Value", "Footprint", "Description"].contains(&k.as_str()) {
                x = x.push(property(k, v, hidden_at.clone(), true, font(1.27)));
            }
        }
        // The unit's pins.
        let numbers = super::lib::pin_numbers(&p.library, part, variant);
        let mut nums: Vec<&String> =
            numbers.iter().filter(|((u, _), _)| *u == inst.unit).flat_map(|(_, v)| v).collect();
        nums.sort_by(|a, b| natural_cmp(a, b));
        for n in nums {
            x = x.push(l("pin").str(n).push(sub_uuid(inst.id, &format!("pin:{n}"))));
        }
        x = x.push(self.instances(sheet, &c.refdes, inst.unit + 1));
        Some(x)
    }

    fn instances(&self, sheet: Id, reference: &str, unit: u32) -> X {
        l("instances").push(
            l("project").str(&self.project).push(
                l("path")
                    .str(self.instance_path(sheet))
                    .push(l("reference").str(reference))
                    .push(l("unit").sym(unit.to_string())),
            ),
        )
    }

    fn label(&mut self, label: &NetLabel, used: &mut BTreeMap<String, X>) -> X {
        let sheet = label.sheet;
        let a = quarter(label.rotation.degrees());
        if label.kind == LabelKind::Power {
            return self.power_port(label, used);
        }
        let justify = if a == 180.0 || a == 270.0 { "right" } else { "left" };
        let fx = l("effects")
            .push(l("font").push(l("size").num(1.27).num(1.27)))
            .push(l("justify").sym(justify).sym("bottom"));
        let at = self.at(sheet, label.pos, a);
        let shape = l("shape").sym(match label.shape {
            LabelShape::Input => "input",
            LabelShape::Output => "output",
            LabelShape::Bidirectional => "bidirectional",
            LabelShape::TriState => "tri_state",
            LabelShape::Passive => "passive",
        });
        match label.kind {
            LabelKind::Local => l("label").str(net_text(&label.name)).push(at).push(fx).push(uuid(label.id)),
            LabelKind::Global => {
                let fx =
                    l("effects").push(l("font").push(l("size").num(1.27).num(1.27))).push(l("justify").sym(justify));
                let refs_at = self.at(sheet, label.pos, 0.0);
                l("global_label")
                    .str(net_text(&label.name))
                    .push(shape)
                    .push(at)
                    .yes_no("fields_autoplaced", true)
                    .push(fx)
                    .push(uuid(label.id))
                    .push(property("Intersheetrefs", "${INTERSHEET_REFS}", refs_at, true, font(1.27)))
            }
            _ => {
                let fx =
                    l("effects").push(l("font").push(l("size").num(1.27).num(1.27))).push(l("justify").sym(justify));
                l("hierarchical_label").str(net_text(&label.name)).push(shape).push(at).push(fx).push(uuid(label.id))
            }
        }
    }

    /// A power port as a KiCad power symbol of its own (one per glyph and name).
    fn power_port(&mut self, label: &NetLabel, used: &mut BTreeMap<String, X>) -> X {
        let style = label.power_style();
        let key = (style.label().to_string(), label.name.clone());
        let name = match self.power.get(&key) {
            Some(n) => n.clone(),
            None => {
                let base = format!("PWR_{}", item_name(&label.name));
                let mut name = base.clone();
                let mut n = 2;
                while self.power.values().any(|v| *v == name) {
                    name = format!("{base}_{n}");
                    n += 1;
                }
                self.power.insert(key, name.clone());
                self.power_defs.push((name.clone(), power_symbol(&name, &net_text(&label.name), style)));
                name
            }
        };
        self.report.note(
            "KiCad counts power ports as power inputs, not sources: a net powered only by power ports needs a \
             PWR_FLAG in KiCad (ERC power_pin_not_driven)",
        );
        self.power_refs += 1;
        let reference = format!("#PWR{:03}", self.power_refs);
        let lib_id = format!("{}:{name}", self.names.lib);
        used.entry(lib_id.clone()).or_insert_with(|| power_symbol(&lib_id, &net_text(&label.name), style));
        let down = style.points_down();
        let value_at = Transform::new(label.pos, label.rotation, false)
            .point(Point::new(0, if down { -3_810_000 } else { 3_556_000 }));
        self.power_instance(
            &lib_id,
            label.id,
            label.sheet,
            label.pos,
            label.rotation,
            &reference,
            &net_text(&label.name),
            value_at,
        )
    }

    fn power_flag(&mut self, f: &PowerFlag, used: &mut BTreeMap<String, X>) -> X {
        let lib_id = format!("{}:PWR_FLAG", self.names.lib);
        if !self.power_defs.iter().any(|(n, _)| n == "PWR_FLAG") {
            self.power_defs.push(("PWR_FLAG".into(), flag_symbol("PWR_FLAG")));
        }
        used.entry(lib_id.clone()).or_insert_with(|| flag_symbol(&lib_id));
        self.flag_refs += 1;
        let reference = format!("#FLG{:03}", self.flag_refs);
        let value_at = Transform::new(f.pos, f.rotation, false).point(Point::new(0, 3_810_000));
        self.power_instance(&lib_id, f.id, f.sheet, f.pos, f.rotation, &reference, "PWR_FLAG", value_at)
    }

    #[allow(clippy::too_many_arguments)]
    fn power_instance(
        &mut self,
        lib_id: &str,
        id: Id,
        sheet: Id,
        pos: Point,
        rotation: Angle,
        reference: &str,
        value: &str,
        value_at: Point,
    ) -> X {
        let a = quarter(rotation.degrees());
        let at = self.at(sheet, pos, a);
        let ref_at = self.at(sheet, pos, 0.0);
        let val_at = self.at(sheet, value_at, 0.0);
        l("symbol")
            .push(l("lib_id").str(lib_id))
            .push(at)
            .push(l("unit").sym("1"))
            .yes_no("exclude_from_sim", false)
            .yes_no("in_bom", true)
            .yes_no("on_board", true)
            .yes_no("dnp", false)
            .push(uuid(id))
            .push(property("Reference", reference, ref_at.clone(), true, font(1.27)))
            .push(property("Value", value, val_at, false, font(1.27)))
            .push(property("Footprint", "", ref_at.clone(), true, font(1.27)))
            .push(property("Datasheet", "", ref_at.clone(), true, font(1.27)))
            .push(property("Description", "", ref_at, true, font(1.27)))
            .push(l("pin").str("1").push(sub_uuid(id, "pin:1")))
            .push(self.instances(sheet, reference, 1))
    }

    /// The symbol of sub-sheet `child` on its parent.
    fn sheet_symbol(&mut self, child: &Sheet) -> X {
        let s = &self.p.schematic;
        let parent = child.parent.unwrap_or(child.id);
        let (rect, pins) = s.sheet_symbol(child.id).unwrap_or_else(|| (child.rect(), Vec::new()));
        let top_left = Point::new(rect.min.x, rect.max.y);
        let at = self.pt("at", parent, top_left);
        let file = self.sheets.get(&child.id).map(|i| i.file.clone()).unwrap_or_default();
        let name_at = self.at(parent, top_left + Point::new(0, 711_200), 0.0);
        let file_at = self.at(parent, Point::new(rect.min.x, rect.min.y - 589_000), 0.0);
        let justify = |v: &str| {
            l("effects").push(l("font").push(l("size").num(1.27).num(1.27))).push(l("justify").sym("left").sym(v))
        };
        let mut x = l("sheet")
            .push(at)
            .push(l("size").mm(rect.width()).mm(rect.height()))
            .yes_no("exclude_from_sim", false)
            .yes_no("in_bom", true)
            .yes_no("on_board", true)
            .yes_no("dnp", false)
            .yes_no("fields_autoplaced", true)
            .push(l("stroke").push(l("width").num(0.1524)).push(l("type").sym("solid")))
            .push(l("fill").push(l("color").num(0.0).num(0.0).num(0.0).num(0.0)))
            .push(uuid(child.id))
            .push(l("property").str("Sheetname").str(&child.name).push(name_at).push(justify("bottom")))
            .push(l("property").str("Sheetfile").str(&file).push(file_at).push(justify("top")));
        for pin in &pins {
            let shape = match pin.shape {
                LabelShape::Input => "input",
                LabelShape::Output => "output",
                LabelShape::Bidirectional => "bidirectional",
                LabelShape::TriState => "tri_state",
                LabelShape::Passive => "passive",
            };
            let at = self.at(parent, pin.pos, if pin.right { 0.0 } else { 180.0 });
            let justify = if pin.right { "right" } else { "left" };
            x = x.push(
                l("pin")
                    .str(net_text(&pin.name))
                    .sym(shape)
                    .push(at)
                    .push(sub_uuid(child.id, &format!("pin:{}", pin.name)))
                    .push(
                        l("effects")
                            .push(l("font").push(l("size").num(1.27).num(1.27)))
                            .push(l("justify").sym(justify)),
                    ),
            );
        }
        let page = self.pages.get(&child.id).copied().unwrap_or(1);
        x.push(
            l("instances").push(
                l("project")
                    .str(&self.project)
                    .push(l("path").str(self.instance_path(parent)).push(l("page").str(page.to_string()))),
            ),
        )
    }
}

/// A net name as KiCad writes it in labels and net names: `/` separates sheets there,
/// so one in a name is escaped (`{slash}`, which the import undoes).
pub fn net_text(name: &str) -> String {
    name.replace('/', "{slash}")
}

// ---------------------------------------------------------------------------------------
// Power symbols

fn line(a: (f64, f64), b: (f64, f64)) -> X {
    poly(&[a, b], false)
}

fn poly(pts: &[(f64, f64)], fill: bool) -> X {
    l("polyline")
        .push(l("pts").extend(pts.iter().map(|&(x, y)| l("xy").num(x).num(y))))
        .push(l("stroke").push(l("width").num(0.0)).push(l("type").sym("default")))
        .push(l("fill").push(l("type").sym(if fill { "outline" } else { "none" })))
}

fn circle(c: (f64, f64), r: f64) -> X {
    l("circle")
        .push(l("center").num(c.0).num(c.1))
        .push(l("radius").num(r))
        .push(l("stroke").push(l("width").num(0.0)).push(l("type").sym("default")))
        .push(l("fill").push(l("type").sym("none")))
}

/// The glyph of a power port, drawn as ours (mm, y up, pin at the origin).
fn glyph(style: PowerStyle) -> Vec<X> {
    match style {
        PowerStyle::Bar => vec![line((0.0, 0.0), (0.0, 1.27)), line((-1.27, 1.27), (1.27, 1.27))],
        PowerStyle::Arrow => {
            vec![line((0.0, 0.0), (0.0, 2.54)), poly(&[(-0.762, 1.27), (0.0, 2.54), (0.762, 1.27)], false)]
        }
        PowerStyle::Circle => vec![line((0.0, 0.0), (0.0, 1.27)), circle((0.0, 1.905), 0.635)],
        PowerStyle::Ground => vec![
            line((0.0, 0.0), (0.0, -1.27)),
            line((-1.27, -1.27), (1.27, -1.27)),
            line((-0.762, -1.778), (0.762, -1.778)),
            line((-0.254, -2.286), (0.254, -2.286)),
        ],
        PowerStyle::SignalGround => vec![
            line((0.0, 0.0), (0.0, -1.27)),
            poly(&[(-1.27, -1.27), (1.27, -1.27), (0.0, -2.54), (-1.27, -1.27)], false),
        ],
        PowerStyle::Chassis => vec![
            line((0.0, 0.0), (0.0, -1.27)),
            line((-1.27, -1.27), (1.27, -1.27)),
            line((-1.27, -1.27), (-1.905, -2.54)),
            line((0.0, -1.27), (-0.635, -2.54)),
            line((1.27, -1.27), (0.635, -2.54)),
        ],
        PowerStyle::ProtectiveEarth => vec![
            line((0.0, 0.0), (0.0, -1.27)),
            line((-1.27, -1.27), (1.27, -1.27)),
            line((-0.762, -1.778), (0.762, -1.778)),
            line((-0.254, -2.286), (0.254, -2.286)),
            circle((0.0, -1.778), 2.032),
        ],
    }
}

/// A power symbol: a hidden power input pin named like the net, which KiCad connects to
/// the global net of that name.
pub fn power_symbol(name: &str, net: &str, style: PowerStyle) -> X {
    let base = name.rsplit(':').next().unwrap_or(name);
    let value_y = if style.points_down() { -3.81 } else { 3.556 };
    let hidden = |k: &str, v: &str| property(k, v, l("at").num(0.0).num(0.0).num(0.0), true, font(1.27));
    l("symbol")
        .str(name)
        .push(l("power"))
        .push(l("pin_numbers").yes_no("hide", true))
        .push(l("pin_names").push(l("offset").num(0.0)).yes_no("hide", true))
        .yes_no("exclude_from_sim", false)
        .yes_no("in_bom", true)
        .yes_no("on_board", true)
        .push(hidden("Reference", "#PWR"))
        .push(property("Value", net, l("at").num(0.0).num(value_y).num(0.0), false, font(1.27)))
        .push(hidden("Footprint", ""))
        .push(hidden("Datasheet", ""))
        .push(hidden("Description", &format!("Power port {net}")))
        .push(l("symbol").str(format!("{base}_0_1")).extend(glyph(style)))
        .push(
            l("symbol").str(format!("{base}_1_1")).push(
                l("pin")
                    .sym("power_in")
                    .sym("line")
                    .push(l("at").num(0.0).num(0.0).num(if style.points_down() { 270.0 } else { 90.0 }))
                    .push(l("length").num(0.0))
                    .yes_no("hide", true)
                    .push(l("name").str(net).push(font(1.27)))
                    .push(l("number").str("1").push(font(1.27))),
            ),
        )
        .yes_no("embedded_fonts", false)
}

/// KiCad's PWR_FLAG: a power output pin, so ERC sees the net driven.
pub fn flag_symbol(name: &str) -> X {
    let base = name.rsplit(':').next().unwrap_or(name);
    let hidden = |k: &str, v: &str| property(k, v, l("at").num(0.0).num(0.0).num(0.0), true, font(1.27));
    l("symbol")
        .str(name)
        .push(l("power"))
        .push(l("pin_numbers").yes_no("hide", true))
        .push(l("pin_names").push(l("offset").num(0.0)).yes_no("hide", true))
        .yes_no("exclude_from_sim", false)
        .yes_no("in_bom", true)
        .yes_no("on_board", true)
        .push(hidden("Reference", "#FLG"))
        .push(property("Value", "PWR_FLAG", l("at").num(0.0).num(3.81).num(0.0), false, font(1.27)))
        .push(hidden("Footprint", ""))
        .push(hidden("Datasheet", ""))
        .push(hidden("Description", "Special symbol for telling ERC where power comes from"))
        .push(
            l("symbol").str(format!("{base}_0_0")).push(
                l("pin")
                    .sym("power_out")
                    .sym("line")
                    .push(l("at").num(0.0).num(0.0).num(90.0))
                    .push(l("length").num(0.0))
                    .push(l("name").str("~").push(font(1.27)))
                    .push(l("number").str("1").push(font(1.27))),
            ),
        )
        .push(
            l("symbol").str(format!("{base}_0_1")).push(poly(
                &[(0.0, 0.0), (0.0, 1.27), (-1.016, 1.905), (0.0, 2.54), (1.016, 1.905), (0.0, 1.27)],
                false,
            )),
        )
        .yes_no("embedded_fonts", false)
}
