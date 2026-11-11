//! Layer menus: every user-facing layer once, in the editor's colours.

use editor_core::Command;
use editor_core::query::footprint_of_instance;
use model::{LayerId, mm};
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
