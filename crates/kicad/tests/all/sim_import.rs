//! KiCad simulation import.
//!
//! * A hand-written project with the field forms KiCad uses (`Sim.*`, KiCad 5's
//!   `Spice_*`, inferred R/C/L/V, a library model, directives).
//! * KiCad's simulation demo projects, compared against KiCad itself: `kicad-cli`'s
//!   SPICE netlist and ours of the imported project run through ngspice, and every pin
//!   of every device must see the same operating point voltage (and the shared library,
//!   when found, the same as the process on ours). Skipped unless
//!   `KICAD_CLI` is set and ngspice is found. The demos are `KICAD_SIM_DEMOS` (KiCad's
//!   `demos/simulation` folder), else fetched from KiCad's GitLab (a pinned commit,
//!   sparse) into `target/kicad-sim-demos`.
//!
//! ```sh
//! KICAD_CLI=/Applications/KiCad/KiCad.app/Contents/MacOS/kicad-cli \
//!     cargo test -p kicad --test all sim_import:: -- --nocapture
//! ```

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use editor_core::connectivity::Connectivity;
use model::*;
use sim::Backend;

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/sim/sim.kicad_pro")
}

#[test]
fn simulation_fields_of_a_project() {
    let imported = kicad::import_project(&fixture()).unwrap();
    let p = &imported.project;
    let report = imported.report.lines().join("\n");
    let c = |r: &str| p.schematic.component_by_refdes(r).unwrap_or_else(|| panic!("{r} missing"));
    let model = |r: &str| p.library.parts.get(c(r).part).unwrap().sim.clone();
    let pin_numbers = |r: &str| -> Vec<String> {
        let part = p.library.parts.get(c(r).part).unwrap();
        part.sim
            .as_ref()
            .unwrap()
            .pins
            .iter()
            .map(|sp| {
                let sym = p.library.symbols.get(part.units[sp.unit as usize].symbol).unwrap();
                sym.pins.get(sp.pin).unwrap().number.clone()
            })
            .collect()
    };

    // Inferred from the reference and value.
    assert_eq!(model("R1").unwrap().device, SimDevice::Resistor);
    assert_eq!(c("R1").value, "1k");
    // Sim.Device R with Sim.Pins swapped.
    assert_eq!(model("R2").unwrap().device, SimDevice::Resistor);
    assert_eq!(pin_numbers("R2"), ["2", "1"]);
    // A sine source from Sim.Type / Sim.Params; its value shows the waveform.
    let v1 = c("V1");
    assert_eq!(model("V1").unwrap().device, SimDevice::VoltageSource);
    let s = v1.sim.source.as_ref().unwrap();
    assert_eq!(
        s.waveform,
        Waveform::Sine { offset: 0.0, amplitude: 1.0, frequency: 1e3, delay: 0.0, damping: 0.0, phase: 0.0 }
    );
    assert_eq!(s.ac, 1.0);
    assert_eq!(v1.value, s.describe("V"));
    // KiCad 5's fields: a DC source.
    assert_eq!(c("V2").sim.source.as_ref().unwrap().waveform, Waveform::Dc { value: 5.0 });
    // A diode from a library, pins by name (1=K 2=A: anode first is pin 2).
    let d = model("D1").unwrap();
    let SimDevice::Primitive { kind, model: card } = &d.device else { panic!("{d:?}") };
    assert_eq!(*kind, PrimitiveKind::Diode);
    assert!(card.contains("1N4148"), "{card}");
    assert_eq!(pin_numbers("D1"), ["2", "1"]);
    // Two diodes of one symbol with different models: two parts.
    assert_ne!(c("D1").part, c("D2").part);
    assert_ne!(p.library.parts.get(c("D1").part).unwrap().name, p.library.parts.get(c("D2").part).unwrap().name);
    // A subcircuit from the library, with the model it uses.
    let u1 = model("U1").unwrap();
    let SimDevice::Subckt { text } = &u1.device else { panic!("{u1:?}") };
    assert!(text.starts_with(".subckt BUF"), "{text}");
    assert!(text.contains(".model QBUF NPN"), "{text}");
    // XSPICE code models: an A line in a subcircuit with the card; a gate's inputs as a
    // vector; Sim.Params override the card.
    let u2 = model("U2").unwrap_or_else(|| panic!("{report}"));
    let SimDevice::Subckt { text } = &u2.device else { panic!("{u2:?}") };
    assert!(text.contains("A1 p1 p2 INV1"), "{text}");
    assert!(text.contains(".model INV1 d_inverter(rise_delay=1n fall_delay=1n)"), "{text}");
    let u3 = model("U3").unwrap();
    let SimDevice::Subckt { text } = &u3.device else { panic!("{u3:?}") };
    assert!(text.contains("A1 [p1 p2] p3 NAND1"), "{text}");
    assert!(text.contains("rise_delay=2e-9") && text.contains("fall_delay=1n"), "{text}");
    assert_eq!(pin_numbers("U3"), ["1", "2", "3"]);
    // Excluded from the simulation, and not simulated (no model): no warning for those.
    assert!(c("R3").sim.exclude);
    assert!(model("J1").is_none());
    // Sim fields aren't kept as properties.
    assert!(c("V1").properties.keys().all(|k| !k.starts_with("Sim.")));
    // Directives: analyses, extra lines, temperature.
    let sim = &p.schematic.simulation;
    let kinds: Vec<&str> = sim.analyses.iter().map(|a| a.kind.label()).collect();
    assert_eq!(kinds, ["Transient", "AC"]);
    assert!(sim.extra.contains(".param rload=10k"), "{}", sim.extra);
    assert_eq!(sim.temperature, 50.0);
    assert!(!report.contains("not simulated"), "{report}");
}

// ---------------------------------------------------------------------------------------
// Against KiCad

/// The KiCad commit whose demos are compared.
const DEMOS_COMMIT: &str = "2692baeeaf0d01516162cd165166d55db955decb";
const KICAD_GIT: &str = "https://gitlab.com/kicad/code/kicad.git";

/// Demos left out, and why.
const SKIPPED: [(&str, &str); 3] = [
    ("ibis", "IBIS drivers aren't supported"),
    // Its d_state model reads a state file by a relative path: KiCad's own netlist
    // fails in ngspice too (and takes the shared library down with it).
    ("up-down-c", "its XSPICE state file isn't found, by KiCad's netlist either"),
    ("Q17ng", "kicad-cli can't netlist it"),
];

fn demos() -> Option<PathBuf> {
    if let Ok(d) = std::env::var("KICAD_SIM_DEMOS") {
        return Some(PathBuf::from(d));
    }
    let target = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/kicad-sim-demos");
    let dir = target.join("demos/simulation");
    if dir.is_dir() {
        return Some(dir);
    }
    eprintln!("fetching KiCad's simulation demos into {}", target.display());
    let _ = std::fs::remove_dir_all(&target);
    std::fs::create_dir_all(&target).ok()?;
    let git = |args: &[&str]| Command::new("git").arg("-C").arg(&target).args(args).status().is_ok_and(|s| s.success());
    let ok = git(&["init", "-q"])
        && git(&["sparse-checkout", "set", "--no-cone", "/demos/simulation/"])
        && git(&["fetch", "-q", "--depth", "1", "--filter=blob:none", KICAD_GIT, DEMOS_COMMIT])
        && git(&["checkout", "-q", "FETCH_HEAD"]);
    if !ok || !dir.is_dir() {
        eprintln!("fetching the demos failed");
        return None;
    }
    Some(dir)
}

/// Every `.kicad_pro` with its schematic under `dir`.
fn projects(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    let mut entries: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
    entries.sort();
    for p in entries {
        if p.is_dir() {
            projects(&p, out);
        } else if p.extension().is_some_and(|e| e == "kicad_pro") && p.with_extension("kicad_sch").is_file() {
            out.push(p);
        }
    }
}

/// SPICE text with continuation lines joined.
fn joined(text: &str) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    for l in text.lines() {
        if let Some(rest) = l.trim_start().strip_prefix('+')
            && let Some(last) = lines.last_mut()
        {
            last.push(' ');
            last.push_str(rest);
        } else {
            lines.push(l.to_string());
        }
    }
    lines
}

/// KiCad's netlist as an operating point (its analyses and control scripts removed) and
/// its device lines by upper-case name.
fn kicad_op(netlist: &str) -> (String, HashMap<String, Vec<String>>) {
    let mut out = Vec::new();
    let mut devices = HashMap::new();
    let mut control = false;
    for l in joined(netlist) {
        let lower = l.trim().to_ascii_lowercase();
        let word = lower.split_whitespace().next().unwrap_or("");
        if control {
            control = word != ".endc";
            continue;
        }
        match word {
            ".control" => control = true,
            ".tran" | ".ac" | ".dc" | ".op" | ".noise" | ".end" | ".probe" | ".save" | ".meas" | ".tf" => {}
            _ => {
                if !word.starts_with('.') && !word.starts_with('*') && !word.is_empty() {
                    let tokens: Vec<String> = l.split_whitespace().map(str::to_string).collect();
                    devices.insert(tokens[0].to_ascii_uppercase(), tokens[1..].to_vec());
                }
                out.push(l);
            }
        }
    }
    out.push(".options rshunt=1e12".into());
    out.push(".op".into());
    out.push(".end".into());
    (out.join("\n") + "\n", devices)
}

fn op_voltage(plot: &sim::Plot, node: &str) -> Option<f64> {
    if node == "0" || node.eq_ignore_ascii_case("gnd") {
        return Some(0.0);
    }
    plot.vector(&format!("v({})", node.to_ascii_lowercase())).and_then(|v| v.re.first().copied())
}

#[test]
fn kicad_simulation_demos_match_kicad() {
    let Some(cli) = std::env::var("KICAD_CLI").ok().map(PathBuf::from) else {
        eprintln!("KICAD_CLI not set; skipped");
        return;
    };
    let library = match sim::Libngspice::find(None) {
        Ok(l) => l,
        Err(e) => {
            sim::skip_without_ngspice(e);
            return;
        }
    };
    let Some(dir) = demos() else { return };
    let mut list = Vec::new();
    projects(&dir, &mut list);
    assert!(list.len() >= 10, "only {} demo projects in {}", list.len(), dir.display());
    let tmp = tempfile::tempdir().unwrap();
    let cancel = sim::Cancel::default();
    let mut failures = Vec::new();
    let mut compared = 0;
    for pro in &list {
        let name = pro.file_stem().unwrap().to_string_lossy().to_string();
        if let Some((_, why)) = SKIPPED.iter().find(|(n, _)| *n == name) {
            eprintln!("{name}: skipped ({why})");
            continue;
        }
        let sch = pro.with_extension("kicad_sch");
        let cir = tmp.path().join(format!("{name}.cir"));
        let status = Command::new(&cli)
            .args(["sch", "export", "netlist", "--format", "spice", "-o"])
            .arg(&cir)
            .arg(&sch)
            .output()
            .unwrap();
        if !status.status.success() {
            failures.push(format!("{name}: kicad-cli failed: {}", String::from_utf8_lossy(&status.stderr)));
            continue;
        }
        let (theirs, devices) = kicad_op(&std::fs::read_to_string(&cir).unwrap());

        let imported = match kicad::import_project(pro) {
            Ok(i) => i,
            Err(e) => {
                failures.push(format!("{name}: import failed: {e}"));
                continue;
            }
        };
        let p = &imported.project;
        let report = imported.report.lines();
        for w in report.iter().filter(|w| w.contains("not simulated") || w.contains("Simulation")) {
            eprintln!("{name}: {w}");
        }
        let conn = Connectivity::build(p);
        let ours = sim::netlist::build(p, &conn, &Analysis::new("op", AnalysisKind::Op));
        if ours.has_errors() {
            failures.push(format!("{name}: our netlist: {:?}", ours.problems));
            continue;
        }
        let live = sim::Live::default();
        let run = |text: &str, who: &str| match library.run(text, &cancel, &live) {
            Ok(o) => o.plots.into_iter().next(),
            Err(e) => {
                eprintln!("{name}: {who} netlist failed in ngspice: {e}\n{text}");
                None
            }
        };
        let (Some(their_plot), Some(our_plot)) = (run(&theirs, "KiCad's"), run(&ours.text, "our")) else {
            failures.push(format!("{name}: ngspice failed"));
            continue;
        };
        let mut checked = 0;
        for c in p.schematic.components.iter() {
            let Some(device) = ours.devices.get(&c.id) else { continue };
            let refdes = c.refdes.to_ascii_uppercase();
            // KiCad names devices like we do; ours may be a subcircuit (X) where KiCad's is
            // a primitive.
            let line = devices.get(&device.to_ascii_uppercase()).or_else(|| {
                devices.iter().find(|(n, _)| n.len() == refdes.len() + 1 && n.ends_with(&refdes)).map(|(_, l)| l)
            });
            let Some(line) = line.or_else(|| devices.get(&refdes)) else {
                failures.push(format!("{name}: {} has no device line in KiCad's netlist", c.refdes));
                continue;
            };
            for (k, pin) in ours.pins.iter().filter(|x| x.component == c.id).enumerate() {
                let Some(node) = line.get(k) else { continue };
                let (a, b) = (op_voltage(&our_plot, &pin.node), op_voltage(&their_plot, node));
                match (a, b) {
                    (Some(a), Some(b)) if (a - b).abs() <= 1e-3 * a.abs().max(1.0) => checked += 1,
                    _ => failures
                        .push(format!("{name}: {}.{}: ours {a:?} V, KiCad's {b:?} V ({node})", c.refdes, pin.key)),
                }
            }
        }
        eprintln!("{name}: {checked} pins agree");
        compared += 1;
    }
    assert!(failures.is_empty(), "{} differences:\n{}", failures.len(), failures.join("\n"));
    assert!(compared >= 8, "only {compared} demos compared");
}
