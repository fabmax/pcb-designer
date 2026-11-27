//! The browser entry: runs the app in the page's canvas (`index.html`, built with Trunk).
//!
//! It opens an example: `?example=amp` picks one, the LED demo otherwise.

use std::cell::Cell;

use wasm_bindgen::JsCast;
use wasm_bindgen::closure::Closure;

use crate::app::{self, Options, Startup};
use view::demo::Example;

/// The canvas in `index.html`.
const CANVAS_ID: &str = "pcb_designer_canvas";

pub fn start() {
    eframe::WebLogger::init(log::LevelFilter::Info).ok();
    let example = query("example").and_then(|name| Example::parse(&name)).unwrap_or(Example::Led);
    let opts = Options::new(Startup::Example(example));
    watch_wheel();
    wasm_bindgen_futures::spawn_local(async move {
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
