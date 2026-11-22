//! Net names on pads and tracks: only zoomed in, only near the viewport, redrawn when
//! the viewport leaves what was drawn or the board changes.

use editor_core::Command;
use editor_core::copper::CopperItem;
use model::{ObjectId, Point, Rect, mm};
use render::Instance;
use view::demo::demo_project;
use view::net_labels::{LABEL_ZOOM, PAD_LABEL_ZOOM};
use view::{Document, ViewId};

fn count(doc: &Document, name: &str) -> usize {
    let scene = &doc.view(ViewId::Board).scene;
    let layer = (0..scene.layer_count() as u16).find(|&i| scene.layer(i).name == name).unwrap();
    scene.instances(layer).iter().filter(|i| **i != Instance::EMPTY).count()
}

fn layer_shown(doc: &Document, name: &str, zoom: f64) -> bool {
    let scene = &doc.view(ViewId::Board).scene;
    let layer = (0..scene.layer_count() as u16).find(|&i| scene.layer(i).name == name).unwrap();
    scene.visible_layers(zoom)[layer as usize]
}

/// A viewport of 1600×1000 points centred on `c` at `zoom`.
fn viewport(c: Point, zoom: f64) -> Rect {
    let (hw, hh) = (mm(800.0 / zoom), mm(500.0 / zoom));
    Rect { min: Point::new(c.x - hw, c.y - hh), max: Point::new(c.x + hw, c.y + hh) }
}

fn doc() -> Document {
    let mut doc = Document::new(demo_project());
    doc.update_connectivity(false);
    doc
}

/// A footprint with a pad on a net, and where that pad is.
fn connected_pad(doc: &Document) -> Point {
    let p = doc.project();
    for f in p.board.footprints.iter() {
        let fp = editor_core::query::footprint_of_instance(p, f).unwrap();
        for pad in fp.pads.iter() {
            if doc.copper().net_of(CopperItem::Pad { footprint: f.id, pad: pad.id }).is_some() {
                return f.transform().point(pad.pos);
            }
        }
    }
    panic!("no connected pad");
}

#[test]
fn pads_show_their_net_zoomed_in() {
    let mut doc = doc();
    let pad = connected_pad(&doc);
    // Zoomed out: nothing.
    assert!(!doc.update_net_labels(viewport(pad, LABEL_ZOOM / 2.0), LABEL_ZOOM / 2.0));
    assert_eq!(count(&doc, "Pad net names"), 0);
    // Pad numbers alone: the static ones.
    doc.update_net_labels(viewport(pad, LABEL_ZOOM * 1.2), LABEL_ZOOM * 1.2);
    assert_eq!(count(&doc, "Pad net names"), 0);
    assert!(layer_shown(&doc, "Pad numbers", LABEL_ZOOM * 1.2));
    // Further in: the labels take over the numbers.
    let zoom = PAD_LABEL_ZOOM * 3.0;
    assert!(doc.update_net_labels(viewport(pad, zoom), zoom));
    assert!(count(&doc, "Pad net names") > 0);
    assert!(!layer_shown(&doc, "Pad numbers", zoom));
    // Hiding the pad numbers hides them.
    let scene = &mut doc.view_mut(ViewId::Board).scene;
    let numbers = (0..scene.layer_count() as u16).find(|&i| scene.layer(i).name == "Pad numbers").unwrap();
    scene.layer_mut(numbers).visible = false;
    assert!(doc.update_net_labels(viewport(pad, zoom), zoom));
    assert_eq!(count(&doc, "Pad net names"), 0);
}

#[test]
fn only_what_is_near_the_viewport_is_labelled() {
    let mut doc = doc();
    let pad = connected_pad(&doc);
    let zoom = PAD_LABEL_ZOOM * 3.0;
    let far = Point::new(pad.x + mm(500.0), pad.y);
    doc.update_net_labels(viewport(far, zoom), zoom);
    assert_eq!(count(&doc, "Pad net names"), 0);
    doc.update_net_labels(viewport(pad, zoom), zoom);
    let near = count(&doc, "Pad net names");
    assert!(near > 0);
    // A small pan stays inside what was drawn: nothing redrawn.
    let nudge = Point::new(pad.x + mm(1.0), pad.y);
    assert!(!doc.update_net_labels(viewport(nudge, zoom), zoom));
    // A zoom step or an edit redraws.
    assert!(doc.update_net_labels(viewport(nudge, zoom * 1.5), zoom * 1.5));
    let track = doc.project().board.tracks.iter().next().unwrap().id;
    doc.execute(Command::Delete { ids: vec![ObjectId::Track(track)] }).unwrap();
    assert!(doc.update_net_labels(viewport(nudge, zoom * 1.5), zoom * 1.5));
}

#[test]
fn tracks_are_labelled_once_the_text_fits() {
    let mut doc = doc();
    // The longest straight track with a net.
    let (tr, _) = doc
        .project()
        .board
        .tracks
        .iter()
        .filter(|t| t.mid.is_none())
        .filter_map(|t| Some((t.clone(), doc.copper().net_of(CopperItem::Track(t.id))?.to_string())))
        .max_by_key(|(t, _)| (t.b - t.a).length() as i64)
        .unwrap();
    let mid = Point::new((tr.a.x + tr.b.x) / 2, (tr.a.y + tr.b.y) / 2);
    let layer = format!("{} net names", doc.project().board.layers.iter().find(|l| l.id == tr.layer).unwrap().name);
    // Too small on screen: 0.6 of the width under 8 pixels (on any track).
    let widest = doc.project().board.tracks.iter().map(|t| t.width).max().unwrap();
    let small = 7.0 / (widest as f64 * 1e-6 * 0.6);
    doc.update_net_labels(viewport(mid, small.max(LABEL_ZOOM)), small.max(LABEL_ZOOM));
    assert_eq!(count(&doc, &layer), 0);
    let large = 20.0 / (tr.width as f64 * 1e-6 * 0.6);
    doc.update_net_labels(viewport(mid, large), large);
    assert!(count(&doc, &layer) > 0, "track {tr:?}");
}
