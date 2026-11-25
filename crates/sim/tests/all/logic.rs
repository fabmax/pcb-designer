//! The 74HC logic parts' XSPICE models: truth tables, Schmitt thresholds, flip-flops and
//! the shift register.

use model::*;
use sim::SimResult;

use super::bench::Bench;
use super::ngspice::ok;

const NS: f64 = 1e-9;

/// A gate's function of its inputs.
type Logic = fn(bool, bool) -> bool;

fn tran(b: &mut Bench, stop: f64) -> Analysis {
    b.analysis(AnalysisKind::Tran { stop, step: Some(NS), start: 0.0, max_step: Some(NS) })
}

/// `net`'s voltage at time `t`.
fn at(r: &SimResult, net: &str, t: f64) -> f64 {
    r.voltage(net).unwrap_or_else(|| panic!("no {net}")).at(r.scale().unwrap(), t).unwrap()
}

/// A square wave from low to `high` with rising edges at `rises` (seconds), each high
/// for `width`.
fn pulses(high: f64, rises: &[f64], width: f64) -> Waveform {
    let mut points = vec![(0.0, 0.0)];
    for &t in rises {
        points.extend([(t, 0.0), (t + NS, high), (t + width, high), (t + width + NS, 0.0)]);
    }
    Waveform::Pwl { points, repeat: false }
}

/// Every gate part with its function: each gate gets one input combination (the
/// inverters two), all at once in one operating point; at 5 V and at 3.3 V (the outputs
/// follow the supply).
#[test]
fn gates_follow_their_truth_tables() {
    let parts: [(&str, Logic); 7] = [
        ("74HC00", |a, b| !(a && b)),
        ("74HC02", |a, b| !(a || b)),
        ("74HC08", |a, b| a && b),
        ("74HC32", |a, b| a || b),
        ("74HC86", |a, b| a != b),
        ("74HC04", |a, _| !a),
        ("74HC14", |a, _| !a),
    ];
    for vcc in [5.0, 3.3] {
        let mut b = Bench::new();
        b.dc(vcc, "VCC", "GND");
        let mut expect = Vec::new();
        for (part, f) in parts {
            let inverter = part == "74HC04" || part == "74HC14";
            let combos: &[(bool, bool)] = if inverter {
                &[(false, false), (true, false)]
            } else {
                &[(false, false), (false, true), (true, false), (true, true)]
            };
            let level = |x: bool| if x { "VCC" } else { "GND" };
            let mut component = None;
            for (i, &(x, y)) in combos.iter().enumerate() {
                let out = format!("{part}_{i}");
                let nets: Vec<&str> = if inverter { vec![level(x), &out] } else { vec![level(x), level(y), &out] };
                match component {
                    None => component = Some(b.place(part, "", &nets).0),
                    Some(c) => {
                        b.place_unit(c, i as u32, &nets);
                    }
                }
                expect.push((out, f(x, y)));
            }
            let power = if inverter { 6 } else { 4 };
            b.place_unit(component.unwrap(), power, &["VCC", "GND"]);
        }
        let a = b.analysis(AnalysisKind::Op);
        let Some(r) = ok(&b, &a) else { return };
        for (net, high) in expect {
            let v = r.op_voltage(&net).unwrap();
            let want = if high { vcc } else { 0.0 };
            assert!((v - want).abs() < 0.05, "{net} at {vcc} V: {v}, want {want}");
        }
    }
}

/// The 74HC14 switches up at about 0.6 VCC and down at about 0.4 VCC.
#[test]
fn schmitt_inputs_have_hysteresis() {
    let mut b = Bench::new();
    b.dc(5.0, "VCC", "GND");
    let ramp = Waveform::Pwl { points: vec![(0.0, 0.0), (500.0 * NS, 5.0), (1000.0 * NS, 0.0)], repeat: false };
    b.source("VSource", ramp, 0.0, &["IN", "GND"]);
    let (u1, _) = b.place("74HC14", "", &["IN", "OUT"]);
    b.place_unit(u1, 6, &["VCC", "GND"]);
    let a = tran(&mut b, 1000.0 * NS);
    let Some(r) = ok(&b, &a) else { return };
    let scale = r.scale().unwrap();
    let (input, output) = (r.voltage("IN").unwrap(), r.voltage("OUT").unwrap());
    // The input when the output crossed half the supply, going down then up.
    let crossing = |from: usize, falling: bool| {
        let i = (from..output.re.len())
            .find(|&i| if falling { output.re[i] < 2.5 } else { output.re[i] > 2.5 })
            .expect("the output switches");
        (i, input.re[i], scale.re[i])
    };
    let (i, up, _) = crossing(0, true);
    let (_, down, _) = crossing(i, false);
    // Plus the gate's delay (about 12 ns of a 10 mV/ns ramp).
    assert!((3.0..3.25).contains(&up), "switched up at {up} V");
    assert!((1.75..2.0).contains(&down), "switched down at {down} V");
}

/// A 74HC74 flip-flop takes D on CLK's rising edge; ~CLR clears it.
#[test]
fn d_flip_flop_clocks_and_clears() {
    let mut b = Bench::new();
    b.dc(5.0, "VCC", "GND");
    // D high from 100 ns to 300 ns; clock edges at 50, 150, 250, 350 ns; ~CLR low from
    // 450 ns.
    b.source("VSource", pulses(5.0, &[100.0 * NS], 200.0 * NS), 0.0, &["D", "GND"]);
    let edges: Vec<f64> = [50.0, 150.0, 250.0, 350.0, 500.0].iter().map(|t| t * NS).collect();
    b.source("VSource", pulses(5.0, &edges, 40.0 * NS), 0.0, &["CLK", "GND"]);
    let clear = Waveform::Pwl { points: vec![(0.0, 5.0), (450.0 * NS, 5.0), (451.0 * NS, 0.0)], repeat: false };
    b.source("VSource", clear, 0.0, &["CLR", "GND"]);
    // CLR D CLK PRE Q ~Q
    let (u1, _) = b.place("74HC74", "", &["CLR", "D", "CLK", "VCC", "Q", "NQ"]);
    b.place_unit(u1, 2, &["VCC", "GND"]);
    let a = tran(&mut b, 600.0 * NS);
    let Some(r) = ok(&b, &a) else { return };
    let q = |t: f64| at(&r, "Q", t * NS);
    assert!(q(120.0) < 0.1, "D was low at the first edge");
    assert!(q(220.0) > 4.9 && at(&r, "NQ", 220.0 * NS) < 0.1, "D high at the second edge");
    assert!(q(320.0) > 4.9, "still high at the third");
    assert!(q(420.0) < 0.1, "D low again at the fourth");
    // Cleared: a clock edge with ~CLR low doesn't set it (D is low anyway), Q stays low.
    assert!(q(590.0) < 0.1);
}

/// The 74HC595 shifts eight bits in, latches them on RCLK, and lets go of its outputs
/// when ~OE goes high.
#[test]
fn shift_register_shifts_latches_and_tristates() {
    let mut b = Bench::new();
    b.dc(5.0, "VCC", "GND");
    let bits = [true, false, true, true, false, false, true, false];
    // SER holds bit k from k·100 ns; SRCLK rises in the middle of each.
    let mut ser = vec![];
    for (k, &bit) in bits.iter().enumerate() {
        let v = if bit { 5.0 } else { 0.0 };
        ser.extend([(k as f64 * 100.0 * NS + NS, v), ((k + 1) as f64 * 100.0 * NS, v)]);
    }
    b.source("VSource", Waveform::Pwl { points: ser, repeat: false }, 0.0, &["SER", "GND"]);
    let edges: Vec<f64> = (0..8).map(|k| (k as f64 * 100.0 + 50.0) * NS).collect();
    b.source("VSource", pulses(5.0, &edges, 30.0 * NS), 0.0, &["SRCLK", "GND"]);
    b.source("VSource", pulses(5.0, &[900.0 * NS], 30.0 * NS), 0.0, &["RCLK", "GND"]);
    let oe = Waveform::Pwl { points: vec![(0.0, 0.0), (1200.0 * NS, 0.0), (1201.0 * NS, 5.0)], repeat: false };
    b.source("VSource", oe, 0.0, &["OE", "GND"]);
    let outs = ["QA", "QB", "QC", "QD", "QE", "QF", "QG", "QH"];
    let mut nets = vec!["SER", "SRCLK", "VCC", "RCLK", "OE"];
    nets.extend(outs);
    nets.extend(["QHS", "VCC", "GND"]);
    b.place("74HC595", "", &nets);
    for out in outs {
        b.place("Resistor", "10k", &[out, "GND"]);
    }
    let a = tran(&mut b, 1400.0 * NS);
    let Some(r) = ok(&b, &a) else { return };
    // Before RCLK the outputs show the storage register's reset state.
    assert!(at(&r, "QA", 850.0 * NS) < 0.1);
    // QA is the last bit shifted in, QH the first.
    for (i, out) in outs.iter().enumerate() {
        let want = bits[7 - i];
        let v = at(&r, out, 1100.0 * NS);
        assert!(if want { v > 4.9 } else { v < 0.1 }, "{out}: {v} V, want {want}");
    }
    // QH' is the shift register's last stage: the first bit.
    assert!(at(&r, "QHS", 1100.0 * NS) > 4.9);
    // ~OE high: the outputs let go, the resistors pull them down.
    for out in outs {
        assert!(at(&r, out, 1390.0 * NS) < 0.1, "{out} still driven");
    }
}
