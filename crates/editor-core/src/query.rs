//! Read-only queries on a project.

use model::*;

/// World positions of the pins of a placed symbol: `(pin id, pin name, position)`.
pub fn pin_positions(project: &Project, symbol: Id) -> Vec<(Id, String, Point)> {
    let Some(inst) = project.schematic.symbols.get(symbol) else {
        return Vec::new();
    };
    let Some(sym) = symbol_of_instance(project, inst) else {
        return Vec::new();
    };
    let t = inst.transform();
    sym.pins.iter().map(|p| (p.id, p.name.clone(), t.point(p.pos))).collect()
}

/// The library symbol drawn by a symbol instance.
pub fn symbol_of_instance<'a>(project: &'a Project, inst: &SymbolInstance) -> Option<&'a Symbol> {
    let c = project.schematic.components.get(inst.component)?;
    let part = project.library.parts.get(c.part)?;
    project.library.symbols.get(part.units.get(inst.unit as usize)?.symbol)
}

/// The library footprint drawn by a footprint instance.
pub fn footprint_of_instance<'a>(project: &'a Project, inst: &FootprintInstance) -> Option<&'a Footprint> {
    let c = project.schematic.components.get(inst.component)?;
    project.library.footprint_of(c.part, c.variant)
}

/// World positions of the pads of a placed footprint: `(pad id, pad name, position)`.
pub fn pad_positions(project: &Project, footprint: Id) -> Vec<(Id, String, Point)> {
    let Some(inst) = project.board.footprints.get(footprint) else {
        return Vec::new();
    };
    let Some(fp) = footprint_of_instance(project, inst) else {
        return Vec::new();
    };
    let t = inst.transform();
    fp.pads.iter().map(|p| (p.id, p.name.clone(), t.point(p.pos))).collect()
}

/// Pad names connected to each pin of a component, e.g. `"K" → ["1"]`.
pub fn pin_pad_names(project: &Project, component: Id) -> Vec<(String, Vec<String>)> {
    let Some(c) = project.schematic.components.get(component) else {
        return Vec::new();
    };
    let Some(part) = project.library.parts.get(c.part) else {
        return Vec::new();
    };
    let Some(variant) = part.variant(c.variant) else {
        return Vec::new();
    };
    let Some(fp) = project.library.footprints.get(variant.footprint) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for (unit_idx, unit) in part.units.iter().enumerate() {
        let Some(sym) = project.library.symbols.get(unit.symbol) else {
            continue;
        };
        for pin in sym.pins.iter() {
            let pads = variant
                .connections
                .iter()
                .filter(|cn| cn.unit as usize == unit_idx && cn.pin == pin.id)
                .filter_map(|cn| fp.pads.get(cn.pad).map(|p| p.name.clone()))
                .collect();
            out.push((pin.name.clone(), pads));
        }
    }
    out
}

/// How a project part relates to the library it came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PartState {
    /// Same content as the library part.
    UpToDate,
    /// The library part changed since it was copied.
    LibraryChanged,
    /// The source library is not open.
    LibraryMissing,
    /// The library no longer contains the part.
    RemovedFromLibrary,
    /// Not copied from a library (no provenance).
    Local,
}

#[derive(Clone, Debug)]
pub struct PartStatus {
    pub part: Id,
    pub name: String,
    pub library_name: Option<String>,
    pub state: PartState,
    /// Index into `libraries` of the source library, if open.
    pub library: Option<usize>,
}

/// Compare every project part with its source among `libraries`.
pub fn part_statuses(project: &Project, libraries: &[&Library]) -> Vec<PartStatus> {
    project
        .library
        .parts
        .iter()
        .map(|part| {
            let mut status = PartStatus {
                part: part.id,
                name: part.name.clone(),
                library_name: part.source.as_ref().map(|s| s.library_name.clone()),
                state: PartState::Local,
                library: None,
            };
            let Some(src) = &part.source else { return status };
            let Some(index) = libraries.iter().position(|l| l.id == Some(src.library)) else {
                status.state = PartState::LibraryMissing;
                return status;
            };
            status.library = Some(index);
            status.state = match libraries[index].content_hash(src.part) {
                None => PartState::RemovedFromLibrary,
                Some(h) if h == src.content_hash => PartState::UpToDate,
                Some(_) => PartState::LibraryChanged,
            };
            status
        })
        .collect()
}
