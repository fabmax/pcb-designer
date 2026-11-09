//! The library tools on the headless host: a part from a "datasheet", start to finish.

use mcp::headless::HeadlessHost;
use mcp::library::LibraryRequest;
use mcp::{Call, Reply, Request, handle};
use model::Id;
use serde_json::{Value, json};
use view::Document;

fn ask(host: &mut HeadlessHost, request: Request) -> Result<Reply, String> {
    let (tx, mut rx) = tokio::sync::oneshot::channel();
    handle(Call { request, reply: tx }, host);
    rx.try_recv().expect("answered at once")
}

fn lib(host: &mut HeadlessHost, request: LibraryRequest) -> Result<Value, String> {
    match ask(host, Request::Library(request))? {
        Reply::Json(v) => Ok(v),
        Reply::Png(_) => panic!("expected JSON"),
    }
}

fn edit(host: &mut HeadlessHost, upsert: Value, delete: &[&str]) -> Result<Value, String> {
    let upsert = upsert.as_array().cloned().unwrap_or_default();
    let delete = delete.iter().map(|s| s.to_string()).collect();
    lib(host, LibraryRequest::Edit { library: "Driver".into(), upsert, delete })
}

fn get(host: &mut HeadlessHost, item: &str) -> Value {
    let v = lib(host, LibraryRequest::Get { library: "Driver".into(), items: vec![item.into()] }).unwrap();
    v["items"][0].clone()
}

const MM: i64 = 1_000_000;

/// A three pin driver: symbol, SOT-23 footprint and the part, written the way an agent
/// would (no ids, references by name).
fn driver() -> Value {
    json!([
        {
            "type": "lib_part",
            "name": "DRV1",
            "description": "Low side driver",
            "units": [{ "symbol": "DRV" }],
            "variants": [{ "footprint": "SOT-23", "name": "DRV1-SOT" }]
        },
        {
            "type": "lib_symbol",
            "name": "DRV",
            "graphics": [{ "type": "rect", "a": [-3 * MM, -3 * MM], "b": [3 * MM, 3 * MM], "width": 254000, "fill": true }],
            "texts": [{ "text": "${REF}", "pos": [0, 4 * MM], "size": 1270000 }],
            "pins": [
                { "name": "IN", "number": "1", "pos": [-5_080_000, 0], "direction": 0, "kind": "input" },
                { "name": "OUT", "number": "3", "pos": [5_080_000, 0], "direction": 180, "kind": "open_collector" },
                { "name": "GND", "number": "2", "pos": [0, -5_080_000], "direction": 90, "kind": "power_in" }
            ]
        },
        {
            "type": "lib_footprint",
            "name": "SOT-23",
            "pads": [
                { "name": "1", "pos": [-950_000, -MM], "size": [600_000, 700_000], "shape": { "type": "rect" }, "kind": { "type": "smd" } },
                { "name": "2", "pos": [950_000, -MM], "size": [600_000, 700_000], "shape": { "type": "rect" }, "kind": { "type": "smd" } },
                { "name": "3", "pos": [0, MM], "size": [600_000, 700_000], "shape": { "type": "rect" }, "kind": { "type": "smd" } }
            ]
        }
    ])
}

#[test]
fn part_from_a_datasheet() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("Driver.pcblib");
    let mut host = HeadlessHost::new(Document::new(view::demo::demo_project()));

    let created = lib(&mut host, LibraryRequest::Create { name: "Driver".into(), path: Some(file.clone()) }).unwrap();
    assert_eq!(created["name"], "Driver");
    assert!(file.exists(), "saved right away");
    let again = lib(&mut host, LibraryRequest::Create { name: "Driver".into(), path: None });
    assert!(again.unwrap_err().contains("already open"));

    // Parts come after what they use, whatever the order given.
    let reply = edit(&mut host, driver(), &[]).unwrap();
    assert_eq!(reply["added"], json!(["symbol:DRV", "footprint:SOT-23", "part:DRV1"]), "{reply}");
    let variant = &reply["parts"][0]["variants"][0];
    assert_eq!(variant["connected_pads"], "3 of 3", "pads map to pin numbers: {reply}");
    assert_eq!(variant["pins_without_pad"], json!([]));

    let part = get(&mut host, "part:DRV1");
    assert_eq!(part["type"], "lib_part");
    assert_eq!(part["default_value"], "DRV1", "defaults to the name");
    assert_eq!(part["prefix"], "U");
    let symbol = get(&mut host, "symbol:DRV");
    assert_eq!(symbol["pins"][0]["length"], 2_540_000, "default pin length");
    assert_eq!(symbol["texts"][0]["width"], 1_270_000 / 7, "default text width");

    // Updating by name keeps pad and pin ids, so the part's mapping stays valid.
    let pad_ids = |fp: &Value| fp["pads"].as_array().unwrap().iter().map(|p| p["id"].clone()).collect::<Vec<_>>();
    let before = pad_ids(&get(&mut host, "footprint:SOT-23"));
    let mut bigger = driver()[2].clone();
    for pad in bigger["pads"].as_array_mut().unwrap() {
        pad["size"] = json!([700_000, 900_000]);
    }
    let reply = edit(&mut host, json!([bigger]), &[]).unwrap();
    assert_eq!(reply["updated"], json!(["footprint:SOT-23"]));
    let fp = get(&mut host, "footprint:SOT-23");
    assert_eq!(pad_ids(&fp), before);
    assert_eq!(fp["pads"][0]["size"], json!([700_000, 900_000]));
    // Fields left out keep their value.
    edit(&mut host, json!([{ "type": "lib_part", "name": "DRV1", "description": "Sink driver" }]), &[]).unwrap();
    let part = get(&mut host, "part:DRV1");
    assert_eq!(part["description"], "Sink driver");
    assert_eq!(part["variants"][0]["connections"].as_array().unwrap().len(), 3);

    // Explicit connections by pad name and pin name or number; a second package.
    let sot_variant = part["variants"][0]["id"].clone();
    let reply = edit(
        &mut host,
        json!([
            { "type": "lib_footprint", "name": "TO-92", "pads": [
                { "name": "1", "pos": [-1_270_000, 0], "size": [1_300_000, 1_300_000], "shape": { "type": "rect" }, "kind": { "type": "through_hole", "drill": 750_000 } },
                { "name": "2", "pos": [0, 0], "size": [1_300_000, 1_300_000], "shape": { "type": "oval" }, "kind": { "type": "through_hole", "drill": 750_000 } },
                { "name": "3", "pos": [1_270_000, 0], "size": [1_300_000, 1_300_000], "shape": { "type": "oval" }, "kind": { "type": "through_hole", "drill": 750_000 } }
            ] },
            { "type": "lib_part", "name": "DRV1", "variants": [
                { "footprint": "SOT-23", "name": "DRV1-SOT" },
                { "footprint": "TO-92", "connections": [
                    { "pad": "1", "pin": "OUT" }, { "pad": "2", "pin": "GND" }, { "pad": 3, "pin": "1" }
                ] }
            ] }
        ]),
        &[],
    )
    .unwrap();
    let variants = &reply["parts"][0]["variants"];
    assert_eq!(variants[1]["name"], "TO-92", "named after the footprint");
    assert_eq!(variants[1]["connected_pads"], "3 of 3");
    let part = get(&mut host, "part:DRV1");
    assert_eq!(part["variants"][0]["id"], sot_variant, "variant id kept by name");

    // Refused, and nothing changes: unknown references, deleting what a part uses.
    let revision = host.libraries[1].doc.revision;
    let err = edit(
        &mut host,
        json!([{ "type": "lib_part", "name": "DRV2", "units": [{ "symbol": "DRV" }], "variants": [{ "footprint": "SOT-99" }] }]),
        &[],
    )
    .unwrap_err();
    assert!(err.contains("no footprint \"SOT-99\""), "{err}");
    let err = edit(&mut host, json!([]), &["footprint:TO-92"]).unwrap_err();
    assert!(err.contains("nothing was changed") && err.contains("missing footprint"), "{err}");
    let err = edit(&mut host, json!([]), &["DRV"]).unwrap_err();
    assert!(err.contains("references missing symbol"), "the part uses it: {err}");
    let err =
        edit(&mut host, json!([{ "type": "lib_symbol", "name": "X", "pins": [{ "name": "A" }] }]), &[]).unwrap_err();
    assert!(err.contains("missing field"), "{err}");
    assert_eq!(host.libraries[1].doc.revision, revision);

    // Deleting a variant's footprint together with the variant works.
    let reply = edit(
        &mut host,
        json!([{ "type": "lib_part", "name": "DRV1", "variants": [{ "footprint": "SOT-23", "name": "DRV1-SOT" }] }]),
        &["footprint:TO-92"],
    )
    .unwrap();
    assert_eq!(reply["deleted"], json!(["footprint:TO-92"]));

    // Look at it.
    for item in ["DRV", "footprint:SOT-23", "part:DRV1"] {
        let png = ask(
            &mut host,
            Request::Library(LibraryRequest::Render {
                library: "Driver".into(),
                item: item.into(),
                width: 300,
                height: 300,
            }),
        );
        match png {
            Ok(Reply::Png(png)) => assert_eq!(&png[1..4], b"PNG"),
            other => panic!("{item}: expected a PNG, got {other:?}"),
        }
    }

    // Save, reload, use it in the project.
    let saved = lib(&mut host, LibraryRequest::Save { library: "Driver".into(), path: None }).unwrap();
    assert_eq!(saved["library"], "Driver");
    let reloaded = model::io::load_library(&file).unwrap();
    assert_eq!(reloaded.parts.len(), 1);
    assert_eq!(reloaded.validate(), Vec::<String>::new());
    let listed = lib(&mut host, LibraryRequest::List).unwrap();
    let names: Vec<&str> =
        listed["libraries"].as_array().unwrap().iter().map(|l| l["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["Starter", "Driver"]);
    assert_eq!(listed["libraries"][1]["modified"], false);

    let part_id: Id = serde_json::from_value(get(&mut host, "part:DRV1")["id"].clone()).unwrap();
    ask(&mut host, Request::ImportParts { parts: vec![part_id] }).unwrap();
    assert!(host.doc.project().library.parts.contains(part_id), "ready to place");

    // Opening the open file again finds the same library; another file is added.
    let opened = lib(&mut host, LibraryRequest::Open { path: file.clone() }).unwrap();
    assert_eq!(opened["name"], "Driver");
    let copy = dir.path().join("Copy.pcblib");
    std::fs::copy(&file, &copy).unwrap();
    lib(&mut host, LibraryRequest::Open { path: copy }).unwrap();
    assert_eq!(host.libraries.len(), 3);
}
