//! The bundled Built-in library (`crates/model/library/builtin.pcblib`):
//!
//! - `cargo run -p model --example gen_builtin` rewrites its generated items, keeping the
//!   hand-made ones.
//! - `… -- edit FILE` writes a copy with an id of its own, to open in the app (or through
//!   MCP) next to the Built-in library and draw hand-made parts in.
//! - `… -- take FILE` brings such a copy back: its hand-made items replace the bundled
//!   ones, the generated items are generated afresh (edits to them are reported and
//!   dropped: change the generator instead).

use model::builtin::{EDIT_LIBRARY_ID, FILE_PATH, LIBRARY_ID, generate, is_generated};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let load = |path: &str| model::io::load_library(path).unwrap_or_else(|e| panic!("{path}: {e}"));
    let mut file = load(FILE_PATH);
    match args.iter().map(String::as_str).collect::<Vec<_>>()[..] {
        [] => {}
        ["edit", path] => {
            file.id = Some(EDIT_LIBRARY_ID);
            file.name = "Built-in (editing)".into();
            model::io::save_library(&file, path).expect("the copy saves");
            println!("{path}: open it, edit hand-made parts, save, then run `… -- take {path}`");
            return;
        }
        ["take", path] => {
            let copy = load(path);
            let generated = generate::library();
            let edited: Vec<String> = copy
                .parts
                .iter()
                .filter(|p| is_generated(p.id) && generated.parts.get(p.id) != Some(*p))
                .map(|p| p.name.clone())
                .chain(
                    copy.footprints
                        .iter()
                        .filter(|f| is_generated(f.id) && generated.footprints.get(f.id) != Some(*f))
                        .map(|f| f.name.clone()),
                )
                .collect();
            if !edited.is_empty() {
                eprintln!("dropped edits to generated items (change the generator instead): {}", edited.join(", "));
            }
            file = copy;
            file.id = Some(LIBRARY_ID);
        }
        _ => panic!("usage: gen_builtin [edit FILE | take FILE]"),
    }
    let out = generate::merge(&file);
    model::io::save_library(&out, FILE_PATH).expect("the bundled library saves");
    let hand = out.parts.iter().filter(|p| !is_generated(p.id)).count();
    println!(
        "{FILE_PATH}: {} parts ({hand} hand-made), {} symbols, {} footprints",
        out.parts.len(),
        out.symbols.len(),
        out.footprints.len()
    );
}
