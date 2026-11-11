//! Theme switching (own test binary: the palette is global).

use std::collections::BTreeMap;

use render::scene::LayerData;
use view::demo::demo_project;
use view::style::{self, Theme};
use view::{Document, View, ViewId};

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
