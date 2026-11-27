//! ngspice compiled to WebAssembly (scripts/build-ngspice-wasm.sh), for the browser
//! build: each run goes to a Web Worker (`web/ngspice-worker.mjs`, served beside the app
//! as `ngspice/`), which runs it synchronously and posts back what the shared library's
//! callbacks give: the vectors (`init`), the points in batches (`data`), the status
//! (`stat`), and at the end the log and the raw file (`done`).
//!
//! * The Emscripten module can't link into the app's wasm (another ABI), hence workers
//!   and messages. A worker keeps its ngspice for the next run; when ngspice asks to exit
//!   the worker starts a fresh instance (milliseconds) instead of the desktop's fallback
//!   to processes.
//! * Stop terminates the worker: it's busy in `run` and reads no messages. The result is
//!   the streamed points so far, as the desktop's `bg_halt` + `write` gives.
//! * Sweeps run several workers ([`Backend::parallel`]); `Session` starts them.
//!
//! What decodes the messages ([`Stream`], [`finish`]) is plain Rust, tested on the
//! desktop; the workers are in `web` (wasm only).

#[cfg(target_arch = "wasm32")]
use crate::Pending;
use crate::ngspice::{error_lines, warning_lines};
use crate::raw::{self, Plot};
use crate::shared::{live_plot, progress};
use crate::{Backend, Cancel, Live, Output, SimError};

/// ngspice in the browser's Web Workers.
#[derive(Clone, Copy, Debug)]
pub struct Ngspice;

impl Backend for Ngspice {
    fn run(&self, _: &str, _: &Cancel, _: &Live) -> Result<Output, SimError> {
        Err(SimError::NotFound("ngspice in the browser only runs started (Backend::start)".into()))
    }

    #[cfg(target_arch = "wasm32")]
    fn start(&self, netlist: &str, cancel: &Cancel, live: &Live) -> Option<Box<dyn Pending + Send>> {
        Some(Box::new(web::start(netlist, cancel, live)))
    }

    fn describe(&self) -> String {
        "ngspice, WebAssembly (in the browser)".into()
    }
}

/// Whether the app comes with ngspice (the page found `ngspice/`; see [`set_available`]).
pub fn available() -> bool {
    AVAILABLE.load(std::sync::atomic::Ordering::Relaxed)
}

/// Tells [`available`]: the browser app checks once at start whether it was built with
/// ngspice.
pub fn set_available(yes: bool) {
    AVAILABLE.store(yes, std::sync::atomic::Ordering::Relaxed);
}

static AVAILABLE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// A run's streamed points going into its [`Live`] plot.
#[derive(Default)]
pub(crate) struct Stream {
    pub live: Live,
    /// The plot's vectors by the index ngspice sends them at.
    order: Vec<usize>,
    /// Vectors per point as sent.
    width: usize,
}

impl Stream {
    pub fn new(live: Live) -> Stream {
        Stream { live, ..Stream::default() }
    }

    /// A plot begins: ngspice's kind ("tran1") and vectors (name, complex).
    pub fn init(&mut self, kind: &str, vectors: &[(String, bool)]) {
        let (order, plot) = live_plot(kind, vectors);
        self.order = order;
        self.width = vectors.len();
        self.live.begin(plot);
    }

    /// Points: per point a real and an imaginary part per vector, in the order sent.
    pub fn data(&self, values: &[f64]) {
        if self.width == 0 {
            return;
        }
        for point in values.chunks_exact(2 * self.width) {
            let ordered: Vec<(f64, f64)> = self.order.iter().map(|&k| (point[2 * k], point[2 * k + 1])).collect();
            self.live.push(&ordered);
        }
    }

    /// A status line ("tran: 42.3%").
    pub fn stat(&self, status: &str) {
        if let Some(p) = progress(status) {
            self.live.set_progress(p);
        }
    }
}

/// How a worker's run ended.
#[derive(Debug, Default)]
pub(crate) struct Ended {
    pub stdout: String,
    pub stderr: String,
    /// `ngSpice_Circ` failed or ngspice said "circuit not parsed".
    pub not_loaded: bool,
    /// ngspice asked to exit (the worker starts a new one for the next run).
    pub exited: bool,
    /// The raw file `write` wrote (`None` when it failed).
    pub raw: Option<Vec<u8>>,
}

/// A run's output, as [`crate::Libngspice`]'s `run` makes it.
pub(crate) fn finish(end: Ended) -> Result<Output, SimError> {
    let log = format!("{}{}", end.stdout, end.stderr);
    let failed = |fallback: &str| {
        let errors: Vec<String> =
            error_lines(&end.stderr).into_iter().filter(|l| !l.contains("no circuit loaded")).collect();
        let message = if errors.is_empty() { fallback.to_string() } else { errors.join("\n") };
        Err(SimError::Failed { message, log: log.clone() })
    };
    if end.exited {
        return failed("ngspice exited");
    }
    if end.not_loaded {
        return failed("ngspice couldn't read the netlist");
    }
    let plots: Vec<Plot> = match &end.raw {
        Some(data) => raw::parse(data)?,
        None => Vec::new(),
    };
    // `write` of nothing run writes the constants plot.
    let plots: Vec<Plot> = plots.into_iter().filter(|p| !p.name.eq_ignore_ascii_case("constants")).collect();
    if plots.is_empty() {
        return failed("ngspice wrote no results");
    }
    let warnings = warning_lines(&log);
    Ok(Output { plots, log, warnings, stopped: false })
}

/// A stopped run's output: the points streamed so far, or [`SimError::Cancelled`].
pub(crate) fn stopped(live: &Live) -> Result<Output, SimError> {
    match live.plot() {
        Some(plot) => Ok(Output { plots: vec![plot], log: String::new(), warnings: Vec::new(), stopped: true }),
        None => Err(SimError::Cancelled),
    }
}

#[cfg(target_arch = "wasm32")]
mod web {
    //! The Web Workers: started as needed, kept for the next run (thread-local: the
    //! browser's main thread is the only one).

    use std::cell::RefCell;
    use std::collections::HashMap;

    use js_sys::{Array, Float64Array, Object, Reflect, Uint8Array};
    use wasm_bindgen::JsCast;
    use wasm_bindgen::prelude::*;
    use web_sys::{ErrorEvent, MessageEvent, Worker, WorkerOptions, WorkerType};

    use super::{Ended, Stream, finish, stopped};
    use crate::{Cancel, Live, Output, Pending, SimError};

    /// The worker script, relative to the page.
    const SCRIPT: &str = "ngspice/ngspice-worker.mjs";

    struct Slot {
        worker: Worker,
        /// The run it's busy with.
        run: Option<u64>,
        /// Failed: removed by the next start (not in its own callback, which it owns).
        dead: bool,
        _message: Closure<dyn FnMut(MessageEvent)>,
        _error: Closure<dyn FnMut(ErrorEvent)>,
    }

    struct Running {
        stream: Stream,
        ended: Option<Result<Output, SimError>>,
    }

    #[derive(Default)]
    struct Pool {
        slots: Vec<Slot>,
        runs: HashMap<u64, Running>,
        next: u64,
    }

    thread_local! {
        static POOL: RefCell<Pool> = RefCell::default();
    }

    /// A run in a worker.
    pub struct Run {
        id: u64,
        cancel: Cancel,
        live: Live,
    }

    pub fn start(netlist: &str, cancel: &Cancel, live: &Live) -> Run {
        let id = POOL.with(|p| {
            let mut p = p.borrow_mut();
            p.next += 1;
            let id = p.next;
            p.runs.insert(id, Running { stream: Stream::new(live.clone()), ended: None });
            id
        });
        let message = Object::new();
        let _ = Reflect::set(&message, &"id".into(), &JsValue::from_f64(id as f64));
        let _ = Reflect::set(&message, &"netlist".into(), &JsValue::from_str(netlist));
        let posted = idle_worker().and_then(|w| {
            POOL.with(|p| {
                if let Some(s) = p.borrow_mut().slots.iter_mut().find(|s| s.worker == w && !s.dead) {
                    s.run = Some(id);
                }
            });
            w.post_message(&message)
        });
        if let Err(e) = posted {
            end(id, Err(SimError::NotFound(format!("ngspice couldn't start: {}", describe(&e)))));
        }
        Run { id, cancel: cancel.clone(), live: live.clone() }
    }

    impl Pending for Run {
        fn poll(&mut self) -> Option<Result<Output, SimError>> {
            POOL.with(|p| {
                let mut p = p.borrow_mut();
                if let Some(ended) = p.runs.get_mut(&self.id).and_then(|r| r.ended.take()) {
                    p.runs.remove(&self.id);
                    return Some(ended);
                }
                if !self.cancel.is_cancelled() {
                    return None;
                }
                // Busy in `run`, the worker reads no messages: end it.
                if let Some(k) = p.slots.iter().position(|s| s.run == Some(self.id)) {
                    p.slots.remove(k).worker.terminate();
                }
                p.runs.remove(&self.id);
                Some(stopped(&self.live))
            })
        }
    }

    impl Drop for Run {
        /// A run dropped before it ended (abandoned for another): its worker is ended.
        fn drop(&mut self) {
            POOL.with(|p| {
                let mut p = p.borrow_mut();
                if p.runs.remove(&self.id).is_some()
                    && let Some(k) = p.slots.iter().position(|s| s.run == Some(self.id))
                {
                    p.slots.remove(k).worker.terminate();
                }
            });
        }
    }

    /// The run's end (its worker is free again).
    fn end(id: u64, result: Result<Output, SimError>) {
        POOL.with(|p| {
            let mut p = p.borrow_mut();
            if let Some(r) = p.runs.get_mut(&id) {
                r.ended = Some(result);
            }
            for s in p.slots.iter_mut().filter(|s| s.run == Some(id)) {
                s.run = None;
            }
        });
    }

    fn idle_worker() -> Result<Worker, JsValue> {
        let idle = POOL.with(|p| {
            let mut p = p.borrow_mut();
            p.slots.retain(|s| !s.dead);
            p.slots.iter().find(|s| s.run.is_none()).map(|s| s.worker.clone())
        });
        if let Some(w) = idle {
            return Ok(w);
        }
        let options = WorkerOptions::new();
        options.set_type(WorkerType::Module);
        let worker = Worker::new_with_options(SCRIPT, &options)?;
        let message = Closure::<dyn FnMut(MessageEvent)>::new(received);
        worker.set_onmessage(Some(message.as_ref().unchecked_ref()));
        // The script didn't load or threw: its run fails, and the worker goes.
        let w = worker.clone();
        let error = Closure::<dyn FnMut(ErrorEvent)>::new(move |e: ErrorEvent| {
            e.prevent_default();
            let run = POOL.with(|p| {
                let mut p = p.borrow_mut();
                let s = p.slots.iter_mut().find(|s| s.worker == w)?;
                s.dead = true;
                s.run.take()
            });
            w.terminate();
            let text = e.message();
            let why = if text.is_empty() { "the worker didn't load".to_string() } else { text };
            log::error!("ngspice worker: {why}");
            if let Some(id) = run {
                end(id, Err(SimError::NotFound(format!("ngspice couldn't start: {why}"))));
            }
        });
        worker.set_onerror(Some(error.as_ref().unchecked_ref()));
        POOL.with(|p| {
            p.borrow_mut().slots.push(Slot {
                worker: worker.clone(),
                run: None,
                dead: false,
                _message: message,
                _error: error,
            })
        });
        Ok(worker)
    }

    fn get(o: &JsValue, key: &str) -> JsValue {
        Reflect::get(o, &key.into()).unwrap_or(JsValue::UNDEFINED)
    }

    fn string(o: &JsValue, key: &str) -> String {
        get(o, key).as_string().unwrap_or_default()
    }

    fn describe(e: &JsValue) -> String {
        e.as_string()
            .or_else(|| e.dyn_ref::<js_sys::Error>().map(|e| String::from(e.message())))
            .unwrap_or_else(|| format!("{e:?}"))
    }

    /// A message from a worker.
    fn received(e: MessageEvent) {
        let m = e.data();
        let Some(id) = get(&m, "id").as_f64().map(|i| i as u64) else { return };
        match string(&m, "type").as_str() {
            "init" => {
                let names = Array::from(&get(&m, "names"));
                let complex = Array::from(&get(&m, "complex"));
                let vectors: Vec<(String, bool)> = names
                    .iter()
                    .zip(complex.iter())
                    .map(|(n, c)| (n.as_string().unwrap_or_default(), c.is_truthy()))
                    .collect();
                let kind = string(&m, "kind");
                with_stream(id, |s| s.init(&kind, &vectors));
            }
            "data" => {
                let values = Float64Array::from(get(&m, "values")).to_vec();
                with_stream(id, |s| s.data(&values));
            }
            "stat" => {
                let text = string(&m, "text");
                with_stream(id, |s| s.stat(&text));
            }
            "done" => {
                let raw = get(&m, "raw");
                let ended = Ended {
                    stdout: string(&m, "stdout"),
                    stderr: string(&m, "stderr"),
                    not_loaded: get(&m, "notLoaded").is_truthy(),
                    exited: get(&m, "exited").is_truthy(),
                    raw: (!raw.is_null() && !raw.is_undefined()).then(|| Uint8Array::new(&raw).to_vec()),
                };
                end(id, finish(ended));
            }
            "failed" => end(id, Err(SimError::NotFound(format!("ngspice couldn't start: {}", string(&m, "message"))))),
            _ => {}
        }
    }

    fn with_stream(id: u64, f: impl FnOnce(&mut Stream)) {
        POOL.with(|p| {
            if let Some(r) = p.borrow_mut().runs.get_mut(&id) {
                f(&mut r.stream);
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn streamed_points_go_in_plot_order() {
        let mut s = Stream::new(Live::default());
        s.init("tran1", &[("out".into(), false), ("time".into(), false)]);
        // Two points: (out, time) = (0.5, 0), (0.75, 1e-3).
        s.data(&[0.5, 0.0, 0.0, 0.0, 0.75, 0.0, 1e-3, 0.0]);
        s.stat("tran: 50%");
        let plot = s.live.plot().unwrap();
        assert_eq!(plot.vectors[0].name, "time");
        assert_eq!(plot.vectors[0].re, [0.0, 1e-3]);
        assert_eq!(plot.vectors[1].re, [0.5, 0.75]);
        assert_eq!(s.live.progress(), Some(0.5));
    }

    #[test]
    fn complex_points_keep_their_imaginary_parts() {
        let mut s = Stream::new(Live::default());
        s.init("ac1", &[("frequency".into(), true), ("out".into(), true)]);
        s.data(&[10.0, 0.0, 0.5, -0.5]);
        let plot = s.live.plot().unwrap();
        assert_eq!(plot.vectors[1].im.as_deref(), Some(&[-0.5][..]));
    }

    #[test]
    fn ends_report_errors_like_the_library() {
        let stderr = "Error: Missing token in line 2:\nr1 a 0\n".to_string();
        let end = Ended { stderr, exited: true, ..Ended::default() };
        let Err(SimError::Failed { message, log }) = finish(end) else { panic!("expected a failure") };
        assert!(message.contains("Missing token"), "{message}");
        assert!(log.contains("r1 a 0"));
        let Err(SimError::Failed { message, .. }) = finish(Ended { not_loaded: true, ..Ended::default() }) else {
            panic!()
        };
        assert_eq!(message, "ngspice couldn't read the netlist");
        assert!(
            matches!(finish(Ended::default()), Err(SimError::Failed { message, .. }) if message.contains("no results"))
        );
    }

    #[test]
    fn a_stopped_run_keeps_its_points() {
        let live = Live::default();
        assert!(matches!(stopped(&live), Err(SimError::Cancelled)));
        let mut s = Stream::new(live.clone());
        s.init("tran1", &[("time".into(), false), ("out".into(), false)]);
        s.data(&[0.0, 0.0, 1.0, 0.0]);
        let out = stopped(&live).unwrap();
        assert!(out.stopped);
        assert_eq!(out.plots[0].vectors[1].re, [1.0]);
    }
}
