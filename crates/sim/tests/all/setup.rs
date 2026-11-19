//! The simulation setup as project content: undo, validation, files.

use editor_core::{Command, Editor};
use model::*;

fn setup() -> Simulation {
    Simulation {
        analyses: vec![
            Analysis::new("Transient", AnalysisKind::Tran { stop: 1e-3, step: None, start: 0.0, max_step: None }),
            Analysis::new("Bode", AnalysisKind::Ac { start: 10.0, stop: 1e6, points_per_decade: 50 }),
        ],
        supplies: vec![Supply { net: "+5V".into(), voltage: 5.0 }],
        ..Default::default()
    }
}

#[test]
fn set_simulation_undoes_and_saves() {
    let mut ed = Editor::new(Project::new("t"));
    let setup = setup();
    let changes = ed.execute(Command::SetSimulation { simulation: setup.clone() }).unwrap();
    assert!(changes.simulation_changed());
    assert!(!changes.erc_changed() && !changes.board_setup_changed());
    assert_eq!(ed.project().schematic.simulation, setup);

    let dir = tempfile::tempdir().unwrap();
    io::save_project(ed.project(), dir.path()).unwrap();
    let loaded = io::load_project(dir.path()).unwrap();
    assert_eq!(loaded.schematic.simulation, setup);

    ed.undo().unwrap();
    assert!(ed.project().schematic.simulation.is_default());
    // An empty setup isn't written.
    io::save_project(ed.project(), dir.path()).unwrap();
    let text = std::fs::read_to_string(dir.path().join(io::SCHEMATIC_FILE)).unwrap();
    assert!(!text.contains("simulation"), "{text}");
}

#[test]
fn bad_setups_are_refused() {
    let mut ed = Editor::new(Project::new("t"));
    let refused = |ed: &mut Editor, f: &dyn Fn(&mut Simulation)| {
        let mut s = setup();
        f(&mut s);
        ed.execute(Command::SetSimulation { simulation: s }).unwrap_err().to_string()
    };
    assert!(refused(&mut ed, &|s| s.analyses[1].name = "transient".into()).contains("used twice"));
    assert!(refused(&mut ed, &|s| s.analyses[0].name = " ".into()).contains("needs a name"));
    assert!(
        refused(&mut ed, &|s| s.analyses[0].kind =
            AnalysisKind::Tran { stop: 0.0, step: None, start: 0.0, max_step: None })
        .contains("stop > 0")
    );
    assert!(
        refused(&mut ed, &|s| s.analyses[1].kind = AnalysisKind::Ac { start: 0.0, stop: 1.0, points_per_decade: 10 })
            .contains("0 < start < stop")
    );
    assert!(
        refused(&mut ed, &|s| s.analyses[0].kind =
            AnalysisKind::Dc { source: Id::new(), start: 0.0, stop: 1.0, step: 0.1 })
        .contains("not a component")
    );
    assert!(ed.project().schematic.simulation.is_default());
}
