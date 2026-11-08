//! A library file opened for editing.
//!
//! Internally the library lives inside an otherwise empty [`Project`], so library edits are
//! ordinary editor commands (`insert` / `update` / `delete` of `lib_symbol`, `lib_footprint`
//! and `lib_part` objects) with the same undo, grouping and validation as project edits.

use std::path::{Path, PathBuf};

use editor_core::Command;
use model::io::{FileError, load_library, save_library};
use model::*;

use crate::document::Document;
use crate::interaction::new_group;

pub struct LibraryDoc {
    pub doc: Document,
    /// Library file; `None` until saved (e.g. the built-in starter library).
    pub path: Option<PathBuf>,
}

impl LibraryDoc {
    pub fn new(library: Library) -> Self {
        let mut project = Project::new(library.name.clone());
        project.library = library;
        LibraryDoc { doc: Document::new(project), path: None }
    }

    pub fn open(path: impl AsRef<Path>) -> Result<Self, FileError> {
        let mut lib = LibraryDoc::new(load_library(path.as_ref())?);
        lib.path = Some(path.as_ref().to_path_buf());
        Ok(lib)
    }

    pub fn library(&self) -> &Library {
        &self.doc.project().library
    }

    pub fn name(&self) -> &str {
        &self.library().name
    }

    pub fn is_modified(&self) -> bool {
        self.doc.is_modified()
    }

    pub fn save(&mut self) -> Result<(), FileError> {
        let path = self.path.clone().ok_or_else(|| FileError::Io {
            path: PathBuf::new(),
            source: std::io::Error::new(std::io::ErrorKind::NotFound, "library has no file yet"),
        })?;
        self.save_as(path)
    }

    /// Save to `path`; the library takes the file's name (without extension).
    pub fn save_as(&mut self, path: impl AsRef<Path>) -> Result<(), FileError> {
        if let Some(stem) = path.as_ref().file_stem().map(|s| s.to_string_lossy().into_owned())
            && stem != self.name()
        {
            self.doc.set_library_name(stem);
        }
        save_library(self.library(), path.as_ref())?;
        self.path = Some(path.as_ref().to_path_buf());
        self.doc.mark_saved();
        Ok(())
    }

    /// Run several commands as one undo step.
    pub fn execute_all(&mut self, commands: Vec<Command>) -> Result<(), String> {
        let group = new_group();
        for (i, c) in commands.into_iter().enumerate() {
            if let Err(e) = self.doc.execute_grouped(c, Some(group)) {
                if i > 0 {
                    // Grouped commands form one undo step.
                    self.doc.close_group();
                    self.doc.undo();
                }
                return Err(e.to_string());
            }
        }
        self.doc.close_group();
        Ok(())
    }

    /// `base`, or `base 2`, `base 3`, … whichever is not taken by an object of the same kind.
    pub fn unique_name(&self, kind: ObjectKind, base: &str) -> String {
        let lib = self.library();
        let taken = |n: &str| match kind {
            ObjectKind::LibSymbol => lib.symbols.iter().any(|s| s.name == n),
            ObjectKind::LibFootprint => lib.footprints.iter().any(|f| f.name == n),
            _ => lib.parts.iter().any(|p| p.name == n),
        };
        if !taken(base) {
            return base.to_string();
        }
        (2..).map(|i| format!("{base} {i}")).find(|n| !taken(n)).unwrap()
    }

    pub fn new_symbol(&mut self, name: &str) -> Result<Id, String> {
        let symbol = blank_symbol(self.unique_name(ObjectKind::LibSymbol, name));
        let id = symbol.id;
        self.execute_all(vec![Command::Insert { objects: vec![Object::LibSymbol(symbol)] }])?;
        Ok(id)
    }

    pub fn new_footprint(&mut self, name: &str) -> Result<Id, String> {
        let footprint = blank_footprint(self.unique_name(ObjectKind::LibFootprint, name));
        let id = footprint.id;
        self.execute_all(vec![Command::Insert { objects: vec![Object::LibFootprint(footprint)] }])?;
        Ok(id)
    }

    /// A new part. Without a symbol or footprint, blank ones with the part's name are created.
    pub fn new_part(&mut self, name: &str, symbol: Option<Id>, footprint: Option<Id>) -> Result<Id, String> {
        let name = self.unique_name(ObjectKind::LibPart, name);
        let mut objects = Vec::new();
        let symbol = symbol.unwrap_or_else(|| {
            let s = blank_symbol(self.unique_name(ObjectKind::LibSymbol, &name));
            let id = s.id;
            objects.push(Object::LibSymbol(s));
            id
        });
        let footprint = footprint.unwrap_or_else(|| {
            let f = blank_footprint(self.unique_name(ObjectKind::LibFootprint, &name));
            let id = f.id;
            objects.push(Object::LibFootprint(f));
            id
        });
        let units = vec![PartUnit { name: "A".into(), symbol }];
        let fp_name = self.library().footprints.get(footprint).map(|f| f.name.clone());
        let part = Part {
            id: Id::new(),
            name: name.clone(),
            description: String::new(),
            prefix: "U".into(),
            default_value: name,
            variants: vec![PartVariant {
                id: Id::new(),
                name: fp_name.unwrap_or_else(|| "Default".into()),
                footprint,
                connections: self.library().map_by_name(&units, footprint),
            }],
            units,
            properties: Default::default(),
            source: None,
        };
        let id = part.id;
        objects.push(Object::LibPart(part));
        self.execute_all(vec![Command::Insert { objects }])?;
        Ok(id)
    }

    /// Copy a symbol, footprint or part (a part copy shares symbols and footprints).
    pub fn duplicate(&mut self, id: ObjectId) -> Result<ObjectId, String> {
        let lib = self.library();
        let object = match id {
            ObjectId::LibSymbol(i) => {
                let mut s = lib.symbols.get(i).ok_or("symbol not found")?.clone();
                s.id = Id::new();
                s.name = self.unique_name(ObjectKind::LibSymbol, &format!("{} copy", s.name));
                Object::LibSymbol(s)
            }
            ObjectId::LibFootprint(i) => {
                let mut f = lib.footprints.get(i).ok_or("footprint not found")?.clone();
                f.id = Id::new();
                f.name = self.unique_name(ObjectKind::LibFootprint, &format!("{} copy", f.name));
                Object::LibFootprint(f)
            }
            ObjectId::LibPart(i) => {
                let mut p = lib.parts.get(i).ok_or("part not found")?.clone();
                p.id = Id::new();
                p.name = self.unique_name(ObjectKind::LibPart, &format!("{} copy", p.name));
                p.source = None;
                for v in &mut p.variants {
                    v.id = Id::new();
                }
                Object::LibPart(p)
            }
            _ => return Err("not a library object".into()),
        };
        let new_id = object.id();
        self.execute_all(vec![Command::Insert { objects: vec![object] }])?;
        Ok(new_id)
    }

    pub fn delete(&mut self, id: ObjectId) -> Result<(), String> {
        self.execute_all(vec![Command::Delete { ids: vec![id] }])
    }

    /// Replace a part (after editing it in the part form).
    pub fn update_part(&mut self, part: Part) -> Result<(), String> {
        self.execute_all(vec![Command::Update { objects: vec![Object::LibPart(part)] }])
    }
}

/// A symbol with just the reference and value texts.
pub fn blank_symbol(name: String) -> Symbol {
    Symbol {
        id: Id::new(),
        name,
        graphics: Vec::new(),
        texts: vec![
            Text::new("${REF}", Point::mm(0.0, 3.81), mm(1.27)).aligned(HAlign::Center, VAlign::Bottom),
            Text::new("${VALUE}", Point::mm(0.0, -3.81), mm(1.27)).aligned(HAlign::Center, VAlign::Top),
        ],
        pins: Default::default(),
    }
}

/// A footprint with just the reference text.
pub fn blank_footprint(name: String) -> Footprint {
    Footprint {
        id: Id::new(),
        name,
        description: String::new(),
        pads: Default::default(),
        graphics: Vec::new(),
        texts: vec![FootprintText {
            layer: FootprintLayer::Silkscreen,
            text: Text::new("${REF}", Point::mm(0.0, 2.0), mm(1.0)).aligned(HAlign::Center, VAlign::Bottom),
        }],
    }
}
