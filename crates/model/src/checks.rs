//! Settings of the electrical rules check (schematic) and the design rules check (board):
//! a severity per rule, the pin conflict matrix and excluded findings. Only what differs
//! from the defaults is stored.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::PinKind;
use crate::hash::Fnv1a;

/// How much a finding matters.
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    #[default]
    Error,
    Warning,
    /// Not reported (in the pin conflict matrix: the pair is fine).
    Ignore,
}

impl Severity {
    pub const ALL: [Severity; 3] = [Severity::Error, Severity::Warning, Severity::Ignore];

    pub fn label(self) -> &'static str {
        match self {
            Severity::Error => "Error",
            Severity::Warning => "Warning",
            Severity::Ignore => "Ignore",
        }
    }
}

/// The kinds of schematic findings.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ErcRule {
    /// Two pins whose kinds don't go together on one net (see [`PinMatrix`]).
    PinConflict,
    UndrivenInput,
    PowerInputUnpowered,
    UnconnectedPin,
    NoConnectMisplaced,
    PowerFlagMisplaced,
    NetNames,
    DuplicateReference,
    SheetPinUnconnected,
    HierLabelOnRoot,
    BusMisuse,
}

impl ErcRule {
    pub const ALL: [ErcRule; 11] = [
        ErcRule::PinConflict,
        ErcRule::UndrivenInput,
        ErcRule::PowerInputUnpowered,
        ErcRule::UnconnectedPin,
        ErcRule::NoConnectMisplaced,
        ErcRule::PowerFlagMisplaced,
        ErcRule::NetNames,
        ErcRule::DuplicateReference,
        ErcRule::SheetPinUnconnected,
        ErcRule::HierLabelOnRoot,
        ErcRule::BusMisuse,
    ];

    pub fn label(self) -> &'static str {
        match self {
            ErcRule::PinConflict => "Pin conflicts",
            ErcRule::UndrivenInput => "Input not driven",
            ErcRule::PowerInputUnpowered => "Unpowered power input",
            ErcRule::UnconnectedPin => "Unconnected pin",
            ErcRule::NoConnectMisplaced => "Misplaced no-connect flag",
            ErcRule::PowerFlagMisplaced => "Misplaced power flag",
            ErcRule::NetNames => "Net with several names",
            ErcRule::DuplicateReference => "Duplicate reference",
            ErcRule::SheetPinUnconnected => "Unconnected sheet pin",
            ErcRule::HierLabelOnRoot => "Hier. label on root sheet",
            ErcRule::BusMisuse => "Bus misuse",
        }
    }

    /// The snake_case name (as in files and MCP).
    pub fn code(self) -> &'static str {
        match self {
            ErcRule::PinConflict => "pin_conflict",
            ErcRule::UndrivenInput => "undriven_input",
            ErcRule::PowerInputUnpowered => "power_input_unpowered",
            ErcRule::UnconnectedPin => "unconnected_pin",
            ErcRule::NoConnectMisplaced => "no_connect_misplaced",
            ErcRule::PowerFlagMisplaced => "power_flag_misplaced",
            ErcRule::NetNames => "net_names",
            ErcRule::DuplicateReference => "duplicate_reference",
            ErcRule::SheetPinUnconnected => "sheet_pin_unconnected",
            ErcRule::HierLabelOnRoot => "hier_label_on_root",
            ErcRule::BusMisuse => "bus_misuse",
        }
    }

    /// Pin conflicts take theirs from the matrix; this is for the others.
    pub fn default_severity(self) -> Severity {
        match self {
            ErcRule::NoConnectMisplaced
            | ErcRule::PowerFlagMisplaced
            | ErcRule::NetNames
            | ErcRule::SheetPinUnconnected
            | ErcRule::HierLabelOnRoot => Severity::Warning,
            _ => Severity::Error,
        }
    }
}

/// The kinds of design rule violations.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ViolationKind {
    Short,
    Clearance,
    /// Copper or a footprint inside a keepout that forbids it.
    Keepout,
    BoardEdge,
    Courtyard,
    TrackWidth,
    Drill,
    /// A via whose layers aren't two different copper layers of the board, top one first.
    ViaSpan,
    /// A zone's net name matches no net on the board (renamed, or its pads are gone),
    /// so it connects nothing.
    ZoneNet,
    Unconnected,
    Unrouted,
}

impl ViolationKind {
    pub const ALL: [ViolationKind; 11] = [
        ViolationKind::Short,
        ViolationKind::Clearance,
        ViolationKind::Keepout,
        ViolationKind::BoardEdge,
        ViolationKind::Courtyard,
        ViolationKind::TrackWidth,
        ViolationKind::Drill,
        ViolationKind::ViaSpan,
        ViolationKind::ZoneNet,
        ViolationKind::Unconnected,
        ViolationKind::Unrouted,
    ];

    pub fn label(self) -> &'static str {
        match self {
            ViolationKind::Short => "Short",
            ViolationKind::Clearance => "Clearance",
            ViolationKind::Keepout => "Keepout",
            ViolationKind::BoardEdge => "Board edge",
            ViolationKind::Courtyard => "Courtyard overlap",
            ViolationKind::TrackWidth => "Track width",
            ViolationKind::Drill => "Drill",
            ViolationKind::ViaSpan => "Via span",
            ViolationKind::ZoneNet => "Zone net",
            ViolationKind::Unconnected => "Unconnected copper",
            ViolationKind::Unrouted => "Unrouted",
        }
    }

    pub fn code(self) -> &'static str {
        match self {
            ViolationKind::Short => "short",
            ViolationKind::Clearance => "clearance",
            ViolationKind::Keepout => "keepout",
            ViolationKind::BoardEdge => "board_edge",
            ViolationKind::Courtyard => "courtyard",
            ViolationKind::TrackWidth => "track_width",
            ViolationKind::Drill => "drill",
            ViolationKind::ViaSpan => "via_span",
            ViolationKind::ZoneNet => "zone_net",
            ViolationKind::Unconnected => "unconnected",
            ViolationKind::Unrouted => "unrouted",
        }
    }

    pub fn default_severity(self) -> Severity {
        match self {
            ViolationKind::Courtyard | ViolationKind::ZoneNet => Severity::Warning,
            _ => Severity::Error,
        }
    }
}

/// A finding the user looked at and accepted. It matches the finding with the same key.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Exclusion {
    /// [`finding_key`] of the finding.
    pub key: String,
    /// The finding's message when it was excluded (shown while it no longer occurs).
    pub message: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub comment: String,
}

/// The stable identity of a finding: its rule and what it is about (object ids, pins as
/// "symbol id/number", net names), in any order. `"pin_conflict-0123456789abcdef"`.
pub fn finding_key(rule: &str, parts: &[String]) -> String {
    let mut parts: Vec<&str> = parts.iter().map(String::as_str).collect();
    parts.sort_unstable();
    parts.dedup();
    let mut h = Fnv1a::default();
    h.update(rule.as_bytes());
    for p in parts {
        h.update(&[0]);
        h.update(p.as_bytes());
    }
    format!("{rule}-{:016x}", h.finish())
}

/// The pin kinds in matrix order.
pub const PIN_KINDS: [PinKind; 9] = [
    PinKind::Input,
    PinKind::Output,
    PinKind::Bidirectional,
    PinKind::TriState,
    PinKind::Passive,
    PinKind::PowerIn,
    PinKind::PowerOut,
    PinKind::OpenCollector,
    PinKind::NotConnected,
];

impl PinKind {
    pub fn label(self) -> &'static str {
        match self {
            PinKind::Input => "Input",
            PinKind::Output => "Output",
            PinKind::Bidirectional => "Bidirectional",
            PinKind::TriState => "Tri-state",
            PinKind::Passive => "Passive",
            PinKind::PowerIn => "Power in",
            PinKind::PowerOut => "Power out",
            PinKind::OpenCollector => "Open collector",
            PinKind::NotConnected => "Not connected",
        }
    }
}

fn kind_index(k: PinKind) -> usize {
    PIN_KINDS.iter().position(|&p| p == k).expect("every pin kind is in the matrix")
}

/// KiCad's default pin conflict matrix on our pin kinds (O: fine, W: warning, E: error).
const DEFAULT_MATRIX: [&str; 9] = [
    // I O B T P i o C N
    "OOOOOOOOE", // input
    "OEOWOOEEE", // output
    "OOOOOOWOE", // bidirectional
    "OWOOOWEWE", // tri-state
    "OOOOOOOOE", // passive
    "OOOWOOOOE", // power input
    "OEWEOOEEE", // power output
    "OEOWOOEOE", // open collector
    "EEEEEEEEE", // not connected
];

/// One changed cell of the pin conflict matrix (the matrix is symmetric).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct MatrixCell {
    pub a: PinKind,
    pub b: PinKind,
    pub severity: Severity,
}

/// The pin conflict matrix: the defaults with `cells` changed.
pub fn default_pin_pair(a: PinKind, b: PinKind) -> Severity {
    match DEFAULT_MATRIX[kind_index(a)].as_bytes()[kind_index(b)] {
        b'E' => Severity::Error,
        b'W' => Severity::Warning,
        _ => Severity::Ignore,
    }
}

/// Settings of the electrical rules check (stored in the schematic).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ErcSettings {
    /// Rules whose severity differs from the default.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub severities: BTreeMap<ErcRule, Severity>,
    /// Changed cells of the pin conflict matrix.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub matrix: Vec<MatrixCell>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub exclusions: Vec<Exclusion>,
}

impl ErcSettings {
    pub fn is_default(&self) -> bool {
        self == &ErcSettings::default()
    }

    pub fn severity(&self, rule: ErcRule) -> Severity {
        self.severities.get(&rule).copied().unwrap_or(rule.default_severity())
    }

    pub fn set_severity(&mut self, rule: ErcRule, s: Severity) {
        if s == rule.default_severity() {
            self.severities.remove(&rule);
        } else {
            self.severities.insert(rule, s);
        }
    }

    /// What a net with pins of kinds `a` and `b` is (`Ignore`: fine).
    pub fn pin_pair(&self, a: PinKind, b: PinKind) -> Severity {
        self.matrix
            .iter()
            .find(|c| (c.a, c.b) == (a, b) || (c.a, c.b) == (b, a))
            .map_or(default_pin_pair(a, b), |c| c.severity)
    }

    pub fn set_pin_pair(&mut self, a: PinKind, b: PinKind, severity: Severity) {
        self.matrix.retain(|c| (c.a, c.b) != (a, b) && (c.a, c.b) != (b, a));
        if severity != default_pin_pair(a, b) {
            let (a, b) = if kind_index(a) <= kind_index(b) { (a, b) } else { (b, a) };
            self.matrix.push(MatrixCell { a, b, severity });
        }
    }

    pub fn excluded(&self, key: &str) -> Option<&Exclusion> {
        self.exclusions.iter().find(|e| e.key == key)
    }
}

/// Settings of the design rules check (stored in the board).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct DrcSettings {
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub severities: BTreeMap<ViolationKind, Severity>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub exclusions: Vec<Exclusion>,
}

impl DrcSettings {
    pub fn is_default(&self) -> bool {
        self == &DrcSettings::default()
    }

    pub fn severity(&self, kind: ViolationKind) -> Severity {
        self.severities.get(&kind).copied().unwrap_or(kind.default_severity())
    }

    pub fn set_severity(&mut self, kind: ViolationKind, s: Severity) {
        if s == kind.default_severity() {
            self.severities.remove(&kind);
        } else {
            self.severities.insert(kind, s);
        }
    }

    pub fn excluded(&self, key: &str) -> Option<&Exclusion> {
        self.exclusions.iter().find(|e| e.key == key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_matrix_is_symmetric() {
        for a in PIN_KINDS {
            for b in PIN_KINDS {
                assert_eq!(default_pin_pair(a, b), default_pin_pair(b, a), "{a:?} {b:?}");
            }
        }
        assert_eq!(default_pin_pair(PinKind::Output, PinKind::Output), Severity::Error);
        assert_eq!(default_pin_pair(PinKind::Passive, PinKind::Output), Severity::Ignore);
    }

    #[test]
    fn only_changes_are_stored() {
        let mut s = ErcSettings::default();
        s.set_pin_pair(PinKind::PowerOut, PinKind::Output, Severity::Warning);
        assert_eq!(s.pin_pair(PinKind::Output, PinKind::PowerOut), Severity::Warning);
        s.set_pin_pair(PinKind::Output, PinKind::PowerOut, Severity::Error);
        s.set_severity(ErcRule::NetNames, Severity::Warning);
        assert!(s.is_default());
        assert_eq!(finding_key("x", &["b".into(), "a".into()]), finding_key("x", &["a".into(), "b".into()]));
    }
}
