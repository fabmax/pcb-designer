//! The Built-in library: a standard library of generic parts, bundled with the app as
//! `library/builtin.pcblib` (embedded, parsed once).
//!
//! Most of it is generated ([`generate`]: passives, diodes, transistors, connectors, …);
//! `cargo run -p model --example gen_builtin` rewrites the generated items in the file and
//! keeps the others, which are drawn by hand (in the app or through MCP). Generated ids
//! are hashed from item names ([`gid`]) into their own range ([`is_generated`]), so the
//! two kinds never mix.
//!
//! Text fields may contain `${REF}` and `${VALUE}`, which are replaced by the component's
//! reference designator and value when drawn.

use std::sync::OnceLock;

use crate::id::Id;
use crate::io::{FileKind, MIGRATIONS, from_file_str};
use crate::library::Library;

pub mod generate;

/// Fixed id of a hand-made item.
const fn bid(n: u128) -> Id {
    Id::from_u128(0x5ce1_0000_0000_4000_8000_0000_0000_0000 | n)
}

/// The id range of generated items: the top 80 bits.
const GENERATED: u128 = 0x5ce1_0000_0000_4000_9000_0000_0000_0000;
const GENERATED_MASK: u128 = !0xffff_ffff_ffff;

/// The id of the generated item `key` (`part/Resistor`, `fp/SOT-23/pad/0`): FNV-1a of the
/// key in the generated range.
pub const fn gid(key: &str) -> Id {
    let bytes = key.as_bytes();
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    let mut i = 0;
    while i < bytes.len() {
        h = (h ^ bytes[i] as u64).wrapping_mul(0x0000_0100_0000_01b3);
        i += 1;
    }
    Id::from_u128(GENERATED | (h as u128 & !GENERATED_MASK))
}

/// Does `id` belong to a generated item (replaced by `gen_builtin`)?
pub fn is_generated(id: Id) -> bool {
    id.as_uuid().as_u128() & GENERATED_MASK == GENERATED
}

pub const LIBRARY_ID: Id = bid(1);
/// The id of an editable copy (`gen_builtin edit`), so it opens next to the Built-in one.
pub const EDIT_LIBRARY_ID: Id = bid(2);
pub const PART_RESISTOR: Id = gid("part/Resistor");
pub const PART_CAPACITOR: Id = gid("part/Capacitor");
pub const PART_LED: Id = gid("part/LED");
pub const PART_HEADER_1X4: Id = gid("part/Conn_01x04");
pub const PART_DIODE: Id = gid("part/Diode");
pub const PART_NPN: Id = gid("part/NPN");
pub const PART_MOUNTING_HOLE: Id = gid("part/MountingHole_M3");
pub const PART_VSOURCE: Id = gid("part/VSource");
pub const PART_ISOURCE: Id = gid("part/ISource");
/// Hand-made.
pub const PART_OPAMP: Id = bid(0x700);
pub const PART_74HC00: Id = bid(0x900);

/// The bundled file, in the source tree (what `gen_builtin` rewrites).
pub const FILE_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/library/builtin.pcblib");
const FILE: &str = include_str!("../../library/builtin.pcblib");

pub fn builtin_library() -> Library {
    static LIBRARY: OnceLock<Library> = OnceLock::new();
    LIBRARY
        .get_or_init(|| {
            from_file_str(FILE, FileKind::Library, MIGRATIONS, std::path::Path::new("builtin.pcblib"))
                .expect("the bundled library loads")
        })
        .clone()
}

#[cfg(test)]
mod tests;
