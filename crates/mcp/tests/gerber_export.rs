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

    // The export, read back and rendered (all files, or some).
    for layers in [None, Some(vec!["Top Copper".to_string(), "Plated Drill".into()])] {
        let (tx, mut rx) = tokio::sync::oneshot::channel();
        handle(Call { request: Request::GerberScreenshot { layers, width: 200, height: 150 }, reply: tx }, &mut host);
        match rx.try_recv().unwrap() {
            Ok(Reply::Png(png)) => assert!(png.starts_with(b"\x89PNG")),
            Ok(Reply::Json(v)) => panic!("expected a PNG: {v}"),
            Err(e) => panic!("{e}"),
        }
    }
    let err = ask(&mut host, Request::GerberScreenshot { layers: Some(vec!["nope".into()]), width: 64, height: 64 });
    assert!(err.unwrap_err().contains("Top Copper"));
}
