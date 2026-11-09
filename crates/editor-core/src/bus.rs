//! Bus names (KiCad syntax). A label whose name is a bus name names a bus instead of a
//! net:
//!
//! * vector: `D[0..7]` → `D0` … `D7` (ranges may count down: `A[7..0]`);
//! * group: `{SDA SCL}` → `SDA`, `SCL`;
//! * named group: `I2C{SDA SCL}` → `I2C.SDA`, `I2C.SCL`.
//!
//! Group members may be vectors (`{D[0..3] CLK}`).

/// Vectors longer than this are not bus names (guards against `D[0..999999999]`).
pub const MAX_MEMBERS: usize = 1024;

/// Members of a vector like `D[0..7]`, or `None` if `s` isn't one.
fn vector(s: &str) -> Option<Vec<String>> {
    let open = s.find('[')?;
    let inner = s.strip_suffix(']')?.get(open + 1..)?;
    let prefix = &s[..open];
    if prefix.is_empty() || prefix.contains(|c: char| c.is_whitespace() || "[]{}".contains(c)) {
        return None;
    }
    let (a, b) = inner.split_once("..")?;
    let (a, b): (u32, u32) = (a.trim().parse().ok()?, b.trim().parse().ok()?);
    if a.abs_diff(b) as usize >= MAX_MEMBERS {
        return None;
    }
    let range: Vec<u32> = if a <= b { (a..=b).collect() } else { (b..=a).rev().collect() };
    Some(range.into_iter().map(|i| format!("{prefix}{i}")).collect())
}

/// Members of `s` if it is a bus name, in order and without duplicates.
pub fn members(s: &str) -> Option<Vec<String>> {
    let s = s.trim();
    let mut out: Vec<String> = if let Some(open) = s.find('{') {
        let inner = s.strip_suffix('}')?.get(open + 1..)?;
        let prefix = &s[..open];
        if prefix.contains(|c: char| c.is_whitespace() || "[]{}".contains(c)) || inner.contains(['{', '}']) {
            return None;
        }
        let mut out = Vec::new();
        for m in inner.split(|c: char| c.is_whitespace() || c == ',').filter(|m| !m.is_empty()) {
            let names = vector(m).unwrap_or_else(|| vec![m.to_string()]);
            if names.iter().any(|n| n.contains(['[', ']'])) {
                return None;
            }
            out.extend(names.into_iter().map(|n| if prefix.is_empty() { n } else { format!("{prefix}.{n}") }));
        }
        if out.is_empty() || out.len() > MAX_MEMBERS {
            return None;
        }
        out
    } else {
        vector(s)?
    };
    let mut seen = std::collections::HashSet::new();
    out.retain(|m| seen.insert(m.clone()));
    Some(out)
}

/// Does `s` name a bus (rather than a net)?
pub fn is_bus_name(s: &str) -> bool {
    members(s).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(s: &str) -> Option<Vec<String>> {
        members(s)
    }

    #[test]
    fn vectors() {
        assert_eq!(m("D[0..3]").unwrap(), ["D0", "D1", "D2", "D3"]);
        assert_eq!(m("A[2..0]").unwrap(), ["A2", "A1", "A0"]);
        assert_eq!(m("ADDR[5..5]").unwrap(), ["ADDR5"]);
        assert_eq!(m("D[0..99999]"), None, "too long");
        for not in ["D", "[0..3]", "D[0..x]", "D[0-3]", "D[0..3]x", "GND", "+3V3", "Net-(R1-1)"] {
            assert_eq!(m(not), None, "{not}");
        }
    }

    #[test]
    fn groups() {
        assert_eq!(m("{SDA SCL}").unwrap(), ["SDA", "SCL"]);
        assert_eq!(m("I2C{SDA SCL}").unwrap(), ["I2C.SDA", "I2C.SCL"]);
        assert_eq!(m("{D[0..1] CLK}").unwrap(), ["D0", "D1", "CLK"]);
        assert_eq!(m("MEM{A[1..0] WE}").unwrap(), ["MEM.A1", "MEM.A0", "MEM.WE"]);
        assert_eq!(m("{A A B}").unwrap(), ["A", "B"], "duplicates dropped");
        for not in ["{}", "{A", "A}", "{A {B}}", "X Y{A}", "{A}x"] {
            assert_eq!(m(not), None, "{not}");
        }
    }
}
