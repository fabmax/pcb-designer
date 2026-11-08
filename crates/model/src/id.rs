//! Stable object identifiers.

use std::fmt;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Id(Uuid);

impl Id {
    /// A new random id.
    #[allow(clippy::new_without_default)]
    pub fn new() -> Self {
        Id(Uuid::new_v4())
    }

    /// Deterministic id (tests, built-in library content).
    pub const fn from_u128(v: u128) -> Self {
        Id(Uuid::from_u128(v))
    }

    pub fn as_uuid(&self) -> Uuid {
        self.0
    }
}

impl fmt::Debug for Id {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Short form keeps debug output readable.
        write!(f, "#{}", &self.0.simple().to_string()[..8])
    }
}

impl fmt::Display for Id {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl std::str::FromStr for Id {
    type Err = uuid::Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Id(Uuid::parse_str(s)?))
    }
}
