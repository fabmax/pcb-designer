//! Import a KiCad project and print the report and footprint definitions:
//! `cargo run -p kicad --example import_project -- PROJECT.kicad_pro [--footprints]`.

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let path = std::path::PathBuf::from(&args[0]);
    match kicad::import_project(&path) {
        Ok(i) => {
            for l in i.report.lines() {
                println!("{l}");
            }
            if args.iter().any(|a| a == "--footprints") {
                for f in i.project.library.footprints.iter() {
                    println!("{}\n{}", f.name, serde_json::to_string(f).unwrap());
                }
            }
        }
        Err(e) => println!("error: {e}"),
    }
}
