//! Fabrication outputs: the Gerber and assembly folders as the Project panel shows them
//! (Outputs › Gerber › files and Outputs › Assembly › files, marked outdated when the
//! design changed since), and the read-only Gerber tab that shows the files as read back
//! from disk.

use std::path::{Path, PathBuf};

use egui::{Align2, RichText};
use model::{Coord, Point};
use view::gerber::{self, FileKind, Manifest, assembly_dir, design_hash, gerber_dir, read_manifest};
use view::measure::Measure;
use view::{Document, GerberView};

use crate::canvas::Canvas;
use crate::icons::Icon;
use crate::keymap::{Action, Keymap};
use crate::project_panel::{self, Info, NodeId, TreeNode};
use crate::toolbar::ToolButton;
use crate::{theme, toolbar};

/// What the export folder holds, as last scanned.
#[derive(Default)]
pub struct Outputs {
    /// The export folder (`None`: the project isn't saved).
    pub dir: Option<PathBuf>,
    pub manifest: Option<Manifest>,
    /// Files in the folder, sorted (the manifest's order when there is one).
    pub files: Vec<String>,
    /// The assembly folder, and its files in the same order.
    pub assembly_dir: Option<PathBuf>,
    pub assembly: Vec<String>,
    /// The output folder (`None`: the project isn't saved) and the schematic PDF in it,
    /// if exported.
    pub output_dir: Option<PathBuf>,
    pub schematic_pdf: Option<PathBuf>,
    /// The design hash and the document revision it was computed at.
    hash: Option<(u64, String)>,
}

impl Outputs {
    /// Read the folder of the project at `project_dir` again.
    pub fn scan(&mut self, project_dir: Option<&Path>) {
        self.dir = project_dir.map(gerber_dir);
        self.assembly_dir = project_dir.map(assembly_dir);
        self.manifest = project_dir.and_then(read_manifest);
        self.files = list(self.dir.as_deref(), self.manifest.as_ref().map(|m| &m.files[..]));
        self.assembly = list(self.assembly_dir.as_deref(), self.manifest.as_ref().map(|m| &m.assembly[..]));
        self.output_dir = project_dir.map(|d| d.join("output"));
        self.schematic_pdf = self.output_dir.as_deref().and_then(find_schematic_pdf);
    }

    pub fn exported(&self) -> bool {
        !self.files.is_empty()
    }

    /// The export folders that exist.
    pub fn dirs(&self) -> Vec<PathBuf> {
        [&self.dir, &self.assembly_dir].into_iter().flatten().filter(|d| model::fs::exists(d)).cloned().collect()
    }

    /// The design changed since the export. The hash is recomputed when the document
    /// changed, but not while `busy` (dragging): it serializes the whole design.
    pub fn outdated(&mut self, doc: &Document, busy: bool) -> bool {
        let Some(m) = &self.manifest else { return false };
        if self.hash.as_ref().is_none_or(|(rev, _)| *rev != doc.revision) && !busy {
            self.hash = Some((doc.revision, design_hash(doc.project())));
        }
        self.hash.as_ref().is_some_and(|(_, h)| *h != m.hash)
    }

    fn listed(&self, name: &str) -> Option<&gerber::ManifestFile> {
        let m = self.manifest.as_ref()?;
        m.files.iter().chain(&m.assembly).find(|f| f.name == name)
    }

    /// The nodes of the files in `dir`.
    fn file_nodes(&self, dir: &Option<PathBuf>, files: &[String], assembly: bool) -> Vec<TreeNode> {
        files
            .iter()
            .map(|name| {
                let path = dir.as_ref().map(|d| d.join(name));
                let listed = self.listed(name);
                let (label, detail) = match listed {
                    Some(f) => (f.label.clone(), name.clone()),
                    None => (name.clone(), String::new()),
                };
                let hash =
                    project_panel::path_hash(&Path::new(if assembly { "assembly" } else { "gerber" }).join(name));
                // Unlisted files in the Gerber folder may be images (another tool's export).
                let (id, icon) = match listed.map(|f| f.kind) {
                    Some(k) if k.is_image() => (NodeId::OutputFile(hash), Icon::Layers),
                    None if !assembly => (NodeId::OutputFile(hash), Icon::Layers),
                    Some(FileKind::Job) => (NodeId::DataFile(hash), Icon::FileCode),
                    _ => (NodeId::DataFile(hash), Icon::Spreadsheet),
                };
                TreeNode::new(id, label, Info { detail, path, tip: name.clone(), ..Info::icon(icon) })
            })
            .collect()
    }

    /// The Outputs node.
    pub fn node(&self, outdated: bool) -> TreeNode {
        let files = self.file_nodes(&self.dir, &self.files, false);
        let gerber_info = Info {
            detail: match (self.exported(), outdated) {
                (false, _) => "not exported".into(),
                (true, true) => format!("outdated · {}", project_panel::count(self.files.len(), "file")),
                (true, false) => project_panel::count(self.files.len(), "file"),
            },
            warn: outdated,
            dim: !self.exported(),
            editable: self.dir.is_some(),
            flag: self.exported(),
            tip: if outdated {
                "The design changed since these files were exported".into()
            } else {
                "Gerber X2 and Excellon drill files for manufacturing (File › Export Gerber files)".into()
            },
            path: self.dir.clone(),
            ..Info::icon(Icon::Board)
        };
        let gerber = TreeNode::new(NodeId::Gerber, "Gerber", gerber_info).with_children(files);
        let done = !self.assembly.is_empty();
        let assembly_info = Info {
            detail: match (done, outdated) {
                (false, _) => "not exported".into(),
                (true, true) => format!("outdated · {}", project_panel::count(self.assembly.len(), "file")),
                (true, false) => project_panel::count(self.assembly.len(), "file"),
            },
            warn: done && outdated,
            dim: !done,
            editable: self.assembly_dir.is_some(),
            flag: done,
            tip: "Bill of materials and component positions (pick and place) for assembly, written \
                  with the Gerber files"
                .into(),
            path: self.assembly_dir.clone(),
            ..Info::icon(Icon::Part)
        };
        let assembly = TreeNode::new(NodeId::Assembly, "Assembly", assembly_info).with_children(self.file_nodes(
            &self.assembly_dir,
            &self.assembly,
            true,
        ));
        let pdf = &self.schematic_pdf;
        let pdf_info = Info {
            detail: pdf
                .as_ref()
                .and_then(|p| p.file_name())
                .map_or("not exported".into(), |n| n.to_string_lossy().into()),
            dim: pdf.is_none(),
            editable: self.output_dir.is_some(),
            flag: pdf.is_some(),
            tip: match pdf {
                Some(_) => {
                    "The schematic as a PDF, a page per sheet: double-click opens it in the system's viewer".into()
                }
                None => "The schematic as a PDF, a page per sheet (File › Export schematic PDF)".into(),
            },
            path: pdf.clone(),
            ..Info::icon(Icon::FilePdf)
        };
        let pdf = TreeNode::new(NodeId::SchematicPdf, "Schematic PDF", pdf_info);
        TreeNode::new(NodeId::Outputs, "Outputs", Info::icon(Icon::Folder)).with_children(vec![gerber, assembly, pdf])
    }
}

/// The schematic PDF in the output folder (`<name>-schematic.pdf`; the newest if the
/// project was renamed since).
fn find_schematic_pdf(output: &Path) -> Option<PathBuf> {
    let entries = model::fs::list_dir(output).ok()?;
    entries
        .into_iter()
        .filter(|p| p.file_name().is_some_and(|n| n.to_string_lossy().ends_with("-schematic.pdf")))
        .max_by_key(|p| model::fs::modified(p))
}

/// The files in `dir` (not the manifest or hidden ones): those `listed` first in their
/// order, then the others sorted.
fn list(dir: Option<&Path>, listed: Option<&[gerber::ManifestFile]>) -> Vec<String> {
    let mut on_disk: Vec<String> = match dir.map(model::fs::list_dir) {
        Some(Ok(entries)) => entries
            .iter()
            .filter(|p| model::fs::is_file(p))
            .filter_map(|p| Some(p.file_name()?.to_string_lossy().into_owned()))
            .filter(|n| n != gerber::MANIFEST && !n.starts_with('.'))
            .collect(),
        _ => Vec::new(),
    };
    on_disk.sort();
    match listed {
        Some(m) => {
            let mut v: Vec<String> = m.iter().map(|f| f.name.clone()).filter(|n| on_disk.contains(n)).collect();
            v.extend(on_disk.into_iter().filter(|n| !m.iter().any(|f| f.name == *n)));
            v
        }
        None => on_disk,
    }
}

/// The Gerber tab.
pub struct GerberPane {
    pub canvas: Canvas,
    pub view: Option<GerberView>,
    /// Why the files couldn't be shown.
    pub error: Option<String>,
    pub grid: model::Coord,
    pub snap: bool,
    /// The Measure tool is on (else Select: hover readouts only).
    pub measuring: bool,
    pub measure: Measure,
}

impl Default for GerberPane {
    fn default() -> Self {
        GerberPane {
            canvas: Canvas::new(view::style::Editor::Pcb, 0.5),
            view: None,
            error: None,
            grid: model::mm(0.5),
            snap: false,
            measuring: false,
            measure: Measure::default(),
        }
    }
}

impl GerberPane {
    /// Read the files from `dir` again (keeping the camera), showing only `only` if given.
    pub fn load(&mut self, dir: &Path, only: Option<&str>) {
        let first = self.view.is_none();
        match GerberView::load(dir) {
            Ok(mut v) => {
                if let Some(old) = &self.view {
                    // Keep what was hidden, by file name.
                    for f in &v.files {
                        if let Some(o) = old.files.iter().find(|o| o.name == f.name) {
                            v.scene.layer_mut(f.render_layer).visible = old.scene.layer(o.render_layer).visible;
                        }
                    }
                }
                if only.is_some() {
                    v.show_only(only);
                }
                self.view = Some(v);
                self.error = None;
                if first {
                    self.canvas.request_fit();
                }
            }
            Err(e) => {
                self.view = None;
                self.error = Some(format!("Can't read {}: {e}", dir.display()));
            }
        }
    }

    pub fn set_measuring(&mut self, on: bool) {
        self.measuring = on;
        self.measure.clear();
    }

    /// Esc: clear the measurement, else go back to Select.
    pub fn escape(&mut self) {
        if !self.measure.escape() {
            self.measuring = false;
        }
    }

    fn snap_point(&self, raw: Point) -> Point {
        if !self.snap || self.grid <= 0 {
            return raw;
        }
        let g = self.grid;
        Point::new((raw.x as f64 / g as f64).round() as Coord * g, (raw.y as f64 / g as f64).round() as Coord * g)
    }

    pub fn restyle(&mut self) {
        self.canvas.restyle(view::style::Editor::Pcb);
        if let Some(v) = &mut self.view {
            v.rebuild();
        }
    }
}

/// What the Gerber tab asks for.
pub enum GerberAct {
    Reexport,
    /// The tab was clicked: it takes the keys.
    Focus,
}

/// The tab: the files on the canvas, view buttons, a banner when they are outdated or
/// unreadable, and what lies under the pointer.
pub fn gerber_ui(ui: &mut egui::Ui, pane: &mut GerberPane, outdated: bool, keymap: &Keymap) -> Option<GerberAct> {
    let rect = ui.max_rect();
    let mut empty = render::Scene::new();
    let scene = match &mut pane.view {
        Some(v) => &mut v.scene,
        None => &mut empty,
    };
    let response = ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| pane.canvas.ui(ui, scene)).inner;
    let mut act = None;

    if let Some(v) = &mut pane.view {
        let entries = v.layer_menu();
        let fit = toolbar::view_buttons(
            ui,
            rect,
            egui::Id::unique("gerber_view_buttons"),
            keymap,
            |ui| toolbar::layer_menu(ui, &mut v.scene, &entries),
            |ui| {
                toolbar::grid_menu(
                    ui,
                    &mut pane.grid,
                    &mut pane.snap,
                    &mut pane.canvas.background,
                    crate::units::GridKind::Board,
                )
            },
        );
        if fit {
            pane.canvas.request_fit();
        }
        let b = |measuring: bool, icon, name, about, action| ToolButton {
            tool: measuring,
            icon,
            name,
            about,
            action: Some(action),
        };
        let select = [b(false, Icon::Select, "Select", "Point at objects to see what they are.", Action::ToolSelect)];
        let measure = [b(true, Icon::Measure, "Measure", "Measure distances between two points.", Action::ToolMeasure)];
        let id = egui::Id::unique("gerber_toolbar");
        let (chosen, _) = toolbar::strip(ui, rect, id, &[&select, &measure], pane.measuring, keymap);
        if let Some(on) = chosen {
            pane.measuring = on;
            pane.measure.clear();
            act = Some(GerberAct::Focus);
        }
        if pane.measuring {
            toolbar::hint(ui, rect, view::measure::HINT);
            let tol = (8.0 / pane.canvas.camera.zoom * 1e6) as Coord;
            let free = ui.input(|i| i.modifiers.alt);
            let at = response.hover_pos().map(|p| {
                let [x, y] = pane.canvas.screen_to_world(p);
                let raw = Point::mm(x, y);
                let grid = pane.snap_point(raw);
                let points = pane.view.as_ref().map(|v| v.snap_points(raw, tol)).unwrap_or_default();
                let inside = pane.view.as_ref().and_then(|v| v.flash_at(raw));
                view::measure::snap(raw, grid, points, inside, tol, free)
            });
            pane.measure.hover(at.map(|a| a.0), at.is_some_and(|a| a.1));
            if let Some((p, _)) = at
                && response.hovered()
                && ui.input(|i| i.pointer.primary_pressed())
            {
                pane.measure.click(p);
            }
            crate::canvas::draw_measure(ui, &pane.canvas, response.rect, &pane.measure);
            if response.hovered() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
            }
        } else if response.hovered()
            && let Some(v) = &pane.view
            && let Some(p) = pane.canvas.cursor_world
            && let Some(text) = v.describe(p)
        {
            response.clone().on_hover_text_at_pointer(text);
        }
    }
    if act.is_none() && response.hovered() && ui.input(|i| i.pointer.any_pressed()) {
        act = Some(GerberAct::Focus);
    }

    // Banner: outdated export, unreadable files.
    let mut lines: Vec<(String, bool)> = Vec::new();
    if let Some(e) = &pane.error {
        lines.push((e.clone(), true));
    }
    if let Some(v) = &pane.view {
        lines.extend(v.errors.iter().map(|(n, e)| (format!("{n}: {e}"), true)));
        for f in &v.files {
            lines.extend(f.image.warnings.iter().take(3).map(|w| (format!("{}: {w}", f.name), false)));
        }
        if v.files.is_empty() && v.errors.is_empty() {
            lines.push(("No Gerber or drill files in this folder".into(), false));
        }
    }
    if outdated || !lines.is_empty() {
        egui::Area::new(egui::Id::unique("gerber_banner"))
            .order(egui::Order::Foreground)
            .fixed_pos(egui::pos2(rect.center().x, rect.top() + 10.0))
            .pivot(Align2::CENTER_TOP)
            .show(ui.ctx(), |ui| {
                egui::Frame::popup(ui.style()).show(ui, |ui| {
                    if outdated {
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new("The design changed since these files were exported.")
                                    .color(theme::warn()),
                            );
                            if ui.button("Re-export").clicked() {
                                act = Some(GerberAct::Reexport);
                            }
                        });
                    }
                    for (text, error) in &lines {
                        let color = if *error { theme::warn() } else { theme::text_dim() };
                        ui.label(RichText::new(text).color(color));
                    }
                });
            });
    }
    act
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn outputs_list_gerber_and_assembly_files() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("demo");
        let mut doc = Document::new(view::demo::demo_project());
        doc.save_as(&dir).unwrap();
        let mut outputs = Outputs::default();
        outputs.scan(Some(&dir));
        let node = outputs.node(false);
        let [gerber, assembly, pdf] = &node.children[..] else { panic!() };
        assert!(gerber.children.is_empty() && assembly.children.is_empty());
        assert_eq!(assembly.data.detail, "not exported");
        // Not exported yet: activating it exports.
        assert_eq!(pdf.data.detail, "not exported");
        assert!(matches!(project_panel::activate(pdf.id, &pdf.data), Some(project_panel::Act::ExportSchematicPdf)));

        doc.export_gerber().unwrap();
        outputs.scan(Some(&dir));
        assert_eq!(outputs.dirs().len(), 2);
        let node = outputs.node(false);
        let [gerber, assembly, _] = &node.children[..] else { panic!() };
        let job = gerber.children.iter().find(|n| n.label == "Job File").unwrap();
        assert!(matches!(job.id, NodeId::DataFile(_)), "the job file opens in the system's app");
        assert!(matches!(gerber.children[0].id, NodeId::OutputFile(_)));
        let labels: Vec<&str> = assembly.children.iter().map(|n| n.label.as_str()).collect();
        assert_eq!(labels, ["Bill of Materials", "Component Positions"]);
        let bom = &assembly.children[0];
        assert!(bom.data.path.as_ref().unwrap().is_file());
        assert!(matches!(project_panel::activate(bom.id, &bom.data), Some(project_panel::Act::OpenFile(_))));
        assert_eq!(assembly.data.detail, "2 files");

        // The schematic PDF opens in the system's viewer.
        let file = doc.export_schematic_pdf().unwrap();
        outputs.scan(Some(&dir));
        let node = outputs.node(false);
        let pdf = &node.children[2];
        assert_eq!(pdf.data.path.as_ref(), Some(&file));
        assert!(
            matches!(project_panel::activate(pdf.id, &pdf.data), Some(project_panel::Act::OpenFile(p)) if p == file)
        );
    }
}
