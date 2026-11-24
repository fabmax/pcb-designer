//! Re-saves the stored example projects (`crates/view/demo/*.pcbproj`) in the current
//! file format, after a format change:
//! `cargo run -p view --example save_examples`.
//!
//! With `--release VERSION`, saves every example to `crates/view/tests/saved/VERSION/`
//! instead: the files a release writes, which later versions must keep loading
//! (`tests/all/saved_files.rs`).

use std::path::Path;
use view::demo::{Example, STORED_DIR};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let [flag, version] = args.as_slice()
        && flag == "--release"
    {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/saved").join(version);
        for e in Example::ALL {
            let path = root.join(format!("{}.pcbproj", e.name()));
            model::io::save_project(&e.project(), &path).expect("saved");
            println!("{}", path.display());
        }
        return;
    }
    for e in Example::ALL {
        let Some((dir, _)) = e.stored() else { continue };
        let path = Path::new(STORED_DIR).join(dir);
        model::io::save_project(&e.project(), &path).expect("saved");
        println!("{}", path.display());
    }
}
