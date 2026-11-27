//! A minimal zip writer (deflated files, no directories), enough for handing the
//! fabrication files to a manufacturer, and a reader of what other tools write (stored
//! or deflated files; no zip64, no encryption).

use std::io::{Read, Write};

use flate2::Compression;
use flate2::write::DeflateEncoder;

/// Zip archive of `(name, content)` files.
pub fn zip(files: &[(String, Vec<u8>)]) -> std::io::Result<Vec<u8>> {
    let mut out = Vec::new();
    let mut central = Vec::new();
    // A fixed DOS date (1980-01-01): archives of the same files are identical.
    let (time, date) = (0u16, (1 << 5) | 1u16);
    for (name, data) in files {
        let mut enc = DeflateEncoder::new(Vec::new(), Compression::default());
        enc.write_all(data)?;
        let packed = enc.finish()?;
        let crc = crc32fast::hash(data);
        let offset = out.len() as u32;
        let common = |v: &mut Vec<u8>| {
            v.extend(20u16.to_le_bytes()); // version needed
            v.extend(0u16.to_le_bytes()); // flags
            v.extend(8u16.to_le_bytes()); // deflate
            v.extend(time.to_le_bytes());
            v.extend(date.to_le_bytes());
            v.extend(crc.to_le_bytes());
            v.extend((packed.len() as u32).to_le_bytes());
            v.extend((data.len() as u32).to_le_bytes());
            v.extend((name.len() as u16).to_le_bytes());
            v.extend(0u16.to_le_bytes()); // extra length
        };
        out.extend(0x0403_4b50u32.to_le_bytes());
        common(&mut out);
        out.extend(name.as_bytes());
        out.extend(&packed);

        central.extend(0x0201_4b50u32.to_le_bytes());
        central.extend(20u16.to_le_bytes()); // version made by
        common(&mut central);
        central.extend(0u16.to_le_bytes()); // comment length
        central.extend(0u16.to_le_bytes()); // disk
        central.extend(0u16.to_le_bytes()); // internal attributes
        central.extend(0u32.to_le_bytes()); // external attributes
        central.extend(offset.to_le_bytes());
        central.extend(name.as_bytes());
    }
    let start = out.len() as u32;
    out.extend(&central);
    out.extend(0x0605_4b50u32.to_le_bytes());
    out.extend(0u16.to_le_bytes());
    out.extend(0u16.to_le_bytes());
    out.extend((files.len() as u16).to_le_bytes());
    out.extend((files.len() as u16).to_le_bytes());
    out.extend((central.len() as u32).to_le_bytes());
    out.extend(start.to_le_bytes());
    out.extend(0u16.to_le_bytes());
    Ok(out)
}

/// The files of a zip archive, `(path, content)`, in the archive's order (folders, whose
/// names end in `/`, left out).
pub fn unzip(data: &[u8]) -> std::io::Result<Vec<(String, Vec<u8>)>> {
    let bad = |what: &str| std::io::Error::new(std::io::ErrorKind::InvalidData, format!("not a zip file: {what}"));
    let u16_at = |at: usize| data.get(at..at + 2).map(|b| u16::from_le_bytes([b[0], b[1]]) as usize);
    let u32_at = |at: usize| data.get(at..at + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as usize);
    // The end record is the last thing, after a comment of at most 64 kB.
    let end = (0..data.len().saturating_sub(21))
        .rev()
        .take(65_536 + 22)
        .find(|&i| u32_at(i) == Some(0x0605_4b50))
        .ok_or_else(|| bad("no end record"))?;
    let count = u16_at(end + 10).ok_or_else(|| bad("end record"))?;
    let mut at = u32_at(end + 16).ok_or_else(|| bad("end record"))?;
    let mut files = Vec::with_capacity(count);
    for _ in 0..count {
        if u32_at(at) != Some(0x0201_4b50) {
            return Err(bad("central directory"));
        }
        let field = |offset: usize, size: usize| {
            if size == 2 { u16_at(at + offset) } else { u32_at(at + offset) }.ok_or_else(|| bad("central directory"))
        };
        let (flags, method) = (field(8, 2)?, field(10, 2)?);
        let (packed_len, len) = (field(20, 4)?, field(24, 4)?);
        let (name_len, extra_len, comment_len) = (field(28, 2)?, field(30, 2)?, field(32, 2)?);
        let local = field(42, 4)?;
        let name = data.get(at + 46..at + 46 + name_len).ok_or_else(|| bad("name"))?;
        let name = String::from_utf8_lossy(name).replace('\\', "/");
        at += 46 + name_len + extra_len + comment_len;
        if name.ends_with('/') {
            continue;
        }
        if flags & 1 != 0 {
            return Err(bad("encrypted"));
        }
        if u32_at(local) != Some(0x0403_4b50) {
            return Err(bad("local header"));
        }
        let start = local + 30 + u16_at(local + 26).unwrap_or(0) + u16_at(local + 28).unwrap_or(0);
        let packed = data.get(start..start + packed_len).ok_or_else(|| bad("truncated"))?;
        let content = match method {
            0 => packed.to_vec(),
            8 => {
                let mut out = Vec::with_capacity(len);
                flate2::read::DeflateDecoder::new(packed).read_to_end(&mut out)?;
                out
            }
            m => return Err(bad(&format!("compression method {m}"))),
        };
        files.push((name, content));
    }
    Ok(files)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_is_zipped_unzips() {
        let files = vec![
            ("a/board.json".to_string(), b"{}".repeat(500)),
            ("empty".to_string(), Vec::new()),
            ("b.txt".to_string(), "Grüße".as_bytes().to_vec()),
        ];
        assert_eq!(unzip(&zip(&files).unwrap()).unwrap(), files);
    }

    #[test]
    fn stored_files_and_folders() {
        // What `zip -0` writes: a folder entry and a stored file, with a comment.
        let mut archive = Vec::new();
        let mut central = Vec::new();
        for (name, data) in [("p/", &b""[..]), ("p/x", &b"hi"[..])] {
            let offset = archive.len() as u32;
            let head = |v: &mut Vec<u8>| {
                v.extend(10u16.to_le_bytes());
                v.extend(0u16.to_le_bytes());
                v.extend(0u16.to_le_bytes()); // stored
                v.extend([0; 4]);
                v.extend(crc32fast::hash(data).to_le_bytes());
                v.extend((data.len() as u32).to_le_bytes());
                v.extend((data.len() as u32).to_le_bytes());
                v.extend((name.len() as u16).to_le_bytes());
                v.extend(0u16.to_le_bytes());
            };
            archive.extend(0x0403_4b50u32.to_le_bytes());
            head(&mut archive);
            archive.extend(name.as_bytes());
            archive.extend(data);
            central.extend(0x0201_4b50u32.to_le_bytes());
            central.extend(10u16.to_le_bytes());
            head(&mut central);
            central.extend([0; 10]); // comment length, disk, attributes
            central.extend(offset.to_le_bytes());
            central.extend(name.as_bytes());
        }
        let start = archive.len() as u32;
        archive.extend(&central);
        archive.extend(0x0605_4b50u32.to_le_bytes());
        archive.extend([0; 4]);
        archive.extend(2u16.to_le_bytes());
        archive.extend(2u16.to_le_bytes());
        archive.extend((central.len() as u32).to_le_bytes());
        archive.extend(start.to_le_bytes());
        archive.extend(3u16.to_le_bytes());
        archive.extend(b"hey");
        assert_eq!(unzip(&archive).unwrap(), vec![("p/x".to_string(), b"hi".to_vec())]);
    }

    #[test]
    fn other_files_are_refused() {
        assert!(unzip(b"PK not really a zip file at all, no").is_err());
        assert!(unzip(b"").is_err());
    }
}
