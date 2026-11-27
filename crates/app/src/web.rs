//! The browser entry: runs the app in the page's canvas (`index.html`, built with Trunk).
//!
//! It opens an example: `?example=amp` picks one, the LED demo otherwise.

use wasm_bindgen::JsCast;

use crate::app::{self, Options, Startup};
use view::demo::Example;

/// The canvas in `index.html`.
const CANVAS_ID: &str = "pcb_designer_canvas";

pub fn start() {
    eframe::WebLogger::init(log::LevelFilter::Info).ok();
    let example = query("example").and_then(|name| Example::parse(&name)).unwrap_or(Example::Led);
    let opts = Options::new(Startup::Example(example));
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
