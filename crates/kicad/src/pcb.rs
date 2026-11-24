//! KiCad boards (`.kicad_pcb`, KiCad 6–10) → our board.
//!
//! Board coordinates are y-down in KiCad and y-up here: y is negated, angles keep their
//! value (counterclockwise on screen in both). Footprints are stored placed in KiCad:
//! pad and text angles include the footprint's rotation, and footprints on the back
//! are mirrored (local y negated, front and back layers swapped). Both are undone so
//! every footprint becomes a top-side definition placed by its instance. KiCad's drill
//! and place origin (`aux_axis_origin`) becomes our origin.

use std::collections::HashMap;
use std::path::Path;

use model::*;

use crate::common::*;
use crate::project::Report;
use crate::sch::parse_id;
use crate::sexpr::{self, Node};

/// Oldest supported board format (KiCad 6.0).
const MIN_VERSION: u64 = 20211014;

/// A footprint placed on the KiCad board.
#[derive(Clone, Debug)]
pub struct PcbFootprint {
    pub refdes: String,
    pub value: String,
    /// `Lib:Name`.
    pub lib_id: String,
    /// Its definition (top side, unrotated), reference and value templates included.
    pub def: Footprint,
    /// Placement; `component` is set by the project import.
    pub instance: FootprintInstance,
    /// Its reference and value texts (`footprint` is the instance).
    pub fields: Vec<FootprintField>,
    /// Pad name → KiCad net name.
    pub pad_nets: Vec<(String, String)>,
    /// KiCad's "board only" attribute (no symbol).
    pub board_only: bool,
}

#[derive(Default)]
pub struct PcbImport {
    pub copper_layers: u16,
    pub footprints: Vec<PcbFootprint>,
    pub tracks: Vec<Track>,
    pub vias: Vec<Via>,
    pub zones: Vec<Zone>,
    pub keepouts: Vec<Keepout>,
    pub graphics: Vec<BoardGraphic>,
    pub texts: Vec<BoardText>,
    pub mask_expansion: Option<Coord>,
    pub paste_margin: Option<Coord>,
}

pub fn import(path: &Path, report: &mut Report) -> Result<PcbImport, String> {
    let source = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let root = sexpr::parse(&source).map_err(|e| format!("{}: {e}", path.display()))?;
    if root.head() != "kicad_pcb" {
        return Err(format!("{}: not a KiCad board", path.display()));
    }
    let version = root.child("version").and_then(|v| v.num(0)).unwrap_or(0.0) as u64;
    if version < MIN_VERSION {
        return Err(format!(
            "{}: board format {version} is older than KiCad 6; open and save it in KiCad 6 or newer first",
            path.display()
        ));
    }
    let mut out = PcbImport::default();
    let layers = Layers::of(&root);
    out.copper_layers = layers.copper.max(2);
    // KiCad's drill/place origin (fabrication outputs are relative to it) becomes ours.
    let mut origin = Point::ZERO;
    if let Some(setup) = root.child("setup") {
        out.mask_expansion = setup.child("pad_to_mask_clearance").and_then(|n| n.num(0)).map(nm);
        out.paste_margin = setup.child("pad_to_paste_clearance").and_then(|n| n.num(0)).map(nm);
        origin = child_point(setup, "aux_axis_origin", true).unwrap_or(Point::ZERO);
    }
    let mut ids = Ids::default();
    for item in root.lists() {
        match item.head() {
            "footprint" | "module" => {
                if let Some(f) = footprint(item, &layers, &mut ids, report) {
                    out.footprints.push(f);
                }
            }
            "segment" | "arc" => {
                let (Some(a), Some(b)) = (child_point(item, "start", true), child_point(item, "end", true)) else {
                    continue;
                };
                let Some(layer) = item.child("layer").and_then(|l| l.arg(0)).and_then(|l| layers.copper(l)) else {
                    continue;
                };
                let width = item.child("width").and_then(|w| w.num(0)).map_or(250_000, nm);
                let mid = if item.head() == "arc" { child_point(item, "mid", true) } else { None };
                let locked = item.flag("locked");
                out.tracks.push(Track { id: ids.of(item), layer, a, b, width, mid, locked });
            }
            "via" => {
                let Some(pos) = child_point(item, "at", true) else { continue };
                let names: Vec<&str> = item
                    .child("layers")
                    .map(|l| l.items().iter().skip(1).filter_map(Node::as_atom).collect())
                    .unwrap_or_default();
                let span: Vec<LayerId> = names.iter().filter_map(|n| layers.copper(n)).collect();
                let (from, to) = match (span.iter().min(), span.iter().max()) {
                    (Some(&a), Some(&b)) if a != b => (a, b),
                    _ => (LayerId::TOP_COPPER, LayerId::BOTTOM_COPPER),
                };
                out.vias.push(Via {
                    id: ids.of(item),
                    pos,
                    diameter: item.child("size").and_then(|s| s.num(0)).map_or(600_000, nm),
                    drill: item.child("drill").and_then(|s| s.num(0)).map_or(300_000, nm),
                    from,
                    to,
                    locked: item.flag("locked"),
                });
            }
            "zone" => zone(item, &layers, &mut ids, &mut out, report),
            "gr_line" | "gr_rect" | "gr_circle" | "gr_arc" | "gr_poly" | "gr_curve" | "gr_bbox" => {
                let name = item.child("layer").and_then(|l| l.arg(0)).unwrap_or("");
                let Some(layer) = layers.board(name) else {
                    report.skip("board graphics on documentation and user layers");
                    continue;
                };
                for shape in crate::footprint::graphic(item, Point::ZERO, Angle::ZERO) {
                    out.graphics.push(BoardGraphic { id: ids.next(item), layer, shape });
                }
            }
            "gr_text" => {
                let name = item.child("layer").and_then(|l| l.arg(0)).unwrap_or("");
                let Some(layer) = layers.board(name) else {
                    report.skip("board texts on documentation and user layers");
                    continue;
                };
                let content = plain(item.arg(0).unwrap_or(""));
                if let Some(at) = item.child("at")
                    && let Some(mut text) = text(content, at, item.child("effects"), true, 1.0)
                {
                    text.bold = true;
                    out.texts.push(BoardText { id: ids.of(item), layer, text });
                }
            }
            "gr_text_box" => report.skip("board text boxes"),
            "dimension" => report.skip("dimensions"),
            "group" => report.skip("groups (their members are imported)"),
            "image" => report.skip("board images"),
            "target" => report.skip("alignment targets"),
            "generated" => report.skip("generated objects (tuning patterns; their tracks are imported)"),
            "table" => report.skip("board tables"),
            _ => {}
        }
    }
    if origin != Point::ZERO {
        out.translate(Point::ZERO - origin);
    }
    Ok(out)
}

impl PcbImport {
    /// Move everything on the board by `d`.
    fn translate(&mut self, d: Point) {
        let t = Transform::new(d, Angle::ZERO, false);
        for f in &mut self.footprints {
            f.instance.pos = f.instance.pos + d;
        }
        for tr in &mut self.tracks {
            tr.a = tr.a + d;
            tr.b = tr.b + d;
            tr.mid = tr.mid.map(|m| m + d);
        }
        for v in &mut self.vias {
            v.pos = v.pos + d;
        }
        let shift = |pts: &mut Vec<Point>| pts.iter_mut().for_each(|p| *p = *p + d);
        for z in &mut self.zones {
            shift(&mut z.outline);
            z.holes.iter_mut().for_each(shift);
        }
        for k in &mut self.keepouts {
            shift(&mut k.outline);
            k.holes.iter_mut().for_each(shift);
        }
        for g in &mut self.graphics {
            g.shape = g.shape.transformed(&t);
        }
        for x in &mut self.texts {
            x.text.pos = x.text.pos + d;
        }
    }
}

/// Ids from KiCad UUIDs; repeated or missing ones get fresh ids.
#[derive(Default)]
struct Ids {
    seen: std::collections::HashSet<Id>,
}

impl Ids {
    fn of(&mut self, n: &Node) -> Id {
        let id = n.child("uuid").or_else(|| n.child("tstamp")).and_then(|u| u.arg(0)).and_then(parse_id);
        match id {
            Some(id) if self.seen.insert(id) => id,
            _ => Id::new(),
        }
    }

    /// Like [`Ids::of`], for the second and later objects made from one KiCad object.
    fn next(&mut self, n: &Node) -> Id {
        self.of(n)
    }
}

/// The board's layer table.
struct Layers {
    copper: u16,
    /// Inner layer names by number (`In1.Cu` → 1).
    inner: HashMap<String, u16>,
}

impl Layers {
    fn of(root: &Node) -> Layers {
        let mut names: Vec<String> = Vec::new();
        if let Some(l) = root.child("layers") {
            for e in l.lists() {
                if let Some(name) = e.arg(0)
                    && name.ends_with(".Cu")
                {
                    names.push(name.to_string());
                }
            }
        }
        let mut inner: Vec<(u16, String)> = names
            .iter()
            .filter_map(|n| Some((n.strip_prefix("In")?.strip_suffix(".Cu")?.parse().ok()?, n.clone())))
            .collect();
        inner.sort();
        Layers {
            copper: names.len() as u16,
            inner: inner.into_iter().enumerate().map(|(i, (_, n))| (n, i as u16 + 1)).collect(),
        }
    }

    fn copper(&self, name: &str) -> Option<LayerId> {
        match name {
            "F.Cu" => Some(LayerId::TOP_COPPER),
            "B.Cu" => Some(LayerId::BOTTOM_COPPER),
            _ => self.inner.get(name).map(|&n| LayerId::inner(n)),
        }
    }

    /// Board layer of a KiCad layer name (copper, technical layers and the outline).
    fn board(&self, name: &str) -> Option<LayerId> {
        Some(match name {
            "F.SilkS" | "F.Silkscreen" => LayerId::TOP_SILK,
            "B.SilkS" | "B.Silkscreen" => LayerId::BOTTOM_SILK,
            "F.Mask" => LayerId::TOP_MASK,
            "B.Mask" => LayerId::BOTTOM_MASK,
            "F.Paste" => LayerId::TOP_PASTE,
            "B.Paste" => LayerId::BOTTOM_PASTE,
            "F.Fab" => LayerId::TOP_FAB,
            "B.Fab" => LayerId::BOTTOM_FAB,
            "F.CrtYd" | "F.Courtyard" => LayerId::TOP_COURTYARD,
            "B.CrtYd" | "B.Courtyard" => LayerId::BOTTOM_COURTYARD,
            "Edge.Cuts" => LayerId::OUTLINE,
            other => return self.copper(other),
        })
    }

    /// Copper layers of a `(layers …)` list (`*.Cu`, `F&B.Cu` included).
    fn copper_list(&self, names: &[&str]) -> Vec<LayerId> {
        let mut out = Vec::new();
        for n in names {
            match *n {
                "*.Cu" => return Vec::new(),
                "F&B.Cu" => out.extend([LayerId::TOP_COPPER, LayerId::BOTTOM_COPPER]),
                other => out.extend(self.copper(other)),
            }
        }
        out.sort();
        out.dedup();
        out
    }
}

fn atoms(n: Option<&Node>) -> Vec<&str> {
    n.map(|l| l.items().iter().skip(1).filter_map(Node::as_atom).collect()).unwrap_or_default()
}

fn zone(item: &Node, layers: &Layers, ids: &mut Ids, out: &mut PcbImport, report: &mut Report) {
    let mut names = atoms(item.child("layers"));
    names.extend(atoms(item.child("layer")));
    let copper = layers.copper_list(&names);
    let all = names.contains(&"*.Cu");
    // Outlines; KiCad stores holes as further polygons inside an outline.
    let polys: Vec<Vec<Point>> = item.children("polygon").map(|p| pts(p, true)).filter(|p| p.len() >= 3).collect();
    let mut outlines: Vec<(Vec<Point>, Vec<Vec<Point>>)> = Vec::new();
    for (i, p) in polys.iter().enumerate() {
        let inside = polys.iter().enumerate().any(|(j, q)| j != i && point_in(p[0], q) && !point_in(q[0], p));
        if !inside {
            outlines.push((p.clone(), Vec::new()));
        }
    }
    for (i, p) in polys.iter().enumerate() {
        if let Some(o) = outlines.iter_mut().find(|(o, _)| *o != *p && point_in(p[0], o)) {
            let _ = i;
            o.1.push(p.clone());
        }
    }
    if outlines.is_empty() {
        return;
    }
    if let Some(k) = item.child("keepout") {
        let forbidden = |what: &str| k.child(what).and_then(|c| c.arg(0)) == Some("not_allowed");
        for (outline, holes) in outlines {
            out.keepouts.push(Keepout {
                id: ids.next(item),
                layers: if all { Vec::new() } else { copper.clone() },
                outline,
                holes,
                no_tracks: forbidden("tracks"),
                no_vias: forbidden("vias"),
                no_pads: forbidden("pads"),
                no_pour: forbidden("copperpour"),
                no_footprints: forbidden("footprints"),
            });
        }
        return;
    }
    let net = plain(
        item.child("net_name")
            .and_then(|n| n.arg(0))
            .or_else(|| item.child("net").and_then(|n| n.arg(1)))
            .unwrap_or(""),
    );
    let connect = item.child("connect_pads");
    let connection = match connect.and_then(|c| c.arg(0)) {
        Some("yes") => ZoneConnection::Solid,
        Some("no") => ZoneConnection::None,
        Some("thru_hole_only") => ZoneConnection::ThermalThroughHole,
        _ => ZoneConnection::Thermal,
    };
    let clearance = connect.and_then(|c| c.child("clearance")).and_then(|c| c.num(0)).map(nm);
    let fill = item.child("fill");
    let thermal_gap = fill.and_then(|f| f.child("thermal_gap")).and_then(|c| c.num(0)).map(nm);
    let thermal_spoke = fill.and_then(|f| f.child("thermal_bridge_width")).and_then(|c| c.num(0)).map(nm);
    let islands = match fill.and_then(|f| f.child("island_removal_mode")).and_then(|c| c.num(0)).map(|m| m as i64) {
        Some(1) => IslandRemoval::Never,
        Some(2) => {
            let area = fill.and_then(|f| f.child("island_area_min")).and_then(|c| c.num(0)).unwrap_or(10.0);
            IslandRemoval::BelowArea((area * 1e12).round() as i64)
        }
        _ => IslandRemoval::Always,
    };
    if fill.and_then(|f| f.child("mode")).and_then(|m| m.arg(0)) == Some("hatch") {
        report.skip("hatched zone fills (filled solid)");
    }
    if item.child("min_thickness").and_then(|m| m.num(0)).is_some_and(|m| m > 0.0) {
        report.skip("zone minimum widths");
    }
    let layers_of_zone =
        if all || copper.is_empty() { vec![LayerId::TOP_COPPER, LayerId::BOTTOM_COPPER] } else { copper };
    if layers_of_zone.len() > 1 {
        report.note("zones on several layers became one zone per layer");
    }
    for &layer in &layers_of_zone {
        for (outline, holes) in &outlines {
            out.zones.push(Zone {
                id: ids.next(item),
                layer,
                net: net.clone(),
                outline: outline.clone(),
                holes: holes.clone(),
                priority: item.child("priority").and_then(|p| p.num(0)).unwrap_or(0.0) as u32,
                clearance: clearance.filter(|&c| c > 0),
                connection,
                thermal_gap,
                thermal_spoke,
                islands,
            });
        }
    }
}

fn point_in(p: Point, poly: &[Point]) -> bool {
    let mut inside = false;
    let n = poly.len();
    for i in 0..n {
        let (a, b) = (poly[i], poly[(i + 1) % n]);
        if (a.y > p.y) != (b.y > p.y) {
            let x = a.x as f64 + (p.y - a.y) as f64 * (b.x - a.x) as f64 / (b.y - a.y) as f64;
            if (p.x as f64) < x {
                inside = !inside;
            }
        }
    }
    inside
}

fn footprint(n: &Node, layers: &Layers, ids: &mut Ids, report: &mut Report) -> Option<PcbFootprint> {
    let lib_id = n.arg(0).unwrap_or("").to_string();
    let at = n.child("at")?;
    let pos = point(at.num(0)?, at.num(1)?, true);
    let orient = at.num(2).unwrap_or(0.0);
    let back = n.child("layer").and_then(|l| l.arg(0)) == Some("B.Cu");
    let locked = n.flag("locked");
    let attrs = atoms(n.child("attr"));
    let board_only = attrs.contains(&"board_only");

    // Reference, value and pad nets from the placed footprint.
    let mut refdes = String::new();
    let mut value = String::new();
    let mut fields: Vec<(&'static str, Node)> = Vec::new();
    let mut pad_nets = Vec::new();
    for item in n.lists() {
        match (item.head(), item.arg(0)) {
            ("property", Some("Reference")) | ("fp_text", Some("reference")) => {
                refdes = item.arg(1).unwrap_or("").to_string();
                fields.push(("${REF}", item.clone()));
            }
            ("property", Some("Value")) | ("fp_text", Some("value")) => {
                value = item.arg(1).unwrap_or("").to_string();
                fields.push(("${VALUE}", item.clone()));
            }
            ("pad", Some(name)) => {
                if let Some(net) = item.child("net") {
                    let net_name = net.arg(1).or(net.arg(0)).unwrap_or("");
                    if !net_name.is_empty() && net_name.parse::<f64>().is_err() {
                        pad_nets.push((name.to_string(), plain(net_name)));
                    }
                }
            }
            ("zone", _) => report.skip("zones and keepouts inside footprints"),
            ("net_tie_pad_groups", _) => report.skip("net ties (their pads are not tied)"),
            _ => {}
        }
    }

    // A top-side, unrotated copy of the footprint, without its fields.
    let local = canonical(n, orient, back);
    let body = Node::List(
        local
            .items()
            .iter()
            .filter(|i| {
                !matches!(
                    (i.head(), i.arg(0)),
                    ("property", Some("Reference" | "Value")) | ("fp_text", Some("reference" | "value"))
                )
            })
            .cloned()
            .collect(),
    );
    let (lib, _) = lib_id.split_once(':').unwrap_or(("", &lib_id));
    let ki = crate::footprint::convert(lib, &body);
    for w in &ki.warnings {
        report.skip(&format!("footprint content: {w}"));
    }
    let mut def = ki.footprint;

    let id = ids.of(n);
    let mut instance = FootprintInstance {
        id,
        component: Id::new(),
        pos,
        rotation: Angle::deg(if back { orient + 180.0 } else { orient }),
        side: if back { Side::Bottom } else { Side::Top },
        locked,
    };
    instance.rotation = Angle::deg(instance.rotation.degrees().rem_euclid(360.0));

    // Fields: templates in the definition, and the instance's own copies.
    let mut field_objects = Vec::new();
    for (content, node) in fields {
        let local_node = canonical_item(&node, orient, back);
        let Some((layer, field_back, text, hidden)) = field(&local_node, content) else { continue };
        def.texts.push(FootprintText { back: field_back, layer, text: text.clone() });
        field_objects.push(FootprintField { id: Id::new(), footprint: id, layer, back: field_back, text, hidden });
    }
    let _ = layers;
    Some(PcbFootprint { refdes, value, lib_id, def, instance, fields: field_objects, pad_nets, board_only })
}

/// A field text in footprint coordinates: layer, back, text, hidden.
fn field(n: &Node, content: &str) -> Option<(FootprintLayer, bool, Text, bool)> {
    let layer_name = n.child("layer").and_then(|l| l.arg(0)).unwrap_or("F.SilkS");
    let (layer, back) = crate::footprint::layer(layer_name).unwrap_or((FootprintLayer::Silkscreen, false));
    let layer = match layer {
        FootprintLayer::Silkscreen | FootprintLayer::Fabrication => layer,
        _ => FootprintLayer::Silkscreen,
    };
    let effects = n.child("effects");
    let hidden = n.flag("hide") || effects.is_some_and(|e| e.flag("hide"));
    let effects = effects.map(without_hide);
    let mut t = text(content.to_string(), n.child("at")?, effects.as_ref(), true, 1.0)?;
    // Board texts are bold: thin silkscreen doesn't fabricate well.
    t.bold = true;
    Some((layer, back, t, hidden))
}

fn without_hide(n: &Node) -> Node {
    match n {
        Node::List(items) => Node::List(
            items.iter().filter(|i| i.as_atom() != Some("hide") && i.head() != "hide").map(without_hide).collect(),
        ),
        other => other.clone(),
    }
}

/// The footprint `n` as if unrotated on the top side: absolute pad and text angles made
/// relative, and back side content mirrored back (local y and angles negated, front and
/// back layers swapped).
fn canonical(n: &Node, orient: f64, back: bool) -> Node {
    match n {
        Node::List(items) => Node::List(
            items
                .iter()
                .map(|i| match i.head() {
                    "pad" | "property" | "fp_text" | "fp_text_box" => canonical_item(i, orient, back),
                    _ if back => mirrored(i),
                    _ => i.clone(),
                })
                .collect(),
        ),
        other => other.clone(),
    }
}

/// A pad or text of a placed footprint: its angle relative to the footprint, mirrored
/// back for back-side footprints.
fn canonical_item(n: &Node, orient: f64, back: bool) -> Node {
    let Node::List(items) = n else { return n.clone() };
    let items: Vec<Node> = items
        .iter()
        .map(|i| {
            if i.head() == "at"
                && let (Some(x), Some(y)) = (i.num(0), i.num(1))
            {
                let a = i.num(2).unwrap_or(0.0) - orient;
                let a = if back { -a } else { a };
                let y = if back { -y } else { y };
                let mut v = vec![Node::Atom("at".into()), num(x), num(y)];
                if a.abs() > 1e-9 {
                    v.push(num(a));
                }
                Node::List(v)
            } else if back {
                mirrored(i)
            } else {
                i.clone()
            }
        })
        .collect();
    Node::List(items)
}

fn num(v: f64) -> Node {
    Node::Atom(format!("{}", (v * 1e6).round() / 1e6))
}

/// Mirror back-side content to the front: y of points negated (also in `at`, with its
/// angle), layer names swapped, chamfered corners swapped top and bottom.
fn mirrored(n: &Node) -> Node {
    match n {
        Node::Atom(a) => Node::Atom(swap_side(a)),
        Node::List(items) => {
            let head = n.head();
            if matches!(head, "start" | "end" | "mid" | "center" | "xy" | "at")
                && let (Some(x), Some(y)) = (n.num(0), n.num(1))
            {
                let mut v = vec![Node::Atom(head.to_string()), num(x), num(-y)];
                if let Some(a) = n.num(2) {
                    v.push(num(-a));
                }
                return Node::List(v);
            }
            if head == "angle" {
                // KiCad 5 arcs.
                return Node::List(vec![Node::Atom("angle".into()), num(-n.num(0).unwrap_or(0.0))]);
            }
            if head == "chamfer" {
                return Node::List(
                    items
                        .iter()
                        .map(|i| match i.as_atom() {
                            Some("top_left") => Node::Atom("bottom_left".into()),
                            Some("top_right") => Node::Atom("bottom_right".into()),
                            Some("bottom_left") => Node::Atom("top_left".into()),
                            Some("bottom_right") => Node::Atom("top_right".into()),
                            _ => i.clone(),
                        })
                        .collect(),
                );
            }
            Node::List(items.iter().map(mirrored).collect())
        }
    }
}

/// `F.X` ↔ `B.X` (layer names only).
fn swap_side(a: &str) -> String {
    if let Some(rest) = a.strip_prefix("F.") {
        format!("B.{rest}")
    } else if let Some(rest) = a.strip_prefix("B.") {
        format!("F.{rest}")
    } else {
        a.to_string()
    }
}
