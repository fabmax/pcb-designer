//! KiCad project import, checked against KiCad itself: for every project in
//! `KICAD_TEMPLATE_DIR` (e.g. KiCad's `template` folder), `KICAD_CLI` exports the
//! netlist and the IPC-D-356 pad list, and the imported project must have the same
//! nets (by reference and pin) and every pad at the same place. Skipped unless both
//! variables are set.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::process::Command;

use editor_core::copper::BoardConnectivity;
use editor_core::drc::ViolationKind;
use kicad::sexpr;
use model::*;

/// A hand-written KiCad 9 project: a sheet used twice, a power port, a routed board with
/// an arc, vias, a zone on a net named by KiCad, a keepout and a rounded outline.
fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/hier/hier.kicad_pro")
}

#[test]
fn hierarchical_routed_project() {
    let imported = kicad::import_project(&fixture()).unwrap();
    let p = &imported.project;
    let report = imported.report.lines().join("\n");

    // Sheets: the sub-sheet file is used twice.
    let names: Vec<&str> = p.schematic.sheets.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, ["Schematic", "amp", "amp2"]);
    let sheet_of = |r: &str| {
        let c = p.schematic.component_by_refdes(r).unwrap().id;
        let s = p.schematic.symbols.iter().find(|s| s.component == c).unwrap().sheet;
        p.schematic.sheets.get(s).unwrap().name.clone()
    };
    assert_eq!((sheet_of("R1"), sheet_of("R2"), sheet_of("R3")), ("amp".into(), "amp2".into(), "Schematic".into()));
    assert!(report.contains("wires to them were stretched"), "{report}");

    // Pages: each sheet file's paper; the content is shifted onto it.
    let page_of = |name: &str| p.schematic.sheets.iter().find(|s| s.name == name).unwrap().page;
    assert_eq!(page_of("Schematic"), model::Page { size: model::PageSize::A4, portrait: false });
    assert_eq!(page_of("amp2"), model::Page { size: model::PageSize::A3, portrait: true });
    for w in p.schematic.wires.iter() {
        let page = p.schematic.sheets.get(w.sheet).unwrap().page.rect().unwrap();
        assert!(page.contains(w.a) && page.contains(w.b), "wire {:?}–{:?} off its page", w.a, w.b);
    }
    let tb = p.schematic.title_block();
    assert_eq!((tb.title.as_str(), tb.date.as_str(), tb.revision.as_str()), ("Hier test", "2026-11-15", "B"));
    assert_eq!((tb.company.as_str(), tb.comment.as_str()), ("ACME", "\nsecond"));
    assert!(!report.contains("title block"), "{report}");

    // Nets as KiCad has them (power port included).
    let mut nets: Vec<BTreeSet<String>> = our_nets(p);
    nets.sort();
    let expected: Vec<BTreeSet<String>> =
        vec![["R1.2", "R2.1"].map(String::from).into(), ["R2.2", "R3.1"].map(String::from).into()];
    assert_eq!(nets, expected);
    let conn = editor_core::connectivity::Connectivity::build(p);
    assert!(conn.nets.iter().any(|n| n.name == "GND" && n.pins.iter().any(|x| x.refdes == "R3")));
    assert!(conn.nets.iter().any(|n| n.name == "VIN" && n.pins.iter().any(|x| x.refdes == "R1")));

    // PWR_FLAG is a power flag; the project's ERC and DRC severities come along.
    assert_eq!(p.schematic.power_flags.len(), 1);
    let flag = p.schematic.power_flags.iter().next().unwrap().id;
    assert!(conn.nets.iter().any(|n| n.name == "GND" && n.power_flags.contains(&flag)));
    assert!(!report.contains("PWR_FLAG"), "{report}");
    assert_eq!(p.schematic.erc.severity(ErcRule::NetNames), Severity::Error);
    assert_eq!(p.schematic.erc.severity(ErcRule::UnconnectedPin), Severity::Warning);
    assert_eq!(p.schematic.erc.pin_pair(PinKind::Output, PinKind::Output), Severity::Warning);
    assert_eq!(p.board.drc.severity(ViolationKind::Courtyard), Severity::Ignore);

    // The board is fully routed, with an arc and two vias.
    let bc = BoardConnectivity::build(p, &conn);
    assert_eq!(bc.unrouted(), 0, "{:?}", bc.ratsnest);
    assert!(bc.shorts.is_empty(), "{:?}", bc.shorts);
    assert_eq!(bc.flagged().count(), 0);
    assert_eq!(p.board.tracks.iter().filter(|t| t.mid.is_some()).count(), 1);
    assert_eq!(p.board.vias.len(), 2);

    // The zone takes our name of its net, keeps its hole and settings, and fills.
    let r3_net = conn.nets.iter().find(|n| n.pins.iter().any(|x| x.refdes == "R3" && x.pin_number == "1")).unwrap();
    let zone = p.board.zones.iter().next().unwrap();
    assert_eq!(zone.net, r3_net.name);
    assert_eq!((zone.layer, zone.holes.len(), zone.connection), (LayerId::BOTTOM_COPPER, 1, ZoneConnection::Solid));
    assert_eq!((zone.islands, zone.clearance), (IslandRemoval::Never, Some(300_000)));
    assert!(bc.fills.iter().any(|f| !f.shapes.is_empty()));

    // The keepout.
    let k = p.board.keepouts.iter().next().unwrap();
    assert_eq!(k.layers, [LayerId::TOP_COPPER, LayerId::BOTTOM_COPPER]);
    assert!(k.no_tracks && k.no_vias && k.no_pour && !k.no_pads && !k.no_footprints);

    // The rounded outline closes; DRC finds nothing of substance.
    assert_eq!(editor_core::zones::board_area(&p.board).unwrap().len(), 1);
    let v = editor_core::drc::run(p, &bc);
    assert!(
        v.iter().all(|v| !matches!(
            v.kind,
            ViolationKind::Short | ViolationKind::Unrouted | ViolationKind::Unconnected | ViolationKind::Keepout
        )),
        "{v:?}"
    );

    // Rules and the report.
    let r = &p.board.rules;
    assert_eq!((r.clearance, r.track_width, r.min_track_width, r.min_drill), (200_000, 250_000, 150_000, 250_000));
    assert_eq!((r.mask_expansion, r.paste_margin), (50_000, -30_000));
    for s in ["net class Power", "schematic graphics", "board texts on documentation", "zone minimum widths"] {
        assert!(report.contains(s), "{s}: {report}");
    }

    // Board texts import in Bold (silkscreen that fabricates), schematic ones as drawn.
    assert!(p.board.fields.iter().all(|f| f.text.bold) && p.board.texts.iter().all(|t| t.text.bold));
    assert!(p.schematic.fields.iter().all(|f| !f.text.bold));
    // Each footprint has its reference and value, as placed in KiCad.
    for f in p.board.footprints.iter() {
        assert_eq!(p.board.fields.iter().filter(|x| x.footprint == f.id).count(), 2);
    }
    // So does each symbol: R1's texts sit 2 mm right of it, the value 2 mm lower (KiCad's
    // y points down).
    let r1 = p.schematic.component_by_refdes("R1").unwrap().id;
    let sym = p.schematic.symbols.iter().find(|s| s.component == r1).unwrap();
    let field = |t: &str| p.schematic.fields.iter().find(|f| f.symbol == sym.id && f.text.text == t).unwrap();
    assert_eq!(field("${REF}").text.pos, Point::mm(2.0, 0.0));
    assert_eq!(field("${VALUE}").text.pos, Point::mm(2.0, -2.0));
    for s in p.schematic.symbols.iter() {
        assert_eq!(p.schematic.fields.iter().filter(|f| f.symbol == s.id).count(), 2, "{s:?}");
    }
}

fn projects() -> Option<(PathBuf, Vec<PathBuf>)> {
    let cli = PathBuf::from(std::env::var("KICAD_CLI").ok()?);
    let dir = PathBuf::from(std::env::var("KICAD_TEMPLATE_DIR").ok()?);
    let mut out = Vec::new();
    for e in std::fs::read_dir(dir).ok()?.flatten() {
        for f in std::fs::read_dir(e.path()).into_iter().flatten().flatten() {
            if f.path().extension().is_some_and(|x| x == "kicad_pro") {
                out.push(f.path());
            }
        }
    }
    out.sort();
    out.push(fixture());
    Some((cli, out))
}

/// KiCad's nets as sets of "REF.PIN" (nets with two or more pins).
fn kicad_nets(cli: &Path, sch: &Path, tmp: &Path) -> Vec<BTreeSet<String>> {
    let out = tmp.join("net.net");
    let ok = Command::new(cli)
        .args(["sch", "export", "netlist", "--format", "kicadsexpr", "-o"])
        .arg(&out)
        .arg(sch)
        .output()
        .unwrap();
    assert!(ok.status.success(), "{}", String::from_utf8_lossy(&ok.stderr));
    let root = sexpr::parse(&std::fs::read_to_string(&out).unwrap()).unwrap();
    let mut nets = Vec::new();
    for net in root.child("nets").unwrap().children("net") {
        let pins: BTreeSet<String> = net
            .children("node")
            .filter_map(|n| {
                let r = n.child("ref")?.arg(0)?;
                let pin = n.child("pin")?.arg(0)?;
                (!r.starts_with('#')).then(|| format!("{r}.{pin}"))
            })
            .collect();
        if pins.len() >= 2 {
            nets.push(pins);
        }
    }
    nets
}

/// Our nets as sets of "REF.PIN".
fn our_nets(p: &Project) -> Vec<BTreeSet<String>> {
    let conn = editor_core::connectivity::Connectivity::build(p);
    conn.nets
        .iter()
        .map(|n| n.pins.iter().map(|np| format!("{}.{}", np.refdes, np.pin_number)).collect::<BTreeSet<_>>())
        .filter(|s| s.len() >= 2)
        .collect()
}

/// Pad positions from KiCad's IPC-D-356 export: "REF.PAD" → positions (nm, y up).
fn kicad_pads(cli: &Path, pcb: &Path, tmp: &Path) -> HashMap<String, Vec<Point>> {
    let out = tmp.join("pads.d356");
    let ok = Command::new(cli).args(["pcb", "export", "ipcd356", "-o"]).arg(&out).arg(pcb).output().unwrap();
    assert!(ok.status.success(), "{}", String::from_utf8_lossy(&ok.stderr));
    let text = std::fs::read_to_string(&out).unwrap();
    let mut pads: HashMap<String, Vec<Point>> = HashMap::new();
    for line in text.lines().filter(|l| l.starts_with("317") || l.starts_with("327")) {
        let refdes = line.get(20..26).unwrap_or("").trim();
        let pad = line.get(27..31).unwrap_or("").trim();
        if refdes == "VIA" {
            continue;
        }
        let Some(xi) = line.find("PA").and_then(|i| line[i..].find('X').map(|j| i + j)) else { continue };
        let num = |s: &str| -> Option<(i64, usize)> {
            let sign = if s.starts_with('-') { -1 } else { 1 };
            let digits: String = s[1..].chars().take_while(|c| c.is_ascii_digit()).collect();
            Some((sign * digits.parse::<i64>().ok()?, digits.len() + 1))
        };
        let Some((x, n)) = num(&line[xi + 1..]) else { continue };
        let yi = xi + 1 + n;
        let Some((y, _)) = num(&line[yi + 1..]) else { continue };
        // 0.0001 inch units.
        pads.entry(format!("{refdes}.{pad}")).or_default().push(Point::new(x * 2540, y * 2540));
    }
    pads
}

#[test]
fn templates_import_like_kicad() {
    let Some((cli, projects)) = projects() else {
        eprintln!("KICAD_CLI / KICAD_TEMPLATE_DIR not set; skipped");
        return;
    };
    let tmp = tempfile::tempdir().unwrap();
    let mut failures = Vec::new();
    for pro in &projects {
        let name = pro.file_stem().unwrap().to_string_lossy().to_string();
        let imported = match kicad::import_project(pro) {
            Ok(i) => i,
            Err(e) => {
                failures.push(format!("{name}: {e}"));
                continue;
            }
        };
        let p = &imported.project;
        eprintln!("{name}: {} components, {} footprints", p.schematic.components.len(), p.board.footprints.len());
        for l in imported.report.lines() {
            eprintln!("    {l}");
        }

        // Nets.
        let sch = pro.with_extension("kicad_sch");
        let theirs: BTreeSet<BTreeSet<String>> = kicad_nets(&cli, &sch, tmp.path()).into_iter().collect();
        let ours: BTreeSet<BTreeSet<String>> = our_nets(p).into_iter().collect();
        for n in theirs.difference(&ours) {
            failures.push(format!("{name}: KiCad net {n:?} differs here"));
        }
        for n in ours.difference(&theirs) {
            failures.push(format!("{name}: our net {n:?} isn't one of KiCad's"));
        }

        // Pads.
        let pcb = pro.with_extension("kicad_pcb");
        if !pcb.exists() {
            continue;
        }
        let theirs = kicad_pads(&cli, &pcb, tmp.path());
        let mut ours: BTreeMap<String, Vec<Point>> = BTreeMap::new();
        for f in p.board.footprints.iter() {
            let r = &p.schematic.components.get(f.component).unwrap().refdes;
            for (_, pad, pos) in editor_core::query::pad_positions(p, f.id) {
                ours.entry(format!("{r}.{pad}")).or_default().push(pos);
            }
        }
        // KiCad writes IPC-D-356 relative to an origin of its own: the most common
        // offset between the two is that origin.
        let mut offsets: HashMap<(i64, i64), usize> = HashMap::new();
        for (key, positions) in &theirs {
            if let (Some(q), Some(m)) = (positions.first(), ours.get(key).and_then(|m| m.first())) {
                *offsets.entry(((q.x - m.x) / 10_000, (q.y - m.y) / 10_000)).or_default() += 1;
            }
        }
        let Some((&(ox, oy), _)) = offsets.iter().max_by_key(|e| e.1) else { continue };
        let offset = Point::new(ox * 10_000, oy * 10_000);
        let ours: BTreeMap<String, Vec<Point>> =
            ours.into_iter().map(|(k, v)| (k, v.into_iter().map(|m| m + offset).collect())).collect();
        for (key, positions) in &theirs {
            let Some(mine) = ours.get(key) else {
                failures.push(format!("{name}: pad {key} missing"));
                continue;
            };
            for q in positions {
                if !mine.iter().any(|m| (m.x - q.x).abs() <= 13_000 && (m.y - q.y).abs() <= 13_000) {
                    failures.push(format!("{name}: pad {key} at {q:?} in KiCad, here at {mine:?}"));
                }
            }
        }
    }
    assert!(failures.is_empty(), "{} problems:\n{}", failures.len(), failures.join("\n"));
}

#[test]
fn legacy_projects_and_hidden_power_pins_are_refused() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("old.pro"), "").unwrap();
    std::fs::write(dir.path().join("old.kicad_pro"), "{}").unwrap();
    std::fs::write(dir.path().join("old.sch"), "EESchema Schematic File Version 4").unwrap();
    let e = kicad::import_project(&dir.path().join("old.kicad_pro")).unwrap_err();
    assert!(e.contains("KiCad 5"), "{e}");

    // The fixture's resistor with a hidden power input pin.
    let src = fixture().parent().unwrap().to_path_buf();
    for f in ["hier.kicad_pro", "hier.kicad_pcb", "amp.kicad_sch"] {
        std::fs::copy(src.join(f), dir.path().join(f)).unwrap();
    }
    let sch = std::fs::read_to_string(src.join("hier.kicad_sch")).unwrap();
    let sch = sch.replacen("(pin passive line", "(pin power_in line (hide yes)", 1);
    std::fs::write(dir.path().join("hier.kicad_sch"), sch).unwrap();
    let e = kicad::import_project(&dir.path().join("hier.kicad_pro")).unwrap_err();
    assert!(e.contains("Device:R") && e.contains("hidden power pin"), "{e}");
}
