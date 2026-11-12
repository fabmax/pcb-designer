//! Library content: symbols, footprints and parts.
//!
//! A [`Part`] combines one symbol per unit with footprint variants (none for parts that
//! aren't on the board, like power flags or logos). Each variant maps footprint pads to
//! symbol pins, so symbol and footprint can be designed independently and connected
//! afterwards. A pin may map to several pads (e.g. GND), a pad
//! to at most one pin. Pads without a pin are not connected.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::geom::{Angle, Point, Shape, Size, Text};
use crate::id::Id;
use crate::store::Store;
use crate::units::Coord;

macro_rules! has_id {
    ($($t:ty),*) => {$(
        impl $crate::store::HasId for $t {
            fn id(&self) -> $crate::id::Id {
                self.id
            }
        }
    )*};
}
pub(crate) use has_id;

// ---------------------------------------------------------------------------------------
// Symbols

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PinKind {
    Input,
    Output,
    Bidirectional,
    TriState,
    #[default]
    Passive,
    PowerIn,
    PowerOut,
    OpenCollector,
    NotConnected,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Pin {
    pub id: Id,
    pub name: String,
    /// Pin number from the datasheet ("1", "A3"); optional. Pads are connected to pins
    /// through the part's mapping, and "map by name" matches pad names against it.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub number: String,
    /// Connection point (outer end of the pin).
    pub pos: Point,
    /// Direction from the connection point toward the symbol body.
    pub direction: Angle,
    pub length: Coord,
    #[serde(default)]
    pub kind: PinKind,
    #[serde(default = "yes")]
    pub show_name: bool,
    #[serde(default = "yes")]
    pub show_number: bool,
}

fn yes() -> bool {
    true
}

impl Pin {
    /// The name pads are matched against: the number if there is one, else the name.
    pub fn key(&self) -> &str {
        if self.number.is_empty() { &self.name } else { &self.number }
    }

    /// Where the pin meets the symbol body.
    pub fn inner_end(&self) -> Point {
        self.pos + Point::new(self.length, 0).rotated(self.direction)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Symbol {
    pub id: Id,
    pub name: String,
    #[serde(default)]
    pub graphics: Vec<Shape>,
    #[serde(default)]
    pub texts: Vec<Text>,
    pub pins: Store<Pin>,
}

// ---------------------------------------------------------------------------------------
// Footprints

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PadShape {
    Circle,
    Rect,
    /// Corner radius as a fraction of the shorter side (0 … 0.5).
    RoundRect {
        ratio: f64,
    },
    Oval,
    /// Custom outline relative to the pad origin.
    Polygon {
        points: Vec<Point>,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PadKind {
    /// Surface mount pad on the footprint's side.
    Smd,
    /// Plated through hole on all copper layers.
    ThroughHole { drill: Coord },
    /// Non-plated hole (mounting holes).
    NonPlatedHole { drill: Coord },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Pad {
    pub id: Id,
    /// Pad number/name as printed in datasheets ("1", "A3", "EP").
    pub name: String,
    pub pos: Point,
    #[serde(default)]
    pub rotation: Angle,
    pub size: Size,
    pub shape: PadShape,
    pub kind: PadKind,
}

/// Layers available inside a footprint (defined on the top side; mirrored when placed on
/// the bottom).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum FootprintLayer {
    Copper,
    Silkscreen,
    SolderMask,
    SolderPaste,
    Fabrication,
    Courtyard,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct FootprintGraphic {
    pub layer: FootprintLayer,
    pub shape: Shape,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct FootprintText {
    pub layer: FootprintLayer,
    pub text: Text,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Footprint {
    pub id: Id,
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub pads: Store<Pad>,
    #[serde(default)]
    pub graphics: Vec<FootprintGraphic>,
    #[serde(default)]
    pub texts: Vec<FootprintText>,
}

// ---------------------------------------------------------------------------------------
// Parts

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct PartUnit {
    pub name: String,
    pub symbol: Id,
}

/// Connects a footprint pad to a pin of one of the part's units.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, schemars::JsonSchema)]
pub struct PadConnection {
    pub pad: Id,
    pub unit: u32,
    pub pin: Id,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct PartVariant {
    pub id: Id,
    pub name: String,
    pub footprint: Id,
    pub connections: Vec<PadConnection>,
}

/// Where an embedded part came from; used by "update from library".
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct PartSource {
    pub library: Id,
    pub library_name: String,
    pub part: Id,
    /// Hash of the source part content when it was copied.
    pub content_hash: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Part {
    pub id: Id,
    pub name: String,
    #[serde(default)]
    pub description: String,
    /// Reference designator prefix ("R", "U", …).
    pub prefix: String,
    #[serde(default)]
    pub default_value: String,
    pub units: Vec<PartUnit>,
    pub variants: Vec<PartVariant>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub properties: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<PartSource>,
}

impl Part {
    pub fn variant(&self, id: Id) -> Option<&PartVariant> {
        self.variants.iter().find(|v| v.id == id)
    }

    /// Whether a component of this part may use `variant`: one of its variants, or `None`
    /// (no footprint).
    pub fn has_variant(&self, variant: Option<Id>) -> bool {
        variant.is_none_or(|v| self.variant(v).is_some())
    }
}

has_id!(Pin, Symbol, Pad, Footprint, Part);

/// Compare names so that embedded numbers sort by value ("2" < "10", "A2" < "A10").
/// Other text compares case-insensitively; ties fall back to a byte comparison.
/// Doesn't allocate, so it's fine as a sort comparator.
pub fn natural_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    /// Runs of digits and of other characters, flagged `true` for digits.
    fn chunks(s: &str) -> impl Iterator<Item = (bool, &str)> {
        let mut rest = s;
        std::iter::from_fn(move || {
            let digit = rest.as_bytes().first()?.is_ascii_digit();
            let end = rest.bytes().position(|c| c.is_ascii_digit() != digit).unwrap_or(rest.len());
            let (chunk, tail) = rest.split_at(end);
            rest = tail;
            Some((digit, chunk))
        })
    }
    let (mut ca, mut cb) = (chunks(a), chunks(b));
    loop {
        let o = match (ca.next(), cb.next()) {
            (None, None) => return a.cmp(b),
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some((true, x)), Some((true, y))) => {
                let (x, y) = (x.trim_start_matches('0'), y.trim_start_matches('0'));
                x.len().cmp(&y.len()).then_with(|| x.cmp(y))
            }
            (Some((_, x)), Some((_, y))) => {
                x.chars().flat_map(char::to_lowercase).cmp(y.chars().flat_map(char::to_lowercase))
            }
        };
        if o.is_ne() {
            return o;
        }
    }
}

/// A library part with the symbols and footprints it references: what importing or
/// updating a part copies into a project ([`Library::bundles`]).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct PartBundle {
    pub part: Part,
    pub symbols: Vec<Symbol>,
    pub footprints: Vec<Footprint>,
}

/// A collection of symbols, footprints and parts. Used both for library files and for the
/// copies embedded in a project.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Library {
    pub id: Option<Id>,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub symbols: Store<Symbol>,
    #[serde(default)]
    pub footprints: Store<Footprint>,
    #[serde(default)]
    pub parts: Store<Part>,
}

impl Library {
    pub fn new(name: impl Into<String>) -> Self {
        Library { id: Some(Id::new()), name: name.into(), ..Default::default() }
    }

    /// Footprint of a part variant (none without a variant).
    pub fn footprint_of(&self, part: Id, variant: Option<Id>) -> Option<&Footprint> {
        let v = self.parts.get(part)?.variant(variant?)?;
        self.footprints.get(v.footprint)
    }

    /// A part together with the symbols and footprints it references.
    pub fn extract_part(&self, part: Id) -> Option<(Part, Vec<Symbol>, Vec<Footprint>)> {
        let p = self.parts.get(part)?.clone();
        let mut symbols: Vec<Symbol> = Vec::new();
        for u in &p.units {
            if !symbols.iter().any(|s| s.id == u.symbol) {
                symbols.push(self.symbols.get(u.symbol)?.clone());
            }
        }
        let mut footprints: Vec<Footprint> = Vec::new();
        for v in &p.variants {
            if !footprints.iter().any(|f| f.id == v.footprint) {
                footprints.push(self.footprints.get(v.footprint)?.clone());
            }
        }
        Some((p, symbols, footprints))
    }

    /// The parts `ids` with what they reference, ready to copy into a project: each part's
    /// `source` names this library (none if the library has no id). Errors with the
    /// first id that isn't a part here (or references something missing).
    pub fn bundles(&self, ids: &[Id]) -> Result<Vec<PartBundle>, Id> {
        ids.iter()
            .map(|&id| {
                let (mut part, symbols, footprints) = self.extract_part(id).ok_or(id)?;
                part.source = self.id.map(|library| PartSource {
                    library,
                    library_name: self.name.clone(),
                    part: id,
                    content_hash: self.content_hash(id).unwrap_or_default(),
                });
                Ok(PartBundle { part, symbols, footprints })
            })
            .collect()
    }

    /// Stable hash of a part's content (part, symbols, footprints; excluding provenance).
    /// Used to detect library changes for "update from library".
    pub fn content_hash(&self, part: Id) -> Option<String> {
        let (mut p, symbols, footprints) = self.extract_part(part)?;
        p.source = None;
        crate::Fnv1a::of_json(&(p, symbols, footprints)).ok()
    }

    /// Connect pads to pins with the same number (or name, for pins without a number),
    /// case-insensitive: pad "1" ↔ pin number "1", pad "GND" ↔ pin "GND".
    /// Pins are searched unit by unit; each pad maps to at most one pin.
    pub fn map_by_name(&self, units: &[PartUnit], footprint: Id) -> Vec<PadConnection> {
        let Some(fp) = self.footprints.get(footprint) else { return Vec::new() };
        let mut out = Vec::new();
        for pad in fp.pads.iter() {
            let found = units.iter().enumerate().find_map(|(u, unit)| {
                let sym = self.symbols.get(unit.symbol)?;
                let pin = sym.pins.iter().find(|p| p.key().eq_ignore_ascii_case(&pad.name))?;
                Some(PadConnection { pad: pad.id, unit: u as u32, pin: pin.id })
            });
            out.extend(found);
        }
        out
    }

    /// Connect pins (in unit and drawing order) to pads (in natural name order: 1, 2, …, 10).
    pub fn map_in_order(&self, units: &[PartUnit], footprint: Id) -> Vec<PadConnection> {
        let Some(fp) = self.footprints.get(footprint) else { return Vec::new() };
        let mut pads: Vec<&Pad> = fp.pads.iter().collect();
        pads.sort_by(|a, b| natural_cmp(&a.name, &b.name));
        let pins = units.iter().enumerate().flat_map(|(u, unit)| {
            self.symbols.get(unit.symbol).into_iter().flat_map(move |s| s.pins.iter().map(move |p| (u as u32, p.id)))
        });
        pads.into_iter().zip(pins).map(|(pad, (unit, pin))| PadConnection { pad: pad.id, unit, pin }).collect()
    }

    /// Consistency problems (dangling references, bad pad mappings).
    pub fn validate(&self) -> Vec<String> {
        self.issues().into_iter().map(|(_, m)| m).collect()
    }

    /// Problems with the parts, each with the part it concerns.
    pub fn issues(&self) -> Vec<(Id, String)> {
        let mut issues = Vec::new();
        for part in self.parts.iter() {
            for (i, u) in part.units.iter().enumerate() {
                if !self.symbols.contains(u.symbol) {
                    issues.push((part.id, format!("part {:?}: unit {i} references missing symbol", part.name)));
                }
            }
            for v in &part.variants {
                let Some(fp) = self.footprints.get(v.footprint) else {
                    issues.push((part.id, format!("part {:?} variant {:?}: missing footprint", part.name, v.name)));
                    continue;
                };
                let mut seen = std::collections::HashSet::new();
                for c in &v.connections {
                    if !fp.pads.contains(c.pad) {
                        issues.push((part.id, format!("part {:?} variant {:?}: unknown pad", part.name, v.name)));
                    }
                    if !seen.insert(c.pad) {
                        issues.push((part.id, format!("part {:?} variant {:?}: pad mapped twice", part.name, v.name)));
                    }
                    let pin_ok = part
                        .units
                        .get(c.unit as usize)
                        .and_then(|u| self.symbols.get(u.symbol))
                        .is_some_and(|s| s.pins.contains(c.pin));
                    if !pin_ok {
                        issues.push((part.id, format!("part {:?} variant {:?}: unknown pin", part.name, v.name)));
                    }
                }
            }
        }
        issues
    }
}

#[cfg(test)]
mod tests {
    use super::natural_cmp;

    #[test]
    fn natural_order() {
        let mut v = ["R10", "r2", "R2", "+5V", "5V", "A", "R02", "R1a", "R1", "", "GND", "10", "9"];
        v.sort_by(|a, b| natural_cmp(a, b));
        assert_eq!(v, ["", "+5V", "5V", "9", "10", "A", "GND", "R1", "R1a", "R02", "R2", "r2", "R10"]);
    }
}
