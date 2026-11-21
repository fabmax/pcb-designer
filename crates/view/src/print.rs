//! The schematic as a PDF: a page per sheet in the light palette, as the canvas draws it
//! without grid, selection, findings or simulation values.

use editor_core::connectivity::Connectivity;
use model::*;
use render::Instance;
use render::primitive::kind;

use crate::style::{self, LIGHT};
use crate::view::{View, ViewId};

/// Space around the drawing of a sheet without a page (mm).
const MARGIN: f32 = 10.0;

/// The sheets root first, each followed by its sub-sheets (depth first).
pub fn sheets_in_order(project: &Project) -> Vec<Id> {
    fn visit(s: &Schematic, id: Id, out: &mut Vec<Id>) {
        if out.contains(&id) {
            return; // a cycle (invalid project)
        }
        out.push(id);
        for c in s.children(id) {
            visit(s, c.id, out);
        }
    }
    let mut out = Vec::new();
    if let Some(root) = project.schematic.root_sheet() {
        visit(&project.schematic, root, &mut out);
    }
    out
}

/// The PDF of the schematic: a page per sheet ([`sheets_in_order`]) the size of its
/// page (a sheet without one: its drawing and a margin), bookmarked by its path.
pub fn schematic_pdf(project: &Project, conn: &Connectivity) -> Vec<u8> {
    let pages: Vec<gerber::pdf::Page> = style::with_palette(&LIGHT, || {
        sheets_in_order(project).into_iter().map(|id| page(project, conn, id)).collect()
    });
    gerber::pdf::write(&project.name, &pages)
}

fn page(project: &Project, conn: &Connectivity, sheet: Id) -> gerber::pdf::Page {
    let mut view = View::new(ViewId::Sheet(sheet), project);
    view.set_junctions(conn);
    let mut instances = view.drawing();
    let s = project.schematic.sheets.get(sheet);
    let size = match s.and_then(|s| s.page.dimensions()) {
        Some(d) => [d.w as f32 * 1e-6, d.h as f32 * 1e-6],
        None => match view.scene.bounds() {
            Some([x0, y0, x1, y1]) => {
                let shift = [MARGIN - x0 as f32, MARGIN - y0 as f32];
                for i in &mut instances {
                    translate(i, shift);
                }
                [(x1 - x0) as f32 + 2.0 * MARGIN, (y1 - y0) as f32 + 2.0 * MARGIN]
            }
            None => [210.0, 297.0],
        },
    };
    let title = match s {
        Some(s) if s.is_root() => s.name.clone(),
        _ => project.schematic.sheet_path(sheet),
    };
    gerber::pdf::Page { size, title, instances }
}

fn translate(i: &mut Instance, [dx, dy]: [f32; 2]) {
    i.p0 = [i.p0[0] + dx, i.p0[1] + dy];
    match i.kind & kind::MASK {
        kind::CAPSULE => i.p1 = [i.p1[0] + dx, i.p1[1] + dy],
        kind::TRIANGLE => {
            i.p1 = [i.p1[0] + dx, i.p1[1] + dy];
            (i.r, i.r2) = (i.r + dx, i.r2 + dy);
        }
        _ => {}
    }
}
