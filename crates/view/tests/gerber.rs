//! Gerber export of the demo board through the document.

use gerber::{ExportError, FileKind, design_hash, read_manifest};
use model::{LayerId, Project};
use view::Document;
use view::demo::demo_project;

fn saved_demo() -> (tempfile::TempDir, Document) {
    let dir = tempfile::tempdir().unwrap();
    let mut doc = Document::new(demo_project());
    doc.save_as(dir.path().join("demo.pcbproj")).unwrap();
    (dir, doc)
}

#[test]
fn exports_all_layers_and_drills() {
    let (_tmp, mut doc) = saved_demo();
    let export = doc.export_gerber().unwrap();
    let names: Vec<&str> = export.manifest.files.iter().map(|f| f.name.as_str()).collect();
    let stem = doc.project().name.clone();
    for suffix in ["F_Cu.gbr", "B_Cu.gbr", "F_Mask.gbr", "B_Paste.gbr", "F_Silkscreen.gbr", "Edge_Cuts.gbr", "PTH.drl"]
    {
        assert!(names.contains(&format!("{stem}-{suffix}").as_str()), "{suffix} in {names:?}");
    }
    for name in &names {
        assert!(export.dir.join(name).is_file(), "{name}");
    }
    let project_dir = doc.path.clone().unwrap();
    assert!(project_dir.join("output").join(&export.manifest.zip).is_file());
    let manifest = read_manifest(&project_dir).unwrap();
    assert_eq!(manifest, export.manifest);
    assert_eq!(manifest.hash, design_hash(doc.project()));
    // The demo board's unrouted connections come back as warnings.
    assert!(!export.warnings.is_empty());

    let read = |kind: FileKind| {
        let f = manifest.files.iter().find(|f| f.kind == kind).unwrap();
        std::fs::read_to_string(export.dir.join(&f.name)).unwrap()
    };
    let top = read(FileKind::Layer(LayerId::TOP_COPPER));
    assert!(top.contains("%TF.FileFunction,Copper,L1,Top*%"));
    assert!(top.contains("%TA.AperFunction,SMDPad,CuDef*%"));
    assert!(top.contains("D03*"), "pads are flashed");
    let zone = doc.project().board.zones.iter().next().unwrap().layer;
    assert!(read(FileKind::Layer(zone)).contains("G36*"), "the zone fill is a region");
    assert!(read(FileKind::Layer(LayerId::TOP_MASK)).contains("%TF.FilePolarity,Negative*%"));
    assert!(read(FileKind::Layer(LayerId::OUTLINE)).contains("%TA.AperFunction,Profile*%"));
    let drills = read(FileKind::PlatedDrill);
    let holes = drills.lines().filter(|l| l.starts_with('X')).count();
    let p = doc.project();
    let th = p
        .board
        .footprints
        .iter()
        .map(|f| editor_core::query::footprint_of_instance(p, f).unwrap())
        .flat_map(|fp| fp.pads.iter())
        .filter(|pad| matches!(pad.kind, model::PadKind::ThroughHole { .. }))
        .count();
    assert_eq!(holes, th + p.board.vias.len());
}

#[test]
fn refuses_without_outline_or_folder() {
    let mut doc = Document::new(demo_project());
    assert!(matches!(doc.export_gerber(), Err(ExportError::NotSaved)));
    let dir = tempfile::tempdir().unwrap();
    let mut empty = Document::new(Project::new("empty"));
    empty.save_as(dir.path().join("empty.pcbproj")).unwrap();
    assert!(matches!(empty.export_gerber(), Err(ExportError::NoOutline)));
}

#[test]
fn export_replaces_the_folder() {
    let (_tmp, mut doc) = saved_demo();
    let dir = doc.export_gerber().unwrap().dir;
    std::fs::write(dir.join("stale.gbr"), "x").unwrap();
    doc.export_gerber().unwrap();
    assert!(!dir.join("stale.gbr").exists());
}

/// Written files read back cover every pad (copper, mask, paste) just inside its edge and
/// not just outside: rotations, mirroring, round rectangles, ovals and polygons.
#[test]
fn pads_round_trip() {
    use editor_core::fab::{pad_copper_layers, pad_mask_layers, pad_paste_layer};
    use gerber::reader::parse_gerber;
    use model::*;

    let mut p = demo_project();
    for fp in p.library.footprints.iter_mut() {
        let mut pads: Vec<Pad> = fp.pads.iter().cloned().collect();
        if pads.len() != 2 || !matches!(pads[0].kind, PadKind::Smd) {
            continue;
        }
        pads[0].rotation = Angle::deg(30.0);
        pads[1].shape = PadShape::Oval;
        pads[1].size = Size::mm(0.6, 1.2);
        let tri = vec![Point::mm(-0.5, -0.4), Point::mm(0.5, -0.4), Point::mm(0.0, 0.6)];
        let mut extra = pads[0].clone();
        extra.id = Id::new();
        extra.name = "3".into();
        extra.pos = Point::mm(0.0, 3.0);
        extra.rotation = Angle::deg(20.0);
        extra.shape = PadShape::Polygon { points: tri };
        pads.push(extra);
        fp.pads = Store::default();
        for pad in pads {
            fp.pads.insert(pad);
        }
    }
    let ids: Vec<Id> = p.board.footprints.iter().map(|f| f.id).collect();
    p.board.footprints.get_mut(ids[1]).unwrap().rotation = Angle::deg(45.0);
    p.board.footprints.get_mut(ids[2]).unwrap().side = Side::Bottom;
    p.board.tracks = Store::default();
    p.board.vias = Store::default();
    p.board.zones = Store::default();
    p.board.rules.mask_expansion = mm(0.1);
    p.board.rules.paste_margin = -mm(0.05);

    let mut doc = Document::new(p);
    let copper = doc.current_copper().clone();
    let files = gerber::generate(doc.project(), &copper).unwrap();
    let image = |layer: LayerId| {
        let f = files.iter().find(|f| f.kind == FileKind::Layer(layer)).unwrap();
        parse_gerber(&f.content).unwrap_or_else(|e| panic!("{}: {e}", f.name))
    };
    let p = doc.project();
    let copper: Vec<LayerId> = p.board.copper_layers().map(|l| l.id).collect();
    let mut images = std::collections::HashMap::new();
    let e = mm(0.02);
    let mut checked = 0;
    for inst in p.board.footprints.iter() {
        let fp = editor_core::query::footprint_of_instance(p, inst).unwrap();
        let t = inst.transform();
        for pad in fp.pads.iter() {
            let mut layers: Vec<(LayerId, Coord)> =
                pad_copper_layers(&pad.kind, inst.side, &copper).into_iter().map(|l| (l, 0)).collect();
            layers.extend(pad_mask_layers(&pad.kind, inst.side).into_iter().map(|l| (l, p.board.rules.mask_expansion)));
            layers.extend(pad_paste_layer(&pad.kind, inst.side).map(|l| (l, p.board.rules.paste_margin)));
            for (layer, margin) in layers {
                let img = images.entry(layer).or_insert_with(|| image(layer));
                let at = |q: Point| t.point(pad.pos + q.rotated(pad.rotation));
                let (w, h) = match pad.shape {
                    PadShape::Circle => (pad.size.w.min(pad.size.h), pad.size.w.min(pad.size.h)),
                    PadShape::Polygon { .. } => {
                        // The centroid of the triangle is inside; far away is not.
                        let c = at(Point::mm(0.0, (-0.4 - 0.4 + 0.6) / 3.0));
                        assert!(img.covers(c), "{} {layer:?} polygon pad centre", inst.id);
                        assert!(!img.covers(at(Point::mm(0.0, 1.0))), "{layer:?} beyond the polygon tip");
                        checked += 1;
                        continue;
                    }
                    _ => (pad.size.w, pad.size.h),
                };
                let (hw, hh) = (w / 2 + margin, h / 2 + margin);
                for (q, inside) in [
                    (Point::ZERO, true),
                    (Point::new(hw - e, 0), true),
                    (Point::new(-hw + e, 0), true),
                    (Point::new(0, hh - e), true),
                    (Point::new(0, -hh + e), true),
                    (Point::new(hw + e, 0), false),
                    (Point::new(-hw - e, 0), false),
                    (Point::new(0, hh + e), false),
                    (Point::new(0, -hh - e), false),
                ] {
                    assert_eq!(
                        img.covers(at(q)),
                        inside,
                        "pad {} {:?} on {layer:?} at local {q:?}",
                        pad.name,
                        pad.shape
                    );
                    checked += 1;
                }
            }
        }
    }
    assert!(checked > 100, "{checked}");
}

/// Tracks, vias, drills and the zone fill of the demo read back where they are.
#[test]
fn copper_round_trip() {
    use gerber::reader::{parse_excellon, parse_gerber};
    let mut doc = Document::new(demo_project());
    let copper = doc.current_copper().clone();
    let p = doc.project();
    let files = gerber::generate(p, &copper).unwrap();
    let find = |kind: FileKind| files.iter().find(|f| f.kind == kind).unwrap();
    for t in p.board.tracks.iter() {
        let img = parse_gerber(&find(FileKind::Layer(t.layer)).content).unwrap();
        let mid = model::Point::new((t.a.x + t.b.x) / 2, (t.a.y + t.b.y) / 2);
        assert!(img.covers(mid), "track {:?}", t.id);
    }
    let drills = parse_excellon(&find(FileKind::PlatedDrill).content).unwrap();
    for v in p.board.vias.iter() {
        assert!(drills.covers(v.pos), "via drill");
    }
    for fill in &copper.fills {
        let img = parse_gerber(&find(FileKind::Layer(fill.layer)).content).unwrap();
        let regions = img.objects.iter().filter(|o| o.dcode.is_none()).count();
        assert_eq!(regions, fill.shapes.len());
        // Sampled on a grid, the regions (bridged holes) cover what the fill covers.
        use gerber::reader::Prim;
        let regions: Vec<&Prim> = img.objects.iter().filter(|o| o.dcode.is_none()).flat_map(|o| &o.prims).collect();
        let bounds =
            model::Rect::union_all(fill.shapes.iter().flatten().flatten().map(|&q| model::Rect::from_points(q, q)))
                .unwrap();
        let step = 97_000; // not a multiple of the 0.05 mm grid things sit on
        let mut samples = 0;
        let mut x = bounds.min.x;
        while x <= bounds.max.x {
            let mut y = bounds.min.y;
            while y <= bounds.max.y {
                let q = model::Point::new(x, y);
                let expected = fill.shapes.iter().any(|shape| {
                    Prim::Polygon(shape[0].clone()).contains(q)
                        && !shape[1..].iter().any(|h| Prim::Polygon(h.clone()).contains(q))
                });
                assert_eq!(regions.iter().any(|r| r.contains(q)), expected, "{q:?}");
                samples += 1;
                y += step;
            }
            x += step;
        }
        assert!(samples > 1000);
    }
}

/// The viewer reads an export back: every file, labels from the manifest, and what lies
/// under a pad.
#[test]
fn viewer_loads_an_export() {
    let (_tmp, mut doc) = saved_demo();
    let export = doc.export_gerber().unwrap();
    let mut gv = view::GerberView::load(&export.dir).unwrap();
    assert!(gv.errors.is_empty(), "{:?}", gv.errors);
    assert_eq!(gv.files.len(), export.manifest.files.len());
    assert!(gv.files.iter().any(|f| f.label == "Top Copper" && f.layer == Some(LayerId::TOP_COPPER)));
    assert!(gv.files.iter().any(|f| f.drill));
    assert_eq!(gv.layer_menu().len(), gv.files.len());
    // An SMD pad of R1, on top copper (drawn above the bottom layers).
    let p = doc.project();
    let r1 = p.schematic.component_by_refdes("R1").unwrap().id;
    let inst = p.board.footprints.iter().find(|f| f.component == r1).unwrap();
    let pad = editor_core::query::footprint_of_instance(p, inst).unwrap().pads.iter().next().unwrap();
    let at = inst.transform().point(pad.pos);
    gv.show_only(Some(&gv.files.iter().find(|f| f.label == "Top Copper").unwrap().name.clone()));
    let text = gv.describe([at.x as f64 * 1e-6, at.y as f64 * 1e-6]).unwrap();
    assert!(text.contains("F_Cu") && text.contains("SMDPad"), "{text}");
    // The measure tool snaps to the pad's flash centre and to track ends.
    let near = at + model::Point::new(model::mm(0.05), 0);
    assert!(gv.snap_points(near, model::mm(0.2)).contains(&at));
    gv.show_only(None);
    assert!(gv.describe([-1000.0, -1000.0]).is_none());
    let track = p.board.tracks.iter().next().unwrap();
    let ends = gv.snap_points(track.a, model::mm(0.01));
    assert!(ends.contains(&track.a), "{ends:?}");
    // Hidden files give none.
    gv.show_only(Some("nothing"));
    assert!(gv.snap_points(at, model::mm(0.2)).is_empty());
}
