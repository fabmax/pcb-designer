//! Editing texts in place: keys, the caret, and what a commit changes.

use editor_core::Command;
use model::builtin::{self, PART_RESISTOR};
use model::*;
use view::Document;
use view::text_edit::{EditKey, Mods, TextEdit, TextTarget};
use view::view::ViewId;

fn doc_with_text(text: &str) -> (Document, ViewId, Id) {
    let mut doc = Document::new(Project::new("T"));
    let sheet = doc.project().schematic.root_sheet().unwrap();
    let t = SchText { id: Id::new(), sheet, text: Text::new(text, Point::mm(10.0, 10.0), mm(1.27)) };
    let id = t.id;
    doc.execute(Command::Insert { objects: vec![Object::SchText(t)] }).unwrap();
    (doc, ViewId::Sheet(sheet), id)
}

const NONE: Mods = Mods { shift: false, word: false, line: false };
const SHIFT: Mods = Mods { shift: true, word: false, line: false };

fn edit(text: &str) -> TextEdit {
    TextEdit::new(TextTarget::SchText(Id::new()), text.into(), Text::new("", Point::ZERO, mm(1.0)))
}

#[test]
fn keys_move_the_caret_and_edit() {
    let mut e = edit("hello world");
    // Everything is selected at first: typing replaces it.
    e.insert("abc def");
    assert_eq!(e.text, "abc def");
    e.key(EditKey::Left, Mods { word: true, ..NONE });
    assert_eq!(e.cursor(), (4, 4));
    e.key(EditKey::Backspace, NONE);
    assert_eq!(e.text, "abcdef");
    e.key(EditKey::Home, NONE);
    e.key(EditKey::Right, SHIFT);
    e.key(EditKey::Right, SHIFT);
    assert_eq!(e.selected_text(), "ab");
    assert_eq!(e.cut().as_deref(), Some("ab"));
    assert_eq!(e.text, "cdef");
    e.key(EditKey::End, NONE);
    e.key(EditKey::Backspace, Mods { word: true, ..NONE });
    assert_eq!(e.text, "");
}

#[test]
fn multi_line_texts_take_shift_enter_and_move_between_lines() {
    let mut e = edit("");
    e.insert("ab");
    e.key(EditKey::Enter, SHIFT);
    e.insert("cdef");
    assert_eq!(e.text, "ab\ncdef");
    e.key(EditKey::Up, NONE);
    assert_eq!(e.cursor().0, 2, "end of the shorter line above");
    e.key(EditKey::Down, NONE);
    e.key(EditKey::Home, NONE);
    assert_eq!(e.cursor().0, 3);
    // Single-line texts: newlines become spaces, Shift+Enter commits.
    let mut l = TextEdit::new(TextTarget::Label(Id::new()), String::new(), Text::new("", Point::ZERO, mm(1.0)));
    l.insert("A\nB");
    assert_eq!(l.text, "A B");
    assert_eq!(l.key(EditKey::Enter, SHIFT), view::text_edit::Outcome::Commit);
}

#[test]
fn clicking_places_the_caret_by_the_drawn_glyphs() {
    let mut e = edit("abcd");
    let g = e.geometry();
    // Between the 2nd and 3rd character: half way along a left-aligned 4-char text.
    let mid = Point::new((g.frame[0].x + g.frame[1].x) / 2, g.text.pos.y + mm(0.5));
    e.click(mid, false);
    assert_eq!(e.cursor(), (2, 2));
    // Turned texts too.
    let mut turned = edit("abcd");
    turned.shape.rotation = Angle::DEG90;
    turned.click(Point::new(-mm(0.5), mm(0.1)), false);
    assert_eq!(turned.cursor().0, 0);
    turned.select_word(Point::new(-mm(0.5), mm(1.0)));
    assert_eq!(turned.selected_text(), "abcd");
}

#[test]
fn double_click_edits_a_text_and_commits_one_undo_step() {
    let (mut doc, view, id) = doc_with_text("old");
    assert!(doc.begin_text_edit(view, ObjectId::SchText(id), Point::ZERO));
    doc.with_text_edit(|e| e.insert("new\ntext"));
    doc.text_edit_key(EditKey::Enter, NONE).unwrap();
    assert!(doc.text_edit().is_none());
    assert_eq!(doc.project().schematic.texts.get(id).unwrap().text.text, "new\ntext");
    doc.undo();
    assert_eq!(doc.project().schematic.texts.get(id).unwrap().text.text, "old");
    // Esc leaves it as it was; an empty free text is deleted.
    doc.begin_text_edit(view, ObjectId::SchText(id), Point::ZERO);
    doc.with_text_edit(|e| e.insert("x"));
    doc.text_edit_key(EditKey::Escape, NONE).unwrap();
    assert_eq!(doc.project().schematic.texts.get(id).unwrap().text.text, "old");
    doc.begin_text_edit(view, ObjectId::SchText(id), Point::ZERO);
    doc.with_text_edit(|e| e.insert(""));
    doc.commit_text_edit().unwrap();
    assert!(doc.project().schematic.texts.is_empty());
}

#[test]
fn references_are_edited_at_their_symbol_text() {
    let mut doc = Document::new(Project::new("T"));
    doc.execute(Command::import_parts(&builtin::builtin_library(), &[PART_RESISTOR]).unwrap()).unwrap();
    let place = |doc: &mut Document, x: f64| {
        doc.execute(Command::PlacePart {
            part: PART_RESISTOR,
            variant: None,
            sheet: None,
            pos: Point::mm(x, 10.0),
            rotation: Angle::ZERO,
            mirror: false,
            refdes: None,
            value: None,
        })
        .unwrap();
    };
    place(&mut doc, 10.0);
    place(&mut doc, 30.0);
    let view = ViewId::Sheet(doc.project().schematic.root_sheet().unwrap());
    let s = &doc.project().schematic;
    let r2 = s.component_by_refdes("R2").unwrap().id;
    let sym = s.symbols_of(r2).next().unwrap().id;
    let field = |doc: &Document, t: &str| {
        doc.project().schematic.fields.iter().find(|f| f.symbol == sym && f.text.text == t).unwrap().id
    };
    let reference = field(&doc, "${REF}");
    assert!(doc.begin_text_edit(view, ObjectId::SymbolField(reference), Point::ZERO));
    assert_eq!(doc.text_edit().unwrap().1.text, "R2");
    // A used reference is refused and the old one stays.
    doc.with_text_edit(|e| e.insert("R1"));
    assert!(doc.commit_text_edit().unwrap_err().contains("already used"));
    assert!(doc.project().schematic.component_by_refdes("R2").is_some());
    doc.begin_text_edit(view, ObjectId::SymbolField(reference), Point::ZERO);
    doc.with_text_edit(|e| e.insert("R7"));
    doc.commit_text_edit().unwrap();
    assert_eq!(doc.project().schematic.components.get(r2).unwrap().refdes, "R7");
    // The value.
    doc.begin_text_edit(view, ObjectId::SymbolField(field(&doc, "${VALUE}")), Point::ZERO);
    doc.with_text_edit(|e| e.insert("4k7"));
    doc.commit_text_edit().unwrap();
    assert_eq!(doc.project().schematic.components.get(r2).unwrap().value, "4k7");
}

#[test]
fn library_texts_are_edited_in_their_item() {
    use view::item::Item;
    use view::text_edit::EditSite;
    let mut lib = view::LibraryDoc::new(Library::new("L"));
    let sym = lib.new_symbol("S").unwrap();
    let item = Item::Symbol(sym);
    let doc = &mut lib.doc;
    assert!(doc.begin_item_text_edit(item, 0));
    assert_eq!(doc.text_edit().map(|(s, e)| (s, e.text.clone())), Some((EditSite::Item(item), "${REF}".into())));
    // The editor of that item draws it; others don't.
    assert!(doc.item_text_edit(item).is_some());
    assert!(doc.item_text_edit(Item::Symbol(Id::new())).is_none());
    doc.with_text_edit(|e| e.insert("Note"));
    doc.commit_text_edit().unwrap();
    assert_eq!(doc.project().library.symbols.get(sym).unwrap().texts[0].text, "Note");
    // Emptied, it is removed (one undo step brings it back).
    assert!(doc.begin_item_text_edit(item, 0));
    doc.text_edit_key(EditKey::Backspace, NONE).unwrap();
    doc.text_edit_key(EditKey::Enter, NONE).unwrap();
    assert_eq!(doc.project().library.symbols.get(sym).unwrap().texts.len(), 1);
    doc.undo();
    assert_eq!(doc.project().library.symbols.get(sym).unwrap().texts[0].text, "Note");
    assert!(!doc.begin_item_text_edit(item, 5));
}

#[test]
fn text_tools_start_typing_where_clicked() {
    use view::pcb_tools::{PcbPlacer, PcbTool};
    use view::sch_tools::{SchPlacer, SchTool};
    let (mut doc, _, _) = doc_with_text("x");
    let before = doc.project().schematic.texts.len();
    let mut sch = SchPlacer::default();
    sch.set_tool(SchTool::Text);
    sch.rotate();
    sch.hover(&mut doc, Some(Point::mm(20.0, 20.0)));
    sch.click(&mut doc, Point::mm(20.0, 20.0)).unwrap();
    assert!(doc.text_edit().is_some_and(|(_, e)| e.text.is_empty()));
    doc.with_text_edit(|e| e.insert("Note"));
    doc.text_edit_key(EditKey::Enter, NONE).unwrap();
    let texts = &doc.project().schematic.texts;
    assert_eq!(texts.len(), before + 1);
    let t = &texts.iter().last().unwrap().text;
    assert_eq!((t.text.as_str(), t.pos, t.size, t.rotation), ("Note", Point::mm(20.0, 20.0), mm(1.27), Angle::DEG90));
    // Nothing typed: nothing added.
    sch.click(&mut doc, Point::mm(30.0, 20.0)).unwrap();
    doc.commit_text_edit().unwrap();
    assert_eq!(doc.project().schematic.texts.len(), before + 1);

    // On the board: Top Silkscreen, F puts it on the bottom.
    let mut pcb = PcbPlacer::default();
    pcb.set_tool(PcbTool::Text);
    pcb.flip();
    let p = Point::mm(5.0, 5.0);
    pcb.hover(&mut doc, Some((p, p)), mm(0.3));
    pcb.click(&mut doc).unwrap();
    doc.with_text_edit(|e| e.insert("v1"));
    doc.commit_text_edit().unwrap();
    let b = doc.project().board.texts.iter().last().unwrap();
    assert_eq!((b.layer, b.text.text.as_str(), b.text.bold), (LayerId::BOTTOM_SILK, "v1", true));
}
