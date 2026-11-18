//! Net classes: their clearance in DRC, the routing check and zone fills, and the
//! validation of the classes.

use editor_core::connectivity::Connectivity;
use editor_core::copper::{BoardConnectivity, CopperItem, Geo};
use editor_core::query::{pad_positions, pin_positions};
use editor_core::route_check::{self, Probe};
use editor_core::{Command, Editor, drc};
use model::builtin::{self, PART_RESISTOR};
use model::*;

/// R1 and R2 joined in the schematic, on a 30 × 20 mm board, with a track from R1.2
/// passing `gap` above R1.1. Returns the editor, the track's net and R1.1's centre.
fn board(gap: Coord) -> (Editor, String, Point) {
    let mut ed = Editor::new(Project::new("classes"));
    ed.execute(Command::import_parts(&builtin::builtin_library(), &[PART_RESISTOR]).unwrap()).unwrap();
    let mut placed = Vec::new();
    for (x, at) in [(0.0, Point::mm(10.0, 10.0)), (20.32, Point::mm(20.0, 10.0))] {
        let c = ed
            .execute(Command::PlacePart {
                part: PART_RESISTOR,
                variant: None,
                sheet: None,
                pos: Point::mm(x, 0.0),
                rotation: Angle::ZERO,
                mirror: false,
                refdes: None,
                value: None,
            })
            .unwrap();
        let sym = c.ids().find_map(|id| if let ObjectId::Symbol(s) = id { Some(s) } else { None }).unwrap();
        let fp = c.ids().find_map(|id| if let ObjectId::Footprint(f) = id { Some(f) } else { None }).unwrap();
        let pos = ed.project().board.footprints.get(fp).unwrap().pos;
        ed.execute(Command::Move { ids: vec![ObjectId::Footprint(fp)], delta: at - pos, keep_connected: false })
            .unwrap();
        placed.push((sym, fp));
    }
    let pin = |ed: &Editor, s, n| pin_positions(ed.project(), s).into_iter().find(|p| p.1 == n).unwrap().2;
    let (a, b) = (pin(&ed, placed[0].0, "2"), pin(&ed, placed[1].0, "1"));
    ed.execute(Command::AddWire { sheet: None, points: vec![a, b] }).unwrap();
    let outline = Shape::rect(Point::mm(0.0, 0.0), Point::mm(30.0, 20.0), 100_000);
    let g = BoardGraphic { id: Id::new(), layer: LayerId::OUTLINE, shape: outline };
    ed.execute(Command::Insert { objects: vec![Object::BoardGraphic(g)] }).unwrap();

    let pad = |ed: &Editor, f, n| pad_positions(ed.project(), f).into_iter().find(|p| p.1 == n).unwrap().2;
    let (p12, p21, p11) = (pad(&ed, placed[0].1, "2"), pad(&ed, placed[1].1, "1"), pad(&ed, placed[0].1, "1"));
    let top = LayerId::TOP_COPPER;
    ed.execute(Command::AddTrack { layer: top, points: vec![p12, p21], width: None, mids: Vec::new() }).unwrap();
    // The pad's top edge is 0.5 mm above its centre, the track 0.25 mm wide.
    let y = p11.y + mm(0.5) + mm(0.125) + gap;
    let points = vec![p12, Point::new(p12.x, y), Point::new(p11.x, y)];
    ed.execute(Command::AddTrack { layer: top, points, width: None, mids: Vec::new() }).unwrap();
    let copper = copper(&ed);
    let net = copper.items.iter().find_map(|c| match c.item {
        CopperItem::Track(_) => copper.net_of(c.item).map(str::to_string),
        _ => None,
    });
    (ed, net.expect("the track has a net"), p11)
}

fn copper(ed: &Editor) -> BoardConnectivity {
    let p = ed.project();
    BoardConnectivity::build(p, &Connectivity::build(p))
}

fn clearance_findings(ed: &Editor) -> Vec<String> {
    drc::run(ed.project(), &copper(ed))
        .into_iter()
        .filter(|v| v.kind == drc::ViolationKind::Clearance)
        .map(|v| v.message)
        .collect()
}

fn with_class(ed: &mut Editor, class: NetClass) {
    let mut rules = ed.project().board.rules.clone();
    rules.net_classes = vec![class];
    ed.execute(Command::SetDesignRules { rules }).unwrap();
}

fn wide(net: &str) -> NetClass {
    NetClass { name: "Wide".into(), clearance: Some(mm(0.4)), nets: vec![net.into()], ..Default::default() }
}

#[test]
fn drc_takes_the_larger_class_clearance() {
    let (mut ed, net, _) = board(mm(0.3));
    assert_eq!(clearance_findings(&ed), Vec::<String>::new(), "0.3 mm beats the board's 0.2 mm");
    with_class(&mut ed, wide(&net));
    let found = clearance_findings(&ed);
    assert_eq!(found.len(), 1, "{found:?}");
    assert!(found[0].contains("0.300 mm apart (clearance 0.400 mm)"), "{}", found[0]);
    // The pad's side: its net is in the Default class, the track's still wins.
    let mut rules = ed.project().board.rules.clone();
    rules.net_classes[0].nets = vec!["*".into()];
    ed.execute(Command::SetDesignRules { rules }).unwrap();
    assert_eq!(clearance_findings(&ed).len(), 1);
}

#[test]
fn routing_check_knows_the_class_of_the_routed_net() {
    let (mut ed, net, p11) = board(mm(1.0));
    // A new track of the net 0.3 mm below R1.1 (bottom edge 0.5 mm below its centre).
    let y = p11.y - mm(0.5) - mm(0.125) - mm(0.3);
    let probe = Probe::Track {
        layer: LayerId::TOP_COPPER,
        a: Point::new(p11.x - mm(3.0), y),
        b: Point::new(p11.x + mm(3.0), y),
        mid: None,
        width: mm(0.25),
    };
    let check =
        |ed: &Editor| route_check::check(ed.project(), &copper(ed), std::slice::from_ref(&probe), Some(&net), &[]);
    assert_eq!(check(&ed), None);
    with_class(&mut ed, wide(&net));
    let clash = check(&ed).expect("too close for the class");
    assert!(clash.message.contains("clearance 0.400 mm"), "{}", clash.message);
}

#[test]
fn zone_fills_keep_the_class_clearance() {
    let (mut ed, net, _) = board(mm(1.0));
    let mut zone = Zone::new(
        LayerId::TOP_COPPER,
        "",
        vec![Point::mm(1.0, 1.0), Point::mm(29.0, 1.0), Point::mm(29.0, 19.0), Point::mm(1.0, 19.0)],
    );
    zone.islands = IslandRemoval::Never;
    ed.execute(Command::Insert { objects: vec![Object::Zone(zone)] }).unwrap();
    // The fill is part of the board's copper (filled when it is built).
    let nearest = |ed: &Editor| {
        let copper = copper(ed);
        let of = |zone: bool| -> Vec<&Geo> {
            copper
                .items
                .iter()
                .filter(|c| match c.item {
                    CopperItem::Zone { .. } => zone,
                    CopperItem::Track(_) => !zone && copper.net_of(c.item) == Some(net.as_str()),
                    _ => false,
                })
                .map(|c| &c.geo)
                .collect()
        };
        let (fills, tracks) = (of(true), of(false));
        assert!(!fills.is_empty() && !tracks.is_empty());
        fills.iter().flat_map(|f| tracks.iter().map(|t| f.distance(t))).fold(f64::MAX, f64::min)
    };
    let before = nearest(&ed);
    assert!((before - 2e5).abs() < 1e4, "the board's 0.2 mm: {before}");
    with_class(&mut ed, wide(&net));
    let after = nearest(&ed);
    assert!((after - 4e5).abs() < 1e4, "the class's 0.4 mm: {after}");
}

#[test]
fn classes_are_validated() {
    let (mut ed, net, _) = board(mm(1.0));
    let rules = ed.project().board.rules.clone();
    let try_with = |ed: &mut Editor, classes: Vec<NetClass>| {
        let mut r = rules.clone();
        r.net_classes = classes;
        ed.execute(Command::SetDesignRules { rules: r }).map_err(|e| e.to_string())
    };
    let named = |name: &str| NetClass { name: name.into(), ..Default::default() };
    assert!(try_with(&mut ed, vec![named("default")]).unwrap_err().contains("Default"));
    assert!(try_with(&mut ed, vec![named(" ")]).is_err());
    assert!(try_with(&mut ed, vec![named("A"), named("a")]).unwrap_err().contains("two net classes"));
    let tiny_via = NetClass { via_diameter: Some(mm(0.2)), ..named("A") };
    assert!(try_with(&mut ed, vec![tiny_via]).unwrap_err().contains("drill"));
    let zero = NetClass { clearance: Some(0), ..named("A") };
    assert!(try_with(&mut ed, vec![zero]).is_err());
    assert!(try_with(&mut ed, vec![wide(&net), named("B")]).is_ok());
}
