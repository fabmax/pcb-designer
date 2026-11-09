//! Board connectivity, ratsnest and layer stack: route small boards with commands.

use editor_core::connectivity::Connectivity;
use editor_core::copper::{BoardConnectivity, CopperItem, Flag};
use editor_core::query::{pad_positions, pin_positions};
use editor_core::{Command, Editor};
use model::builtin::{self, PART_HEADER_1X4, PART_LED, PART_RESISTOR};
use model::*;

struct B {
    ed: Editor,
}

impl B {
    fn new() -> Self {
        let mut ed = Editor::new(Project::new("test"));
        let parts = vec![PART_RESISTOR, PART_LED, PART_HEADER_1X4];
        ed.execute(Command::ImportParts { library: builtin::starter_library(), parts }).unwrap();
        B { ed }
    }

    /// Place a part in the schematic; returns (symbol, footprint).
    fn place(&mut self, part: Id, x: f64) -> (Id, Id) {
        let c = self
            .ed
            .execute(Command::PlacePart {
                part,
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
        (sym, fp)
    }

    fn pin(&self, symbol: Id, name: &str) -> Point {
        pin_positions(self.ed.project(), symbol).into_iter().find(|p| p.1 == name).unwrap().2
    }

    fn pad(&self, footprint: Id, name: &str) -> Point {
        pad_positions(self.ed.project(), footprint).into_iter().find(|p| p.1 == name).unwrap().2
    }

    fn wire(&mut self, a: Point, b: Point) {
        let pts = if a.x == b.x || a.y == b.y { vec![a, b] } else { vec![a, Point::new(b.x, a.y), b] };
        self.ed.execute(Command::AddWire { sheet: None, points: pts }).unwrap();
    }

    fn label(&mut self, pos: Point, name: &str) {
        let cmd =
            Command::AddLabel { sheet: None, pos, name: name.into(), kind: LabelKind::Net, rotation: Angle::ZERO };
        self.ed.execute(cmd).unwrap();
    }

    fn move_fp(&mut self, fp: Id, x: f64, y: f64) {
        let pos = self.ed.project().board.footprints.get(fp).unwrap().pos;
        let delta = Point::mm(x, y) - pos;
        self.ed.execute(Command::Move { ids: vec![ObjectId::Footprint(fp)], delta, keep_connected: false }).unwrap();
    }

    fn track(&mut self, layer: LayerId, points: &[Point]) -> Id {
        let c = self.ed.execute(Command::AddTrack { layer, points: points.to_vec(), width: None }).unwrap();
        c.ids().find_map(|id| if let ObjectId::Track(t) = id { Some(t) } else { None }).unwrap()
    }

    fn via(&mut self, pos: Point) -> Id {
        let c = self.ed.execute(Command::AddVia { pos, diameter: None, drill: None }).unwrap();
        c.ids().find_map(|id| if let ObjectId::Via(v) = id { Some(v) } else { None }).unwrap()
    }

    fn bc(&self) -> BoardConnectivity {
        let p = self.ed.project();
        BoardConnectivity::build(p, &Connectivity::build(p))
    }
}

/// R1 and R2 in series, with labels IN (R1.1) and OUT (R2.2); footprints side by side.
fn series() -> (B, [Id; 2]) {
    let mut b = B::new();
    let (s1, f1) = b.place(PART_RESISTOR, 0.0);
    let (s2, f2) = b.place(PART_RESISTOR, 20.32);
    b.wire(b.pin(s1, "2"), b.pin(s2, "1"));
    b.label(b.pin(s1, "1"), "IN");
    b.label(b.pin(s2, "2"), "OUT");
    b.move_fp(f1, 10.0, 10.0);
    b.move_fp(f2, 20.0, 10.0);
    (b, [f1, f2])
}

#[test]
fn routing_removes_ratsnest_lines() {
    let (mut b, [f1, f2]) = series();
    let bc = b.bc();
    assert_eq!(bc.unrouted(), 1, "R1.2 to R2.1");
    let line = bc.ratsnest[0];
    let net = &bc.nets[line.net as usize];
    assert_eq!(net, "Net-(R1-2)");
    let (p1, p2) = (b.pad(f1, "2"), b.pad(f2, "1"));
    assert!([line.a, line.b].contains(&p1) && [line.a, line.b].contains(&p2));

    // A track between pad centers (any posture) routes it.
    let mid = Point::new((p1.x + p2.x) / 2, p1.y + 2_000_000);
    let t = b.track(LayerId::TOP_COPPER, &[p1, mid, p2]);
    let bc = b.bc();
    assert_eq!(bc.unrouted(), 0);
    assert_eq!(bc.net_of(CopperItem::Track(t)), Some("Net-(R1-2)"));
    assert!(bc.shorts.is_empty());

    // On the wrong layer, the SMD pads are not reached.
    b.ed.undo();
    b.track(LayerId::BOTTOM_COPPER, &[p1, p2]);
    assert_eq!(b.bc().unrouted(), 1);
}

#[test]
fn vias_connect_layers() {
    let (mut b, [f1, f2]) = series();
    let (p1, p2) = (b.pad(f1, "2"), b.pad(f2, "1"));
    let (v1, v2) = (p1 + Point::mm(0.0, -3.0), p2 + Point::mm(0.0, -3.0));
    b.track(LayerId::TOP_COPPER, &[p1, v1]);
    let via = b.via(v1);
    b.track(LayerId::BOTTOM_COPPER, &[v1, v2]);
    let bc = b.bc();
    assert_eq!(bc.unrouted(), 1, "still open at the second via");
    assert_eq!(bc.net_of(CopperItem::Via(via)), Some("Net-(R1-2)"));
    // The ratsnest now starts at the end of the routed copper, not at the pad.
    let line = bc.ratsnest[0];
    assert!([line.a, line.b].contains(&v2) && [line.a, line.b].contains(&p2), "{line:?}");
    b.via(v2);
    b.track(LayerId::TOP_COPPER, &[v2, p2]);
    assert_eq!(b.bc().unrouted(), 0);
}

#[test]
fn shorts_are_reported() {
    let (mut b, [f1, f2]) = series();
    // IN to OUT directly.
    b.track(
        LayerId::TOP_COPPER,
        &[b.pad(f1, "1"), b.pad(f1, "1") + Point::mm(0.0, 5.0), b.pad(f2, "2") + Point::mm(0.0, 5.0), b.pad(f2, "2")],
    );
    let bc = b.bc();
    assert_eq!(bc.shorts.len(), 1);
    assert_eq!(bc.shorts[0].0, vec!["IN".to_string(), "OUT".to_string()]);
}

#[test]
fn through_hole_pads_are_on_all_layers() {
    let mut b = B::new();
    let (sj, fj) = b.place(PART_HEADER_1X4, 0.0);
    let (sr, fr) = b.place(PART_RESISTOR, 20.32);
    b.wire(b.pin(sj, "1"), b.pin(sr, "1"));
    b.move_fp(fj, 0.0, 0.0);
    b.move_fp(fr, 10.0, 0.0);
    let (pj, pr) = (b.pad(fj, "1"), b.pad(fr, "1"));
    // Bottom track from the header pad, via, top track to the SMD pad.
    let v = Point::new(pr.x, pj.y - 3_000_000);
    b.track(LayerId::BOTTOM_COPPER, &[pj, Point::new(pj.x, v.y), v]);
    b.via(v);
    b.track(LayerId::TOP_COPPER, &[v, pr]);
    assert_eq!(b.bc().unrouted(), 0);
}

#[test]
fn small_led_board_fully_routed() {
    // J1 (power in) → R1 → D1 → J1 GND, every connection routed on the top layer.
    let mut b = B::new();
    let (sj, fj) = b.place(PART_HEADER_1X4, 0.0);
    let (sr, fr) = b.place(PART_RESISTOR, 20.32);
    let (sd, fd) = b.place(PART_LED, 40.64);
    b.wire(b.pin(sr, "2"), b.pin(sd, "A"));
    b.label(b.pin(sj, "1"), "VIN");
    b.label(b.pin(sr, "1"), "VIN");
    b.label(b.pin(sj, "4"), "GND");
    b.label(b.pin(sd, "K"), "GND");
    b.move_fp(fj, 0.0, 0.0);
    b.move_fp(fr, 10.0, 0.0);
    b.move_fp(fd, 20.0, 0.0);
    assert_eq!(b.bc().unrouted(), 3);
    let route = |b: &mut B, a: Point, c: Point, dy: f64| {
        let y = a.y.max(c.y) + mm(dy);
        b.track(LayerId::TOP_COPPER, &[a, Point::new(a.x, y), Point::new(c.x, y), c]);
    };
    let (a, c) = (b.pad(fj, "1"), b.pad(fr, "1"));
    route(&mut b, a, c, 3.0);
    // LED: pad 1 = K, pad 2 = A.
    let (a, c) = (b.pad(fr, "2"), b.pad(fd, "2"));
    route(&mut b, a, c, 2.0);
    // GND below the parts.
    let (g1, g2) = (b.pad(fj, "4"), b.pad(fd, "1"));
    let y = g1.y.min(g2.y) - mm(3.0);
    b.track(
        LayerId::TOP_COPPER,
        &[g1, Point::new(g1.x + mm(2.0), g1.y), Point::new(g1.x + mm(2.0), y), Point::new(g2.x, y), g2],
    );
    let bc = b.bc();
    assert!(bc.shorts.is_empty(), "{:?}", bc.shorts);
    assert_eq!(bc.unrouted(), 0, "{:?}", bc.ratsnest);
}

#[test]
fn layer_stack_changes_are_undoable() {
    let (mut b, [f1, _]) = series();
    b.ed.execute(Command::SetCopperLayers { count: 4 }).unwrap();
    let names: Vec<String> = b.ed.project().board.copper_layers().map(|l| l.name.clone()).collect();
    assert_eq!(names, ["Top", "Inner 1", "Inner 2", "Bottom"]);
    let p = b.pad(f1, "1");
    b.track(LayerId::inner(2), &[p, p + Point::mm(5.0, 0.0)]);
    let err = b.ed.execute(Command::SetCopperLayers { count: 2 }).unwrap_err();
    assert!(err.to_string().contains("Inner 2"), "{err}");
    assert!(b.ed.execute(Command::SetCopperLayers { count: 3 }).is_err(), "odd");
    b.ed.undo(); // the track
    let c = b.ed.execute(Command::SetCopperLayers { count: 2 }).unwrap();
    assert!(c.setup.is_some());
    assert_eq!(b.ed.project().board.copper_layers().count(), 2);
    b.ed.undo();
    assert_eq!(b.ed.project().board.copper_layers().count(), 4);
    b.ed.undo();
    assert_eq!(b.ed.project().board.copper_layers().count(), 2);
    b.ed.redo();
    assert_eq!(b.ed.project().board.copper_layers().count(), 4);

    let mut rules = b.ed.project().board.rules.clone();
    rules.track_width = mm(0.4);
    b.ed.execute(Command::SetDesignRules { rules }).unwrap();
    let t = b.track(LayerId::TOP_COPPER, &[p, p + Point::mm(0.0, 5.0)]);
    assert_eq!(b.ed.project().board.tracks.get(t).unwrap().width, mm(0.4));
    rules = b.ed.project().board.rules.clone();
    rules.via_diameter = rules.via_drill;
    assert!(b.ed.execute(Command::SetDesignRules { rules }).is_err());
}

// ---- Schematic edits on a routed board: copper is kept and flagged, never deleted.

/// `series()` with R1.2–R2.1 routed on top. Returns the track.
fn routed_series() -> (B, [Id; 2], Id) {
    let (mut b, fps) = series();
    let (p1, p2) = (b.pad(fps[0], "2"), b.pad(fps[1], "1"));
    let t = b.track(LayerId::TOP_COPPER, &[p1, p2]);
    assert_eq!(b.bc().unrouted(), 0);
    (b, fps, t)
}

fn symbol_of(b: &B, fp: Id) -> Id {
    let c = b.ed.project().board.footprints.get(fp).unwrap().component;
    b.ed.project().schematic.symbols_of(c).next().unwrap().id
}

#[test]
fn rewiring_the_schematic_flags_tracks_as_shorts() {
    let (mut b, [f1, _], t) = routed_series();
    // R1.2 moves to net IN: the routed track now joins IN and R2.1.
    let wire = b.ed.project().schematic.wires.iter().next().unwrap().id;
    b.ed.execute(Command::Delete { ids: vec![ObjectId::Wire(wire)] }).unwrap();
    let s1 = symbol_of(&b, f1);
    b.label(b.pin(s1, "2"), "IN");
    let bc = b.bc();
    assert!(b.ed.project().board.tracks.contains(t), "kept");
    assert_eq!(bc.get(CopperItem::Track(t)).unwrap().flag, Some(Flag::Short));
    assert_eq!(bc.shorts.len(), 1);
    assert!(bc.shorts[0].0.contains(&"IN".to_string()));
    // Undo restores a clean board.
    b.ed.undo();
    b.ed.undo();
    let bc = b.bc();
    assert!(bc.flagged().next().is_none() && bc.shorts.is_empty() && bc.unrouted() == 0);
}

#[test]
fn deleting_a_component_leaves_its_copper_flagged() {
    let (mut b, [f1, f2], t) = routed_series();
    let p = b.pad(f2, "2");
    let stub = b.track(LayerId::TOP_COPPER, &[p, p + Point::mm(0.0, 3.0)]);
    let pos = b.ed.project().board.footprints.get(f2).unwrap().pos;
    let s2 = symbol_of(&b, f2);
    b.ed.execute(Command::Delete { ids: vec![ObjectId::Symbol(s2)] }).unwrap();
    assert!(!b.ed.project().board.footprints.contains(f2), "footprint follows the schematic");
    let bc = b.bc();
    assert_eq!(bc.get(CopperItem::Track(stub)).unwrap().flag, Some(Flag::Unconnected));
    // Still touching R1's pad: keeps that net.
    assert_eq!(bc.get(CopperItem::Track(t)).unwrap().flag, None);
    assert_eq!(bc.net_of(CopperItem::Track(t)), Some("Net-(R1-2)"));
    b.ed.undo();
    assert_eq!(b.ed.project().board.footprints.get(f2).unwrap().pos, pos, "back where it was");
    assert!(b.bc().flagged().next().is_none());
    let _ = f1;
}

#[test]
fn renaming_nets_and_references_moves_routed_copper_along() {
    let (mut b, [f1, f2], t) = routed_series();
    // Label the routed net: the track takes the name.
    let s1 = symbol_of(&b, f1);
    b.label(b.pin(s1, "2"), "MID");
    assert_eq!(b.bc().net_of(CopperItem::Track(t)), Some("MID"));
    // Rename the label.
    let mut label = b.ed.project().schematic.labels.iter().find(|l| l.name == "MID").unwrap().clone();
    label.name = "DIVIDER".into();
    b.ed.execute(Command::Update { objects: vec![Object::Label(label)] }).unwrap();
    // Swap reference designators: placements belong to components, not names.
    let c1 = b.ed.project().board.footprints.get(f1).unwrap().component;
    let c2 = b.ed.project().board.footprints.get(f2).unwrap().component;
    for (c, r) in [(c1, "R9"), (c2, "R1"), (c1, "R2")] {
        b.ed.execute(Command::SetComponent { id: c, refdes: Some(r.into()), value: None, variant: None }).unwrap();
    }
    let bc = b.bc();
    assert_eq!(bc.net_of(CopperItem::Track(t)), Some("DIVIDER"));
    assert!(bc.flagged().next().is_none() && bc.shorts.is_empty() && bc.unrouted() == 0);
    assert_eq!(b.ed.project().board.footprints.get(f1).unwrap().component, c1);
}

#[test]
fn changing_the_footprint_variant_updates_copper() {
    let (mut b, [f1, _], t) = routed_series();
    let c1 = b.ed.project().board.footprints.get(f1).unwrap().component;
    let v0805 = b.ed.project().library.parts.get(PART_RESISTOR).unwrap().variants[1].id;
    let before = b.pad(f1, "2");
    b.ed.execute(Command::SetComponent { id: c1, refdes: None, value: None, variant: Some(v0805) }).unwrap();
    let after = b.pad(f1, "2");
    assert_ne!(before, after, "0805 pads are further apart");
    let bc = b.bc();
    // The track ends inside the bigger pad: still connected, nothing flagged.
    assert_eq!(bc.net_of(CopperItem::Track(t)), Some("Net-(R1-2)"));
    assert!(bc.flagged().next().is_none());
    assert_eq!(bc.unrouted(), 0);
}

#[test]
fn flipping_a_footprint_disconnects_top_tracks() {
    let (mut b, [f1, _], _) = routed_series();
    let x = b.ed.project().board.footprints.get(f1).unwrap().pos.x;
    b.ed.execute(Command::FlipSide { ids: vec![ObjectId::Footprint(f1)], axis_x: x }).unwrap();
    assert_eq!(b.bc().unrouted(), 1, "SMD pads moved to the bottom");
}
