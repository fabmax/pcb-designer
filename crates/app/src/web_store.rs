//! The browser's file system (`model::fs`) kept in IndexedDB: loaded before the app
//! starts, its changes written after each frame.
//!
//! One object store, `files`: the key is the path, the value a `Uint8Array` of a kind
//! byte (0 folder, 1 file), the time written (8 bytes, little-endian milliseconds) and
//! the content.

use std::cell::RefCell;

use js_sys::{Array, Promise, Uint8Array};
use model::fs::Entry;
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::JsFuture;
use web_sys::{IdbDatabase, IdbRequest, IdbTransactionMode};

const DB: &str = "pcb-designer";
const STORE: &str = "files";

thread_local! {
    static DATABASE: RefCell<Option<IdbDatabase>> = const { RefCell::new(None) };
}

/// The request's result once it succeeds.
async fn finished(request: &IdbRequest) -> Result<JsValue, JsValue> {
    let promise = Promise::new(&mut |resolve, reject| {
        let r = request.clone();
        let ok = wasm_bindgen::closure::Closure::once_into_js(move |_: JsValue| {
            resolve.call1(&JsValue::NULL, &r.result().unwrap_or(JsValue::UNDEFINED)).ok();
        });
        let r = request.clone();
        let failed = wasm_bindgen::closure::Closure::once_into_js(move |_: JsValue| {
            let error = r.error().ok().flatten().map_or(JsValue::from_str("request failed"), JsValue::from);
            reject.call1(&JsValue::NULL, &error).ok();
        });
        request.set_onsuccess(Some(ok.unchecked_ref()));
        request.set_onerror(Some(failed.unchecked_ref()));
    });
    JsFuture::from(promise).await
}

/// Opens the database and loads every saved file into `model::fs`.
pub async fn load() -> Result<usize, JsValue> {
    let factory = web_sys::window().and_then(|w| w.indexed_db().ok().flatten()).ok_or("no IndexedDB")?;
    let open = factory.open_with_u32(DB, 1)?;
    let o = open.clone();
    let upgrade = wasm_bindgen::closure::Closure::once_into_js(move |_: JsValue| {
        if let Ok(db) = o.result().and_then(|r| r.dyn_into::<IdbDatabase>())
            && !db.object_store_names().contains(STORE)
            && let Err(e) = db.create_object_store(STORE)
        {
            log::error!("IndexedDB store not made: {e:?}");
        }
    });
    open.set_onupgradeneeded(Some(upgrade.unchecked_ref()));
    let db: IdbDatabase = finished(&open).await?.dyn_into()?;

    let tx = db.transaction_with_str(STORE)?;
    let store = tx.object_store(STORE)?;
    let keys: Array = finished(&store.get_all_keys()?).await?.dyn_into()?;
    let values: Array = finished(&store.get_all()?).await?.dyn_into()?;
    let mut count = 0;
    model::fs::with_mem(|fs| {
        for (key, value) in keys.iter().zip(values.iter()) {
            let (Some(path), Ok(bytes)) = (key.as_string(), value.dyn_into::<Uint8Array>()) else { continue };
            if let Some(entry) = decode(&bytes.to_vec()) {
                fs.restore(path.as_ref(), entry);
                count += 1;
            }
        }
    });
    DATABASE.with(|d| *d.borrow_mut() = Some(db));
    Ok(count)
}

/// Writes what changed in `model::fs` since the last call (transactions run in order).
pub fn persist() {
    let changes = model::fs::with_mem(|fs| fs.take_changes());
    if changes.is_empty() {
        return;
    }
    DATABASE.with(|d| {
        let Some(db) = &*d.borrow() else { return };
        let written = (|| -> Result<(), JsValue> {
            let tx = db.transaction_with_str_and_mode(STORE, IdbTransactionMode::Readwrite)?;
            let store = tx.object_store(STORE)?;
            for (path, entry) in &changes {
                let key = JsValue::from_str(&path.to_string_lossy());
                match entry {
                    Some(e) => store.put_with_key(&Uint8Array::from(encode(e).as_slice()), &key)?,
                    None => store.delete(&key)?,
                };
            }
            Ok(())
        })();
        if let Err(e) = written {
            log::error!("browser storage not written: {e:?}");
        }
    });
}

fn encode(entry: &Entry) -> Vec<u8> {
    match entry {
        Entry::Dir => vec![0],
        Entry::File(data, modified) => {
            let mut out = Vec::with_capacity(9 + data.len());
            out.push(1);
            out.extend(modified.to_le_bytes());
            out.extend(data);
            out
        }
    }
}

fn decode(bytes: &[u8]) -> Option<Entry> {
    match bytes.split_first()? {
        (0, _) => Some(Entry::Dir),
        (1, rest) if rest.len() >= 8 => {
            let (time, data) = rest.split_at(8);
            Some(Entry::File(data.to_vec(), u64::from_le_bytes(time.try_into().ok()?)))
        }
        _ => None,
    }
}
