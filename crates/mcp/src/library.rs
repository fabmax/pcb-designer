//! Library tools: agents create, open, read, edit, save and render libraries, e.g. to
//! turn a datasheet into a part.
//!
//! Edits take library objects in their file format (`lib_symbol`, `lib_footprint`,
//! `lib_part`) with conveniences for hand-written JSON:
//!
//! * objects are found by id or name; a found object is updated, otherwise added;
//! * fields left out keep their value (new objects get defaults);
//! * ids are optional: pins keep theirs when matched by number (or name), pads by name,
//!   part variants by name, so parts and projects using them stay valid;
//! * parts name their symbols and footprints, and connections name pads and pins
//!   (pad name ↔ pin number); without connections, pads map to pins by number.
//!
//! An edit is one undo step of the library and all-or-nothing: it is refused if it would
//! leave dangling references (a part using a deleted footprint, an unknown pad).
//!
//! Library files belong to the project (its External Libraries): opening or creating one
//! adds it (an undoable project edit), `remove_library` removes it. A library created
//! without a file stays out of the project until it is saved.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use editor_core::Command;
use model::*;
use render::offscreen::Offscreen;
use serde_json::{Map, Value, json};
use view::item::{Item, ItemView};
use view::library_doc::{blank_footprint, blank_symbol};
use view::project_libs::{self, MissingLibrary};
use view::{Document, LibraryDoc};

use crate::host::{Reply, render_png};

#[derive(Debug)]
pub enum LibraryRequest {
    List,
    Create { name: String, path: Option<PathBuf> },
    Open { path: PathBuf },
    Get { library: String, items: Vec<String> },
    Edit { library: String, upsert: Vec<Value>, delete: Vec<String> },
    Save { library: String, path: Option<PathBuf> },
    Remove { library: String, discard: bool },
    Render { library: String, item: String, width: u32, height: u32 },
}

impl LibraryRequest {
    pub fn describe(&self) -> String {
        match self {
            LibraryRequest::List => "list libraries".into(),
            LibraryRequest::Create { name, .. } => format!("create library {name}"),
            LibraryRequest::Open { path } => format!("open library {}", path.display()),
            LibraryRequest::Get { library, .. } => format!("read library {library}"),
            LibraryRequest::Edit { library, upsert, delete } => {
                format!("edit library {library} ({} item(s))", upsert.len() + delete.len())
            }
            LibraryRequest::Save { library, .. } => format!("save library {library}"),
            LibraryRequest::Remove { library, .. } => format!("remove library {library}"),
            LibraryRequest::Render { item, .. } => format!("render {item}"),
        }
    }

    pub fn modifies(&self) -> bool {
        matches!(
            self,
            LibraryRequest::Create { .. }
                | LibraryRequest::Open { .. }
                | LibraryRequest::Edit { .. }
                | LibraryRequest::Save { .. }
                | LibraryRequest::Remove { .. }
        )
    }
}

/// The host's loaded libraries: the built-in one, the project's library files (kept in
/// line with the project by [`project_libs::sync`]) and unsaved ones.
pub trait Libraries {
    fn count(&self) -> usize;
    fn get(&self, index: usize) -> &LibraryDoc;
    fn get_mut(&mut self, index: usize) -> &mut LibraryDoc;
    fn project(&self) -> &Document;
    fn project_mut(&mut self) -> &mut Document;
    /// Listed library files that could not be loaded.
    fn missing(&self) -> Vec<MissingLibrary>;
    /// Keep a library loaded (the app shows it in a tab). Returns its index.
    fn push(&mut self, lib: LibraryDoc) -> usize;
    /// Unload a library (the app closes its tab).
    fn remove(&mut self, index: usize);
    /// An agent changed a library; `show` is worth showing the user.
    fn changed(&mut self, _index: usize, _show: Option<ObjectId>) {}
    /// Offscreen renderer (created on first use).
    fn offscreen(&mut self) -> &mut Option<Offscreen>;
}

pub fn respond(request: LibraryRequest, libs: &mut dyn Libraries) -> Result<Reply, String> {
    let json = match request {
        LibraryRequest::List => {
            let listed = libs.project().project().libraries.clone();
            // The project's own copies first (edit them with `execute`).
            let project = libs.project();
            let mut all = vec![summary_of(&project.project().library, "project", None, project.is_modified())];
            all.extend((0..libs.count()).map(|i| summary(libs.get(i), &listed)));
            all.extend(
                libs.missing().into_iter().map(|m| json!({ "kind": "missing", "path": m.path, "error": m.error })),
            );
            json!({ "libraries": all })
        }
        LibraryRequest::Create { name, path } => {
            let name = name.trim();
            if name.is_empty() {
                return Err("the library needs a name".into());
            }
            if find(libs, name).is_ok() {
                return Err(format!("a library named {name:?} is already open"));
            }
            let mut lib = LibraryDoc::new(Library::new(name));
            // Without a path, next to the project (if it has a folder).
            let dir = libs.project().path.clone();
            let path = path.map(|p| resolve(dir.as_deref(), &with_extension(p)));
            let path = path.or_else(|| dir.map(|d| d.join(format!("{name}.pcblib"))));
            match path {
                Some(p) if p.exists() => {
                    return Err(format!("{} already exists: open it with open_library", p.display()));
                }
                Some(p) => {
                    lib.save_as(&p).map_err(|e| e.to_string())?;
                    add(libs, &lib)?;
                }
                None => lib.doc.mark_saved(),
            }
            let i = libs.push(lib);
            let listed = libs.project().project().libraries.clone();
            summary(libs.get(i), &listed)
        }
        LibraryRequest::Open { path } => {
            let dir = libs.project().path.clone();
            let path = resolve(dir.as_deref(), &path);
            let open = (0..libs.count()).find(|&i| libs.get(i).path.as_deref().is_some_and(|p| same_file(p, &path)));
            let i = match open {
                Some(i) => {
                    let listed = &libs.project().project().libraries;
                    if libs.get(i).path.as_ref().is_some_and(|p| !listed.contains(&project_libs::absolute(p))) {
                        // Dropped from the project while it had unsaved changes.
                        let lib = libs.get(i);
                        let path = project_libs::check_new(listed, &[], lib)?;
                        libs.project_mut().execute(Command::AddLibrary { path }).map_err(|e| e.to_string())?;
                    }
                    i
                }
                None => {
                    let lib = LibraryDoc::open(&path).map_err(|e| format!("cannot open {}: {e}", path.display()))?;
                    add(libs, &lib)?;
                    libs.push(lib)
                }
            };
            let listed = libs.project().project().libraries.clone();
            summary(libs.get(i), &listed)
        }
        LibraryRequest::Get { library, items } => {
            let i = find(libs, &library)?;
            let lib = libs.get(i).library();
            let mut out = Vec::new();
            for spec in &items {
                let ids = lookup(lib, spec);
                if ids.is_empty() {
                    return Err(format!("{spec:?} is not in library {:?}", lib.name));
                }
                out.extend(ids.into_iter().filter_map(|id| object(lib, id)));
            }
            json!({ "library": lib.name, "items": out })
        }
        LibraryRequest::Edit { library, upsert, delete } => {
            let i = find(libs, &library)?;
            let (reply, show) = edit(libs.get_mut(i), upsert, delete)?;
            libs.changed(i, show);
            reply
        }
        LibraryRequest::Save { library, path } => {
            let i = find(libs, &library)?;
            if libs.get(i).is_builtin() {
                return Err("the built-in library can't be changed; create a library of your own".into());
            }
            let dir = libs.project().path.clone();
            let listed = libs.project().project().libraries.clone();
            let doc = libs.get_mut(i);
            let old = doc.path.clone();
            match path.map(|p| resolve(dir.as_deref(), &with_extension(p))) {
                Some(p) => doc.save_as(&p),
                None if doc.path.is_some() => doc.save(),
                None => return Err("the library has never been saved: pass a path (….pcblib)".into()),
            }
            .map_err(|e| e.to_string())?;
            let (name, saved) = (doc.name().to_string(), doc.path.clone());
            if let Some(new) = &saved
                && let Some(command) = project_libs::after_save(&listed, old.as_deref(), new)
            {
                libs.project_mut().execute(command).map_err(|e| e.to_string())?;
            }
            libs.changed(i, None);
            json!({ "library": name, "saved": saved })
        }
        LibraryRequest::Remove { library, discard } => {
            let i = find(libs, &library)?;
            let doc = libs.get(i);
            if doc.is_builtin() {
                return Err("the built-in library is always there".into());
            }
            if doc.is_modified() && !discard {
                return Err(format!(
                    "library {:?} has unsaved changes: save_library first, or pass discard: true",
                    doc.name()
                ));
            }
            let (name, path) = (doc.name().to_string(), doc.path.as_deref().map(project_libs::absolute));
            if let Some(path) = path.filter(|p| libs.project().project().libraries.contains(p)) {
                libs.project_mut().execute(Command::RemoveLibrary { path }).map_err(|e| e.to_string())?;
            }
            libs.remove(i);
            json!({ "removed": name })
        }
        LibraryRequest::Render { library, item, width, height } => {
            let i = find(libs, &library)?;
            let doc = libs.get_mut(i);
            let item = render_target(doc.library(), &item)?;
            let mut view = ItemView::new();
            view.show(Some(item));
            view.sync(&doc.doc);
            let grid = match item {
                Item::Symbol(_) => 1.27,
                Item::Footprint(_) => 0.25,
            };
            let (w, h) = (width.clamp(64, 4096), height.clamp(64, 4096));
            let background = view::style::background(grid);
            return render_png(libs.offscreen(), &view.scene, None, &background, w, h).map(Reply::Png);
        }
    };
    Ok(Reply::Json(json))
}

fn with_extension(p: PathBuf) -> PathBuf {
    if p.extension().is_some_and(|e| e == "pcblib") { p } else { p.with_extension("pcblib") }
}

/// Check `lib` and add it to the project's library files.
fn add(libs: &mut dyn Libraries, lib: &LibraryDoc) -> Result<(), String> {
    let loaded: Vec<&LibraryDoc> = (0..libs.count()).map(|i| libs.get(i)).collect();
    let path = project_libs::check_new(&libs.project().project().libraries, &loaded, lib)?;
    libs.project_mut().execute(Command::AddLibrary { path }).map(drop).map_err(|e| e.to_string())
}

/// A path an agent gave: relative ones are relative to the project folder (else the
/// working directory). Absolute and normalized.
fn resolve(dir: Option<&Path>, path: &Path) -> PathBuf {
    match dir {
        Some(d) if path.is_relative() => project_libs::absolute(&d.join(path)),
        _ => project_libs::absolute(path),
    }
}

fn same_file(a: &Path, b: &Path) -> bool {
    a == b || matches!((a.canonicalize(), b.canonicalize()), (Ok(x), Ok(y)) if x == y)
}

/// Index of the open library named `name` (or with that file path).
fn find(libs: &dyn Libraries, name: &str) -> Result<usize, String> {
    let n = libs.count();
    let names: Vec<String> = (0..n).map(|i| libs.get(i).name().to_string()).collect();
    if let Some(i) = names.iter().position(|l| l == name) {
        return Ok(i);
    }
    if let Some(i) = names.iter().position(|l| l.eq_ignore_ascii_case(name)) {
        return Ok(i);
    }
    if let Some(i) = (0..n).find(|&i| libs.get(i).path.as_deref().is_some_and(|p| same_file(p, Path::new(name)))) {
        return Ok(i);
    }
    Err(format!("no open library {name:?} (open: {})", names.join(", ")))
}

/// `kind`: `built_in`, `external` (one of the project's library files), `unsaved` (no
/// file yet, not in the project) or `unlisted` (removed from the project with unsaved
/// changes).
fn summary(doc: &LibraryDoc, listed: &[PathBuf]) -> Value {
    let kind = match &doc.path {
        None if doc.is_builtin() => "built_in",
        None => "unsaved",
        Some(p) if listed.contains(&project_libs::absolute(p)) => "external",
        Some(_) => "unlisted",
    };
    summary_of(doc.library(), kind, doc.path.as_deref(), doc.is_modified())
}

fn summary_of(lib: &Library, kind: &str, path: Option<&Path>, modified: bool) -> Value {
    let symbol_name = |id: Id| lib.symbols.get(id).map(|s| s.name.clone());
    let footprint_name = |id: Id| lib.footprints.get(id).map(|f| f.name.clone());
    let symbols: Vec<Value> =
        lib.symbols.iter().map(|s| json!({ "id": s.id, "name": s.name, "pins": s.pins.len() })).collect();
    let footprints: Vec<Value> =
        lib.footprints.iter().map(|f| json!({ "id": f.id, "name": f.name, "pads": f.pads.len() })).collect();
    let parts: Vec<Value> = lib
        .parts
        .iter()
        .map(|p| {
            let units: Vec<Value> =
                p.units.iter().map(|u| json!({ "name": u.name, "symbol": symbol_name(u.symbol) })).collect();
            let variants: Vec<Value> = p
                .variants
                .iter()
                .map(|v| json!({ "name": v.name, "footprint": v.footprint.map(footprint_name) }))
                .collect();
            json!({ "id": p.id, "name": p.name, "description": p.description, "units": units, "variants": variants })
        })
        .collect();
    json!({
        "name": lib.name,
        "kind": kind,
        "path": path,
        "modified": modified,
        "symbols": symbols,
        "footprints": footprints,
        "parts": parts,
    })
}

/// A library object in full.
fn object(lib: &Library, id: ObjectId) -> Option<Value> {
    let o = match id {
        ObjectId::LibSymbol(i) => Object::LibSymbol(lib.symbols.get(i)?.clone()),
        ObjectId::LibFootprint(i) => Object::LibFootprint(lib.footprints.get(i)?.clone()),
        ObjectId::LibPart(i) => Object::LibPart(lib.parts.get(i)?.clone()),
        _ => return None,
    };
    serde_json::to_value(o).ok()
}

/// Items `spec` names: `[part:|symbol:|footprint:]NAME`, or an id.
fn lookup(lib: &Library, spec: &str) -> Vec<ObjectId> {
    let (kind, name) = match spec.split_once(':') {
        Some((k @ ("part" | "symbol" | "footprint"), n)) => (Some(k), n.trim()),
        _ => (None, spec.trim()),
    };
    let id: Option<Id> = name.parse().ok();
    let hit = |n: &str, i: Id| Some(i) == id || n == name;
    let mut out = Vec::new();
    if matches!(kind, None | Some("part")) {
        out.extend(lib.parts.iter().filter(|p| hit(&p.name, p.id)).map(|p| ObjectId::LibPart(p.id)));
    }
    if matches!(kind, None | Some("symbol")) {
        out.extend(lib.symbols.iter().filter(|s| hit(&s.name, s.id)).map(|s| ObjectId::LibSymbol(s.id)));
    }
    if matches!(kind, None | Some("footprint")) {
        out.extend(lib.footprints.iter().filter(|f| hit(&f.name, f.id)).map(|f| ObjectId::LibFootprint(f.id)));
    }
    out
}

/// The symbol or footprint to render (symbols win over footprints of the same name; a
/// part shows its first symbol).
fn render_target(lib: &Library, spec: &str) -> Result<Item, String> {
    let ids = lookup(lib, spec);
    let symbol = ids.iter().find_map(|id| match id {
        ObjectId::LibSymbol(i) => Some(Item::Symbol(*i)),
        _ => None,
    });
    let footprint = ids.iter().find_map(|id| match id {
        ObjectId::LibFootprint(i) => Some(Item::Footprint(*i)),
        _ => None,
    });
    let part_symbol = ids.iter().find_map(|id| match id {
        ObjectId::LibPart(i) => lib.parts.get(*i)?.units.first().map(|u| Item::Symbol(u.symbol)),
        _ => None,
    });
    symbol
        .or(footprint)
        .or(part_symbol)
        .ok_or_else(|| format!("no symbol or footprint {spec:?} in library {:?}", lib.name))
}

fn kind_name(id: ObjectId) -> &'static str {
    match id {
        ObjectId::LibSymbol(_) => "symbol",
        ObjectId::LibFootprint(_) => "footprint",
        _ => "part",
    }
}

fn name_of(lib: &Library, id: ObjectId) -> String {
    let name = match id {
        ObjectId::LibSymbol(i) => lib.symbols.get(i).map(|s| s.name.clone()),
        ObjectId::LibFootprint(i) => lib.footprints.get(i).map(|f| f.name.clone()),
        ObjectId::LibPart(i) => lib.parts.get(i).map(|p| p.name.clone()),
        _ => None,
    };
    format!("{}:{}", kind_name(id), name.unwrap_or_default())
}

fn contains(lib: &Library, id: ObjectId) -> bool {
    match id {
        ObjectId::LibSymbol(i) => lib.symbols.contains(i),
        ObjectId::LibFootprint(i) => lib.footprints.contains(i),
        ObjectId::LibPart(i) => lib.parts.contains(i),
        _ => false,
    }
}

fn put(lib: &mut Library, o: Object) {
    match o {
        Object::LibSymbol(s) => {
            lib.symbols.insert(s);
        }
        Object::LibFootprint(f) => {
            lib.footprints.insert(f);
        }
        Object::LibPart(p) => {
            lib.parts.insert(p);
        }
        _ => {}
    }
}

fn remove(lib: &mut Library, id: ObjectId) {
    match id {
        ObjectId::LibSymbol(i) => {
            lib.symbols.remove(i);
        }
        ObjectId::LibFootprint(i) => {
            lib.footprints.remove(i);
        }
        ObjectId::LibPart(i) => {
            lib.parts.remove(i);
        }
        _ => {}
    }
}

/// Apply an edit as one undo step. Returns the reply and an item to show.
fn edit(
    doc: &mut LibraryDoc,
    mut upsert: Vec<Value>,
    delete: Vec<String>,
) -> Result<(Value, Option<ObjectId>), String> {
    if upsert.is_empty() && delete.is_empty() {
        return Err("nothing to do: pass upsert and/or delete".into());
    }
    let before = doc.library().clone();
    let mut work = before.clone();
    // Symbols and footprints first: parts refer to them by name.
    upsert.sort_by_key(|v| v.get("type").and_then(Value::as_str) == Some("lib_part"));
    let mut changed: Vec<Object> = Vec::new();
    for v in upsert {
        let Value::Object(map) = v else {
            return Err("upsert items are objects with a \"type\" (lib_symbol, lib_footprint or lib_part)".into());
        };
        let object = match map.get("type").and_then(Value::as_str).unwrap_or("") {
            "lib_symbol" => Object::LibSymbol(symbol(&work, map)?),
            "lib_footprint" => Object::LibFootprint(footprint(&work, map)?),
            "lib_part" => Object::LibPart(part(&work, map)?),
            other => return Err(format!("unknown type {other:?}: use lib_symbol, lib_footprint or lib_part")),
        };
        changed.retain(|o| o.id() != object.id());
        put(&mut work, object.clone());
        changed.push(object);
    }
    let mut deleted: Vec<ObjectId> = Vec::new();
    for spec in &delete {
        let ids = lookup(&work, spec);
        let id = match ids[..] {
            [id] => id,
            [] => return Err(format!("delete: {spec:?} is not in the library")),
            _ => {
                return Err(format!(
                    "delete: {spec:?} names several items; prefix it with part:, symbol: or footprint:"
                ));
            }
        };
        remove(&mut work, id);
        deleted.push(id);
    }
    changed.retain(|o| !deleted.contains(&o.id()));

    // Refuse what would break the library.
    let old: HashSet<String> = before.issues().into_iter().map(|(_, m)| m).collect();
    let new: Vec<String> = work.issues().into_iter().map(|(_, m)| m).filter(|m| !old.contains(m)).collect();
    if !new.is_empty() {
        return Err(format!("nothing was changed: {}", new.join("; ")));
    }

    // Commands in dependency order: symbols and footprints, then parts, then deletions
    // (parts before what they use).
    let is_part = |o: &Object| matches!(o, Object::LibPart(_));
    let mut commands = Vec::new();
    for parts in [false, true] {
        let (new, updated): (Vec<Object>, Vec<Object>) =
            changed.iter().filter(|o| is_part(o) == parts).cloned().partition(|o| !contains(&before, o.id()));
        if !new.is_empty() {
            commands.push(Command::Insert { objects: new });
        }
        if !updated.is_empty() {
            commands.push(Command::Update { objects: updated });
        }
    }
    deleted.sort_by_key(|id| !matches!(id, ObjectId::LibPart(_)));
    if !deleted.is_empty() {
        commands.push(Command::Delete { ids: deleted.clone() });
    }
    doc.execute_all(commands)?;

    let lib = doc.library();
    let names = |added: bool| -> Vec<String> {
        changed.iter().filter(|o| contains(&before, o.id()) != added).map(|o| name_of(lib, o.id())).collect()
    };
    let parts: Vec<Value> = changed
        .iter()
        .filter_map(|o| match o {
            Object::LibPart(p) => lib.parts.get(p.id),
            _ => None,
        })
        .map(|p| part_report(lib, p))
        .collect();
    let show = changed.iter().find(|o| is_part(o)).or(changed.first()).map(Object::id);
    let reply = json!({
        "library": lib.name,
        "added": names(true),
        "updated": names(false),
        "deleted": deleted.iter().map(|&id| name_of(&before, id)).collect::<Vec<_>>(),
        "parts": parts,
        "saved": false,
        "next": "Check symbols and footprints with render_library_item, then save_library.",
    });
    Ok((reply, show))
}

/// How a part's variants map pads to pins.
fn part_report(lib: &Library, part: &Part) -> Value {
    let variants: Vec<Value> = part
        .variants
        .iter()
        .map(|v| {
            let Some(fp) = v.footprint.and_then(|f| lib.footprints.get(f)) else {
                return json!({ "name": v.name, "footprint": null });
            };
            let mapped: HashSet<Id> = v.connections.iter().map(|c| c.pad).collect();
            let unconnected: Vec<&str> =
                fp.pads.iter().filter(|p| !mapped.contains(&p.id)).map(|p| p.name.as_str()).collect();
            let used: HashSet<(u32, Id)> = v.connections.iter().map(|c| (c.unit, c.pin)).collect();
            let mut pins_without_pad = Vec::new();
            for (u, unit) in part.units.iter().enumerate() {
                for pin in lib.symbols.get(unit.symbol).into_iter().flat_map(|s| s.pins.iter()) {
                    if !used.contains(&(u as u32, pin.id)) {
                        pins_without_pad.push(format!("{} {}", pin.number, pin.name).trim().to_string());
                    }
                }
            }
            json!({
                "name": v.name,
                "footprint": fp.name,
                "connected_pads": format!("{} of {}", mapped.len(), fp.pads.len()),
                "unconnected_pads": unconnected,
                "pins_without_pad": pins_without_pad,
            })
        })
        .collect();
    json!({ "id": part.id, "name": part.name, "variants": variants })
}

// ---------------------------------------------------------------------------------------
// Resolving hand-written objects

fn str_field<'a>(v: &'a Value, key: &str) -> Option<&'a str> {
    v.get(key).and_then(Value::as_str)
}

fn parse_id(v: &Value) -> Option<Id> {
    v.as_str().and_then(|s| s.parse().ok())
}

/// The object `map` refers to (by "id", else by "name") among `items`, as (id, name).
/// Errors if a new object would take another's name.
fn existing(items: &[(Id, &str)], map: &Map<String, Value>, kind: &str) -> Result<Option<Id>, String> {
    let name = map.get("name").and_then(Value::as_str);
    let by_name = |n: &str| items.iter().find(|(_, m)| *m == n).map(|(id, _)| *id);
    match map.get("id") {
        Some(v) => {
            let id = parse_id(v).ok_or_else(|| format!("{kind}: \"id\" is not a valid id"))?;
            let found = items.iter().any(|(i, _)| *i == id);
            if let Some(other) = name.and_then(by_name).filter(|&o| o != id) {
                return Err(format!("{kind} {:?}: the name is taken by {other}", name.unwrap_or_default()));
            }
            Ok(found.then_some(id))
        }
        None => match name {
            Some(n) => Ok(by_name(n)),
            None => Err(format!("{kind}: give a \"name\" (or the \"id\" of an existing {kind})")),
        },
    }
}

/// Start from `base` (the existing object or the defaults) and overlay the given fields.
fn overlay(base: Value, map: Map<String, Value>) -> Map<String, Value> {
    let Value::Object(mut base) = base else { unreachable!("objects serialize to maps") };
    for (k, v) in map {
        if k != "type" {
            base.insert(k, v);
        }
    }
    base
}

/// Give list items without an id the id of the old item they match (`key` of the new item
/// equals `key` of an old one), else a new id.
fn fill_ids(items: Option<&mut Value>, old: &[(String, Id)], key: impl Fn(&Value) -> String) {
    let Some(Value::Array(items)) = items else { return };
    let mut used: HashSet<Id> = items.iter().filter_map(|i| i.get("id").and_then(parse_id)).collect();
    for item in items {
        let Value::Object(m) = item else { continue };
        if m.contains_key("id") {
            continue;
        }
        let k = key(&Value::Object(m.clone()));
        let id = old.iter().find(|(n, id)| !k.is_empty() && *n == k && !used.contains(id)).map(|(_, id)| *id);
        let id = id.unwrap_or_else(Id::new);
        used.insert(id);
        m.insert("id".into(), json!(id));
    }
}

/// Default stroke width of texts: a seventh of the size.
fn fill_text_width(text: &mut Value) {
    if let Value::Object(t) = text
        && !t.contains_key("width")
        && let Some(size) = t.get("size").and_then(Value::as_i64)
    {
        t.insert("width".into(), json!(size / 7));
    }
}

fn parse<T: serde::de::DeserializeOwned>(map: Map<String, Value>, what: &str) -> Result<T, String> {
    serde_json::from_value(Value::Object(map)).map_err(|e| format!("{what}: {e}"))
}

fn symbol(work: &Library, map: Map<String, Value>) -> Result<Symbol, String> {
    let found =
        existing(&work.symbols.iter().map(|s| (s.id, s.name.as_str())).collect::<Vec<_>>(), &map, "lib_symbol")?;
    let old = found.and_then(|id| work.symbols.get(id));
    let base = match old {
        Some(s) => serde_json::to_value(s),
        None => serde_json::to_value(blank_symbol(String::new())),
    }
    .map_err(|e| e.to_string())?;
    let mut m = overlay(base, map);
    // Pins: keep ids of pins with the same number (or name), default length 2.54 mm.
    let pin_key = |p: &Value| match str_field(p, "number") {
        Some(n) if !n.is_empty() => format!("#{n}"),
        _ => str_field(p, "name").unwrap_or_default().to_string(),
    };
    let old_pins: Vec<(String, Id)> = old
        .map(|s| {
            s.pins
                .iter()
                .map(|p| (if p.number.is_empty() { p.name.clone() } else { format!("#{}", p.number) }, p.id))
                .collect()
        })
        .unwrap_or_default();
    fill_ids(m.get_mut("pins"), &old_pins, pin_key);
    if let Some(Value::Array(pins)) = m.get_mut("pins") {
        for p in pins.iter_mut().filter_map(Value::as_object_mut) {
            p.entry("length").or_insert(json!(mm(2.54)));
        }
    }
    if let Some(Value::Array(texts)) = m.get_mut("texts") {
        texts.iter_mut().for_each(fill_text_width);
    }
    let what = format!("lib_symbol {:?}", m.get("name").and_then(Value::as_str).unwrap_or_default());
    m.entry("id").or_insert(json!(Id::new()));
    let s: Symbol = parse(m, &what)?;
    if s.name.trim().is_empty() {
        return Err("lib_symbol: the name is empty".into());
    }
    Ok(s)
}

fn footprint(work: &Library, map: Map<String, Value>) -> Result<Footprint, String> {
    let found =
        existing(&work.footprints.iter().map(|f| (f.id, f.name.as_str())).collect::<Vec<_>>(), &map, "lib_footprint")?;
    let old = found.and_then(|id| work.footprints.get(id));
    let base = match old {
        Some(f) => serde_json::to_value(f),
        None => serde_json::to_value(blank_footprint(String::new())),
    }
    .map_err(|e| e.to_string())?;
    let mut m = overlay(base, map);
    let old_pads: Vec<(String, Id)> =
        old.map(|f| f.pads.iter().map(|p| (p.name.clone(), p.id)).collect()).unwrap_or_default();
    fill_ids(m.get_mut("pads"), &old_pads, |p| str_field(p, "name").unwrap_or_default().to_string());
    if let Some(Value::Array(texts)) = m.get_mut("texts") {
        texts.iter_mut().filter_map(|t| t.get_mut("text")).for_each(fill_text_width);
    }
    let what = format!("lib_footprint {:?}", m.get("name").and_then(Value::as_str).unwrap_or_default());
    m.entry("id").or_insert(json!(Id::new()));
    let f: Footprint = parse(m, &what)?;
    if f.name.trim().is_empty() {
        return Err("lib_footprint: the name is empty".into());
    }
    Ok(f)
}

fn part(work: &Library, mut map: Map<String, Value>) -> Result<Part, String> {
    let found = existing(&work.parts.iter().map(|p| (p.id, p.name.as_str())).collect::<Vec<_>>(), &map, "lib_part")?;
    let old = found.and_then(|id| work.parts.get(id));
    let units_given = map.remove("units");
    let variants_given = map.remove("variants");
    let base = match old {
        Some(p) => p.clone(),
        None => Part {
            id: Id::new(),
            name: String::new(),
            description: String::new(),
            prefix: "U".into(),
            default_value: String::new(),
            units: Vec::new(),
            variants: Vec::new(),
            properties: Default::default(),
            source: None,
        },
    };
    let mut m = overlay(serde_json::to_value(&base).map_err(|e| e.to_string())?, map);
    let name = m.get("name").and_then(Value::as_str).unwrap_or_default().trim().to_string();
    if name.is_empty() {
        return Err("lib_part: the name is empty".into());
    }
    let what = format!("lib_part {name:?}");
    if old.is_none() && m.get("default_value").and_then(Value::as_str).is_none_or(str::is_empty) {
        m.insert("default_value".into(), json!(name));
    }
    let units = match units_given {
        Some(Value::Array(us)) => {
            us.iter().enumerate().map(|(i, u)| unit(work, i, u, &what)).collect::<Result<_, _>>()?
        }
        None => base.units.clone(),
        Some(_) => return Err(format!("{what}: units is a list like [{{\"symbol\": \"NAME\"}}]")),
    };
    if units.is_empty() {
        return Err(format!("{what} needs a symbol: units: [{{\"symbol\": \"SYMBOL NAME\"}}]"));
    }
    m.insert("units".into(), serde_json::to_value(&units).map_err(|e| e.to_string())?);
    m.insert("variants".into(), json!([]));
    let mut part: Part = parse(m, &what)?;
    part.variants = match variants_given {
        Some(Value::Array(vs)) => vs.iter().map(|v| variant(work, &part, old, v, &what)).collect::<Result<_, _>>()?,
        None => base.variants.clone(),
        Some(_) => return Err(format!("{what}: variants is a list like [{{\"footprint\": \"NAME\"}}]")),
    };
    if part.variants.is_empty() {
        return Err(format!(
            "{what} needs a variant: variants: [{{\"footprint\": \"FOOTPRINT NAME\"}}] \
             (or [{{\"name\": \"No footprint\"}}] for a part that isn't on the board)"
        ));
    }
    Ok(part)
}

/// Unit `i`: `{"symbol": NAME or id, "name"?}` (names default to A, B, …).
fn unit(work: &Library, i: usize, u: &Value, what: &str) -> Result<PartUnit, String> {
    let spec = str_field(u, "symbol").ok_or_else(|| format!("{what}: each unit needs a \"symbol\" (name or id)"))?;
    let id: Option<Id> = spec.parse().ok();
    let symbol = work
        .symbols
        .iter()
        .find(|s| s.name == spec || Some(s.id) == id)
        .ok_or_else(|| format!("{what}: no symbol {spec:?} in the library (add it in the same edit or before)"))?;
    let name = str_field(u, "name").map(str::to_string).unwrap_or_else(|| unit_name(i));
    Ok(PartUnit { name, symbol: symbol.id })
}

fn unit_name(i: usize) -> String {
    let mut name = String::new();
    let mut n = i;
    loop {
        name.insert(0, (b'A' + (n % 26) as u8) as char);
        if n < 26 {
            return name;
        }
        n = n / 26 - 1;
    }
}

/// Variant: `{"footprint": NAME or id, "name"?, "connections"?}`; without a footprint a
/// "no footprint" variant (`{"name"?}`, components off the board).
fn variant(work: &Library, part: &Part, old: Option<&Part>, v: &Value, what: &str) -> Result<PartVariant, String> {
    let variant_id = |name: &str| {
        v.get("id")
            .and_then(parse_id)
            .or_else(|| old.and_then(|p| p.variants.iter().find(|x| x.name == name).map(|x| x.id)))
            .unwrap_or_else(Id::new)
    };
    let Some(spec) = str_field(v, "footprint") else {
        if v.get("connections").is_some_and(|c| !c.is_null()) {
            return Err(format!("{what}: a variant without \"footprint\" has no connections"));
        }
        let name = str_field(v, "name").unwrap_or(PartVariant::NO_FOOTPRINT).to_string();
        return Ok(PartVariant { id: variant_id(&name), name, footprint: None, connections: Vec::new() });
    };
    let fp_id: Option<Id> = spec.parse().ok();
    let fp =
        work.footprints.iter().find(|f| f.name == spec || Some(f.id) == fp_id).ok_or_else(|| {
            format!("{what}: no footprint {spec:?} in the library (add it in the same edit or before)")
        })?;
    let name = str_field(v, "name").map(str::to_string).unwrap_or_else(|| fp.name.clone());
    let id = variant_id(&name);
    let what = format!("{what} variant {name:?}");
    let connections = match v.get("connections") {
        None | Some(Value::Null) => work.map_by_name(&part.units, fp.id),
        Some(Value::String(s)) if s == "by_number" || s == "by_name" => work.map_by_name(&part.units, fp.id),
        Some(Value::String(s)) if s == "in_order" => work.map_in_order(&part.units, fp.id),
        Some(Value::Array(cs)) => cs.iter().map(|c| connection(work, part, fp, c, &what)).collect::<Result<_, _>>()?,
        Some(_) => {
            return Err(format!(
                "{what}: connections is \"by_number\" (default), \"in_order\" or a list of {{\"pad\", \"pin\"}}"
            ));
        }
    };
    Ok(PartVariant { id, name, footprint: Some(fp.id), connections })
}

/// `{"pad": PAD NAME or id, "pin": PIN NUMBER, NAME or id, "unit"?: NAME or index}`.
fn connection(work: &Library, part: &Part, fp: &Footprint, c: &Value, what: &str) -> Result<PadConnection, String> {
    let as_text = |v: Option<&Value>| match v {
        Some(Value::String(s)) => Some(s.clone()),
        Some(Value::Number(n)) => Some(n.to_string()),
        _ => None,
    };
    let pad_spec = as_text(c.get("pad")).ok_or_else(|| format!("{what}: each connection needs a \"pad\""))?;
    let pin_spec = as_text(c.get("pin")).ok_or_else(|| format!("{what}: each connection needs a \"pin\""))?;
    let pad_id: Option<Id> = pad_spec.parse().ok();
    let pad = fp
        .pads
        .iter()
        .find(|p| p.name == pad_spec || Some(p.id) == pad_id)
        .ok_or_else(|| format!("{what}: footprint {:?} has no pad {pad_spec:?}", fp.name))?;
    let only_unit: Option<usize> = match c.get("unit") {
        None | Some(Value::Null) => None,
        Some(Value::Number(n)) => n.as_u64().map(|n| n as usize),
        Some(Value::String(s)) => part.units.iter().position(|u| u.name == *s),
        Some(_) => None,
    };
    let pin_id: Option<Id> = pin_spec.parse().ok();
    let units = part.units.iter().enumerate().filter(|(u, _)| only_unit.is_none_or(|o| o == *u));
    let symbols: Vec<(usize, &Symbol)> =
        units.filter_map(|(u, unit)| Some((u, work.symbols.get(unit.symbol)?))).collect();
    let find =
        |m: &dyn Fn(&Pin) -> bool| symbols.iter().find_map(|(u, s)| s.pins.iter().find(|p| m(p)).map(|p| (*u, p.id)));
    let (unit, pin) = find(&|p| Some(p.id) == pin_id)
        .or_else(|| find(&|p| !p.number.is_empty() && p.number == pin_spec))
        .or_else(|| find(&|p| p.name == pin_spec))
        .ok_or_else(|| format!("{what}: no pin {pin_spec:?} (number or name) in the part's symbols"))?;
    Ok(PadConnection { pad: pad.id, unit: unit as u32, pin })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unit_names() {
        assert_eq!([0, 1, 25, 26, 27].map(unit_name), ["A", "B", "Z", "AA", "AB"]);
    }

    #[test]
    fn lookup_by_kind_name_and_id() {
        let lib = builtin::builtin_library();
        let opamp = lookup(&lib, "OpAmp");
        assert_eq!(opamp.len(), 2, "part and symbol share the name");
        assert!(matches!(lookup(&lib, "part:OpAmp")[..], [ObjectId::LibPart(_)]));
        assert!(matches!(lookup(&lib, "footprint:SOIC-8")[..], [ObjectId::LibFootprint(_)]));
        let id = builtin::PART_OPAMP.to_string();
        assert_eq!(lookup(&lib, &id), vec![ObjectId::LibPart(builtin::PART_OPAMP)]);
        assert!(lookup(&lib, "symbol:SOIC-8").is_empty());
    }
}
