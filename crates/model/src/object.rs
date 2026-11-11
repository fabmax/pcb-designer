//! Uniform access to every editable object in a project.
//!
//! The command/undo layer only needs [`Project::get`], [`Project::take`] and
//! [`Project::put`]; it never has to know where an object kind is stored.

use serde::{Deserialize, Serialize};

use crate::board::{BoardGraphic, BoardText, FootprintInstance, Track, Via, Zone};
use crate::id::Id;
use crate::library::{Footprint, Part, Symbol};
use crate::project::Project;
use crate::schematic::{BusEntry, BusWire, Component, Junction, NetLabel, SchText, Sheet, SymbolInstance, Wire};

macro_rules! objects {
    ($( $variant:ident ( $ty:ty ) => $($path:ident).+ ;)*) => {
        /// Any editable object.
        #[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
        #[serde(tag = "type", rename_all = "snake_case")]
        pub enum Object {
            $( $variant($ty), )*
        }

        /// Typed reference to an object. Serialized as `{"type": "...", "id": "..."}`.
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, schemars::JsonSchema)]
        #[serde(tag = "type", content = "id", rename_all = "snake_case")]
        pub enum ObjectId {
            $( $variant(Id), )*
        }

        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, schemars::JsonSchema)]
        #[serde(rename_all = "snake_case")]
        pub enum ObjectKind {
            $( $variant, )*
        }

        impl ObjectKind {
            pub const ALL: &'static [ObjectKind] = &[$( ObjectKind::$variant, )*];
        }

        impl Object {
            pub fn id(&self) -> ObjectId {
                match self {
                    $( Object::$variant(o) => ObjectId::$variant(o.id), )*
                }
            }
        }

        impl ObjectId {
            pub fn kind(&self) -> ObjectKind {
                match self {
                    $( ObjectId::$variant(_) => ObjectKind::$variant, )*
                }
            }

            pub fn raw(&self) -> Id {
                match self {
                    $( ObjectId::$variant(id) => *id, )*
                }
            }
        }

        impl Project {
            /// All objects of one kind, in stored order.
            pub fn objects(&self, kind: ObjectKind) -> Box<dyn Iterator<Item = Object> + '_> {
                match kind {
                    $( ObjectKind::$variant => Box::new(self.$($path).+.iter().cloned().map(Object::$variant)), )*
                }
            }

            pub fn count(&self, kind: ObjectKind) -> usize {
                match kind {
                    $( ObjectKind::$variant => self.$($path).+.len(), )*
                }
            }

            pub fn get(&self, id: ObjectId) -> Option<Object> {
                match id {
                    $( ObjectId::$variant(i) => self.$($path).+.get(i).cloned().map(Object::$variant), )*
                }
            }

            pub fn contains(&self, id: ObjectId) -> bool {
                match id {
                    $( ObjectId::$variant(i) => self.$($path).+.contains(i), )*
                }
            }

            /// Remove an object, returning it and its index in its collection.
            pub fn take(&mut self, id: ObjectId) -> Option<(usize, Object)> {
                match id {
                    $( ObjectId::$variant(i) => self.$($path).+.remove(i).map(|(n, o)| (n, Object::$variant(o))), )*
                }
            }

            /// Replace an existing object in place (keeping its index). Returns the index
            /// and the old object; `None` (dropping `object`) if no object has its id.
            pub fn replace(&mut self, object: Object) -> Option<(usize, Object)> {
                match object {
                    $( Object::$variant(o) => self.$($path).+.replace(o).map(|(n, old)| (n, Object::$variant(old))), )*
                }
            }

            /// Replace an existing object in place, or insert a new one at `index`
            /// (default: append).
            pub fn put(&mut self, object: Object, index: Option<usize>) {
                match object {
                    $( Object::$variant(o) => {
                        let store = &mut self.$($path).+;
                        let at = index.unwrap_or(usize::MAX);
                        store.insert_at(at, o);
                    } )*
                }
            }
        }
    };
}

objects! {
    Sheet(Sheet) => schematic.sheets;
    Component(Component) => schematic.components;
    Symbol(SymbolInstance) => schematic.symbols;
    Wire(Wire) => schematic.wires;
    BusWire(BusWire) => schematic.bus_wires;
    BusEntry(BusEntry) => schematic.bus_entries;
    Junction(Junction) => schematic.junctions;
    Label(NetLabel) => schematic.labels;
    SchText(SchText) => schematic.texts;
    Footprint(FootprintInstance) => board.footprints;
    Track(Track) => board.tracks;
    Via(Via) => board.vias;
    Zone(Zone) => board.zones;
    BoardGraphic(BoardGraphic) => board.graphics;
    BoardText(BoardText) => board.texts;
    LibSymbol(Symbol) => library.symbols;
    LibFootprint(Footprint) => library.footprints;
    LibPart(Part) => library.parts;
}

impl ObjectKind {
    /// Objects that belong to the schematic.
    pub fn is_schematic(self) -> bool {
        matches!(
            self,
            ObjectKind::Sheet
                | ObjectKind::Component
                | ObjectKind::Symbol
                | ObjectKind::Wire
                | ObjectKind::BusWire
                | ObjectKind::BusEntry
                | ObjectKind::Junction
                | ObjectKind::Label
                | ObjectKind::SchText
        )
    }

    pub fn is_board(self) -> bool {
        matches!(
            self,
            ObjectKind::Footprint
                | ObjectKind::Track
                | ObjectKind::Via
                | ObjectKind::Zone
                | ObjectKind::BoardGraphic
                | ObjectKind::BoardText
        )
    }

    pub fn is_library(self) -> bool {
        matches!(self, ObjectKind::LibSymbol | ObjectKind::LibFootprint | ObjectKind::LibPart)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geom::Point;

    #[test]
    fn take_and_put_restore_position() {
        let mut p = Project::new("t");
        let sheet = p.schematic.root_sheet().unwrap();
        let wires: Vec<Wire> =
            (0..3).map(|i| Wire { id: Id::new(), sheet, a: Point::new(i, 0), b: Point::new(i, 10) }).collect();
        for w in &wires {
            p.put(Object::Wire(w.clone()), None);
        }
        let before = p.clone();
        let (idx, obj) = p.take(ObjectId::Wire(wires[1].id)).unwrap();
        assert_eq!(idx, 1);
        assert!(!p.contains(ObjectId::Wire(wires[1].id)));
        p.put(obj, Some(idx));
        assert_eq!(p, before);
    }

    #[test]
    fn object_id_json_shape() {
        let id = ObjectId::Wire(Id::from_u128(1));
        let json = serde_json::to_string(&id).unwrap();
        assert_eq!(json, r#"{"type":"wire","id":"00000000-0000-0000-0000-000000000001"}"#);
        assert_eq!(serde_json::from_str::<ObjectId>(&json).unwrap(), id);
    }
}
