//! The findings under the pointer: what the canvas' hover tip shows.

use editor_core::Command;
use editor_core::copper::CopperItem;
use model::*;
use view::demo::demo_project;
use view::findings::at;
use view::{Document, ViewId};

const TOL: Coord = 100_000;

#[test]
fn schematic_findings_at_their_markers() {
    let mut doc = Document::new(demo_project());
    let root = ViewId::root(doc.project());
    // A power flag touching nothing: a warning, marked at the flag.
    let sheet = root.sheet();
    doc.execute(Command::AddPowerFlag { sheet, pos: Point::mm(20.32, 20.32), rotation: Angle::ZERO }).unwrap();
    doc.current_connectivity();
    let problem = doc
        .connectivity()
        .problems
        .iter()
        .find(|p| p.pos == Some(Point::mm(20.32, 20.32)))
        .cloned()
        .expect("a lone power flag");
    let pos = problem.pos.unwrap();
    let found = at(&doc, root, pos, TOL, None);
    assert!(found.iter().any(|f| f.message == problem.message), "{found:?}");
    // Hovering the object it is about shows it too, anywhere on it.
    let found = at(&doc, root, Point::mm(-500.0, -500.0), TOL, Some(problem.objects[0]));
    assert!(found.iter().any(|f| f.message == problem.message), "{found:?}");
    // Nothing far from any marker.
    assert!(at(&doc, root, Point::mm(-500.0, -500.0), TOL, None).is_empty());
    // The markers are on the ERC markers layer (in the layer menu).
    let v = doc.view(root);
    let erc = (0..v.scene.layer_count() as u16).find(|&i| v.scene.layer(i).name == "ERC markers").unwrap();
    assert!(v.layer_menu().iter().any(|e| e.layer == erc));
    assert!(v.scene.instances(erc).iter().any(|i| i.kind != render::Instance::EMPTY.kind));
    // The ERC markers layer hidden: nothing.
    doc.view_mut(root).set_hidden_layers(&["ERC markers".into()]);
    assert!(!doc.view(root).markers_visible());
    assert!(at(&doc, root, pos, TOL, None).iter().all(|f| f.message != problem.message));
}

#[test]
fn flagged_tracks_and_drc_violations() {
    let mut doc = Document::new(demo_project());
    // Two pads of different nets joined by a track: a short.
    let copper = doc.current_copper();
    let pads: Vec<_> =
        copper.items.iter().filter(|c| matches!(c.item, CopperItem::Pad { .. }) && c.net.is_some()).collect();
    let a = pads[0];
    let b = pads.iter().find(|p| p.net != a.net && p.layers == a.layers).unwrap();
    let names = [&copper.nets[a.net.unwrap() as usize], &copper.nets[b.net.unwrap() as usize]];
    let short = Track::straight(LayerId::TOP_COPPER, a.anchors[0], b.anchors[0], mm(0.2));
    let (short_id, names) = (short.id, names.map(|s| s.clone()));
    // A track touching no pad.
    let stray = Track::straight(LayerId::TOP_COPPER, Point::mm(-50.0, -50.0), Point::mm(-40.0, -50.0), mm(0.2));
    let stray_id = stray.id;
    doc.execute(Command::Insert { objects: vec![Object::Track(short), Object::Track(stray)] }).unwrap();
    doc.current_copper();

    let found = at(&doc, ViewId::Board, Point::mm(-500.0, -500.0), TOL, Some(ObjectId::Track(short_id)));
    let msg = &found[0].message;
    assert!(msg.starts_with("Short: ") && names.iter().all(|n| msg.contains(n.as_str())), "{found:?}");
    assert_eq!(found[0].severity, Severity::Error);
    let found = at(&doc, ViewId::Board, Point::mm(-45.0, -50.0), TOL, Some(ObjectId::Track(stray_id)));
    assert!(found[0].message.starts_with("Unconnected copper"), "{found:?}");
    assert!(at(&doc, ViewId::Board, Point::mm(-500.0, -500.0), TOL, None).is_empty());

    // DRC markers.
    let v = doc.run_drc().violations.iter().find(|v| !v.excluded).cloned().unwrap();
    let found = at(&doc, ViewId::Board, v.pos, TOL, None);
    assert!(found.iter().any(|f| f.message == v.message), "{found:?}");
    // Errors first.
    assert!(found.windows(2).all(|w| w[0].severity == Severity::Error || w[1].severity != Severity::Error));
    // On the DRC markers layer (in the layer menu, never printed); hidden, the tip
    // leaves them out.
    let board = doc.view(ViewId::Board);
    let drc = (0..board.scene.layer_count() as u16).find(|&i| board.scene.layer(i).name == "DRC markers").unwrap();
    assert!(board.layer_menu().iter().any(|e| e.layer == drc));
    let live: Vec<_> = board.scene.instances(drc).iter().filter(|i| i.kind != render::Instance::EMPTY.kind).collect();
    assert!(!live.is_empty());
    assert!(!board.drawing().iter().any(|i| live.contains(&i)));
    doc.view_mut(ViewId::Board).set_hidden_layers(&["DRC markers".into()]);
    assert!(at(&doc, ViewId::Board, v.pos, TOL, None).iter().all(|f| f.message != v.message));
}
