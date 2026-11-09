//! Render the demo board offscreen (no window needed), after running the design rule
//! check: `cargo run -p mcp --example render_board -- out.png [width height] [--light]
//! [--schematic]`. Development aid for screenshots.

use mcp::Host;
use mcp::headless::HeadlessHost;
use view::{Document, ViewKind};

fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let mut flag = |name: &str| {
        let on = args.iter().any(|a| a == name);
        args.retain(|a| a != name);
        on
    };
    let light = flag("--light");
    let kind = if flag("--schematic") { ViewKind::Schematic } else { ViewKind::Board };
    if light {
        view::style::set_theme(view::style::Theme::Light);
    }
    let out = args.first().cloned().unwrap_or_else(|| "board.png".into());
    let w = args.get(1).and_then(|v| v.parse().ok()).unwrap_or(1200);
    let h = args.get(2).and_then(|v| v.parse().ok()).unwrap_or(900);
    let mut host = HeadlessHost::new(Document::new(view::demo::demo_project()));
    let report = host.doc.run_drc();
    for v in &report.violations {
        println!("{:?}: {}", v.kind, v.message);
    }
    let png = host.render_view(kind, true, w, h).expect("render");
    std::fs::write(&out, png).expect("write");
    println!("wrote {out}");
}
