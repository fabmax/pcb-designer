//! ngspice as a separate process in batch mode: `ngspice -b -r out.raw circuit.cir` in a
//! temporary folder, the raw file read back afterwards.
//!
//! A process per run keeps a crashing model away from the app, makes Stop a kill, and
//! lets several runs go at once (libngspice is one instance per process). ngspice exits
//! with status 1 on netlist errors, with the reasons on stderr.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use crate::raw::{self, Plot};
use crate::{Backend, Live, Output, SimError};

/// The ngspice executable.
#[derive(Clone, Debug, PartialEq)]
pub struct Ngspice {
    pub path: PathBuf,
}

/// Where ngspice is usually installed, after `PATH`.
const PLACES: &[&str] = &[
    "/opt/homebrew/bin/ngspice",
    "/usr/local/bin/ngspice",
    "/usr/bin/ngspice",
    "C:\\Program Files\\Spice64\\bin\\ngspice_con.exe",
    "C:\\Program Files\\Spice64\\bin\\ngspice.exe",
];

impl Ngspice {
    /// `configured` if given (an error when it isn't a file), else `NGSPICE` from the
    /// environment, else `ngspice` on `PATH`, else the usual install places.
    pub fn find(configured: Option<&Path>) -> Result<Ngspice, SimError> {
        if let Some(path) = configured.filter(|p| !p.as_os_str().is_empty()) {
            return if path.is_file() {
                Ok(Ngspice { path: path.into() })
            } else {
                Err(SimError::NotFound(format!("{} is not a file", path.display())))
            };
        }
        if let Some(path) = std::env::var_os("NGSPICE").map(PathBuf::from).filter(|p| p.is_file()) {
            return Ok(Ngspice { path });
        }
        let exe = if cfg!(windows) { ["ngspice_con.exe", "ngspice.exe"].as_slice() } else { ["ngspice"].as_slice() };
        if let Some(paths) = std::env::var_os("PATH") {
            for dir in std::env::split_paths(&paths) {
                for name in exe {
                    let path = dir.join(name);
                    if path.is_file() {
                        return Ok(Ngspice { path });
                    }
                }
            }
        }
        PLACES
            .iter()
            .map(PathBuf::from)
            .find(|p| p.is_file())
            .map(|path| Ngspice { path })
            .ok_or_else(|| SimError::NotFound("ngspice is not installed (or not on PATH)".into()))
    }
}

impl Ngspice {
    /// "ngspice-47" (from `ngspice -v`), if it runs.
    pub fn version(&self) -> Option<String> {
        let out = Command::new(&self.path).arg("-v").stdin(Stdio::null()).output().ok()?;
        let text = String::from_utf8_lossy(&out.stdout);
        text.lines().find_map(|l| {
            let l = l.trim_start_matches('*').trim();
            l.starts_with("ngspice").then(|| l.split(':').next().unwrap_or(l).trim().to_string())
        })
    }
}

/// A stop button for a running simulation.
#[derive(Clone, Debug, Default)]
pub struct Cancel(Arc<AtomicBool>);

impl Cancel {
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Relaxed);
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }
}

impl Backend for Ngspice {
    fn run(&self, netlist: &str, cancel: &Cancel, _live: &Live) -> Result<Output, SimError> {
        let dir = tempfile::Builder::new().prefix("pcb-designer-sim").tempdir().map_err(SimError::io)?;
        let cir = dir.path().join("circuit.cir");
        let raw_path = dir.path().join("out.raw");
        std::fs::write(&cir, netlist).map_err(SimError::io)?;
        // LTspice and PSpice compatibility (as KiCad runs it): vendor models need it
        // (`params:` subcircuits, their expressions). Read at start-up from the folder.
        std::fs::write(dir.path().join(".spiceinit"), "set ngbehavior=ltpsa\n").map_err(SimError::io)?;
        let mut child = Command::new(&self.path)
            .arg("-b")
            .arg("-r")
            .arg(&raw_path)
            .arg(&cir)
            .current_dir(dir.path())
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| SimError::NotFound(format!("can't start {}: {e}", self.path.display())))?;
        let out = drain(child.stdout.take());
        let err = drain(child.stderr.take());
        let status = wait(&mut child, cancel)?;
        let stdout = out.join().unwrap_or_default();
        let stderr = err.join().unwrap_or_default();
        let log = format!("{stdout}{stderr}");
        let Some(status) = status else {
            // Killed: the points written so far, if any.
            let plots = std::fs::read(&raw_path).ok().and_then(|d| raw::parse(&d).ok()).unwrap_or_default();
            if plots.first().is_some_and(|p| !p.is_empty()) {
                return Ok(Output { plots, log, warnings: Vec::new(), stopped: true });
            }
            return Err(SimError::Cancelled);
        };
        let errors = error_lines(&stderr);
        if !status.success() || !raw_path.is_file() {
            let message = if errors.is_empty() { format!("ngspice failed ({status})") } else { errors.join("\n") };
            return Err(SimError::Failed { message, log });
        }
        let data = std::fs::read(&raw_path).map_err(SimError::io)?;
        let mut plots: Vec<Plot> = raw::parse(&data)?;
        // ngspice writes the plots last analysis first.
        plots.reverse();
        let warnings = warning_lines(&log);
        Ok(Output { plots, log, warnings, stopped: false })
    }

    fn describe(&self) -> String {
        format!("{}, process", self.version().unwrap_or_else(|| "ngspice".into()))
    }
}

/// Reads a pipe to its end on a thread (both pipes must be drained, or ngspice blocks
/// writing into a full one).
fn drain(pipe: Option<impl Read + Send + 'static>) -> std::thread::JoinHandle<String> {
    std::thread::spawn(move || {
        let mut text = String::new();
        if let Some(mut p) = pipe {
            let mut bytes = Vec::new();
            let _ = p.read_to_end(&mut bytes);
            text = String::from_utf8_lossy(&bytes).into_owned();
        }
        text
    })
}

/// Waits for the process; `None` when it was cancelled (and killed).
fn wait(child: &mut Child, cancel: &Cancel) -> Result<Option<std::process::ExitStatus>, SimError> {
    loop {
        if let Some(status) = child.try_wait().map_err(SimError::io)? {
            return Ok(Some(status));
        }
        if cancel.is_cancelled() {
            let _ = child.kill();
            let _ = child.wait();
            return Ok(None);
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// ngspice's error messages: lines with "error" and the line after an "Error on line"
/// (which quotes the netlist line), without the noise around them.
pub(crate) fn error_lines(stderr: &str) -> Vec<String> {
    let lines: Vec<&str> = stderr.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
    let mut out = Vec::new();
    for (i, l) in lines.iter().enumerate() {
        let lower = l.to_ascii_lowercase();
        if lower.contains("error") || lower.starts_with("warning") || lower.contains("can't find") {
            out.push(l.to_string());
            // The quoted netlist line.
            if (lower.ends_with("from line") || lower.contains("on line"))
                && let Some(next) = lines.get(i + 1)
            {
                out.push(format!("  {next}"));
            }
        }
    }
    out.dedup();
    out
}

/// Warnings of a run that succeeded (timestep troubles, unknown parameters).
pub(crate) fn warning_lines(log: &str) -> Vec<String> {
    log.lines().map(str::trim).filter(|l| l.to_ascii_lowercase().starts_with("warning")).map(str::to_string).collect()
}
