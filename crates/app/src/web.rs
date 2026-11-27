//! The browser entry: runs the app in the page's canvas (`index.html`, built with Trunk).
//!
//! Files live in the browser's storage (`model::fs`, kept by `web_store`), projects in
//! [`HOME`]. It opens `?example=NAME`, else the last project, else the LED demo.

use std::cell::Cell;

use wasm_bindgen::JsCast;
use wasm_bindgen::closure::Closure;

use crate::app::{self, Options, Startup};
use view::demo::Example;

/// The canvas in `index.html`.
const CANVAS_ID: &str = "pcb_designer_canvas";

/// The folder of the projects in the browser's storage.
pub const HOME: &str = "/Projects";

pub fn start() {
    eframe::WebLogger::init(log::LevelFilter::Info).ok();
    watch_wheel();
    warn_before_leaving();
    wasm_bindgen_futures::spawn_local(async move {
        match crate::web_store::load().await {
            Ok(n) => log::info!("{n} files and folders from the browser's storage"),
            Err(e) => log::error!("browser storage not loaded (nothing will be kept): {e:?}"),
        }
        let _ = model::fs::create_dir_all(HOME);
        let opts = Options::new(startup());
        let document = web_sys::window().and_then(|w| w.document()).expect("a document");
        let canvas = document
            .get_element_by_id(CANVAS_ID)
            .and_then(|e| e.dyn_into::<web_sys::HtmlCanvasElement>().ok())
            .expect("the canvas in index.html");
        let started = eframe::WebRunner::new()
            .start(canvas, eframe::WebOptions::default(), Box::new(move |cc| Ok(Box::new(app::PcbApp::new(cc, opts)))))
            .await;
        if let Some(loading) = document.get_element_by_id("loading") {
            match started {
                Ok(()) => loading.remove(),
                Err(e) => {
                    let why = if has_webgpu() {
                        format!("PCB Designer could not start: {e:?}")
                    } else {
                        "PCB Designer needs a browser with WebGPU (a recent Chrome, Edge, Safari or Firefox).".into()
                    };
                    loading.set_inner_html(&format!("<p>{why}</p>"));
                }
            }
        }
    });
}

/// `?example=NAME`, else the last project opened, else the LED demo.
fn startup() -> Startup {
    if let Some(example) = query("example").and_then(|name| Example::parse(&name)) {
        return Startup::Example(example);
    }
    let recent = crate::settings::Settings::load().recent;
    match recent.into_iter().find(|p| model::io::is_project_dir(p)) {
        Some(dir) => Startup::Open(dir),
        None => Startup::Example(Example::Led),
    }
}

thread_local! {
    /// Whether the app has unsaved changes (asks before the page is left).
    static UNSAVED: Cell<bool> = const { Cell::new(false) };
    /// When the workspace was last saved (ms, `Date.now()`).
    static WORKSPACE_SAVED: Cell<f64> = const { Cell::new(0.0) };
}

/// After each frame: what the desktop does when the app quits, since a page just goes
/// away. Keeps the unsaved state for the warning, says when to save the workspace (about
/// once a second; it isn't written again unchanged) and stores the changed files.
pub fn after_frame(unsaved: bool, save_workspace: impl FnOnce()) {
    UNSAVED.with(|u| u.set(unsaved));
    let now = js_sys::Date::now();
    if now - WORKSPACE_SAVED.with(Cell::get) > 1000.0 {
        WORKSPACE_SAVED.with(|t| t.set(now));
        save_workspace();
    }
    crate::web_store::persist();
}

/// The browser asks before leaving a page with unsaved changes.
fn warn_before_leaving() {
    let Some(window) = web_sys::window() else { return };
    let listener = Closure::<dyn Fn(web_sys::BeforeUnloadEvent)>::new(|e: web_sys::BeforeUnloadEvent| {
        if UNSAVED.with(Cell::get) {
            e.prevent_default();
            e.set_return_value("unsaved");
        }
    });
    if window.add_event_listener_with_callback("beforeunload", listener.as_ref().unchecked_ref()).is_ok() {
        listener.forget();
    }
}

thread_local! {
    /// The device of the last wheel event (see `wheel.rs`).
    static WHEEL: Cell<egui::MouseWheelSource> = const { Cell::new(egui::MouseWheelSource::Unknown) };
}

/// The device of the last wheel event: eframe passes `Unknown`.
pub fn wheel_source() -> egui::MouseWheelSource {
    WHEEL.with(Cell::get)
}

/// Classifies wheel events before eframe's handler on the canvas sees them (the
/// window's capture phase comes first).
fn watch_wheel() {
    let Some(window) = web_sys::window() else { return };
    let listener = Closure::<dyn Fn(web_sys::WheelEvent)>::new(|e: web_sys::WheelEvent| {
        let legacy = js_sys::Reflect::get(&e, &"wheelDeltaY".into()).ok().and_then(|v| v.as_f64());
        let source = crate::wheel::classify(e.delta_mode(), e.delta_x(), e.delta_y(), legacy);
        WHEEL.with(|w| w.set(source));
    });
    let options = web_sys::AddEventListenerOptions::new();
    options.set_capture(true);
    options.set_passive(true);
    let added = window.add_event_listener_with_callback_and_add_event_listener_options(
        "wheel",
        listener.as_ref().unchecked_ref(),
        &options,
    );
    if added.is_ok() {
        listener.forget(); // for the page's lifetime
    }
}

/// Hands `data` to the browser as a download named `name`.
pub fn download(name: &str, data: &[u8]) {
    let done = (|| -> Result<(), wasm_bindgen::JsValue> {
        let parts = js_sys::Array::of1(&js_sys::Uint8Array::from(data));
        let blob = web_sys::Blob::new_with_u8_array_sequence(&parts)?;
        let url = web_sys::Url::create_object_url_with_blob(&blob)?;
        let document = web_sys::window().and_then(|w| w.document()).ok_or("no document")?;
        let a: web_sys::HtmlAnchorElement = document.create_element("a")?.dyn_into()?;
        a.set_href(&url);
        a.set_download(name);
        a.click();
        web_sys::Url::revoke_object_url(&url)
    })();
    if let Err(e) = done {
        log::error!("download of {name} failed: {e:?}");
    }
}

/// Whether the browser offers WebGPU (the only backend of the browser build).
fn has_webgpu() -> bool {
    web_sys::window().is_some_and(|w| js_sys::Reflect::has(&w.navigator(), &"gpu".into()).unwrap_or(false))
}

/// The page's title (eframe's web backend ignores `ViewportCommand::Title`).
pub fn set_title(title: &str) {
    if let Some(document) = web_sys::window().and_then(|w| w.document()) {
        document.set_title(title);
    }
}

/// A parameter of the page's URL query.
fn query(name: &str) -> Option<String> {
    let search = web_sys::window()?.location().search().ok()?;
    web_sys::UrlSearchParams::new_with_str(&search).ok()?.get(name)
}
