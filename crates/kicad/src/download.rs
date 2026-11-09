//! Download KiCad's official symbol and footprint libraries from GitLab, so no local KiCad
//! installation is needed.
//!
//! Releases are the `kicad-symbols` and `kicad-footprints` repositories' archives of a
//! stable tag (`10.0.8`; release candidates are skipped) of the major version the importer
//! supports ([`SUPPORTED_MAJOR`]). A download lands in `<root>/<version>/{symbols,footprints}`.
//! It is unpacked into a temporary folder and moved into place when complete, so a cancelled
//! or failed download leaves nothing behind. After a successful download, older versions in
//! `root` are deleted. 3D models are not downloaded.

use std::io::{self, Read};
use std::path::{Component, Path, PathBuf};
use std::time::Duration;

/// KiCad major version whose libraries the importer is tested with. Newer majors are only
/// offered once this is raised.
pub const SUPPORTED_MAJOR: u32 = 10;

/// Rough size of a download (both archives), for progress display.
pub const APPROX_DOWNLOAD_BYTES: u64 = 24_000_000;
/// Rough size on disk once unpacked.
pub const APPROX_DISK_BYTES: u64 = 450_000_000;

/// Repository name and the folder it is unpacked to.
const REPOS: [(&str, &str); 2] = [("kicad-symbols", "symbols"), ("kicad-footprints", "footprints")];

/// Archives are far smaller; this only guards against a misbehaving server.
const MAX_ARCHIVE_BYTES: u64 = 2_000_000_000;

#[derive(Debug, thiserror::Error)]
pub enum DownloadError {
    #[error("download failed: {0}")]
    Http(String),
    #[error("{0}")]
    Io(#[from] io::Error),
    #[error("no KiCad {SUPPORTED_MAJOR}.x release found")]
    NoRelease,
    #[error("download cancelled")]
    Cancelled,
}

impl From<ureq::Error> for DownloadError {
    fn from(e: ureq::Error) -> Self {
        DownloadError::Http(e.to_string())
    }
}

/// Where the libraries are downloaded from (tests point this at a local server).
#[derive(Clone, Debug)]
pub struct Source {
    /// GitLab instance, e.g. `https://gitlab.com`.
    pub base: String,
}

impl Default for Source {
    fn default() -> Self {
        Source { base: "https://gitlab.com".into() }
    }
}

impl Source {
    fn tags_url(&self, repo: &str) -> String {
        format!(
            "{}/api/v4/projects/kicad%2Flibraries%2F{repo}/repository/tags?per_page=100&search=%5E{SUPPORTED_MAJOR}.",
            self.base
        )
    }

    fn archive_url(&self, repo: &str, version: &str) -> String {
        format!("{}/kicad/libraries/{repo}/-/archive/{version}/{repo}-{version}.tar.gz", self.base)
    }
}

/// A complete download.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Installed {
    pub version: String,
    pub dir: PathBuf,
}

impl Installed {
    pub fn symbols(&self) -> PathBuf {
        self.dir.join("symbols")
    }

    pub fn footprints(&self) -> PathBuf {
        self.dir.join("footprints")
    }

    pub fn paths(&self) -> Vec<PathBuf> {
        vec![self.symbols(), self.footprints()]
    }
}

/// `"10.0.8"` → `(10, 0, 8)`; anything else (release candidates, other names) → `None`.
pub fn parse_version(s: &str) -> Option<(u32, u32, u32)> {
    let mut it = s.split('.').map(|p| p.parse::<u32>().ok());
    let v = (it.next()??, it.next()??, it.next()??);
    it.next().is_none().then_some(v)
}

/// The newest complete download in `root`, if any.
pub fn installed(root: &Path) -> Option<Installed> {
    let entries = std::fs::read_dir(root).ok()?;
    entries
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            let v = parse_version(&name)?;
            let dir = e.path();
            (dir.join("symbols").is_dir() && dir.join("footprints").is_dir())
                .then_some((v, Installed { version: name, dir }))
        })
        .max_by_key(|(v, _)| *v)
        .map(|(_, i)| i)
}

/// The library folders to search: the download in `root` if there is one, else an
/// installed KiCad's ([`crate::default_paths`]).
pub fn library_paths(root: Option<&Path>) -> Vec<PathBuf> {
    if let Some(i) = root.and_then(installed) {
        return i.paths();
    }
    let (s, f) = crate::default_paths();
    s.into_iter().chain(f).collect()
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_connect(Some(Duration::from_secs(20)))
        .timeout_recv_body(Some(Duration::from_secs(60)))
        .user_agent("pcb-designer")
        .build()
        .into()
}

/// The newest stable release of [`SUPPORTED_MAJOR`] that both repositories have.
pub fn latest_version(source: &Source) -> Result<String, DownloadError> {
    let agent = agent();
    let mut common: Option<Vec<String>> = None;
    for (repo, _) in REPOS {
        let json = agent.get(source.tags_url(repo)).call()?.into_body().read_to_string()?;
        let tags: serde_json::Value =
            serde_json::from_str(&json).map_err(|e| DownloadError::Http(format!("unexpected tag list: {e}")))?;
        let names: Vec<String> = tags
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|t| t["name"].as_str())
            .filter(|n| parse_version(n).is_some_and(|v| v.0 == SUPPORTED_MAJOR))
            .map(str::to_string)
            .collect();
        common = Some(match common {
            None => names,
            Some(c) => c.into_iter().filter(|n| names.contains(n)).collect(),
        });
    }
    common.unwrap_or_default().into_iter().max_by_key(|n| parse_version(n)).ok_or(DownloadError::NoRelease)
}

/// Counts bytes read and asks `progress` whether to go on.
struct Counting<'a, R> {
    inner: R,
    read: u64,
    progress: &'a mut dyn FnMut(u64) -> bool,
    cancelled: bool,
}

impl<R: Read> Read for Counting<'_, R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        // Not `Interrupted`: readers retry on that.
        if self.cancelled {
            return Err(io::Error::other("cancelled"));
        }
        let n = self.inner.read(buf)?;
        self.read += n as u64;
        if !(self.progress)(self.read) {
            self.cancelled = true;
            return Err(io::Error::other("cancelled"));
        }
        Ok(n)
    }
}

/// Unpack a `.tar.gz` whose entries all sit in one top folder into `dest`, without that
/// folder. Only plain files and folders are written, never outside `dest`.
fn unpack(reader: impl Read, dest: &Path) -> io::Result<()> {
    let mut archive = tar::Archive::new(flate2::read::GzDecoder::new(reader));
    for entry in archive.entries()? {
        let mut entry = entry?;
        let kind = entry.header().entry_type();
        if !kind.is_file() && !kind.is_dir() {
            continue;
        }
        let path = entry.path()?.into_owned();
        let rel: PathBuf = path.components().skip(1).collect();
        if rel.as_os_str().is_empty() || !rel.components().all(|c| matches!(c, Component::Normal(_))) {
            continue;
        }
        let target = dest.join(rel);
        if kind.is_dir() {
            std::fs::create_dir_all(&target)?;
        } else {
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent)?;
            }
            entry.unpack(&target)?;
        }
    }
    Ok(())
}

/// Download and unpack `version` into `root/<version>`, then delete older versions.
/// `progress` receives the bytes downloaded so far and returns `false` to cancel.
pub fn install(
    root: &Path,
    version: &str,
    source: &Source,
    progress: &mut dyn FnMut(u64) -> bool,
) -> Result<Installed, DownloadError> {
    if parse_version(version).is_none() {
        return Err(DownloadError::Http(format!("not a release version: {version:?}")));
    }
    std::fs::create_dir_all(root)?;
    let tmp = root.join(format!(".download-{version}"));
    let result = (|| {
        if tmp.exists() {
            std::fs::remove_dir_all(&tmp)?;
        }
        let agent = agent();
        let mut done = 0;
        for (repo, folder) in REPOS {
            let response = agent.get(source.archive_url(repo, version)).call()?;
            let body = response.into_body().into_with_config().limit(MAX_ARCHIVE_BYTES).reader();
            let mut report = |n: u64| progress(done + n);
            let mut counting = Counting { inner: body, read: 0, progress: &mut report, cancelled: false };
            let unpacked = unpack(&mut counting, &tmp.join(folder));
            if counting.cancelled {
                return Err(DownloadError::Cancelled);
            }
            unpacked?;
            done += counting.read;
        }
        let dir = root.join(version);
        if dir.exists() {
            std::fs::remove_dir_all(&dir)?;
        }
        std::fs::rename(&tmp, &dir)?;
        Ok(Installed { version: version.to_string(), dir })
    })();
    match result {
        Ok(installed) => {
            remove_other_versions(root, version);
            Ok(installed)
        }
        Err(e) => {
            let _ = std::fs::remove_dir_all(&tmp);
            Err(e)
        }
    }
}

/// Make sure libraries are in `root` and return them, plus whether they were downloaded
/// now. `version` installs that release. Otherwise the newest supported release is looked
/// up, unless a download exists and `update` is false (then no network access happens).
pub fn ensure(
    root: &Path,
    update: bool,
    version: Option<&str>,
    source: &Source,
    progress: &mut dyn FnMut(u64) -> bool,
) -> Result<(Installed, bool), DownloadError> {
    let current = installed(root);
    if let Some(c) =
        current.as_ref().filter(|c| version.is_none_or(|v| v == c.version) && (!update || version.is_some()))
    {
        return Ok((c.clone(), false));
    }
    let version = match version {
        Some(v) => v.to_string(),
        None => latest_version(source)?,
    };
    if let Some(c) = current.filter(|c| c.version == version) {
        return Ok((c, false));
    }
    Ok((install(root, &version, source, progress)?, true))
}

/// Delete downloads of other versions (best effort).
fn remove_other_versions(root: &Path, keep: &str) {
    let Ok(entries) = std::fs::read_dir(root) else { return };
    for e in entries.filter_map(|e| e.ok()) {
        let name = e.file_name().to_string_lossy().into_owned();
        if name != keep && parse_version(&name).is_some() {
            let _ = std::fs::remove_dir_all(e.path());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions() {
        assert_eq!(parse_version("10.0.8"), Some((10, 0, 8)));
        assert_eq!(parse_version("10.0.8-rc1"), None);
        assert_eq!(parse_version("10.0"), None);
        assert_eq!(parse_version("10.0.8.1"), None);
        assert!(parse_version("10.0.10") > parse_version("10.0.9"));
    }
}
