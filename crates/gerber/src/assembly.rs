//! Assembly files: the bill of materials and the component positions (pick and place),
//! as CSV in the column layout assembly services take (JLCPCB, PCBWay).
//!
//! Only placed footprints of schematic components count: board parts (mounting holes,
//! logos) and "do not populate" components aren't assembled, and components without a
//! footprint aren't on the board.

use std::collections::BTreeMap;

use editor_core::query::footprint_of_instance;
use model::*;

/// A placed component as the assembly files see it.
struct Placed<'a> {
    refdes: &'a str,
    value: &'a str,
    footprint: &'a str,
    properties: &'a BTreeMap<String, String>,
    inst: &'a FootprintInstance,
}

fn placed(project: &Project) -> Vec<Placed<'_>> {
    let mut out: Vec<Placed> = project
        .board
        .footprints
        .iter()
        .filter_map(|inst| {
            let c = project.schematic.components.get(inst.component)?;
            let part = project.library.parts.get(c.part)?;
            if part.is_board_part() || c.dnp {
                return None;
            }
            let fp = footprint_of_instance(project, inst)?;
            Some(Placed { refdes: &c.refdes, value: &c.value, footprint: &fp.name, properties: &c.properties, inst })
        })
        .collect();
    out.sort_by(|a, b| natural_cmp(a.refdes, b.refdes));
    out
}

/// One CSV field, quoted when it needs to be.
fn field(s: &str) -> String {
    if s.contains([',', '"', '\n', '\r']) || s.starts_with(' ') || s.ends_with(' ') {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

fn row(out: &mut String, fields: impl IntoIterator<Item = String>) {
    let line: Vec<String> = fields.into_iter().map(|f| field(&f)).collect();
    out.push_str(&line.join(","));
    out.push_str("\r\n");
}

/// The bill of materials: one line per value, footprint and properties, with the
/// designators (in natural order) and the quantity, then a column per property key used
/// by any component (MPN, manufacturer, LCSC part number, …).
pub fn bom(project: &Project) -> String {
    let placed = placed(project);
    let keys: Vec<&String> = {
        let mut k: Vec<&String> = placed.iter().flat_map(|p| p.properties.keys()).collect();
        k.sort();
        k.dedup();
        k
    };
    // Lines in the order of their first designator.
    let mut lines: Vec<(Vec<&str>, &Placed)> = Vec::new();
    for p in &placed {
        let same = |q: &Placed| q.value == p.value && q.footprint == p.footprint && q.properties == p.properties;
        match lines.iter_mut().find(|(_, q)| same(q)) {
            Some((refs, _)) => refs.push(p.refdes),
            None => lines.push((vec![p.refdes], p)),
        }
    }
    let mut out = String::new();
    let head = ["Designator", "Value", "Footprint", "Quantity"].map(String::from);
    row(&mut out, head.into_iter().chain(keys.iter().map(|k| k.to_string())));
    for (refs, p) in &lines {
        let props = keys.iter().map(|k| p.properties.get(*k).cloned().unwrap_or_default());
        let fields = [refs.join(","), p.value.to_string(), p.footprint.to_string(), refs.len().to_string()];
        row(&mut out, fields.into_iter().chain(props));
    }
    out
}

/// Millimetres with four decimals.
fn mm4(v: Coord) -> String {
    format!("{:.4}", v as f64 / 1e6)
}

/// The component positions: designator, value, footprint, the footprint origin in board
/// coordinates (mm, as in the Gerber files), the rotation (degrees counterclockwise, as
/// placed) and the side.
pub fn positions(project: &Project) -> String {
    let mut out = String::new();
    row(&mut out, ["Designator", "Value", "Footprint", "Mid X", "Mid Y", "Rotation", "Layer"].map(String::from));
    for p in placed(project) {
        let side = match p.inst.side {
            Side::Top => "Top",
            Side::Bottom => "Bottom",
        };
        let rotation = format!("{:.2}", p.inst.rotation.degrees());
        row(
            &mut out,
            [
                p.refdes.to_string(),
                p.value.to_string(),
                p.footprint.to_string(),
                mm4(p.inst.pos.x),
                mm4(p.inst.pos.y),
                rotation,
                side.to_string(),
            ],
        );
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fields_are_quoted_when_needed() {
        assert_eq!(field("R1"), "R1");
        assert_eq!(field("R1,R2"), "\"R1,R2\"");
        assert_eq!(field("2\" wide"), "\"2\"\" wide\"");
    }
}
