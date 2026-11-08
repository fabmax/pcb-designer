//! Length units. Internally everything is an integer number of nanometers.

/// Length in nanometers.
pub type Coord = i64;

pub const NM: Coord = 1;
pub const UM: Coord = 1_000;
pub const MM: Coord = 1_000_000;
pub const MIL: Coord = 25_400;
pub const INCH: Coord = 25_400_000;

/// Millimeters → nanometers (rounded).
pub fn mm(v: f64) -> Coord {
    (v * MM as f64).round() as Coord
}

/// Mils (thousandths of an inch) → nanometers (rounded).
pub fn mil(v: f64) -> Coord {
    (v * MIL as f64).round() as Coord
}

/// Nanometers → millimeters.
pub fn to_mm(c: Coord) -> f64 {
    c as f64 / MM as f64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conversions() {
        assert_eq!(mm(2.54), 100 * MIL);
        assert_eq!(mil(100.0), 2_540_000);
        assert_eq!(to_mm(1_500_000), 1.5);
    }
}
