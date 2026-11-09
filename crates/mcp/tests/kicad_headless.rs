//! Headless mode searches and imports KiCad libraries; imported parts go straight into
//! the project.

use std::fs;

use mcp::headless::HeadlessHost;
use mcp::host::{Host, KicadImport, Reply};
use model::Project;
use view::Document;

const SYM: &str = r#"(kicad_symbol_lib (version 20231120) (generator "kicad_symbol_editor")
  (symbol "R" (pin_numbers hide) (pin_names (offset 0) hide) (in_bom yes) (on_board yes)
    (property "Reference" "R" (at 2.032 0 90) (effects (font (size 1.27 1.27))))
    (property "Value" "R" (at 0 0 90) (effects (font (size 1.27 1.27))))
    (property "Footprint" "Resistor_SMD:R_0603_1608Metric" (at 0 0 0) (effects (font (size 1.27 1.27)) hide))
    (symbol "R_1_1"
      (pin passive line (at 0 3.81 270) (length 1.27) (name "~" (effects (font (size 1.27 1.27)))) (number "1" (effects (font (size 1.27 1.27)))))
      (pin passive line (at 0 -3.81 90) (length 1.27) (name "~" (effects (font (size 1.27 1.27)))) (number "2" (effects (font (size 1.27 1.27)))))
    )
  )
)"#;

const FP: &str = r#"(footprint "R_0603_1608Metric" (version 20240108) (generator "pcbnew") (layer "F.Cu")
  (pad "1" smd roundrect (at -0.825 0) (size 0.8 0.9) (layers "F.Cu" "F.Paste" "F.Mask") (roundrect_rratio 0.25))
  (pad "2" smd roundrect (at 0.825 0) (size 0.8 0.9) (layers "F.Cu" "F.Paste" "F.Mask") (roundrect_rratio 0.25))
)"#;

fn reply(f: impl FnOnce(mcp::host::Responder)) -> Result<serde_json::Value, String> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    f(tx);
    rx.blocking_recv().unwrap().map(|r| match r {
        Reply::Json(v) => v,
        Reply::Png(_) => panic!("json expected"),
    })
}

#[test]
fn search_and_import_into_the_project() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("Device.kicad_sym"), SYM).unwrap();
    fs::create_dir_all(dir.path().join("Resistor_SMD.pretty")).unwrap();
    fs::write(dir.path().join("Resistor_SMD.pretty/R_0603_1608Metric.kicad_mod"), FP).unwrap();

    let mut host = HeadlessHost::new(Document::new(Project::new("Test")));
    // Nothing to download into: refused with a clear message.
    let refused = reply(|r| host.kicad_download(false, r));
    assert!(refused.unwrap_err().contains("no folder"));

    host.kicad_paths = vec![dir.path().to_path_buf()];
    let found = host.kicad_search("device r", false, 10).unwrap();
    assert_eq!(found["results"][0], "Device:R", "{found}");

    let job = KicadImport { symbols: vec![("Device:R".into(), None)], ..Default::default() };
    let imported = reply(|r| host.kicad_import(job, r)).unwrap();
    assert_eq!(imported["in_project"], true, "{imported}");
    assert_eq!(imported["parts"][0]["variants"][0]["footprint"], "R_0603_1608Metric", "{imported}");
    let project = host.doc.project();
    assert_eq!(project.library.parts.len(), 1, "the part is ready to place");
    assert_eq!(host.libraries().len(), 2, "starter and KiCad import libraries");
}
