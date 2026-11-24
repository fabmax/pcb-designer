//! Our board → `NAME.kicad_pcb`.
//!
//! Board coordinates are y-down in KiCad: y is negated, angles keep their value.
//! Footprints are written as KiCad stores them placed (see [`super::lib::Place`]).
//! Tracks and vias get the nets our connectivity gives them; zones are written unfilled.

use std::collections::{BTreeSet, HashMap};

use editor_core::connectivity::Connectivity;
use editor_core::copper::{BoardConnectivity, CopperItem};
use model::*;

use super::lib::{Names, Place, Placed, effects, footprint, graphic, thickness};
use super::sch::SchExport;
use super::sx::{X, l};
use crate::project::Report;

/// KiCad's name of a board layer.
pub fn layer_name(id: LayerId, copper: u16) -> Option<String> {
    Some(match id {
        LayerId::TOP_COPPER => "F.Cu".into(),
        LayerId::BOTTOM_COPPER => "B.Cu".into(),
        LayerId::TOP_SILK => "F.SilkS".into(),
        LayerId::BOTTOM_SILK => "B.SilkS".into(),
        LayerId::TOP_MASK => "F.Mask".into(),
        LayerId::BOTTOM_MASK => "B.Mask".into(),
        LayerId::TOP_PASTE => "F.Paste".into(),
        LayerId::BOTTOM_PASTE => "B.Paste".into(),
        LayerId::TOP_FAB => "F.Fab".into(),
        LayerId::BOTTOM_FAB => "B.Fab".into(),
        LayerId::TOP_COURTYARD => "F.CrtYd".into(),
        LayerId::BOTTOM_COURTYARD => "B.CrtYd".into(),
        LayerId::OUTLINE => "Edge.Cuts".into(),
        LayerId(n) if n >= 1 && n + 1 < copper => format!("In{n}.Cu"),
        _ => return None,
    })
}

/// Our board coordinates in KiCad's: y down, moved by `origin` (where our origin is in
/// KiCad's, so the board lands on KiCad's page).
#[derive(Clone, Copy)]
struct Origin(Point);

impl Origin {
    /// The board on KiCad's page: its top left corner 25 mm or so in from the page's.
    fn of(b: &Board) -> Origin {
        let outline = b.graphics.iter().filter(|g| g.layer == LayerId::OUTLINE).map(|g| g.shape.bounds());
        let bounds = Rect::union_all(outline)
            .or_else(|| Rect::union_all(b.footprints.iter().map(|f| Rect::around(f.pos, 0))))
            .unwrap_or(Rect::around(Point::ZERO, 0));
        let mm = |v: Coord| (v as f64 / 1e6).round() as Coord * 1_000_000;
        Origin(Point::new(mm(25_000_000 - bounds.min.x), mm(25_000_000 + bounds.max.y)))
    }

    fn at(self, p: Point) -> (Coord, Coord) {
        (p.x + self.0.x, self.0.y - p.y)
    }

    fn xy(self, head: &str, p: Point) -> X {
        let (x, y) = self.at(p);
        l(head).mm(x).mm(y)
    }
}

fn uuid(id: Id) -> X {
    l("uuid").str(id.to_string())
}

/// KiCad's net numbers by our net names, and its names by number (from 1).
struct Nets {
    numbers: HashMap<String, usize>,
    list: Vec<String>,
}

impl Nets {
    fn of(&self, name: Option<&str>) -> X {
        match name.and_then(|n| self.numbers.get(n).map(|&i| (i, n))) {
            Some((i, _)) => l("net").sym(i.to_string()),
            None => l("net").sym("0"),
        }
    }
}

pub fn export(p: &Project, names: &Names, sch: &SchExport, report: &mut Report) -> String {
    let b = &p.board;
    let conn = Connectivity::build(p);
    let bc = BoardConnectivity::build(p, &conn);
    let copper = b.copper_layers().count() as u16;
    let lname = |id: LayerId| layer_name(id, copper);
    let origin = Origin::of(b);
    let xy = |head: &str, p: Point| origin.xy(head, p);

    // Nets: the schematic's, named as KiCad names them, and the zones' (in case one has
    // no pads). Pads without a pin have no net in KiCad.
    let kicad_names = kicad_net_names(p, &conn);
    let mut all: BTreeSet<String> = kicad_names.values().cloned().collect();
    all.extend(b.zones.iter().filter(|z| !z.net.is_empty()).map(|z| kicad_names.get(&z.net).unwrap_or(&z.net).clone()));
    let net_list: Vec<String> = all.into_iter().collect();
    let mut numbers: HashMap<String, usize> = HashMap::new();
    for (ours, theirs) in &kicad_names {
        if let Some(i) = net_list.iter().position(|n| n == theirs) {
            numbers.insert(ours.clone(), i + 1);
        }
    }
    for z in b.zones.iter() {
        if let Some(i) = net_list.iter().position(|n| *n == z.net) {
            numbers.entry(z.net.clone()).or_insert(i + 1);
        }
    }
    let nets = Nets { numbers, list: net_list.clone() };

    let mut x = l("kicad_pcb")
        .push(l("version").sym("20241229"))
        .push(l("generator").str("pcb-designer"))
        .push(l("generator_version").str("9.0"))
        .push(l("general").push(l("thickness").mm(b.stackup.thickness)).yes_no("legacy_teardrops", false))
        .push(l("paper").str("A4"));
    let tb = p.schematic.title_block();
    let mut title = l("title_block");
    for (k, v) in [("title", &tb.title), ("date", &tb.date), ("rev", &tb.revision), ("company", &tb.company)] {
        if !v.is_empty() {
            title = title.push(l(k).str(v));
        }
    }
    x = x.push(title);

    // Layers: copper, then the technical layers KiCad always has.
    let mut layers = l("layers").push(l("0").str("F.Cu").sym("signal"));
    for n in 1..copper.saturating_sub(1) {
        layers = layers.push(l(&(2 + 2 * n as u32).to_string()).str(format!("In{n}.Cu")).sym("signal"));
    }
    layers = layers.push(l("2").str("B.Cu").sym("signal"));
    for (n, name, user) in [
        (9, "F.Adhes", Some("F.Adhesive")),
        (11, "B.Adhes", Some("B.Adhesive")),
        (13, "F.Paste", None),
        (15, "B.Paste", None),
        (5, "F.SilkS", Some("F.Silkscreen")),
        (7, "B.SilkS", Some("B.Silkscreen")),
        (1, "F.Mask", None),
        (3, "B.Mask", None),
        (17, "Dwgs.User", Some("User.Drawings")),
        (19, "Cmts.User", Some("User.Comments")),
        (21, "Eco1.User", Some("User.Eco1")),
        (23, "Eco2.User", Some("User.Eco2")),
        (25, "Edge.Cuts", None),
        (27, "Margin", None),
        (31, "F.CrtYd", Some("F.Courtyard")),
        (29, "B.CrtYd", Some("B.Courtyard")),
        (35, "F.Fab", None),
        (33, "B.Fab", None),
    ] {
        let mut e = l(&n.to_string()).str(name).sym("user");
        if let Some(u) = user {
            e = e.str(u);
        }
        layers = layers.push(e);
    }
    x = x.push(layers);
    x = x.push(setup(b, copper, origin));

    x = x.push(l("net").sym("0").str(""));
    for (i, n) in net_list.iter().enumerate() {
        x = x.push(l("net").sym((i + 1).to_string()).str(n));
    }

    // Footprints.
    for inst in b.footprints.iter() {
        let Some(c) = p.schematic.components.get(inst.component) else { continue };
        let Some(part) = p.library.parts.get(c.part) else { continue };
        let Some(def) = p.library.footprint_of(c.part, c.variant) else { continue };
        let Some(lib_id) = names.footprint_id(def.id) else { continue };
        let back = inst.side == Side::Bottom;
        let rot = inst.rotation.degrees();
        let place = Place { orient: if back { rot - 180.0 } else { rot }, back };
        let mut pad_nets = HashMap::new();
        for pad in def.pads.iter() {
            if let Some(net) = bc.net_of(CopperItem::Pad { footprint: inst.id, pad: pad.id })
                && let Some(&n) = nets.numbers.get(net)
            {
                pad_nets.insert(pad.id, (n, nets.list[n - 1].clone()));
            }
        }
        let anchor = sch.anchor.get(&c.id).and_then(|s| p.schematic.symbols.get(*s));
        let info = anchor.and_then(|a| sch.sheets.get(&a.sheet));
        let path = match (anchor, info) {
            (Some(a), Some(i)) if !part.is_board_part() => Some(format!("{}/{}", i.uuid_path, a.id)),
            _ => None,
        };
        if path.is_none() && !part.is_board_part() {
            report.warn(format!("{}: no symbol placed; its footprint is board-only in KiCad", c.refdes));
        }
        let in_bom = c.properties.get("In BOM").is_none_or(|v| v != "no");
        let mut properties = c.properties.clone();
        properties.retain(|k, _| k != "In BOM");
        let placed = Placed {
            at: origin.at(inst.pos),
            id: inst.id,
            refdes: &c.refdes,
            value: if c.value.is_empty() { &part.default_value } else { &c.value },
            fields: b.fields.iter().filter(|f| f.footprint == inst.id).collect(),
            nets: pad_nets,
            path,
            sheet_name: info.map_or("/".into(), |i| i.name_path.clone()),
            sheet_file: info.map_or(String::new(), |i| i.file.clone()),
            locked: inst.locked,
            dnp: c.dnp,
            in_bom,
            properties: &properties,
        };
        x = x.push(footprint(def, &lib_id, place, Some(&placed), report));
    }

    // Graphics and texts.
    for g in b.graphics.iter() {
        let Some(layer) = lname(g.layer) else { continue };
        x = x.push(graphic("gr", &g.shape, &xy).push(l("layer").str(layer)).push(uuid(g.id)));
    }
    for t in b.texts.iter() {
        let Some(layer) = lname(t.layer) else { continue };
        let back = matches!(
            t.layer,
            LayerId::BOTTOM_COPPER
                | LayerId::BOTTOM_SILK
                | LayerId::BOTTOM_MASK
                | LayerId::BOTTOM_PASTE
                | LayerId::BOTTOM_FAB
        );
        let a = t.text.rotation.degrees();
        let mut at = xy("at", t.text.pos);
        if a.abs() > 1e-9 {
            at = at.num(a);
        }
        x = x.push(
            l("gr_text")
                .str(super::lib::field_vars(&t.text.text))
                .push(at)
                .push(l("layer").str(layer))
                .push(uuid(t.id))
                .push(effects(&t.text, back, Some(thickness(t.text.size)))),
        );
    }

    // Copper.
    for t in b.tracks.iter() {
        let Some(layer) = lname(t.layer) else { continue };
        let net = nets.of(bc.net_of(CopperItem::Track(t.id)));
        let mut s = match t.mid {
            Some(m) => l("arc").push(xy("start", t.a)).push(xy("mid", m)).push(xy("end", t.b)),
            None => l("segment").push(xy("start", t.a)).push(xy("end", t.b)),
        };
        s = s.push(l("width").mm(t.width));
        if t.locked {
            s = s.yes_no("locked", true);
        }
        x = x.push(s.push(l("layer").str(layer)).push(net).push(uuid(t.id)));
    }
    for v in b.vias.iter() {
        let net = nets.of(bc.net_of(CopperItem::Via(v.id)));
        let mut s = l("via");
        if !v.is_through() {
            s = s.sym("blind");
        }
        if v.locked {
            s = s.yes_no("locked", true);
        }
        let (Some(from), Some(to)) = (lname(v.from), lname(v.to)) else { continue };
        x = x.push(
            s.push(xy("at", v.pos))
                .push(l("size").mm(v.diameter))
                .push(l("drill").mm(v.drill))
                .push(l("layers").str(from).str(to))
                .push(net)
                .push(uuid(v.id)),
        );
    }
    for z in b.zones.iter() {
        let Some(layer) = lname(z.layer) else { continue };
        x = x.push(zone(z, &layer, &nets, &b.rules, origin));
    }
    for k in b.keepouts.iter() {
        x = x.push(keepout(k, &lname, origin));
    }
    if !b.zones.is_empty() {
        report.note("zones are exported unfilled: fill them in KiCad (B)");
    }
    x.yes_no("embedded_fonts", false).to_text()
}

/// KiCad's names of our nets (by our name): labelled nets keep theirs; the others are
/// named after a pin as KiCad 9 does (`SCH_PIN::GetDefaultNetName`): `Net-(R1-Pad2)`
/// for a pin without a name, `Net-(Q1-B)` with one (`-Pad3` added when the symbol has
/// the name twice), `unconnected-(…)` for a lone pin. Names without `-Pad` win, then the
/// smallest.
fn kicad_net_names(p: &Project, conn: &Connectivity) -> HashMap<String, String> {
    let s = &p.schematic;
    let mut numbers_of: HashMap<(Id, Id), super::lib::PinNumbers> = HashMap::new();
    let mut out = HashMap::new();
    for net in &conn.nets {
        if net.named {
            // The label naming it, its `/` escaped as KiCad does (sheet paths keep theirs).
            let label = net
                .labels
                .iter()
                .filter_map(|id| s.labels.get(*id))
                .map(|l| l.name.as_str())
                .filter(|n| n.contains('/') && (net.name == *n || net.name.ends_with(&format!("/{n}"))))
                .max_by_key(|n| n.len());
            let name = match label {
                Some(n) => format!("{}{}", &net.name[..net.name.len() - n.len()], super::sch::net_text(n)),
                None => net.name.clone(),
            };
            out.insert(net.name.clone(), name);
            continue;
        }
        let lone = net.pins.len() == 1;
        let mut best: Option<String> = None;
        for np in &net.pins {
            let Some(c) = s.components.get(np.component) else { continue };
            let Some(part) = p.library.parts.get(c.part) else { continue };
            let Some(variant) = part.variant(c.variant) else { continue };
            let numbers = numbers_of
                .entry((part.id, variant.id))
                .or_insert_with(|| super::lib::pin_numbers(&p.library, part, variant));
            let Some(sym) = part.units.get(np.unit as usize).and_then(|u| p.library.symbols.get(u.symbol)) else {
                continue;
            };
            let Some(pin) = sym.pins.get(np.pin) else { continue };
            let unconnected = lone || pin.kind == PinKind::NotConnected;
            let prefix = if unconnected { "unconnected-(" } else { "Net-(" };
            // The symbol's pins as KiCad sees them: (name, number, no-connect).
            let unit_pins: Vec<(&str, &str, bool)> = sym
                .pins
                .iter()
                .flat_map(|q| {
                    numbers
                        .get(&(np.unit, q.id))
                        .into_iter()
                        .flatten()
                        .map(move |n| (q.name.as_str(), n.as_str(), q.kind == PinKind::NotConnected))
                })
                .collect();
            for number in numbers.get(&(np.unit, pin.id)).into_iter().flatten() {
                let name = if pin.name == "~" { "" } else { pin.name.as_str() };
                let candidate = if !name.is_empty() && name != number {
                    let multiple =
                        unit_pins.iter().any(|&(n, num, nc)| n == name && num != number && unconnected == nc);
                    let unit = if part.units.len() > 1 { unit_letters(np.unit as usize) } else { String::new() };
                    let pad =
                        if unconnected || multiple { format!("-Pad{}", net_escape(number)) } else { String::new() };
                    format!("{prefix}{}{unit}-{}{pad})", c.refdes, net_escape(name))
                } else {
                    format!("{prefix}{}-Pad{})", c.refdes, net_escape(number))
                };
                let better = match &best {
                    None => true,
                    Some(b) => {
                        let (lc, lb) = (candidate.contains("-Pad"), b.contains("-Pad"));
                        if lc != lb { !lc } else { candidate < *b }
                    }
                };
                if better {
                    best = Some(candidate);
                }
            }
        }
        if let Some(b) = best {
            out.insert(net.name.clone(), b);
        }
    }
    out
}

use super::sch::net_text as net_escape;

fn setup(b: &Board, copper: u16, origin: Origin) -> X {
    let s = &b.stackup;
    let mask = s.mask_color.label();
    let silk = s.silk_color.label();
    let cu = |n: u16| if n == 0 || n + 1 == copper { s.outer_copper } else { s.inner_copper };
    let names: Vec<String> = (0..copper)
        .map(|i| match i {
            0 => "F.Cu".to_string(),
            i if i + 1 == copper => "B.Cu".to_string(),
            i => format!("In{i}.Cu"),
        })
        .collect();
    let copper_total: Coord = (0..copper).map(cu).sum();
    let dielectrics = (copper - 1) as Coord;
    let mask_t: Coord = 10_000;
    let core = ((s.thickness - copper_total - 2 * mask_t) / dielectrics).max(10_000);
    let mut st = l("stackup")
        .push(l("layer").str("F.SilkS").push(l("type").str("Top Silk Screen")).push(l("color").str(silk)))
        .push(l("layer").str("F.Paste").push(l("type").str("Top Solder Paste")))
        .push(
            l("layer")
                .str("F.Mask")
                .push(l("type").str("Top Solder Mask"))
                .push(l("color").str(mask))
                .push(l("thickness").mm(mask_t)),
        );
    for (i, n) in names.iter().enumerate() {
        st = st.push(l("layer").str(n).push(l("type").str("copper")).push(l("thickness").mm(cu(i as u16))));
        if i + 1 < names.len() {
            let kind = if i % 2 == 0 { "core" } else { "prepreg" };
            st = st.push(
                l("layer")
                    .str(format!("dielectric {}", i + 1))
                    .push(l("type").str(kind))
                    .push(l("thickness").mm(core))
                    .push(l("material").str("FR4"))
                    .push(l("epsilon_r").num(4.5))
                    .push(l("loss_tangent").num(0.02)),
            );
        }
    }
    st = st
        .push(
            l("layer")
                .str("B.Mask")
                .push(l("type").str("Bottom Solder Mask"))
                .push(l("color").str(mask))
                .push(l("thickness").mm(mask_t)),
        )
        .push(l("layer").str("B.Paste").push(l("type").str("Bottom Solder Paste")))
        .push(l("layer").str("B.SilkS").push(l("type").str("Bottom Silk Screen")).push(l("color").str(silk)));
    let finish = match s.finish {
        Finish::Unspecified => "None",
        Finish::Hasl => "HAL SnPb",
        Finish::HaslLeadFree => "HAL lead-free",
        Finish::Enig => "ENIG",
        Finish::Osp => "OSP",
        Finish::ImmersionTin => "Immersion tin",
        Finish::ImmersionSilver => "Immersion silver",
        Finish::None => "None",
    };
    st = st.push(l("copper_finish").str(finish)).yes_no("dielectric_constraints", false);
    l("setup")
        .push(st)
        .push(l("pad_to_mask_clearance").mm(b.rules.mask_expansion))
        .push(l("pad_to_paste_clearance").mm(b.rules.paste_margin))
        .yes_no("allow_soldermask_bridges_in_footprints", false)
        .push(l("tenting").sym("front").sym("back"))
        // Our origin, for fabrication outputs and the grid.
        .push(l("aux_axis_origin").mm(origin.0.x).mm(origin.0.y))
        .push(l("grid_origin").mm(origin.0.x).mm(origin.0.y))
}

fn polygons(outline: &[Point], holes: &[Vec<Point>], o: Origin) -> Vec<X> {
    std::iter::once(outline)
        .chain(holes.iter().map(Vec::as_slice))
        .map(|p| l("polygon").push(l("pts").extend(p.iter().map(|q| o.xy("xy", *q)))))
        .collect()
}

fn zone(z: &Zone, layer: &str, nets: &Nets, rules: &DesignRules, o: Origin) -> X {
    let net = nets.numbers.get(&z.net).copied().unwrap_or(0);
    let net_name = if net == 0 { "" } else { nets.list[net - 1].as_str() };
    let connect = match z.connection {
        ZoneConnection::Solid => l("connect_pads").sym("yes"),
        ZoneConnection::None => l("connect_pads").sym("no"),
        ZoneConnection::ThermalThroughHole => l("connect_pads").sym("thru_hole_only"),
        ZoneConnection::Thermal => l("connect_pads"),
    };
    let net_rules = rules.for_net(Some(&z.net));
    let connect = connect.push(l("clearance").mm(z.clearance.unwrap_or(net_rules.clearance)));
    let mut fill = l("fill")
        .push(l("thermal_gap").mm(z.thermal_gap.unwrap_or(rules.thermal_gap)))
        .push(l("thermal_bridge_width").mm(z.thermal_spoke.unwrap_or(rules.thermal_spoke)));
    match z.islands {
        IslandRemoval::Always => {}
        IslandRemoval::Never => fill = fill.push(l("island_removal_mode").sym("1")),
        IslandRemoval::BelowArea(a) => {
            fill = fill.push(l("island_removal_mode").sym("2")).push(l("island_area_min").num(a as f64 / 1e12))
        }
    }
    l("zone")
        .push(l("net").sym(net.to_string()))
        .push(l("net_name").str(net_name))
        .push(l("layer").str(layer))
        .push(uuid(z.id))
        .push(l("hatch").sym("edge").num(0.5))
        .push(l("priority").sym(z.priority.to_string()))
        .push(connect)
        .push(l("min_thickness").num(0.0))
        .yes_no("filled_areas_thickness", false)
        .push(fill)
        .extend(polygons(&z.outline, &z.holes, o))
}

fn keepout(k: &Keepout, lname: &dyn Fn(LayerId) -> Option<String>, o: Origin) -> X {
    let mut layers = l("layers");
    if k.layers.is_empty() {
        layers = layers.str("*.Cu");
    } else {
        for &id in &k.layers {
            if let Some(n) = lname(id) {
                layers = layers.str(n);
            }
        }
    }
    let rule = |what: &str, no: bool| l(what).sym(if no { "not_allowed" } else { "allowed" });
    l("zone")
        .push(l("net").sym("0"))
        .push(l("net_name").str(""))
        .push(layers)
        .push(uuid(k.id))
        .push(l("hatch").sym("edge").num(0.5))
        .push(l("connect_pads").push(l("clearance").num(0.0)))
        .push(l("min_thickness").num(0.25))
        .yes_no("filled_areas_thickness", false)
        .push(
            l("keepout")
                .push(rule("tracks", k.no_tracks))
                .push(rule("vias", k.no_vias))
                .push(rule("pads", k.no_pads))
                .push(rule("copperpour", k.no_pour))
                .push(rule("footprints", k.no_footprints)),
        )
        .push(l("fill").push(l("thermal_gap").num(0.5)).push(l("thermal_bridge_width").num(0.5)))
        .extend(polygons(&k.outline, &k.holes, o))
}
