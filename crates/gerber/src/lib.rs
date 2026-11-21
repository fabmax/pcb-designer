//! Fabrication output: Gerber X2 layers, Excellon drill files and a job file of the
//! board, written to `<project>/output/gerber/` with a zip for the manufacturer and an
//! `export.json` that records what the export was made from; the bill of materials and
//! component positions go to `<project>/output/assembly/`.

pub mod assembly;
mod bridge;
pub mod excellon;
pub mod job;
pub mod layers;
pub mod pdf;
pub mod reader;
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
/// The assembly folder inside a project folder.
pub const ASSEMBLY_DIR: &str = "output/assembly";
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

impl OutputFile {
    fn listed(&self) -> ManifestFile {
        ManifestFile { name: self.name.clone(), kind: self.kind, label: self.label.clone() }
    }
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
    /// The files in the assembly folder.
    pub assembly: Vec<ManifestFile>,
}

/// Stable hash of everything the export depends on (board, schematic nets, library
/// footprints and texts), to tell whether an export is outdated.
pub fn design_hash(project: &Project) -> String {
    model::Fnv1a::of_json(&(&project.board, &project.schematic, &project.library)).unwrap_or_default()
}

/// File name stem: the project name with only file-name-safe characters.
pub fn stem(project: &Project) -> String {
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
    // Blind and buried vias: a file per span, named by its layers (front, in1, …, back).
    let copper: Vec<&BoardLayer> = board.copper_layers().collect();
    let number = |l: LayerId| copper.iter().position(|c| c.id == l).map(|i| i + 1);
    for ([from, to], holes) in excellon::span_holes(project) {
        let (Some(a), Some(b)) = (number(from), number(to)) else { continue };
        let short = |n: usize| match n {
            1 => "front".to_string(),
            n if n == copper_count => "back".to_string(),
            n => format!("in{}", n - 1),
        };
        let kind = if a == 1 || b == copper_count { "Blind" } else { "Buried" };
        files.push(OutputFile {
            name: format!("{stem}-{}-{}.drl", short(a), short(b)),
            kind: FileKind::SpanDrill { from, to },
            label: format!("{kind} Drill {}–{}", copper[a - 1].name, copper[b - 1].name),
            content: excellon::span_drill_file(&holes, a, b, copper_count),
        });
    }
    let content = job::job_file(project, &files, &job::now());
    files.push(OutputFile {
        name: format!("{stem}-job.gbrjob"),
        kind: FileKind::Job,
        label: "Job File".into(),
        content,
    });
    Ok(files)
}

/// The assembly files: bill of materials and component positions.
pub fn generate_assembly(project: &Project) -> Vec<OutputFile> {
    let stem = stem(project);
    vec![
        OutputFile {
            name: format!("{stem}-bom.csv"),
            kind: FileKind::Bom,
            label: "Bill of Materials".into(),
            content: assembly::bom(project, project.board.assembly),
        },
        OutputFile {
            name: format!("{stem}-positions.csv"),
            kind: FileKind::Positions,
            label: "Component Positions".into(),
            content: assembly::positions(project, project.board.assembly),
        },
    ]
}

/// The export folder of a project folder.
pub fn gerber_dir(project_dir: &Path) -> PathBuf {
    project_dir.join(GERBER_DIR)
}

/// The assembly folder of a project folder.
pub fn assembly_dir(project_dir: &Path) -> PathBuf {
    project_dir.join(ASSEMBLY_DIR)
}

/// The manifest of the last export, if there is one.
pub fn read_manifest(project_dir: &Path) -> Option<Manifest> {
    let text = std::fs::read_to_string(gerber_dir(project_dir).join(MANIFEST)).ok()?;
    serde_json::from_str(&text).ok()
}

/// Write the files into `<project_dir>/output/gerber/` (replacing what was there) with
/// `<name>-gerber.zip` next to that folder and the manifest inside it, and the assembly
/// files into `<project_dir>/output/assembly/`.
pub fn export(project: &Project, copper: &BoardConnectivity, project_dir: &Path) -> Result<Manifest, ExportError> {
    let files = generate(project, copper)?;
    let assembly = generate_assembly(project);
    let dir = gerber_dir(project_dir);
    for (d, files) in [(&dir, &files), (&assembly_dir(project_dir), &assembly)] {
        if d.exists() {
            std::fs::remove_dir_all(d)?;
        }
        std::fs::create_dir_all(d)?;
        for f in files {
            std::fs::write(d.join(&f.name), &f.content)?;
        }
    }
    let zip_name = format!("{}-gerber.zip", stem(project));
    let entries: Vec<(String, Vec<u8>)> =
        files.iter().map(|f| (f.name.clone(), f.content.clone().into_bytes())).collect();
    std::fs::write(dir.parent().expect("output folder").join(&zip_name), zip::zip(&entries)?)?;
    let manifest = Manifest {
        hash: design_hash(project),
        zip: zip_name,
        files: files.iter().map(OutputFile::listed).collect(),
        assembly: assembly.iter().map(OutputFile::listed).collect(),
    };
    std::fs::write(dir.join(MANIFEST), serde_json::to_string_pretty(&manifest).expect("serializable"))?;
    Ok(manifest)
}
