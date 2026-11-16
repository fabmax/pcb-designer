//! Excellon drill files: plated holes (through-hole pads and vias) and non-plated holes,
//! in mm with explicit decimal points, X2 attributes as comments (as KiCad writes them).
//! Slots are routed with G85 (as KiCad writes them).

use std::fmt::Write;

pub use editor_core::fab::Hole;
use editor_core::fab::pad_hole;
use editor_core::query::footprint_of_instance;
use model::*;

/// The holes of the board: `(plated, non_plated)`.
pub fn holes(project: &Project) -> (Vec<Hole>, Vec<Hole>) {
    let (mut plated, mut npth) = (Vec::new(), Vec::new());
    for inst in project.board.footprints.iter() {
        let Some(fp) = footprint_of_instance(project, inst) else { continue };
        let t = inst.transform();
        for pad in fp.pads.iter() {
            let Some(hole) = pad_hole(pad, &t) else { continue };
            match pad.kind {
                PadKind::ThroughHole { .. } => plated.push(hole),
                _ => npth.push(hole),
            }
        }
    }
    plated.extend(project.board.vias.iter().filter(|v| v.is_through()).map(via_hole));
    (plated, npth)
}

fn via_hole(v: &Via) -> Hole {
    Hole { a: v.pos, b: v.pos, diameter: v.drill }
}

/// Holes of blind and buried vias by span (top layer first), in layer order.
pub fn span_holes(project: &Project) -> Vec<([LayerId; 2], Vec<Hole>)> {
    let mut spans: std::collections::BTreeMap<(u16, u16), Vec<Hole>> = Default::default();
    for v in project.board.vias.iter().filter(|v| !v.is_through() && v.from != v.to) {
        let key = (v.from.0.min(v.to.0), v.from.0.max(v.to.0));
        spans.entry(key).or_default().push(via_hole(v));
    }
    spans.into_iter().map(|((a, b), h)| ([LayerId(a), LayerId(b)], h)).collect()
}

/// Millimetres with four decimals (0.1 µm).
fn mm4(v: Coord) -> String {
    format!("{:.4}", v as f64 * 1e-6)
}

/// The drill file of through `holes`; `layers` is the number of copper layers.
pub fn drill_file(holes: &[Hole], plated: bool, layers: usize) -> String {
    let function = if plated { format!("Plated,1,{layers},PTH") } else { format!("NonPlated,1,{layers},NPTH") };
    write_drills(holes, &function)
}

/// The drill file of blind (reaching the top or bottom) or buried vias from copper layer
/// number `from` to `to` (1 is the top).
pub fn span_drill_file(holes: &[Hole], from: usize, to: usize, layers: usize) -> String {
    let kind = if from == 1 || to == layers { "Blind" } else { "Buried" };
    write_drills(holes, &format!("Plated,{from},{to},{kind}"))
}

/// A drill file with the X2 file function `function`.
fn write_drills(holes: &[Hole], function: &str) -> String {
    let mut tools: Vec<Coord> = holes.iter().map(|h| h.diameter).collect();
    tools.sort();
    tools.dedup();
    let mut out = String::new();
    out.push_str("M48\n");
    let _ = writeln!(out, "; DRILL file {{pcb-designer {}}}", env!("CARGO_PKG_VERSION"));
    out.push_str("; FORMAT={-:-/ absolute / metric / decimal}\n");
    let _ = writeln!(out, "; #@! TF.FileFunction,{function}");
    out.push_str("FMAT,2\nMETRIC\n");
    for (i, d) in tools.iter().enumerate() {
        let _ = writeln!(out, "T{}C{}", i + 1, mm4(*d));
    }
    out.push_str("%\nG90\nG05\n");
    for (i, d) in tools.iter().enumerate() {
        let _ = writeln!(out, "T{}", i + 1);
        for h in holes.iter().filter(|h| h.diameter == *d) {
            if h.is_slot() {
                // A routed slot: from one end to the other.
                let _ = writeln!(out, "X{}Y{}G85X{}Y{}", mm4(h.a.x), mm4(h.a.y), mm4(h.b.x), mm4(h.b.y));
            } else {
                let _ = writeln!(out, "X{}Y{}", mm4(h.a.x), mm4(h.a.y));
            }
        }
    }
    out.push_str("M30\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tools_by_size() {
        let round = |p: Point, d: Coord| Hole { a: p, b: p, diameter: d };
        let slot = Hole { a: Point::mm(5.0, 0.0), b: Point::mm(6.0, 0.0), diameter: mm(0.8) };
        let holes = [
            round(Point::mm(1.0, 2.0), mm(0.8)),
            round(Point::mm(-3.0, 0.5), mm(0.3)),
            round(Point::ZERO, mm(0.8)),
            slot,
        ];
        let f = drill_file(&holes, true, 2);
        assert!(f.contains("T1C0.3000\nT2C0.8000\n"));
        assert!(f.contains("T2\nX1.0000Y2.0000\nX0.0000Y0.0000\nX5.0000Y0.0000G85X6.0000Y0.0000\n"), "{f}");
        assert!(f.contains("TF.FileFunction,Plated,1,2,PTH"));
        assert!(f.ends_with("M30\n"));
    }
}
