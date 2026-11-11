//! `export_gerber` on the headless host.

use mcp::headless::HeadlessHost;
use mcp::{Call, Reply, Request, handle};
use serde_json::Value;
use view::Document;

fn ask(host: &mut HeadlessHost, request: Request) -> Result<Value, String> {
    let (tx, mut rx) = tokio::sync::oneshot::channel();
    handle(Call { request, reply: tx }, host);
    match rx.try_recv().expect("answered at once")? {
        Reply::Json(v) => Ok(v),
        Reply::Png(_) => panic!("expected JSON"),
    }
}

#[test]
fn exports_into_the_project_folder() {
    let mut host = HeadlessHost::new(Document::new(view::demo::demo_project()));
    let err = ask(&mut host, Request::ExportGerber).unwrap_err();
    assert!(err.contains("save the project"), "{err}");

    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("demo.pcbproj");
    host.doc.save_as(&dir).unwrap();
    let v = ask(&mut host, Request::ExportGerber).unwrap();
    let files = v["files"].as_array().unwrap();
    assert!(files.iter().any(|f| f["label"] == "Top Copper"), "{v}");
    for f in files {
        assert!(dir.join("output/gerber").join(f["name"].as_str().unwrap()).is_file());
    }
    assert!(std::path::Path::new(v["zip"].as_str().unwrap()).is_file());
    assert!(v["drc_violations"].as_u64().unwrap() > 0, "the demo has unrouted connections");
}
