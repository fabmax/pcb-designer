//! Headless live-sync benchmark.
//!
//! Usage: sync_bench [components=10000] [edits=600] [seed=1]

use std::collections::BTreeMap;
use std::time::Duration;

use spike::{Project, SyncReport};

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

fn pct(sorted: &[f64], p: f64) -> f64 {
    sorted[((sorted.len() - 1) as f64 * p).round() as usize]
}

fn main() {
    let args: Vec<usize> = std::env::args()
        .skip(1)
        .map(|a| a.parse().expect("numeric argument"))
        .collect();
    let n = args.first().copied().unwrap_or(10_000);
    let edits = args.get(1).copied().unwrap_or(600);
    let seed = args.get(2).copied().unwrap_or(1) as u64;

    let (mut p, s) = Project::generate(n, seed);
    println!("== Synthetic project ({n} components requested, seed {seed})");
    println!(
        "components {}  pins {}  wires {}  labels {}  nets {}  tracks {}  vias {}",
        s.components, s.pins, s.wires, s.labels, s.nets, s.tracks, s.vias
    );
    println!(
        "render instances: schematic {}  pcb {}",
        s.sch_instances, s.pcb_instances
    );
    println!(
        "initial build: generate {:.1} ms, connectivity {:.1} ms, scenes {:.1} ms, ratsnest {:.1} ms",
        ms(s.t_generate),
        ms(s.t_connectivity),
        ms(s.t_scenes),
        ms(s.t_ratsnest)
    );
    let gnd = p.conn.nets().map(|(_, n)| n.pins.len()).max().unwrap_or(0);
    println!("largest net: {gnd} pins");

    // Initial upload happens once when the project is opened.
    let _ = p.sch_scene.take_upload();
    let _ = p.pcb_scene.take_upload();

    let mut by_kind: BTreeMap<&str, Vec<SyncReport>> = BTreeMap::new();
    let mut uploads: Vec<(f64, usize)> = Vec::new();
    for _ in 0..edits {
        let e = p.random_edit();
        let r = p.apply(e);
        by_kind.entry(r.kind).or_default().push(r);
        // One frame per edit: extract the GPU diff like the app does.
        let t = std::time::Instant::now();
        let bytes = p.sch_scene.take_upload().bytes() + p.pcb_scene.take_upload().bytes();
        uploads.push((ms(t.elapsed()), bytes));
    }

    println!("\n== {edits} random edits (times in ms)");
    println!(
        "{:<17} {:>5} {:>7} {:>7} {:>7} {:>7} | {:>6} {:>6} {:>6} {:>6}",
        "edit", "n", "p50", "p95", "max", "mean", "sch", "conn", "board", "rats"
    );
    let mut all = Vec::new();
    for (kind, rs) in &by_kind {
        let mut tot: Vec<f64> = rs.iter().map(|r| ms(r.total)).collect();
        tot.sort_by(f64::total_cmp);
        all.extend_from_slice(&tot);
        let mean = |f: fn(&SyncReport) -> Duration| rs.iter().map(|r| ms(f(r))).sum::<f64>() / rs.len() as f64;
        println!(
            "{:<17} {:>5} {:>7.3} {:>7.3} {:>7.3} {:>7.3} | {:>6.3} {:>6.3} {:>6.3} {:>6.3}",
            kind,
            rs.len(),
            pct(&tot, 0.5),
            pct(&tot, 0.95),
            tot.last().unwrap(),
            mean(|r| r.total),
            mean(|r| r.schematic),
            mean(|r| r.connectivity),
            mean(|r| r.board),
            mean(|r| r.ratsnest),
        );
    }
    all.sort_by(f64::total_cmp);
    println!(
        "{:<17} {:>5} {:>7.3} {:>7.3} {:>7.3} {:>7.3}",
        "ALL",
        all.len(),
        pct(&all, 0.5),
        pct(&all, 0.95),
        all.last().unwrap(),
        all.iter().sum::<f64>() / all.len() as f64
    );
    let mut ut: Vec<f64> = uploads.iter().map(|u| u.0).collect();
    ut.sort_by(f64::total_cmp);
    let mut ub: Vec<f64> = uploads.iter().map(|u| u.1 as f64 / 1024.0).collect();
    ub.sort_by(f64::total_cmp);
    println!(
        "\nper-edit GPU diff: time p50 {:.3} / p95 {:.3} / max {:.3} ms, size p50 {:.1} / p95 {:.1} / max {:.1} KiB",
        pct(&ut, 0.5),
        pct(&ut, 0.95),
        ut.last().unwrap(),
        pct(&ub, 0.5),
        pct(&ub, 0.95),
        ub.last().unwrap()
    );
}
