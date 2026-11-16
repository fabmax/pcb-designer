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
    let Some(fp) = variant.footprint.and_then(|f| project.library.footprints.get(f)) else {
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
    /// The project's copy was edited since it was copied.
    ModifiedLocally,
    /// Both changed: updating would drop the project's edits.
    BothChanged,
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

/// Project library objects nothing uses: parts without components, then symbols and
/// footprints no remaining part uses. In this order, one `Delete` removes them all.
pub fn unused_library_objects(project: &Project) -> Vec<ObjectId> {
    let lib = &project.library;
    let used = |p: &&model::Part| project.schematic.components.iter().any(|c| c.part == p.id);
    let kept: Vec<&model::Part> = lib.parts.iter().filter(used).collect();
    let mut out: Vec<ObjectId> = lib.parts.iter().filter(|p| !used(p)).map(|p| ObjectId::LibPart(p.id)).collect();
    out.extend(
        lib.symbols
            .iter()
            .filter(|s| !kept.iter().any(|p| p.units.iter().any(|u| u.symbol == s.id)))
            .map(|s| ObjectId::LibSymbol(s.id)),
    );
    out.extend(
        lib.footprints
            .iter()
            .filter(|f| !kept.iter().any(|p| p.variants.iter().any(|v| v.footprint == Some(f.id))))
            .map(|f| ObjectId::LibFootprint(f.id)),
    );
    out
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
            let local = project.library.content_hash(part.id).is_some_and(|h| h != src.content_hash);
            status.state = match libraries[index].content_hash(src.part) {
                None => PartState::RemovedFromLibrary,
                Some(h) => match (h != src.content_hash, local) {
                    (false, false) => PartState::UpToDate,
                    (true, false) => PartState::LibraryChanged,
                    (false, true) => PartState::ModifiedLocally,
                    (true, true) => PartState::BothChanged,
                },
            };
            status
        })
        .collect()
}

/// Positions of the pins of sub-sheet `sheet`'s symbol (on its parent sheet).
pub fn sheet_pin_points(project: &Project, sheet: Id) -> Vec<Point> {
    project.schematic.sheet_symbol(sheet).map(|(_, pins)| pins.into_iter().map(|p| p.pos).collect()).unwrap_or_default()
}

/// The wires (or bus wires) of the chain through `id`: from segment to segment through
/// points where exactly two of them meet, stopping at branches (three or more ends, or an
/// end on the inside of another segment), dangling ends, pins, bus entries and sheet pins.
/// Labels and junctions don't stop it. Includes `id`; empty if `id` isn't a wire.
pub fn wire_chain(project: &Project, id: ObjectId) -> Vec<ObjectId> {
    use crate::rubber_band::Kind;
    let Some((kind, start)) = Kind::of(id) else { return Vec::new() };
    let Some(first) = kind.get(project, start) else { return Vec::new() };
    let sheet = first.sheet;
    let segs: Vec<_> = kind.segments(project).into_iter().filter(|s| s.sheet == sheet).collect();
    let mut ends: std::collections::HashMap<Point, Vec<usize>> = Default::default();
    for (i, s) in segs.iter().enumerate() {
        ends.entry(s.a).or_default().push(i);
        ends.entry(s.b).or_default().push(i);
    }
    // Things that end a chain.
    let s = &project.schematic;
    let mut stops: std::collections::HashSet<Point> = Default::default();
    match kind {
        Kind::Wire => {
            for i in s.symbols.iter().filter(|i| i.sheet == sheet) {
                stops.extend(pin_positions(project, i.id).into_iter().map(|(_, _, p)| p));
            }
            stops.extend(s.bus_entries.iter().filter(|e| e.sheet == sheet).map(|e| e.wire_end()));
            for sub in s.sheets.iter().filter(|x| x.parent == Some(sheet)) {
                stops.extend(sheet_pin_points(project, sub.id));
            }
        }
        Kind::Bus => stops.extend(s.bus_entries.iter().filter(|e| e.sheet == sheet).map(|e| e.pos)),
    }
    let passes = |q: Point| {
        !stops.contains(&q)
            && ends[&q].len() == 2
            && !segs.iter().any(|o| crate::connectivity::on_segment_inside(q, o.a, o.b))
    };
    let Some(first) = segs.iter().position(|s| s.id == start) else { return Vec::new() };
    let mut seen = vec![false; segs.len()];
    seen[first] = true;
    let mut work = vec![first];
    let mut out = Vec::new();
    while let Some(i) = work.pop() {
        out.push(kind.object_id(segs[i].id));
        for q in [segs[i].a, segs[i].b] {
            if !passes(q) {
                continue;
            }
            for &j in &ends[&q] {
                if !seen[j] {
                    seen[j] = true;
                    work.push(j);
                }
            }
        }
    }
    out
}

/// The tracks of the chain through track `id` on its layer: from track to track through
/// points where exactly two track ends meet, stopping at pads, vias, branches and
/// dangling ends. Includes `id`; empty if there is no such track.
pub fn track_chain(project: &Project, id: Id) -> Vec<Id> {
    let b = &project.board;
    let Some(first) = b.tracks.get(id) else { return Vec::new() };
    let layer = first.layer;
    let bit = crate::copper::layer_bit(layer);
    let tracks: Vec<&Track> = b.tracks.iter().filter(|t| t.layer == layer).collect();
    let mut ends: std::collections::HashMap<Point, Vec<usize>> = Default::default();
    for (i, t) in tracks.iter().enumerate() {
        ends.entry(t.a).or_default().push(i);
        ends.entry(t.b).or_default().push(i);
    }
    // Copper that ends a chain: pads and vias on the layer.
    let mut copper: Vec<(Rect, crate::copper::Geo)> = Vec::new();
    for f in b.footprints.iter() {
        for (_, layers, geo) in crate::copper::footprint_copper(project, f) {
            if layers & bit != 0 {
                copper.push((geo.bounds(), geo));
            }
        }
    }
    for v in b.vias.iter().filter(|v| crate::copper::via_layers(b, v) & bit != 0) {
        let geo = crate::copper::via_geo(v);
        copper.push((geo.bounds(), geo));
    }
    let passes = |q: Point| ends[&q].len() == 2 && !copper.iter().any(|(r, g)| r.contains(q) && g.contains(q));
    let Some(first) = tracks.iter().position(|t| t.id == id) else { return Vec::new() };
    let mut seen = vec![false; tracks.len()];
    seen[first] = true;
    let mut work = vec![first];
    let mut out = Vec::new();
    while let Some(i) = work.pop() {
        out.push(tracks[i].id);
        for q in [tracks[i].a, tracks[i].b] {
            if !passes(q) {
                continue;
            }
            for &j in &ends[&q] {
                if !seen[j] {
                    seen[j] = true;
                    work.push(j);
                }
            }
        }
    }
    out
}

/// World positions of the pins of a placed symbol (cheaper than [`pin_positions`]).
pub fn pin_points(project: &Project, symbol: Id) -> Vec<Point> {
    let Some(inst) = project.schematic.symbols.get(symbol) else { return Vec::new() };
    let Some(sym) = symbol_of_instance(project, inst) else { return Vec::new() };
    let t = inst.transform();
    sym.pins.iter().map(|p| t.point(p.pos)).collect()
}
