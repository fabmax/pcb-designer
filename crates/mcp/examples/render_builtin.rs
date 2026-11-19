//! Contact sheets of the Built-in library: one component per variant of the parts whose
//! name starts with one of the given prefixes, footprints packed on the board, symbols
//! on the schematic. Development aid for reviewing library parts:
//! `cargo run -p mcp --example render_builtin -- OUT_PREFIX [--light] PREFIX…`
//! writes `OUT_PREFIX-board.png` and `OUT_PREFIX-schematic.png`.

use editor_core::Command;
use mcp::Host;
use mcp::headless::HeadlessHost;
use model::*;
use view::{Document, ViewId};

fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    if let Some(i) = args.iter().position(|a| a == "--light") {
        args.remove(i);
        view::style::set_theme(view::style::Theme::Light);
    }
    let out = args.remove(0);
    let lib = builtin::builtin_library();
    let parts: Vec<&Part> =
        lib.parts.iter().filter(|p| args.is_empty() || args.iter().any(|a| p.name.starts_with(a.as_str()))).collect();
    let ids: Vec<Id> = parts.iter().map(|p| p.id).collect();
    let mut doc = Document::new(Project::new("Built-in"));
    doc.execute(Command::import_parts(&lib, &ids).unwrap()).unwrap();
    // No page frame, so the schematic view fits the symbols.
    let mut root =
        doc.project().schematic.root_sheet().map(|s| doc.project().schematic.sheets.get(s).unwrap().clone()).unwrap();
    root.page.size = PageSize::None;
    doc.execute(Command::Update { objects: vec![Object::Sheet(root)] }).unwrap();
    // Footprints in shelves 120 mm wide, symbols in a grid.
    let (mut x, mut y, mut shelf) = (0.0_f64, 0.0_f64, 0.0_f64);
    let mut n = 0;
    for part in &parts {
        for v in &part.variants {
            let fp = lib.footprints.get(v.footprint.unwrap()).unwrap();
            let court = fp
                .graphics
                .iter()
                .find(|g| g.layer == FootprintLayer::Courtyard)
                .map(|g| g.shape.bounds())
                .unwrap_or_else(|| Rect::from_points(Point::ZERO, Point::ZERO));
            let (w, h) = (to_mm(court.width()) + 2.0, to_mm(court.height()) + 2.0);
            if x + w > 120.0 {
                (x, y, shelf) = (0.0, y - shelf, 0.0);
            }
            let at = Point::mm(x, y) - Point::new(court.min.x, court.max.y) + Point::mm(1.0, -1.0);
            x += w;
            shelf = shelf.max(h);
            let sch = Point::mm((n % 10) as f64 * 25.4, -((n / 10) as f64) * 20.32);
            n += 1;
            let command = if part.is_board_part() {
                Command::PlaceBoardPart {
                    part: part.id,
                    variant: Some(v.id),
                    pos: Point::ZERO,
                    rotation: Angle::ZERO,
                    side: Side::Top,
                    refdes: None,
                    value: Some(v.name.clone()),
                }
            } else {
                Command::PlacePart {
                    part: part.id,
                    variant: Some(v.id),
                    sheet: None,
                    pos: sch,
                    rotation: Angle::ZERO,
                    mirror: false,
                    refdes: None,
                    value: Some(v.name.clone()),
                }
            };
            let changes = doc.execute(command).unwrap();
            let f = changes.ids().find_map(|id| if let ObjectId::Footprint(f) = id { Some(f) } else { None });
            let f = f.unwrap();
            let pos = doc.project().board.footprints.get(f).unwrap().pos;
            doc.execute(Command::Move { ids: vec![ObjectId::Footprint(f)], delta: at - pos, keep_connected: false })
                .unwrap();
        }
    }
    let mut host = HeadlessHost::new(doc);
    let root = ViewId::root(host.doc.project());
    for (view, name, size) in [(ViewId::Board, "board", (1600, 1600)), (root, "schematic", (1600, 1000))] {
        let png = host.render_view(view, true, size.0, size.1).expect("render");
        std::fs::write(format!("{out}-{name}.png"), png).expect("write");
    }
    println!("{n} variants: {out}-board.png, {out}-schematic.png");
}
