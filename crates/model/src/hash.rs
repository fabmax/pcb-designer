//! 64-bit FNV-1a: a small hash that is stable across platforms and Rust versions, for
//! content hashes that are stored in files.

/// Streaming FNV-1a. Implements [`std::io::Write`], so a value can be hashed by
/// serializing straight into it.
#[derive(Clone, Copy, Debug)]
pub struct Fnv1a(u64);

impl Default for Fnv1a {
    fn default() -> Self {
        Fnv1a(0xcbf2_9ce4_8422_2325)
    }
}

impl Fnv1a {
    pub fn update(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.0 = (self.0 ^ b as u64).wrapping_mul(0x0000_0100_0000_01b3);
        }
    }

    pub fn finish(self) -> u64 {
        self.0
    }

    /// The hash of `value`'s JSON form, as 16 hex digits.
    pub fn of_json<T: serde::Serialize + ?Sized>(value: &T) -> serde_json::Result<String> {
        let mut h = Fnv1a::default();
        serde_json::to_writer(&mut h, value)?;
        Ok(format!("{:016x}", h.finish()))
    }
}

impl std::io::Write for Fnv1a {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.update(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::Fnv1a;

    #[test]
    fn matches_reference_values() {
        assert_eq!(Fnv1a::default().finish(), 0xcbf2_9ce4_8422_2325);
        let mut h = Fnv1a::default();
        h.update(b"a");
        assert_eq!(h.finish(), 0xaf63_dc4c_8601_ec8c);
        let mut h = Fnv1a::default();
        h.update(b"foo");
        h.update(b"bar");
        assert_eq!(h.finish(), 0x85944171f73967e8);
        assert_eq!(Fnv1a::of_json("x").unwrap(), {
            let mut h = Fnv1a::default();
            h.update(b"\"x\"");
            format!("{:016x}", h.finish())
        });
    }
}
