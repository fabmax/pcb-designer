//! The project's list of library files: add, remove, relink, undo.

use std::path::PathBuf;

use editor_core::{Command, Editor};
use model::Project;

fn abs(p: &str) -> PathBuf {
    std::env::temp_dir().join(p)
}

#[test]
fn add_remove_relink_with_undo() {
    let mut ed = Editor::new(Project::new("p"));
    let a = abs("libs/a.pcblib");
    let b = abs("libs/b.pcblib");
    let changes = ed.execute(Command::AddLibrary { path: a.clone() }).unwrap();
    assert_eq!(changes.libraries, Some((vec![], vec![a.clone()])));
    assert!(ed.is_modified());
    ed.execute(Command::AddLibrary { path: b.clone() }).unwrap();
    assert_eq!(ed.project().libraries, [a.clone(), b.clone()]);

    let c = abs("moved/c.pcblib");
    ed.execute(Command::RelinkLibrary { from: a.clone(), to: c.clone() }).unwrap();
    assert_eq!(ed.project().libraries, [c.clone(), b.clone()], "relinking keeps the position");
    ed.execute(Command::RemoveLibrary { path: b.clone() }).unwrap();
    assert_eq!(ed.project().libraries, std::slice::from_ref(&c));

    let undone = ed.undo().unwrap();
    assert_eq!(undone.libraries, Some((vec![c.clone()], vec![c.clone(), b.clone()])));
    ed.undo();
    assert_eq!(ed.project().libraries, [a.clone(), b.clone()]);
    ed.redo();
    ed.redo();
    assert_eq!(ed.project().libraries, [c]);
}

#[test]
fn refuses_duplicates_relative_paths_and_unknown_entries() {
    let mut ed = Editor::new(Project::new("p"));
    let a = abs("libs/a.pcblib");
    ed.execute(Command::AddLibrary { path: a.clone() }).unwrap();
    // The same file spelled differently is still a duplicate.
    let spelled = abs("libs/../libs/./a.pcblib");
    assert!(ed.execute(Command::AddLibrary { path: spelled }).is_err());
    assert!(ed.execute(Command::AddLibrary { path: "rel/b.pcblib".into() }).is_err());
    assert!(ed.execute(Command::RemoveLibrary { path: abs("nope.pcblib") }).is_err());
    let b = abs("libs/b.pcblib");
    ed.execute(Command::AddLibrary { path: b.clone() }).unwrap();
    assert!(ed.execute(Command::RelinkLibrary { from: a.clone(), to: b }).is_err());
    assert_eq!(ed.project().libraries.len(), 2);
    assert_eq!(ed.history().undo_label(), Some("Add library b.pcblib"));
}
