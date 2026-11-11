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
