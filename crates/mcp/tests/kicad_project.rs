//! `import_kicad_project` replaces the open project by an imported KiCad project, which
//! `save_project` then writes.

use std::path::PathBuf;

use mcp::handle;
use mcp::headless::HeadlessHost;
use mcp::host::{Call, Reply, Request};
use model::Project;
use view::Document;

fn call(host: &mut HeadlessHost, request: Request) -> Result<serde_json::Value, String> {
    let (reply, rx) = tokio::sync::oneshot::channel();
    handle(Call { request, reply }, host);
    rx.blocking_recv().unwrap().map(|r| match r {
        Reply::Json(v) => v,
        Reply::Png(_) => panic!("json expected"),
    })
}

fn fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../kicad/tests/fixtures/hier/hier.kicad_pro")
}

#[test]
fn import_replaces_the_project_and_saves() {
    let mut host = HeadlessHost::new(Document::new(Project::new("Old")));
    let report = call(&mut host, Request::ImportKicadProject { path: fixture() }).unwrap();
    assert_eq!(report["saved"], false, "{report}");
    assert!(report["skipped"].is_object(), "{report}");

    let project = host.doc.project();
    assert_eq!(project.name, "hier");
    assert!(host.doc.path.is_none(), "the import is not saved yet");
    let refdes: Vec<&str> = project.schematic.components.iter().map(|c| c.refdes.as_str()).collect();
    for r in ["R1", "R2", "R3"] {
        assert!(refdes.contains(&r), "{refdes:?}");
    }
    assert!(!project.board.tracks.is_empty() && !project.board.zones.is_empty());
    let components = project.schematic.components.len();

    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("hier.pcbproj");
    call(&mut host, Request::Save { path: Some(out.clone()) }).unwrap();
    let reopened = Document::open(&out).unwrap();
    assert_eq!(reopened.project().schematic.components.len(), components);
}

#[test]
fn a_failed_import_keeps_the_project() {
    let mut host = HeadlessHost::new(Document::new(Project::new("Old")));
    let err = call(&mut host, Request::ImportKicadProject { path: "/nonexistent/x.kicad_pro".into() }).unwrap_err();
    assert!(!err.is_empty());
    assert_eq!(host.doc.project().name, "Old");
}
