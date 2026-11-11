//! Fabrication outputs: the Gerber export folder as the Project panel shows it (Outputs ›
//! Gerber › files, marked outdated when the design changed since), and the read-only
//! Gerber tab that shows the files as read back from disk.

use std::path::{Path, PathBuf};

use egui::{Align2, RichText};
use view::gerber::{self, Manifest, design_hash, gerber_dir, read_manifest};
use view::{Document, GerberView};

use crate::canvas::Canvas;
use crate::icons::Icon;
use crate::keymap::Keymap;
use crate::project_panel::{self, Info, NodeId, TreeNode};
use crate::{theme, toolbar};

/// What the export folder holds, as last scanned.
#[derive(Default)]
pub struct Outputs {
    /// The export folder (`None`: the project isn't saved).
    pub dir: Option<PathBuf>,
    pub manifest: Option<Manifest>,
    /// Files in the folder, sorted (the manifest's order when there is one).
    pub files: Vec<String>,
    /// The design hash and the document revision it was computed at.
    hash: Option<(u64, String)>,
}

impl Outputs {
    /// Read the folder of the project at `project_dir` again.
    pub fn scan(&mut self, project_dir: Option<&Path>) {
        self.dir = project_dir.map(gerber_dir);
        self.manifest = project_dir.and_then(read_manifest);
        let on_disk: Vec<String> = match &self.dir {
            Some(d) => match std::fs::read_dir(d) {
                Ok(entries) => {
                    let mut v: Vec<String> = entries
                        .filter_map(|e| e.ok())
                        .filter(|e| e.path().is_file())
                        .map(|e| e.file_name().to_string_lossy().into_owned())
                        .filter(|n| n != gerber::MANIFEST && !n.starts_with('.'))
                        .collect();
                    v.sort();
                    v
                }
                Err(_) => Vec::new(),
            },
            None => Vec::new(),
        };
        self.files = match &self.manifest {
            Some(m) => {
                let mut v: Vec<String> =
                    m.files.iter().map(|f| f.name.clone()).filter(|n| on_disk.contains(n)).collect();
                v.extend(on_disk.into_iter().filter(|n| !m.files.iter().any(|f| f.name == *n)));
                v
            }
            None => on_disk,
        };
    }

    pub fn exported(&self) -> bool {
        !self.files.is_empty()
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

    fn label(&self, name: &str) -> Option<&str> {
        self.manifest.as_ref()?.files.iter().find(|f| f.name == name).map(|f| f.label.as_str())
    }

    /// The Outputs node.
    pub fn node(&self, outdated: bool) -> TreeNode {
        let files: Vec<TreeNode> = self
            .files
            .iter()
            .map(|name| {
                let path = self.dir.as_ref().map(|d| d.join(name));
                let (label, detail) = match self.label(name) {
                    Some(l) => (l.to_string(), name.clone()),
                    None => (name.clone(), String::new()),
                };
                let id = NodeId::OutputFile(project_panel::path_hash(Path::new(name)));
                TreeNode::new(id, label, Info { detail, path, tip: name.clone(), ..Info::icon(Icon::Layers) })
            })
            .collect();
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
        TreeNode::new(NodeId::Outputs, "Outputs", Info::icon(Icon::Folder)).with_children(vec![gerber])
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
}

impl Default for GerberPane {
    fn default() -> Self {
        GerberPane {
            canvas: Canvas::new(3, view::style::background(0.5)),
            view: None,
            error: None,
            grid: model::mm(0.5),
            snap: false,
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

    pub fn restyle(&mut self) {
        self.canvas.background = view::style::background(self.canvas.background.grid);
        if let Some(v) = &mut self.view {
            v.rebuild();
        }
    }
}

/// What the Gerber tab asks for.
pub enum GerberAct {
    Reexport,
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
        if response.hovered()
            && let Some(p) = pane.canvas.cursor_world
            && let Some(text) = v.describe(p)
        {
            response.clone().on_hover_text_at_pointer(text);
        }
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
