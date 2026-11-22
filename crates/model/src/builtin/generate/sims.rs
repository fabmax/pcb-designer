//! Simulation models of the Built-in parts and the simulation-only parts (sources, and
//! passives without footprint).
//!
//! Models are generic: typical values of a common part of each kind (1N4148, 2N3904,
//! 2N7002), not the exact part in the value field. Vendor macromodels aren't bundled
//! (their licences usually forbid it). The hand-made parts' models (op amps,
//! regulators, logic) are the `pub` texts here, set in the bundled file.

use std::collections::BTreeMap;

use super::super::gid;
use super::{Builder, sym};
use crate::id::Id;
use crate::library::*;
use crate::simulation::*;

pub const DIODE: &str = ".model D1N4148 D(IS=2.52n RS=0.568 N=1.752 CJO=4p M=0.4 TT=20n BV=100 IBV=100u)";
pub const SCHOTTKY: &str = ".model DB5819 D(IS=31.7u RS=0.051 N=1.373 CJO=110p M=0.35 EG=0.69 XTI=2 BV=40 IBV=1m)";
/// Breakdown from the value (`BV` = `${VALUE}`: "5V1", "BZT52C3V3").
pub const ZENER: &str = ".model DZENER D(IS=1n RS=2 N=1.2 CJO=100p BV=5.1 IBV=5m)";
pub const TVS: &str = ".model DTVS D(IS=1n RS=0.05 N=1.2 CJO=1n BV=6.4 IBV=1m)";
pub const TVS_BIDIRECTIONAL: &str = "\
* Two TVS diodes back to back.
.subckt TVS_BI 1 2
D1 m 1 DTVS_BI
D2 m 2 DTVS_BI
.model DTVS_BI D(IS=1n RS=0.05 N=1.2 CJO=1n BV=6.4 IBV=1m)
.ends TVS_BI";
/// About 1.8 V at 10 mA.
pub const LED: &str = ".model DLED_RED D(IS=93.5p RS=2.2 N=3.7 CJO=10p BV=5 IBV=10u)";
pub const NPN: &str = ".model Q2N3904 NPN(IS=6.734f XTI=3 EG=1.11 VAF=74.03 BF=416.4 NE=1.259 ISE=6.734f IKF=66.78m \
XTB=1.5 BR=0.7371 NC=2 ISC=0 IKR=0 RC=1 CJC=3.638p MJC=0.3085 VJC=0.75 FC=0.5 CJE=4.493p MJE=0.2593 VJE=0.75 \
TR=239.5n TF=301.2p ITF=0.4 VTF=4 XTF=2 RB=10)";
pub const PNP: &str = ".model Q2N3906 PNP(IS=1.41f XTI=3 EG=1.11 VAF=18.7 BF=180.7 NE=1.5 ISE=0 IKF=80m XTB=1.5 \
BR=4.977 NC=2 ISC=0 IKR=0 RC=2.5 CJC=9.728p MJC=0.5776 VJC=0.75 FC=0.5 CJE=8.063p MJE=0.3677 VJE=0.75 TR=33.42n \
TF=179.3p ITF=0.4 VTF=4 XTF=6 RB=10)";
pub const NMOS: &str = ".model M2N7002 VDMOS(Vto=2.1 Kp=0.5 Rd=1 Rs=0.5 Rg=10 Cgdmax=40p Cgdmin=4p Cgs=20p Cjo=15p \
Is=1e-14 Rb=0.5)";
pub const PMOS: &str = ".model MBSS84 VDMOS(pchan Vto=-1.6 Kp=0.15 Rd=3 Rs=1 Rg=10 Cgdmax=20p Cgdmin=3p Cgs=20p \
Cjo=12p Is=1e-14 Rb=1)";
/// Butterworth–Van Dyke equivalent; the series resonance `f` comes from the value.
pub const CRYSTAL: &str = "\
* Quartz crystal: motional arm (Lm, Cm = 16 fF, Rm = 30 ohm) parallel to C0 = 5 pF.
.subckt XTAL 1 2 params: f=16meg
Lm 1 a {1/((6.2831853*f)*(6.2831853*f)*16f)}
Cm a b 16f
Rm b 2 30
C0 1 2 5p
.ends XTAL";

/// An op amp named `name`: one pole (`aol`, `gbw`), slew limited (`sr` in V/s), the
/// output within `vhi` / `vlo` of the rails, `iq` from V+ to V−. Nodes: +IN −IN OUT V+ V−.
///
/// Gotcha: B-source expressions use subcircuit parameters by bare name (or through a
/// `.param` inside the subcircuit); `{…}` expressions of parameters inside a B-source
/// line evaluate wrongly in ngspice 47, without an error.
pub fn opamp(name: &str, aol: &str, gbw: &str, sr: &str, vhi: &str, vlo: &str, iq: &str) -> String {
    format!(
        "\
* Generic op amp: open-loop gain aol, one pole at gbw/aol, slew rate sr (V/s),
* output within vhi of V+ and vlo of V-, quiescent current iq.
.subckt {name} inp inn out vp vn params: aol={aol} gbw={gbw} sr={sr} vhi={vhi} vlo={vlo} iq={iq}
Rin inp inn 10meg
.param imax={{sr*1m/(6.2831853*gbw)}}
Bg 0 int I = imax * tanh(v(inp,inn) * 1m / imax)
Rp int 0 {{aol/1m}}
Cp int 0 {{1m/(6.2831853*gbw)}}
Dhi int vp DCLAMP
Dlo vn int DCLAMP
Bo o 0 V = min(max(v(int), v(vn)+vlo), v(vp)-vhi)
Ro o out 50
Iq vp vn {{iq}}
.model DCLAMP D(IS=1e-12)
.ends {name}"
    )
}

/// A linear regulator: `vout` above GND while the input allows (`drop`), the output
/// current drawn from the input, `iq` from IN to GND. Nodes: IN OUT GND, or with `en`:
/// IN EN OUT GND (on above 1 V).
pub fn regulator(name: &str, vout: &str, drop: &str, iq: &str, en: bool) -> String {
    let (ports, gate) = if en { ("in en out gnd", " * u(v(en,gnd)-1)") } else { ("in out gnd", "") };
    format!(
        "\
* Generic linear regulator: OUT = min(vout, IN - drop) above GND, output current taken
* from IN, quiescent current iq.
.subckt {name} {ports} params: vout={vout} drop={drop} iq={iq}
Bv o gnd V = max(min(vout, v(in,gnd) - drop), 0){gate}
Ro o s 0.05
Vs s out 0
F1 in gnd Vs 1
Iq in gnd {{iq}}
.ends {name}"
    )
}

/// The quad NAND: four behavioural gates (thresholds at half the supply, ~7 ns edges,
/// 50 Ω outputs). Nodes: 1A 1B 1Y … 4A 4B 4Y VCC GND (the units' pin order).
pub const NAND4: &str = "\
* Quad 2-input NAND, behavioural: inputs switch at VCC/2, outputs rail to rail.
.subckt HC00 a1 b1 y1 a2 b2 y2 a3 b3 y3 a4 b4 y4 vcc gnd
X1 a1 b1 y1 vcc gnd HC00_NAND
X2 a2 b2 y2 vcc gnd HC00_NAND
X3 a3 b3 y3 vcc gnd HC00_NAND
X4 a4 b4 y4 vcc gnd HC00_NAND
Rq vcc gnd 1meg
.ends HC00
.subckt HC00_NAND a b y vcc gnd
Bo o gnd V = v(vcc,gnd) * (1 - (0.5+0.5*tanh(10*(v(a,gnd)-0.5*v(vcc,gnd)))) * (0.5+0.5*tanh(10*(v(b,gnd)-0.5*v(vcc,gnd)))))
Rd o d 1k
Cd d gnd 7p
Eo e gnd d gnd 1
Ro e y 50
Ca a gnd 3p
Cb b gnd 3p
.ends HC00_NAND";

/// The model of each generated part, by name.
/// A model: the device, its pins by number or name (empty: the symbol's order), parameters.
type ModelSpec = (SimDevice, &'static [&'static str], &'static [(&'static str, &'static str)]);

fn model_of(name: &str) -> Option<ModelSpec> {
    let prim = |kind, model: &str| SimDevice::Primitive { kind, model: model.into() };
    Some(match name {
        "Resistor" => (SimDevice::Resistor, &[], &[]),
        "Capacitor" | "Capacitor_Polarized" => (SimDevice::Capacitor, &[], &[]),
        "Inductor" => (SimDevice::Inductor, &[], &[]),
        // Simplified as a small resistance.
        "FerriteBead" | "Fuse" => (SimDevice::Spice { template: "R${REF} ${N1} ${N2} 0.1".into() }, &[], &[]),
        // Diodes: pins A (2) then K (1), the symbol's order.
        "Diode" => (prim(PrimitiveKind::Diode, DIODE), &[], &[]),
        "Diode_Schottky" => (prim(PrimitiveKind::Diode, SCHOTTKY), &[], &[]),
        "Diode_Zener" => (prim(PrimitiveKind::Diode, ZENER), &[], &[("BV", "${VALUE}")]),
        "TVS" => (prim(PrimitiveKind::Diode, TVS), &[], &[]),
        "TVS_Bidirectional" => (SimDevice::Subckt { text: TVS_BIDIRECTIONAL.into() }, &[], &[]),
        "LED" => (prim(PrimitiveKind::Diode, LED), &[], &[]),
        "NPN" => (prim(PrimitiveKind::Npn, NPN), &["C", "B", "E"], &[]),
        "PNP" => (prim(PrimitiveKind::Pnp, PNP), &["C", "B", "E"], &[]),
        "NMOS" => (prim(PrimitiveKind::Nmos, NMOS), &["D", "G", "S"], &[]),
        "PMOS" => (prim(PrimitiveKind::Pmos, PMOS), &["D", "G", "S"], &[]),
        "Crystal" => (SimDevice::Subckt { text: CRYSTAL.into() }, &[], &[("f", "${VALUE}")]),
        "Crystal_GND24" => (SimDevice::Subckt { text: CRYSTAL.into() }, &["1", "3"], &[("f", "${VALUE}")]),
        n if n.starts_with("Conn_")
            || n.starts_with("MountingHole")
            || matches!(n, "TestPoint" | "SW_Push" | "Fiducial") =>
        {
            (SimDevice::Ignore, &[], &[])
        }
        _ => return None,
    })
}

pub(super) fn add(b: &mut Builder) {
    let parts: Vec<Id> = b.lib.parts.iter().map(|p| p.id).collect();
    for id in parts {
        let part = b.lib.parts.get(id).unwrap();
        let Some((device, pins, params)) = model_of(&part.name) else { continue };
        let symbol = part.units.first().and_then(|u| b.lib.symbols.get(u.symbol));
        let pins = pins
            .iter()
            .map(|key| {
                let pin = symbol
                    .and_then(|s| s.pins.iter().find(|p| p.key() == *key || p.name == *key))
                    .unwrap_or_else(|| panic!("{}: no pin {key}", part.name));
                SimPin { unit: 0, pin: pin.id }
            })
            .collect();
        let params: BTreeMap<String, String> = params.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        b.lib.parts.get_mut(id).unwrap().sim = Some(SimModel { device, pins, params });
    }
    for (symbol, device, prefix, what) in [
        (sym::vsource(), SimDevice::VoltageSource, "V", "Voltage source"),
        (sym::isource(), SimDevice::CurrentSource, "I", "Current source"),
    ] {
        let name = symbol.name.clone();
        let unit = symbol.id;
        b.symbol(symbol);
        if let Some(s) = b.lib.symbols.get_mut(unit) {
            s.category = "Simulation".into();
        }
        let unit_name = if device == SimDevice::CurrentSource { "A" } else { "V" };
        b.lib.parts.insert(Part {
            id: gid(&format!("part/{name}")),
            name: name.clone(),
            category: "Simulation".into(),
            description: format!(
                "{what} for simulation only (not on the board): DC, sine, pulse, piecewise linear or \
                 exponential, picked in its properties"
            ),
            prefix: prefix.into(),
            default_value: Source::default().describe(unit_name),
            units: vec![PartUnit { name: "A".into(), symbol: unit }],
            variants: vec![PartVariant::no_footprint(gid(&format!("part/{name}/variant/none")))],
            properties: Default::default(),
            sim: Some(SimModel::new(device)),
            source: None,
        });
    }
    // The passives again without footprint, e.g. as a load on a test bench sheet, with
    // the board parts' symbols (which stay in Passives).
    for (what, prefix, value, device) in [
        ("Resistor", "R", "1k", SimDevice::Resistor),
        ("Capacitor", "C", "100n", SimDevice::Capacitor),
        ("Inductor", "L", "10u", SimDevice::Inductor),
    ] {
        let unit = *b.symbols.get(what).unwrap_or_else(|| panic!("no symbol {what}"));
        let name = format!("{what}_Sim");
        b.lib.parts.insert(Part {
            id: gid(&format!("part/{name}")),
            name: name.clone(),
            category: "Simulation".into(),
            description: format!(
                "{what} for simulation only (not on the board), e.g. a load or a source's impedance \
                 on a test bench sheet"
            ),
            prefix: prefix.into(),
            default_value: value.into(),
            units: vec![PartUnit { name: "A".into(), symbol: unit }],
            variants: vec![PartVariant::no_footprint(gid(&format!("part/{name}/variant/none")))],
            properties: Default::default(),
            sim: Some(SimModel::new(device)),
            source: None,
        });
    }
}
