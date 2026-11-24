//! Export an example project, or a KiCad project imported first, as a KiCad project:
//! `cargo run -p kicad --example export_kicad -- led|amp|PROJECT.kicad_pro DIR`.

use std::path::Path;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [what, dir] = args.as_slice() else {
        eprintln!("usage: export_kicad led|amp|PROJECT.kicad_pro DIR");
        std::process::exit(2);
    };
    let (project, name) = match view::demo::Example::parse(what) {
        Some(e) => (e.project(), e.name().to_string()),
        None => {
            let pro = Path::new(what);
            let name = pro.file_stem().unwrap_or_default().to_string_lossy().to_string();
            (kicad::import_project(pro).unwrap_or_else(|e| panic!("{e}")).project, name)
        }
    };
    let export = kicad::export::export_project(&project, Path::new(dir), &name).unwrap_or_else(|e| panic!("{e}"));
    for l in export.report.lines() {
        println!("{l}");
    }
}
