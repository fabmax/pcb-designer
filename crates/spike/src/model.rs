//! Provisional data model for the milestone-0 spike. Will be replaced by the `model` crate.

use std::ops::{Add, Sub};

use rustc_hash::FxHashMap;

/// Length in nanometers.
pub type Nm = i64;
pub const MM: Nm = 1_000_000;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default, PartialOrd, Ord)]
pub struct Pt {
    pub x: Nm,
    pub y: Nm,
}

impl Pt {
    pub const fn new(x: Nm, y: Nm) -> Self {
        Self { x, y }
    }

    pub fn mm(x: f64, y: f64) -> Self {
        Self {
            x: (x * MM as f64).round() as Nm,
            y: (y * MM as f64).round() as Nm,
        }
    }

    pub fn to_mm(self) -> [f32; 2] {
        [(self.x as f64 / MM as f64) as f32, (self.y as f64 / MM as f64) as f32]
    }

    pub fn to_mm64(self) -> [f64; 2] {
        [self.x as f64 / MM as f64, self.y as f64 / MM as f64]
    }

    /// Rotate by `rot` quarter turns counterclockwise around the origin.
    pub fn rotated(self, rot: u8) -> Pt {
        match rot & 3 {
            0 => self,
            1 => Pt::new(-self.y, self.x),
            2 => Pt::new(-self.x, -self.y),
            _ => Pt::new(self.y, -self.x),
        }
    }
}

impl Add for Pt {
    type Output = Pt;
    fn add(self, o: Pt) -> Pt {
        Pt::new(self.x + o.x, self.y + o.y)
    }
}

impl Sub for Pt {
    type Output = Pt;
    fn sub(self, o: Pt) -> Pt {
        Pt::new(self.x - o.x, self.y - o.y)
    }
}

/// Vec with stable indices and slot reuse.
#[derive(Clone, Debug)]
pub struct Slab<T> {
    items: Vec<Option<T>>,
    free: Vec<u32>,
    len: usize,
}

impl<T> Default for Slab<T> {
    fn default() -> Self {
        Self {
            items: Vec::new(),
            free: Vec::new(),
            len: 0,
        }
    }
}

impl<T> Slab<T> {
    pub fn insert(&mut self, t: T) -> u32 {
        self.len += 1;
        if let Some(id) = self.free.pop() {
            self.items[id as usize] = Some(t);
            id
        } else {
            self.items.push(Some(t));
            (self.items.len() - 1) as u32
        }
    }

    pub fn remove(&mut self, id: u32) -> Option<T> {
        let t = self.items.get_mut(id as usize)?.take();
        if t.is_some() {
            self.len -= 1;
            self.free.push(id);
        }
        t
    }

    pub fn get(&self, id: u32) -> Option<&T> {
        self.items.get(id as usize)?.as_ref()
    }

    pub fn get_mut(&mut self, id: u32) -> Option<&mut T> {
        self.items.get_mut(id as usize)?.as_mut()
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Upper bound for ids (for random sampling).
    pub fn capacity_ids(&self) -> u32 {
        self.items.len() as u32
    }

    pub fn iter(&self) -> impl Iterator<Item = (u32, &T)> {
        self.items
            .iter()
            .enumerate()
            .filter_map(|(i, t)| t.as_ref().map(|t| (i as u32, t)))
    }
}

// ---------------------------------------------------------------------------------------
// Library

#[derive(Clone, Copy, Debug)]
pub enum PadShape {
    Rect,
    /// Corner radius as a fraction of the smaller side.
    RoundRect(f32),
    Circle,
}

#[derive(Clone, Debug)]
pub struct PadDef {
    pub pos: Pt,
    pub size: Pt,
    pub shape: PadShape,
    /// Drill diameter, 0 for SMD pads.
    pub drill: Nm,
}

#[derive(Clone, Debug)]
pub struct PinDef {
    /// Connection point (outer end of the pin).
    pub pos: Pt,
    /// Where the pin meets the symbol body.
    pub inner: Pt,
}

#[derive(Clone, Debug)]
pub struct PartDef {
    pub name: String,
    pub prefix: String,
    /// Symbol body rectangle (min, max).
    pub body: (Pt, Pt),
    pub pins: Vec<PinDef>,
    pub pads: Vec<PadDef>,
    pub silk: Vec<(Pt, Pt)>,
    pub silk_dots: Vec<(Pt, Nm)>,
    /// Pad index for every pin.
    pub pin_to_pad: Vec<u16>,
    /// Footprint placement pitch used by the generator.
    pub board_pitch: Pt,
}

// ---------------------------------------------------------------------------------------
// Schematic

#[derive(Clone, Debug)]
pub struct SchComponent {
    pub part: u16,
    pub refdes: String,
    pub pos: Pt,
    pub rot: u8,
}

#[derive(Clone, Copy, Debug)]
pub struct Wire {
    pub a: Pt,
    pub b: Pt,
}

#[derive(Clone, Debug)]
pub struct Label {
    pub pos: Pt,
    pub name: String,
}

#[derive(Default)]
pub struct Schematic {
    pub components: Slab<SchComponent>,
    pub wires: Slab<Wire>,
    pub labels: Slab<Label>,
}

// ---------------------------------------------------------------------------------------
// Board

#[derive(Clone, Copy, Debug)]
pub struct BoardFootprint {
    pub pos: Pt,
    pub rot: u8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CopperLayer {
    Top,
    Bottom,
}

#[derive(Clone, Copy, Debug)]
pub struct Track {
    pub a: Pt,
    pub b: Pt,
    pub width: Nm,
    pub layer: CopperLayer,
}

#[derive(Clone, Copy, Debug)]
pub struct Via {
    pub pos: Pt,
    pub diameter: Nm,
    pub drill: Nm,
}

#[derive(Default)]
pub struct Board {
    /// Keyed by schematic component id.
    pub footprints: FxHashMap<u32, BoardFootprint>,
    pub tracks: Slab<Track>,
    pub vias: Slab<Via>,
    pub outline: (Pt, Pt),
}
