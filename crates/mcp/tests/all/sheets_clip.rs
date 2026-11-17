//! move_to_sheet and duplicate through MCP's execute.

use mcp::headless::HeadlessHost;
use mcp::{Call, Reply, Request, handle};
use model::{ObjectId, Point};
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

fn execute(host: &mut HeadlessHost, commands: Vec<Value>) -> Result<Value, String> {
    ask(host, Request::Execute { commands })
}

#[test]
fn objects_move_to_a_sheet_by_path() {
    let mut host = HeadlessHost::new(Document::new(view::demo::demo_project()));
    execute(&mut host, vec![json!({ "command": "add_sheet", "parent": "/", "name": "Power", "pos": [0, 0] })]).unwrap();
    let p = host.doc.project();
    let power = p.schematic.sheets.iter().find(|s| s.name == "Power").unwrap().id;
    let sym = p.schematic.symbols.iter().next().unwrap().id;
    execute(
        &mut host,
        vec![json!({ "command": "move_to_sheet", "ids": [{ "type": "symbol", "id": sym }], "sheet": "/Power" })],
    )
    .unwrap();
    let p = host.doc.project();
    assert_eq!(p.schematic.sheet_of(ObjectId::Symbol(sym)), Some(power), "same id, new sheet");

    let r = execute(
        &mut host,
        vec![json!({ "command": "move_to_sheet", "ids": [{ "type": "sheet", "id": power }], "sheet": "/Power" })],
    );
    assert!(r.is_err(), "a sheet can't move below itself");
}

#[test]
fn duplicate_copies_with_new_references() {
    let mut host = HeadlessHost::new(Document::new(view::demo::demo_project()));
    let p = host.doc.project();
    let components = p.schematic.components.len();
    let sym = p.schematic.symbols.iter().next().unwrap();
    let (id, pos) = (sym.id, sym.pos);
    let r = execute(
        &mut host,
        vec![json!({ "command": "duplicate", "ids": [{ "type": "symbol", "id": id }], "delta": [10_160_000, 0] })],
    )
    .unwrap();
    let p = host.doc.project();
    assert_eq!(p.schematic.components.len(), components + 1);
    let copy = r["changes"]["inserted"].as_array().unwrap().iter().find(|i| i["type"] == "symbol").unwrap();
    let copy: ObjectId = serde_json::from_value(copy.clone()).unwrap();
    let ObjectId::Symbol(copy) = copy else { panic!() };
    let s = p.schematic.symbols.get(copy).unwrap();
    assert_eq!(s.pos, pos + Point::new(10_160_000, 0));
    let refs: std::collections::HashSet<&str> = p.schematic.components.iter().map(|c| c.refdes.as_str()).collect();
    assert_eq!(refs.len(), components + 1, "references stay unique");
}
