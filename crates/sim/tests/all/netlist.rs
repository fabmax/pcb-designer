//! Netlists of small circuits (no simulator needed).

use model::*;
use sim::netlist::{self, Level};

use crate::bench::Bench;

/// A transient: its netlist has no sense sources (an operating point's measures every pin).
fn op() -> Analysis {
    Analysis::new("TRAN", AnalysisKind::Tran { stop: 1e-3, step: None, start: 0.0, max_step: None })
}

fn lines(text: &str) -> Vec<&str> {
    text.lines().collect()
}

fn has_line(n: &sim::Netlist, line: &str) {
    assert!(lines(&n.text).contains(&line), "no line {line:?} in\n{}", n.text);
}

fn errors(n: &sim::Netlist) -> Vec<String> {
    n.problems.iter().filter(|p| p.level == Level::Error).map(|p| p.message.clone()).collect()
}

#[test]
fn rc_lowpass() {
    let mut b = Bench::new();
    let sine = Waveform::Sine { offset: 0.0, amplitude: 1.0, frequency: 1e3, delay: 0.0, damping: 0.0, phase: 0.0 };
    let v1 = b.source("VSource", sine, 1.0, &["IN", "GND"]);
    b.place("Resistor", "10k", &["IN", "OUT"]);
    b.place("Capacitor", "100n", &["OUT", "GND"]);
    let n = netlist::build(
        b.project(),
        &b.conn(),
        &Analysis::new("AC", AnalysisKind::Ac { start: 1.0, stop: 1e6, points_per_decade: 20 }),
    );
    assert_eq!(errors(&n), Vec::<String>::new());
    has_line(&n, "V1 in 0 DC 0 AC 1 0 SIN(0 1 1000 0 0 0)");
    has_line(&n, "R1 in out 10000");
    has_line(&n, "C1 out 0 1e-7");
    has_line(&n, ".ac dec 20 1 1e6");
    has_line(&n, ".options rshunt=1e12");
    assert!(n.text.ends_with(".end\n"));
    assert_eq!(n.node("OUT"), Some("out"));
    assert_eq!(n.node("GND"), Some("0"));
    assert_eq!(n.devices.get(&v1).map(String::as_str), Some("V1"));
    // The source's value shows its waveform.
    assert_eq!(b.component(v1).value, "SIN 1 V 1 kHz AC 1 V");
}

#[test]
fn values_are_plain_numbers() {
    let mut b = Bench::new();
    b.place("Resistor", "4k7", &["A", "GND"]);
    b.place("Resistor", "1M", &["A", "GND"]);
    b.place("Resistor", "2R2", &["A", "GND"]);
    b.place("Inductor", "10uH", &["A", "GND"]);
    let n = netlist::build(b.project(), &b.conn(), &op());
    has_line(&n, "R1 a 0 4700");
    has_line(&n, "R2 a 0 1e6");
    has_line(&n, "R3 a 0 2.2");
    has_line(&n, "L1 a 0 1e-5");
}

#[test]
fn a_value_that_is_no_number_is_an_error_on_its_symbol() {
    let mut b = Bench::new();
    let (_, s) = b.place("Resistor", "ten", &["A", "GND"]);
    let n = netlist::build(b.project(), &b.conn(), &op());
    let e: Vec<_> = n.problems.iter().filter(|p| p.level == Level::Error).collect();
    assert_eq!(e.len(), 1, "{:?}", n.problems);
    assert!(e[0].message.starts_with("R1: the value \"ten\" is not a number"), "{}", e[0].message);
    assert_eq!(e[0].objects, vec![ObjectId::Symbol(s)]);
}

#[test]
fn no_ground_is_an_error() {
    let mut b = Bench::new();
    b.place("Resistor", "1k", &["A", "B"]);
    let n = netlist::build(b.project(), &b.conn(), &op());
    assert_eq!(errors(&n), vec!["No ground: place a GND power port (or choose the ground net)".to_string()]);

    // A chosen ground net.
    let mut sim = b.project().schematic.simulation.clone();
    sim.ground = Some("B".into());
    b.ed.execute(editor_core::Command::SetSimulation { simulation: sim }).unwrap();
    let n = netlist::build(b.project(), &b.conn(), &op());
    assert_eq!(errors(&n), Vec::<String>::new());
    has_line(&n, "R1 a 0 1000");
}

#[test]
fn semiconductors_follow_their_models_pin_order() {
    let mut b = Bench::new();
    // Symbol orders: diode A K; BJT B C E; MOSFET G D S.
    b.place("Diode", "", &["A", "K"]);
    b.place("NPN", "", &["B", "C", "E"]);
    b.place("NMOS", "", &["G", "D", "S"]);
    b.place("PMOS", "", &["G", "D", "S"]);
    b.place("Resistor", "1k", &["S", "GND"]);
    let n = netlist::build(b.project(), &b.conn(), &op());
    assert_eq!(errors(&n), Vec::<String>::new());
    has_line(&n, "D1 a k D1N4148");
    has_line(&n, "Q1 c b e Q2N3904");
    // VDMOS: three nodes; the reference Q2 gets the letter M.
    has_line(&n, "MQ2 d g s M2N7002");
    assert!(n.text.contains(".model D1N4148 D(IS=2.52n"), "{}", n.text);
    assert!(n.text.contains(".model MBSS84 VDMOS(pchan Vto=-1.6"), "{}", n.text);
}

#[test]
fn zener_breakdown_comes_from_the_value() {
    let mut b = Bench::new();
    b.place("Diode_Zener", "5V1", &["A", "GND"]);
    b.place("Diode_Zener", "BZT52C12", &["A", "GND"]);
    b.place("Diode_Zener", "BZT52C3V3", &["A", "GND"]);
    let n = netlist::build(b.project(), &b.conn(), &op());
    assert!(n.text.contains("D1 a 0 DZENER_D1"), "{}", n.text);
    assert!(n.text.contains(".model DZENER_D1 D(IS=1n RS=2 N=1.2 CJO=100p BV=5.1 IBV=5m)"), "{}", n.text);
    assert!(n.text.contains(".model DZENER_D3 D(IS=1n RS=2 N=1.2 CJO=100p BV=3.3 IBV=5m)"), "{}", n.text);
    // No number in "BZT52C12": an error on D2.
    assert_eq!(errors(&n).len(), 1, "{:?}", n.problems);
    assert!(errors(&n)[0].starts_with("D2:"));
}

#[test]
fn component_overrides_get_their_own_model() {
    let mut b = Bench::new();
    let (q1, _) = b.place("NPN", "", &["B", "C", "GND"]);
    b.place("NPN", "", &["B", "C2", "GND"]);
    b.update_component(q1, |c| {
        c.sim.params.insert("BF".into(), "200".into());
    });
    let n = netlist::build(b.project(), &b.conn(), &op());
    has_line(&n, "Q1 c b 0 Q2N3904_Q1");
    has_line(&n, "Q2 c2 b 0 Q2N3904");
    assert!(n.text.contains(" BF=200 "), "{}", n.text);
    assert_eq!(n.text.matches(".model Q2N3904 ").count(), 1);
}

#[test]
fn parts_without_model_or_excluded_are_left_out() {
    let mut b = Bench::new();
    let (r1, _) = b.place("Resistor", "1k", &["A", "GND"]);
    let (r2, _) = b.place("Resistor", "1k", &["A", "GND"]);
    b.place("Conn_01x02", "", &["A", "GND"]);
    b.update_component(r1, |c| c.sim.exclude = true);
    // A part without a model.
    let part = b.project().library.parts.get(b.component(r2).part).unwrap().clone();
    let mut copy = part.clone();
    copy.sim = None;
    b.ed.execute(editor_core::Command::Update { objects: vec![Object::LibPart(copy)] }).unwrap();
    let n = netlist::build(b.project(), &b.conn(), &op());
    assert!(!n.text.contains("R1 "), "{}", n.text);
    assert!(!n.text.contains("J1"), "{}", n.text);
    let warnings: Vec<_> = n.problems.iter().filter(|p| p.level == Level::Warning).map(|p| &p.message).collect();
    assert_eq!(warnings, vec!["Not simulated, no model: R2"]);
}

#[test]
fn open_pins_get_nodes_of_their_own() {
    let mut b = Bench::new();
    b.place("Resistor", "1k", &["", "GND"]);
    b.place("Resistor", "1k", &["", "GND"]);
    let n = netlist::build(b.project(), &b.conn(), &op());
    // A lone pin is a net of its own.
    has_line(&n, "R1 net_r1_1 0 1000");
    has_line(&n, "R2 net_r2_1 0 1000");
}

#[test]
fn multi_unit_parts_join_their_units() {
    let mut b = Bench::new();
    let (u1, _) = b.place("LM358", "", &["INP", "OUT", "OUT"]);
    b.place_unit(u1, 2, &["VCC", "GND"]);
    b.dc(12.0, "VCC", "GND");
    let n = netlist::build(b.project(), &b.conn(), &op());
    assert_eq!(errors(&n), Vec::<String>::new());
    // Unit B (pins 5, 6, 7) isn't placed: its pins are open.
    has_line(&n, "XU1 inp out out nc_u1_4 nc_u1_5 nc_u1_6 vcc 0 LM358");
    assert!(n.text.contains(".subckt LM358 inp_a"), "{}", n.text);
    assert!(n.text.contains(".subckt LM358_AMP inp inn out vp vn"), "{}", n.text);
}

#[test]
fn current_probes_add_sense_sources() {
    let mut b = Bench::new();
    b.dc(5.0, "A", "GND");
    let (_, r1) = b.place("Resistor", "1k", &["A", "GND"]);
    let pin = b.pin(r1, "1");
    let mut a = op();
    a.probes.push(Probe::Current { symbol: r1, pin });
    let n = netlist::build(b.project(), &b.conn(), &a);
    has_line(&n, "VSENSE_R1_1 a vsense_r1_1_in 0");
    has_line(&n, "R1 vsense_r1_1_in 0 1000");
    assert_eq!(n.sense.get(&(r1, pin)).map(String::as_str), Some("vsense_r1_1"));
}

#[test]
fn operating_points_measure_every_pin() {
    let mut b = Bench::new();
    let v1 = b.dc(5.0, "A", "GND");
    let (r1c, r1) = b.place("Resistor", "1k", &["A", "B"]);
    b.place("Resistor", "1k", &["B", "GND"]);
    let n = netlist::build(b.project(), &b.conn(), &Analysis::new("OP", AnalysisKind::Op));
    has_line(&n, "VSENSE_R1_1 a vsense_r1_1_in 0");
    has_line(&n, "VSENSE_R1_2 b vsense_r1_2_in 0");
    has_line(&n, "R1 vsense_r1_1_in vsense_r1_2_in 1000");
    assert_eq!(n.sense.len(), 6);
    let r1_pins: Vec<(&str, &str)> =
        n.pins.iter().filter(|p| p.component == r1c).map(|p| (p.key.as_str(), p.node.as_str())).collect();
    assert_eq!(r1_pins, [("1", "a"), ("2", "b")]);
    assert_eq!(n.pins.first().map(|p| p.component), Some(r1c), "devices in reference order");
    assert_eq!(n.pins.last().map(|p| p.component), Some(v1));
    assert_eq!(n.current_label(r1, b.pin(r1, "2")).as_deref(), Some("I(R1.2)"));
}

#[test]
fn supplies_power_nets() {
    let mut b = Bench::new();
    b.place("Resistor", "1k", &["+5V", "GND"]);
    let mut sim = b.project().schematic.simulation.clone();
    sim.supplies.push(Supply { net: "+5V".into(), voltage: 5.0 });
    sim.supplies.push(Supply { net: "+12V".into(), voltage: 12.0 });
    b.ed.execute(editor_core::Command::SetSimulation { simulation: sim }).unwrap();
    let n = netlist::build(b.project(), &b.conn(), &op());
    has_line(&n, "VSUPPLY_P5V p5v 0 DC 5");
    let warnings: Vec<_> = n.problems.iter().filter(|p| p.level == Level::Warning).map(|p| &p.message).collect();
    assert_eq!(warnings, vec!["Supply +12V: no such net"]);
}

/// Power ports whose name says the voltage get a supply by themselves, unless a source,
/// a supply of the setup or a regulator's output powers them, or the setup leaves them
/// out.
#[test]
fn power_nets_get_automatic_supplies() {
    let mut b = Bench::new();
    b.place("Resistor", "1k", &["+5V", "GND"]);
    b.place("Resistor", "1k", &["+12V", "GND"]);
    b.place("Resistor", "1k", &["+3V3", "GND"]);
    b.place("Resistor", "1k", &["+VCC", "GND"]);
    b.place("Resistor", "1k", &["+9V", "GND"]);
    b.dc(9.0, "+9V", "GND");
    b.place("AMS1117-3.3", "", &["+5V", "+3V3", "GND"]);
    let supplied = |b: &Bench| -> Vec<(String, f64)> {
        netlist::automatic_supplies(b.project(), &b.conn()).into_iter().map(|s| (s.net, s.voltage)).collect()
    };
    assert_eq!(supplied(&b), [("+5V".to_string(), 5.0), ("+12V".to_string(), 12.0)]);
    let n = netlist::build(b.project(), &b.conn(), &op());
    has_line(&n, "VSUPPLY_P5V p5v 0 DC 5");
    has_line(&n, "VSUPPLY_P12V p12v 0 DC 12");
    // What's left: +3V3 (the regulator's output) and +VCC (no voltage in the name).
    let left: Vec<(String, Option<f64>, bool)> =
        netlist::unsupplied_nets(b.project(), &b.conn()).into_iter().map(|n| (n.net, n.voltage, n.power_out)).collect();
    assert_eq!(left, [("+3V3".to_string(), Some(3.3), true), ("+VCC".to_string(), None, false)]);

    // A supply of the setup sets another voltage; +12V left out.
    let mut sim = b.project().schematic.simulation.clone();
    sim.supplies.push(Supply { net: "+5V".into(), voltage: 4.5 });
    sim.unsupplied.push("+12V".into());
    b.ed.execute(editor_core::Command::SetSimulation { simulation: sim }).unwrap();
    assert_eq!(supplied(&b), []);
    let n = netlist::build(b.project(), &b.conn(), &op());
    has_line(&n, "VSUPPLY_P5V p5v 0 DC 4.5");
    assert!(!n.text.contains("VSUPPLY_P12V"), "{}", n.text);
    assert!(netlist::unsupplied_nets(b.project(), &b.conn()).iter().any(|n| n.net == "+12V"));
}

#[test]
fn supply_voltages_from_names() {
    assert_eq!(netlist::supply_voltage("+5V"), Some(5.0));
    assert_eq!(netlist::supply_voltage("3V3"), Some(3.3));
    assert_eq!(netlist::supply_voltage("-12V"), Some(-12.0));
    assert_eq!(netlist::supply_voltage("VCC"), None);
}

#[test]
fn analysis_lines() {
    let mut b = Bench::new();
    let v1 = b.dc(1.0, "A", "GND");
    let (r1, _) = b.place("Resistor", "1k", &["A", "GND"]);
    let line = |b: &Bench, kind| {
        let n = netlist::build(b.project(), &b.conn(), &Analysis::new("x", kind));
        n.text.lines().rev().nth(1).unwrap().to_string()
    };
    assert_eq!(line(&b, AnalysisKind::Op), ".op");
    assert_eq!(line(&b, AnalysisKind::Dc { source: v1, start: 0.0, stop: 5.0, step: 0.1 }), ".dc V1 0 5 0.1");
    assert_eq!(line(&b, AnalysisKind::Tran { stop: 1e-3, step: None, start: 0.0, max_step: None }), ".tran 1e-6 0.001");
    assert_eq!(
        line(&b, AnalysisKind::Tran { stop: 1e-3, step: Some(1e-5), start: 0.0, max_step: Some(1e-6) }),
        ".tran 1e-5 0.001 0 1e-6"
    );
    // Sweeping something that isn't a source.
    let n = netlist::build(
        b.project(),
        &b.conn(),
        &Analysis::new("x", AnalysisKind::Dc { source: r1, start: 0.0, stop: 1.0, step: 0.1 }),
    );
    assert_eq!(errors(&n), vec!["DC sweep: the swept component is not a simulated source".to_string()]);
}

#[test]
fn several_grounds_join() {
    let mut b = Bench::new();
    b.place("Resistor", "1k", &["A", "GND"]);
    b.place("Resistor", "1k", &["A", "AGND"]);
    // AGND as a power port too.
    let label = b.project().schematic.labels.iter().find(|l| l.name == "AGND").unwrap().clone();
    let label = NetLabel { kind: LabelKind::Power, ..label };
    b.ed.execute(editor_core::Command::Update { objects: vec![Object::Label(label)] }).unwrap();
    let n = netlist::build(b.project(), &b.conn(), &op());
    has_line(&n, "R2 a 0 1000");
    assert!(
        n.problems.iter().any(|p| p.level == Level::Note && p.message == "Ground nets joined as node 0: AGND, GND")
    );
}

#[test]
fn sources_describe_their_waveforms() {
    let mut b = Bench::new();
    let pulse =
        Waveform::Pulse { low: 0.0, high: 3.3, delay: 0.0, rise: 1e-9, fall: 1e-9, width: 0.5e-3, period: 1e-3 };
    let v = b.source("VSource", pulse, 0.0, &["A", "GND"]);
    assert_eq!(b.component(v).value, "PULSE 0–3.3 V 1 kHz");
    let i = b.source("ISource", Waveform::Dc { value: 1e-3 }, 0.0, &["A", "GND"]);
    assert_eq!(b.component(i).value, "DC 1 mA");
    let n = netlist::build(b.project(), &b.conn(), &op());
    has_line(&n, "V1 a 0 DC 0 PULSE(0 3.3 0 1e-9 1e-9 5e-4 0.001)");
    has_line(&n, "I1 a 0 DC 0.001");
    assert_eq!(netlist::suggested_stop(b.project()), 5e-3);
}
