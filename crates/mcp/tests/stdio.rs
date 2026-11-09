//! Headless mode: an MCP client talks to `mcp::stdio::serve` over an in-memory pipe,
//! edits the project, checks it and saves it.

use mcp::headless::HeadlessHost;
use model::Project;
use rmcp::ServiceExt;
use rmcp::model::{CallToolRequestParams, CallToolResult, ClientConfig, ContentBlock};
use serde_json::{Value, json};
use view::Document;

fn ok_json(r: CallToolResult) -> Value {
    let text = match r.content.first() {
        Some(ContentBlock::Text(t)) => t.text.clone(),
        other => panic!("expected text, got {other:?}"),
    };
    assert_ne!(r.is_error, Some(true), "tool failed: {text}");
    serde_json::from_str(&text).expect("json")
}

#[test]
fn headless_session_edits_checks_and_saves() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("demo.pcbproj");
    let (client_side, server_side) = tokio::io::duplex(1 << 16);
    let server = std::thread::spawn(move || {
        let mut host = HeadlessHost::new(Document::new(view::demo::demo_project()));
        mcp::stdio::serve(&mut host, move || tokio::io::split(server_side)).unwrap();
        host.doc.project().clone()
    });

    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    let saved_path = path.clone();
    rt.block_on(async move {
        let client = ClientConfig::default().serve(tokio::io::split(client_side)).await.expect("connects");
        let info = client.peer_info().unwrap();
        assert!(info.instructions.as_deref().unwrap().contains("headless"));
        let call = |name: &'static str, a: Value| {
            let client = &client;
            async move {
                let mut p = CallToolRequestParams::new(name);
                if let Some(o) = a.as_object() {
                    p = p.with_arguments(o.clone());
                }
                client.call_tool(p).await.expect("protocol")
            }
        };
        let project = ok_json(call("get_project", Value::Null).await);
        assert_eq!(project["name"], "Demo", "{project}");
        let routing = ok_json(call("get_routing", Value::Null).await);
        let line = routing["unrouted"][0].clone();
        ok_json(
            call(
                "execute",
                json!({ "commands": [{ "command": "add_track", "layer": 0, "points": [line["from"], line["to"]] }] }),
            )
            .await,
        );
        let drc = ok_json(call("run_drc", Value::Null).await);
        assert!(drc["count"].as_u64().unwrap() > 0);
        let shot = call("screenshot", json!({ "target": "board", "width": 200, "height": 150 })).await;
        assert!(matches!(shot.content.first(), Some(ContentBlock::Image(_))), "offscreen render");
        ok_json(call("save_project", json!({ "path": saved_path.to_string_lossy() })).await);
        client.cancel().await.unwrap();
    });
    // The server returns once the client is gone, with the edit applied.
    let project: Project = server.join().unwrap();
    assert!(!project.board.tracks.is_empty());
    let loaded = model::io::load_project(&path).unwrap();
    assert_eq!(loaded.board.tracks.len(), project.board.tracks.len());
}
