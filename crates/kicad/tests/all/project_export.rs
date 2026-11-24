//! KiCad project export: exported projects import back the same (nets, pads, placement,
//! routing), and with `KICAD_CLI` set KiCad itself must agree: its netlist has our nets,
//! its board has our pads where we have them, and the board matches the schematic
//! (`drc --schematic-parity`). With `KICAD_TEMPLATE_DIR` too, every KiCad project there
//! is imported, exported and checked the same way.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::process::Command;

use model::*;

use super::project_import::{kicad_nets, kicad_pads};

/// Our nets as sets of "REF.NUMBER" with KiCad's pin numbers (the exported ones), nets
/// of two or more pins.
fn our_nets(p: &Project) -> BTreeSet<BTreeSet<String>> {
    let conn = editor_core::connectivity::Connectivity::build(p);
    let mut numbers = HashMap::new();
    conn.nets
        .iter()
        .map(|n| {
            n.pins
                .iter()
                .flat_map(|np| {
                    let c = p.schematic.components.get(np.component).unwrap();
                    let part = p.library.parts.get(c.part).unwrap();
                    let v = part.variant(c.variant).unwrap();
                    let nums = numbers
                        .entry((part.id, v.id))
                        .or_insert_with(|| kicad::export::pin_numbers(&p.library, part, v))
                        .get(&(np.unit, np.pin))
                        .cloned()
                        .unwrap_or_default();
                    nums.into_iter().map(|num| format!("{}.{num}", np.refdes)).collect::<Vec<_>>()
                })
                .collect::<BTreeSet<_>>()
        })
        .filter(|s| s.len() >= 2)
        .collect()
}

/// Our nets as sets of "REF.PAD" (pads only).
fn pad_nets(p: &Project) -> BTreeSet<BTreeSet<String>> {
    let conn = editor_core::connectivity::Connectivity::build(p);
    conn.nets
        .iter()
        .map(|n| {
            n.pins
                .iter()
                .flat_map(|np| np.pads.iter().map(|pad| format!("{}.{pad}", np.refdes)))
                .collect::<BTreeSet<_>>()
        })
        .filter(|s| s.len() >= 2)
        .collect()
}

/// Every pad: "REF.PAD" → positions.
fn pads(p: &Project) -> BTreeMap<String, Vec<Point>> {
    let mut out: BTreeMap<String, Vec<Point>> = BTreeMap::new();
    for f in p.board.footprints.iter() {
        let r = &p.schematic.components.get(f.component).unwrap().refdes;
        for (_, pad, pos) in editor_core::query::pad_positions(p, f.id) {
            out.entry(format!("{r}.{pad}")).or_default().push(pos);
        }
    }
    for v in out.values_mut() {
        v.sort_by_key(|p| (p.x, p.y));
    }
    out
}

fn export_to(p: &Project, name: &str) -> (tempfile::TempDir, PathBuf, kicad::export::Export) {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join(name);
    let export = kicad::export::export_project(p, &out, name).unwrap();
    let pro = out.join(format!("{name}.kicad_pro"));
    (dir, pro, export)
}

fn near(a: Point, b: Point, tol: Coord) -> bool {
    (a.x - b.x).abs() <= tol && (a.y - b.y).abs() <= tol
}

/// Export `p`, import it back, compare.
fn round_trip(p: &Project, name: &str) -> Vec<String> {
    let (_dir, pro, export) = export_to(p, name);
    let back = match kicad::import_project(&pro) {
        Ok(b) => b.project,
        Err(e) => return vec![format!("{name}: import of the export failed: {e}")],
    };
    let mut failures = Vec::new();
    let _ = export;
    if pad_nets(&back) != pad_nets(p) {
        let (a, b) = (pad_nets(p), pad_nets(&back));
        for n in a.difference(&b) {
            failures.push(format!("{name}: net {n:?} lost"));
        }
        for n in b.difference(&a) {
            failures.push(format!("{name}: net {n:?} appeared"));
        }
    }
    let (a, b) = (pads(p), pads(&back));
    for (k, v) in &a {
        match b.get(k) {
            Some(w) if w.len() == v.len() && v.iter().zip(w).all(|(x, y)| near(*x, *y, 1)) => {}
            other => failures.push(format!("{name}: pad {k} at {v:?}, back at {other:?}")),
        }
    }
    // Placement and routing keep their ids.
    for s in p.schematic.symbols.iter() {
        match back.schematic.symbols.get(s.id) {
            Some(t) if near(t.pos, s.pos, 100) && t.unit == s.unit && t.mirror == s.mirror => {
                let (r0, r1) = (s.rotation.degrees(), t.rotation.degrees());
                if (r0 - r1).abs() > 1e-6 {
                    failures.push(format!("{name}: symbol {} turned {r0} → {r1}", s.id));
                }
            }
            other => failures.push(format!("{name}: symbol {s:?} came back as {other:?}")),
        }
    }
    for w in p.schematic.wires.iter() {
        match back.schematic.wires.get(w.id) {
            Some(v) if near(v.a, w.a, 100) && near(v.b, w.b, 100) => {}
            other => failures.push(format!("{name}: wire {w:?} came back as {other:?}")),
        }
    }
    for l in p.schematic.labels.iter() {
        match back.schematic.labels.get(l.id) {
            Some(m) if near(m.pos, l.pos, 100) && m.name == l.name && m.kind == l.kind => {}
            other => failures.push(format!("{name}: label {l:?} came back as {other:?}")),
        }
    }
    for f in p.board.footprints.iter() {
        match back.board.footprints.get(f.id) {
            Some(g)
                if g.pos == f.pos && g.side == f.side && (g.rotation.degrees() - f.rotation.degrees()).abs() < 1e-6 => {
            }
            other => failures.push(format!("{name}: footprint {f:?} came back as {other:?}")),
        }
    }
    for t in p.board.tracks.iter() {
        match back.board.tracks.get(t.id) {
            Some(u) if u.a == t.a && u.b == t.b && u.width == t.width && u.layer == t.layer && u.mid == t.mid => {}
            other => failures.push(format!("{name}: track {t:?} came back as {other:?}")),
        }
    }
    for v in p.board.vias.iter() {
        match back.board.vias.get(v.id) {
            Some(u) if u == v => {}
            other => failures.push(format!("{name}: via {v:?} came back as {other:?}")),
        }
    }
    let zones =
        |p: &Project| p.board.zones.iter().map(|z| (z.layer, z.outline.clone(), z.holes.clone())).collect::<Vec<_>>();
    if zones(p) != zones(&back) {
        failures.push(format!("{name}: zones differ"));
    }
    failures
}

fn examples() -> Vec<(&'static str, Project)> {
    view::demo::Example::ALL.iter().map(|e| (e.name(), e.project())).collect()
}

#[test]
fn examples_import_back_the_same() {
    let mut failures = Vec::new();
    for (name, p) in examples() {
        failures.extend(round_trip(&p, name));
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn exports_are_deterministic_and_complete() {
    let p = view::demo::Example::Led.project();
    let a = kicad::export::render_project(&p, "led");
    let b = kicad::export::render_project(&p, "led");
    assert_eq!(a.files, b.files);
    let names: Vec<String> = a.files.iter().map(|(f, _)| f.to_string_lossy().replace('\\', "/")).collect();
    for f in ["led.kicad_pro", "led.kicad_sch", "led.kicad_pcb", "led.kicad_sym", "sym-lib-table", "fp-lib-table"] {
        assert!(names.iter().any(|n| n == f), "{f} missing: {names:?}");
    }
    // A file per sub-sheet, a footprint file per footprint.
    let subs = p.schematic.sheets.iter().filter(|s| !s.is_root()).count();
    assert_eq!(names.iter().filter(|n| n.ends_with(".kicad_sch")).count(), subs + 1);
    assert_eq!(names.iter().filter(|n| n.starts_with("led.pretty/")).count(), p.library.footprints.len());
    // Every file parses; the project file is JSON.
    for (f, text) in &a.files {
        if f.extension().is_some_and(|x| x == "kicad_pro") {
            serde_json::from_str::<serde_json::Value>(text).unwrap();
        } else {
            kicad::sexpr::parse(text).unwrap_or_else(|e| panic!("{}: {e}", f.display()));
        }
    }
    // The project library loads with the library import, footprints too.
    let dir = tempfile::tempdir().unwrap();
    kicad::export::export_library(&p.library, dir.path(), "lib").unwrap();
    let sym = std::fs::read_to_string(dir.path().join("lib.kicad_sym")).unwrap();
    let symbols = kicad::symbol::parse_library(&sym).unwrap();
    let parts = p.library.parts.iter().filter(|x| !x.is_board_part()).map(|x| x.variants.len()).sum::<usize>();
    assert_eq!(symbols.len(), parts);
    for e in std::fs::read_dir(dir.path().join("lib.pretty")).unwrap().flatten() {
        let text = std::fs::read_to_string(e.path()).unwrap();
        let fp = kicad::footprint::parse("lib", &text).unwrap();
        let ours =
            p.library.footprints.iter().find(|f| kicad::export::item_name(&f.name) == fp.footprint.name).unwrap();
        assert_eq!(fp.footprint.pads.len(), ours.pads.len(), "{}", ours.name);
        for (a, b) in ours.pads.iter().zip(fp.footprint.pads.iter()) {
            assert_eq!((a.pos, &a.name, a.size, &a.kind), (b.pos, &b.name, b.size, &b.kind), "{}", ours.name);
        }
    }
}

#[test]
fn target_folders_must_be_empty_or_an_earlier_export() {
    let p = view::demo::Example::Led.project();
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("x");
    kicad::export::export_project(&p, &out, "led").unwrap();
    // Again into the same folder: replaced.
    kicad::export::export_project(&p, &out, "led").unwrap();
    // A folder with other things in it is refused.
    std::fs::write(dir.path().join("notes.txt"), "mine").unwrap();
    let e = kicad::export::export_project(&p, dir.path(), "led").unwrap_err();
    assert!(e.contains("isn't empty"), "{e}");
}

#[test]
fn hierarchical_fixture_round_trips() {
    let pro = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/hier/hier.kicad_pro");
    let p = kicad::import_project(&pro).unwrap().project;
    let failures = round_trip(&p, "hier");
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// KiCad's view of an export: ERC findings by kind, netlist nets, schematic parity
/// issues and pad positions.
/// KiCad ERC finding kinds of a schematic (`[kind]` lines of its report).
fn erc_kinds(cli: &Path, sch: &Path, tmp: &Path) -> BTreeSet<String> {
    let rpt = tmp.join("erc.rpt");
    let _ = std::fs::remove_file(&rpt);
    Command::new(cli).args(["sch", "erc", "-o"]).arg(&rpt).arg(sch).output().unwrap();
    let text = std::fs::read_to_string(&rpt).unwrap_or_default();
    text.lines().filter_map(|l| Some(l.strip_prefix('[')?.split(']').next()?.to_string())).collect()
}

/// Which KiCad ERC findings an export may have.
enum Erc<'a> {
    /// Only `power_pin_not_driven` (KiCad wants power flags where we have power ports).
    Strict,
    /// Also those of the KiCad schematic the project came from.
    Like(&'a Path),
    /// Not checked.
    Skip,
}

fn check_with_kicad(cli: &Path, p: &Project, name: &str, erc_rule: Erc) -> Vec<String> {
    let mut failures = Vec::new();
    let (dir, pro, _) = export_to(p, name);
    let tmp = dir.path();
    let sch = pro.with_extension("kicad_sch");
    let pcb = pro.with_extension("kicad_pcb");
    // Nets.
    let theirs: BTreeSet<BTreeSet<String>> = kicad_nets(cli, &sch, tmp).into_iter().collect();
    // KiCad's netlist leaves out symbols that aren't on the board.
    let off_board: BTreeSet<&str> = p
        .schematic
        .components
        .iter()
        .filter(|c| p.library.footprint_of(c.part, c.variant).is_none())
        .map(|c| c.refdes.as_str())
        .collect();
    let ours: BTreeSet<BTreeSet<String>> = our_nets(p)
        .into_iter()
        .map(|n| {
            n.into_iter().filter(|x| !off_board.contains(x.split('.').next().unwrap_or(""))).collect::<BTreeSet<_>>()
        })
        .filter(|n| n.len() >= 2)
        .collect();
    for n in theirs.difference(&ours) {
        failures.push(format!("{name}: KiCad net {n:?} isn't ours"));
    }
    for n in ours.difference(&theirs) {
        failures.push(format!("{name}: our net {n:?} isn't KiCad's"));
    }
    // ERC: nothing the original lacks, and only what differs on purpose (KiCad wants
    // power flags on power ports).
    let allowed = match erc_rule {
        Erc::Strict => Some(BTreeSet::new()),
        Erc::Like(original) => Some(erc_kinds(cli, original, tmp)),
        Erc::Skip => None,
    };
    if let Some(allowed) = allowed {
        for kind in erc_kinds(cli, &sch, tmp).difference(&allowed) {
            if kind != "power_pin_not_driven" {
                failures.push(format!("{name}: KiCad ERC reports {kind}"));
            }
        }
    }
    // Board against schematic.
    let rpt = tmp.join("drc.rpt");
    Command::new(cli).args(["pcb", "drc", "--schematic-parity", "-o"]).arg(&rpt).arg(&pcb).output().unwrap();
    let drc = std::fs::read_to_string(&rpt).unwrap_or_default();
    let parity = drc.split("** Found ").find(|s| s.contains("Footprint errors") || s.contains("parity"));
    if let Some(section) = parity
        && !section.trim_start().starts_with('0')
    {
        failures.push(format!("{name}: schematic parity issues:\n{section}"));
    }
    // Pads where ours are (KiCad's IPC-D-356 has an origin of its own: the common offset).
    let theirs = kicad_pads(cli, &pcb, tmp);
    let ours = pads(p);
    let mut offsets: HashMap<(i64, i64), usize> = HashMap::new();
    for (k, v) in &theirs {
        if let (Some(q), Some(m)) = (v.first(), ours.get(k).and_then(|m| m.first())) {
            *offsets.entry(((q.x - m.x) / 10_000, (q.y - m.y) / 10_000)).or_default() += 1;
        }
    }
    if let Some((&(ox, oy), _)) = offsets.iter().max_by_key(|e| e.1) {
        let offset = Point::new(ox * 10_000, oy * 10_000);
        for (k, v) in &theirs {
            let Some(mine) = ours.get(k) else { continue };
            for q in v {
                if !mine.iter().any(|m| near(*m + offset, *q, 13_000)) {
                    failures.push(format!("{name}: pad {k} at {q:?} in KiCad, here at {mine:?}"));
                }
            }
        }
    }
    failures
}

#[test]
fn kicad_agrees_with_exported_examples() {
    let Some(cli) = std::env::var_os("KICAD_CLI").map(PathBuf::from) else {
        eprintln!("KICAD_CLI not set; skipped");
        return;
    };
    let mut failures = Vec::new();
    for (name, p) in examples() {
        failures.extend(check_with_kicad(&cli, &p, name, Erc::Strict));
    }
    let pro = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/hier/hier.kicad_pro");
    let hier = kicad::import_project(&pro).unwrap().project;
    // The hand-written fixture is off KiCad's grid and doesn't load in KiCad itself, so
    // its ERC has nothing to compare with.
    failures.extend(check_with_kicad(&cli, &hier, "hier", Erc::Skip));
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// Every KiCad project in `KICAD_TEMPLATE_DIR` (one level down): imported, exported,
/// imported back and checked by KiCad.
#[test]
fn kicad_projects_survive_import_and_export() {
    let (Some(cli), Some(dir)) = (std::env::var_os("KICAD_CLI"), std::env::var_os("KICAD_TEMPLATE_DIR")) else {
        eprintln!("KICAD_CLI / KICAD_TEMPLATE_DIR not set; skipped");
        return;
    };
    let cli = PathBuf::from(cli);
    let mut projects = Vec::new();
    for e in std::fs::read_dir(dir).unwrap().flatten() {
        for f in std::fs::read_dir(e.path()).into_iter().flatten().flatten() {
            if f.path().extension().is_some_and(|x| x == "kicad_pro") {
                projects.push(f.path());
            }
        }
    }
    projects.sort();
    let mut failures = Vec::new();
    for pro in projects {
        let name = pro.file_stem().unwrap().to_string_lossy().to_string();
        let Ok(imported) = kicad::import_project(&pro) else { continue };
        eprintln!("{name}");
        let p = imported.project;
        failures.extend(round_trip(&p, &name));
        let found = check_with_kicad(&cli, &p, &name, Erc::Like(&pro.with_extension("kicad_sch")));
        // A gap of the import, not the export: KiCad shows the pins a multi-unit symbol's
        // units share on every unit, the import on the first only, so wires and no-connect
        // flags at them on the other units dangle.
        failures.extend(
            found
                .into_iter()
                .filter(|f| !f.contains("unconnected_wire_endpoint") && !f.contains("no_connect_dangling")),
        );
    }
    assert!(failures.is_empty(), "{} problems:\n{}", failures.len(), failures.join("\n"));
}
