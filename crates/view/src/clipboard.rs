//! Copy and paste on the schematic and board canvases (library items: see
//! [`crate::item::copy_elems`]). The clip itself is [`editor_core::clip`]; the app moves its
//! text to and from the system clipboard.

use editor_core::Command;
use editor_core::clip::{self, Clip, ClipContent};
use model::*;

use crate::document::Document;
use crate::interaction::reference_point;
use crate::view::{ViewId, ViewKind};

/// A clip of the selection, held by the (snapped) reference point of its first copyable
/// object, and a note on what was left out. An error if nothing can be copied.
pub fn copy(
    project: &Project,
    selection: &[ObjectId],
    snap: impl Fn(Point) -> Point,
) -> Result<(Clip, Option<String>), String> {
    let first = selection.iter().find(|id| clip::is_pasted_kind(id.kind()));
    let anchor = first.and_then(|&id| reference_point(project, id)).map(snap).unwrap_or_default();
    let (clip, skipped) = clip::copy(project, selection, anchor).ok_or_else(|| {
        if selection.is_empty() { "Nothing selected" } else { "Sheet symbols and footprints can't be copied" }
            .to_string()
    })?;
    let note = (skipped > 0).then(|| format!("{skipped} object(s) can't be copied (sheet symbols, footprints)"));
    Ok((clip, note))
}

/// Paste `clip` into `view` with its anchor at `at`, as part of undo `group`. Returns the
/// placed objects.
pub fn paste(
    doc: &mut Document,
    view: ViewId,
    clip: Clip,
    at: Point,
    group: Option<u64>,
) -> Result<Vec<ObjectId>, String> {
    let fits = matches!(
        (&clip.content, view.kind()),
        (ClipContent::Schematic { .. }, ViewKind::Schematic) | (ClipContent::Board { .. }, ViewKind::Board)
    );
    if !fits {
        let here = if view.kind() == ViewKind::Schematic { "schematic" } else { "board" };
        return Err(format!("The clipboard holds {}; they can't be pasted into the {here}", clip.content.describe()));
    }
    let sheet = match view {
        ViewId::Sheet(s) => Some(s),
        ViewId::Board => None,
    };
    let changes = doc.execute_grouped(Command::Paste { clip, sheet, at }, group).map_err(|e| e.to_string())?;
    Ok(changes
        .changes
        .iter()
        .filter(|c| c.before.is_none() && c.after.is_some() && clip::is_pasted_kind(c.id.kind()))
        .map(|c| c.id)
        .collect())
}
