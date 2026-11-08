//! Every kind of object survives a JSON round trip as `Object` (the shape used by commands,
//! MCP and the undo log), i.e. no object field collides with the `type` tag.

use model::*;
use view::demo::demo_project;

#[test]
fn every_object_kind_round_trips() {
    let mut p = demo_project();
    let sheet = p.schematic.first_sheet().unwrap();
    p.put(Object::Junction(Junction { id: Id::new(), sheet, pos: Point::mm(1.0, 2.0) }), None);
    let text = Text::new("REV A", Point::mm(3.0, 3.0), mm(1.0));
    p.put(Object::BoardText(BoardText { id: Id::new(), layer: LayerId::TOP_SILK, text }), None);

    for &kind in ObjectKind::ALL {
        let objects: Vec<Object> = p.objects(kind).collect();
        assert!(!objects.is_empty(), "test project has no {kind:?}");
        for o in objects {
            let json = serde_json::to_string(&o).unwrap();
            let back: Object = serde_json::from_str(&json).unwrap_or_else(|e| panic!("{kind:?}: {e}\n{json}"));
            assert_eq!(back, o, "{kind:?}");
            let tag = serde_json::to_value(kind).unwrap();
            assert_eq!(serde_json::from_str::<serde_json::Value>(&json).unwrap()["type"], tag);
        }
    }
}
