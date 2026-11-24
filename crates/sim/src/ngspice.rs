//! What every way of running ngspice shares: [`Cancel`], and reading errors and warnings
//! out of its log.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

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
