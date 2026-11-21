//! The Gerber job file (`.gbrjob`, JSON): what the board is and which file holds what,
//! so a manufacturer's front end can take the fileset without guessing.

use serde_json::{Value, json};

use model::*;

use crate::OutputFile;

/// The X2 file attributes of a Gerber or Excellon file, scanned from its text:
/// `%TF.FileFunction,…*%` and `; #@! TF.FileFunction,…` lines.
fn file_attributes(content: &str) -> (Option<String>, Option<String>) {
    let mut function = None;
    let mut polarity = None;
    for line in content.lines() {
        let attr =
            line.strip_prefix("%TF").and_then(|l| l.strip_suffix("*%")).or_else(|| line.strip_prefix("; #@! TF"));
        let Some(attr) = attr else { continue };
        if let Some(v) = attr.strip_prefix(".FileFunction,") {
            function = Some(v.to_string());
        } else if let Some(v) = attr.strip_prefix(".FilePolarity,") {
            polarity = Some(v.to_string());
        }
    }
    (function, polarity)
}

/// The job file of `files`. `created` is the creation time (ISO 8601, see [`now`]).
pub fn job_file(project: &Project, files: &[OutputFile], created: &str) -> String {
    let board = &project.board;
    // The board edge is the outline's stroke centre line.
    let half_stroke = |s: &Shape| match s {
        Shape::Line { width, .. }
        | Shape::Arc { width, .. }
        | Shape::Circle { width, .. }
        | Shape::Rect { width, .. }
        | Shape::Polygon { width, .. } => width.max(&0) / 2,
    };
    let outline = Rect::union_all(
        board
            .graphics
            .iter()
            .filter(|g| g.layer == LayerId::OUTLINE)
            .map(|g| g.shape.bounds().grow(-half_stroke(&g.shape))),
    );
    let mm = |v: Coord| (v as f64 / 1e3).round() / 1e3;
    let mut specs = json!({
        "ProjectId": { "Name": project.name, "GUID": project_guid(project), "Revision": "1" },
        "LayerNumber": board.copper_layers().count(),
        "BoardThickness": mm(board.stackup.thickness),
    });
    if let Some(finish) = board.stackup.finish.job_name() {
        specs["Finish"] = json!(finish);
    }
    if let Some(r) = outline {
        specs["Size"] = json!({ "X": mm(r.width()), "Y": mm(r.height()) });
    }
    let attributes: Vec<Value> = files
        .iter()
        .filter_map(|f| {
            let (function, polarity) = file_attributes(&f.content);
            let mut v = json!({ "Path": f.name, "FileFunction": function? });
            v["FilePolarity"] = json!(polarity.unwrap_or_else(|| "Positive".into()));
            Some(v)
        })
        .collect();
    let job = json!({
        "Header": {
            "GenerationSoftware": {
                "Vendor": "pcb-designer",
                "Application": "pcb-designer",
                "Version": env!("CARGO_PKG_VERSION"),
            },
            "CreationDate": created,
        },
        "GeneralSpecs": specs,
        "DesignRules": [{
            "Layers": "Outer",
            "PadToPad": mm(board.rules.clearance),
            "PadToTrack": mm(board.rules.clearance),
            "TrackToTrack": mm(board.rules.clearance),
            "MinLineWidth": mm(board.rules.min_track_width),
        }],
        "FilesAttributes": attributes,
        "MaterialStackup": material_stackup(board),
    });
    serde_json::to_string_pretty(&job).expect("serializable")
}

/// The layers top to bottom: silkscreen, mask, copper and FR-4 between the copper
/// layers, splitting what the copper leaves of the thickness evenly.
fn material_stackup(board: &Board) -> Vec<Value> {
    let s = &board.stackup;
    let mm = |v: Coord| (v as f64 / 1e3).round() / 1e3;
    let copper: Vec<&BoardLayer> = board.copper_layers().collect();
    let n = copper.len();
    let copper_total = s.outer_copper * 2 + s.inner_copper * (n as Coord - 2).max(0);
    let dielectric = (s.thickness - copper_total).max(0) / (n as Coord - 1).max(1);
    let mut out = vec![
        json!({ "Type": "Legend", "Color": s.silk_color.label(), "Name": "Top Silkscreen" }),
        json!({ "Type": "SolderMask", "Color": s.mask_color.label(), "Name": "Top Solder Mask" }),
    ];
    for (i, layer) in copper.iter().enumerate() {
        let outer = i == 0 || i == n - 1;
        let thickness = if outer { s.outer_copper } else { s.inner_copper };
        out.push(json!({ "Type": "Copper", "Thickness": mm(thickness), "Name": layer.name }));
        if i + 1 < n {
            let name = format!("Dielectric {}", i + 1);
            out.push(json!({ "Type": "Dielectric", "Thickness": mm(dielectric), "Material": "FR4", "Name": name }));
        }
    }
    out.push(json!({ "Type": "SolderMask", "Color": s.mask_color.label(), "Name": "Bottom Solder Mask" }));
    out.push(json!({ "Type": "Legend", "Color": s.silk_color.label(), "Name": "Bottom Silkscreen" }));
    out
}

/// A stable GUID for the project: its name hashed (the job file wants one; the project
/// has no id of its own).
fn project_guid(project: &Project) -> String {
    let h = model::Fnv1a::of_json(&project.name).unwrap_or_default();
    let hex: String = h.chars().filter(char::is_ascii_hexdigit).cycle().take(32).collect();
    format!("{}-{}-{}-{}-{}", &hex[0..8], &hex[8..12], &hex[12..16], &hex[16..20], &hex[20..32])
}

/// The current time in UTC as ISO 8601 (`2026-11-18T09:30:00Z`).
pub fn now() -> String {
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64);
    iso8601(secs)
}

/// Seconds since 1970 as an ISO 8601 UTC time (days to a civil date after Howard
/// Hinnant's algorithm).
fn iso8601(secs: i64) -> String {
    let (days, rem) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z", rem / 3600, rem % 3600 / 60, rem % 60)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates() {
        assert_eq!(iso8601(0), "1970-01-01T00:00:00Z");
        assert_eq!(iso8601(951_782_400), "2000-02-29T00:00:00Z");
        assert_eq!(iso8601(1_794_990_645), "2026-11-18T08:30:45Z");
    }

    #[test]
    fn the_stackup_adds_up_to_the_thickness() {
        let mut board = Board::with_copper_layers(4);
        board.stackup.finish = Finish::Enig;
        let layers = material_stackup(&board);
        let types: Vec<&str> = layers.iter().map(|l| l["Type"].as_str().unwrap()).collect();
        assert_eq!(types.iter().filter(|t| **t == "Copper").count(), 4);
        assert_eq!(types.iter().filter(|t| **t == "Dielectric").count(), 3);
        assert_eq!((types[0], types[1], types[2]), ("Legend", "SolderMask", "Copper"));
        let total: f64 = layers.iter().filter_map(|l| l["Thickness"].as_f64()).sum();
        assert!((total - 1.6).abs() < 0.002, "{total}");
        assert_eq!(layers[2]["Thickness"], 0.035);
        assert_eq!(layers[4]["Thickness"], 0.018, "inner copper");
    }

    #[test]
    fn attributes_of_gerber_and_drill_files() {
        let gbr = "%TF.GenerationSoftware,x*%\n%TF.FileFunction,Copper,L1,Top*%\n%TF.FilePolarity,Positive*%\n";
        assert_eq!(file_attributes(gbr), (Some("Copper,L1,Top".into()), Some("Positive".into())));
        let drl = "M48\n; #@! TF.FileFunction,Plated,1,2,PTH\n";
        assert_eq!(file_attributes(drl), (Some("Plated,1,2,PTH".into()), None));
    }
}
