//! Library content: symbols, footprints and parts.
//!
//! A [`Part`] combines one symbol per unit with one or more footprint variants. Each variant
//! (except a "no footprint" one, for parts off the board) maps footprint pads to symbol pins, so symbol and footprint can be designed
//! independently and connected afterwards. A pin may map to several pads (e.g. GND), a pad
//! to at most one pin. Pads without a pin are not connected.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::geom::{Angle, Point, Shape, Size, Text};
use crate::id::Id;
use crate::simulation::{SimDevice, SimPin};
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
    /// Plain-text group in library trees ("Passives", "SMD"); empty: none.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub category: String,
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
    ThroughHole { drill: Size },
    /// Non-plated hole (mounting holes).
    NonPlatedHole { drill: Size },
}

impl PadKind {
    /// The drill of a hole: round when width and height are equal, else an oval slot
    /// along the longer side (turning with the pad).
    pub fn drill(&self) -> Option<Size> {
        match self {
            PadKind::Smd => None,
            PadKind::ThroughHole { drill } | PadKind::NonPlatedHole { drill } => Some(*drill),
        }
    }
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
    /// An SMD pad on the opposite side of the footprint (card edge fingers). Holes go
    /// through, so this only matters for SMD pads.
    #[serde(default, skip_serializing_if = "is_false")]
    pub back: bool,
    /// The solder mask opens over the pad (default).
    #[serde(default = "yes", skip_serializing_if = "is_true")]
    pub mask: bool,
    /// SMD pads get solder paste (default); off for test points, fiducials, pads with
    /// paste drawn separately.
    #[serde(default = "yes", skip_serializing_if = "is_true")]
    pub paste: bool,
    /// How far the solder mask opening extends beyond the pad (negative: smaller);
    /// `None`: the board's design rule.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mask_margin: Option<Coord>,
    /// Added to the pad for its paste opening (negative shrinks); `None`: the board's
    /// design rule.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub paste_margin: Option<Coord>,
}

fn is_true(b: &bool) -> bool {
    *b
}

fn is_false(b: &bool) -> bool {
    !*b
}

impl Pad {
    /// The side the pad's copper (SMD) is on, for a footprint placed on `side`.
    pub fn side(&self, side: crate::board::Side) -> crate::board::Side {
        if self.back { side.flipped() } else { side }
    }
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
    /// On the opposite side of the footprint (KiCad's `B.SilkS` in a front footprint).
    #[serde(default, skip_serializing_if = "is_false")]
    pub back: bool,
    pub shape: Shape,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct FootprintText {
    pub layer: FootprintLayer,
    /// On the opposite side of the footprint.
    #[serde(default, skip_serializing_if = "is_false")]
    pub back: bool,
    pub text: Text,
}

impl FootprintGraphic {
    /// Board layer of this graphic for a footprint placed on `side`.
    pub fn board_layer(&self, side: crate::board::Side) -> crate::board::LayerId {
        crate::board::LayerId::for_footprint(self.layer, if self.back { side.flipped() } else { side })
    }
}

impl FootprintText {
    /// The reference or value text (exactly `${REF}` or `${VALUE}`): a template that
    /// placed footprints copy into a [`crate::board::FootprintField`] of their own.
    pub fn is_field(&self) -> bool {
        crate::geom::is_field_text(&self.text.text)
    }

    /// Board layer of this text for a footprint placed on `side`.
    pub fn board_layer(&self, side: crate::board::Side) -> crate::board::LayerId {
        crate::board::LayerId::for_footprint(self.layer, if self.back { side.flipped() } else { side })
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Footprint {
    pub id: Id,
    pub name: String,
    /// Plain-text group in library trees ("Passives", "SMD"); empty: none.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub category: String,
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
    /// `None`: a "no footprint" variant; its components aren't on the board (power flags,
    /// logos, KiCad's "exclude from board"). It has no connections.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub footprint: Option<Id>,
    pub connections: Vec<PadConnection>,
}

impl PartVariant {
    /// Default name of a "no footprint" variant.
    pub const NO_FOOTPRINT: &str = "No footprint";

    /// A "no footprint" variant: its components aren't on the board.
    pub fn no_footprint(id: Id) -> PartVariant {
        PartVariant { id, name: Self::NO_FOOTPRINT.into(), footprint: None, connections: Vec::new() }
    }
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
    /// Plain-text group in library trees ("Passives", "SMD"); empty: none.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub category: String,
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
    /// How the part simulates; `None`: no model (left out of simulations with a warning).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sim: Option<crate::simulation::SimModel>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<PartSource>,
}

impl Part {
    pub fn variant(&self, id: Id) -> Option<&PartVariant> {
        self.variants.iter().find(|v| v.id == id)
    }

    /// A board part has no units (no symbol): it is placed on the board only (mounting
    /// holes, logos, fiducials), and its pads have no net.
    pub fn is_board_part(&self) -> bool {
        self.units.is_empty()
    }

    /// The reference shown on a unit's symbol: `U1-A` when the part has several units
    /// (the unit's name, or its letter when unnamed), else the plain `refdes`.
    pub fn unit_reference(&self, refdes: &str, unit: u32) -> String {
        if self.units.len() < 2 {
            return refdes.to_string();
        }
        let name = self.units.get(unit as usize).map_or("", |u| u.name.as_str());
        let name = if name.is_empty() { unit_letters(unit as usize) } else { name.to_string() };
        format!("{refdes}-{name}")
    }
}

/// Letters of unit `i`: A, B, … Z, AA, AB, …
pub fn unit_letters(i: usize) -> String {
    let mut name = String::new();
    let mut n = i;
    loop {
        name.insert(0, (b'A' + (n % 26) as u8) as char);
        if n < 26 {
            return name;
        }
        n = n / 26 - 1;
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

    /// The categories of parts and symbols (one vocabulary), or of footprints, in
    /// natural order without duplicates.
    pub fn categories(&self, footprints: bool) -> Vec<String> {
        let mut out: Vec<&str> = if footprints {
            self.footprints.iter().map(|f| f.category.as_str()).collect()
        } else {
            self.parts
                .iter()
                .map(|p| p.category.as_str())
                .chain(self.symbols.iter().map(|s| s.category.as_str()))
                .collect()
        };
        out.retain(|c| !c.is_empty());
        out.sort_by(|a, b| crate::natural_cmp(a, b).then_with(|| a.cmp(b)));
        out.dedup();
        out.into_iter().map(str::to_string).collect()
    }

    /// Footprint of a part variant (none for a "no footprint" variant).
    pub fn footprint_of(&self, part: Id, variant: Id) -> Option<&Footprint> {
        let v = self.parts.get(part)?.variant(variant)?;
        self.footprints.get(v.footprint?)
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
        for fp in p.variants.iter().filter_map(|v| v.footprint) {
            if !footprints.iter().any(|f| f.id == fp) {
                footprints.push(self.footprints.get(fp)?.clone());
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

    /// A part's pins in SPICE node order: its model's, or every unit's pins in symbol
    /// order when the model lists none.
    pub fn sim_pins(&self, part: &Part) -> Vec<SimPin> {
        match &part.sim {
            Some(m) if !m.pins.is_empty() => m.pins.clone(),
            _ => part
                .units
                .iter()
                .enumerate()
                .flat_map(|(u, unit)| {
                    self.symbols
                        .get(unit.symbol)
                        .into_iter()
                        .flat_map(move |s| s.pins.iter().map(move |p| SimPin { unit: u as u32, pin: p.id }))
                })
                .collect(),
        }
    }

    /// Pins for `ports` (a model's SPICE ports, in order): by name or number, the usual
    /// aliases (`A` anode, `K` cathode, `C`/`B`/`E`, `D`/`G`/`S`, `+`/`−`, `1`/`2`), then
    /// the unused pins in symbol order for what's left.
    pub fn auto_map_pins(&self, part: &Part, ports: &[String]) -> Vec<SimPin> {
        let pins: Vec<(SimPin, &Pin)> = part
            .units
            .iter()
            .enumerate()
            .flat_map(|(u, unit)| {
                self.symbols
                    .get(unit.symbol)
                    .into_iter()
                    .flat_map(move |s| s.pins.iter().map(move |p| (SimPin { unit: u as u32, pin: p.id }, p)))
            })
            .collect();
        let aliases = |port: &str| -> Vec<&'static str> {
            match port.to_ascii_lowercase().as_str() {
                "a" => vec!["anode", "a", "+"],
                "k" => vec!["cathode", "k", "-", "−"],
                "c" => vec!["collector", "c"],
                "b" => vec!["base", "b"],
                "e" => vec!["emitter", "e"],
                "d" => vec!["drain", "d"],
                "g" => vec!["gate", "g"],
                "s" => vec!["source", "s"],
                "+" => vec!["+", "p", "pos", "plus", "1"],
                "−" | "-" => vec!["-", "−", "n", "neg", "minus", "2"],
                _ => Vec::new(),
            }
        };
        let mut used = std::collections::HashSet::new();
        let mut out: Vec<Option<SimPin>> = ports
            .iter()
            .map(|port| {
                let names = aliases(port);
                let hit = pins.iter().find(|(sp, p)| {
                    !used.contains(sp)
                        && (p.name.eq_ignore_ascii_case(port)
                            || p.number.eq_ignore_ascii_case(port)
                            || names.iter().any(|n| p.name.eq_ignore_ascii_case(n)))
                });
                let hit = hit.or_else(|| {
                    pins.iter()
                        .find(|(sp, p)| !used.contains(sp) && names.iter().any(|n| p.number.eq_ignore_ascii_case(n)))
                });
                hit.map(|(sp, _)| {
                    used.insert(*sp);
                    *sp
                })
            })
            .collect();
        let mut rest = pins.iter().map(|(sp, _)| *sp).filter(|sp| !used.contains(sp));
        for slot in out.iter_mut().filter(|s| s.is_none()) {
            *slot = rest.next();
        }
        out.into_iter().flatten().collect()
    }

    /// Problems of a part's simulation model (none without one, or for board parts).
    pub fn sim_issues(&self, part: &Part) -> Vec<String> {
        let Some(m) = &part.sim else { return Vec::new() };
        // Board parts (no units) are never simulated.
        if m.device == SimDevice::Ignore || part.units.is_empty() {
            return Vec::new();
        }
        let pins = self.sim_pins(part);
        let mut out = m.problems(pins.len());
        let mut seen = std::collections::HashSet::new();
        for p in &m.pins {
            if !seen.insert(*p) {
                out.push("a pin is mapped twice".into());
            }
            let ok = part
                .units
                .get(p.unit as usize)
                .and_then(|u| self.symbols.get(u.symbol))
                .is_some_and(|s| s.pins.contains(p.pin));
            if !ok {
                out.push("a mapped pin isn't on the part".into());
            }
        }
        out.dedup();
        out
    }

    /// Problems with the parts, each with the part it concerns.
    pub fn issues(&self) -> Vec<(Id, String)> {
        let mut issues = Vec::new();
        for part in self.parts.iter() {
            for m in self.sim_issues(part) {
                issues.push((part.id, format!("part {:?}: simulation model: {m}", part.name)));
            }
            for (i, u) in part.units.iter().enumerate() {
                if !self.symbols.contains(u.symbol) {
                    issues.push((part.id, format!("part {:?}: unit {i} references missing symbol", part.name)));
                }
            }
            if part.variants.is_empty() {
                issues.push((part.id, format!("part {:?} has no variant", part.name)));
            }
            for v in &part.variants {
                if part.is_board_part() && v.footprint.is_none() {
                    issues.push((part.id, format!("board part {:?} variant {:?}: no footprint", part.name, v.name)));
                }
                let Some(id) = v.footprint else {
                    if !v.connections.is_empty() {
                        issues.push((
                            part.id,
                            format!("part {:?} variant {:?}: pads without footprint", part.name, v.name),
                        ));
                    }
                    continue;
                };
                let Some(fp) = self.footprints.get(id) else {
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
    use super::*;

    #[test]
    fn unit_references() {
        assert_eq!([0, 1, 25, 26, 27].map(unit_letters), ["A", "B", "Z", "AA", "AB"]);
        let unit = |name: &str| PartUnit { name: name.into(), symbol: Id::new() };
        let mut part = crate::builtin::builtin_library().parts.get(crate::builtin::PART_RESISTOR).unwrap().clone();
        assert_eq!(part.unit_reference("R1", 0), "R1", "a single unit has no suffix");
        part.units = vec![unit("A"), unit(""), unit("Power")];
        assert_eq!(part.unit_reference("U1", 0), "U1-A");
        assert_eq!(part.unit_reference("U1", 1), "U1-B", "unnamed: the unit's letter");
        assert_eq!(part.unit_reference("U1", 2), "U1-Power");
    }

    #[test]
    fn auto_map_finds_the_builtin_models_pins() {
        let lib = crate::builtin::builtin_library();
        let mut checked = 0;
        // Every part with a fixed-port model and up to three pins: the order the model needs.
        for part in lib.parts.iter() {
            let Some(m) = &part.sim else { continue };
            let fixed =
                matches!(m.device, SimDevice::Primitive { .. } | SimDevice::VoltageSource | SimDevice::CurrentSource);
            if !fixed || lib.sim_pins(part).len() > 3 {
                continue;
            }
            let name = &part.name;
            let want = lib.sim_pins(part);
            assert_eq!(lib.auto_map_pins(part, &m.device.ports()), want, "{name}");
            checked += 1;
        }
        assert!(checked >= 6, "{checked}");
    }

    #[test]
    fn natural_order() {
        let mut v = ["R10", "r2", "R2", "+5V", "5V", "A", "R02", "R1a", "R1", "", "GND", "10", "9"];
        v.sort_by(|a, b| natural_cmp(a, b));
        assert_eq!(v, ["", "+5V", "5V", "9", "10", "A", "GND", "R1", "R1a", "R02", "R2", "r2", "R10"]);
    }
}
