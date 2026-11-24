//! A worker process (`sim::worker`) for tests; the app is its own worker
//! (`pcb-designer --sim-worker`).
fn main() {
    let args: Vec<std::ffi::OsString> = std::env::args_os().skip(1).collect();
    std::process::exit(sim::worker::worker_main(&args));
}
