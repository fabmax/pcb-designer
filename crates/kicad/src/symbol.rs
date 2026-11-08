//! KiCad symbols → [`Symbol`]s (one per unit) plus the data needed to make a [`Part`].
//!
//! A KiCad symbol `NAME` contains sub-symbols `NAME_U_C`: unit `U` (0 = shared by all
//! units) and body style `C` (0 = shared, 1 = normal, 2 = De Morgan alternative, which is
//! not imported). Derived symbols (`extends`) take the graphics and pins of their parent
//! and override its properties. Symbol coordinates are y-up in mm, like ours.

use std::collections::BTreeMap;

use model::*;

use crate::common::*;
use crate::sexpr::{self, Node};

/// Default line width for zero-width strokes (KiCad's schematic default, 6 mil).
const DEFAULT_LINE: Coord = 152_400;

/// A converted KiCad symbol.
#[derive(Clone, Debug)]
pub struct KiSymbol {
    pub lib: String,
    pub name: String,
    /// Reference prefix ("R", "U").
    pub reference: String,
    pub value: String,
    /// Default footprint as `Library:Name`, if set.
    pub footprint: Option<String>,
    pub datasheet: String,
    pub description: String,
    pub keywords: String,
    /// Footprint name patterns (`ki_fp_filters`).
    pub fp_filters: Vec<String>,
    /// Power symbols (GND, +5V) are net labels in this application; they are not imported.
    pub power: bool,
    /// One symbol per unit.
    pub units: Vec<Symbol>,
    pub unit_names: Vec<String>,
    pub warnings: Vec<String>,
}

impl KiSymbol {
    pub fn full_name(&self) -> String {
        format!("{}:{}", self.lib, self.name)
    }
}

/// Top-level symbols of a library file, by name.
pub fn parse_library(text: &str) -> Result<Vec<(String, Node)>, sexpr::ParseError> {
    let root = sexpr::parse(text)?;
    if root.head() != "kicad_symbol_lib" {
        return Err(sexpr::ParseError { line: 1, message: "not a KiCad symbol library".into() });
    }
    Ok(root.children("symbol").filter_map(|n| Some((n.arg(0)?.to_string(), n.clone()))).collect())
}

fn properties(n: &Node) -> BTreeMap<String, Node> {
    n.children("property").filter_map(|p| Some((p.arg(0)?.to_string(), p.clone()))).collect()
}

fn prop_value<'a>(props: &'a BTreeMap<String, Node>, key: &str) -> &'a str {
    props.get(key).and_then(|p| p.arg(1)).unwrap_or("")
}

/// Convert symbol `node` of library `lib`. `parent` is the symbol it `extends`, if any.
pub fn convert(lib: &str, node: &Node, parent: Option<&Node>) -> KiSymbol {
    let name = node.arg(0).unwrap_or("").to_string();
    let body = parent.unwrap_or(node);
    let body_name = body.arg(0).unwrap_or("");
    let mut props = parent.map(properties).unwrap_or_default();
    props.extend(properties(node));
    let mut warnings = Vec::new();

    let reference = prop_value(&props, "Reference").trim_end_matches('?').to_string();
    let footprint = Some(prop_value(&props, "Footprint").trim().to_string()).filter(|f| !f.is_empty());
    let description = match prop_value(&props, "Description") {
        "" => prop_value(&props, "ki_description"),
        d => d,
    }
    .to_string();
    let power = body.child("power").is_some() || body.flag("power");
    let names_hidden = body.child("pin_names").is_some_and(|p| p.flag("hide"));
    let numbers_hidden = body.child("pin_numbers").is_some_and(|p| p.flag("hide"));

    // Sub-symbols by (unit, body style).
    let mut subs: Vec<(u32, u32, &Node)> = Vec::new();
    for sub in body.children("symbol") {
        let sub_name = sub.arg(0).unwrap_or("");
        let suffix = sub_name.strip_prefix(body_name).and_then(|s| s.strip_prefix('_')).unwrap_or(sub_name);
        let mut it = suffix.rsplitn(2, '_');
        let style = it.next().and_then(|c| c.parse().ok()).unwrap_or(1);
        let unit = it.next().and_then(|u| u.parse().ok()).unwrap_or(0);
        if style >= 2 {
            continue; // De Morgan alternative
        }
        subs.push((unit, style, sub));
    }
    if body.children("symbol").any(|s| s.arg(0).is_some_and(|n| n.ends_with("_2"))) {
        warnings.push("alternate (De Morgan) body style skipped".into());
    }
    let unit_count = subs.iter().map(|s| s.0).max().unwrap_or(0).max(1);

    let mut units = Vec::new();
    let mut unit_names = Vec::new();
    for u in 1..=unit_count {
        let letter = unit_letter(u as usize - 1);
        let unit_name = subs
            .iter()
            .filter(|s| s.0 == u)
            .find_map(|s| s.2.child("unit_name").and_then(|n| n.arg(0)))
            .map_or(letter.clone(), str::to_string);
        let symbol_name = if unit_count > 1 { format!("{name} {letter}") } else { name.clone() };
        let id = kid(&["kicad-symbol", lib, &name, &u.to_string()]);
        let mut sym = Symbol { id, name: symbol_name, graphics: Vec::new(), texts: Vec::new(), pins: Store::default() };
        let mut seen: BTreeMap<(String, String), usize> = BTreeMap::new();
        for &(su, _, sub) in subs.iter().filter(|s| s.0 == 0 || s.0 == u) {
            for item in sub.lists() {
                match item.head() {
                    "pin" => {
                        // Pins shared by all units are placed once, in the first unit.
                        if su == 0 && u != 1 {
                            continue;
                        }
                        if let Some(mut pin) = convert_pin(item, names_hidden, numbers_hidden) {
                            let k = (pin.number.clone(), pin.name.clone());
                            let n = seen.entry(k).or_default();
                            *n += 1;
                            pin.id =
                                kid(&["kicad-pin", lib, &name, &u.to_string(), &pin.number, &pin.name, &n.to_string()]);
                            sym.pins.insert(pin);
                        }
                    }
                    "text" => {
                        let content = plain(item.arg(0).unwrap_or(""));
                        if let Some(at) = item.child("at")
                            && let Some(t) = text(content, at, item.child("effects"), false, 0.1)
                        {
                            sym.texts.push(t);
                        }
                    }
                    "text_box" => warnings.push("text box skipped".into()),
                    _ => sym.graphics.extend(convert_graphic(item)),
                }
            }
        }
        // Reference and value as placeholders where KiCad shows them.
        for (key, placeholder) in [("Reference", "${REF}"), ("Value", "${VALUE}")] {
            if let Some(p) = props.get(key)
                && !p.flag("hide")
                && let Some(at) = p.child("at")
                && let Some(t) = text(placeholder.into(), at, p.child("effects"), false, 1.0)
            {
                sym.texts.push(t);
            }
        }
        units.push(sym);
        unit_names.push(unit_name);
    }
    warnings.sort();
    warnings.dedup();

    KiSymbol {
        lib: lib.to_string(),
        name,
        reference,
        value: prop_value(&props, "Value").to_string(),
        footprint,
        datasheet: Some(prop_value(&props, "Datasheet")).filter(|d| *d != "~").unwrap_or("").to_string(),
        description,
        keywords: prop_value(&props, "ki_keywords").to_string(),
        fp_filters: prop_value(&props, "ki_fp_filters").split_whitespace().map(str::to_string).collect(),
        power,
        units,
        unit_names,
        warnings,
    }
}

/// "A" … "Z", "AA", "AB", …
fn unit_letter(i: usize) -> String {
    if i < 26 {
        ((b'A' + i as u8) as char).to_string()
    } else {
        format!("{}{}", unit_letter(i / 26 - 1), unit_letter(i % 26))
    }
}

fn convert_pin(n: &Node, names_hidden: bool, numbers_hidden: bool) -> Option<Pin> {
    let at = n.child("at")?;
    let pos = point(at.num(0)?, at.num(1)?, false);
    let raw_name = n.child("name").and_then(|c| c.arg(0)).unwrap_or("");
    let number = n.child("number").and_then(|c| c.arg(0)).unwrap_or("").to_string();
    let has_name = !raw_name.is_empty() && raw_name != "~";
    let name = if has_name { plain(raw_name) } else { number.clone() };
    let kind = match n.arg(0).unwrap_or("") {
        "input" => PinKind::Input,
        "output" => PinKind::Output,
        "bidirectional" => PinKind::Bidirectional,
        "tri_state" => PinKind::TriState,
        "power_in" => PinKind::PowerIn,
        "power_out" => PinKind::PowerOut,
        "open_collector" | "open_emitter" => PinKind::OpenCollector,
        "no_connect" => PinKind::NotConnected,
        _ => PinKind::Passive,
    };
    Some(Pin {
        id: Id::from_u128(0), // set by the caller
        name,
        number,
        pos,
        direction: Angle::deg(at.num(2).unwrap_or(0.0)),
        length: nm(n.child("length").and_then(|l| l.num(0)).unwrap_or(2.54)),
        kind,
        show_name: has_name && !names_hidden,
        show_number: !numbers_hidden,
    })
}

fn width_of(n: &Node) -> Coord {
    match stroke_width(n) {
        Some(w) if w > 0.0 => nm(w),
        _ => DEFAULT_LINE,
    }
}

fn filled(n: &Node) -> bool {
    n.child("fill").and_then(|f| f.child("type")).and_then(|t| t.arg(0)).is_some_and(|t| t != "none")
}

/// Rectangle, circle, arc, polyline or Bézier → shapes.
fn convert_graphic(n: &Node) -> Vec<Shape> {
    let width = width_of(n);
    let fill = filled(n);
    match n.head() {
        "rectangle" => {
            let (Some(a), Some(b)) = (child_point(n, "start", false), child_point(n, "end", false)) else {
                return Vec::new();
            };
            vec![Shape::Rect { a, b, width, fill }]
        }
        "circle" => {
            let (Some(center), Some(r)) = (child_point(n, "center", false), n.child("radius").and_then(|r| r.num(0)))
            else {
                return Vec::new();
            };
            vec![Shape::Circle { center, radius: nm(r), width, fill }]
        }
        "arc" => {
            let (Some(s), Some(m), Some(e)) =
                (child_point(n, "start", false), child_point(n, "mid", false), child_point(n, "end", false))
            else {
                return Vec::new();
            };
            arc3(s, m, e, width).map_or_else(|| vec![Shape::line(s, e, width)], |a| vec![a])
        }
        "polyline" => polyline(pts(n, false), width, fill),
        "bezier" => polyline(bezier(&pts(n, false)), width, fill),
        _ => Vec::new(),
    }
}

/// Filled or closed polylines become polygons, open ones line segments.
pub fn polyline(mut p: Vec<Point>, width: Coord, fill: bool) -> Vec<Shape> {
    if p.len() < 2 {
        return Vec::new();
    }
    let closed = p.len() > 3 && p.first() == p.last();
    if fill || closed {
        if closed {
            p.pop();
        }
        return vec![Shape::Polygon { points: p, width, fill }];
    }
    p.windows(2).map(|w| Shape::line(w[0], w[1], width)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// KiCad 6 style: bare `hide` flags, multi-unit with a shared power unit, De Morgan.
    const KICAD6: &str = r##"(kicad_symbol_lib (version 20211014) (generator kicad_symbol_editor)
  (symbol "LM358" (pin_names (offset 0.127)) (in_bom yes) (on_board yes)
    (property "Reference" "U" (id 0) (at 0 5.08 0) (effects (font (size 1.27 1.27)) (justify left)))
    (property "Value" "LM358" (id 1) (at 0 -5.08 0) (effects (font (size 1.27 1.27)) (justify left)))
    (property "Footprint" "" (id 2) (at 0 0 0) (effects (font (size 1.27 1.27)) hide))
    (property "Datasheet" "http://www.ti.com/lit/ds/symlink/lm2904-n.pdf" (id 3) (at 0 0 0) (effects (font (size 1.27 1.27)) hide))
    (property "ki_description" "Low-Power, Dual Operational Amplifiers" (id 5) (at 0 0 0) (effects (font (size 1.27 1.27)) hide))
    (property "ki_fp_filters" "SOIC*3.9x4.9mm*P1.27mm* DIP*W7.62mm*" (id 6) (at 0 0 0) (effects (font (size 1.27 1.27)) hide))
    (symbol "LM358_1_1"
      (polyline (pts (xy -5.08 5.08) (xy 5.08 0) (xy -5.08 -5.08) (xy -5.08 5.08)) (stroke (width 0.254) (type default) (color 0 0 0 0)) (fill (type background)))
      (pin output line (at 7.62 0 180) (length 2.54) (name "~" (effects (font (size 1.27 1.27)))) (number "1" (effects (font (size 1.27 1.27)))))
      (pin input line (at -7.62 -2.54 0) (length 2.54) (name "-" (effects (font (size 1.27 1.27)))) (number "2" (effects (font (size 1.27 1.27)))))
      (pin input line (at -7.62 2.54 0) (length 2.54) (name "+" (effects (font (size 1.27 1.27)))) (number "3" (effects (font (size 1.27 1.27)))))
    )
    (symbol "LM358_2_1"
      (polyline (pts (xy -5.08 5.08) (xy 5.08 0) (xy -5.08 -5.08) (xy -5.08 5.08)) (stroke (width 0.254) (type default) (color 0 0 0 0)) (fill (type background)))
      (pin input line (at -7.62 2.54 0) (length 2.54) (name "+" (effects (font (size 1.27 1.27)))) (number "5" (effects (font (size 1.27 1.27)))))
      (pin input line (at -7.62 -2.54 0) (length 2.54) (name "-" (effects (font (size 1.27 1.27)))) (number "6" (effects (font (size 1.27 1.27)))))
      (pin output line (at 7.62 0 180) (length 2.54) (name "~" (effects (font (size 1.27 1.27)))) (number "7" (effects (font (size 1.27 1.27)))))
    )
    (symbol "LM358_2_2"
      (pin output line (at 7.62 0 180) (length 2.54) (name "~" (effects (font (size 1.27 1.27)))) (number "7" (effects (font (size 1.27 1.27)))))
    )
    (symbol "LM358_3_1"
      (pin power_in line (at -2.54 -7.62 90) (length 3.81) (name "V-" (effects (font (size 1.27 1.27)))) (number "4" (effects (font (size 1.27 1.27)))))
      (pin power_in line (at -2.54 7.62 270) (length 3.81) (name "V+" (effects (font (size 1.27 1.27)))) (number "8" (effects (font (size 1.27 1.27)))))
    )
  )
  (symbol "LM2904" (extends "LM358")
    (property "Reference" "U" (id 0) (at 0 5.08 0) (effects (font (size 1.27 1.27)) (justify left)))
    (property "Value" "LM2904" (id 1) (at 0 -5.08 0) (effects (font (size 1.27 1.27)) (justify left)))
    (property "Footprint" "Package_SO:SOIC-8_3.9x4.9mm_P1.27mm" (id 2) (at 0 0 0) (effects (font (size 1.27 1.27)) hide))
  )
  (symbol "GND" (power) (pin_names (offset 0)) (in_bom yes) (on_board yes)
    (property "Reference" "#PWR" (id 0) (at 0 -6.35 0) (effects (font (size 1.27 1.27)) hide))
    (property "Value" "GND" (id 1) (at 0 -3.81 0) (effects (font (size 1.27 1.27))))
    (symbol "GND_0_1" (polyline (pts (xy 0 0) (xy 0 -1.27) (xy 1.27 -1.27) (xy 0 -2.54) (xy -1.27 -1.27) (xy 0 -1.27)) (stroke (width 0) (type default) (color 0 0 0 0)) (fill (type none))))
    (symbol "GND_1_1" (pin power_in line (at 0 0 270) (length 0) hide (name "GND" (effects (font (size 1.27 1.27)))) (number "1" (effects (font (size 1.27 1.27))))))
  )
)"##;

    /// KiCad 10 style: `(hide yes)`, pin number visibility, arcs, circles, text.
    const KICAD10: &str = r##"(kicad_symbol_lib (version 20251024) (generator "kicad_symbol_editor")
  (symbol "R" (pin_numbers (hide yes)) (pin_names (offset 0) (hide yes)) (exclude_from_sim no) (in_bom yes) (on_board yes)
    (property "Reference" "R" (at 2.032 0 90) (show_name no) (effects (font (size 1.27 1.27))))
    (property "Value" "R" (at 0 0 90) (effects (font (size 1.27 1.27))))
    (property "Footprint" "" (at -1.778 0 90) (hide yes) (effects (font (size 1.27 1.27))))
    (property "Datasheet" "~" (at 0 0 0) (hide yes) (effects (font (size 1.27 1.27))))
    (property "Description" "Resistor" (at 0 0 0) (hide yes) (effects (font (size 1.27 1.27))))
    (property "ki_keywords" "R res resistor" (at 0 0 0) (hide yes) (effects (font (size 1.27 1.27))))
    (property "ki_fp_filters" "R_*" (at 0 0 0) (hide yes) (effects (font (size 1.27 1.27))))
    (symbol "R_0_1"
      (rectangle (start -1.016 -2.54) (end 1.016 2.54) (stroke (width 0.254) (type default)) (fill (type none)))
      (arc (start 0 3) (mid 1 2) (end 0 1) (stroke (width 0) (type default)) (fill (type none)))
      (circle (center 0 0) (radius 0.5) (stroke (width 0.1) (type default)) (fill (type outline)))
      (text "~{X}" (at 0 -3 900) (effects (font (size 1 1))))
    )
    (symbol "R_1_1"
      (pin passive line (at 0 3.81 270) (length 1.27) (name "~" (effects (font (size 1.27 1.27)))) (number "1" (effects (font (size 1.27 1.27)))))
      (pin passive line (at 0 -3.81 90) (length 1.27) (name "~" (effects (font (size 1.27 1.27)))) (number "2" (effects (font (size 1.27 1.27)))))
    )
  )
)"##;

    fn load(text: &str, name: &str) -> KiSymbol {
        let syms = parse_library(text).unwrap();
        let node = &syms.iter().find(|s| s.0 == name).unwrap().1;
        let parent =
            node.child("extends").and_then(|e| e.arg(0)).map(|p| syms.iter().find(|s| s.0 == p).unwrap().1.clone());
        convert("Lib", node, parent.as_ref())
    }

    #[test]
    fn multi_unit_symbol_kicad6() {
        let s = load(KICAD6, "LM358");
        assert_eq!(s.reference, "U");
        assert_eq!(s.description, "Low-Power, Dual Operational Amplifiers");
        assert_eq!(s.footprint, None);
        assert_eq!(s.fp_filters.len(), 2);
        assert_eq!(s.units.len(), 3);
        assert_eq!(s.units[0].name, "LM358 A");
        assert_eq!(s.unit_names, ["A", "B", "C"]);
        // The De Morgan body of unit 2 is skipped.
        assert_eq!(s.units[1].pins.len(), 3);
        assert!(s.warnings.iter().any(|w| w.contains("De Morgan")));
        let out = s.units[0].pins.iter().find(|p| p.number == "1").unwrap();
        assert_eq!(out.name, "1");
        assert!(!out.show_name);
        assert_eq!(out.pos, Point::new(7_620_000, 0));
        assert_eq!(out.direction.degrees(), 180.0);
        assert_eq!(out.kind, PinKind::Output);
        let plus = s.units[0].pins.iter().find(|p| p.number == "3").unwrap();
        assert_eq!(plus.name, "+");
        assert!(plus.show_name && plus.show_number);
        assert_eq!(s.units[2].pins.iter().filter(|p| p.kind == PinKind::PowerIn).count(), 2);
        // Filled triangle body, reference and value texts.
        assert!(matches!(&s.units[0].graphics[0], Shape::Polygon { points, fill: true, .. } if points.len() == 3));
        assert_eq!(s.units[0].texts.iter().filter(|t| t.text.starts_with("${")).count(), 2);
    }

    #[test]
    fn derived_and_power_symbols() {
        let d = load(KICAD6, "LM2904");
        let base = load(KICAD6, "LM358");
        assert_eq!(d.name, "LM2904");
        assert_eq!(d.value, "LM2904");
        assert_eq!(d.footprint.as_deref(), Some("Package_SO:SOIC-8_3.9x4.9mm_P1.27mm"));
        assert_eq!(d.datasheet, base.datasheet, "inherited");
        assert_eq!(d.units.len(), 3);
        assert_ne!(d.units[0].id, base.units[0].id);
        assert_eq!(d.units[0].pins.len(), 3);
        assert!(load(KICAD6, "GND").power);
    }

    #[test]
    fn single_unit_symbol_kicad10() {
        let s = load(KICAD10, "R");
        assert_eq!(s.units.len(), 1);
        let u = &s.units[0];
        assert_eq!(u.name, "R");
        assert_eq!(s.datasheet, "");
        assert_eq!(s.description, "Resistor");
        assert_eq!(s.keywords, "R res resistor");
        assert!(u.pins.iter().all(|p| !p.show_name && !p.show_number && p.number == p.name));
        assert!(matches!(u.graphics[0], Shape::Rect { width: 254_000, fill: false, .. }));
        assert!(matches!(u.graphics[1], Shape::Arc { width: DEFAULT_LINE, .. }));
        assert!(matches!(u.graphics[2], Shape::Circle { fill: true, .. }));
        let x = u.texts.iter().find(|t| t.text == "X").unwrap();
        assert_eq!(x.rotation.degrees(), 90.0, "text angles are in tenths of a degree");
        let r = u.texts.iter().find(|t| t.text == "${REF}").unwrap();
        assert_eq!(r.rotation.degrees(), 90.0);
        assert_eq!(r.pos, Point::new(2_032_000, 0));
        assert_eq!(u.pins.len(), 2);
        assert_eq!(s.units[0].id, load(KICAD10, "R").units[0].id, "stable ids");
    }
}
