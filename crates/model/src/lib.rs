//! Document model for PCB Designer.
//!
//! * All lengths are integer nanometers ([`Coord`]), angles are degrees ([`Angle`]).
//! * Every object has a stable UUID ([`Id`]). Collections keep insertion order
//!   ([`Store`]) so saved files are stable and diff well.
//! * A [`Project`] holds an embedded copy of the library parts it uses, one schematic and
//!   one board. It is saved as a folder of JSON files (see [`io`]).
//! * [`Object`]/[`ObjectId`] give uniform access to every editable object, which the
//!   command and undo layer (`editor-core`) builds on.

pub mod board;
pub mod builtin;
pub mod geom;
pub mod hash;
pub mod id;
pub mod io;
pub mod library;
pub mod object;
pub mod project;
pub mod schematic;
pub mod store;
pub mod units;

pub use board::*;
pub use geom::*;
pub use hash::Fnv1a;
pub use id::Id;
pub use library::*;
pub use object::{Object, ObjectId, ObjectKind};
pub use project::*;
pub use schematic::*;
pub use store::{HasId, Store};
pub use units::*;
