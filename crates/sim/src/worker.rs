//! libngspice in a separate process of our own (a *worker*): the app's executable with
//! `--sim-worker` (tests: the `sim-worker` binary) loads the library and runs one
//! netlist. That gives parallel runs (a sweep's steps; the library is one instance per
//! process), a fresh instance every run, and a crashing model can't take the app down.
//! Stop kills it.
//!
//! Protocol: the parent starts `PROGRAM ARGS… LIBRARY` and writes the netlist to its
//! stdin. The worker answers on stdout with lines of tab-separated fields, each starting
//! with [`MARK`] (ngspice may print to stdout itself: other lines are log). Text fields
//! are escaped (`\\`, `\n`, `\t`); numbers are Rust's shortest round-trip form.
//!
//! * `begin NAME COMPLEX VECTOR…` (a vector `name:kind`) and `point RE IM…`: the live
//!   plot as it grows; `progress P` (0…1).
//! * At the end: `plot …` like `begin`, its `data RE IM…` points, `log TEXT`,
//!   `warning TEXT`, then `done STOPPED`, or `error KIND MESSAGE LOG` (KIND `failed`,
//!   `notfound`, `raw`, `cancelled`, `io`).
//! * `version V` answers `--version LIBRARY`.

use std::ffi::OsString;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use crate::raw::{Plot, Vector};
use crate::{Backend, Cancel, Libngspice, Live, Output, SimError};

/// Starts every protocol line.
const MARK: &str = "\u{1}sim";

/// How to start a worker: the app's executable and `--sim-worker`.
#[derive(Clone, Debug, PartialEq)]
pub struct WorkerCommand {
    pub program: PathBuf,
    pub args: Vec<OsString>,
}

impl WorkerCommand {
    /// The running executable with `arg` (the app: `--sim-worker`).
    pub fn current_exe(arg: &str) -> Option<WorkerCommand> {
        Some(WorkerCommand { program: std::env::current_exe().ok()?, args: vec![arg.into()] })
    }
}

/// Runs netlists in worker processes with the library at `library`.
#[derive(Clone, Debug)]
pub struct Worker {
    pub command: WorkerCommand,
    pub library: PathBuf,
    version: Arc<OnceLock<Option<String>>>,
}

impl PartialEq for Worker {
    fn eq(&self, other: &Worker) -> bool {
        self.command == other.command && self.library == other.library
    }
}

impl Worker {
    pub fn new(command: WorkerCommand, library: PathBuf) -> Worker {
        Worker { command, library, version: Arc::default() }
    }

    fn spawn(&self, extra: Option<&str>) -> Result<std::process::Child, SimError> {
        let mut cmd = Command::new(&self.command.program);
        cmd.args(&self.command.args);
        if let Some(e) = extra {
            cmd.arg(e);
        }
        cmd.arg(&self.library)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| SimError::NotFound(format!("can't start {}: {e}", self.command.program.display())))
    }

    /// "ngspice-47", asked of a worker once.
    pub fn version(&self) -> Option<String> {
        self.version
            .get_or_init(|| {
                let out = self.spawn(Some("--version")).ok()?.wait_with_output().ok()?;
                String::from_utf8_lossy(&out.stdout).lines().find_map(|l| {
                    let f = fields(l)?;
                    (f.first().map(String::as_str) == Some("version")).then(|| f.get(1).cloned()).flatten()
                })
            })
            .clone()
    }
}

impl Backend for Worker {
    fn run(&self, netlist: &str, cancel: &Cancel, live: &Live) -> Result<Output, SimError> {
        let mut child = self.spawn(None)?;
        let mut stdin = child.stdin.take().expect("piped");
        let netlist = netlist.to_string();
        // On a thread: a large netlist mustn't block on a full pipe while the worker
        // already writes.
        let writer = std::thread::spawn(move || {
            let _ = stdin.write_all(netlist.as_bytes());
        });
        let stderr = drain(child.stderr.take().expect("piped"));
        let reader = {
            let stdout = child.stdout.take().expect("piped");
            let live = live.clone();
            std::thread::spawn(move || read_answer(stdout, &live))
        };
        // Wait, killing it when cancelled.
        let status = loop {
            if let Some(status) = child.try_wait().map_err(SimError::io)? {
                break Some(status);
            }
            if cancel.is_cancelled() {
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
            std::thread::sleep(Duration::from_millis(5));
        };
        let _ = writer.join();
        let answer = reader.join().unwrap_or_default();
        let stderr = stderr.join().unwrap_or_default();
        let mut log = answer.log.clone();
        log.push_str(&stderr);
        if status.is_none() {
            // Stopped: the points that came so far.
            return match live.plot() {
                Some(plot) if !plot.is_empty() => {
                    Ok(Output { plots: vec![plot], log, warnings: answer.warnings, stopped: true })
                }
                _ => Err(SimError::Cancelled),
            };
        }
        match answer.end {
            Some(End::Done { stopped }) => Ok(Output { plots: answer.plots, log, warnings: answer.warnings, stopped }),
            Some(End::Error(e)) => Err(match e {
                SimError::Failed { message, log: l } => SimError::Failed { message, log: l + &stderr },
                e => e,
            }),
            None => {
                let status = status.map(|s| s.to_string()).unwrap_or_default();
                Err(SimError::Failed { message: format!("ngspice crashed ({status})"), log })
            }
        }
    }

    fn describe(&self) -> String {
        format!("lib{}, worker process", self.version().unwrap_or_else(|| "ngspice".into()))
    }
}

fn drain(mut pipe: impl Read + Send + 'static) -> std::thread::JoinHandle<String> {
    std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = pipe.read_to_end(&mut bytes);
        String::from_utf8_lossy(&bytes).into_owned()
    })
}

#[derive(Default)]
struct Answer {
    plots: Vec<Plot>,
    log: String,
    warnings: Vec<String>,
    end: Option<End>,
}

enum End {
    Done { stopped: bool },
    Error(SimError),
}

/// Reads the worker's stdout: live points into `live`, the rest into the answer.
fn read_answer(stdout: impl Read, live: &Live) -> Answer {
    let mut a = Answer::default();
    for line in BufReader::new(stdout).lines() {
        let Ok(line) = line else { break };
        let Some(f) = fields(&line) else {
            a.log.push_str(&line);
            a.log.push('\n');
            continue;
        };
        let nums = || f[1..].iter().map(|x| x.parse::<f64>().unwrap_or(f64::NAN)).collect::<Vec<_>>();
        match f[0].as_str() {
            "begin" => {
                if let Some(p) = plot_header(&f[1..]) {
                    live.begin(p);
                }
            }
            "point" => live.push(&pairs(&nums())),
            "progress" => live.set_progress(nums().first().copied().unwrap_or(0.0)),
            "plot" => a.plots.extend(plot_header(&f[1..])),
            "data" => {
                if let Some(p) = a.plots.last_mut() {
                    for (v, (re, im)) in p.vectors.iter_mut().zip(pairs(&nums())) {
                        v.re.push(re);
                        if let Some(i) = &mut v.im {
                            i.push(im);
                        }
                    }
                }
            }
            "log" => {
                a.log.push_str(f.get(1).map_or("", String::as_str));
            }
            "warning" => a.warnings.extend(f.get(1).cloned()),
            "done" => a.end = Some(End::Done { stopped: f.get(1).is_some_and(|s| s == "1") }),
            "error" => {
                let (kind, message, log) = (f.get(1), f.get(2).cloned().unwrap_or_default(), f.get(3).cloned());
                a.end = Some(End::Error(match kind.map(String::as_str) {
                    Some("notfound") => SimError::NotFound(message),
                    Some("raw") => SimError::Raw(message),
                    Some("cancelled") => SimError::Cancelled,
                    Some("io") => SimError::Io(message),
                    _ => SimError::Failed { message, log: log.unwrap_or_default() },
                }));
            }
            _ => {}
        }
    }
    a
}

fn pairs(n: &[f64]) -> Vec<(f64, f64)> {
    n.chunks(2).map(|c| (c[0], c.get(1).copied().unwrap_or(0.0))).collect()
}

/// A plot without points from `NAME COMPLEX VECTOR…`.
fn plot_header(f: &[String]) -> Option<Plot> {
    let (name, complex) = (f.first()?.clone(), f.get(1)? == "1");
    let vectors = f[2..]
        .iter()
        .map(|v| {
            let (name, kind) = v.split_once(':').unwrap_or((v, ""));
            Vector { name: name.into(), kind: kind.into(), re: Vec::new(), im: complex.then(Vec::new) }
        })
        .collect();
    Some(Plot { name, complex, vectors })
}

/// The fields of a protocol line, unescaped; `None` for other lines.
fn fields(line: &str) -> Option<Vec<String>> {
    let rest = line.strip_prefix(MARK)?.strip_prefix('\t')?;
    Some(rest.split('\t').map(unescape).collect())
}

fn escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('\n', "\\n").replace('\t', "\\t").replace('\r', "")
}

fn unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some(c) => out.push(c),
            None => {}
        }
    }
    out
}

// ----------------------------------------------------------------------------------------
// The worker's side

/// What the worker writes: one protocol line at a time.
struct Out(Mutex<std::io::Stdout>);

impl Out {
    fn line(&self, fields: &[String]) {
        let mut s = MARK.to_string();
        for f in fields {
            s.push('\t');
            s.push_str(f);
        }
        s.push('\n');
        let mut out = self.0.lock().unwrap_or_else(|e| e.into_inner());
        let _ = out.write_all(s.as_bytes());
        let _ = out.flush();
    }

    fn header(&self, tag: &str, p: &Plot) {
        let mut f = vec![tag.to_string(), escape(&p.name), if p.complex { "1" } else { "0" }.into()];
        f.extend(p.vectors.iter().map(|v| escape(&format!("{}:{}", v.name, v.kind))));
        self.line(&f);
    }

    fn point(&self, tag: &str, p: &Plot, i: usize) {
        let mut f = vec![tag.to_string()];
        for v in &p.vectors {
            f.push(v.re[i].to_string());
            f.push(v.im.as_ref().map_or(0.0, |im| im[i]).to_string());
        }
        self.line(&f);
    }
}

/// The worker: `[--version] LIBRARY`, the netlist on stdin. The app calls it for
/// `--sim-worker`, before anything else.
pub fn worker_main(args: &[OsString]) -> i32 {
    let out = Out(Mutex::new(std::io::stdout()));
    let (version, library) = match args {
        [flag, lib] if flag == "--version" => (true, lib),
        [lib] => (false, lib),
        _ => {
            eprintln!("usage: --sim-worker [--version] LIBRARY  (netlist on stdin)");
            return 2;
        }
    };
    let lib = match Libngspice::load(Path::new(library)) {
        Ok(l) => l,
        Err(e) => {
            report_error(&out, &e);
            return 1;
        }
    };
    if version {
        out.line(&["version".into(), escape(&lib.version().unwrap_or_default())]);
        return 0;
    }
    let mut netlist = String::new();
    if let Err(e) = std::io::stdin().read_to_string(&mut netlist) {
        report_error(&out, &SimError::io(e));
        return 1;
    }
    let live = Live::default();
    let out = Arc::new(out);
    let finished = Arc::new(AtomicBool::new(false));
    // Streams the live plot's new points every few milliseconds.
    let streamer = {
        let (live, out, finished) = (live.clone(), out.clone(), finished.clone());
        std::thread::spawn(move || {
            let mut sent = Plot::default();
            let mut progress = None;
            loop {
                let last = finished.load(Ordering::SeqCst);
                let before = (sent.name.clone(), sent.vectors.len(), sent.len());
                live.update(&mut sent);
                let restarted = sent.name != before.0 || sent.vectors.len() != before.1 || sent.len() < before.2;
                let from = if restarted {
                    if !sent.vectors.is_empty() {
                        out.header("begin", &sent);
                    }
                    0
                } else {
                    before.2
                };
                for i in from..sent.len() {
                    out.point("point", &sent, i);
                }
                let p = live.progress();
                if p != progress {
                    progress = p;
                    out.line(&["progress".into(), p.unwrap_or(0.0).to_string()]);
                }
                if last {
                    break;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
        })
    };
    let result = lib.run(&netlist, &Cancel::default(), &live);
    finished.store(true, Ordering::SeqCst);
    let _ = streamer.join();
    match result {
        Ok(output) => {
            for p in &output.plots {
                out.header("plot", p);
                for i in 0..p.len() {
                    out.point("data", p, i);
                }
            }
            out.line(&["log".into(), escape(&output.log)]);
            for w in &output.warnings {
                out.line(&["warning".into(), escape(w)]);
            }
            out.line(&["done".into(), if output.stopped { "1" } else { "0" }.into()]);
            0
        }
        Err(e) => {
            report_error(&out, &e);
            1
        }
    }
}

fn report_error(out: &Out, e: &SimError) {
    let (kind, log) = match e {
        SimError::NotFound(_) => ("notfound", String::new()),
        SimError::Raw(_) => ("raw", String::new()),
        SimError::Cancelled => ("cancelled", String::new()),
        SimError::Io(_) => ("io", String::new()),
        SimError::Failed { log, .. } => ("failed", log.clone()),
        SimError::Netlist(_) => ("failed", String::new()),
    };
    let message = match e {
        SimError::Failed { message, .. } => message.clone(),
        e => e.to_string(),
    };
    out.line(&["error".into(), kind.into(), escape(&message), escape(&log)]);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escaping_round_trips() {
        for s in ["", "a\tb", "line\nnext\\n", "C:\\path\\x"] {
            assert_eq!(unescape(&escape(s)), s);
        }
        let line = format!("{MARK}\tlog\t{}", escape("a\tb\nc"));
        assert_eq!(fields(&line).unwrap(), vec!["log".to_string(), "a\tb\nc".into()]);
        assert_eq!(fields("ngspice says hi"), None);
    }

    #[test]
    fn numbers_round_trip() {
        for x in [0.1, 1e-300, -3.75e12, f64::MAX, 0.37499999971875003] {
            assert_eq!(x.to_string().parse::<f64>().unwrap(), x);
        }
    }
}
