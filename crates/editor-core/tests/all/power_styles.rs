//! Power port glyphs: chosen per label, or derived from the name.

use editor_core::{Command, Editor};
use model::io::{FileKind, to_file_string};
use model::*;

#[test]
fn names_suggest_a_glyph() {
    for (name, style) in [
        ("GND", PowerStyle::Ground),
        ("AGND", PowerStyle::Ground),
        ("DGND", PowerStyle::Ground),
        ("GNDA", PowerStyle::Ground),
        ("gndpwr", PowerStyle::Ground),
        ("Earth", PowerStyle::Ground),
        ("Chassis", PowerStyle::Chassis),
        ("PE", PowerStyle::ProtectiveEarth),
        ("+5V", PowerStyle::Bar),
        ("VCC", PowerStyle::Bar),
        ("PEN", PowerStyle::Bar),
    ] {
        assert_eq!(PowerStyle::for_name(name), style, "{name}");
    }
}

#[test]
fn a_chosen_style_is_saved_and_overrides_the_name() {
    let mut ed = Editor::new(Project::new("t"));
    // As an agent sends it through `execute`.
    let cmd: Command = serde_json::from_str(
        r#"{"command": "add_label", "pos": [0, 0], "name": "GND", "kind": "power", "style": "signal_ground"}"#,
    )
    .unwrap();
    ed.execute(cmd).unwrap();
    let cmd: Command =
        serde_json::from_str(r#"{"command": "add_label", "pos": [0, 0], "name": "GND", "kind": "power"}"#).unwrap();
    ed.execute(cmd).unwrap();
    let s = &ed.project().schematic;
    let styles: Vec<_> = s.labels.iter().map(|l| (l.style, l.power_style())).collect();
    assert_eq!(styles, [(Some(PowerStyle::SignalGround), PowerStyle::SignalGround), (None, PowerStyle::Ground)]);

    // Only the chosen one is written; a missing field reads back as "by name".
    let file = to_file_string(FileKind::Schematic, s).unwrap();
    assert_eq!(file.matches("\"style\"").count(), 1, "{file}");
    assert!(file.contains("\"style\": \"signal_ground\""));
    let back: Schematic = serde_json::from_str(&file).unwrap();
    assert_eq!(&back, s);
}
