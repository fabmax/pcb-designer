//! The findings (ERC problems, DRC violations, flagged copper) under the pointer: what a
//! hover tip on the canvas shows. Only findings that are drawn count: markers, unconnected
//! pin rings and the halo of flagged tracks and vias.

use editor_core::copper::{CopperItem, Flag};
use editor_core::drc::short_key;
use model::{Coord, ErcRule, ObjectId, Point, Severity, ViolationKind};

use crate::Document;
use crate::view::ViewId;

/// Radius of a finding's marker (a ring with a cross), in nm.
const MARKER_RADIUS: Coord = 600_000;
/// Radius of an unconnected pin's ring, in nm.
const RING_RADIUS: Coord = 400_000;

/// One finding under the pointer.
#[derive(Clone, Debug, PartialEq)]
pub struct Finding {
    pub severity: Severity,
    pub message: String,
}

/// The findings shown at `p` in `view` (within `tol`), and those about `hovered` (the
/// object under the pointer), most severe first, without duplicates.
pub fn at(doc: &Document, view: ViewId, p: Point, tol: Coord, hovered: Option<ObjectId>) -> Vec<Finding> {
    let near = |q: Point, r: Coord| (q - p).length() <= (r + tol) as f64;
    let mut out: Vec<Finding> = Vec::new();
    match view {
        ViewId::Sheet(_) => {
            let v = doc.view(view);
            let markers = v.markers_visible();
            for f in doc.connectivity().problems.iter().filter(|f| !f.excluded) {
                if !f.objects.iter().any(|&o| v.index.bounds(o).is_some()) {
                    continue;
                }
                // Unconnected pins have their ring (always drawn); the rest a marker.
                let hit = if f.rule == ErcRule::UnconnectedPin {
                    f.pos.is_some_and(|q| near(q, RING_RADIUS))
                } else {
                    markers
                        && (f.pos.is_some_and(|q| near(q, MARKER_RADIUS))
                            || hovered.is_some_and(|h| f.objects.contains(&h)))
                };
                if hit {
                    out.push(Finding { severity: f.severity, message: f.message.clone() });
                }
            }
        }
        ViewId::Board => {
            let copper = doc.copper();
            let drc = &doc.project().board.drc;
            let flagged = match hovered {
                Some(ObjectId::Track(id)) => copper.get(CopperItem::Track(id)),
                Some(ObjectId::Via(id)) => copper.get(CopperItem::Via(id)),
                _ => None,
            };
            if let Some(c) = flagged {
                match c.flag {
                    Some(Flag::Short) => {
                        let mut nets: Vec<String> = copper
                            .items
                            .iter()
                            .filter(|o| o.island == c.island && matches!(o.item, CopperItem::Pad { .. }))
                            .filter_map(|o| o.net.map(|n| copper.nets[n as usize].clone()))
                            .collect();
                        nets.sort();
                        nets.dedup();
                        let severity = drc.severity(ViolationKind::Short);
                        if severity != Severity::Ignore && drc.excluded(&short_key(&nets)).is_none() {
                            out.push(Finding { severity, message: format!("Short: {}", nets.join(", ")) });
                        }
                    }
                    Some(Flag::Unconnected) => out.push(Finding {
                        severity: Severity::Warning,
                        message: "Unconnected copper: touches no pad".into(),
                    }),
                    None => {}
                }
            }
            let markers = doc.view(view).markers_visible();
            for v in doc.drc().into_iter().flat_map(|r| &r.violations).filter(|v| markers && !v.excluded) {
                if near(v.pos, MARKER_RADIUS) || hovered.is_some_and(|h| v.objects.contains(&h)) {
                    out.push(Finding { severity: v.severity, message: v.message.clone() });
                }
            }
        }
    }
    let mut seen = std::collections::HashSet::new();
    out.retain(|f| seen.insert(f.message.clone()));
    out.sort_by_key(|f| f.severity != Severity::Error);
    out
}
