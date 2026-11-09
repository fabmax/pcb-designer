//! Edit latency on a large design: `cargo run -p view --release --example edit_bench [n]`.
//! Measures command + incremental view update (both views), as the GUI does per frame.

use std::time::Instant;

use model::*;
use view::demo::stress_project;
use view::{Document, Interaction, KeyAction, PointerInput, ViewKind};

fn main() {
    let n: usize = std::env::args().nth(1).and_then(|v| v.parse().ok()).unwrap_or(10_000);
    let t = Instant::now();
    let mut doc = Document::new(stress_project(n));
    println!("{n} components: generate + build views {:?}", t.elapsed());
    for _ in 0..3 {
        let t = Instant::now();
        let c = editor_core::connectivity::Connectivity::build(doc.project());
        println!(
            "connectivity: {} nets, {} dots, {} unconnected pins in {:?}",
            c.nets.len(),
            c.dots.len(),
            c.unconnected.len(),
            t.elapsed()
        );
        let t = Instant::now();
        let b = editor_core::copper::BoardConnectivity::build(doc.project(), &c);
        println!(
            "board connectivity: {} copper items, {} ratsnest lines, {} shorts in {:?}",
            b.items.len(),
            b.ratsnest.len(),
            b.shorts.len(),
            t.elapsed()
        );
    }
    // Design rule check, then a zone of the largest net over the whole board.
    let t = Instant::now();
    let v = doc.run_drc().violations.len();
    println!("drc: {v} violations in {:?}", t.elapsed());
    let copper = doc.current_copper();
    let mut counts = vec![0usize; copper.nets.len()];
    for c in &copper.items {
        if let Some(n) = c.net {
            counts[n as usize] += 1;
        }
    }
    let net = (0..counts.len()).max_by_key(|&i| counts[i]).map(|i| copper.nets[i].clone()).unwrap_or_default();
    let r = Rect::union_all(copper.items.iter().map(|c| c.geo.bounds())).unwrap().grow(mm(2.0));
    let outline = vec![r.min, Point::new(r.max.x, r.min.y), r.max, Point::new(r.min.x, r.max.y)];
    let zone =
        Zone { id: Id::new(), layer: LayerId::TOP_COPPER, net: net.clone(), outline, priority: 0, clearance: None };
    let t = Instant::now();
    doc.execute(editor_core::Command::Insert { objects: vec![Object::Zone(zone)] }).unwrap();
    let b = doc.current_copper();
    let pieces: usize = b.fills.iter().map(|f| f.shapes.len()).sum();
    let vertices: usize = b.fills.iter().flat_map(|f| &f.shapes).flatten().map(Vec::len).sum();
    println!(
        "zone of {net} ({} items): {pieces} piece(s), {vertices} vertices, fill + connectivity {:?}",
        counts.iter().max().unwrap_or(&0),
        t.elapsed()
    );
    let t = Instant::now();
    let v = doc.run_drc().violations.len();
    println!("drc with the zone: {v} violations in {:?}", t.elapsed());

    println!(
        "  schematic {} prims, board {} prims",
        doc.schematic.scene.instance_count(),
        doc.board.scene.instance_count()
    );

    let mut it = Interaction::new(mm(1.27));
    let sym = doc.project().schematic.symbols.iter().nth(n / 2).unwrap().clone();
    let input = |p: Point, pressed, down, released| PointerInput {
        world: Some(p),
        pressed,
        down,
        released,
        tolerance: mm(0.3),
        drag_threshold: mm(0.5),
        ..Default::default()
    };
    it.pointer(&mut doc.target(ViewKind::Schematic), &input(sym.pos, true, true, false));
    let mut times = Vec::new();
    for i in 1..=200 {
        let p = sym.pos + Point::mm(i as f64 * 0.7, (i % 7) as f64);
        let t = Instant::now();
        it.pointer(&mut doc.target(ViewKind::Schematic), &input(p, false, true, false));
        times.push(t.elapsed().as_secs_f64() * 1e3);
    }
    it.pointer(&mut doc.target(ViewKind::Schematic), &input(sym.pos, false, false, true));
    times.sort_by(f64::total_cmp);
    println!(
        "drag one symbol (per frame): p50 {:.3} ms  p95 {:.3} ms  max {:.3} ms",
        times[times.len() / 2],
        times[times.len() * 95 / 100],
        times[times.len() - 1]
    );

    let t = Instant::now();
    let mut hits = 0;
    for i in 0..1000 {
        let p = Point::mm((i % 100) as f64 * 20.32 + 3.0, (i / 10) as f64 * 1.3);
        hits += doc.schematic.hit(doc.project(), p, mm(0.3)).is_some() as usize;
    }
    println!("hit test: {:.4} ms each ({hits} hits / 1000)", t.elapsed().as_secs_f64());

    for (label, action) in [("select all", KeyAction::SelectAll), ("rotate all", KeyAction::Rotate)] {
        let t = Instant::now();
        it.key(&mut doc.target(ViewKind::Schematic), action);
        println!("{label}: {:?}", t.elapsed());
    }
    let t = Instant::now();
    doc.undo();
    it.refresh(&mut doc.target(ViewKind::Schematic));
    println!("undo rotate all: {:?}", t.elapsed());

    let t = Instant::now();
    it.key(&mut doc.target(ViewKind::Schematic), KeyAction::Delete);
    println!("delete all: {:?}", t.elapsed());
    let t = Instant::now();
    doc.undo();
    println!("undo delete all: {:?}", t.elapsed());
    it.key(&mut doc.target(ViewKind::Schematic), KeyAction::SelectAll);

    // Where does a bulk edit spend its time?
    let ids = it.selection.clone();
    let mut ed = editor_core::Editor::new(doc.project().clone());
    let t = Instant::now();
    let changes = ed
        .execute(editor_core::Command::Rotate { ids, center: Point::ZERO, angle: Angle::DEG90, keep_connected: false })
        .unwrap();
    println!("  editor only: {:?} ({} changes)", t.elapsed(), changes.changes.len());
    let mut v = view::View::new(ViewKind::Schematic, ed.project());
    let t = Instant::now();
    v.apply(ed.project(), &changes);
    println!("  schematic view apply: {:?}", t.elapsed());
    let mut b = view::View::new(ViewKind::Board, ed.project());
    let t = Instant::now();
    b.apply(ed.project(), &changes);
    println!("  board view apply: {:?}", t.elapsed());
}
