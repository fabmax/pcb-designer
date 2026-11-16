//! Free texts and symbol fields through MCP: add_text on a sheet and on a board layer,
//! and the reference and value texts listed as symbol_field objects.

use mcp::headless::HeadlessHost;
use mcp::{Call, Reply, Request, handle};
use model::ObjectKind;
use serde_json::{Value, json};
use view::Document;

fn ask(host: &mut HeadlessHost, request: Request) -> Result<Value, String> {
    let (tx, mut rx) = tokio::sync::oneshot::channel();
    handle(Call { request, reply: tx }, host);
    match rx.try_recv().expect("answered at once")? {
        Reply::Json(v) => Ok(v),
        Reply::Png(_) => panic!("expected JSON"),
    }
}

fn execute(host: &mut HeadlessHost, command: Value) -> Result<Value, String> {
    ask(host, Request::Execute { commands: vec![command] })
}

#[test]
fn texts_are_added_on_sheets_and_board_layers() {
    let mut host = HeadlessHost::new(Document::new(view::demo::demo_project()));
    let sch = host.doc.project().schematic.texts.len();
    execute(&mut host, json!({"command": "add_text", "pos": [10_000_000, 20_000_000], "text": "Power\nsupply"}))
        .unwrap();
    let p = host.doc.project();
    assert_eq!(p.schematic.texts.len(), sch + 1);
    let t = p.schematic.texts.iter().last().unwrap();
    assert_eq!((t.text.text.as_str(), t.text.size), ("Power\nsupply", 1_270_000));

    execute(&mut host, json!({"command": "add_text", "layer": 100, "pos": [0, 0], "text": "v1.0", "rotation": 90}))
        .unwrap();
    let b = host.doc.project().board.texts.iter().last().unwrap();
    assert_eq!((b.text.size, b.text.width, b.text.rotation.degrees()), (1_000_000, 150_000, 90.0));

    let empty = execute(&mut host, json!({"command": "add_text", "pos": [0, 0], "text": " "}));
    assert!(empty.unwrap_err().contains("empty"));
}

#[test]
fn symbol_fields_are_listed() {
    let mut host = HeadlessHost::new(Document::new(view::demo::demo_project()));
    let symbols = host.doc.project().schematic.symbols.len();
    let listed =
        ask(&mut host, Request::ListObjects { kind: Some(ObjectKind::SymbolField), offset: 0, limit: 1000 }).unwrap();
    let objects = listed["objects"].as_array().unwrap();
    // A reference and a value for every symbol.
    assert_eq!(objects.len(), 2 * symbols);
    assert!(objects.iter().any(|o| o["text"]["text"] == "${REF}"));
}
