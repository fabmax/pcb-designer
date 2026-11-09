//! Render the demo board offscreen (no window needed), after running the design rule
//! check: `cargo run -p mcp --example render_board -- out.png [width height]`.
//! Development aid for screenshots.

use mcp::Host;
use mcp::headless::HeadlessHost;
use view::{Document, ViewKind};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let out = args.first().cloned().unwrap_or_else(|| "board.png".into());
    let w = args.get(1).and_then(|v| v.parse().ok()).unwrap_or(1200);
    let h = args.get(2).and_then(|v| v.parse().ok()).unwrap_or(900);
    let mut host = HeadlessHost::new(Document::new(view::demo::demo_project()));
    let report = host.doc.run_drc();
    for v in &report.violations {
        println!("{:?}: {}", v.kind, v.message);
    }
    let png = host.render_view(ViewKind::Board, true, w, h).expect("render");
    std::fs::write(&out, png).expect("write");
    println!("wrote {out}");
}
