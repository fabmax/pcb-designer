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
pub const VERSION: u32 = 1;

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
    /// Symbols (with their components), wires, buses, bus entries, junctions, labels and
    /// texts; `library` holds the parts the symbols use.
    Schematic { objects: Vec<Object>, library: Box<Library> },
    /// Tracks, vias, zones, graphics and texts.
    Board { objects: Vec<Object> },
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
            ClipContent::Board { objects } => objects.len(),
            ClipContent::Symbol { pins, graphics, texts } => pins.len() + graphics.len() + texts.len(),
            ClipContent::Footprint { pads, graphics, texts } => pads.len() + graphics.len() + texts.len(),
        }
    }

    pub fn is_empty(&self) -> bool {
        match self {
            ClipContent::Schematic { objects, .. } | ClipContent::Board { objects } => objects.is_empty(),
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

/// Copy schematic or board objects (sheet symbols and footprints are left out). Returns
/// the clip and how many selected objects were left out; `None` if nothing is copyable.
pub fn copy(project: &Project, ids: &[ObjectId], anchor: Point) -> Option<(Clip, usize)> {
    let mut seen = HashSet::new();
    let mut objects = Vec::new();
    let mut components = Vec::new();
    let mut skipped = 0;
    let schematic = ids.iter().any(|id| is_schematic_copyable(id.kind()));
    for &id in ids {
        if !seen.insert(id) {
            continue;
        }
        let kind = id.kind();
        let ok = if schematic { is_schematic_copyable(kind) } else { is_board_copyable(kind) };
        let Some(o) = project.get(id).filter(|_| ok) else {
            skipped += 1;
            continue;
        };
        if let Object::Symbol(s) = &o
            && !components.contains(&s.component)
        {
            components.push(s.component);
        }
        objects.push(o);
    }
    if objects.is_empty() {
        return None;
    }
    let content = if schematic {
        let mut library = Library { id: None, name: "clipboard".into(), ..Default::default() };
        let mut comps = Vec::new();
        for c in components.iter().filter_map(|&c| project.schematic.components.get(c)) {
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
        // Components first: pasting creates them before their symbols.
        comps.extend(objects);
        ClipContent::Schematic { objects: comps, library: Box::new(library) }
    } else {
        ClipContent::Board { objects }
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
            | ObjectKind::Label
            | ObjectKind::SchText
    )
}

fn is_board_copyable(kind: ObjectKind) -> bool {
    matches!(
        kind,
        ObjectKind::Track | ObjectKind::Via | ObjectKind::Zone | ObjectKind::BoardGraphic | ObjectKind::BoardText
    )
}

/// Kinds a paste places (and selects afterwards).
pub fn is_pasted_kind(kind: ObjectKind) -> bool {
    is_schematic_copyable(kind) || is_board_copyable(kind)
}
