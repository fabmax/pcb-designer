//! Layer menus: every user-facing layer once, in the editor's colours.

use editor_core::Command;
use editor_core::query::footprint_of_instance;
use model::{LayerId, ObjectId, mm};
use view::demo::demo_project;
use view::style::board_layer_color;
use view::{Document, ViewId};

#[test]
fn layer_menus_cover_all_user_layers() {
    let doc = Document::new(demo_project());
    for kind in [doc.root(), ViewId::Board] {
        let view = doc.view(kind);
        let mut listed: Vec<u16> = view.layer_menu().iter().map(|e| e.layer).collect();
        listed.sort();
        assert_eq!(listed, view.user_layers().collect::<Vec<_>>(), "{kind:?}");
    }
    let board = doc.view(ViewId::Board).layer_menu();
    // Copper first, top to bottom, in the colours the board is drawn in.
    assert_eq!(board[0].color, Some(board_layer_color(LayerId::TOP_COPPER)));
    assert_eq!(board[1].color, Some(board_layer_color(LayerId::BOTTOM_COPPER)));
    // Groups only ever increase (separators between them).
    assert!(board.windows(2).all(|w| w[0].group <= w[1].group));
}

/// Pads open the solder mask (both sides for through-hole pads) and SMD pads get paste;
/// the layers redraw when the rules change.
#[test]
fn pads_draw_mask_and_paste() {
    let mut doc = Document::new(demo_project());
    let count = |doc: &Document, name: &str| {
        let scene = &doc.view(ViewId::Board).scene;
        let layer = (0..scene.layer_count() as u16).find(|&i| scene.layer(i).name == name).unwrap();
        scene.snapshot().layers[layer as usize].len
    };
    let pads: usize = doc
        .project()
        .board
        .footprints
        .iter()
        .map(|f| footprint_of_instance(doc.project(), f).unwrap().pads.len())
        .sum();
    assert!(count(&doc, "Top Solder Mask") > 0);
    assert!(count(&doc, "Top Paste") > 0);
    assert!(count(&doc, "Top Solder Mask") <= pads);
    let mut rules = doc.project().board.rules.clone();
    rules.paste_margin = -mm(5.0);
    doc.execute(Command::SetDesignRules { rules }).unwrap();
    assert_eq!(count(&doc, "Top Paste"), 0, "shrunk to nothing");
}

/// A selected track keeps its layer's colour (with a rim), not the accent colour, which
/// would read as another layer's.
#[test]
fn selected_tracks_keep_their_layer_colour() {
    let mut doc = Document::new(demo_project());
    let track = doc.project().board.tracks.iter().find(|t| t.layer == LayerId::TOP_COPPER).unwrap().id;
    let mut it = view::Interaction::new(mm(0.5));
    it.set_selection(&mut doc.target(ViewId::Board), vec![ObjectId::Track(track)]);
    let scene = &doc.view(ViewId::Board).scene;
    let selection = (0..scene.layer_count() as u16).find(|&i| scene.layer(i).name == "Selection").unwrap();
    let colors: Vec<_> =
        scene.instances(selection).iter().filter(|i| i.kind != render::Instance::EMPTY.kind).map(|i| i.color).collect();
    let (core, rim) = view::style::selected_copper(board_layer_color(LayerId::TOP_COPPER));
    assert_eq!(colors, [rim, core]);
    assert!(!colors.contains(&view::style::pal().selection));
}
