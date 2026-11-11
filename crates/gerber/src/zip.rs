//! A minimal zip writer (deflated files, no directories), enough for handing the
//! fabrication files to a manufacturer.

use std::io::Write;

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
