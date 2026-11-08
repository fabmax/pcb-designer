//! Index of the KiCad libraries found in a set of directories. Only names are read up
//! front; symbols and footprints are parsed when requested.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::common::glob;
use crate::footprint::{self, KiFootprint};
use crate::sexpr::{self, Node};
use crate::symbol::{self, KiSymbol};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0}: {1}")]
    Io(PathBuf, std::io::Error),
    #[error("{0}:{line}: {msg}", line = .1.line, msg = .1.message)]
    Parse(PathBuf, sexpr::ParseError),
    #[error("not found: {0}")]
    NotFound(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SymbolSource {
    /// `Name.kicad_sym`: all symbols in one file (KiCad 6–9).
    File(PathBuf),
    /// `Name.kicad_symdir`: one `.kicad_sym` file per symbol (KiCad 10).
    Dir(PathBuf),
}

#[derive(Clone, Debug)]
pub struct SymbolLib {
    pub name: String,
    pub source: SymbolSource,
    pub symbols: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct FootprintLib {
    pub name: String,
    pub dir: PathBuf,
    pub footprints: Vec<String>,
}

#[derive(Default)]
pub struct Catalog {
    pub symbol_libs: Vec<SymbolLib>,
    pub footprint_libs: Vec<FootprintLib>,
    /// Parsed `.kicad_sym` files (whole-library files are parsed once).
    cache: HashMap<PathBuf, Vec<(String, Node)>>,
    pad_names: HashMap<String, Vec<String>>,
}

fn read(path: &Path) -> Result<String, Error> {
    std::fs::read_to_string(path).map_err(|e| Error::Io(path.to_path_buf(), e))
}

fn entries(dir: &Path) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> =
        std::fs::read_dir(dir).map(|r| r.filter_map(|e| Some(e.ok()?.path())).collect()).unwrap_or_default();
    v.sort();
    v
}

fn stem(p: &Path) -> String {
    p.file_stem().and_then(|s| s.to_str()).unwrap_or("").to_string()
}

fn ext(p: &Path) -> &str {
    p.extension().and_then(|s| s.to_str()).unwrap_or("")
}

/// Split `Lib:Name`.
pub fn split(full: &str) -> Option<(&str, &str)> {
    full.split_once(':')
}

impl Catalog {
    /// Index the libraries in (or at) the given paths. A path may be a library itself
    /// (`.kicad_sym`, `.kicad_symdir`, `.pretty`) or a directory containing libraries.
    pub fn scan(paths: &[PathBuf]) -> Catalog {
        let mut c = Catalog::default();
        for p in paths {
            if is_library(p) {
                c.add(p);
            } else {
                for e in entries(p) {
                    if is_library(&e) {
                        c.add(&e);
                    }
                }
            }
        }
        c.symbol_libs.sort_by_key(|l| l.name.to_lowercase());
        c.footprint_libs.sort_by_key(|l| l.name.to_lowercase());
        c
    }

    fn add(&mut self, p: &Path) {
        let name = stem(p);
        match ext(p) {
            "kicad_sym" => {
                let symbols = read(p).map(|t| sexpr::top_level_names(&t, "symbol")).unwrap_or_default();
                self.symbol_libs.push(SymbolLib { name, source: SymbolSource::File(p.into()), symbols });
            }
            "kicad_symdir" => {
                let symbols = entries(p).iter().filter(|e| ext(e) == "kicad_sym").map(|e| stem(e)).collect();
                self.symbol_libs.push(SymbolLib { name, source: SymbolSource::Dir(p.into()), symbols });
            }
            "pretty" => {
                let footprints = entries(p).iter().filter(|e| ext(e) == "kicad_mod").map(|e| stem(e)).collect();
                self.footprint_libs.push(FootprintLib { name, dir: p.into(), footprints });
            }
            _ => {}
        }
    }

    pub fn symbol_count(&self) -> usize {
        self.symbol_libs.iter().map(|l| l.symbols.len()).sum()
    }

    pub fn footprint_count(&self) -> usize {
        self.footprint_libs.iter().map(|l| l.footprints.len()).sum()
    }

    /// Symbols whose `Lib:Name` contains all whitespace-separated words of `query`
    /// (case-insensitive), at most `limit`.
    pub fn search_symbols(&self, query: &str, limit: usize) -> Vec<String> {
        search(self.symbol_libs.iter().map(|l| (&l.name, &l.symbols)), query, limit)
    }

    pub fn search_footprints(&self, query: &str, limit: usize) -> Vec<String> {
        search(self.footprint_libs.iter().map(|l| (&l.name, &l.footprints)), query, limit)
    }

    /// The raw node of a symbol in a library (no `extends` resolution).
    fn symbol_node(&mut self, lib: &str, name: &str) -> Result<Node, Error> {
        let not_found = || Error::NotFound(format!("{lib}:{name}"));
        let l = self.symbol_libs.iter().find(|l| l.name == lib).ok_or_else(not_found)?;
        let path = match &l.source {
            SymbolSource::File(p) => p.clone(),
            SymbolSource::Dir(d) => {
                // File names usually equal the symbol name, but not always in case.
                let file = l
                    .symbols
                    .iter()
                    .find(|s| *s == name)
                    .or_else(|| l.symbols.iter().find(|s| s.eq_ignore_ascii_case(name)));
                d.join(format!("{}.kicad_sym", file.map_or(name, |f| f.as_str())))
            }
        };
        if !self.cache.contains_key(&path) {
            let text = read(&path)?;
            let parsed = symbol::parse_library(&text).map_err(|e| Error::Parse(path.clone(), e))?;
            // Per-symbol files are small and numerous: don't keep them.
            if matches!(l.source, SymbolSource::Dir(_)) {
                let only = parsed.len() == 1;
                return parsed
                    .into_iter()
                    .find(|(n, _)| only || n.eq_ignore_ascii_case(name))
                    .map(|(_, n)| n)
                    .ok_or_else(not_found);
            }
            self.cache.insert(path.clone(), parsed);
        }
        self.cache[&path].iter().find(|(n, _)| n == name).map(|(_, n)| n.clone()).ok_or_else(not_found)
    }

    /// Load and convert a symbol, resolving `extends`.
    pub fn load_symbol(&mut self, full: &str) -> Result<KiSymbol, Error> {
        let (lib, name) = split(full).ok_or_else(|| Error::NotFound(full.into()))?;
        let node = self.symbol_node(lib, name)?;
        let parent = match node.child("extends").and_then(|e| e.arg(0)) {
            Some(p) => Some(self.symbol_node(lib, p)?),
            None => None,
        };
        Ok(symbol::convert(lib, &node, parent.as_ref()))
    }

    pub fn load_footprint(&self, full: &str) -> Result<KiFootprint, Error> {
        let (lib, name) = split(full).ok_or_else(|| Error::NotFound(full.into()))?;
        let l = self.footprint_libs.iter().find(|l| l.name == lib).ok_or_else(|| Error::NotFound(full.into()))?;
        let path = l.dir.join(format!("{name}.kicad_mod"));
        let text = read(&path)?;
        footprint::parse(lib, &text).map_err(|e| Error::Parse(path, e))
    }

    pub fn has_footprint(&self, full: &str) -> bool {
        split(full).is_some_and(|(lib, name)| {
            self.footprint_libs.iter().any(|l| l.name == lib && l.footprints.iter().any(|f| f == name))
        })
    }

    /// Footprints matching a symbol's filters (`ki_fp_filters`), at most `limit`, with
    /// the index of the first filter they match. Ordered by filter (the first filter
    /// usually names the most common package), then plainest name first.
    pub fn footprint_candidates(&self, filters: &[String], limit: usize) -> Vec<(usize, String)> {
        let mut out: Vec<(usize, String)> = Vec::new();
        for (i, pat) in filters.iter().enumerate() {
            let (lib_pat, name_pat) = pat.split_once(':').unwrap_or(("*", pat));
            let mut hits: Vec<(&str, &str)> = Vec::new();
            for l in self.footprint_libs.iter().filter(|l| glob(lib_pat, &l.name)) {
                hits.extend(l.footprints.iter().filter(|f| glob(name_pat, f)).map(|f| (l.name.as_str(), f.as_str())));
            }
            hits.sort_by(|a, b| a.1.len().cmp(&b.1.len()).then_with(|| model::natural_cmp(a.1, b.1)));
            for (l, f) in hits {
                let full = format!("{l}:{f}");
                if !out.iter().any(|o| o.1 == full) {
                    out.push((i, full));
                    if out.len() >= limit {
                        return out;
                    }
                }
            }
        }
        out
    }

    /// Distinct pad names of a footprint (cached).
    pub fn pad_names(&mut self, full: &str) -> Option<&[String]> {
        if !self.pad_names.contains_key(full) {
            let fp = self.load_footprint(full).ok()?;
            let mut names: Vec<String> =
                fp.footprint.pads.iter().map(|p| p.name.clone()).filter(|n| !n.is_empty()).collect();
            names.sort_unstable();
            names.dedup();
            self.pad_names.insert(full.to_string(), names);
        }
        self.pad_names.get(full).map(Vec::as_slice)
    }

    /// The best footprint for a symbol with the given (sorted, distinct) pin numbers:
    /// one whose pads are exactly the pins if possible (earlier filters first), else one
    /// with all pins and the fewest extra pads (unconnected pins are often not drawn).
    pub fn best_footprint(&mut self, filters: &[String], pins: &[&str]) -> Option<String> {
        let candidates = self.footprint_candidates(filters, 300);
        let mut best: Option<(Score, String)> = None;
        for (i, (filter, full)) in candidates.into_iter().enumerate() {
            let Some(pads) = self.pad_names(&full) else { continue };
            let all = pins.iter().all(|p| pads.binary_search_by(|x| x.as_str().cmp(p)).is_ok());
            let class = match (all, pads.len() == pins.len()) {
                (true, true) => 0,
                (true, false) => 1,
                _ => 2,
            };
            let key = (class, filter, pads.len().abs_diff(pins.len()), common_rank(&full), i);
            if best.as_ref().is_none_or(|(k, _)| key < *k) {
                best = Some((key, full));
            }
        }
        best.map(|b| b.1)
    }
}

/// Footprint ranking: (fit class, filter index, pad count difference, commonness, order).
type Score = (u8, usize, usize, usize, usize);

/// Tie-breaker between equally fitting footprints: everyday packages first
/// (R_0603 rather than R_MELF for a resistor).
fn common_rank(full: &str) -> usize {
    const COMMON: &[&str] =
        &["_0603_", "SOT-23", "SOIC-", "SOD-123", "TSSOP-", "LQFP-", "QFN-", "TO-220", "PinHeader_"];
    COMMON.iter().position(|c| full.contains(c)).unwrap_or(COMMON.len())
}

fn is_library(p: &Path) -> bool {
    matches!(ext(p), "kicad_sym" | "kicad_symdir" | "pretty")
}

fn search<'a>(libs: impl Iterator<Item = (&'a String, &'a Vec<String>)>, query: &str, limit: usize) -> Vec<String> {
    let words: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
    let mut out = Vec::new();
    for (lib, names) in libs {
        let lib_lower = lib.to_lowercase();
        for n in names {
            let n_lower = n.to_lowercase();
            if words.iter().all(|w| n_lower.contains(w.as_str()) || lib_lower.contains(w.as_str())) {
                out.push(format!("{lib}:{n}"));
                if out.len() >= limit {
                    return out;
                }
            }
        }
    }
    out
}

/// Standard install locations of KiCad's symbol and footprint libraries (those that
/// exist), newest version first. Environment variables (`KICAD10_SYMBOL_DIR`, …) win.
pub fn default_paths() -> (Vec<PathBuf>, Vec<PathBuf>) {
    let mut syms = Vec::new();
    let mut fps = Vec::new();
    for v in (6..=10).rev() {
        if let Ok(p) = std::env::var(format!("KICAD{v}_SYMBOL_DIR")) {
            syms.push(PathBuf::from(p));
        }
        if let Ok(p) = std::env::var(format!("KICAD{v}_FOOTPRINT_DIR")) {
            fps.push(PathBuf::from(p));
        }
    }
    let mut roots: Vec<PathBuf> = vec![
        "/Applications/KiCad/KiCad.app/Contents/SharedSupport".into(),
        "/usr/share/kicad".into(),
        "/usr/local/share/kicad".into(),
    ];
    if let Ok(home) = std::env::var("HOME") {
        roots.push(Path::new(&home).join(".local/share/kicad"));
    }
    for pf in ["C:\\Program Files\\KiCad", "C:\\Program Files (x86)\\KiCad"] {
        let mut versions: Vec<PathBuf> = entries(Path::new(pf));
        versions.reverse();
        roots.extend(versions.into_iter().map(|v| v.join("share\\kicad")));
    }
    for r in roots {
        let (s, f) = (r.join("symbols"), r.join("footprints"));
        if s.is_dir() && !syms.contains(&s) {
            syms.push(s);
        }
        if f.is_dir() && !fps.contains(&f) {
            fps.push(f);
        }
    }
    syms.retain(|p| p.exists());
    fps.retain(|p| p.exists());
    (syms, fps)
}
