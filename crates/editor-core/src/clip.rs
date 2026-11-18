//! The clipboard format: copied objects as JSON text on the system clipboard, so clips
//! can be pasted into another sheet, project or app instance.
//!
//! A clip holds whole objects in their original coordinates plus the `anchor` they are
//! carried by; pasting at `at` moves them by `at - anchor`. Schematic clips carry the
//! definitions of the parts their symbols use (see [`Command::Paste`](crate::Command)).
//! Library item clips (pins, pads, graphics, texts) are pasted by the item editors.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use model::*;

/// Format version, also the marker that tells our clips from other text.
pub const VERSION: u32 = 2;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Clip {
    /// [`VERSION`]; the key marks the text as a clip.
    pub pcb_designer_clip: u32,
    /// The point the clip is held by while pasting (snapped to the grid).
    pub anchor: Point,
    #[serde(flatten)]
    pub content: ClipContent,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(tag = "domain", rename_all = "snake_case")]
pub enum ClipContent {
    /// Symbols (with their components), wires, buses, bus entries, junctions, no-connect
    /// flags, labels, texts and sheet symbols (with their sheets and everything on them,
    /// sub-sheets included); `library` holds the parts the symbols use.
    Schematic { objects: Vec<Object>, library: Box<Library> },
    /// Tracks, vias, zones, graphics, texts and board parts (components, footprints and
    /// their texts); `library` holds the board parts' parts.
    Board {
        objects: Vec<Object>,
        #[serde(default)]
        library: Box<Library>,
    },
    Symbol {
        #[serde(default)]
        pins: Vec<Pin>,
        #[serde(default)]
        graphics: Vec<Shape>,
        #[serde(default)]
        texts: Vec<Text>,
    },
    Footprint {
        #[serde(default)]
        pads: Vec<Pad>,
        #[serde(default)]
        graphics: Vec<FootprintGraphic>,
        #[serde(default)]
        texts: Vec<FootprintText>,
    },
}

impl ClipContent {
    /// What it holds, for messages ("schematic objects").
    pub fn describe(&self) -> &'static str {
        match self {
            ClipContent::Schematic { .. } => "schematic objects",
            ClipContent::Board { .. } => "board objects",
            ClipContent::Symbol { .. } => "symbol pins and graphics",
            ClipContent::Footprint { .. } => "footprint pads and graphics",
        }
    }

    /// Number of objects (components don't count: their symbols do).
    pub fn len(&self) -> usize {
        match self {
            ClipContent::Schematic { objects, .. } => {
                objects.iter().filter(|o| !matches!(o, Object::Component(_))).count()
            }
            ClipContent::Board { objects, .. } => {
                objects.iter().filter(|o| !matches!(o, Object::Component(_) | Object::FootprintField(_))).count()
            }
            ClipContent::Symbol { pins, graphics, texts } => pins.len() + graphics.len() + texts.len(),
            ClipContent::Footprint { pads, graphics, texts } => pads.len() + graphics.len() + texts.len(),
        }
    }

    pub fn is_empty(&self) -> bool {
        match self {
            ClipContent::Schematic { objects, .. } | ClipContent::Board { objects, .. } => objects.is_empty(),
            ClipContent::Symbol { pins, graphics, texts } => pins.is_empty() && graphics.is_empty() && texts.is_empty(),
            ClipContent::Footprint { pads, graphics, texts } => {
                pads.is_empty() && graphics.is_empty() && texts.is_empty()
            }
        }
    }
}

impl Clip {
    pub fn new(anchor: Point, content: ClipContent) -> Clip {
        Clip { pcb_designer_clip: VERSION, anchor, content }
    }

    pub fn to_text(&self) -> String {
        serde_json::to_string_pretty(self).expect("clips serialize")
    }

    /// The clip in `text`. `None` if the text isn't a clip at all; an error if it is one
    /// that can't be read (another version, damaged).
    pub fn from_text(text: &str) -> Option<Result<Clip, String>> {
        let value: serde_json::Value = serde_json::from_str(text.trim()).ok()?;
        let version = value.get("pcb_designer_clip")?.as_u64()?;
        if version != u64::from(VERSION) {
            return Some(Err(format!("the clipboard holds a clip of another version ({version})")));
        }
        Some(serde_json::from_value(value).map_err(|e| format!("the clipboard's clip can't be read: {e}")))
    }
}

/// Copy schematic or board objects. A sheet symbol takes its sheet along with everything
/// on it and below it; a footprint is copied only if it is a board part's (schematic
/// parts' footprints follow the schematic). Returns the clip and how many selected objects
/// were left out; `None` if nothing is copyable.
pub fn copy(project: &Project, ids: &[ObjectId], anchor: Point) -> Option<(Clip, usize)> {
    let sch = &project.schematic;
    let mut seen = HashSet::new();
    let mut objects = Vec::new();
    let mut components = Vec::new();
    let mut skipped = 0;
    let schematic = ids.iter().any(|id| is_schematic_copyable(id.kind()));
    let mut take = |o: Object, objects: &mut Vec<Object>, components: &mut Vec<Id>| {
        if !seen.insert(o.id()) {
            return;
        }
        let component = match &o {
            Object::Symbol(s) => Some(s.component),
            Object::Footprint(f) => Some(f.component),
            _ => None,
        };
        if let Some(c) = component
            && !components.contains(&c)
        {
            components.push(c);
        }
        let owner = o.id();
        objects.push(o);
        // Reference and value texts go with their symbol or footprint.
        match owner {
            ObjectId::Symbol(s) => {
                objects.extend(sch.fields.iter().filter(|f| f.symbol == s).cloned().map(Object::SymbolField))
            }
            ObjectId::Footprint(fp) => objects
                .extend(project.board.fields.iter().filter(|f| f.footprint == fp).cloned().map(Object::FootprintField)),
            _ => {}
        }
    };
    for &id in ids {
        let kind = id.kind();
        if matches!(kind, ObjectKind::SymbolField | ObjectKind::FootprintField) {
            continue; // goes with its owner
        }
        let ok = if schematic { is_schematic_copyable(kind) } else { is_board_copyable(kind) };
        let board_part = match id {
            ObjectId::Footprint(f) => project
                .board
                .footprints
                .get(f)
                .and_then(|f| sch.components.get(f.component))
                .and_then(|c| project.library.parts.get(c.part))
                .is_some_and(|p| p.is_board_part()),
            _ => true,
        };
        let Some(o) = project.get(id).filter(|_| ok && board_part) else {
            skipped += 1;
            continue;
        };
        take(o, &mut objects, &mut components);
        // A sheet symbol: its sheet tree (parents before children), then their contents.
        if let ObjectId::Sheet(sheet) = id {
            let tree = sch.sheet_and_descendants(sheet);
            for &s in &tree[1..] {
                if let Some(o) = project.get(ObjectId::Sheet(s)) {
                    take(o, &mut objects, &mut components);
                }
            }
            for &s in &tree {
                for o in sch.objects_on(s) {
                    if !matches!(o, ObjectId::Sheet(_) | ObjectId::SymbolField(_))
                        && let Some(o) = project.get(o)
                    {
                        take(o, &mut objects, &mut components);
                    }
                }
            }
        }
    }
    if objects.is_empty() {
        return None;
    }
    // The parts the components use, and the components first: pasting creates them
    // before their symbols and footprints.
    let mut library = Library { id: None, name: "clipboard".into(), ..Default::default() };
    let mut comps = Vec::new();
    for c in components.iter().filter_map(|&c| sch.components.get(c)) {
        comps.push(Object::Component(c.clone()));
        if library.parts.contains(c.part) {
            continue;
        }
        if let Some((part, symbols, footprints)) = project.library.extract_part(c.part) {
            for s in symbols {
                if !library.symbols.contains(s.id) {
                    library.symbols.insert(s);
                }
            }
            for f in footprints {
                if !library.footprints.contains(f.id) {
                    library.footprints.insert(f);
                }
            }
            library.parts.insert(part);
        }
    }
    comps.extend(objects);
    let library = Box::new(library);
    let content = if schematic {
        ClipContent::Schematic { objects: comps, library }
    } else {
        ClipContent::Board { objects: comps, library }
    };
    Some((Clip::new(anchor, content), skipped))
}

fn is_schematic_copyable(kind: ObjectKind) -> bool {
    matches!(
        kind,
        ObjectKind::Symbol
            | ObjectKind::Wire
            | ObjectKind::BusWire
            | ObjectKind::BusEntry
            | ObjectKind::Junction
            | ObjectKind::NoConnect
            | ObjectKind::PowerFlag
            | ObjectKind::Label
            | ObjectKind::SchText
            | ObjectKind::Sheet
    )
}

fn is_board_copyable(kind: ObjectKind) -> bool {
    matches!(
        kind,
        ObjectKind::Track
            | ObjectKind::Via
            | ObjectKind::Zone
            | ObjectKind::Keepout
            | ObjectKind::BoardGraphic
            | ObjectKind::BoardText
            | ObjectKind::Footprint
    )
}

/// Kinds a paste places (and selects afterwards).
pub fn is_pasted_kind(kind: ObjectKind) -> bool {
    is_schematic_copyable(kind) || is_board_copyable(kind)
}
