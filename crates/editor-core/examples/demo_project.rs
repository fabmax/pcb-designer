//! Writes a small demo project folder: `cargo run -p editor-core --example demo_project [dir]`.

use editor_core::{Command, Editor};
use model::builtin::{self, PART_LED, PART_RESISTOR};
use model::*;

fn main() {
    let dir = std::env::args().nth(1).unwrap_or_else(|| "out/demo.pcbproj".into());
    let mut ed = Editor::new(Project::new("Demo"));
    let lib = builtin::builtin_library();
    ed.execute(Command::import_parts(&lib, &[PART_RESISTOR, PART_LED]).unwrap()).unwrap();
    for (part, x) in [(PART_RESISTOR, 0.0), (PART_LED, 15.24)] {
        ed.execute(Command::PlacePart {
            part,
            variant: None,
            sheet: None,
            pos: Point::mm(x, 0.0),
            rotation: Angle::ZERO,
            mirror: false,
            refdes: None,
            value: None,
        })
        .unwrap();
    }
    ed.execute(Command::AddWire { sheet: None, points: vec![Point::mm(5.08, 0.0), Point::mm(10.16, 0.0)] }).unwrap();
    ed.execute(Command::AddLabel {
        sheet: None,
        pos: Point::mm(20.32, 0.0),
        name: "GND".into(),
        kind: LabelKind::Power,
        shape: LabelShape::default(),
        rotation: Angle::ZERO,
    })
    .unwrap();
    model::io::save_project(ed.project(), &dir).unwrap();
    println!("wrote {dir}");
}
