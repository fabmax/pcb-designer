//! The project file (`.kicad_pro`, JSON): net classes, design rules, ERC and DRC
//! severities, the pin conflict matrix and the sheet list. KiCad fills in what's missing
//! with its defaults.

use model::*;
use serde_json::{Value, json};

/// ERC rules with a KiCad equivalent (the import's table, reversed).
pub const ERC_RULES: [(&str, ErcRule); 8] = [
    ("pin_not_driven", ErcRule::UndrivenInput),
    ("power_pin_not_driven", ErcRule::PowerInputUnpowered),
    ("pin_not_connected", ErcRule::UnconnectedPin),
    ("no_connect_connected", ErcRule::NoConnectMisplaced),
    ("multiple_net_names", ErcRule::NetNames),
    ("duplicate_reference", ErcRule::DuplicateReference),
    ("hier_label_mismatch", ErcRule::SheetPinUnconnected),
    ("bus_entry_needed", ErcRule::BusMisuse),
];

/// DRC checks with a KiCad equivalent.
pub const DRC_RULES: [(&str, ViolationKind); 9] = [
    ("shorting_items", ViolationKind::Short),
    ("clearance", ViolationKind::Clearance),
    ("items_not_allowed", ViolationKind::Keepout),
    ("copper_edge_clearance", ViolationKind::BoardEdge),
    ("courtyards_overlap", ViolationKind::Courtyard),
    ("track_width", ViolationKind::TrackWidth),
    ("drill_out_of_range", ViolationKind::Drill),
    ("track_dangling", ViolationKind::Unconnected),
    ("unconnected_items", ViolationKind::Unrouted),
];

/// KiCad's pin types in its matrix order, as ours where there is one.
const KICAD_PINS: [Option<PinKind>; 12] = [
    Some(PinKind::Input),
    Some(PinKind::Output),
    Some(PinKind::Bidirectional),
    Some(PinKind::TriState),
    Some(PinKind::Passive),
    None, // free
    None, // unspecified
    Some(PinKind::PowerIn),
    Some(PinKind::PowerOut),
    Some(PinKind::OpenCollector),
    None, // open emitter
    Some(PinKind::NotConnected),
];

/// KiCad's default pin conflict matrix (0 fine, 1 warning, 2 error), for the pin types
/// we don't have.
const KICAD_DEFAULT_PIN_MAP: [[u8; 12]; 12] = [
    [0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 2],
    [0, 2, 0, 1, 0, 0, 1, 2, 2, 2, 2, 2],
    [0, 0, 0, 0, 0, 0, 1, 0, 1, 0, 1, 2],
    [0, 1, 0, 0, 0, 0, 1, 1, 2, 1, 1, 2],
    [0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 2],
    [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2],
    [1, 1, 1, 1, 1, 0, 1, 1, 1, 1, 1, 2],
    [0, 2, 0, 1, 0, 0, 1, 0, 0, 0, 1, 2],
    [0, 2, 1, 2, 0, 0, 1, 0, 2, 2, 2, 2],
    [0, 2, 0, 1, 0, 0, 1, 0, 2, 0, 0, 2],
    [0, 2, 1, 1, 0, 0, 1, 1, 2, 0, 0, 2],
    [2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2],
];

fn severity(s: Severity) -> &'static str {
    match s {
        Severity::Error => "error",
        Severity::Warning => "warning",
        Severity::Ignore => "ignore",
    }
}

fn mm(v: Coord) -> f64 {
    v as f64 / 1e6
}

fn class(name: &str, c: &NetRules, priority: i64) -> Value {
    json!({
        "name": name,
        "priority": priority,
        "clearance": mm(c.clearance),
        "track_width": mm(c.track_width),
        "via_diameter": mm(c.via_diameter),
        "via_drill": mm(c.via_drill),
        "microvia_diameter": 0.3,
        "microvia_drill": 0.1,
        "diff_pair_gap": 0.25,
        "diff_pair_via_gap": 0.25,
        "diff_pair_width": 0.2,
        "wire_width": 6,
        "bus_width": 12,
        "line_style": 0,
        "schematic_color": "rgba(0, 0, 0, 0.000)",
        "pcb_color": "rgba(0, 0, 0, 0.000)",
    })
}

pub fn export(p: &Project, file_name: &str) -> String {
    let rules = &p.board.rules;
    let defaults = NetRules {
        clearance: rules.clearance,
        track_width: rules.track_width,
        via_diameter: rules.via_diameter,
        via_drill: rules.via_drill,
    };
    let mut classes = vec![class("Default", &defaults, i32::MAX as i64)];
    let mut patterns = Vec::new();
    for (i, c) in rules.net_classes.iter().enumerate() {
        let r = NetRules {
            clearance: c.clearance.unwrap_or(rules.clearance),
            track_width: c.track_width.unwrap_or(rules.track_width),
            via_diameter: c.via_diameter.unwrap_or(rules.via_diameter),
            via_drill: c.via_drill.unwrap_or(rules.via_drill),
        };
        classes.push(class(&c.name, &r, i as i64));
        for n in &c.nets {
            patterns.push(json!({ "netclass": c.name, "pattern": n }));
        }
    }

    let erc = &p.schematic.erc;
    let mut erc_rules = serde_json::Map::new();
    for (k, r) in ERC_RULES {
        erc_rules.insert(k.into(), severity(erc.severity(r)).into());
    }
    let mut pin_map = KICAD_DEFAULT_PIN_MAP.map(|r| r.map(Value::from));
    for (i, a) in KICAD_PINS.iter().enumerate() {
        for (j, b) in KICAD_PINS.iter().enumerate() {
            if let (Some(a), Some(b)) = (a, b) {
                pin_map[i][j] = match erc.pin_pair(*a, *b) {
                    Severity::Ignore => 0,
                    Severity::Warning => 1,
                    Severity::Error => 2,
                }
                .into();
            }
        }
    }
    let drc = &p.board.drc;
    let mut drc_rules = serde_json::Map::new();
    for (k, v) in DRC_RULES {
        drc_rules.insert(k.into(), severity(drc.severity(v)).into());
    }

    // The sheets: the root first, then each sub-sheet with its name.
    let s = &p.schematic;
    let sheets: Vec<Value> = s
        .page_order()
        .into_iter()
        .filter_map(|id| s.sheets.get(id))
        .map(|sh| json!([sh.id.to_string(), if sh.is_root() { "Root".to_string() } else { sh.name.clone() }]))
        .collect();

    let v = json!({
        "board": {
            "design_settings": {
                "rules": {
                    "min_clearance": 0.0,
                    "min_track_width": mm(rules.min_track_width),
                    "min_through_hole_diameter": mm(rules.min_drill),
                    "min_via_diameter": mm(rules.min_drill),
                },
                "rule_severities": Value::Object(drc_rules),
                "drc_exclusions": [],
            },
        },
        "erc": {
            "erc_exclusions": [],
            "meta": { "version": 0 },
            "pin_map": pin_map.iter().map(|r| Value::Array(r.to_vec())).collect::<Vec<_>>(),
            "rule_severities": Value::Object(erc_rules),
        },
        "libraries": { "pinned_footprint_libs": [], "pinned_symbol_libs": [] },
        "meta": { "filename": file_name, "version": 3 },
        "net_settings": {
            "classes": classes,
            "meta": { "version": 4 },
            "net_colors": null,
            "netclass_assignments": null,
            "netclass_patterns": patterns,
        },
        "schematic": { "legacy_lib_dir": "", "legacy_lib_list": [] },
        "sheets": sheets,
        "text_variables": {},
    });
    let mut out = serde_json::to_string_pretty(&v).unwrap_or_default();
    out.push('\n');
    out
}
