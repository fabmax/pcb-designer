//! Cross-probing: what to highlight in one view for a selection in the other.

use editor_core::connectivity::Connectivity;
use editor_core::copper::{BoardConnectivity, CopperItem};
use model::*;

use crate::view::ViewKind;

/// Highlights for the other view.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Probe {
    /// Objects of the other view.
    pub objects: Vec<ObjectId>,
    /// Nets of the selection, highlighted on the board.
    pub nets: Vec<String>,
}

/// Highlights in the other view for `selection` in view `from`:
/// * schematic symbols → their footprints; wires, labels and junctions → their nets'
///   copper on the board;
/// * board footprints → their symbols; tracks and vias → their nets' wires, labels and
///   junctions in the schematic.
pub fn cross_probe(
    project: &Project,
    conn: &Connectivity,
    copper: &BoardConnectivity,
    from: ViewKind,
    selection: &[ObjectId],
) -> Probe {
    let mut probe = Probe::default();
    let mut nets: Vec<String> = Vec::new();
    for &id in selection {
        match (from, id) {
            (ViewKind::Schematic, ObjectId::Symbol(s)) => {
                if let Some(sym) = project.schematic.symbols.get(s) {
                    probe.objects.extend(
                        project
                            .board
                            .footprints
                            .iter()
                            .filter(|f| f.component == sym.component)
                            .map(|f| ObjectId::Footprint(f.id)),
                    );
                }
            }
            (ViewKind::Schematic, ObjectId::Wire(_) | ObjectId::Label(_) | ObjectId::Junction(_)) => {
                nets.extend(conn.net_of(id).map(|n| n.name.clone()));
            }
            (ViewKind::Board, ObjectId::Footprint(f)) => {
                if let Some(fp) = project.board.footprints.get(f) {
                    probe.objects.extend(project.schematic.symbols_of(fp.component).map(|s| ObjectId::Symbol(s.id)));
                }
            }
            (ViewKind::Board, ObjectId::Track(t)) => {
                nets.extend(copper.net_of(CopperItem::Track(t)).map(str::to_string))
            }
            (ViewKind::Board, ObjectId::Via(v)) => nets.extend(copper.net_of(CopperItem::Via(v)).map(str::to_string)),
            _ => {}
        }
    }
    nets.sort();
    nets.dedup();
    if from == ViewKind::Board {
        {
            for name in &nets {
                if let Some(net) = conn.net(name) {
                    probe.objects.extend(net.wires.iter().map(|&w| ObjectId::Wire(w)));
                    probe.objects.extend(net.labels.iter().map(|&l| ObjectId::Label(l)));
                    probe.objects.extend(net.junctions.iter().map(|&j| ObjectId::Junction(j)));
                }
            }
        }
    }
    probe.nets = nets;
    probe.objects.sort();
    probe.objects.dedup();
    probe
}
