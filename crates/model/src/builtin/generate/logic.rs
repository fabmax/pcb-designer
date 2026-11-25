//! 74HC logic: gates (a unit per gate plus a power unit), the 74HC74 dual D flip-flop
//! and the 74HC595 shift register, with XSPICE digital simulation models.
//!
//! The hand-made 74HC00 keeps its symbols; [`hc00_model`] gives it the same kind of
//! model (`merge` sets it).
//!
//! Models: each input is divided by VCC and switches at half of it (Schmitt inputs at
//! 0.6 and 0.4 VCC), into an `adc_bridge`; XSPICE gates and flip-flops do the logic, with
//! typical 5 V delays (they don't change with the supply); a `dac_bridge` and a
//! B-source scaled by VCC drive each output through 50 Ω. The pins stay analog nets, so
//! everything else (probes, plots, the worker) works unchanged. Supply current isn't
//! modelled: the outputs' current comes from GND, not through VCC.

use std::collections::BTreeSet;

use super::super::gid;
use super::Builder;
use super::sym::{G, Sym};
use crate::geom::{HAlign, VAlign};
use crate::library::*;
use crate::simulation::*;

#[derive(Clone, Copy, PartialEq)]
enum Gate {
    Nand,
    Nor,
    And,
    Or,
    Xor,
    Inverter,
    SchmittInverter,
}

impl Gate {
    fn inputs(self) -> usize {
        match self {
            Gate::Inverter | Gate::SchmittInverter => 1,
            _ => 2,
        }
    }

    fn inverted(self) -> bool {
        matches!(self, Gate::Nand | Gate::Nor | Gate::Inverter | Gate::SchmittInverter)
    }

    /// The XSPICE model doing the logic.
    fn model(self) -> &'static str {
        match self {
            Gate::Nand => "hc_nand",
            Gate::Nor => "hc_nor",
            Gate::And => "hc_and",
            Gate::Or => "hc_or",
            Gate::Xor => "hc_xor",
            Gate::Inverter | Gate::SchmittInverter => "hc_inv",
        }
    }
}

/// A gate part: name, gate, what it is, each gate's pins (inputs then output, by
/// number), VCC and GND pins, package pin count.
struct Gates {
    name: &'static str,
    gate: Gate,
    what: &'static str,
    pins: &'static [&'static [&'static str]],
    vcc: &'static str,
    gnd: &'static str,
}

const QUAD: &[&[&str]] = &[&["1", "2", "3"], &["4", "5", "6"], &["9", "10", "8"], &["12", "13", "11"]];
const HEX: &[&[&str]] = &[&["1", "2"], &["3", "4"], &["5", "6"], &["9", "8"], &["11", "10"], &["13", "12"]];

const GATES: &[Gates] = &[
    Gates {
        name: "74HC02",
        gate: Gate::Nor,
        what: "Quad 2-input NOR gate",
        pins: &[&["2", "3", "1"], &["5", "6", "4"], &["8", "9", "10"], &["11", "12", "13"]],
        vcc: "14",
        gnd: "7",
    },
    Gates { name: "74HC04", gate: Gate::Inverter, what: "Hex inverter", pins: HEX, vcc: "14", gnd: "7" },
    Gates { name: "74HC08", gate: Gate::And, what: "Quad 2-input AND gate", pins: QUAD, vcc: "14", gnd: "7" },
    Gates {
        name: "74HC14",
        gate: Gate::SchmittInverter,
        what: "Hex Schmitt-trigger inverter",
        pins: HEX,
        vcc: "14",
        gnd: "7",
    },
    Gates { name: "74HC32", gate: Gate::Or, what: "Quad 2-input OR gate", pins: QUAD, vcc: "14", gnd: "7" },
    Gates { name: "74HC86", gate: Gate::Xor, what: "Quad 2-input XOR gate", pins: QUAD, vcc: "14", gnd: "7" },
];

const UNITS: [&str; 7] = ["A", "B", "C", "D", "E", "F", "G"];

pub(super) fn add(b: &mut Builder) {
    for g in GATES {
        let mut units: Vec<Symbol> = g
            .pins
            .iter()
            .enumerate()
            .map(|(i, pins)| gate_symbol(&format!("{} {}", g.name, UNITS[i]), g.gate, pins))
            .collect();
        units.push(power_symbol(&format!("{} {}", g.name, UNITS[g.pins.len()]), g.vcc, g.gnd));
        let model = gates_model(&format!("HC{}", &g.name[4..]), g.gate, g.pins.len());
        let last = UNITS[g.pins.len() - 1];
        let description = format!("{}: gates A to {last}, power {}", g.what, UNITS[g.pins.len()]);
        part(b, g.name, description, units, 14, model);
    }
    let mut units = vec![flip_flop_symbol("74HC74 A", ["1", "2", "3", "4", "5", "6"])];
    units.push(flip_flop_symbol("74HC74 B", ["13", "12", "11", "10", "9", "8"]));
    units.push(power_symbol("74HC74 C", "14", "7"));
    let description = "Dual D flip-flop with preset and clear (active low): flip-flops A and B, power C".into();
    part(b, "74HC74", description, units, 14, flip_flops_model());
    let description = "8-bit shift register with output latches and 3-state outputs (clear and output \
                       enable active low)"
        .into();
    part(b, "74HC595", description, vec![shift_register_symbol()], 16, shift_register_model());
}

/// The 74HC00's model: four NANDs, ports in its symbols' order (1A 1B 1Y … 4Y VCC GND).
pub fn hc00_model() -> SimModel {
    SimModel::new(SimDevice::Subckt { text: gates_model("HC00", Gate::Nand, 4) })
}

/// Adds a part of `units` (pins in model port order) in DIP and SOIC packages of `n`
/// pins, its pads joined to the pins of their number.
fn part(b: &mut Builder, name: &str, description: String, units: Vec<Symbol>, n: usize, model: String) {
    let mut ids = Vec::new();
    for (i, mut s) in units.into_iter().enumerate() {
        s.category = "Logic".into();
        ids.push((UNITS[i], s.id));
        b.symbol(s);
    }
    let soic = if n == 16 { "SOIC-16_3.9x9.9mm_P1.27mm" } else { "SOIC-14_3.9x8.7mm_P1.27mm" };
    let variants = [(format!("DIP-{n}"), format!("DIP-{n}_W7.62mm")), (format!("SOIC-{n}"), soic.to_string())]
        .into_iter()
        .map(|(vname, fp_name)| {
            let fid = *b.footprints.get(&fp_name).unwrap_or_else(|| panic!("no footprint {fp_name}"));
            let fp = b.lib.footprints.get(fid).unwrap();
            let connections = fp
                .pads
                .iter()
                .filter_map(|pad| {
                    ids.iter().enumerate().find_map(|(u, (_, sid))| {
                        let pin = b.lib.symbols.get(*sid)?.pins.iter().find(|p| p.number == pad.name)?;
                        Some(PadConnection { pad: pad.id, unit: u as u32, pin: pin.id })
                    })
                })
                .collect();
            PartVariant {
                id: gid(&format!("part/{name}/variant/{vname}")),
                name: vname,
                footprint: Some(fid),
                connections,
            }
        })
        .collect();
    b.lib.parts.insert(Part {
        id: gid(&format!("part/{name}")),
        name: name.into(),
        category: "Logic".into(),
        description,
        prefix: "U".into(),
        default_value: name.into(),
        units: ids.iter().map(|(u, id)| PartUnit { name: (*u).into(), symbol: *id }).collect(),
        variants,
        properties: Default::default(),
        sim: Some(SimModel::new(SimDevice::Subckt { text: model })),
        source: None,
    });
}

// ---- Symbols (mm, Y up)

const BODY: f64 = 1.5 * G;

/// Points along a circle around (x, y) from `from` to `to` degrees.
fn arc(x: f64, y: f64, r: f64, from: f64, to: f64) -> Vec<(f64, f64)> {
    let n = (((to - from).abs() / 7.5).ceil() as usize).max(2);
    (0..=n)
        .map(|i| {
            let a = (from + (to - from) * i as f64 / n as f64).to_radians();
            (x + r * a.cos(), y + r * a.sin())
        })
        .collect()
}

/// The OR shape's back curve: a circle through (−BODY, ±BODY) bulging 1 mm into the body.
const BACK_R: f64 = 7.758;
const BACK_X: f64 = -BODY - 6.758;

fn back_x(cx: f64, y: f64) -> f64 {
    cx + (BACK_R * BACK_R - y * y).sqrt()
}

fn pin_kind(p: &mut Pin, kind: PinKind) {
    p.kind = kind;
    p.show_name = true;
    p.show_number = true;
}

/// A gate in the distinctive shape: inputs left (A at the top), output Y right.
fn gate_symbol(name: &str, gate: Gate, pins: &[&str]) -> Symbol {
    let mut s = Sym::new(name);
    let back = match gate {
        Gate::Or | Gate::Nor | Gate::Xor => Some(if gate == Gate::Xor { BACK_X - 0.76 } else { BACK_X }),
        _ => None,
    };
    let ys: &[f64] = if gate.inputs() == 1 { &[0.0] } else { &[G, -G] };
    for (i, &y) in ys.iter().enumerate() {
        let end = back.map_or(-BODY, |cx| back_x(cx, y));
        let pin = s.pin(pins[i], ["A", "B"][i], -3.0 * G, y, 0.0, end + 3.0 * G);
        pin_kind(pin, PinKind::Input);
        // The shape says what the pins are (names would cross the XOR's curves).
        pin.show_name = false;
    }
    match gate {
        Gate::And | Gate::Nand => {
            let mut pts = vec![(0.0, BODY), (-BODY, BODY), (-BODY, -BODY), (0.0, -BODY)];
            pts.extend(arc(0.0, 0.0, BODY, -90.0, 90.0));
            s.poly(&pts, true);
        }
        Gate::Or | Gate::Nor | Gate::Xor => {
            // Front arcs from the corners to the tip at (BODY, 0), the back curve inside.
            // Radius r: the circle around (−BODY, BODY − r) through (BODY, 0).
            let r = (4.0 * BODY * BODY + BODY * BODY) / (2.0 * BODY);
            let tip = (r - BODY).atan2(2.0 * BODY).to_degrees();
            let mut pts = arc(-BODY, BODY - r, r, 90.0, tip);
            pts.extend(arc(-BODY, r - BODY, r, -tip, -90.0));
            let a = (BODY / BACK_R).asin().to_degrees();
            pts.extend(arc(BACK_X, 0.0, BACK_R, -a, a));
            s.poly(&pts, true);
            if gate == Gate::Xor {
                s.arc(BACK_X - 0.76, 0.0, BACK_R, -a, 2.0 * a);
            }
        }
        Gate::Inverter | Gate::SchmittInverter => {
            s.poly(&[(-BODY, BODY), (-BODY, -BODY), (BODY, 0.0)], true);
            if gate == Gate::SchmittInverter {
                s.line(&[(-3.0, -0.6), (-1.4, -0.6), (-1.4, 0.6)]);
                s.line(&[(-2.4, -0.6), (-2.4, 0.6), (-0.8, 0.6)]);
            }
        }
    }
    let out_end = if gate.inverted() {
        s.circle(BODY + 0.635, 0.0, 0.635, false);
        BODY + 1.27
    } else {
        BODY
    };
    let pin = s.pin(pins[gate.inputs()], "Y", 3.0 * G, 0.0, 180.0, 3.0 * G - out_end);
    pin_kind(pin, PinKind::Output);
    pin.show_name = false;
    s.ref_value(BODY + 0.5, -BODY - 0.5);
    s.finish()
}

/// The power unit: VCC at the top, GND at the bottom.
fn power_symbol(name: &str, vcc: &str, gnd: &str) -> Symbol {
    let mut s = Sym::new(name);
    s.rect(-G, -BODY, G, BODY, true);
    pin_kind(s.pin(vcc, "VCC", 0.0, 2.0 * G, 270.0, G / 2.0), PinKind::PowerIn);
    pin_kind(s.pin(gnd, "GND", 0.0, -2.0 * G, 90.0, G / 2.0), PinKind::PowerIn);
    s.ref_value_right(BODY, 0.0);
    s.finish()
}

/// A D flip-flop: D and CLK left, ~PRE top, ~CLR bottom, Q and ~Q right (`~`: active
/// low). `pins`: CLR, D, CLK, PRE, Q, ~Q (the model's port order).
fn flip_flop_symbol(name: &str, pins: [&str; 6]) -> Symbol {
    let mut s = Sym::new(name);
    // Taller than wide: ~PRE and ~CLR read down and up into the middle.
    let (w, h) = (2.0 * G, 3.0 * G);
    s.rect(-w, -h, w, h, true);
    pin_kind(s.pin(pins[0], "~CLR", 0.0, -h - G, 90.0, G), PinKind::Input);
    pin_kind(s.pin(pins[1], "D", -3.0 * G, G, 0.0, G), PinKind::Input);
    pin_kind(s.pin(pins[2], "CLK", -3.0 * G, -G, 0.0, G), PinKind::Input);
    pin_kind(s.pin(pins[3], "~PRE", 0.0, h + G, 270.0, G), PinKind::Input);
    pin_kind(s.pin(pins[4], "Q", 3.0 * G, G, 180.0, G), PinKind::Output);
    pin_kind(s.pin(pins[5], "~Q", 3.0 * G, -G, 180.0, G), PinKind::Output);
    s.text("${REF}", -w, h + 0.5, HAlign::Right, VAlign::Bottom);
    s.text("${VALUE}", -w, -h - 0.5, HAlign::Right, VAlign::Top);
    s.finish()
}

/// The 74HC595: inputs left, QA…QH and QH' right, VCC top, GND bottom (`~`: active
/// low).
fn shift_register_symbol() -> Symbol {
    let mut s = Sym::new("74HC595");
    let (w, h) = (3.0 * G, 5.0 * G);
    s.rect(-w, -h, w, h, true);
    for (num, name, y) in
        [("14", "SER", 4.0), ("11", "SRCLK", 2.0), ("10", "~SRCLR", 1.0), ("12", "RCLK", -1.0), ("13", "~OE", -3.0)]
    {
        pin_kind(s.pin(num, name, -w - G, y * G, 0.0, G), PinKind::Input);
    }
    let outs =
        [("15", "QA"), ("1", "QB"), ("2", "QC"), ("3", "QD"), ("4", "QE"), ("5", "QF"), ("6", "QG"), ("7", "QH")];
    for (i, (num, name)) in outs.into_iter().enumerate() {
        pin_kind(s.pin(num, name, w + G, 4.0 * G - i as f64 * G, 180.0, G), PinKind::TriState);
    }
    pin_kind(s.pin("9", "QH'", w + G, -4.0 * G, 180.0, G), PinKind::Output);
    pin_kind(s.pin("16", "VCC", 0.0, h + G, 270.0, G), PinKind::PowerIn);
    pin_kind(s.pin("8", "GND", 0.0, -h - G, 90.0, G), PinKind::PowerIn);
    // Both above: the pin numbers sit left of VCC's pin and right of GND's.
    s.text("${REF}", -w, h + 0.5, HAlign::Left, VAlign::Bottom);
    s.text("${VALUE}", w, h + 0.5, HAlign::Right, VAlign::Bottom);
    s.finish()
}

// ---- Models

/// A subcircuit being written: analog pins in and out of XSPICE digital nodes.
#[derive(Default)]
struct Net {
    lines: Vec<String>,
    models: BTreeSet<&'static str>,
    next: usize,
}

impl Net {
    fn node(&mut self, prefix: &str) -> String {
        self.next += 1;
        format!("{prefix}{}", self.next)
    }

    fn line(&mut self, l: String) {
        self.lines.push(l);
    }

    /// Input pin `port`: switches at VCC/2; its digital node.
    fn input(&mut self, port: &str) -> String {
        self.line(format!("C{port} {port} gnd 3.5p"));
        self.line(format!("B{port} n_{port} 0 V = v({port},gnd)/max(v(vcc,gnd),1)"));
        self.bridge_in(port)
    }

    /// Schmitt-trigger input: up at 0.6 VCC, down at 0.4 VCC (the state fed back).
    fn schmitt(&mut self, port: &str) -> String {
        self.line(format!("C{port} {port} gnd 3.5p"));
        self.line(format!("B{port} n_{port} 0 V = v({port},gnd)/max(v(vcc,gnd),1) - 0.1 + 0.2*v(s_{port})"));
        let d = self.bridge_in(port);
        self.models.insert("hc_fb");
        self.line(format!("As{port} [{d}] [s_{port}] hc_fb"));
        d
    }

    fn bridge_in(&mut self, port: &str) -> String {
        self.models.insert("hc_in");
        self.line(format!("A{port} [n_{port}] [d_{port}] hc_in"));
        format!("d_{port}")
    }

    /// Output pin `port` driven by digital node `d`: VCC or GND through 50 Ω; with
    /// `enable` (an analog 0…1 node) through a switch that's open while it's low.
    fn output(&mut self, d: &str, port: &str, enable: Option<&str>) {
        self.models.insert("hc_out");
        self.line(format!("A{port} [{d}] [m_{port}] hc_out"));
        self.line(format!("B{port} o_{port} gnd V = v(m_{port})*v(vcc,gnd)"));
        match enable {
            None => self.line(format!("R{port} o_{port} {port} 50")),
            Some(en) => {
                self.models.insert("hc_sw");
                self.line(format!("S{port} o_{port} {port} {en} 0 hc_sw"));
            }
        }
    }

    /// A digital device of `model` with connections `conns`.
    fn device(&mut self, model: &'static str, conns: &str) {
        self.models.insert(model);
        let name = self.node("U");
        self.line(format!("A{name} {conns} {model}"));
    }

    fn not(&mut self, d: &str) -> String {
        let out = self.node("x");
        self.device("hc_not", &format!("{d} {out}"));
        out
    }

    fn finish(self, name: &str, comment: &str, ports: &[String]) -> String {
        let mut out = vec![comment.to_string(), format!(".subckt {name} {}", ports.join(" "))];
        out.extend(self.lines);
        // Keeps VCC from floating when only the B-sources sense it.
        out.push("Rq vcc gnd 1meg".into());
        for m in self.models {
            out.push(format!(".model {m} {}", model_text(m)));
        }
        out.push(format!(".ends {name}"));
        out.join("\n")
    }
}

fn model_text(m: &str) -> &'static str {
    match m {
        "hc_in" => "adc_bridge(in_low=0.5 in_high=0.5)",
        "hc_out" => "dac_bridge(out_low=0 out_high=1 out_undef=0.5 t_rise=4n t_fall=4n)",
        "hc_fb" => "dac_bridge(out_low=0 out_high=1 out_undef=0.5 t_rise=1n t_fall=1n)",
        "hc_sw" => "sw(vt=0.5 vh=0.1 ron=50 roff=1g)",
        "hc_nand" => "d_nand(rise_delay=8n fall_delay=8n)",
        "hc_nor" => "d_nor(rise_delay=8n fall_delay=8n)",
        "hc_and" => "d_and(rise_delay=8n fall_delay=8n)",
        "hc_or" => "d_or(rise_delay=8n fall_delay=8n)",
        "hc_xor" => "d_xor(rise_delay=10n fall_delay=10n)",
        "hc_inv" => "d_inverter(rise_delay=7n fall_delay=7n)",
        "hc_not" => "d_inverter(rise_delay=1p fall_delay=1p)",
        "hc_dff" => "d_dff(clk_delay=14n set_delay=14n reset_delay=14n)",
        _ => unreachable!("model {m}"),
    }
}

const NOTE: &str = "* XSPICE digital: inputs switch at VCC/2, outputs follow VCC through 50 ohm, typical\n\
                    * 5 V delays. Supply current isn't modelled.";

/// `n` gates: ports A B Y (or A Y) per gate, then VCC GND.
fn gates_model(name: &str, gate: Gate, n: usize) -> String {
    let mut net = Net::default();
    let mut ports = Vec::new();
    for i in 1..=n {
        let ins: Vec<String> = ["a", "b"][..gate.inputs()].iter().map(|p| format!("{p}{i}")).collect();
        let ds: Vec<String> =
            ins.iter().map(|p| if gate == Gate::SchmittInverter { net.schmitt(p) } else { net.input(p) }).collect();
        let out = format!("y{i}");
        let d = format!("g{i}");
        let conns = if gate.inputs() == 1 { format!("{} {d}", ds[0]) } else { format!("[{}] {d}", ds.join(" ")) };
        net.device(gate.model(), &conns);
        net.output(&d, &out, None);
        ports.extend(ins);
        ports.push(out);
    }
    ports.extend(["vcc".into(), "gnd".into()]);
    let comment = format!("* {name}: {n} gates.\n{NOTE}");
    net.finish(name, &comment, &ports)
}

/// Two D flip-flops: CLR D CLK PRE Q ~Q each, then VCC GND.
fn flip_flops_model() -> String {
    let mut net = Net::default();
    let mut ports = Vec::new();
    for i in 1..=2 {
        let p: Vec<String> = ["clr", "d", "clk", "pre", "q", "nq"].iter().map(|n| format!("{n}{i}")).collect();
        let clr = net.input(&p[0]);
        let d = net.input(&p[1]);
        let clk = net.input(&p[2]);
        let pre = net.input(&p[3]);
        let (set, reset) = (net.not(&pre), net.not(&clr));
        let (q, nq) = (format!("q{i}_"), format!("nq{i}_"));
        net.device("hc_dff", &format!("{d} {clk} {set} {reset} {q} {nq}"));
        net.output(&q, &p[4], None);
        net.output(&nq, &p[5], None);
        ports.extend(p);
    }
    ports.extend(["vcc".into(), "gnd".into()]);
    net.finish("HC74", &format!("* HC74: two D flip-flops, preset and clear active low.\n{NOTE}"), &ports)
}

/// The 74HC595: SER SRCLK SRCLR RCLK OE QA…QH QH' VCC GND.
fn shift_register_model() -> String {
    let mut net = Net::default();
    let ser = net.input("ser");
    let srclk = net.input("srclk");
    let srclr = net.input("srclr");
    let rclk = net.input("rclk");
    let oe = net.input("oe");
    let clear = net.not(&srclr);
    // Outputs drive while OE is low.
    let enable = net.not(&oe);
    net.models.insert("hc_fb");
    net.line(format!("Aen [{enable}] [m_en] hc_fb"));
    let outs = ["qa", "qb", "qc", "qd", "qe", "qf", "qg", "qh"];
    let mut data = ser;
    for (i, out) in outs.iter().enumerate() {
        let (sr, nsr) = (format!("sr{i}"), format!("nsr{i}"));
        net.device("hc_dff", &format!("{data} {srclk} NULL {clear} {sr} {nsr}"));
        let (st, nst) = (format!("st{i}"), format!("nst{i}"));
        net.device("hc_dff", &format!("{sr} {rclk} NULL NULL {st} {nst}"));
        net.output(&st, out, Some("m_en"));
        data = sr;
    }
    net.output(&data, "qhs", None);
    let mut ports: Vec<String> = ["ser", "srclk", "srclr", "rclk", "oe"].iter().map(|s| s.to_string()).collect();
    ports.extend(outs.iter().map(|s| s.to_string()));
    ports.extend(["qhs".into(), "vcc".into(), "gnd".into()]);
    let comment = format!(
        "* HC595: 8-bit shift register (SRCLK, cleared by SRCLR low) into a storage register\n\
         * (RCLK); QA…QH drive while OE is low, QH' (qhs) is the shift register's last stage.\n{NOTE}"
    );
    net.finish("HC595", &comment, &ports)
}
