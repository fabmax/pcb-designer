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
        pads[1].paste = false;
        let tri = vec![Point::mm(-0.5, -0.4), Point::mm(0.5, -0.4), Point::mm(0.0, 0.6)];
        let mut extra = pads[0].clone();
        extra.id = Id::new();
        extra.name = "3".into();
        extra.pos = Point::mm(0.0, 3.0);
        extra.rotation = Angle::deg(20.0);
        extra.shape = PadShape::Polygon { points: tri };
        extra.back = true;
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
                pad_copper_layers(pad, inst.side, &copper).into_iter().map(|l| (l, 0)).collect();
            layers.extend(pad_mask_layers(pad, inst.side).into_iter().map(|l| (l, p.board.rules.mask_expansion)));
            layers.extend(pad_paste_layer(pad, inst.side).map(|l| (l, p.board.rules.paste_margin)));
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

    // Nothing where a pad has no paste, or on the footprint's side for back pads.
    let mut absent = 0;
    for inst in p.board.footprints.iter() {
        let fp = editor_core::query::footprint_of_instance(p, inst).unwrap();
        for pad in fp.pads.iter() {
            let c = inst.transform().point(pad.pos);
            let not_on = if !pad.paste {
                LayerId::for_footprint(FootprintLayer::SolderPaste, inst.side)
            } else if pad.back {
                LayerId::for_footprint(FootprintLayer::Copper, inst.side)
            } else {
                continue;
            };
            let img = images.entry(not_on).or_insert_with(|| image(not_on));
            assert!(!img.covers(c), "pad {} on {not_on:?}", pad.name);
            absent += 1;
        }
    }
    assert!(absent >= 4, "{absent}");
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
    assert_eq!(gv.files.len(), export.manifest.files.iter().filter(|f| f.kind.is_image()).count());
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
    assert!(text.contains(&format!("R1 pad {}", pad.name)) && text.contains(" · net "), "{text}");
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

#[test]
fn blind_and_buried_vias_get_their_own_drill_files() {
    use editor_core::Command;
    let (_tmp, mut doc) = saved_demo();
    doc.execute(Command::SetCopperLayers { count: 4 }).unwrap();
    let (top, in1, in2) = (LayerId::TOP_COPPER, LayerId::inner(1), LayerId::inner(2));
    for (x, span) in [(1.0, [top, in1]), (2.0, [top, in1]), (3.0, [in1, in2])] {
        let pos = model::Point::mm(100.0 + x, 100.0);
        doc.execute(Command::AddVia { pos, diameter: None, drill: None, span: Some(span) }).unwrap();
    }
    let export = doc.export_gerber().unwrap();
    let stem = doc.project().name.clone();
    let files = &export.manifest.files;
    let blind = files.iter().find(|f| f.kind == FileKind::SpanDrill { from: top, to: in1 }).unwrap();
    assert_eq!(blind.name, format!("{stem}-front-in1.drl"));
    let text = std::fs::read_to_string(export.dir.join(&blind.name)).unwrap();
    assert!(text.contains("TF.FileFunction,Plated,1,2,Blind"), "{text}");
    assert_eq!(text.lines().filter(|l| l.starts_with('X')).count(), 2);
    let buried = files.iter().find(|f| f.kind == FileKind::SpanDrill { from: in1, to: in2 }).unwrap();
    assert_eq!(buried.name, format!("{stem}-in1-in2.drl"));
    assert!(std::fs::read_to_string(export.dir.join(&buried.name)).unwrap().contains("Plated,2,3,Buried"));
    // Through holes don't include them.
    let pth = files.iter().find(|f| f.kind == FileKind::PlatedDrill).unwrap();
    let pth = std::fs::read_to_string(export.dir.join(&pth.name)).unwrap();
    assert!(!pth.contains("X101.0000Y100.0000"));
    // The viewer reads each as its own drill layer.
    let gv = view::GerberView::load(&export.dir).unwrap();
    assert!(gv.errors.is_empty(), "{:?}", gv.errors);
    assert!(gv.files.iter().filter(|f| f.drill).count() >= 3);
}

/// A footprint turned upside down keeps its reference readable on the silkscreen: the
/// Gerber strokes are the same as unturned (for a centred text on the footprint's origin).
#[test]
fn footprint_texts_stay_upright_in_gerber() {
    use editor_core::Command;
    use model::builtin::{self, PART_MOUNTING_HOLE};
    use model::*;
    let mut doc = Document::new(Project::new("T"));
    doc.execute(Command::import_parts(&builtin::builtin_library(), &[PART_MOUNTING_HOLE]).unwrap()).unwrap();
    let pos = Point::mm(10.0, 10.0);
    let place = Command::PlaceBoardPart {
        part: PART_MOUNTING_HOLE,
        variant: None,
        pos,
        rotation: Angle::ZERO,
        side: Side::Top,
        refdes: Some("H1".into()),
        value: None,
    };
    doc.execute(place).unwrap();
    let fp = doc.project().board.footprints.iter().next().unwrap().id;
    let fields: Vec<Object> = doc
        .project()
        .board
        .fields
        .iter()
        .filter(|f| f.footprint == fp)
        .map(|f| {
            let mut f = f.clone();
            f.layer = FootprintLayer::Silkscreen;
            f.hidden = f.text.text != "${REF}";
            f.text.pos = Point::ZERO;
            f.text.rotation = Angle::ZERO;
            f.text.halign = HAlign::Center;
            f.text.valign = VAlign::Center;
            Object::FootprintField(f)
        })
        .collect();
    doc.execute(Command::Update { objects: fields }).unwrap();
    let strokes = |doc: &mut Document| {
        let copper = doc.current_copper().clone();
        let file = gerber::layers::layer_file(doc.project(), &copper, LayerId::TOP_SILK);
        file.lines().filter(|l| l.starts_with('X') || l.starts_with('Y')).map(str::to_string).collect::<Vec<_>>()
    };
    let upright = strokes(&mut doc);
    assert!(upright.len() > 4, "the reference is drawn: {upright:?}");
    let turn = Command::Rotate {
        ids: vec![ObjectId::Footprint(fp)],
        center: pos,
        angle: Angle::DEG180,
        keep_connected: false,
    };
    doc.execute(turn).unwrap();
    assert_eq!(strokes(&mut doc), upright);
}

/// Texts are written as filled regions that cover what the view fills: sampled on a grid,
/// the Gerber read back and the font's triangles agree (holes of O and 8 stay open), on
/// the bottom mirrored.
#[test]
fn texts_round_trip_as_regions() {
    use gerber::reader::parse_gerber;
    use model::*;
    use render::font::{self, Placement};

    let mut p = Project::new("T");
    let at = Point::mm(10.0, 10.0);
    for layer in [LayerId::TOP_SILK, LayerId::BOTTOM_SILK] {
        let text = Text::new("O8g", at, mm(2.0)).aligned(HAlign::Center, VAlign::Center).with_bold(true);
        p.board.texts.insert(BoardText { id: Id::new(), layer, text });
    }
    let copper = editor_core::copper::BoardConnectivity::default();
    for (layer, mirror) in [(LayerId::TOP_SILK, false), (LayerId::BOTTOM_SILK, true)] {
        let image = parse_gerber(&gerber::layers::layer_file(&p, &copper, layer)).unwrap();
        assert!(image.objects.len() >= 3, "a region per glyph outline");
        let place = Placement {
            origin: [10.0, 10.0],
            height: 2.0,
            rotation: 0.0,
            halign: font::HAlign::Center,
            valign: font::VAlign::Center,
            bold: true,
            mirror,
        };
        let mut tris = Vec::new();
        font::text_triangles("O8g", &place, |(t, _)| tris.push(t));
        let side =
            |a: [f32; 2], b: [f32; 2], q: [f32; 2]| (b[0] - a[0]) * (q[1] - a[1]) - (b[1] - a[1]) * (q[0] - a[0]);
        let filled = |q: [f32; 2]| {
            tris.iter().any(|&[a, b, c]| {
                let (d1, d2, d3) = (side(a, b, q), side(b, c, q), side(c, a, q));
                !((d1 < 0.0 || d2 < 0.0 || d3 < 0.0) && (d1 > 0.0 || d2 > 0.0 || d3 > 0.0))
            })
        };
        let (mut inside, mut differ) = (0, 0);
        for i in 0..120 {
            for j in 0..60 {
                let q = [6.0 + i as f32 * 0.07, 7.5 + j as f32 * 0.085];
                let f = filled(q);
                inside += f as usize;
                if f != image.covers(Point::mm(q[0] as f64, q[1] as f64)) {
                    differ += 1;
                }
            }
        }
        assert!(inside > 300, "{inside}");
        // Only samples within a hair of an edge may disagree (flattening, nm rounding).
        assert!(differ * 50 < inside, "{layer:?}: {differ} of {inside} samples differ");
    }
}

#[test]
fn copper_carries_nets_pads_and_components() {
    let (_tmp, mut doc) = saved_demo();
    let export = doc.export_gerber().unwrap();
    let f = export.manifest.files.iter().find(|f| f.kind == FileKind::Layer(LayerId::TOP_COPPER)).unwrap();
    let top = std::fs::read_to_string(export.dir.join(&f.name)).unwrap();
    assert!(top.contains("%TO.N,GND*%"), "net attributes");
    assert!(top.lines().any(|l| l.starts_with("%TO.P,R1,")), "pad attributes");
    assert!(top.contains("%TO.C,R1*%"), "component attributes");
    // Every attribute set is cleared again before the next one and at the end.
    let set = top.lines().filter(|l| l.starts_with("%TO.N")).count();
    assert!(set > 1 && top.matches("%TD*%").count() >= set - 1);
    assert!(top.ends_with("%TD*%\nM02*\n"));
    // The files still read back.
    gerber::reader::parse_gerber(&top).unwrap();
}

#[test]
fn exports_a_job_file_and_assembly_files() {
    let (_tmp, mut doc) = saved_demo();
    let export = doc.export_gerber().unwrap();
    let job = export.manifest.files.iter().find(|f| f.kind == FileKind::Job).unwrap();
    assert!(job.name.ends_with("-job.gbrjob"));
    let json: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(export.dir.join(&job.name)).unwrap()).unwrap();
    let listed = json["FilesAttributes"].as_array().unwrap();
    let images = export.manifest.files.iter().filter(|f| f.kind.is_image()).count();
    assert_eq!(listed.len(), images, "every Gerber and drill file, not the job file itself");
    assert!(listed.iter().any(|a| a["FileFunction"] == "Copper,L1,Top" && a["FilePolarity"] == "Positive"));
    assert_eq!(json["GeneralSpecs"]["LayerNumber"], doc.project().board.copper_layers().count());
    assert!(json["GeneralSpecs"]["Size"]["X"].as_f64().unwrap() > 10.0);

    let project_dir = doc.path.clone().unwrap();
    let assembly = gerber::assembly_dir(&project_dir);
    let kinds: Vec<FileKind> = export.manifest.assembly.iter().map(|f| f.kind).collect();
    assert_eq!(kinds, [FileKind::Bom, FileKind::Positions]);
    let read = |kind| {
        let f = export.manifest.assembly.iter().find(|f| f.kind == kind).unwrap();
        std::fs::read_to_string(assembly.join(&f.name)).unwrap()
    };
    let p = doc.project();
    let assembled = p
        .board
        .footprints
        .iter()
        .filter(|f| {
            let c = p.schematic.components.get(f.component).unwrap();
            !p.library.parts.get(c.part).unwrap().is_board_part()
        })
        .count();

    let positions = read(FileKind::Positions);
    let mut lines = positions.lines();
    assert_eq!(lines.next().unwrap(), "Designator,Value,Footprint,Mid X,Mid Y,Rotation,Layer");
    assert_eq!(lines.clone().count(), assembled);
    assert!(lines.clone().any(|l| l.starts_with("R1,") && (l.ends_with(",Top") || l.ends_with(",Bottom"))));

    let bom = read(FileKind::Bom);
    assert!(bom.starts_with("Designator,Value,Footprint,Quantity"));
    let quantities: usize = bom.lines().skip(1).map(quantity).sum();
    assert_eq!(quantities, assembled, "every placed component on one line\n{bom}");
}

/// The Quantity column of a BOM line (the fourth, after a possibly quoted designator list).
fn quantity(line: &str) -> usize {
    let rest = match line.strip_prefix('"') {
        Some(r) => &r[r.find('"').unwrap() + 2..],
        None => &line[line.find(',').unwrap() + 1..],
    };
    rest.split(',').nth(2).unwrap().parse().unwrap()
}
