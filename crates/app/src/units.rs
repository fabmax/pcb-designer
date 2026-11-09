//! Display units for lengths (mm, mil, inch). The model is always integer nanometres;
//! the unit only changes how lengths are shown and typed. The active unit is global, like
//! the theme, and saved in the settings.

use std::sync::atomic::{AtomicU8, Ordering};

use model::{Coord, Point};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Unit {
    #[default]
    Mm,
    Mil,
    Inch,
}

impl Unit {
    pub const ALL: [Unit; 3] = [Unit::Mm, Unit::Mil, Unit::Inch];

    pub fn label(self) -> &'static str {
        match self {
            Unit::Mm => "mm",
            Unit::Mil => "mil",
            Unit::Inch => "in",
        }
    }

    /// Nanometres per unit.
    pub fn nm(self) -> f64 {
        match self {
            Unit::Mm => 1e6,
            Unit::Mil => 25_400.0,
            Unit::Inch => 25.4e6,
        }
    }

    /// Decimals worth showing (about 0.1 µm).
    pub fn decimals(self) -> usize {
        match self {
            Unit::Mm => 4,
            Unit::Mil => 2,
            Unit::Inch => 5,
        }
    }

    /// `v` nanometres in this unit.
    pub fn value(self, v: Coord) -> f64 {
        v as f64 / self.nm()
    }

    pub fn to_nm(self, v: f64) -> Coord {
        (v * self.nm()).round() as Coord
    }
}

static UNIT: AtomicU8 = AtomicU8::new(0);

pub fn set_unit(unit: Unit) {
    UNIT.store(unit as u8, Ordering::Relaxed);
}

/// The active display unit.
pub fn unit() -> Unit {
    match UNIT.load(Ordering::Relaxed) {
        1 => Unit::Mil,
        2 => Unit::Inch,
        _ => Unit::Mm,
    }
}

/// A number with up to `decimals` decimals, without trailing zeros.
fn trim(v: f64, decimals: usize) -> String {
    let s = format!("{v:.decimals$}");
    if s.contains('.') { s.trim_end_matches('0').trim_end_matches('.').to_string() } else { s }
}

/// "0.25 mm", "10 mil", "0.01 in" in the active unit.
pub fn fmt(v: Coord) -> String {
    let u = unit();
    format!("{} {}", trim(u.value(v), u.decimals()), u.label())
}

/// "1.27, -2.54 mm" in the active unit.
pub fn fmt_point(p: Point) -> String {
    let u = unit();
    format!("{}, {} {}", trim(u.value(p.x), u.decimals()), trim(u.value(p.y), u.decimals()), u.label())
}

/// A length edited by dragging or typing in the active unit, at least `min` (nm).
/// Returns the new value.
pub fn drag(ui: &mut egui::Ui, value: Coord, min: Coord, max: Coord) -> (egui::Response, Option<Coord>) {
    let u = unit();
    let mut v = u.value(value);
    // About 0.05 mm per point of dragging, whatever the unit.
    let speed = 50_000.0 / u.nm();
    let r = ui.add(
        egui::DragValue::new(&mut v)
            .speed(speed)
            .range(u.value(min)..=u.value(max))
            .suffix(format!(" {}", u.label()))
            .max_decimals(u.decimals()),
    );
    let changed = r.changed().then(|| u.to_nm(v));
    (r, changed)
}

/// Kinds of editors with their own grid presets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GridKind {
    /// Schematics and symbols: the 100 mil grid and its halves, whatever the unit.
    Schematic,
    /// Boards and footprints: round numbers in the active unit.
    Board,
}

impl GridKind {
    /// Grid presets in nm, coarse to fine.
    pub fn presets(self, unit: Unit) -> Vec<Coord> {
        let mil = |v: f64| Unit::Mil.to_nm(v);
        match (self, unit) {
            (GridKind::Schematic, _) => vec![mil(100.0), mil(50.0), mil(25.0)],
            (GridKind::Board, Unit::Mm) => [1.0, 0.5, 0.25, 0.1, 0.05].map(|v| Unit::Mm.to_nm(v)).to_vec(),
            (GridKind::Board, Unit::Mil | Unit::Inch) => [50.0, 25.0, 10.0, 5.0, 1.0].map(mil).to_vec(),
        }
    }

    /// `grid` if it is a preset of `unit`, else the closest preset (by ratio).
    pub fn nearest_preset(self, grid: Coord, unit: Unit) -> Coord {
        let presets = self.presets(unit);
        if presets.contains(&grid) {
            return grid;
        }
        let off = |p: Coord| (p as f64 / grid.max(1) as f64).ln().abs();
        presets.into_iter().min_by(|a, b| off(*a).total_cmp(&off(*b))).unwrap_or(grid)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conversions_and_formatting() {
        assert_eq!(Unit::Mil.to_nm(1.0), 25_400);
        assert_eq!(Unit::Inch.to_nm(0.1), 2_540_000);
        assert_eq!(Unit::Mm.value(1_270_000), 1.27);
        set_unit(Unit::Mil);
        assert_eq!(fmt(2_540_000), "100 mil");
        assert_eq!(fmt(127_000), "5 mil");
        set_unit(Unit::Inch);
        assert_eq!(fmt(1_270_000), "0.05 in");
        assert_eq!(fmt_point(Point::new(2_540_000, -25_400_000)), "0.1, -1 in");
        set_unit(Unit::Mm);
        assert_eq!(fmt(250_000), "0.25 mm");
        assert_eq!(fmt(-1_000_000), "-1 mm");
        assert_eq!(GridKind::Board.presets(Unit::Inch)[0], 1_270_000);
        // Switching units moves the grid to a round value of the new unit.
        assert_eq!(GridKind::Board.nearest_preset(500_000, Unit::Mil), Unit::Mil.to_nm(25.0));
        assert_eq!(GridKind::Board.nearest_preset(Unit::Mil.to_nm(10.0), Unit::Mm), 250_000);
        assert_eq!(GridKind::Schematic.nearest_preset(1_270_000, Unit::Mm), 1_270_000);
    }
}
