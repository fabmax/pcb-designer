//! ERC and DRC review through MCP: power flags, finding keys, excluding findings and
//! changing severities.

use mcp::headless::HeadlessHost;
use mcp::{Call, Reply, Request, handle};
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

fn problems(host: &mut HeadlessHost, include_excluded: bool) -> Vec<Value> {
    let n = ask(host, Request::Netlist { include_excluded }).unwrap();
    n["problems"].as_array().unwrap().clone()
}

#[test]
fn findings_are_excluded_by_key_and_rated() {
    let mut host = HeadlessHost::new(Document::new(view::demo::demo_project()));
    assert!(problems(&mut host, false).is_empty(), "the demo is clean");
    // A stray power flag: a warning with a key.
    let cmd = json!({"command": "add_power_flag", "pos": [150_000_000, 50_000_000]});
    ask(&mut host, Request::Execute { commands: vec![cmd] }).unwrap();
    let p = problems(&mut host, false);
    assert_eq!(p.len(), 1);
    assert_eq!((p[0]["rule"].as_str(), p[0]["severity"].as_str()), (Some("power_flag_misplaced"), Some("warning")));
    let key = p[0]["key"].as_str().unwrap().to_string();

    let r = ask(
        &mut host,
        Request::ExcludeFindings { keys: vec![key.clone()], comment: "test point".into(), include: false },
    )
    .unwrap();
    assert_eq!(r["changed"], true);
    assert!(problems(&mut host, false).is_empty());
    let all = problems(&mut host, true);
    assert_eq!(all[0]["excluded"], true);
    assert_eq!(host.doc.project().schematic.erc.exclusions[0].comment, "test point");
    assert!(
        ask(&mut host, Request::ExcludeFindings { keys: vec!["nope".into()], comment: String::new(), include: false })
            .is_err()
    );

    // Included again, then the rule ignored.
    ask(&mut host, Request::ExcludeFindings { keys: vec![key], comment: String::new(), include: true }).unwrap();
    assert_eq!(problems(&mut host, false).len(), 1);
    let settings =
        serde_json::from_value(json!({"erc": {"power_flag_misplaced": "ignore"}, "drc": {"unrouted": "warning"}}))
            .unwrap();
    let r = ask(&mut host, Request::SetCheckSettings(settings)).unwrap();
    assert_eq!(r["drc"]["unrouted"], "warning");
    assert!(problems(&mut host, false).is_empty());
    let drc = ask(&mut host, Request::Drc { limit: 10, include_excluded: false }).unwrap();
    assert!(
        drc["violations"].as_array().unwrap().iter().all(|v| v["kind"] != "unrouted" || v["severity"] == "warning")
    );
}
