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
    footprint: &'a Footprint,
    properties: &'a BTreeMap<String, String>,
    inst: &'a FootprintInstance,
}

impl Placed<'_> {
    /// The first property among `keys` (any case) that's set.
    fn property(&self, keys: &[&str]) -> String {
        keys.iter()
            .find_map(|k| self.properties.iter().find(|(p, v)| p.eq_ignore_ascii_case(k) && !v.trim().is_empty()))
            .map_or(String::new(), |(_, v)| v.trim().to_string())
    }

    /// "SMD", or "THT" when any pad has a plated hole.
    fn mount(&self) -> &'static str {
        let tht = self.footprint.pads.iter().any(|p| matches!(p.kind, PadKind::ThroughHole { .. }));
        if tht { "THT" } else { "SMD" }
    }

    fn side(&self) -> &'static str {
        match self.inst.side {
            Side::Top => "Top",
            Side::Bottom => "Bottom",
        }
    }
}

/// Part number properties JLCPCB's BOM takes.
const LCSC_KEYS: [&str; 4] = ["LCSC", "LCSC Part #", "LCSC Part", "JLCPCB Part #"];
const MPN_KEYS: [&str; 3] = ["MPN", "Manufacturer Part Number", "Mfr Part #"];
const MANUFACTURER_KEYS: [&str; 2] = ["Manufacturer", "Mfr"];

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
            let footprint = footprint_of_instance(project, inst)?;
            Some(Placed { refdes: &c.refdes, value: &c.value, footprint, properties: &c.properties, inst })
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

/// The bill of materials laid out for `preset`: one line per value, footprint and
/// properties, with the designators (in natural order) and the quantity.
/// * Generic: then a column per property key used by any component (MPN, …).
/// * JLCPCB: `Comment, Designator, Footprint, LCSC Part #`.
/// * PCBWay: item number, designators, quantity, manufacturer, part number, value,
///   footprint, SMD or THT, and an empty notes column.
pub fn bom(project: &Project, preset: AssemblyPreset) -> String {
    let placed = placed(project);
    // Lines in the order of their first designator.
    let mut lines: Vec<(Vec<&str>, &Placed)> = Vec::new();
    for p in &placed {
        let same =
            |q: &Placed| q.value == p.value && q.footprint.name == p.footprint.name && q.properties == p.properties;
        match lines.iter_mut().find(|(_, q)| same(q)) {
            Some((refs, _)) => refs.push(p.refdes),
            None => lines.push((vec![p.refdes], p)),
        }
    }
    let mut out = String::new();
    match preset {
        AssemblyPreset::Generic => {
            let keys: Vec<&String> = {
                let mut k: Vec<&String> = placed.iter().flat_map(|p| p.properties.keys()).collect();
                k.sort();
                k.dedup();
                k
            };
            let head = ["Designator", "Value", "Footprint", "Quantity"].map(String::from);
            row(&mut out, head.into_iter().chain(keys.iter().map(|k| k.to_string())));
            for (refs, p) in &lines {
                let props = keys.iter().map(|k| p.properties.get(*k).cloned().unwrap_or_default());
                let fields = [refs.join(","), p.value.to_string(), p.footprint.name.clone(), refs.len().to_string()];
                row(&mut out, fields.into_iter().chain(props));
            }
        }
        AssemblyPreset::Jlcpcb => {
            row(&mut out, ["Comment", "Designator", "Footprint", "LCSC Part #"].map(String::from));
            for (refs, p) in &lines {
                row(&mut out, [p.value.to_string(), refs.join(","), p.footprint.name.clone(), p.property(&LCSC_KEYS)]);
            }
        }
        AssemblyPreset::Pcbway => {
            let head = [
                "Item #",
                "Designator",
                "Qty",
                "Manufacturer",
                "Mfg Part #",
                "Description / Value",
                "Package/Footprint",
                "Type",
                "Your Instructions / Notes",
            ];
            row(&mut out, head.map(String::from));
            for (i, (refs, p)) in lines.iter().enumerate() {
                row(
                    &mut out,
                    [
                        (i + 1).to_string(),
                        refs.join(","),
                        refs.len().to_string(),
                        p.property(&MANUFACTURER_KEYS),
                        p.property(&MPN_KEYS),
                        p.value.to_string(),
                        p.footprint.name.clone(),
                        p.mount().to_string(),
                        String::new(),
                    ],
                );
            }
        }
    }
    out
}

/// Millimetres with four decimals.
fn mm4(v: Coord) -> String {
    format!("{:.4}", v as f64 / 1e6)
}

/// The component positions laid out for `preset`: the footprint origin in board
/// coordinates (mm, as in the Gerber files), the rotation (degrees counterclockwise, as
/// placed) and the side.
/// * Generic: `Designator, Value, Footprint, Mid X, Mid Y, Rotation, Layer`.
/// * JLCPCB: `Designator, Mid X, Mid Y, Layer, Rotation`.
/// * PCBWay: `Designator, Footprint, Mid X, Mid Y, Layer, Rotation, Comment` (the value).
pub fn positions(project: &Project, preset: AssemblyPreset) -> String {
    let mut out = String::new();
    let head: &[&str] = match preset {
        AssemblyPreset::Generic => &["Designator", "Value", "Footprint", "Mid X", "Mid Y", "Rotation", "Layer"],
        AssemblyPreset::Jlcpcb => &["Designator", "Mid X", "Mid Y", "Layer", "Rotation"],
        AssemblyPreset::Pcbway => &["Designator", "Footprint", "Mid X", "Mid Y", "Layer", "Rotation", "Comment"],
    };
    row(&mut out, head.iter().map(|h| h.to_string()));
    for p in placed(project) {
        let (x, y) = (mm4(p.inst.pos.x), mm4(p.inst.pos.y));
        let rotation = format!("{:.2}", p.inst.rotation.degrees());
        let (refdes, value, footprint, side) =
            (p.refdes.to_string(), p.value.to_string(), p.footprint.name.clone(), p.side().to_string());
        let fields = match preset {
            AssemblyPreset::Generic => vec![refdes, value, footprint, x, y, rotation, side],
            AssemblyPreset::Jlcpcb => vec![refdes, x, y, side, rotation],
            AssemblyPreset::Pcbway => vec![refdes, footprint, x, y, side, rotation, value],
        };
        row(&mut out, fields);
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
