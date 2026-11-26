//! The Probe tool's targets and the simulation overlay on the schematic.

use editor_core::query::{pin_positions, symbol_of_instance};
use model::*;
use view::Document;
use view::demo::demo_project;
use view::sim_overlay::{ProbeTarget, SimOverlay, probe_target};

fn symbol(doc: &Document, refdes: &str) -> SymbolInstance {
    let p = doc.project();
    let c = p.schematic.components.iter().find(|c| c.refdes == refdes).unwrap();
    p.schematic.symbols.iter().find(|s| s.component == c.id).unwrap().clone()
}

#[test]
fn wires_probe_their_net_and_pin_bodies_their_current() {
    let doc = Document::new(demo_project());
    let p = doc.project();
    let conn = doc.connectivity();
    let tol = 300_000;
    // A wire: its net.
    let w = p.schematic.wires.iter().next().unwrap();
    let view = doc.view(view::ViewId::Sheet(w.sheet));
    let mid = Point::new((w.a.x + w.b.x) / 2, (w.a.y + w.b.y) / 2);
    let net = conn.net_of(ObjectId::Wire(w.id)).unwrap().name.clone();
    assert_eq!(probe_target(p, conn, view, mid, tol, false), Some(ProbeTarget::Net { net, at: ObjectId::Wire(w.id) }));

    // The middle of a pin: the current into it; its end is not the pin.
    let r1 = symbol(&doc, "R1");
    let view = doc.view(view::ViewId::Sheet(r1.sheet));
    let pin = &symbol_of_instance(p, &r1).unwrap().pins.iter().next().unwrap().clone();
    let t = r1.transform();
    let (a, b) = (t.point(pin.pos), t.point(pin.inner_end()));
    let middle = Point::new((a.x + b.x) / 2, (a.y + b.y) / 2);
    let target = probe_target(p, conn, view, middle, tol, false).unwrap();
    assert_eq!(target, ProbeTarget::Pin { symbol: r1.id, pin: pin.id });
    assert_eq!(target.describe(p), format!("Current into R1 pin {}", pin.key()));
    assert!(!matches!(probe_target(p, conn, view, a, tol, false), Some(ProbeTarget::Pin { .. })));
    assert_eq!(pin_positions(p, r1.id).iter().find(|q| q.0 == pin.id).unwrap().2, a);

    // Alt: the part under the pointer, its power; nothing off the parts.
    let part = probe_target(p, conn, view, middle, tol, true).unwrap();
    assert_eq!(part, ProbeTarget::Part { component: r1.component });
    assert_eq!(part.describe(p), "Power of R1");
    assert_eq!(part.probe(), Probe::Power { component: r1.component });
    assert_eq!(probe_target(p, conn, view, Point::mm(-500.0, -500.0), tol, true), None);
}

#[test]
fn a_differential_target_names_both_nets() {
    let at = ObjectId::Wire(Id::new());
    let t = ProbeTarget::Diff { net: "OUT".into(), at, reference: "IN".into(), ref_at: at };
    let p = demo_project();
    assert_eq!(t.describe(&p), "Voltage of OUT to IN");
    let Probe::Voltage { net, reference: Some(r), .. } = t.probe() else { panic!() };
    assert_eq!((net.as_str(), r.net.as_str()), ("OUT", "IN"));
}

#[test]
fn the_overlay_draws_on_the_sheets_and_clears() {
    let mut doc = Document::new(demo_project());
    let sheet = doc.project().schematic.wires.iter().next().unwrap().sheet;
    let net = doc
        .connectivity()
        .net_of(ObjectId::Wire(doc.project().schematic.wires.iter().next().unwrap().id))
        .unwrap()
        .name
        .clone();
    let count = |doc: &Document| doc.view(view::ViewId::Sheet(sheet)).scene.instance_count();
    let before = count(&doc);
    doc.set_sim_overlay(SimOverlay {
        nets: vec![(net.clone(), render::Color::rgb(255, 0, 0))],
        voltages: vec![(net, "4.2 V".into())],
        parts: doc.project().schematic.components.iter().map(|c| (c.id, render::Color::rgb(0, 0, 255))).collect(),
        ..Default::default()
    });
    assert!(count(&doc) > before);
    doc.set_sim_overlay(SimOverlay::default());
    assert_eq!(count(&doc), before);
}

/// Probes and values go on the Simulation layer (in the layer menu, never printed),
/// hover marks on the hover overlay.
#[test]
fn the_overlay_draws_on_the_simulation_layer() {
    let mut doc = Document::new(demo_project());
    let wire = doc.project().schematic.wires.iter().next().unwrap().clone();
    let net = doc.connectivity().net_of(ObjectId::Wire(wire.id)).unwrap().name.clone();
    let id = view::ViewId::Sheet(wire.sheet);
    let layer = {
        let v = doc.view(id);
        let layer = (0..v.scene.layer_count() as u16).find(|&i| v.scene.layer(i).name == "Simulation").unwrap();
        assert!(v.layer_menu().iter().any(|e| e.layer == layer));
        layer
    };
    // Freed slots stay behind as empty instances until the layer is compacted.
    let on_layer = |doc: &Document| {
        doc.view(id).scene.instances(layer).iter().filter(|i| i.kind != render::Instance::EMPTY.kind).count()
    };
    let printed = doc.view(id).drawing().len();
    doc.set_sim_overlay(SimOverlay { voltages: vec![(net.clone(), "4.2 V".into())], ..Default::default() });
    assert!(on_layer(&doc) > 0);
    assert_eq!(doc.view(id).drawing().len(), printed);
    doc.set_sim_overlay(SimOverlay { hover_nets: vec![net], ..Default::default() });
    assert_eq!(on_layer(&doc), 0);
}

/// The tags overlapping texts (part references and values, labels, free texts) or each
/// other, with every net's voltage and every two-pin part's current shown at once.
fn crowding(doc: &mut Document) -> (usize, usize, usize) {
    let p = doc.project();
    let conn = doc.connectivity();
    let mut nets: Vec<String> =
        p.schematic.wires.iter().filter_map(|w| conn.net_of(ObjectId::Wire(w.id)).map(|n| n.name.clone())).collect();
    nets.sort();
    nets.dedup();
    let currents: Vec<(Id, Id, String)> = p
        .schematic
        .symbols
        .iter()
        .filter_map(|s| {
            let pins = &symbol_of_instance(p, s)?.pins;
            (pins.len() == 2).then(|| (s.id, pins.iter().next().unwrap().id, "1.23 mA".to_string()))
        })
        .collect();
    let voltages = nets.into_iter().map(|n| (n, "12.3 V".to_string())).collect();
    doc.set_sim_overlay(SimOverlay { voltages, currents, ..Default::default() });
    let (mut on_text, mut on_tag, mut tags) = (0, 0, 0);
    let sheets: Vec<Id> = doc.project().schematic.sheets.iter().map(|s| s.id).collect();
    for sheet in sheets {
        let v = doc.view(view::ViewId::Sheet(sheet));
        tags += v.sim_tags.len();
        for (i, t) in v.sim_tags.iter().enumerate() {
            let texts = v
                .index
                .in_rect(*t, true)
                .into_iter()
                .filter(|id| matches!(id, ObjectId::SymbolField(_) | ObjectId::Label(_) | ObjectId::SchText(_)));
            on_text += usize::from(texts.count() > 0);
            on_tag += usize::from(
                v.sim_tags[..i]
                    .iter()
                    .any(|u| u.min.x < t.max.x && t.min.x < u.max.x && u.min.y < t.max.y && t.min.y < u.max.y),
            );
        }
    }
    (tags, on_text, on_tag)
}

#[test]
fn value_tags_keep_clear_of_texts_and_each_other() {
    for example in view::demo::Example::ALL {
        let mut doc = Document::new(example.project());
        let (tags, on_text, on_tag) = crowding(&mut doc);
        let name = example.name();
        assert!(tags > 10, "{name}: {tags} tags");
        assert_eq!(on_tag, 0, "{name}: tags on tags");
        // The LED demo's +5V label next to J1 leaves no free spot for two of its tags (they
        // touch it by a sliver); before, 5 of 13 sat on texts there and 9 of 41 in the amp.
        let most = if name == "led" { 2 } else { 0 };
        assert!(on_text <= most, "{name}: {on_text} of {tags} tags on texts");
    }
}

/// Switch and potentiometer controls go next to their part, clear of it, and say
/// where they are for clicks; a slider maps its track to 0…1.
#[test]
fn controls_sit_by_their_part_and_take_clicks() {
    let mut doc = Document::new(demo_project());
    let r1 = symbol(&doc, "R1");
    let id = view::ViewId::Sheet(r1.sheet);
    doc.set_sim_overlay(SimOverlay {
        controls: vec![view::sim_overlay::Control { component: r1.component, kind: SimControl::Position, value: 0.3 }],
        ..Default::default()
    });
    let v = doc.view(id);
    let [area] = v.sim_controls.as_slice() else { panic!("{:?}", v.sim_controls) };
    assert_eq!((area.component, area.kind), (r1.component, SimControl::Position));
    let body = v.index.bounds(ObjectId::Symbol(r1.id)).unwrap();
    let r = area.rect;
    assert!(r.max.x <= body.min.x || r.min.x >= body.max.x || r.max.y <= body.min.y || r.min.y >= body.max.y);
    let mid = Point::new((r.min.x + r.max.x) / 2, (r.min.y + r.max.y) / 2);
    assert_eq!(v.sim_control_at(mid).map(|a| a.component), Some(r1.component));
    assert!(v.sim_control_at(Point::new(r.max.x + 1_000_000, mid.y)).is_none());
    let (a, b) = area.track.unwrap();
    assert_eq!(area.value_at(Point::new(a - 5_000_000, mid.y)), 0.0);
    assert_eq!(area.value_at(Point::new(b + 5_000_000, mid.y)), 1.0);
    assert!((area.value_at(Point::new((a + b) / 2, mid.y)) - 0.5).abs() < 1e-6);

    // A switch: no track; gone with the overlay.
    doc.set_sim_overlay(SimOverlay {
        controls: vec![view::sim_overlay::Control { component: r1.component, kind: SimControl::Switch, value: 1.0 }],
        ..Default::default()
    });
    assert_eq!(doc.view(id).sim_controls[0].track, None);
    doc.set_sim_overlay(SimOverlay::default());
    assert!(doc.view(id).sim_controls.is_empty());
}
