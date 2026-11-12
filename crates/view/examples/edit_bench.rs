//! Edit latency on a large design: `cargo run -p view --release --example edit_bench [n]`.
//! Measures command + incremental view update (both views), as the GUI does per frame.

use std::time::Instant;

use model::*;
use view::demo::stress_project;
use view::{Document, Interaction, KeyAction, PointerInput, ViewId};

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
        doc.schematic().scene.instance_count(),
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
    it.pointer(&mut doc.target(doc.root()), &input(sym.pos, true, true, false));
    let mut times = Vec::new();
    for i in 1..=200 {
        let p = sym.pos + Point::mm(i as f64 * 0.7, (i % 7) as f64);
        let t = Instant::now();
        it.pointer(&mut doc.target(doc.root()), &input(p, false, true, false));
        times.push(t.elapsed().as_secs_f64() * 1e3);
    }
    it.pointer(&mut doc.target(doc.root()), &input(sym.pos, false, false, true));
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
        hits += doc.schematic().hit(doc.project(), p, mm(0.3)).is_some() as usize;
    }
    println!("hit test: {:.4} ms each ({hits} hits / 1000)", t.elapsed().as_secs_f64());

    for (label, action) in [("select all", KeyAction::SelectAll), ("rotate all", KeyAction::Rotate)] {
        let t = Instant::now();
        it.key(&mut doc.target(doc.root()), action);
        println!("{label}: {:?}", t.elapsed());
    }
    let t = Instant::now();
    doc.undo();
    it.refresh(&mut doc.target(doc.root()));
    println!("undo rotate all: {:?}", t.elapsed());

    let t = Instant::now();
    it.key(&mut doc.target(doc.root()), KeyAction::Delete);
    println!("delete all: {:?}", t.elapsed());
    let t = Instant::now();
    doc.undo();
    println!("undo delete all: {:?}", t.elapsed());
    it.key(&mut doc.target(doc.root()), KeyAction::SelectAll);

    // Editing the Project Library: each drag step in the symbol / footprint editor is a
    // project command that redraws the parts using it (here: every capacitor).
    let cap = doc.project().library.parts.get(model::builtin::PART_CAPACITOR).unwrap().clone();
    let mut sym = doc.project().library.symbols.get(cap.units[0].symbol).unwrap().clone();
    let mut fp = doc.project().library.footprint_of(cap.id, Some(cap.variants[0].id)).unwrap().clone();
    let users = doc.project().schematic.components.iter().filter(|c| c.part == cap.id).count();
    let pad = fp.pads.ids().next().unwrap();
    sym.graphics.push(Shape::Line { a: Point::ZERO, b: Point::mm(1.0, 0.0), width: mm(0.2) });
    let step = |doc: &mut Document, o: Object| {
        let t = Instant::now();
        doc.execute_grouped(editor_core::Command::Update { objects: vec![o] }, Some(77)).unwrap();
        doc.update_connectivity(true);
        t.elapsed().as_secs_f64() * 1e3
    };
    let mut line = Vec::new();
    let mut pads = Vec::new();
    for i in 0..50 {
        if let Some(Shape::Line { b, .. }) = sym.graphics.last_mut() {
            b.x += mm(0.1);
        }
        line.push(step(&mut doc, Object::LibSymbol(sym.clone())));
        fp.pads.get_mut(pad).unwrap().pos.x += mm(0.01) * if i % 2 == 0 { 1 } else { -1 };
        pads.push(step(&mut doc, Object::LibFootprint(fp.clone())));
    }
    doc.close_group();
    let t = Instant::now();
    doc.update_connectivity(false);
    println!("  catching up at the end of the drag: {:?}", t.elapsed());
    for (label, mut times) in [("symbol line", line), ("footprint pad", pads)] {
        times.sort_by(f64::total_cmp);
        println!(
            "drag in the Project Library, {label} ({users} users, per frame, redraws wait): p50 {:.3} ms  max {:.3} ms",
            times[times.len() / 2],
            times[times.len() - 1]
        );
    }
    {
        let mut ed = editor_core::Editor::new(doc.project().clone());
        fp.pads.get_mut(pad).unwrap().pos.x += mm(0.01);
        let t = Instant::now();
        let changes =
            ed.execute(editor_core::Command::Update { objects: vec![Object::LibFootprint(fp.clone())] }).unwrap();
        println!("  pad: editor only {:?}", t.elapsed());
        let mut v = view::View::new(view::ViewId::root(ed.project()), ed.project());
        let t = Instant::now();
        v.apply(ed.project(), &changes);
        println!("  pad: schematic view apply {:?}", t.elapsed());
        let mut b = view::View::new(ViewId::Board, ed.project());
        let t = Instant::now();
        b.apply(ed.project(), &changes);
        println!("  pad: board view apply {:?}", t.elapsed());
    }
    doc.update_connectivity(false);
    let t = Instant::now();
    doc.restyle();
    println!("  for comparison, rebuilding both views: {:?}", t.elapsed());

    // Where does a bulk edit spend its time?
    let ids = it.selection.clone();
    let mut ed = editor_core::Editor::new(doc.project().clone());
    let t = Instant::now();
    let changes = ed
        .execute(editor_core::Command::Rotate { ids, center: Point::ZERO, angle: Angle::DEG90, keep_connected: false })
        .unwrap();
    println!("  editor only: {:?} ({} changes)", t.elapsed(), changes.changes.len());
    let mut v = view::View::new(view::ViewId::root(ed.project()), ed.project());
    let t = Instant::now();
    v.apply(ed.project(), &changes);
    println!("  schematic view apply: {:?}", t.elapsed());
    let mut b = view::View::new(ViewId::Board, ed.project());
    let t = Instant::now();
    b.apply(ed.project(), &changes);
    println!("  board view apply: {:?}", t.elapsed());
}
