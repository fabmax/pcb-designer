//! Library items → KiCad: symbols (one per part variant, pins numbered by its pads) and
//! footprints, for the schematic's and board's embedded copies and the project library.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use model::*;

use super::sx::{X, l};
use crate::project::Report;

/// KiCad's name of the project library (symbols and footprints): the project's name.
pub struct Names {
    pub lib: String,
    /// (part, variant) → symbol name.
    symbols: HashMap<(Id, Id), String>,
    /// Footprint → name.
    footprints: HashMap<Id, String>,
}

/// A name KiCad takes as a library item name and a file name: no `:` (the library
/// separator), path separators, quotes or whitespace.
pub fn item_name(s: &str) -> String {
    let out: String = s
        .trim()
        .chars()
        .map(|c| if c.is_whitespace() || ":/\\\"*?<>|".contains(c) || c.is_control() { '_' } else { c })
        .collect();
    if out.is_empty() { "_".into() } else { out }
}

/// `base`, or `base_2`, `base_3`… if taken.
fn unique(base: String, taken: &mut BTreeSet<String>) -> String {
    let mut name = base.clone();
    let mut n = 2;
    while !taken.insert(name.to_ascii_lowercase()) {
        name = format!("{base}_{n}");
        n += 1;
    }
    name
}

impl Names {
    /// Names for every part variant and footprint of `library`. A part with one variant
    /// keeps its name; one with several gets a symbol per variant, `Part_Variant`.
    pub fn of(lib: &str, library: &Library) -> Names {
        let mut taken = BTreeSet::new();
        let mut symbols = HashMap::new();
        for part in library.parts.iter().filter(|p| !p.is_board_part()) {
            for v in &part.variants {
                let base = if part.variants.len() == 1 {
                    item_name(&part.name)
                } else {
                    item_name(&format!("{}_{}", part.name, v.name))
                };
                symbols.insert((part.id, v.id), unique(base, &mut taken));
            }
        }
        let mut taken = BTreeSet::new();
        let footprints = library.footprints.iter().map(|f| (f.id, unique(item_name(&f.name), &mut taken))).collect();
        Names { lib: item_name(lib), symbols, footprints }
    }

    pub fn symbol(&self, part: Id, variant: Id) -> Option<&str> {
        self.symbols.get(&(part, variant)).map(String::as_str)
    }

    pub fn footprint(&self, id: Id) -> Option<&str> {
        self.footprints.get(&id).map(String::as_str)
    }

    /// `Lib:Name` of a symbol.
    pub fn symbol_id(&self, part: Id, variant: Id) -> Option<String> {
        self.symbol(part, variant).map(|s| format!("{}:{s}", self.lib))
    }

    /// `Lib:Name` of a footprint.
    pub fn footprint_id(&self, id: Id) -> Option<String> {
        self.footprint(id).map(|s| format!("{}:{s}", self.lib))
    }
}

/// KiCad pin numbers per (unit, pin) of a symbol.
pub type PinNumbers = HashMap<(u32, Id), Vec<String>>;

/// KiCad pin numbers of a variant, per (unit, pin): the pad names the pin connects to
/// (the first is the visible pin, the others are stacked under it). A pin without pads
/// keeps its number unless a pad has that name.
pub fn pin_numbers(library: &Library, part: &Part, variant: &PartVariant) -> PinNumbers {
    let fp = variant.footprint.and_then(|f| library.footprints.get(f));
    let pad_names: HashMap<Id, &str> =
        fp.map(|f| f.pads.iter().map(|p| (p.id, p.name.as_str())).collect()).unwrap_or_default();
    let mut out = PinNumbers::new();
    let mut used: BTreeSet<String> = pad_names.values().map(|s| s.to_string()).collect();
    for c in &variant.connections {
        let Some(&name) = pad_names.get(&c.pad) else { continue };
        let v = out.entry((c.unit, c.pin)).or_default();
        if !v.iter().any(|n| n == name) {
            v.push(name.to_string());
        }
    }
    for (u, unit) in part.units.iter().enumerate() {
        let Some(sym) = library.symbols.get(unit.symbol) else { continue };
        for pin in sym.pins.iter() {
            let key = (u as u32, pin.id);
            if out.contains_key(&key) {
                continue;
            }
            let base = if pin.key().is_empty() { "~".to_string() } else { pin.key().to_string() };
            let mut number = base.clone();
            let mut n = 2;
            while !used.insert(number.clone()) {
                number = format!("{base}_{n}");
                n += 1;
            }
            out.insert(key, vec![number]);
        }
    }
    out
}

/// `(effects (font (size h h) …) (justify …))` of a text.
pub fn effects(t: &Text, mirror: bool, thickness: Option<Coord>) -> X {
    let mut font = l("font").push(l("size").mm(t.size).mm(t.size));
    if let Some(th) = thickness {
        font = font.push(l("thickness").mm(th));
    } else if t.bold {
        font = font.yes_no("bold", true);
    }
    let mut e = l("effects").push(font);
    let mut justify = l("justify");
    let mut any = false;
    match t.halign {
        HAlign::Left => (justify, any) = (justify.sym("left"), true),
        HAlign::Right => (justify, any) = (justify.sym("right"), true),
        HAlign::Center => {}
    }
    match t.valign {
        VAlign::Top => (justify, any) = (justify.sym("top"), true),
        VAlign::Bottom => (justify, any) = (justify.sym("bottom"), true),
        VAlign::Center => {}
    }
    if mirror {
        (justify, any) = (justify.sym("mirror"), true);
    }
    if any {
        e = e.push(justify);
    }
    e
}

fn stroke(width: Coord) -> X {
    l("stroke").push(l("width").mm(width)).push(l("type").sym("default"))
}

/// Start, mid and end of an arc.
pub fn arc_points(center: Point, radius: Coord, start: Angle, sweep: f64) -> [Point; 3] {
    let at = |deg: f64| {
        let r = deg.to_radians();
        Point::new(
            center.x + (radius as f64 * r.cos()).round() as Coord,
            center.y + (radius as f64 * r.sin()).round() as Coord,
        )
    };
    let s = start.degrees();
    [at(s), at(s + sweep / 2.0), at(s + sweep)]
}

/// `(xy x y)` in y-up (symbol) coordinates.
fn xy_up(p: Point) -> X {
    l("xy").mm(p.x).mm(p.y)
}

fn at_up(head: &str, p: Point) -> X {
    l(head).mm(p.x).mm(p.y)
}

/// A symbol graphic (y up, as ours).
fn symbol_graphic(s: &Shape) -> X {
    let fill = |f: bool| l("fill").push(l("type").sym(if f { "background" } else { "none" }));
    match s {
        Shape::Line { a, b, width } => {
            l("polyline").push(l("pts").push(xy_up(*a)).push(xy_up(*b))).push(stroke(*width)).push(fill(false))
        }
        Shape::Arc { center, radius, start, sweep, width } => {
            let [s, m, e] = arc_points(*center, *radius, *start, *sweep);
            l("arc")
                .push(at_up("start", s))
                .push(at_up("mid", m))
                .push(at_up("end", e))
                .push(stroke(*width))
                .push(fill(false))
        }
        Shape::Circle { center, radius, width, fill: f } => {
            l("circle").push(at_up("center", *center)).push(l("radius").mm(*radius)).push(stroke(*width)).push(fill(*f))
        }
        Shape::Rect { a, b, width, fill: f } => {
            l("rectangle").push(at_up("start", *a)).push(at_up("end", *b)).push(stroke(*width)).push(fill(*f))
        }
        Shape::Polygon { points, width, fill: f } => {
            let mut pts = l("pts").extend(points.iter().map(|p| xy_up(*p)));
            if let Some(first) = points.first() {
                pts = pts.push(xy_up(*first));
            }
            l("polyline").push(pts).push(stroke(*width)).push(fill(*f))
        }
    }
}

fn pin_type(k: PinKind) -> &'static str {
    match k {
        PinKind::Input => "input",
        PinKind::Output => "output",
        PinKind::Bidirectional => "bidirectional",
        PinKind::TriState => "tri_state",
        PinKind::Passive => "passive",
        PinKind::PowerIn => "power_in",
        PinKind::PowerOut => "power_out",
        PinKind::OpenCollector => "open_collector",
        PinKind::NotConnected => "no_connect",
    }
}

fn small_font() -> X {
    l("effects").push(l("font").push(l("size").num(1.27).num(1.27)))
}

/// A symbol property: `(property "Key" "value" (at …) [(hide yes)] (effects …))`.
pub fn property(key: &str, value: &str, at: X, hidden: bool, effects: X) -> X {
    let mut p = l("property").str(key).str(value).push(at);
    if hidden {
        p = p.yes_no("hide", true);
    }
    p.push(effects)
}

fn hidden_property(key: &str, value: &str) -> X {
    property(key, value, l("at").num(0.0).num(0.0).num(0.0), true, small_font())
}

/// What a part variant becomes in KiCad.
pub struct SymbolDef<'a> {
    pub library: &'a Library,
    pub part: &'a Part,
    pub variant: &'a PartVariant,
    /// The symbol's name: `Lib:Name` in a schematic, `Name` in a library file.
    pub name: String,
    /// `Lib:Name` of its footprint.
    pub footprint: Option<String>,
}

/// The symbol, as in a `.kicad_sym` or a schematic's `lib_symbols`.
pub fn symbol(d: &SymbolDef, report: &mut Report) -> X {
    let (library, part) = (d.library, d.part);
    let numbers = pin_numbers(library, part, d.variant);
    let units: Vec<&Symbol> = part.units.iter().filter_map(|u| library.symbols.get(u.symbol)).collect();
    let pins = || units.iter().flat_map(|s| s.pins.iter());
    let names_shown = pins().any(|p| p.show_name);
    let numbers_shown = pins().any(|p| p.show_number);
    if pins().any(|p| !p.show_name && p.name != p.number) && names_shown {
        report.note("pin names hidden on single pins are shown in KiCad (it hides them per symbol)");
    }
    let mut s = l("symbol").str(&d.name);
    if !numbers_shown {
        s = s.push(l("pin_numbers").yes_no("hide", true));
    }
    let mut pn = l("pin_names").push(l("offset").num(0.508));
    if !names_shown {
        pn = pn.yes_no("hide", true);
    }
    s = s
        .push(pn)
        .yes_no("exclude_from_sim", false)
        .yes_no("in_bom", true)
        .yes_no("on_board", d.variant.footprint.is_some());

    // Reference and value where the first unit shows them.
    let field = |placeholder: &str| units.first().and_then(|u| u.texts.iter().find(|t| t.text == placeholder));
    let at_field = |t: Option<&Text>| match t {
        Some(t) => (l("at").mm(t.pos.x).mm(t.pos.y).num(t.rotation.degrees()), false, effects(t, false, None)),
        None => (l("at").num(0.0).num(0.0).num(0.0), true, small_font()),
    };
    let (at, hide, fx) = at_field(field("${REF}"));
    s = s.push(property("Reference", &part.prefix, at, hide, fx));
    let value = if part.default_value.is_empty() { part.name.as_str() } else { part.default_value.as_str() };
    let (at, hide, fx) = at_field(field("${VALUE}"));
    s = s.push(property("Value", value, at, hide, fx));
    s = s.push(hidden_property("Footprint", d.footprint.as_deref().unwrap_or("")));
    s = s.push(hidden_property("Datasheet", part.properties.get("Datasheet").map_or("", String::as_str)));
    s = s.push(hidden_property("Description", &part.description));
    for (k, v) in &part.properties {
        if !["Datasheet", "Reference", "Value", "Footprint", "Description"].contains(&k.as_str()) {
            s = s.push(hidden_property(k, v));
        }
    }

    let base = d.name.rsplit(':').next().unwrap_or(&d.name).to_string();
    for (u, unit) in part.units.iter().enumerate() {
        let Some(sym) = library.symbols.get(unit.symbol) else { continue };
        let mut sub = l("symbol").str(format!("{base}_{}_1", u + 1));
        if part.units.len() > 1 && !unit.name.is_empty() {
            sub = sub.push(l("unit_name").str(&unit.name));
        }
        for g in &sym.graphics {
            sub = sub.push(symbol_graphic(g));
        }
        for t in sym.texts.iter().filter(|t| !is_field_text(&t.text)) {
            // Library texts turn in tenths of a degree.
            sub = sub.push(
                l("text")
                    .str(field_vars(&t.text))
                    .push(l("at").mm(t.pos.x).mm(t.pos.y).num(t.rotation.degrees() * 10.0))
                    .push(effects(t, false, None)),
            );
        }
        for pin in sym.pins.iter() {
            let nums = numbers.get(&(u as u32, pin.id)).cloned().unwrap_or_else(|| vec![pin.key().to_string()]);
            let quarter = (pin.direction.degrees() / 90.0).round() * 90.0;
            if (quarter - pin.direction.degrees()).abs() > 1e-6 {
                report.skip("pins at angles other than multiples of 90° (turned to the nearest)");
            }
            for (i, number) in nums.iter().enumerate() {
                // Stacked pins under the first are hidden passive ones, as KiCad's
                // libraries make them.
                let stacked = i > 0;
                let kind = if stacked { "passive" } else { pin_type(pin.kind) };
                let mut p = l("pin")
                    .sym(kind)
                    .sym("line")
                    .push(l("at").mm(pin.pos.x).mm(pin.pos.y).num(quarter.rem_euclid(360.0)))
                    .push(l("length").mm(pin.length));
                if stacked {
                    p = p.yes_no("hide", true);
                }
                let name = if pin.name.is_empty() { "~" } else { pin.name.as_str() };
                p = p.push(l("name").str(name).push(small_font())).push(l("number").str(number).push(small_font()));
                sub = sub.push(p);
            }
            if nums.len() > 1 {
                report.note("pins on several pads became stacked pins, one per pad (the others hidden)");
            }
        }
        s = s.push(sub);
    }
    s.yes_no("embedded_fonts", false)
}

/// Our text variables in KiCad's spelling.
pub fn field_vars(s: &str) -> String {
    s.replace("${REF}", "${REFERENCE}")
}

// ---------------------------------------------------------------------------------------
// Footprints

/// KiCad name of a footprint layer (front side).
pub fn fp_layer(layer: FootprintLayer, back: bool) -> &'static str {
    match (layer, back) {
        (FootprintLayer::Copper, false) => "F.Cu",
        (FootprintLayer::Copper, true) => "B.Cu",
        (FootprintLayer::Silkscreen, false) => "F.SilkS",
        (FootprintLayer::Silkscreen, true) => "B.SilkS",
        (FootprintLayer::SolderMask, false) => "F.Mask",
        (FootprintLayer::SolderMask, true) => "B.Mask",
        (FootprintLayer::SolderPaste, false) => "F.Paste",
        (FootprintLayer::SolderPaste, true) => "B.Paste",
        (FootprintLayer::Fabrication, false) => "F.Fab",
        (FootprintLayer::Fabrication, true) => "B.Fab",
        (FootprintLayer::Courtyard, false) => "F.CrtYd",
        (FootprintLayer::Courtyard, true) => "B.CrtYd",
    }
}

/// How footprint content is written: KiCad footprints are y-down and stored as placed
/// (pad and text angles absolute, back-side footprints mirrored with their layers
/// swapped).
#[derive(Clone, Copy)]
pub struct Place {
    /// The footprint's KiCad orientation (degrees).
    pub orient: f64,
    pub back: bool,
}

impl Place {
    pub const LIBRARY: Place = Place { orient: 0.0, back: false };

    /// A local point (ours, y up) in KiCad's footprint coordinates.
    pub fn xy(&self, head: &str, p: Point) -> X {
        // Front: y negated (y down). Back: mirrored back again.
        let y = if self.back { p.y } else { -p.y };
        l(head).mm(p.x).mm(y)
    }

    /// A local angle (ours) as KiCad's absolute one.
    pub fn angle(&self, a: f64) -> f64 {
        let a = if self.back { self.orient - a } else { self.orient + a };
        a.rem_euclid(360.0)
    }

    pub fn layer(&self, layer: FootprintLayer, back: bool) -> &'static str {
        fp_layer(layer, back != self.back)
    }

    /// An `(at x y [angle])` of a pad or text.
    pub fn at(&self, p: Point, angle: f64) -> X {
        let a = self.angle(angle);
        let x = self.xy("at", p);
        if a.abs() > 1e-9 { x.num(a) } else { x }
    }
}

fn fill_yes_no(f: bool) -> X {
    l("fill").sym(if f { "yes" } else { "no" })
}

fn fp_stroke(width: Coord) -> X {
    l("stroke").push(l("width").mm(width)).push(l("type").sym("solid"))
}

/// A graphic in footprint (`fp_`) or board (`gr_`) form. `xy` maps a point.
pub fn graphic(prefix: &str, s: &Shape, xy: &dyn Fn(&str, Point) -> X) -> X {
    let h = |n: &str| format!("{prefix}_{n}");
    match s {
        Shape::Line { a, b, width } => l(&h("line")).push(xy("start", *a)).push(xy("end", *b)).push(fp_stroke(*width)),
        Shape::Arc { center, radius, start, sweep, width } => {
            let [s, m, e] = arc_points(*center, *radius, *start, *sweep);
            l(&h("arc")).push(xy("start", s)).push(xy("mid", m)).push(xy("end", e)).push(fp_stroke(*width))
        }
        Shape::Circle { center, radius, width, fill } => l(&h("circle"))
            .push(xy("center", *center))
            .push(xy("end", *center + Point::new(*radius, 0)))
            .push(fp_stroke(*width))
            .push(fill_yes_no(*fill)),
        Shape::Rect { a, b, width, fill } => {
            l(&h("rect")).push(xy("start", *a)).push(xy("end", *b)).push(fp_stroke(*width)).push(fill_yes_no(*fill))
        }
        Shape::Polygon { points, width, fill } => l(&h("poly"))
            .push(l("pts").extend(points.iter().map(|p| xy("xy", *p))))
            .push(fp_stroke(*width))
            .push(fill_yes_no(*fill)),
    }
}

/// Stroke thickness KiCad draws a text of this height with.
pub fn thickness(size: Coord) -> Coord {
    (size as f64 * 0.15).round() as Coord
}

/// What a placed footprint adds: its fields, pad nets and attributes.
pub struct Placed<'a> {
    /// Where it is, in KiCad's board coordinates (nm, y down).
    pub at: (Coord, Coord),
    pub id: Id,
    pub refdes: &'a str,
    pub value: &'a str,
    /// The instance's reference and value texts (footprint coordinates).
    pub fields: Vec<&'a FootprintField>,
    /// Pad id → (KiCad net number, name).
    pub nets: HashMap<Id, (usize, String)>,
    /// `/sheet…/symbol` of its symbol (`None` for board parts).
    pub path: Option<String>,
    pub sheet_name: String,
    pub sheet_file: String,
    pub locked: bool,
    pub dnp: bool,
    pub in_bom: bool,
    pub properties: &'a BTreeMap<String, String>,
}

/// A footprint: placed on the board (`placed`), or as a library file (`None`).
pub fn footprint(fp: &Footprint, lib_id: &str, place: Place, placed: Option<&Placed>, report: &mut Report) -> X {
    let mut x = l("footprint").str(lib_id);
    if placed.is_some_and(|p| p.locked) {
        x = x.sym("locked");
    }
    x = x.push(l("layer").str(if place.back { "B.Cu" } else { "F.Cu" }));
    let uuid_of = |what: &str| -> String {
        let base = placed.map_or(fp.id, |p| p.id);
        crate::common::kid(&["kicad-export", &base.to_string(), what]).to_string()
    };
    if let Some(p) = placed {
        x = x.push(l("uuid").str(p.id.to_string()));
        x = x.push(l("at").mm(p.at.0).mm(p.at.1).num(place.orient.rem_euclid(360.0)));
    } else {
        x = x.push(l("version").sym("20241229")).push(l("generator").str("pcb-designer"));
    }
    if !fp.description.is_empty() {
        x = x.push(l("descr").str(&fp.description));
    }

    // Reference and value: the instance's own texts, else the definition's templates.
    for (key, placeholder, default_layer) in
        [("Reference", "${REF}", FootprintLayer::Silkscreen), ("Value", "${VALUE}", FootprintLayer::Fabrication)]
    {
        let own = placed.and_then(|p| p.fields.iter().find(|f| f.text.text == placeholder));
        let template = fp.texts.iter().find(|t| t.text.text == placeholder);
        let (layer, back, text, hidden) = match (own, template) {
            (Some(f), _) => (f.layer, f.back, f.text.clone(), f.hidden),
            (None, Some(t)) => (t.layer, t.back, t.text.clone(), false),
            (None, None) => (default_layer, false, Text::new("", Point::ZERO, 1_000_000), true),
        };
        let value = match (placed, key) {
            (Some(p), "Reference") => p.refdes.to_string(),
            (Some(p), _) => p.value.to_string(),
            (None, "Reference") => "REF**".to_string(),
            (None, _) => fp.name.clone(),
        };
        let mut prop = l("property").str(key).str(value).push(place.at(text.pos, text.rotation.degrees()));
        prop = prop.push(l("layer").str(place.layer(layer, back)));
        if hidden {
            prop = prop.yes_no("hide", true);
        }
        prop = prop.push(l("uuid").str(uuid_of(key)));
        prop = prop.push(effects(&text, place.back != back, Some(thickness(text.size))));
        x = x.push(prop);
    }
    if let Some(p) = placed {
        for (k, v) in p.properties {
            let at = l("at").num(0.0).num(0.0).num(0.0);
            x = x.push(
                l("property")
                    .str(k)
                    .str(v)
                    .push(at)
                    .push(l("layer").str(place.layer(FootprintLayer::Fabrication, false)))
                    .yes_no("hide", true)
                    .push(l("uuid").str(uuid_of(&format!("prop:{k}"))))
                    .push(effects(&Text::new("", Point::ZERO, 1_000_000), false, Some(150_000))),
            );
        }
        if let Some(path) = &p.path {
            x = x.push(l("path").str(path));
            x = x.push(l("sheetname").str(&p.sheet_name)).push(l("sheetfile").str(&p.sheet_file));
        }
    }
    let through = fp.pads.iter().any(|p| matches!(p.kind, PadKind::ThroughHole { .. }));
    let smd = fp.pads.iter().any(|p| p.kind == PadKind::Smd);
    let mut attr = l("attr");
    attr = if through {
        attr.sym("through_hole")
    } else if smd {
        attr.sym("smd")
    } else {
        attr
    };
    if let Some(p) = placed {
        if p.path.is_none() {
            attr = attr.sym("board_only");
        }
        if !p.in_bom {
            attr = attr.sym("exclude_from_bom");
        }
        if p.dnp {
            attr = attr.sym("dnp");
        }
    }
    x = x.push(attr);

    let xy = |head: &str, p: Point| place.xy(head, p);
    for (i, g) in fp.graphics.iter().enumerate() {
        x = x.push(
            graphic("fp", &g.shape, &xy)
                .push(l("layer").str(place.layer(g.layer, g.back)))
                .push(l("uuid").str(uuid_of(&format!("g{i}")))),
        );
    }
    for (i, t) in fp.texts.iter().enumerate().filter(|(_, t)| !t.is_field()) {
        x = x.push(
            l("fp_text")
                .sym("user")
                .str(field_vars(&t.text.text))
                .push(place.at(t.text.pos, t.text.rotation.degrees()))
                .push(l("layer").str(place.layer(t.layer, t.back)))
                .push(l("uuid").str(uuid_of(&format!("t{i}"))))
                .push(effects(&t.text, place.back != t.back, Some(thickness(t.text.size)))),
        );
    }
    for pad in fp.pads.iter() {
        x = x.push(pad_x(pad, place, placed, &uuid_of(&format!("pad:{}", pad.id)), report));
    }
    x.yes_no("embedded_fonts", false)
}

fn pad_x(pad: &Pad, place: Place, placed: Option<&Placed>, uuid: &str, report: &mut Report) -> X {
    let (kind, layers): (&str, Vec<String>) = match pad.kind {
        PadKind::Smd => {
            let mut v = vec![fp_layer(FootprintLayer::Copper, pad.back != place.back).to_string()];
            if pad.mask {
                v.push(fp_layer(FootprintLayer::SolderMask, pad.back != place.back).into());
            }
            if pad.paste {
                v.push(fp_layer(FootprintLayer::SolderPaste, pad.back != place.back).into());
            }
            ("smd", v)
        }
        PadKind::ThroughHole { .. } => {
            ("thru_hole", if pad.mask { vec!["*.Cu".into(), "*.Mask".into()] } else { vec!["*.Cu".into()] })
        }
        PadKind::NonPlatedHole { .. } => ("np_thru_hole", vec!["*.Cu".into(), "*.Mask".into()]),
    };
    let shape = match pad.shape {
        PadShape::Circle => "circle",
        PadShape::Rect => "rect",
        PadShape::RoundRect { .. } => "roundrect",
        PadShape::Oval => "oval",
        PadShape::Polygon { .. } => "custom",
    };
    let mut x = l("pad").str(&pad.name).sym(kind).sym(shape).push(place.at(pad.pos, pad.rotation.degrees()));
    let size = match &pad.shape {
        PadShape::Circle => Size::square(pad.size.w),
        PadShape::Polygon { points } => {
            let d = anchor_diameter(points);
            if d == 0 {
                report.skip("polygon pads whose origin is outside their outline (anchored at a tiny circle)");
            }
            Size::square(d.max(10_000))
        }
        _ => pad.size,
    };
    x = x.push(l("size").mm(size.w).mm(size.h));
    if let Some(d) = pad.kind.drill() {
        x = x.push(if d.w == d.h { l("drill").mm(d.w) } else { l("drill").sym("oval").mm(d.w).mm(d.h) });
    }
    x = x.push(l("layers").extend(layers.into_iter().map(X::Str)));
    if let PadShape::RoundRect { ratio } = pad.shape {
        x = x.push(l("roundrect_rratio").num(ratio));
    }
    if let Some((n, name)) = placed.and_then(|p| p.nets.get(&pad.id)) {
        x = x.push(l("net").sym(n.to_string()).str(name));
    }
    if let Some(m) = pad.mask_margin {
        x = x.push(l("solder_mask_margin").mm(m));
    }
    if let Some(m) = pad.paste_margin {
        x = x.push(l("solder_paste_margin").mm(m));
    }
    if let PadShape::Polygon { points } = &pad.shape {
        // Primitives are relative to the pad and turn with it, y down; mirrored like the
        // rest of a back-side footprint.
        let y = |p: &Point| if place.back { p.y } else { -p.y };
        let pts = l("pts").extend(points.iter().map(|p| l("xy").mm(p.x).mm(y(p))));
        x = x
            .push(l("options").push(l("clearance").sym("outline")).push(l("anchor").sym("circle")))
            .push(l("primitives").push(l("gr_poly").push(pts).push(l("width").mm(0)).push(fill_yes_no(true))));
    }
    x.push(l("uuid").str(uuid))
}

/// The largest circle around the origin inside a polygon (with a margin), or 0 when
/// the origin is outside it.
fn anchor_diameter(points: &[Point]) -> Coord {
    let n = points.len();
    if n < 3 {
        return 0;
    }
    let mut inside = false;
    let mut dist = f64::MAX;
    for i in 0..n {
        let (a, b) = (points[i], points[(i + 1) % n]);
        if (a.y > 0) != (b.y > 0) {
            let x = a.x as f64 + (0 - a.y) as f64 * (b.x - a.x) as f64 / (b.y - a.y) as f64;
            if x > 0.0 {
                inside = !inside;
            }
        }
        dist = dist.min(editor_core::copper::point_seg_dist(Point::ZERO, a, b));
    }
    if !inside {
        return 0;
    }
    ((dist * 2.0 * 0.9) as Coord).min(1_000_000)
}

/// A footprint library file (`.kicad_mod`).
pub fn footprint_file(fp: &Footprint, name: &str, report: &mut Report) -> String {
    footprint(fp, name, Place::LIBRARY, None, report).to_text()
}

/// A symbol library file (`.kicad_sym`) of every part variant of `library`.
pub fn symbol_library(library: &Library, names: &Names, report: &mut Report) -> String {
    let mut x = l("kicad_symbol_lib")
        .push(l("version").sym("20241209"))
        .push(l("generator").str("pcb-designer"))
        .push(l("generator_version").str("9.0"));
    for part in library.parts.iter().filter(|p| !p.is_board_part()) {
        for v in &part.variants {
            let Some(name) = names.symbol(part.id, v.id) else { continue };
            let footprint = v.footprint.and_then(|f| names.footprint_id(f));
            let d = SymbolDef { library, part, variant: v, name: name.to_string(), footprint };
            x = x.push(symbol(&d, report));
        }
    }
    x.to_text()
}
