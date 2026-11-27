//! The browser build's file dialog: a file browser over the browser's storage
//! (`model::fs`), which is also where files come in and go out: Upload copies files from
//! the computer into the folder shown (a `.zip` unpacked), Download hands a file, or a
//! folder as a `.zip`, to the browser.
//!
//! The logic is plain functions over `model::fs` (tested on the desktop, where it is the
//! real file system); the window and the transfers exist in the browser only.

use std::path::{Path, PathBuf};

use crate::file_dialog::Kind;

/// A row of the folder shown.
#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    pub path: PathBuf,
    pub name: String,
    pub folder: bool,
    /// A project folder (opening it opens the project).
    pub project: bool,
}

/// The folder's entries a dialog of `kind` offers: folders first, then files with one
/// of `extensions` (any without), each sorted by name; hidden ones left out. Folder
/// pickers show folders only.
pub fn rows(dir: &Path, kind: Kind, extensions: &[String]) -> Vec<Row> {
    let mut rows: Vec<Row> = model::fs::list_dir(dir)
        .unwrap_or_default()
        .into_iter()
        .filter_map(|path| {
            let name = path.file_name()?.to_string_lossy().into_owned();
            let folder = model::fs::is_dir(&path);
            let shown = !name.starts_with('.')
                && if folder {
                    true
                } else {
                    kind != Kind::PickFolder
                        && (extensions.is_empty() || extensions.iter().any(|e| has_extension(&name, e)))
                };
            shown.then(|| Row { project: folder && model::io::is_project_dir(&path), path, name, folder })
        })
        .collect();
    rows.sort_by(|a, b| b.folder.cmp(&a.folder).then_with(|| model::natural_cmp(&a.name, &b.name)));
    rows
}

fn has_extension(name: &str, ext: &str) -> bool {
    Path::new(name).extension().is_some_and(|e| e.eq_ignore_ascii_case(ext))
}

/// The file to save as: `name` in `dir`, given the first extension when it has none of
/// `extensions` (native dialogs add it too).
pub fn save_path(dir: &Path, name: &str, extensions: &[String]) -> PathBuf {
    let name = name.trim();
    match extensions.first() {
        Some(ext) if !extensions.iter().any(|e| has_extension(name, e)) => dir.join(format!("{name}.{ext}")),
        _ => dir.join(name),
    }
}

/// `path`, or the first of `path 2`, `path 3`, … (before the extension) that doesn't
/// exist yet.
pub fn unused(path: &Path) -> PathBuf {
    if !model::fs::exists(path) {
        return path.to_path_buf();
    }
    let stem = path.file_stem().map_or(String::new(), |s| s.to_string_lossy().into_owned());
    let ext = path.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
    (2..).map(|n| path.with_file_name(format!("{stem} {n}{ext}"))).find(|p| !model::fs::exists(p)).expect("a free name")
}

/// Puts uploaded files into `dir` (never over existing ones: `name 2.ext`). A `.zip` is
/// unpacked: as it is when everything in it is in one folder, else into a folder named
/// after it. Returns what was added.
pub fn add_uploads(dir: &Path, files: Vec<(String, Vec<u8>)>) -> Result<Vec<PathBuf>, String> {
    let mut added = Vec::new();
    for (name, data) in files {
        if has_extension(&name, "zip") {
            added.push(unpack(dir, &name, &data)?);
        } else {
            let path = unused(&dir.join(&name));
            model::fs::write(&path, data).map_err(|e| format!("{name}: {e}"))?;
            added.push(path);
        }
    }
    Ok(added)
}

fn unpack(dir: &Path, name: &str, data: &[u8]) -> Result<PathBuf, String> {
    let files = view::gerber::zip::unzip(data).map_err(|e| format!("{name}: {e}"))?;
    // Only plain relative paths (no `..`, no root).
    let relative = |p: &str| {
        let p = Path::new(p);
        p.components().all(|c| matches!(c, std::path::Component::Normal(_))).then(|| p.to_path_buf())
    };
    let files: Vec<(PathBuf, Vec<u8>)> = files.into_iter().filter_map(|(p, d)| Some((relative(&p)?, d))).collect();
    let top = |p: &Path| p.components().next().map(|c| c.as_os_str().to_owned());
    let one_folder = files
        .first()
        .and_then(|(p, _)| top(p))
        .filter(|first| files.iter().all(|(p, _)| p.components().count() > 1 && top(p).as_ref() == Some(first)));
    let (target, files) = match one_folder {
        Some(folder) => {
            let target = unused(&dir.join(&folder));
            let inner = files.into_iter().map(|(p, d)| (p.strip_prefix(&folder).expect("in it").to_path_buf(), d));
            (target, inner.collect::<Vec<_>>())
        }
        None => {
            let stem = Path::new(name).file_stem().map_or("Upload".into(), |s| s.to_string_lossy().into_owned());
            (unused(&dir.join(stem)), files)
        }
    };
    for (path, content) in files {
        let path = target.join(path);
        if let Some(parent) = path.parent() {
            model::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        model::fs::write(&path, content).map_err(|e| format!("{}: {e}", path.display()))?;
    }
    model::fs::create_dir_all(&target).map_err(|e| e.to_string())?;
    Ok(target)
}

/// What Download hands over for `path`: the file, or a folder as `<name>.zip` (with
/// the folder in it).
pub fn download(path: &Path) -> Result<(String, Vec<u8>), String> {
    let name = path.file_name().map_or("download".into(), |n| n.to_string_lossy().into_owned());
    if !model::fs::is_dir(path) {
        return model::fs::read(path).map(|data| (name, data)).map_err(|e| e.to_string());
    }
    let mut files = Vec::new();
    let mut todo = vec![path.to_path_buf()];
    while let Some(dir) = todo.pop() {
        for p in model::fs::list_dir(&dir).map_err(|e| e.to_string())? {
            if model::fs::is_dir(&p) {
                todo.push(p);
            } else {
                let rel = p.strip_prefix(path.parent().unwrap_or(path)).expect("below");
                let rel = rel.components().map(|c| c.as_os_str().to_string_lossy()).collect::<Vec<_>>().join("/");
                files.push((rel, model::fs::read(&p).map_err(|e| e.to_string())?));
            }
        }
    }
    files.sort();
    let zip = view::gerber::zip::zip(&files).map_err(|e| e.to_string())?;
    Ok((format!("{name}.zip"), zip))
}

#[cfg(target_arch = "wasm32")]
pub use window::Browser;

#[cfg(target_arch = "wasm32")]
mod window {
    use std::cell::RefCell;
    use std::future::Future;
    use std::path::{Path, PathBuf};
    use std::pin::Pin;
    use std::rc::Rc;
    use std::sync::Arc;
    use std::task::{Context, Poll, Wake, Waker};

    use egui::RichText;

    use super::{Row, add_uploads, download, rows, save_path};
    use crate::file_dialog::{Answer, FileDialog, Kind};
    use crate::icons::{self, Icon};
    use crate::theme;

    type Slot = Rc<RefCell<Option<Option<PathBuf>>>>;
    type Upload = Pin<Box<dyn Future<Output = Vec<(String, Vec<u8>)>>>>;

    /// Resolves once the browser is answered.
    struct Answered(Slot);

    impl Future for Answered {
        type Output = Option<PathBuf>;
        fn poll(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Option<PathBuf>> {
            match self.0.borrow_mut().take() {
                Some(answer) => Poll::Ready(answer),
                None => Poll::Pending,
            }
        }
    }

    struct Repaint(egui::Context);

    impl Wake for Repaint {
        fn wake(self: Arc<Self>) {
            self.0.request_repaint();
        }
    }

    /// An open file browser (see the module docs).
    pub struct Browser {
        ctx: egui::Context,
        dialog: FileDialog,
        kind: Kind,
        dir: PathBuf,
        selected: Option<PathBuf>,
        /// The file name (saving).
        name: String,
        /// A new folder's name, while it's typed.
        new_folder: Option<String>,
        /// Delete was clicked once for this path: a second click deletes.
        deleting: Option<PathBuf>,
        message: Option<String>,
        upload: Option<(Upload, Waker)>,
        answer: Slot,
    }

    impl Browser {
        pub fn new(ctx: &egui::Context, dialog: FileDialog, kind: Kind) -> (Browser, Answer) {
            let home = PathBuf::from(crate::web::HOME);
            // A file's or project's own folder starts in its parent (save as, export).
            let dir = match &dialog.directory {
                Some(d) if model::fs::is_dir(d) && (kind == Kind::PickFolder || model::io::is_project_dir(d)) => {
                    d.parent().filter(|p| model::fs::is_dir(p)).map_or(home.clone(), Path::to_path_buf)
                }
                Some(d) if model::fs::is_dir(d) => d.clone(),
                _ => home,
            };
            let name = dialog.file_name.clone().unwrap_or_default();
            let answer: Slot = Rc::default();
            let browser = Browser {
                ctx: ctx.clone(),
                dialog,
                kind,
                dir,
                selected: None,
                name,
                new_folder: None,
                deleting: None,
                message: None,
                upload: None,
                answer: answer.clone(),
            };
            (browser, Box::pin(Answered(answer)))
        }

        fn extensions(&self) -> Vec<String> {
            self.dialog.filters.iter().flat_map(|(_, e)| e.iter().cloned()).collect()
        }

        fn finish(&mut self, path: Option<PathBuf>) {
            *self.answer.borrow_mut() = Some(path);
            self.ctx.request_repaint();
        }

        fn enter(&mut self, dir: PathBuf) {
            self.dir = dir;
            self.selected = None;
            self.deleting = None;
        }

        /// What OK answers, if it can.
        fn choice(&self) -> Option<PathBuf> {
            match self.kind {
                Kind::PickFile => self.selected.clone().filter(|p| model::fs::is_file(p)),
                Kind::PickFolder => match &self.selected {
                    Some(p) if model::fs::is_dir(p) => Some(p.clone()),
                    _ => (self.dir.parent().is_some()).then(|| self.dir.clone()),
                },
                Kind::Save => {
                    (!self.name.trim().is_empty()).then(|| save_path(&self.dir, &self.name, &self.extensions()))
                }
            }
        }

        fn poll_upload(&mut self) {
            let Some((upload, waker)) = &mut self.upload else { return };
            let Poll::Ready(files) = upload.as_mut().poll(&mut Context::from_waker(waker)) else { return };
            self.upload = None;
            match add_uploads(&self.dir, files) {
                Ok(added) => {
                    self.message = (!added.is_empty()).then(|| format!("Added {}", names(&added)));
                    self.selected = added.last().cloned();
                }
                Err(e) => self.message = Some(format!("Upload failed: {e}")),
            }
        }

        fn start_upload(&mut self) {
            let mut picker = rfd::AsyncFileDialog::new().set_title("Upload");
            for (name, ext) in &self.dialog.filters {
                picker = picker.add_filter(name, ext);
            }
            let upload: Upload = Box::pin(async move {
                let mut files = Vec::new();
                for f in picker.pick_files().await.unwrap_or_default() {
                    files.push((f.file_name(), f.read().await));
                }
                files
            });
            self.upload = Some((upload, Waker::from(Arc::new(Repaint(self.ctx.clone())))));
        }

        pub fn show(&mut self) {
            self.poll_upload();
            let ctx = self.ctx.clone();
            let title = self.dialog.title.clone().unwrap_or_else(|| "Choose".into());
            let modal = theme::dialog(&ctx, egui::Id::unique("file_browser"), &title, 560.0, |ui| self.contents(ui));
            if modal.should_close() && self.answer.borrow().is_none() {
                self.finish(None);
            }
        }

        fn contents(&mut self, ui: &mut egui::Ui) {
            // Where: up, and the folder's path.
            ui.horizontal(|ui| {
                let up = self.dir.parent().map(Path::to_path_buf);
                if ui.add_enabled(up.is_some(), egui::Button::new("Up")).clicked()
                    && let Some(up) = up
                {
                    self.enter(up);
                }
                ui.label(RichText::new(self.dir.display().to_string()).color(theme::text_dim()));
            });
            ui.add_space(4.0);
            let rows = rows(&self.dir, self.kind, &self.extensions());
            let mut open_dir = None;
            let mut chosen = None;
            egui::Frame::NONE.fill(theme::panel_dark()).corner_radius(6).inner_margin(egui::Margin::same(4)).show(
                ui,
                |ui| {
                    egui::ScrollArea::vertical()
                        .min_scrolled_height(260.0)
                        .max_height(260.0)
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            if rows.is_empty() {
                                ui.label(RichText::new("Empty folder").color(theme::text_dim()));
                            }
                            for row in &rows {
                                let r = self.row(ui, row);
                                if r.clicked() {
                                    self.selected = Some(row.path.clone());
                                    self.deleting = None;
                                    if self.kind == Kind::Save && !row.folder {
                                        self.name = row.name.clone();
                                    }
                                }
                                if r.double_clicked() {
                                    match (row.folder, self.kind) {
                                        (true, Kind::PickFolder) if row.project => chosen = Some(row.path.clone()),
                                        (true, _) => open_dir = Some(row.path.clone()),
                                        (false, Kind::PickFile | Kind::Save) => chosen = self.choice(),
                                        (false, Kind::PickFolder) => {}
                                    }
                                }
                            }
                        });
                },
            );
            if let Some(d) = open_dir {
                self.enter(d);
            }
            if self.kind == Kind::Save {
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    ui.label("Name");
                    ui.add(egui::TextEdit::singleline(&mut self.name).desired_width(f32::INFINITY));
                });
            }
            if let Some(name) = &mut self.new_folder {
                ui.add_space(6.0);
                let mut done = None;
                ui.horizontal(|ui| {
                    ui.label("New folder");
                    let r = ui.add(egui::TextEdit::singleline(name).desired_width(240.0));
                    r.request_focus();
                    if ui.button("Create").clicked()
                        || (r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)))
                    {
                        done = Some(true);
                    }
                    if ui.button("Cancel").clicked() {
                        done = Some(false);
                    }
                });
                match done {
                    Some(true) if !name.trim().is_empty() => {
                        let path = self.dir.join(name.trim());
                        match model::fs::create_dir_all(&path) {
                            Ok(()) => self.selected = Some(path),
                            Err(e) => self.message = Some(e.to_string()),
                        }
                        self.new_folder = None;
                    }
                    Some(_) => self.new_folder = None,
                    None => {}
                }
            }
            if let Some(m) = &self.message {
                ui.add_space(4.0);
                ui.label(RichText::new(m).color(theme::text_dim()));
            }
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button("New folder").clicked() {
                    self.new_folder = Some(String::new());
                }
                let uploading = self.upload.is_some();
                if ui
                    .add_enabled(!uploading, egui::Button::new(if uploading { "Uploading…" } else { "Upload…" }))
                    .on_hover_text("Copy files from your computer into this folder (a .zip is unpacked)")
                    .clicked()
                {
                    self.start_upload();
                }
                let target = self.selected.clone().or_else(|| self.dir.parent().map(|_| self.dir.clone()));
                if ui
                    .add_enabled(target.is_some(), egui::Button::new("Download"))
                    .on_hover_text("Save the selected file, or folder as a .zip, to your computer")
                    .clicked()
                    && let Some(t) = &target
                {
                    match download(t) {
                        Ok((name, data)) => crate::web::download(&name, &data),
                        Err(e) => self.message = Some(format!("Download failed: {e}")),
                    }
                }
                let sure = self.deleting.is_some() && self.deleting == self.selected;
                if ui
                    .add_enabled(
                        self.selected.is_some(),
                        egui::Button::new(if sure { "Really delete?" } else { "Delete" }),
                    )
                    .clicked()
                    && let Some(p) = self.selected.clone()
                {
                    if sure {
                        let removed = if model::fs::is_dir(&p) {
                            model::fs::remove_dir_all(&p)
                        } else {
                            model::fs::remove_file(&p)
                        };
                        self.message = Some(match removed {
                            Ok(()) => format!("Deleted {}", names(&[p])),
                            Err(e) => e.to_string(),
                        });
                        self.selected = None;
                        self.deleting = None;
                    } else {
                        self.deleting = Some(p);
                    }
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let ok = match self.kind {
                        Kind::Save => "Save",
                        Kind::PickFile => "Open",
                        Kind::PickFolder => "Choose",
                    };
                    let choice = self.choice();
                    if ui.add_enabled(choice.is_some(), egui::Button::new(ok)).clicked() {
                        chosen = choice;
                    }
                    if ui.button("Cancel").clicked() {
                        self.finish(None);
                    }
                });
            });
            if let Some(path) = chosen {
                self.finish(Some(path));
            }
        }

        fn row(&self, ui: &mut egui::Ui, row: &Row) -> egui::Response {
            let selected = self.selected.as_ref() == Some(&row.path);
            let height = ui.spacing().interact_size.y;
            let (rect, response) =
                ui.allocate_exact_size(egui::vec2(ui.available_width(), height), egui::Sense::click());
            let visuals = ui.visuals();
            if selected {
                ui.painter().rect_filled(rect, 4.0, visuals.selection.bg_fill);
            } else if response.hovered() {
                ui.painter().rect_filled(rect, 4.0, visuals.widgets.hovered.weak_bg_fill);
            }
            let icon = if row.project {
                Icon::Project
            } else if row.folder {
                Icon::Folder
            } else {
                Icon::FileCode
            };
            let size = height - 6.0;
            let icon_rect = egui::Rect::from_min_size(rect.min + egui::vec2(6.0, 3.0), egui::vec2(size, size));
            icons::paint(ui.painter(), icon_rect, icon, theme::text_dim());
            ui.painter().text(
                egui::pos2(icon_rect.max.x + 8.0, rect.center().y),
                egui::Align2::LEFT_CENTER,
                &row.name,
                egui::TextStyle::Body.resolve(ui.style()),
                if selected { ui.visuals().selection.stroke.color } else { ui.visuals().text_color() },
            );
            response
        }
    }

    fn names(paths: &[PathBuf]) -> String {
        paths.iter().filter_map(|p| p.file_name()).map(|n| n.to_string_lossy()).collect::<Vec<_>>().join(", ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn exts(e: &[&str]) -> Vec<String> {
        e.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn folders_come_first_and_files_are_filtered() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        for d in ["b", "A", ".hidden"] {
            std::fs::create_dir(dir.join(d)).unwrap();
        }
        model::io::save_project(&model::Project::new("p"), dir.join("proj")).unwrap();
        for f in ["x.pcblib", "y.txt", "z10.pcblib", "z9.PCBLIB"] {
            std::fs::write(dir.join(f), "").unwrap();
        }
        let names = |rows: Vec<Row>| rows.into_iter().map(|r| r.name).collect::<Vec<_>>();
        assert_eq!(
            names(rows(dir, Kind::PickFile, &exts(&["pcblib"]))),
            ["A", "b", "proj", "x.pcblib", "z9.PCBLIB", "z10.pcblib"]
        );
        assert_eq!(names(rows(dir, Kind::PickFolder, &[])), ["A", "b", "proj"]);
        assert_eq!(rows(dir, Kind::Save, &[]).len(), 7, "every file without filters");
        assert!(rows(dir, Kind::PickFolder, &[]).iter().any(|r| r.name == "proj" && r.project));
    }

    #[test]
    fn saving_adds_the_extension() {
        let dir = Path::new("/p");
        assert_eq!(save_path(dir, "mine", &exts(&["pcblib"])), Path::new("/p/mine.pcblib"));
        assert_eq!(save_path(dir, " mine.PCBLIB ", &exts(&["pcblib"])), Path::new("/p/mine.PCBLIB"));
        assert_eq!(save_path(dir, "Amp", &[]), Path::new("/p/Amp"));
    }

    #[test]
    fn uploads_never_replace_files() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        let added = add_uploads(dir, vec![("a.txt".into(), b"1".to_vec()), ("a.txt".into(), b"2".to_vec())]).unwrap();
        assert_eq!(added, [dir.join("a.txt"), dir.join("a 2.txt")]);
        assert_eq!(std::fs::read(dir.join("a 2.txt")).unwrap(), b"2");
    }

    #[test]
    fn a_downloaded_project_uploads_again() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        let project = view::demo::Example::Led.project();
        model::io::save_project(&project, dir.join("Demo")).unwrap();
        let (name, zip) = download(&dir.join("Demo")).unwrap();
        assert_eq!(name, "Demo.zip");

        let other = dir.join("other");
        std::fs::create_dir(&other).unwrap();
        let added = add_uploads(&other, vec![(name.clone(), zip.clone())]).unwrap();
        assert_eq!(added, [other.join("Demo")], "the folder in the zip");
        assert_eq!(model::io::load_project(other.join("Demo")).unwrap().name, project.name);
        let again = add_uploads(&other, vec![(name, zip)]).unwrap();
        assert_eq!(again, [other.join("Demo 2")]);
    }

    #[test]
    fn a_zip_of_loose_files_gets_its_own_folder() {
        let tmp = tempfile::tempdir().unwrap();
        let zip = view::gerber::zip::zip(&[
            ("board.json".into(), b"{}".to_vec()),
            ("sub/x.txt".into(), b"x".to_vec()),
            ("../escape.txt".into(), b"no".to_vec()),
        ])
        .unwrap();
        let added = add_uploads(tmp.path(), vec![("Fab files.zip".into(), zip)]).unwrap();
        let folder = tmp.path().join("Fab files");
        assert_eq!(added, std::slice::from_ref(&folder));
        assert_eq!(std::fs::read(folder.join("sub/x.txt")).unwrap(), b"x");
        assert!(!tmp.path().join("escape.txt").exists());
    }
}
