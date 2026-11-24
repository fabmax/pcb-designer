//! ngspice as a shared library (libngspice), loaded at run time: no process start per
//! run, results streamed while the analysis runs, Stop through `bg_halt`.
//!
//! * One instance per process: runs are serialised by [`RUN`], and the callbacks write
//!   into one global [`SINK`]. Parameter sweeps run in parallel processes instead
//!   ([`Backend::parallel`]).
//! * The analysis runs on ngspice's own background thread (`bg_run`); the calling
//!   worker waits for `BGThreadRunning`, halting it when cancelled. Then `write` puts the
//!   plot (whole or as far as it got) into a raw file, read like the process backend's,
//!   so both name vectors alike.
//! * `SendInitData` / `SendData` stream every accepted point into the [`Live`] plot,
//!   `SendStat` the percentage.
//! * Gotchas: a crashing model takes the app down (the process backend is the setting
//!   for that). `ControlledExit` (ngspice wants to quit) mustn't end the app, but the
//!   instance is spent: calling `ngSpice_Init` again (or `ngSpice_Reset` first) leaves it
//!   corrupt, and later runs crash. So it's never run again in this process
//!   ([`Libngspice::exited`]; the engine falls back to processes). The library is never unloaded
//!   (its threads may still run). No lock may be held across an ngspice call that
//!   calls back (`SendChar` comes on the calling thread too).

use std::ffi::{CStr, CString, c_char, c_int, c_void};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use crate::ngspice::{error_lines, warning_lines};
use crate::raw::{self, Plot, Vector};
use crate::{Backend, Cancel, Live, Output, SimError};

type SendChar = unsafe extern "C" fn(*mut c_char, c_int, *mut c_void) -> c_int;
type SendStat = unsafe extern "C" fn(*mut c_char, c_int, *mut c_void) -> c_int;
type ControlledExit = unsafe extern "C" fn(c_int, bool, bool, c_int, *mut c_void) -> c_int;
type SendData = unsafe extern "C" fn(*mut VecValuesAll, c_int, c_int, *mut c_void) -> c_int;
type SendInitData = unsafe extern "C" fn(*mut VecInfoAll, c_int, *mut c_void) -> c_int;
type BgThreadRunning = unsafe extern "C" fn(bool, c_int, *mut c_void) -> c_int;

type Init = unsafe extern "C" fn(
    Option<SendChar>,
    Option<SendStat>,
    Option<ControlledExit>,
    Option<SendData>,
    Option<SendInitData>,
    Option<BgThreadRunning>,
    *mut c_void,
) -> c_int;
type CommandFn = unsafe extern "C" fn(*const c_char) -> c_int;
type Circ = unsafe extern "C" fn(*mut *mut c_char) -> c_int;

#[repr(C)]
struct VecValues {
    name: *mut c_char,
    creal: f64,
    cimag: f64,
    is_scale: bool,
    is_complex: bool,
}

#[repr(C)]
struct VecValuesAll {
    veccount: c_int,
    vecindex: c_int,
    vecsa: *mut *mut VecValues,
}

#[repr(C)]
struct VecInfo {
    number: c_int,
    vecname: *mut c_char,
    is_real: bool,
    pdvec: *mut c_void,
    pdvecscale: *mut c_void,
}

#[repr(C)]
struct VecInfoAll {
    name: *mut c_char,
    title: *mut c_char,
    date: *mut c_char,
    kind: *mut c_char,
    veccount: c_int,
    vecs: *mut *mut VecInfo,
}

/// A loaded library's entry points (the library itself is leaked: never unloaded).
struct Api {
    init: Init,
    command: CommandFn,
    circ: Circ,
    /// Initialised.
    ready: AtomicBool,
    version: Mutex<Option<String>>,
}

/// Libraries loaded so far, by path.
static LOADED: Mutex<Vec<(PathBuf, &'static Api)>> = Mutex::new(Vec::new());
/// Held for a whole run: one at a time per process.
static RUN: Mutex<()> = Mutex::new(());
/// Where the callbacks write (the run going).
static SINK: Mutex<Option<Sink>> = Mutex::new(None);
/// ngspice asked to exit: the instance can't run again in this process.
static EXITED: AtomicBool = AtomicBool::new(false);

#[derive(Default)]
struct Sink {
    stdout: String,
    stderr: String,
    live: Live,
    /// The plot's vectors by the index ngspice sends them at (the scale first).
    order: Vec<usize>,
    /// The background thread: started, ended.
    started: bool,
    ended: bool,
}

fn with_sink(f: impl FnOnce(&mut Sink)) {
    if let Ok(mut s) = SINK.lock()
        && let Some(s) = s.as_mut()
    {
        f(s);
    }
}

unsafe fn text(p: *const c_char) -> String {
    if p.is_null() { String::new() } else { unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned() }
}

unsafe extern "C" fn send_char(s: *mut c_char, _id: c_int, _user: *mut c_void) -> c_int {
    let line = unsafe { text(s) };
    with_sink(|sink| {
        let (out, l) = match line.split_once(' ') {
            Some(("stderr", l)) => (&mut sink.stderr, l),
            Some(("stdout", l)) => (&mut sink.stdout, l),
            _ => (&mut sink.stdout, line.as_str()),
        };
        out.push_str(l);
        out.push('\n');
    });
    0
}

unsafe extern "C" fn send_stat(s: *mut c_char, _id: c_int, _user: *mut c_void) -> c_int {
    // "tran: 42.3%", "--ready--".
    let status = unsafe { text(s) };
    let percent = status
        .split_once(':')
        .and_then(|(_, p)| p.trim().trim_end_matches('%').trim().parse::<f64>().ok())
        .map(|p| (p / 100.0).clamp(0.0, 1.0));
    if let Some(p) = percent {
        with_sink(|sink| sink.live.set_progress(p));
    }
    0
}

unsafe extern "C" fn controlled_exit(
    _status: c_int,
    _immediate: bool,
    _quit: bool,
    _id: c_int,
    _: *mut c_void,
) -> c_int {
    EXITED.store(true, Ordering::SeqCst);
    with_sink(|sink| {
        sink.stderr.push_str("ngspice exited\n");
        sink.ended = true;
    });
    0
}

unsafe extern "C" fn send_init_data(info: *mut VecInfoAll, _id: c_int, _user: *mut c_void) -> c_int {
    let Some(info) = (unsafe { info.as_ref() }) else { return 0 };
    let kind = unsafe { text(info.kind) };
    let mut vectors = Vec::new();
    for k in 0..info.veccount.max(0) as usize {
        let v = unsafe { (*info.vecs.add(k)).as_ref() };
        if let Some(v) = v {
            vectors.push(Vector {
                name: vector_name(&unsafe { text(v.vecname) }),
                kind: String::new(),
                re: Vec::new(),
                im: (!v.is_real).then(Vec::new),
            });
        }
    }
    // The scale first (ngspice sends it anywhere).
    let is_scale = |n: &str| matches!(n, "time" | "frequency") || n.ends_with("-sweep)");
    let mut order: Vec<usize> = (0..vectors.len()).collect();
    order.sort_by_key(|&k| !is_scale(&vectors[k].name));
    let mut vectors: Vec<Vector> = order.iter().map(|&k| std::mem::take(&mut vectors[k])).collect();
    let complex = vectors.iter().any(|v| v.im.is_some());
    let name = match kind.trim_end_matches(char::is_numeric) {
        "tran" => "Transient Analysis",
        "ac" => "AC Analysis",
        "dc" => "DC transfer characteristic",
        "op" => "Operating Point",
        _ => "",
    };
    // The scale's kind, as `scale()` expects it.
    if let Some(first) = vectors.first_mut() {
        first.kind = match first.name.as_str() {
            "time" => "time",
            "frequency" => "frequency",
            n if n.starts_with("v(i-") => "current",
            _ => "voltage",
        }
        .into();
    }
    for v in vectors.iter_mut().skip(1) {
        v.kind = if v.name.starts_with("i(") { "current" } else { "voltage" }.into();
    }
    with_sink(|sink| {
        sink.order = order;
        sink.live.begin(Plot { name: name.into(), complex, vectors });
    });
    0
}

/// A vector's name as the raw file has it: `out` → `v(out)`, `v1#branch` → `i(v1)`.
fn vector_name(name: &str) -> String {
    let lower = name.to_ascii_lowercase();
    if let Some(device) = lower.strip_suffix("#branch") {
        format!("i({device})")
    } else if matches!(lower.as_str(), "time" | "frequency") || lower.contains('(') {
        lower
    } else {
        format!("v({lower})")
    }
}

unsafe extern "C" fn send_data(values: *mut VecValuesAll, _count: c_int, _id: c_int, _user: *mut c_void) -> c_int {
    let Some(values) = (unsafe { values.as_ref() }) else { return 0 };
    let n = values.veccount.max(0) as usize;
    let mut point = Vec::with_capacity(n);
    for k in 0..n {
        if let Some(v) = unsafe { (*values.vecsa.add(k)).as_ref() } {
            point.push((v.creal, v.cimag));
        }
    }
    with_sink(|sink| {
        let ordered: Vec<(f64, f64)> = sink.order.iter().filter_map(|&k| point.get(k).copied()).collect();
        sink.live.push(&ordered);
    });
    0
}

unsafe extern "C" fn bg_running(not_running: bool, _id: c_int, _user: *mut c_void) -> c_int {
    with_sink(|sink| {
        if not_running {
            sink.ended = true;
        } else {
            sink.started = true;
        }
    });
    0
}

/// libngspice at a path.
#[derive(Clone)]
pub struct Libngspice {
    pub path: PathBuf,
    api: &'static Api,
}

impl std::fmt::Debug for Libngspice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Libngspice").field("path", &self.path).finish()
    }
}

impl PartialEq for Libngspice {
    fn eq(&self, other: &Libngspice) -> bool {
        self.path == other.path
    }
}

/// The library's file name in the app's bundle and in scripts/build-ngspice.sh's output.
const BUNDLED_NAME: &str = if cfg!(target_os = "macos") {
    "libngspice.0.dylib"
} else if cfg!(windows) {
    "ngspice.dll"
} else {
    "libngspice.so.0"
};

/// The libngspice that comes with the app, if any: beside the executable (macOS:
/// `Contents/Frameworks` of the `.app`; Linux: `lib/`; Windows: the exe's folder). In
/// development, scripts/build-ngspice.sh's output under `target/ngspice/<triple>/lib`,
/// looked for upwards from the executable (`target/<profile>/`, test binaries deeper).
pub fn bundled_library() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?;
    let mut candidates = vec![if cfg!(target_os = "macos") {
        dir.join("../Frameworks").join(BUNDLED_NAME)
    } else if cfg!(windows) {
        dir.join(BUNDLED_NAME)
    } else {
        dir.join("lib").join(BUNDLED_NAME)
    }];
    for up in dir.ancestors().skip(1).take(6) {
        candidates.push(up.join("ngspice").join(env!("SIM_TARGET")).join("lib").join(BUNDLED_NAME));
    }
    candidates.into_iter().find(|p| p.is_file())
}

/// Where libngspice is usually installed: the last resort, in development without
/// scripts/build-ngspice.sh's build.
const PLACES: &[&str] = &[
    "/opt/homebrew/lib/libngspice.0.dylib",
    "/opt/homebrew/lib/libngspice.dylib",
    "/usr/local/lib/libngspice.0.dylib",
    "/usr/local/lib/libngspice.dylib",
    "/usr/lib/x86_64-linux-gnu/libngspice.so.0",
    "/usr/lib/aarch64-linux-gnu/libngspice.so.0",
    "/usr/lib64/libngspice.so.0",
    "/usr/lib/libngspice.so.0",
    "/usr/local/lib/libngspice.so.0",
    "C:\\Program Files\\Spice64\\bin-dll\\ngspice.dll",
    "C:\\Program Files\\Spice64\\dll-vs\\ngspice.dll",
    "C:\\Program Files\\KiCad\\9.0\\bin\\ngspice.dll",
    "/Applications/KiCad/KiCad.app/Contents/Frameworks/libngspice.0.dylib",
];

impl Libngspice {
    /// ngspice asked to exit (a `quit`, or it gave up): no library runs any more in this
    /// process, whichever path was loaded (one instance per process).
    pub fn exited() -> bool {
        EXITED.load(Ordering::SeqCst)
    }

    /// `configured` if given (an error when it doesn't load), else `NGSPICE_LIBRARY` from
    /// the environment (also an error when it doesn't load), else the one that comes with
    /// the app ([`bundled_library`]), else the usual install places.
    pub fn find(configured: Option<&Path>) -> Result<Libngspice, SimError> {
        if let Some(path) = configured.filter(|p| !p.as_os_str().is_empty()) {
            return Libngspice::load(path);
        }
        if let Some(path) = std::env::var_os("NGSPICE_LIBRARY").filter(|p| !p.is_empty()) {
            return Libngspice::load(Path::new(&path));
        }
        let mut last = None;
        for path in bundled_library().into_iter().chain(PLACES.iter().map(PathBuf::from).filter(|p| p.is_file())) {
            match Libngspice::load(&path) {
                Ok(l) => return Ok(l),
                Err(e) => last = Some(e),
            }
        }
        Err(last.unwrap_or_else(|| SimError::NotFound("libngspice is not installed".into())))
    }

    /// Loads the library at `path` (once per path).
    pub fn load(path: &Path) -> Result<Libngspice, SimError> {
        let mut loaded = LOADED.lock().expect("loaded");
        if let Some((_, api)) = loaded.iter().find(|(p, _)| p == path) {
            return Ok(Libngspice { path: path.into(), api });
        }
        if !path.is_file() {
            return Err(SimError::NotFound(format!("{} is not a file", path.display())));
        }
        let fail = |e: libloading::Error| SimError::NotFound(format!("can't load {}: {e}", path.display()));
        // Safety: loading runs the library's initialisers; libngspice has none that
        // matter. The symbols' types are those of sharedspice.h.
        let api = unsafe {
            let lib = libloading::Library::new(path).map_err(fail)?;
            let init = *lib.get::<Init>(b"ngSpice_Init\0").map_err(fail)?;
            let command = *lib.get::<CommandFn>(b"ngSpice_Command\0").map_err(fail)?;
            let circ = *lib.get::<Circ>(b"ngSpice_Circ\0").map_err(fail)?;
            std::mem::forget(lib);
            Api { init, command, circ, ready: AtomicBool::new(false), version: Mutex::new(None) }
        };
        let api: &'static Api = Box::leak(Box::new(api));
        loaded.push((path.into(), api));
        Ok(Libngspice { path: path.into(), api })
    }

    /// "ngspice-47", from the banner of `version`.
    pub fn version(&self) -> Option<String> {
        let _run = RUN.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(v) = self.api.version.lock().expect("version").clone() {
            return Some(v);
        }
        self.ensure_init().ok()?;
        *SINK.lock().expect("sink") = Some(Sink::default());
        self.command("version -s");
        let sink = SINK.lock().expect("sink").take().unwrap_or_default();
        let v = sink.stdout.lines().find_map(|l| {
            let l = l.trim_start_matches('*').trim();
            l.starts_with("ngspice").then(|| l.split(':').next().unwrap_or(l).trim().to_string())
        });
        *self.api.version.lock().expect("version") = v.clone();
        v
    }

    fn command(&self, command: &str) -> c_int {
        let c = CString::new(command).expect("no NUL in commands");
        // Safety: a NUL-terminated string, copied by ngspice.
        unsafe { (self.api.command)(c.as_ptr()) }
    }

    /// Initialises the instance on first use.
    fn ensure_init(&self) -> Result<(), SimError> {
        if Libngspice::exited() {
            return Err(SimError::Failed {
                message: "ngspice exited and can't run again in this session: restart the app".into(),
                log: String::new(),
            });
        }
        if self.api.ready.load(Ordering::SeqCst) {
            return Ok(());
        }
        *SINK.lock().expect("sink") = Some(Sink::default());
        // Safety: the callbacks match sharedspice.h's typedefs and live forever.
        let r = unsafe {
            (self.api.init)(
                Some(send_char),
                Some(send_stat),
                Some(controlled_exit),
                Some(send_data),
                Some(send_init_data),
                Some(bg_running),
                std::ptr::null_mut(),
            )
        };
        let init = SINK.lock().expect("sink").take().unwrap_or_default();
        if r != 0 {
            return Err(SimError::Failed { message: "libngspice failed to start".into(), log: init.stderr });
        }
        // Without its spinit (a library's is looked for in a compiled-in folder), no
        // code models: POLY sources (vendor op amps) fail. Load them from beside it.
        if init.stderr.contains("can't find the initialization file") {
            for file in code_models(&self.path) {
                self.command(&format!("codemodel '{}'", file.display().to_string().replace('\\', "/")));
            }
        }
        // LTspice and PSpice compatibility, as the process backend's `.spiceinit` sets.
        self.command("set ngbehavior=ltpsa");
        self.api.ready.store(true, Ordering::SeqCst);
        Ok(())
    }
}

/// ngspice ended itself during a run: its errors, if any, else that it exited.
fn exited(sink: Sink) -> SimError {
    let errors: Vec<String> =
        error_lines(&sink.stderr).into_iter().filter(|l| !l.contains("no circuit loaded")).collect();
    let mut message = errors.join("\n");
    if !message.is_empty() {
        message.push('\n');
    }
    message.push_str("ngspice exited (later runs use ngspice processes)");
    SimError::Failed { message, log: format!("{}{}", sink.stdout, sink.stderr) }
}

/// The XSPICE code models beside a library: `ngspice/*.cm` in its folder (ours, Homebrew,
/// Linux, Spice64), the `.app`'s `PlugIns/ngspice` (ours on macOS), KiCad's
/// `PlugIns/sim/ngspice`.
fn code_models(library: &Path) -> Vec<PathBuf> {
    const MODELS: [&str; 7] =
        ["spice2poly.cm", "analog.cm", "digital.cm", "xtradev.cm", "xtraevt.cm", "table.cm", "tlines.cm"];
    let Some(dir) = library.parent() else { return Vec::new() };
    let candidates = [
        dir.join("ngspice"),
        dir.join("../lib/ngspice"),
        dir.join("../PlugIns/ngspice"),
        dir.join("../PlugIns/sim/ngspice"),
    ];
    let Some(folder) = candidates.iter().find(|d| d.join(MODELS[0]).is_file()) else { return Vec::new() };
    MODELS.iter().map(|m| folder.join(m)).filter(|p| p.is_file()).collect()
}

fn same_file(a: &Path, b: &Path) -> bool {
    a == b || matches!((a.canonicalize(), b.canonicalize()), (Ok(a), Ok(b)) if a == b)
}

/// How long `bg_run` may take to report its thread started.
const START_TIMEOUT: Duration = Duration::from_secs(5);

impl Backend for Libngspice {
    fn run(&self, netlist: &str, cancel: &Cancel, live: &Live) -> Result<Output, SimError> {
        let _run = RUN.lock().unwrap_or_else(|e| e.into_inner());
        self.ensure_init()?;
        let dir = tempfile::Builder::new().prefix("pcb-designer-sim").tempdir().map_err(SimError::io)?;
        let raw_path = dir.path().join("out.raw");
        *SINK.lock().expect("sink") = Some(Sink { live: live.clone(), ..Sink::default() });
        let take = || SINK.lock().expect("sink").take().unwrap_or_default();

        // The netlist, line by line (NULL-terminated array), title first, `.end` last.
        let lines: Vec<CString> =
            netlist.lines().map(|l| CString::new(l.replace('\0', "")).expect("NUL removed")).collect();
        let mut ptrs: Vec<*mut c_char> = lines.iter().map(|l| l.as_ptr() as *mut c_char).collect();
        ptrs.push(std::ptr::null_mut());
        // Safety: the array and its strings outlive the call; ngspice copies them.
        let loaded = unsafe { (self.api.circ)(ptrs.as_mut_ptr()) };
        let failed = |sink: Sink, fallback: &str| {
            let errors = error_lines(&sink.stderr);
            let message = if errors.is_empty() { fallback.to_string() } else { errors.join("\n") };
            SimError::Failed { message, log: format!("{}{}", sink.stdout, sink.stderr) }
        };
        // Errors ngspice continues after ("Error on line …" of a line it then drops)
        // fail neither here nor in a process.
        let not_parsed = SINK.lock().expect("sink").as_ref().is_some_and(|s| s.stderr.contains("circuit not parsed"));
        // ngspice asked to exit (a `quit`, or it gave up): the instance is spent.
        if EXITED.load(Ordering::SeqCst) {
            return Err(exited(take()));
        }
        if loaded != 0 || not_parsed {
            self.command("remcirc");
            return Err(failed(take(), "ngspice couldn't read the netlist"));
        }

        self.command("bg_run");
        let begun = Instant::now();
        let mut halted = false;
        loop {
            let (started, ended) = SINK.lock().expect("sink").as_ref().map_or((true, true), |s| (s.started, s.ended));
            if ended || (!started && begun.elapsed() > START_TIMEOUT) {
                break;
            }
            if cancel.is_cancelled() && !halted {
                self.command("bg_halt");
                halted = true;
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        // The halted run's results, as far as they got.
        // Single quotes (double ones stay in the name); forward slashes (a backslash
        // escapes).
        let written = self.command(&format!("write '{}'", raw_path.display().to_string().replace('\\', "/")));
        self.command("destroy all");
        self.command("remcirc");
        let sink = take();
        if EXITED.load(Ordering::SeqCst) {
            return Err(exited(sink));
        }
        let log = format!("{}{}", sink.stdout, sink.stderr);
        let plots: Vec<Plot> = match std::fs::read(&raw_path) {
            Ok(data) if written == 0 => raw::parse(&data)?,
            _ => Vec::new(),
        };
        // `write` of nothing run writes the constants plot.
        let plots: Vec<Plot> = plots.into_iter().filter(|p| !p.name.eq_ignore_ascii_case("constants")).collect();
        if plots.is_empty() {
            if halted {
                return Err(SimError::Cancelled);
            }
            return Err(failed(sink, "ngspice wrote no results"));
        }
        let warnings = warning_lines(&log);
        Ok(Output { plots, log, warnings, stopped: halted })
    }

    fn parallel(&self) -> bool {
        false
    }

    fn describe(&self) -> String {
        let v = self.version().unwrap_or_else(|| "libngspice".into());
        let bundled = bundled_library().is_some_and(|b| same_file(&b, &self.path));
        format!("lib{v}, shared library{}", if bundled { " (bundled)" } else { "" })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vector_names() {
        assert_eq!(vector_name("time"), "time");
        assert_eq!(vector_name("OUT"), "v(out)");
        assert_eq!(vector_name("V1#branch"), "i(v1)");
        assert_eq!(vector_name("v-sweep"), "v(v-sweep)");
    }
}
