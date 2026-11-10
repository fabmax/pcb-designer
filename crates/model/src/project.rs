//! Project: embedded library + schematic + board.

use std::collections::HashMap;
use std::path::PathBuf;

use crate::board::Board;
use crate::id::Id;
use crate::library::Library;
use crate::schematic::Schematic;

#[derive(Clone, Debug, PartialEq)]
pub struct Project {
    pub name: String,
    /// Copies of all parts (with their symbols and footprints) used by the design.
    pub library: Library,
    pub schematic: Schematic,
    pub board: Board,
    /// Library files the project takes parts from (External Libraries), in the order the
    /// user added them. Absolute in memory; `project.json` stores them relative to the
    /// project folder.
    pub libraries: Vec<PathBuf>,
}

impl Project {
    /// Empty project with one sheet and a two layer board.
    pub fn new(name: impl Into<String>) -> Self {
        Project {
            name: name.into(),
            library: Library { name: "Project Library".into(), ..Default::default() },
            schematic: Schematic::new(),
            board: Board::default(),
            libraries: Vec::new(),
        }
    }

    /// Referential integrity problems. An empty list means the project is consistent.
    pub fn validate(&self) -> Vec<String> {
        let mut issues = self.library.validate();
        let sch = &self.schematic;
        let mut footprints_per_component: HashMap<Id, usize> = HashMap::new();
        for f in self.board.footprints.iter() {
            if !sch.components.contains(f.component) {
                issues.push(format!("footprint {:?} references missing component", f.id));
            }
            *footprints_per_component.entry(f.component).or_default() += 1;
        }
        for c in sch.components.iter() {
            match self.library.parts.get(c.part) {
                None => issues.push(format!("component {}: missing part", c.refdes)),
                Some(p) if p.variant(c.variant).is_none() => {
                    issues.push(format!("component {}: missing variant", c.refdes))
                }
                _ => {}
            }
            match footprints_per_component.get(&c.id) {
                None => issues.push(format!("component {}: no footprint on board", c.refdes)),
                Some(n) if *n > 1 => issues.push(format!("component {}: {n} footprints on board", c.refdes)),
                _ => {}
            }
        }
        for s in sch.symbols.iter() {
            if !sch.sheets.contains(s.sheet) {
                issues.push(format!("symbol {:?}: missing sheet", s.id));
            }
            match sch.components.get(s.component) {
                None => issues.push(format!("symbol {:?}: missing component", s.id)),
                Some(c) => {
                    let units = self.library.parts.get(c.part).map_or(0, |p| p.units.len());
                    if s.unit as usize >= units {
                        issues.push(format!("symbol {:?}: unit {} out of range", s.id, s.unit));
                    }
                }
            }
        }
        let sheet_refs = sch
            .wires
            .iter()
            .map(|w| w.sheet)
            .chain(sch.junctions.iter().map(|j| j.sheet))
            .chain(sch.labels.iter().map(|l| l.sheet))
            .chain(sch.texts.iter().map(|t| t.sheet));
        for sheet in sheet_refs {
            if !sch.sheets.contains(sheet) {
                issues.push(format!("object on missing sheet {sheet:?}"));
            }
        }
        issues
    }
}
