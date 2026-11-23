//! Re-saves the stored example projects (`crates/view/demo/*.pcbproj`) in the current
//! file format, after a format change:
//! `cargo run -p view --example save_examples`.

use std::path::Path;
use view::demo::{Example, STORED_DIR};

fn main() {
    for e in Example::ALL {
        let Some((dir, _)) = e.stored() else { continue };
        let path = Path::new(STORED_DIR).join(dir);
        model::io::save_project(&e.project(), &path).expect("saved");
        println!("{}", path.display());
    }
}
