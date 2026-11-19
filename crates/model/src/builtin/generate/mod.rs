//! The generated part of the Built-in library: families of parts built from package
//! dimensions. Footprints follow IPC-7351B nominal land patterns where the package is a
//! standard one (chips, SOD, SOT, SOIC, DIP, TO); vendor-specific packages say so in their
//! description. After a change, run `cargo run -p model --example gen_builtin` (a test
//! fails until the bundled file matches).
//!
//! Ids are hashed from names (`part/NAME`, `part/NAME/variant/V`, `sym/NAME/pin/N`,
//! `fp/NAME/pad/I`), so renaming an item gives it new ids.

mod connectors;
mod fp;
mod misc;
mod passives;
mod semis;
pub mod sims;
mod sym;

use std::collections::HashMap;

use super::{LIBRARY_ID, gid, is_generated};
use crate::id::Id;
use crate::library::*;
use crate::store::Store;

/// The generated items.
pub fn library() -> Library {
    let mut b = Builder::default();
    passives::add(&mut b);
    semis::add(&mut b);
    connectors::add(&mut b);
    misc::add(&mut b);
    sims::add(&mut b);
    b.lib
}

/// `file` with its generated items replaced by [`library`]'s: generated ones first, then
/// the hand-made ones in their order.
pub fn merge(file: &Library) -> Library {
    let mut out = library();
    out.id = Some(LIBRARY_ID);
    out.name = "Built-in".into();
    for s in file.symbols.iter().filter(|s| !is_generated(s.id)) {
        out.symbols.insert(s.clone());
    }
    for f in file.footprints.iter().filter(|f| !is_generated(f.id)) {
        out.footprints.insert(f.clone());
    }
    for p in file.parts.iter().filter(|p| !is_generated(p.id)) {
        out.parts.insert(p.clone());
    }
    out
}

/// How a variant's pads connect to the symbol's pins.
pub(super) enum Map<'a> {
    /// Each pad to the pin numbered as the pad's name (pads without such a pin stay
    /// unconnected).
    ByNumber,
    /// (pad name, pin number) pairs; a pad name may repeat (a tab and its pin).
    Pairs(&'a [(&'a str, &'a str)]),
}

pub(super) struct Variant<'a> {
    pub name: String,
    pub footprint: String,
    pub map: Map<'a>,
}

pub(super) fn variant<'a>(name: impl Into<String>, footprint: impl Into<String>, map: Map<'a>) -> Variant<'a> {
    Variant { name: name.into(), footprint: footprint.into(), map }
}

pub(super) struct PartSpec<'a> {
    pub name: String,
    /// The part's and its symbol's.
    pub category: &'a str,
    pub description: String,
    pub prefix: &'a str,
    pub value: String,
    /// `None`: a board part (no symbol).
    pub symbol: Option<&'a str>,
    pub variants: Vec<Variant<'a>>,
}

pub(super) struct Builder {
    lib: Library,
    symbols: HashMap<String, Id>,
    footprints: HashMap<String, Id>,
}

impl Default for Builder {
    fn default() -> Self {
        Builder {
            lib: Library {
                id: Some(LIBRARY_ID),
                name: "Built-in".into(),
                symbols: Store::default(),
                footprints: Store::default(),
                parts: Store::default(),
            },
            symbols: HashMap::new(),
            footprints: HashMap::new(),
        }
    }
}

impl Builder {
    /// Adds a symbol ([`sym::Sym::finish`] gave it its ids); a name already there is kept.
    pub fn symbol(&mut self, s: Symbol) {
        if self.symbols.contains_key(&s.name) {
            return;
        }
        self.symbols.insert(s.name.clone(), s.id);
        self.lib.symbols.insert(s);
    }

    /// Adds a footprint ([`fp::Fp::finish`] gave it its ids); a name already there is kept.
    pub fn footprint(&mut self, f: Footprint) {
        if self.footprints.contains_key(&f.name) {
            return;
        }
        self.footprints.insert(f.name.clone(), f.id);
        self.lib.footprints.insert(f);
    }

    pub fn part(&mut self, spec: PartSpec) {
        let symbol = spec.symbol.map(|name| {
            let id = *self.symbols.get(name).unwrap_or_else(|| panic!("no symbol {name}"));
            self.lib.symbols.get(id).unwrap().clone()
        });
        let pin_of = |number: &str| -> Option<Id> {
            symbol.as_ref().and_then(|s| s.pins.iter().find(|p| p.key() == number)).map(|p| p.id)
        };
        let variants = spec
            .variants
            .iter()
            .map(|v| {
                let fid = *self.footprints.get(&v.footprint).unwrap_or_else(|| panic!("no footprint {}", v.footprint));
                let fp = self.lib.footprints.get(fid).unwrap();
                let connections = match &v.map {
                    Map::ByNumber => fp
                        .pads
                        .iter()
                        .filter_map(|pad| pin_of(&pad.name).map(|pin| PadConnection { pad: pad.id, unit: 0, pin }))
                        .collect(),
                    Map::Pairs(pairs) => {
                        let mut out = Vec::new();
                        for &(pad_name, number) in pairs.iter() {
                            let pin =
                                pin_of(number).unwrap_or_else(|| panic!("{} {}: no pin {number}", spec.name, v.name));
                            let pads: Vec<_> = fp.pads.iter().filter(|p| p.name == pad_name).collect();
                            assert!(!pads.is_empty(), "{} {}: no pad {pad_name}", spec.name, v.name);
                            out.extend(pads.iter().map(|p| PadConnection { pad: p.id, unit: 0, pin }));
                        }
                        out
                    }
                };
                PartVariant {
                    id: gid(&format!("part/{}/variant/{}", spec.name, v.name)),
                    name: v.name.clone(),
                    footprint: Some(fid),
                    connections,
                }
            })
            .collect();
        if let Some(s) = symbol.as_ref().and_then(|s| self.lib.symbols.get_mut(s.id)) {
            s.category = spec.category.into();
        }
        self.lib.parts.insert(Part {
            sim: Default::default(),
            id: gid(&format!("part/{}", spec.name)),
            name: spec.name.clone(),
            category: spec.category.into(),
            description: spec.description,
            prefix: spec.prefix.into(),
            default_value: spec.value,
            units: symbol.iter().map(|s| PartUnit { name: "A".into(), symbol: s.id }).collect(),
            variants,
            properties: Default::default(),
            source: None,
        });
    }
}
