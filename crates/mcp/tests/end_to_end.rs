//! Real MCP client ↔ HTTP server ↔ headless host round trips.

use std::sync::Arc;

use base64::Engine;
use mcp::headless::HeadlessHost;
use mcp::{McpServer, handle};
use model::Project;
use rmcp::ServiceExt;
use rmcp::model::{CallToolRequestParams, CallToolResult, ClientConfig, ContentBlock};
use rmcp::transport::StreamableHttpClientTransport;
use serde_json::{Value, json};
use view::Document;

fn args(v: Value) -> serde_json::Map<String, Value> {
    v.as_object().cloned().expect("object")
}

fn text(r: &CallToolResult) -> String {
    match r.content.first() {
        Some(ContentBlock::Text(t)) => t.text.clone(),
        other => panic!("expected text content, got {other:?}"),
    }
}

fn ok_json(r: CallToolResult) -> Value {
    assert_ne!(r.is_error, Some(true), "tool failed: {}", text(&r));
    serde_json::from_str(&text(&r)).expect("json result")
}

#[tokio::test(flavor = "multi_thread")]
async fn agent_session() {
    let (mut server, calls) = McpServer::start(0, Arc::new(|| {})).expect("server starts");
    // The "application": owns the document and answers calls on its own thread.
    let app = std::thread::spawn(move || {
        let mut host = HeadlessHost::new(Document::new(view::demo::demo_project()));
        while let Ok(call) = calls.recv() {
            handle(call, &mut host);
        }
        host.doc.project().clone()
    });

    let transport = StreamableHttpClientTransport::from_uri(server.url());
    let client = ClientConfig::default().serve(transport).await.expect("client connects");
    let call = |name: &'static str, a: Value| {
        let client = &client;
        async move {
            let mut p = CallToolRequestParams::new(name);
            if !a.is_null() {
                p = p.with_arguments(args(a));
            }
            client.call_tool(p).await.expect("call succeeds at protocol level")
        }
    };

    // Tools and their schemas.
    let tools = client.list_all_tools().await.unwrap();
    let names: Vec<&str> = tools.iter().map(|t| t.name.as_ref()).collect();
    for expected in [
        "get_project",
        "execute",
        "screenshot",
        "undo",
        "get_component",
        "set_selection",
        "search_kicad",
        "import_kicad",
        "get_netlist",
        "get_routing",
    ] {
        assert!(names.contains(&expected), "missing tool {expected}: {names:?}");
    }
    let execute = tools.iter().find(|t| t.name == "execute").unwrap();
    let schema = serde_json::to_string(&execute.input_schema).unwrap();
    assert!(schema.contains("place_part") && schema.contains("add_wire"), "command schema is exposed");

    // Inspect.
    let project = ok_json(call("get_project", Value::Null).await);
    assert_eq!(project["name"], "Demo");
    assert_eq!(project["counts"]["component"], 5);

    // Libraries and project part states.
    let libs = ok_json(call("list_library_parts", Value::Null).await);
    assert_eq!(libs["libraries"][0]["library"], "Starter");
    let states = libs["project_parts"].as_array().unwrap();
    assert!(!states.is_empty() && states.iter().all(|p| p["state"] == "UpToDate"), "{states:?}");
    // The headless host has no KiCad libraries: a clear error, not a hang.
    let r = call("search_kicad", json!({ "query": "lm358" })).await;
    assert_eq!(r.is_error, Some(true));
    assert!(text(&r).contains("KiCad"), "{}", text(&r));
    let r = call("import_kicad", json!({ "symbols": [{ "symbol": "Device:R" }] })).await;
    assert_eq!(r.is_error, Some(true));
    let r = ok_json(call("update_parts", json!({})).await);
    assert_eq!(r["updated"], 0, "nothing changed");

    // Place a resistor; its footprint follows.
    let r = ok_json(
        call(
            "execute",
            json!({ "commands": [
                { "command": "place_part", "part": model::builtin::PART_RESISTOR, "pos": [0, 50_000_000], "value": "4k7" }
            ]}),
        )
        .await,
    );
    let created = r["created"].as_array().unwrap();
    let component = created.iter().find(|o| o["type"] == "component").expect("component created");
    let refdes = component["refdes"].as_str().unwrap().to_string();
    assert!(created.iter().any(|o| o["type"] == "footprint"), "footprint created by live sync");

    let info = ok_json(call("get_component", json!({ "refdes": refdes })).await);
    assert_eq!(info["symbols"][0]["pins"].as_array().unwrap().len(), 2);
    assert_eq!(info["footprint"]["pads"].as_array().unwrap().len(), 2);

    // Wire it up with labels; the netlist follows.
    let pins = info["symbols"][0]["pins"].as_array().unwrap().clone();
    let r = ok_json(
        call(
            "execute",
            json!({ "commands": [
                { "command": "add_label", "pos": pins[0]["pos"], "name": "TEST_A" },
                { "command": "add_label", "pos": pins[1]["pos"], "name": "+5V", "kind": "power" }
            ]}),
        )
        .await,
    );
    assert_eq!(r["created"].as_array().unwrap().len(), 2);
    let netlist = ok_json(call("get_netlist", Value::Null).await);
    let nets = netlist["nets"].as_array().unwrap();
    let net = |name: &str| nets.iter().find(|n| n["name"] == name).cloned();
    let a = net("TEST_A").expect("labelled net");
    assert_eq!(a["pins"].as_array().unwrap().len(), 1);
    assert_eq!(a["pins"][0]["ref"], refdes.as_str());
    assert!(net("+5V").unwrap()["pins"].as_array().unwrap().iter().any(|p| p["ref"] == refdes.as_str()));
    assert!(netlist["unconnected_pins"].is_array());
    ok_json(call("undo", Value::Null).await);

    // The demo board is partly routed; route one ratsnest line with a track.
    let routing = ok_json(call("get_routing", Value::Null).await);
    let lines = routing["unrouted"].as_array().unwrap().clone();
    assert!(!lines.is_empty() && routing["fully_routed"] == false);
    let len = |l: &Value| {
        let (a, b) = (&l["from"], &l["to"]);
        let d = |i: usize| a[i].as_f64().unwrap() - b[i].as_f64().unwrap();
        (d(0).powi(2) + d(1).powi(2)).sqrt()
    };
    let line = lines.iter().filter(|l| l["net"] == "GND").min_by(|a, b| len(a).total_cmp(&len(b))).unwrap();
    ok_json(
        call(
            "execute",
            json!({ "commands": [{ "command": "add_track", "layer": 0, "points": [line["from"], line["to"]] }] }),
        )
        .await,
    );
    let after = ok_json(call("get_routing", Value::Null).await);
    assert_eq!(after["unrouted"].as_array().unwrap().len(), lines.len() - 1, "{after}");
    assert!(after["shorts"].as_array().unwrap().is_empty());
    ok_json(call("undo", Value::Null).await);

    // A failing batch changes nothing.
    let r = call(
        "execute",
        json!({ "commands": [
            { "command": "add_wire", "points": [[0, 0], [10_000_000, 0]] },
            { "command": "delete", "ids": [{ "type": "wire", "id": "00000000-0000-0000-0000-000000000001" }] }
        ]}),
    )
    .await;
    assert_eq!(r.is_error, Some(true));
    assert!(text(&r).contains("command 2 of 2 failed"), "{}", text(&r));
    let project = ok_json(call("get_project", Value::Null).await);
    assert_eq!(project["counts"]["wire"], 10, "first command was rolled back");

    // Select, screenshot, undo.
    let sym = info["symbols"][0]["symbol"]["id"].clone();
    let sel =
        ok_json(call("set_selection", json!({ "view": "schematic", "ids": [{ "type": "symbol", "id": sym }] })).await);
    assert_eq!(sel["selected"], 1);
    let shot = call("screenshot", json!({ "target": "schematic", "width": 400, "height": 300 })).await;
    match shot.content.first() {
        Some(ContentBlock::Image(img)) => {
            let png = base64::engine::general_purpose::STANDARD.decode(&img.data).unwrap();
            assert_eq!(&png[1..4], b"PNG");
        }
        _ => panic!("expected an image: {shot:?}"),
    }
    let window = call("screenshot", json!({ "target": "window" })).await;
    assert_eq!(window.is_error, Some(true), "headless host has no window");

    let r = ok_json(call("undo", Value::Null).await);
    assert_eq!(r["undone"], "Place part");

    client.cancel().await.unwrap();
    server.stop();
    drop(server);
    let final_project: Project = app.join().unwrap();
    assert!(final_project.schematic.component_by_refdes(&refdes).is_none(), "undo removed the part");
    assert_eq!(final_project.schematic.components.len(), 5);
}

/// Status line of a raw MCP `initialize` POST with the given `Host` and optional `Origin`.
fn raw_initialize(port: u16, host: &str, origin: Option<&str>) -> String {
    use std::io::{Read, Write};
    let body = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"t","version":"1"}}}"#;
    let origin = origin.map(|o| format!("Origin: {o}\r\n")).unwrap_or_default();
    let request = format!(
        "POST /mcp HTTP/1.1\r\nHost: {host}\r\n{origin}Content-Type: application/json\r\n\
         Accept: application/json, text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let mut stream = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
    stream.write_all(request.as_bytes()).unwrap();
    let mut response = String::new();
    let _ = stream.read_to_string(&mut response);
    response.lines().next().unwrap_or_default().to_string()
}

#[test]
fn browser_origins_and_foreign_hosts_are_rejected() {
    let (server, _calls) = McpServer::start(0, Arc::new(|| {})).expect("server starts");
    let port = server.addr().port();
    let local = format!("127.0.0.1:{port}");
    let ok = raw_initialize(port, &local, None);
    assert!(ok.contains(" 200"), "local client accepted: {ok}");
    let web = raw_initialize(port, &local, Some("https://evil.example.com"));
    assert!(web.contains(" 403"), "web origin rejected: {web}");
    let rebinding = raw_initialize(port, "evil.example.com", None);
    assert!(rebinding.contains(" 403"), "foreign Host rejected: {rebinding}");
}
