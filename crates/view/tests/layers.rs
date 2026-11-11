//! Layer menus: every user-facing layer once, in the editor's colours.

use model::LayerId;
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
