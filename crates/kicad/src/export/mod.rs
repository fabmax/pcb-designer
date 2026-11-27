//! Export of a project as a KiCad 9 project, and of a library as KiCad libraries.
//!
//! A project becomes a folder: `NAME.kicad_pro`, the root `NAME.kicad_sch` with a file
//! per sub-sheet, `NAME.kicad_pcb`, and a project library (`NAME.kicad_sym`,
//! `NAME.pretty`, and the `sym-lib-table` / `fp-lib-table` naming them) holding every
//! symbol and footprint the files use. Ids are kept as KiCad UUIDs, so the board's
//! footprints stay linked to their symbols and exporting again gives the same files.
//!
//! KiCad numbers a symbol's pins by its footprint's pads, while our parts map pins to
//! pads per variant: every variant becomes a symbol of its own (`Part_Variant`).

mod lib;
mod pcb;
mod pro;
mod sch;
pub mod sx;

use std::path::{Path, PathBuf};

use model::*;

use lib::Names;
pub use lib::{item_name, pin_numbers};
use sx::l;

use crate::project::Report;

/// The files of an export (paths relative to its folder) and what it couldn't keep.
#[derive(Clone, Debug, Default)]
pub struct Export {
    pub files: Vec<(PathBuf, String)>,
    pub report: Report,
}

/// The KiCad project of `project`, in memory. Its files are named after `name`.
pub fn render_project(project: &Project, name: &str) -> Export {
    let name = item_name(name);
    let mut report = Report::default();
    let names = Names::of(&name, &project.library);
    let sch = sch::export(project, &names, &name, &mut report);
    let mut files: Vec<(PathBuf, String)> = Vec::new();
    files.push((format!("{name}.kicad_pro").into(), pro::export(project, &format!("{name}.kicad_pro"))));
    for (f, text) in &sch.files {
        files.push((f.into(), text.clone()));
    }
    files.push((format!("{name}.kicad_pcb").into(), pcb::export(project, &names, &sch, &mut report)));

    // The project library: every part variant, the power symbols, every footprint.
    let symbols = lib::symbol_library(&project.library, &names, &mut report);
    let symbols = with_power_symbols(&symbols, &sch.power_symbols);
    files.push((format!("{name}.kicad_sym").into(), symbols));
    for fp in project.library.footprints.iter() {
        if let Some(n) = names.footprint(fp.id) {
            files.push((
                Path::new(&format!("{name}.pretty")).join(format!("{n}.kicad_mod")),
                lib::footprint_file(fp, n, &mut report),
            ));
        }
    }
    files.push(("sym-lib-table".into(), lib_table("sym_lib_table", &name, &format!("{name}.kicad_sym"))));
    files.push(("fp-lib-table".into(), lib_table("fp_lib_table", &name, &format!("{name}.pretty"))));

    if project.library.parts.iter().any(|p| p.variants.len() > 1 && !p.is_board_part()) {
        report.note("parts with several footprint variants became one KiCad symbol per variant");
    }
    if !project.schematic.simulation.analyses.is_empty() || project.library.parts.iter().any(|p| p.sim.is_some()) {
        report.skip("simulation models and setups (not exported yet)");
    }
    if !project.schematic.erc.exclusions.is_empty() || !project.board.drc.exclusions.is_empty() {
        report.skip("ERC and DRC exclusions");
    }
    Export { files, report }
}

/// The KiCad libraries of `library`: `NAME.kicad_sym` and `NAME.pretty`.
pub fn render_library(library: &Library, name: &str) -> Export {
    let name = item_name(name);
    let mut report = Report::default();
    let names = Names::of(&name, library);
    let mut files = vec![(format!("{name}.kicad_sym").into(), lib::symbol_library(library, &names, &mut report))];
    for fp in library.footprints.iter() {
        if let Some(n) = names.footprint(fp.id) {
            files.push((
                Path::new(&format!("{name}.pretty")).join(format!("{n}.kicad_mod")),
                lib::footprint_file(fp, n, &mut report),
            ));
        }
    }
    if library.parts.iter().any(|p| p.variants.len() > 1 && !p.is_board_part()) {
        report.note("parts with several footprint variants became one KiCad symbol per variant");
    }
    Export { files, report }
}

/// Export `project` into the folder `dir` as `NAME.kicad_pro` and the rest. The folder
/// must be new, empty, or hold a KiCad project of that name already (which is replaced).
pub fn export_project(project: &Project, dir: &Path, name: &str) -> Result<Export, String> {
    let export = render_project(project, name);
    let pro = format!("{}.kicad_pro", item_name(name));
    check_target(dir, &[&pro])?;
    write(dir, &export, &format!("{}.pretty", item_name(name)))?;
    Ok(export)
}

/// Export `library` into the folder `dir` as `NAME.kicad_sym` and `NAME.pretty`. The
/// folder must be new, empty, or hold KiCad libraries of that name already.
pub fn export_library(library: &Library, dir: &Path, name: &str) -> Result<Export, String> {
    let export = render_library(library, name);
    let n = item_name(name);
    check_target(dir, &[&format!("{n}.kicad_sym"), &format!("{n}.pretty")])?;
    write(dir, &export, &format!("{n}.pretty"))?;
    Ok(export)
}

/// A folder may be written when it doesn't exist, is empty, or has one of `ours` in it
/// (an earlier export, which is overwritten).
fn check_target(dir: &Path, ours: &[&str]) -> Result<(), String> {
    let entries: Vec<String> = match model::fs::list_dir(dir) {
        Ok(paths) => paths.iter().filter_map(|p| Some(p.file_name()?.to_string_lossy().into_owned())).collect(),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(format!("{}: {e}", dir.display())),
    };
    if entries.is_empty() || entries.iter().any(|e| ours.contains(&e.as_str())) {
        return Ok(());
    }
    Err(format!("{} isn't empty and holds no {}; choose an empty folder or an earlier export", dir.display(), ours[0]))
}

fn write(dir: &Path, export: &Export, pretty: &str) -> Result<(), String> {
    let err = |p: &Path, e: std::io::Error| format!("{}: {e}", p.display());
    model::fs::create_dir_all(dir).map_err(|e| err(dir, e))?;
    // Footprints of an earlier export that are gone now.
    let pretty = dir.join(pretty);
    if let Ok(paths) = model::fs::list_dir(&pretty) {
        for p in paths {
            if p.extension().is_some_and(|x| x == "kicad_mod") {
                model::fs::remove_file(&p).map_err(|x| err(&p, x))?;
            }
        }
    }
    for (rel, text) in &export.files {
        let path = dir.join(rel);
        if let Some(parent) = path.parent() {
            model::fs::create_dir_all(parent).map_err(|e| err(parent, e))?;
        }
        model::fs::write(&path, text).map_err(|e| err(&path, e))?;
    }
    Ok(())
}

fn lib_table(kind: &str, name: &str, file: &str) -> String {
    l(kind)
        .push(l("version").sym("7"))
        .push(
            l("lib")
                .push(l("name").str(name))
                .push(l("type").str("KiCad"))
                .push(l("uri").str(format!("${{KIPRJMOD}}/{file}")))
                .push(l("options").str(""))
                .push(l("descr").str("Exported from PCB Designer")),
        )
        .to_text()
}

/// The symbol library with the power symbols added before its closing parenthesis.
fn with_power_symbols(library: &str, power: &[sx::X]) -> String {
    let Some(end) = library.rfind(')') else { return library.to_string() };
    let mut out = library[..end].trim_end().to_string();
    for p in power {
        out.push_str("\n\t");
        let t = p.to_text();
        out.push_str(&t.trim_end().replace('\n', "\n\t"));
    }
    out.push_str("\n)\n");
    out
}
