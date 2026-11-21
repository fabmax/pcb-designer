//! route: the router through MCP (shove and stopping short).

use mcp::headless::HeadlessHost;
use mcp::{Call, Reply, Request, handle};
use model::*;
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
fn route_shoves_a_track_aside_in_one_undo_step() {
    let mut doc = Document::new(Project::new("T"));
    let wall = Track::straight(LayerId::TOP_COPPER, Point::mm(0.0, 0.0), Point::mm(20.0, 0.0), mm(0.25));
    doc.execute(editor_core::Command::Insert { objects: vec![Object::Track(wall.clone())] }).unwrap();
    let mut host = HeadlessHost::new(doc);
    let r = ask(
        &mut host,
        Request::Route {
            layer: LayerId::TOP_COPPER,
            points: vec![Point::mm(10.0, -5.0), Point::mm(10.0, 1.0)],
            width: None,
            mode: editor_core::router::RouteMode::Shove,
        },
    )
    .unwrap();
    assert_eq!(r["reached"], true, "{r}");
    assert_eq!(r["pushed"].as_array().unwrap().len(), 1, "{r}");
    assert_eq!(r["pushed"][0]["tracks"][0], serde_json::json!(wall.id));
    assert!(host.doc.project().board.tracks.len() > 2);
    host.doc.undo();
    assert_eq!(host.doc.project().board.tracks.iter().cloned().collect::<Vec<_>>(), vec![wall]);

    // Into a locked track: stops short and says why.
    let mut t = host.doc.project().board.tracks.iter().next().unwrap().clone();
    t.locked = true;
    host.doc.execute(editor_core::Command::Update { objects: vec![Object::Track(t)] }).unwrap();
    let r = ask(
        &mut host,
        Request::Route {
            layer: LayerId::TOP_COPPER,
            points: vec![Point::mm(10.0, -5.0), Point::mm(10.0, -0.2)],
            width: None,
            mode: editor_core::router::RouteMode::Shove,
        },
    )
    .unwrap();
    assert_eq!(r["reached"], false, "{r}");
    assert!(r["blocked"]["message"].is_string(), "{r}");
}
