//! Turn KiCad symbols and footprints into library objects: symbols, footprints and parts
//! (a symbol with one variant per chosen footprint, pads mapped to pins by number).

use std::collections::BTreeMap;

use model::*;

use crate::catalog::{Catalog, Error};
use crate::common::kid;
use crate::footprint::KiFootprint;
use crate::symbol::KiSymbol;

/// What to import: symbols (with the footprints to make parts with) and lone footprints.
#[derive(Clone, Debug, Default)]
pub struct Request {
    /// `Lib:Name` of a symbol and the `Lib:Name`s of its footprints. Without footprints
    /// the part has no variant: its components stay off the board.
    pub symbols: Vec<(String, Vec<String>)>,
    pub footprints: Vec<String>,
}

/// Library objects ready to be inserted, plus notes for the user.
#[derive(Clone, Debug, Default)]
pub struct Imported {
    pub symbols: Vec<Symbol>,
    pub footprints: Vec<Footprint>,
    pub parts: Vec<Part>,
    /// `Lib:Name: message`.
    pub warnings: Vec<String>,
    /// Requested items that failed to load.
    pub errors: Vec<String>,
}

impl Imported {
    fn add_footprint(&mut self, fp: &KiFootprint) {
        if !self.footprints.iter().any(|f| f.id == fp.footprint.id) {
            self.footprints.push(fp.footprint.clone());
            let name = fp.full_name();
            self.warnings.extend(fp.warnings.iter().map(|w| format!("{name}: {w}")));
        }
    }
}

/// Load everything in `req` from the catalog.
pub fn import(catalog: &mut Catalog, req: &Request) -> Imported {
    let mut out = Imported::default();
    let mut loaded: BTreeMap<String, Result<KiFootprint, String>> = BTreeMap::new();
    let mut footprint = |catalog: &Catalog, name: &str| {
        loaded
            .entry(name.to_string())
            .or_insert_with(|| catalog.load_footprint(name).map_err(|e| e.to_string()))
            .clone()
    };
    for (sym_name, fps) in &req.symbols {
        let sym = match catalog.load_symbol(sym_name) {
            Ok(s) => s,
            Err(e) => {
                out.errors.push(e.to_string());
                continue;
            }
        };
        if sym.power {
            out.errors.push(format!("{sym_name}: power symbols are net labels in this application"));
            continue;
        }
        let mut variants = Vec::new();
        for f in fps {
            match footprint(catalog, f) {
                Ok(fp) => {
                    out.add_footprint(&fp);
                    variants.push(fp);
                }
                Err(e) => out.errors.push(e),
            }
        }
        out.warnings.extend(sym.warnings.iter().map(|w| format!("{sym_name}: {w}")));
        for u in &sym.units {
            if !out.symbols.iter().any(|s| s.id == u.id) {
                out.symbols.push(u.clone());
            }
        }
        if fps.is_empty() {
            out.warnings.push(format!("{sym_name}: no footprint; its components aren't placed on the board"));
        }
        // A part unless all of its footprints failed to load (reported above).
        if fps.is_empty() || !variants.is_empty() {
            out.parts.push(make_part(&sym, &variants));
        }
    }
    for f in &req.footprints {
        match footprint(catalog, f) {
            Ok(fp) => out.add_footprint(&fp),
            Err(e) => out.errors.push(e),
        }
    }
    out
}

/// A part from a converted symbol and its footprints.
pub fn make_part(sym: &KiSymbol, footprints: &[KiFootprint]) -> Part {
    let units: Vec<PartUnit> =
        sym.units.iter().zip(&sym.unit_names).map(|(s, n)| PartUnit { name: n.clone(), symbol: s.id }).collect();
    let variants = footprints
        .iter()
        .map(|fp| PartVariant {
            id: kid(&["kicad-variant", &sym.lib, &sym.name, &fp.full_name()]),
            name: fp.footprint.name.clone(),
            footprint: fp.footprint.id,
            connections: connect(sym, &fp.footprint),
        })
        .collect();
    let mut properties = BTreeMap::new();
    properties.insert("KiCad symbol".to_string(), sym.full_name());
    if !sym.datasheet.is_empty() {
        properties.insert("Datasheet".to_string(), sym.datasheet.clone());
    }
    if !sym.keywords.is_empty() {
        properties.insert("Keywords".to_string(), sym.keywords.clone());
    }
    Part {
        id: kid(&["kicad-part", &sym.lib, &sym.name]),
        name: sym.name.clone(),
        description: sym.description.clone(),
        prefix: if sym.reference.is_empty() { "U".into() } else { sym.reference.clone() },
        default_value: sym.value.clone(),
        units,
        variants,
        properties,
        source: None,
    }
}

/// Pads to pins with the same number. Several pads may share a number (thermal pads
/// split into pieces); they all connect to that pin.
fn connect(sym: &KiSymbol, fp: &Footprint) -> Vec<PadConnection> {
    let mut by_number: BTreeMap<&str, (u32, Id)> = BTreeMap::new();
    for (u, unit) in sym.units.iter().enumerate() {
        for pin in unit.pins.iter() {
            by_number.entry(pin.key()).or_insert((u as u32, pin.id));
        }
    }
    let mut out = Vec::new();
    for pad in fp.pads.iter() {
        if let Some(&(unit, pin)) = by_number.get(pad.name.as_str()) {
            out.push(PadConnection { pad: pad.id, unit, pin });
        }
    }
    out
}

/// Distinct pin numbers of a symbol (all units), sorted.
pub fn pin_numbers(sym: &KiSymbol) -> Vec<&str> {
    let mut keys: Vec<&str> = sym.units.iter().flat_map(|u| u.pins.iter().map(|p| p.key())).collect();
    keys.sort_unstable();
    keys.dedup();
    keys
}

/// The footprints suggested for a symbol: its default footprint if it exists, otherwise
/// the best footprint matching its filters.
pub fn suggested_footprints(catalog: &mut Catalog, sym: &KiSymbol) -> Vec<String> {
    if let Some(f) = &sym.footprint
        && catalog.has_footprint(f)
    {
        return vec![f.clone()];
    }
    catalog.best_footprint(&sym.fp_filters, &pin_numbers(sym)).into_iter().collect()
}

/// Load a symbol and its default footprints in one go (used by tests and the MCP tool).
pub fn import_with_defaults(catalog: &mut Catalog, symbols: &[&str]) -> Result<Imported, Error> {
    let mut req = Request::default();
    for s in symbols {
        let sym = catalog.load_symbol(s)?;
        let fps = suggested_footprints(catalog, &sym);
        req.symbols.push((s.to_string(), fps));
    }
    Ok(import(catalog, &req))
}
