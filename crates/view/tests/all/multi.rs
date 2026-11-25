//! Editing the properties several selected objects share (Properties panel).

use model::*;
use view::Document;
use view::demo::demo_project;
use view::multi::{Prop, Value, any_locked_footprint, set_all, shared};

fn value(doc: &Document, ids: &[ObjectId], prop: Prop) -> Option<Option<Value>> {
    shared(doc.project(), ids).into_iter().find(|s| s.prop == prop).map(|s| s.value)
}

#[test]
fn tracks_share_their_width_and_take_a_new_one_in_one_step() {
    let mut doc = Document::new(demo_project());
    let tracks: Vec<Id> = doc.project().board.tracks.iter().map(|t| t.id).take(3).collect();
    assert!(tracks.len() >= 2);
    let ids: Vec<ObjectId> = tracks.iter().map(|&t| ObjectId::Track(t)).collect();
    // One wider: the width differs, the rest is shared.
    let mut t = doc.project().board.tracks.get(tracks[0]).unwrap().clone();
    t.width += mm(0.1);
    doc.execute(editor_core::Command::Update { objects: vec![Object::Track(t)] }).unwrap();
    assert_eq!(value(&doc, &ids, Prop::TrackWidth), Some(None), "differs");
    assert_eq!(value(&doc, &ids, Prop::Locked), Some(Some(Value::Flag(false))));
    assert!(value(&doc, &ids, Prop::CopperLayer).is_some());
    assert_eq!(value(&doc, &ids, Prop::ViaDiameter), None, "tracks have no via size");

    let cmd = set_all(doc.project(), &ids, Prop::TrackWidth, Value::Length(mm(0.6))).unwrap();
    doc.execute(cmd).unwrap();
    assert!(tracks.iter().all(|&t| doc.project().board.tracks.get(t).unwrap().width == mm(0.6)));
    assert_eq!(value(&doc, &ids, Prop::TrackWidth), Some(Some(Value::Length(mm(0.6)))));
    // Nothing to change: no command.
    assert!(set_all(doc.project(), &ids, Prop::TrackWidth, Value::Length(mm(0.6))).is_none());
    doc.undo();
    assert_eq!(value(&doc, &ids, Prop::TrackWidth), Some(None), "one undo step");
}

#[test]
fn tracks_vias_and_parts_share_locked_but_no_position() {
    let mut doc = Document::new(demo_project());
    let b = &doc.project().board;
    let fp = b.footprints.iter().next().unwrap().id;
    let mut ids = vec![ObjectId::Track(b.tracks.iter().next().unwrap().id), ObjectId::Footprint(fp)];
    // Its reference text rides along with the footprint: it doesn't count.
    let field = b.fields.iter().find(|f| f.footprint == fp).unwrap().id;
    ids.push(ObjectId::FootprintField(field));
    let props: Vec<Prop> = shared(doc.project(), &ids).into_iter().map(|s| s.prop).collect();
    assert_eq!(props, [Prop::Locked]);

    doc.execute(set_all(doc.project(), &ids, Prop::Locked, Value::Flag(true)).unwrap()).unwrap();
    assert_eq!(value(&doc, &ids, Prop::Locked), Some(Some(Value::Flag(true))));
    assert!(any_locked_footprint(doc.project(), &ids));
}

#[test]
fn texts_of_several_parts_share_size_and_visibility() {
    let mut doc = Document::new(demo_project());
    let ids: Vec<ObjectId> = doc.project().board.fields.iter().map(|f| ObjectId::FootprintField(f.id)).collect();
    assert!(ids.len() > 2);
    let props: Vec<Prop> = shared(doc.project(), &ids).into_iter().map(|s| s.prop).collect();
    assert_eq!(props, [Prop::TextSize, Prop::TextBold, Prop::Visible]);
    doc.execute(set_all(doc.project(), &ids, Prop::Visible, Value::Flag(false)).unwrap()).unwrap();
    assert!(doc.project().board.fields.iter().all(|f| f.hidden));
}

#[test]
fn via_sizes_keep_a_ring() {
    let mut doc = Document::new(Project::new("T"));
    for x in [0.0, 5.0] {
        let cmd = editor_core::Command::AddVia {
            pos: Point::mm(x, 0.0),
            diameter: Some(mm(0.6)),
            drill: Some(mm(0.3)),
            span: None,
        };
        doc.execute(cmd).unwrap();
    }
    let ids: Vec<ObjectId> = doc.project().board.vias.iter().map(|v| ObjectId::Via(v.id)).collect();
    doc.execute(set_all(doc.project(), &ids, Prop::ViaDrill, Value::Length(mm(1.0))).unwrap()).unwrap();
    assert!(doc.project().board.vias.iter().all(|v| v.drill == mm(0.55)));
}
