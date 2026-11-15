//! KiCad symbols and footprints converted for previews, kept in a small cache so that
//! switching between recently shown items doesn't parse their libraries again.

use std::collections::VecDeque;

use model::{Footprint, Symbol};

use crate::catalog::Catalog;

/// Items kept; the least recently used is dropped beyond this.
pub const CAPACITY: usize = 32;

#[derive(Clone, PartialEq, Eq)]
enum Key {
    Symbol(String),
    Footprint(String),
}

#[derive(Clone)]
enum Entry {
    Symbol(Symbol),
    Footprint(Footprint),
}

/// Converted items by `Lib:Name`, including load errors.
#[derive(Default)]
pub struct PreviewCache {
    entries: VecDeque<(Key, Result<Entry, String>)>,
}

impl PreviewCache {
    /// The first unit of a symbol, converted as an import would.
    pub fn symbol(&mut self, catalog: &mut Catalog, full: &str) -> Result<Symbol, String> {
        let r = self.get(Key::Symbol(full.into()), || {
            let sym = catalog.load_symbol(full).map_err(|e| e.to_string())?;
            sym.units.into_iter().next().map(Entry::Symbol).ok_or_else(|| format!("{full}: symbol has no units"))
        });
        r.map(|e| match e {
            Entry::Symbol(s) => s,
            Entry::Footprint(_) => unreachable!(),
        })
    }

    /// A footprint, converted as an import would.
    pub fn footprint(&mut self, catalog: &Catalog, full: &str) -> Result<Footprint, String> {
        let r = self.get(Key::Footprint(full.into()), || {
            catalog.load_footprint(full).map(|f| Entry::Footprint(f.footprint)).map_err(|e| e.to_string())
        });
        r.map(|e| match e {
            Entry::Footprint(f) => f,
            Entry::Symbol(_) => unreachable!(),
        })
    }

    fn get(&mut self, key: Key, load: impl FnOnce() -> Result<Entry, String>) -> Result<Entry, String> {
        let entry = match self.entries.iter().position(|(k, _)| *k == key) {
            Some(i) => self.entries.remove(i).unwrap(),
            None => (key, load()),
        };
        let r = entry.1.clone();
        self.entries.push_back(entry);
        if self.entries.len() > CAPACITY {
            self.entries.pop_front();
        }
        r
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn clear(&mut self) {
        self.entries.clear();
    }
}
