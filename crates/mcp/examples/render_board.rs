//! Render the demo board offscreen (no window needed), after running the design rule
//! check: `cargo run -p mcp --example render_board -- out.png [width height] [--light]
//! [--schematic | --sheet NAME] [--show-layer NAME]…` (shows a board layer hidden by
//! default, e.g. "Top Solder Mask"). Development aid for screenshots.

use mcp::Host;
use mcp::headless::HeadlessHost;
use view::{Document, ViewId};

fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let mut flag = |name: &str| {
        let on = args.iter().any(|a| a == name);
        args.retain(|a| a != name);
        on
    };
    let light = flag("--light");
    let schematic = flag("--schematic");
    let sheet = args.iter().position(|a| a == "--sheet").map(|i| {
        args.remove(i);
        args.remove(i)
    });
    let mut shown = Vec::new();
    while let Some(i) = args.iter().position(|a| a == "--show-layer") {
        args.remove(i);
        shown.push(args.remove(i));
    }
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
    let scene = &mut host.doc.board.scene;
    for i in 0..scene.layer_count() as u16 {
        if shown.contains(&scene.layer(i).name) {
            scene.layer_mut(i).visible = true;
        }
    }
    let project = host.doc.project();
    let view = match sheet {
        Some(s) => ViewId::Sheet(project.schematic.find_sheet(&s).expect("no such sheet")),
        None if schematic => ViewId::root(project),
        None => ViewId::Board,
    };
    let png = host.render_view(view, true, w, h).expect("render");
    std::fs::write(&out, png).expect("write");
    println!("wrote {out}");
}
