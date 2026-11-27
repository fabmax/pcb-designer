//! Help → About: the app's version, links to its documentation and source, its licence
//! and, on the desktop, a check for a newer release on GitHub.

use egui::{Align, Layout, RichText, TextureHandle};

use crate::theme;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const REPO_URL: &str = "https://github.com/fabmax/pcb-designer";
pub const DOCS_URL: &str = "https://fabmax.github.io/pcb-designer/";
const RELEASES_URL: &str = "https://github.com/fabmax/pcb-designer/releases";
const LICENSE_URL: &str = "https://github.com/fabmax/pcb-designer/blob/main/LICENSE";
/// The release GitHub marks as the latest (no pre-releases or drafts).
#[cfg(not(target_arch = "wasm32"))]
const LATEST_RELEASE_API: &str = "https://api.github.com/repos/fabmax/pcb-designer/releases/latest";

/// Size of the icon in the dialog (points).
const ICON: f32 = 96.0;

/// A release on GitHub: its tag (`v0.6.0`) and page.
#[derive(Clone, Debug, PartialEq)]
pub struct Release {
    pub tag: String,
    pub url: String,
}

/// Where the update check is.
#[derive(Default)]
enum Update {
    #[default]
    NotChecked,
    #[cfg(not(target_arch = "wasm32"))]
    Checking(crate::background::Job<Result<Release, String>>),
    Done(Result<Release, String>),
}

/// The About dialog, while it's open.
#[derive(Default)]
pub struct About {
    icon: Option<TextureHandle>,
    update: Update,
}

impl About {
    /// Shows the dialog; `false` once it's closed.
    pub fn ui(&mut self, ctx: &egui::Context) -> bool {
        self.poll(ctx);
        let icon = self.icon.get_or_insert_with(|| crate::icons::app_icon(ctx, 256)).clone();
        let mut close = false;
        let modal = theme::dialog(ctx, egui::Id::unique("about"), "About", 340.0, |ui| {
            ui.vertical_centered(|ui| {
                ui.add(egui::Image::new((icon.id(), egui::vec2(ICON, ICON))));
                ui.add_space(6.0);
                ui.label(RichText::new(crate::menu::APP_NAME).size(20.0).strong());
                let build = if cfg!(target_arch = "wasm32") { " (browser version)" } else { "" };
                ui.label(RichText::new(format!("Version {VERSION}{build}")).color(theme::text_dim()));
                ui.add_space(4.0);
                ui.label("Schematic, PCB and part editor");
                ui.add_space(10.0);
                self.update_ui(ui);
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    // Centred: the row's width is known after the first frame.
                    let id = egui::Id::unique("about_links");
                    let width: f32 = ui.ctx().data(|d| d.get_temp(id)).unwrap_or(0.0);
                    ui.add_space(((ui.available_width() - width) / 2.0).max(0.0));
                    let start = ui.cursor().min.x;
                    link(ui, "Documentation", DOCS_URL);
                    ui.label(RichText::new("·").color(theme::text_dim()));
                    link(ui, "Source code", REPO_URL);
                    ui.label(RichText::new("·").color(theme::text_dim()));
                    link(ui, "Releases", RELEASES_URL);
                    let w = ui.min_rect().max.x - start;
                    if (w - width).abs() > 0.5 {
                        ui.ctx().data_mut(|d| d.insert_temp(id, w));
                        ui.ctx().request_repaint();
                    }
                });
                ui.add_space(10.0);
                ui.separator();
                ui.add_space(4.0);
                let license = RichText::new("Free software under the MIT License").small();
                ui.add(egui::Hyperlink::from_label_and_url(license, LICENSE_URL).open_in_new_tab(true));
                ui.label(RichText::new("Simulation by ngspice (BSD-3-Clause)").small().color(theme::text_dim()));
            });
            ui.add_space(8.0);
            ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
                if ui.button("Close").clicked() {
                    close = true;
                }
            });
        });
        !(close || modal.should_close())
    }

    /// The update check: a button, then what it found.
    fn update_ui(&mut self, ui: &mut egui::Ui) {
        if cfg!(target_arch = "wasm32") {
            // The page is always the latest release.
            return;
        }
        match &self.update {
            Update::NotChecked => {
                if ui.button("Check for updates").clicked() {
                    self.check();
                }
            }
            #[cfg(not(target_arch = "wasm32"))]
            Update::Checking(_) => {
                ui.horizontal(|ui| {
                    ui.add(egui::Spinner::new());
                    ui.label("Checking for updates…");
                });
            }
            Update::Done(Ok(release)) if is_newer(&release.tag, VERSION) => {
                ui.label(
                    RichText::new(format!("Version {} is available", tag_version(&release.tag)))
                        .color(theme::success()),
                );
                link(ui, "Download it from GitHub", &release.url);
            }
            Update::Done(Ok(_)) => {
                ui.label(RichText::new("PCB Designer is up to date").color(theme::success()));
            }
            Update::Done(Err(e)) => {
                ui.label(RichText::new(format!("Couldn't check for updates: {e}")).color(theme::warn()));
                if ui.button("Try again").clicked() {
                    self.check();
                }
            }
        }
    }

    fn check(&mut self) {
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.update = Update::Checking(crate::background::spawn(latest_release));
        }
    }

    fn poll(&mut self, ctx: &egui::Context) {
        #[cfg(not(target_arch = "wasm32"))]
        if let Update::Checking(job) = &self.update {
            if !job.is_finished() {
                ctx.request_repaint_after(std::time::Duration::from_millis(100));
                return;
            }
            let Update::Checking(job) = std::mem::take(&mut self.update) else { unreachable!() };
            self.update = Update::Done(job.join().unwrap_or_else(|_| Err("the check failed".into())));
        }
        #[cfg(target_arch = "wasm32")]
        let _ = ctx;
    }
}

/// A link that opens in the browser (a new tab in the browser version, which would
/// otherwise be left).
fn link(ui: &mut egui::Ui, text: &str, url: &str) {
    ui.add(egui::Hyperlink::from_label_and_url(text, url).open_in_new_tab(true));
}

/// Asks GitHub for the latest release.
#[cfg(not(target_arch = "wasm32"))]
fn latest_release() -> Result<Release, String> {
    use std::time::Duration;
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(15)))
        .user_agent("pcb-designer")
        .build()
        .into();
    let body = agent
        .get(LATEST_RELEASE_API)
        .header("Accept", "application/vnd.github+json")
        .call()
        .map_err(|e| match e {
            ureq::Error::StatusCode(code) => format!("GitHub answered {code}"),
            e => e.to_string(),
        })?
        .into_body()
        .with_config()
        .limit(1_000_000)
        .read_to_string()
        .map_err(|e| e.to_string())?;
    parse_release(&body)
}

/// The tag and page of GitHub's JSON for a release.
pub fn parse_release(json: &str) -> Result<Release, String> {
    let v: serde_json::Value = serde_json::from_str(json).map_err(|e| e.to_string())?;
    let field = |name: &str| v.get(name).and_then(|s| s.as_str()).map(str::to_string);
    match (field("tag_name"), field("html_url")) {
        (Some(tag), Some(url)) => Ok(Release { tag, url }),
        _ => Err("unexpected answer from GitHub".into()),
    }
}

/// `v0.6.0` → `0.6.0`.
fn tag_version(tag: &str) -> &str {
    tag.strip_prefix('v').unwrap_or(tag)
}

/// The numbers of a version (`0.6.0`, `v0.6.0`, `0.6.0-beta.1` → [0, 6, 0]).
fn numbers(version: &str) -> Vec<u64> {
    let core = tag_version(version.trim()).split(['-', '+']).next().unwrap_or("");
    core.split('.').map(|n| n.parse().unwrap_or(0)).collect()
}

/// Whether release `tag` is newer than version `current`.
pub fn is_newer(tag: &str, current: &str) -> bool {
    let (mut a, mut b) = (numbers(tag), numbers(current));
    let n = a.len().max(b.len());
    a.resize(n, 0);
    b.resize(n, 0);
    a > b
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn newer_releases_are_found() {
        assert!(is_newer("v0.6.0", "0.5.0"));
        assert!(is_newer("v0.10.0", "0.9.3"));
        assert!(is_newer("v1.0", "0.99.99"));
        assert!(is_newer("v0.5.1", "0.5.0"));
        assert!(!is_newer("v0.5.0", "0.5.0"));
        assert!(!is_newer("v0.5", "0.5.0"));
        assert!(!is_newer("v0.4.2", "0.5.0"));
        // A pre-release of this version isn't newer.
        assert!(!is_newer("v0.5.0-beta.1", "0.5.0"));
    }

    #[test]
    fn github_release_json_is_read() {
        let json = r#"{"tag_name": "v0.6.0", "html_url": "https://github.com/fabmax/pcb-designer/releases/tag/v0.6.0",
            "name": "v0.6.0", "draft": false}"#;
        assert_eq!(
            parse_release(json),
            Ok(Release {
                tag: "v0.6.0".into(),
                url: "https://github.com/fabmax/pcb-designer/releases/tag/v0.6.0".into()
            })
        );
        assert!(parse_release(r#"{"message": "Not Found"}"#).is_err());
        assert!(parse_release("not json").is_err());
    }
}
