//! Fabrication output: Gerber X2 layers and Excellon drill files of the board, written
//! to `<project>/output/gerber/` with a zip for the manufacturer and an `export.json`
//! that records what the export was made from.

mod bridge;
pub mod excellon;
pub mod layers;
pub mod writer;
mod zip;

use std::path::{Path, PathBuf};

use editor_core::copper::BoardConnectivity;
use model::*;
use serde::{Deserialize, Serialize};

pub use bridge::bridge_holes;
pub use layers::FileKind;

/// The export folder inside a project folder.
pub const GERBER_DIR: &str = "output/gerber";
/// The manifest inside the export folder.
pub const MANIFEST: &str = "export.json";

#[derive(Debug, thiserror::Error)]
pub enum ExportError {
    #[error("the board has no outline (draw one on the Board Outline layer)")]
    NoOutline,
    #[error("save the project first: the export goes into the project folder")]
    NotSaved,
    #[error("{0}")]
    Io(#[from] std::io::Error),
}

/// One generated file.
#[derive(Clone, Debug)]
pub struct OutputFile {
    pub name: String,
    pub kind: FileKind,
    /// What it holds, e.g. "Top Copper" or "Plated Drill".
    pub label: String,
    pub content: String,
}

/// A file listed in the manifest.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ManifestFile {
    pub name: String,
    pub kind: FileKind,
    pub label: String,
}

/// `export.json`: the [`design_hash`] the files were made from, and the files.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Manifest {
    pub hash: String,
    pub zip: String,
    pub files: Vec<ManifestFile>,
}

/// Stable hash of everything the export depends on (board, schematic nets, library
/// footprints and texts), to tell whether an export is outdated.
pub fn design_hash(project: &Project) -> String {
    let json = serde_json::to_string(&(&project.board, &project.schematic, &project.library)).unwrap_or_default();
    // FNV-1a, 64 bit, as `Library::content_hash`.
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in json.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{h:016x}")
}

/// File name stem: the project name with only file-name-safe characters.
fn stem(project: &Project) -> String {
    let s: String = project
        .name
        .chars()
        .map(|c| if c.is_alphanumeric() || matches!(c, '-' | '_' | '.') { c } else { '_' })
        .collect();
    if s.is_empty() { "board".into() } else { s }
}

/// All files of the board. `copper` must be up to date (its zone fills are written).
pub fn generate(project: &Project, copper: &BoardConnectivity) -> Result<Vec<OutputFile>, ExportError> {
    let board = &project.board;
    if !board.graphics.iter().any(|g| g.layer == LayerId::OUTLINE) {
        return Err(ExportError::NoOutline);
    }
    let stem = stem(project);
    let mut files = Vec::new();
    for layer in layers::exported_layers(board) {
        let (suffix, _) = layers::layer_naming(board, layer).expect("exported layers are named");
        let name = board.layers.iter().find(|l| l.id == layer).map_or(suffix.clone(), |l| l.name.clone());
        let label = if layer.is_copper() { format!("{name} Copper") } else { name };
        files.push(OutputFile {
            name: format!("{stem}-{suffix}.gbr"),
            kind: FileKind::Layer(layer),
            label,
            content: layers::layer_file(project, copper, layer),
        });
    }
    let copper_count = board.copper_layers().count();
    let (plated, npth) = excellon::holes(project);
    for (holes, is_plated) in [(plated, true), (npth, false)] {
        if holes.is_empty() {
            continue;
        }
        let (suffix, kind, label) = if is_plated {
            ("PTH", FileKind::PlatedDrill, "Plated Drill")
        } else {
            ("NPTH", FileKind::NonPlatedDrill, "Non-plated Drill")
        };
        files.push(OutputFile {
            name: format!("{stem}-{suffix}.drl"),
            kind,
            label: label.into(),
            content: excellon::drill_file(&holes, is_plated, copper_count),
        });
    }
    Ok(files)
}

/// The export folder of a project folder.
pub fn gerber_dir(project_dir: &Path) -> PathBuf {
    project_dir.join(GERBER_DIR)
}

/// The manifest of the last export, if there is one.
pub fn read_manifest(project_dir: &Path) -> Option<Manifest> {
    let text = std::fs::read_to_string(gerber_dir(project_dir).join(MANIFEST)).ok()?;
    serde_json::from_str(&text).ok()
}

/// Write the files into `<project_dir>/output/gerber/` (replacing what was there) with
/// `<name>-gerber.zip` next to that folder and the manifest inside it.
pub fn export(project: &Project, copper: &BoardConnectivity, project_dir: &Path) -> Result<Manifest, ExportError> {
    let files = generate(project, copper)?;
    let dir = gerber_dir(project_dir);
    if dir.exists() {
        std::fs::remove_dir_all(&dir)?;
    }
    std::fs::create_dir_all(&dir)?;
    for f in &files {
        std::fs::write(dir.join(&f.name), &f.content)?;
    }
    let zip_name = format!("{}-gerber.zip", stem(project));
    let entries: Vec<(String, Vec<u8>)> =
        files.iter().map(|f| (f.name.clone(), f.content.clone().into_bytes())).collect();
    std::fs::write(dir.parent().expect("output folder").join(&zip_name), zip::zip(&entries)?)?;
    let manifest = Manifest {
        hash: design_hash(project),
        zip: zip_name,
        files: files
            .iter()
            .map(|f| ManifestFile { name: f.name.clone(), kind: f.kind, label: f.label.clone() })
            .collect(),
    };
    std::fs::write(dir.join(MANIFEST), serde_json::to_string_pretty(&manifest).expect("serializable"))?;
    Ok(manifest)
}
