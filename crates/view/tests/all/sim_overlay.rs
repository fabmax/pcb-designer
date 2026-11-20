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
    assert_eq!(probe_target(p, conn, view, mid, tol), Some(ProbeTarget::Net { net, at: ObjectId::Wire(w.id) }));

    // The middle of a pin: the current into it; its end is not the pin.
    let r1 = symbol(&doc, "R1");
    let view = doc.view(view::ViewId::Sheet(r1.sheet));
    let pin = &symbol_of_instance(p, &r1).unwrap().pins.iter().next().unwrap().clone();
    let t = r1.transform();
    let (a, b) = (t.point(pin.pos), t.point(pin.inner_end()));
    let middle = Point::new((a.x + b.x) / 2, (a.y + b.y) / 2);
    let target = probe_target(p, conn, view, middle, tol).unwrap();
    assert_eq!(target, ProbeTarget::Pin { symbol: r1.id, pin: pin.id });
    assert_eq!(target.describe(p), format!("Current into R1 pin {}", pin.key()));
    assert!(!matches!(probe_target(p, conn, view, a, tol), Some(ProbeTarget::Pin { .. })));
    assert_eq!(pin_positions(p, r1.id).iter().find(|q| q.0 == pin.id).unwrap().2, a);
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
        ..Default::default()
    });
    assert!(count(&doc) > before);
    doc.set_sim_overlay(SimOverlay::default());
    assert_eq!(count(&doc), before);
}
