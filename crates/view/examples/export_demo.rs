//! Save the demo project into a folder and export its Gerber files:
//! `cargo run -p view --example export_demo -- DIR`. Development aid.

fn main() {
    let dir = std::env::args().nth(1).unwrap_or_else(|| "demo.pcbproj".into());
    let mut doc = view::Document::new(view::demo::demo_project());
    doc.save_as(&dir).expect("save");
    let export = doc.export_gerber().expect("export");
    for f in &export.manifest.files {
        println!("{} ({})", export.dir.join(&f.name).display(), f.label);
    }
    println!("{} design rule warnings", export.warnings.len());
}
