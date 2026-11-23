//! Theme switching (own test binary: the palette is global).

use std::collections::BTreeMap;

use render::scene::LayerData;
use view::demo::demo_project;
use view::style::{self, Theme};
use view::{Document, View, ViewId};

/// The palette and layer colours are global: tests take turns.
static ONE_AT_A_TIME: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Instance colors per layer (order-independent), excluding empty slots.
fn colors(view: &View) -> Vec<BTreeMap<[u8; 4], usize>> {
    view.scene
        .snapshot()
        .layers
        .into_iter()
        .map(|l| {
            let mut m = BTreeMap::new();
            if let LayerData::Full { instances, .. } = l.data {
                for i in &instances[..l.len] {
                    if i.kind != render::primitive::kind::EMPTY {
                        *m.entry(i.color.0).or_default() += 1;
                    }
                }
            }
            m
        })
        .collect()
}

#[test]
fn restyle_matches_a_fresh_build() {
    let _one = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
    let mut doc = Document::new(demo_project());
    doc.run_drc();
    let views = [doc.root(), ViewId::Board];
    let dark: Vec<_> = views.map(|k| colors(doc.view(k))).into();

    style::set_theme(Theme::Light);
    doc.restyle();
    let fresh = Document::new(doc.project().clone());
    for (i, k) in views.into_iter().enumerate() {
        let restyled = colors(doc.view(k));
        assert_ne!(restyled, dark[i], "{k:?} changed colors");
        // The fresh document has no DRC markers: compare everything else.
        let mut expected = colors(fresh.view(k));
        let mut got = restyled.clone();
        if k == ViewId::Board {
            let marker = style::pal().drc_marker.0;
            for (a, b) in got.iter_mut().zip(expected.iter_mut()) {
                a.remove(&marker);
                b.remove(&marker);
            }
        }
        assert_eq!(got, expected, "{k:?}");
    }
    assert_eq!(style::background(1.0).color, style::LIGHT.background);

    style::set_theme(Theme::Dark);
    doc.restyle();
    assert_eq!(colors(doc.view(doc.root())), dark[0]);
}

#[test]
fn layer_colors_override_per_theme() {
    let _one = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
    use render::Color;
    use style::{LayerColor, LayerColors, board_layer_color, color_hex, parse_color_hex, set_layer_colors};
    let red = Color::rgba(0xff, 0, 0, 0x80);
    assert_eq!(parse_color_hex(&color_hex(red)), Some(red));
    assert_eq!(parse_color_hex("#00ff00"), Some(Color::rgb(0, 0xff, 0)));
    assert_eq!(parse_color_hex("00ff00"), None);
    assert_eq!(LayerColor::of(model::LayerId(5)), Some(LayerColor::Inner1));

    let silk = model::LayerId::TOP_SILK;
    style::set_theme(Theme::Dark);
    let before = board_layer_color(silk);
    set_layer_colors(BTreeMap::from([(Theme::Light, BTreeMap::from([(LayerColor::TopSilk, red)]))]));
    assert_eq!(board_layer_color(silk), before, "other theme");
    set_layer_colors(BTreeMap::from([(Theme::Dark, BTreeMap::from([(LayerColor::TopSilk, red)]))]));
    assert_eq!(board_layer_color(silk), red);
    // The board view draws with it after a restyle.
    let mut doc = Document::new(demo_project());
    doc.restyle();
    assert!(colors(doc.view(ViewId::Board)).iter().any(|l| l.contains_key(&red.0)), "silkscreen redrawn");
    set_layer_colors(LayerColors::new());
}

#[test]
fn editors_draw_in_their_own_theme() {
    let _one = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
    use style::Editor;
    style::set_theme(Theme::Dark);
    let mut doc = Document::new(demo_project());
    let sheet = colors(doc.view(doc.root()));
    let board = colors(doc.view(ViewId::Board));

    // A light PCB in the dark app: only the board changes.
    style::set_editor_theme(Editor::Pcb, Some(Theme::Light));
    assert_eq!(style::editor_theme(Editor::Pcb), Theme::Light);
    assert_eq!(style::editor_theme(Editor::Schematic), Theme::Dark, "follows the app");
    doc.restyle_editor(Editor::Pcb);
    assert_eq!(colors(doc.view(doc.root())), sheet);
    assert_ne!(colors(doc.view(ViewId::Board)), board);
    style::set_theme(Theme::Light);
    let light_board = colors(Document::new(doc.project().clone()).view(ViewId::Board));
    style::set_theme(Theme::Dark);
    assert_eq!(colors(doc.view(ViewId::Board)), light_board, "as in the light app");

    // Scopes nest and end; a forced palette (prints) wins over them.
    assert_eq!(style::pal().background, style::DARK.background);
    style::with_editor(Editor::Pcb, || {
        assert_eq!(style::theme(), Theme::Light);
        assert_eq!(style::with_editor(Editor::Schematic, style::theme), Theme::Dark);
        assert_eq!(style::theme(), Theme::Light);
    });
    assert_eq!(style::theme(), Theme::Dark);
    style::with_palette(&style::DARK, || assert_eq!(style::with_editor(Editor::Pcb, style::theme), Theme::Dark));

    style::set_editor_theme(Editor::Pcb, None);
    doc.restyle_editor(Editor::Pcb);
    assert_eq!(colors(doc.view(ViewId::Board)), board);
}
