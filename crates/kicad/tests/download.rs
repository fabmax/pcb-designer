//! Library download against a local HTTP server serving generated archives (no internet).
//! `cargo test -p kicad --test download -- --ignored` also runs one against the real GitLab.

use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::path::Path;

use kicad::download::{self, DownloadError, Source};

/// A `.tar.gz` like GitLab's: everything in `<repo>-<version>/`.
fn archive(repo: &str, version: &str, files: &[(&str, &str)]) -> Vec<u8> {
    let gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    let mut tar = tar::Builder::new(gz);
    for (path, content) in files {
        let mut header = tar::Header::new_gnu();
        header.set_size(content.len() as u64);
        header.set_mode(0o644);
        header.set_cksum();
        tar.append_data(&mut header, format!("{repo}-{version}/{path}"), content.as_bytes()).unwrap();
    }
    tar.into_inner().unwrap().finish().unwrap()
}

/// Serves tag lists and archives of versions 10.0.8 and 10.0.9 (the symbols repository
/// also has 10.0.10, the footprints one doesn't; release candidates and 11.x are ignored).
fn server() -> Source {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let mut line = String::new();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            reader.read_line(&mut line).unwrap();
            loop {
                let mut h = String::new();
                if reader.read_line(&mut h).unwrap() <= 2 {
                    break;
                }
            }
            let path = line.split_whitespace().nth(1).unwrap_or("").to_string();
            let body: Option<Vec<u8>> = if path.contains("kicad-symbols/repository/tags") {
                Some(br#"[{"name":"11.0.0"},{"name":"10.0.10"},{"name":"10.0.9"},{"name":"10.0.9-rc1"},{"name":"10.0.8"}]"#.to_vec())
            } else if path.contains("kicad-footprints/repository/tags") {
                Some(br#"[{"name":"10.0.9"},{"name":"10.0.8"}]"#.to_vec())
            } else {
                ["10.0.8", "10.0.9"].iter().find_map(|v| {
                    if path.ends_with(&format!("kicad-symbols-{v}.tar.gz")) {
                        Some(archive("kicad-symbols", v, &[("Device.kicad_symdir/R.kicad_sym", v)]))
                    } else if path.ends_with(&format!("kicad-footprints-{v}.tar.gz")) {
                        Some(archive("kicad-footprints", v, &[("Resistor_SMD.pretty/R_0603.kicad_mod", v)]))
                    } else {
                        None
                    }
                })
            };
            let response = match body {
                Some(b) => [
                    format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", b.len()).into_bytes(),
                    b,
                ]
                .concat(),
                None => b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_vec(),
            };
            let _ = stream.write_all(&response);
        }
    });
    Source { base }
}

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap()
}

#[test]
fn download_update_cancel_and_failure() {
    let source = server();
    let root = tempfile::tempdir().unwrap();
    let root = root.path();
    assert_eq!(download::installed(root), None);

    // Newest stable 10.x release both repositories have.
    assert_eq!(download::latest_version(&source).unwrap(), "10.0.9");

    let mut seen = 0;
    let old = download::install(root, "10.0.8", &source, &mut |n| {
        seen = n;
        true
    })
    .unwrap();
    assert!(seen > 0, "progress is reported");
    assert_eq!(read(&old.symbols().join("Device.kicad_symdir/R.kicad_sym")), "10.0.8");
    assert_eq!(read(&old.footprints().join("Resistor_SMD.pretty/R_0603.kicad_mod")), "10.0.8");
    assert_eq!(download::installed(root), Some(old.clone()));
    assert_eq!(download::library_paths(Some(root)), old.paths());

    // A cancelled download leaves the installed version alone and nothing half-done.
    let cancelled = download::install(root, "10.0.9", &source, &mut |_| false);
    assert!(matches!(cancelled, Err(DownloadError::Cancelled)), "{cancelled:?}");
    // A missing release fails the same way.
    let missing = download::install(root, "10.0.10", &source, &mut |_| true);
    assert!(matches!(missing, Err(DownloadError::Http(_))), "{missing:?}");
    let names = |root: &Path| {
        let mut n: Vec<String> =
            std::fs::read_dir(root).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        n.sort();
        n
    };
    assert_eq!(names(root), ["10.0.8"]);

    // Without asking for an update, an existing download is used as it is.
    let (kept, fresh) = download::ensure(root, false, None, &source, &mut |_| true).unwrap();
    assert_eq!((kept.version.as_str(), fresh), ("10.0.8", false));

    // An update replaces the old version.
    let (new, fresh) = download::ensure(root, true, None, &source, &mut |_| true).unwrap();
    assert!(fresh);
    let (same, fresh) = download::ensure(root, true, None, &source, &mut |_| true).unwrap();
    assert_eq!((same.version.as_str(), fresh), ("10.0.9", false), "already up to date");
    assert_eq!(read(&new.symbols().join("Device.kicad_symdir/R.kicad_sym")), "10.0.9");
    assert_eq!(names(root), ["10.0.9"]);
    assert_eq!(download::installed(root), Some(new));
}

#[test]
#[ignore = "downloads ~24 MB from gitlab.com"]
fn real_gitlab_release() {
    let source = Source::default();
    let version = download::latest_version(&source).unwrap();
    assert!(version.starts_with(&format!("{}.", download::SUPPORTED_MAJOR)), "{version}");
    let root = tempfile::tempdir().unwrap();
    let installed = download::install(root.path(), &version, &source, &mut |_| true).unwrap();
    let catalog = kicad::Catalog::scan(&installed.paths());
    assert!(catalog.symbol_libs.len() > 100, "{} symbol libraries", catalog.symbol_libs.len());
    assert!(catalog.footprint_libs.len() > 100, "{} footprint libraries", catalog.footprint_libs.len());
}
