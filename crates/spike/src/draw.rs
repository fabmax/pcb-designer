//! Turns schematic and board objects into render primitives.

use render::font::{self, HAlign, VAlign};
use render::{Color, Instance, LayerId, Scene};

use crate::model::{BoardFootprint, CopperLayer, Label, PadShape, PartDef, Pt, SchComponent, Track, Via, Wire};

pub type Prims = Vec<(LayerId, Instance)>;

pub mod colors {
    use render::Color;

    pub const SCH_WIRE: Color = Color::rgb(0x5f, 0xd3, 0xa0);
    pub const SCH_BODY: Color = Color::rgb(0xe0, 0xa0, 0x60);
    pub const SCH_BODY_FILL: Color = Color::rgba(0xe0, 0xa0, 0x60, 0x1c);
    pub const SCH_PIN: Color = Color::rgb(0xd0, 0x70, 0x70);
    pub const SCH_LABEL: Color = Color::rgb(0xf0, 0xd0, 0x70);
    pub const SCH_TEXT: Color = Color::rgb(0x9f, 0xb4, 0xff);

    pub const F_CU: Color = Color::rgb(0xc8, 0x3c, 0x3c);
    pub const B_CU: Color = Color::rgb(0x3d, 0x6f, 0xb8);
    pub const VIA: Color = Color::rgb(0xa8, 0xa8, 0xa8);
    pub const HOLE: Color = Color::rgb(0x10, 0x11, 0x14);
    pub const SILK: Color = Color::rgb(0xe8, 0xe4, 0xc8);
    pub const EDGE: Color = Color::rgb(0xe6, 0xc8, 0x4f);
    pub const RATSNEST: Color = Color::rgba(0xe6, 0xe6, 0xf0, 0xb0);
}

pub struct SchLayers {
    pub body_fill: LayerId,
    pub body: LayerId,
    pub pins: LayerId,
    pub wires: LayerId,
    pub labels: LayerId,
    pub text: LayerId,
}

impl SchLayers {
    pub fn create(scene: &mut Scene) -> Self {
        let l = Self {
            body_fill: scene.add_layer("Body fill"),
            body: scene.add_layer("Symbols"),
            pins: scene.add_layer("Pins"),
            wires: scene.add_layer("Wires"),
            labels: scene.add_layer("Labels"),
            text: scene.add_layer("Text"),
        };
        scene.layer_mut(l.labels).min_zoom = 2.5;
        scene.layer_mut(l.text).min_zoom = 2.0;
        l
    }
}

pub struct PcbLayers {
    pub b_cu: LayerId,
    pub f_cu: LayerId,
    pub vias: LayerId,
    pub holes: LayerId,
    pub silk: LayerId,
    pub silk_text: LayerId,
    pub edge: LayerId,
    pub ratsnest: LayerId,
}

impl PcbLayers {
    pub fn create(scene: &mut Scene) -> Self {
        let l = Self {
            b_cu: scene.add_layer("B.Cu"),
            f_cu: scene.add_layer("F.Cu"),
            vias: scene.add_layer("Vias"),
            holes: scene.add_layer("Holes"),
            silk: scene.add_layer("F.Silkscreen"),
            silk_text: scene.add_layer("F.Silkscreen text"),
            edge: scene.add_layer("Edge.Cuts"),
            ratsnest: scene.add_layer("Ratsnest"),
        };
        scene.layer_mut(l.silk_text).min_zoom = 6.0;
        l
    }
}

/// Item id namespaces.
pub mod ids {
    const SHIFT: u32 = 56;
    const MASK: u64 = (1 << SHIFT) - 1;
    pub fn component(id: u32) -> u64 {
        1 << SHIFT | id as u64
    }
    pub fn wire(id: u32) -> u64 {
        2 << SHIFT | id as u64
    }
    pub fn label(id: u32) -> u64 {
        3 << SHIFT | id as u64
    }
    pub fn footprint(id: u32) -> u64 {
        1 << SHIFT | id as u64
    }
    pub fn track(id: u32) -> u64 {
        2 << SHIFT | id as u64
    }
    pub fn via(id: u32) -> u64 {
        3 << SHIFT | id as u64
    }
    pub fn ratsnest(hash: u64) -> u64 {
        4 << SHIFT | (hash & MASK)
    }
    pub fn outline() -> u64 {
        5 << SHIFT
    }
}

#[allow(clippy::too_many_arguments)] // spike-only helper
fn text(out: &mut Prims, layer: LayerId, s: &str, at: [f32; 2], height: f32, h: HAlign, v: VAlign, color: Color) {
    let w = height * 0.14;
    font::text_segments(s, at, height, 0.0, h, v, |a, b| {
        out.push((layer, Instance::line(a, b, w, color)))
    });
}

const SCH_LINE: f32 = 0.15;

pub fn sch_component(part: &PartDef, c: &SchComponent, l: &SchLayers, out: &mut Prims) {
    let xf = |p: Pt| (c.pos + p.rotated(c.rot)).to_mm();
    let (lo, hi) = part.body;
    let corners = [lo, Pt::new(hi.x, lo.y), hi, Pt::new(lo.x, hi.y)].map(xf);
    let center = xf(Pt::new((lo.x + hi.x) / 2, (lo.y + hi.y) / 2));
    let size = (hi - lo).rotated(c.rot).to_mm();
    out.push((
        l.body_fill,
        Instance::rect(center, [size[0].abs(), size[1].abs()], 0.0, 0.0, colors::SCH_BODY_FILL),
    ));
    for i in 0..4 {
        out.push((
            l.body,
            Instance::line(corners[i], corners[(i + 1) % 4], 0.2, colors::SCH_BODY),
        ));
    }
    for pin in &part.pins {
        out.push((
            l.pins,
            Instance::line(xf(pin.inner), xf(pin.pos), SCH_LINE, colors::SCH_PIN),
        ));
    }
    let top = corners.iter().map(|p| p[1]).fold(f32::MIN, f32::max);
    text(
        out,
        l.text,
        &c.refdes,
        [center[0], top + 0.6],
        1.5,
        HAlign::Center,
        VAlign::Bottom,
        colors::SCH_TEXT,
    );
}

pub fn sch_wire(w: &Wire, l: &SchLayers, out: &mut Prims) {
    out.push((
        l.wires,
        Instance::line(w.a.to_mm(), w.b.to_mm(), SCH_LINE, colors::SCH_WIRE),
    ));
}

pub fn sch_label(lab: &Label, l: &SchLayers, out: &mut Prims) {
    let p = lab.pos.to_mm();
    out.push((l.wires, Instance::circle(p, 0.25, colors::SCH_LABEL)));
    text(
        out,
        l.labels,
        &lab.name,
        [p[0] + 0.5, p[1] + 0.3],
        1.27,
        HAlign::Left,
        VAlign::Bottom,
        colors::SCH_LABEL,
    );
}

pub fn pcb_footprint(part: &PartDef, fp: &BoardFootprint, refdes: &str, l: &PcbLayers, out: &mut Prims) {
    let xf = |p: Pt| (fp.pos + p.rotated(fp.rot)).to_mm();
    let mut top = f32::MIN;
    for pad in &part.pads {
        let center = xf(pad.pos);
        let mut size = pad.size.to_mm();
        if fp.rot & 1 == 1 {
            size.swap(0, 1);
        }
        top = top.max(center[1] + size[1] * 0.5);
        let mut push = |layer: LayerId, color: Color| {
            let inst = match pad.shape {
                PadShape::Rect => Instance::rect(center, size, 0.0, 0.0, color),
                PadShape::RoundRect(r) => Instance::rect(center, size, r * size[0].min(size[1]), 0.0, color),
                PadShape::Circle => Instance::circle(center, size[0] * 0.5, color),
            };
            out.push((layer, inst));
        };
        push(l.f_cu, colors::F_CU);
        if pad.drill > 0 {
            push(l.b_cu, colors::B_CU);
            out.push((
                l.holes,
                Instance::circle(center, pad.drill as f32 * 0.5e-6, colors::HOLE),
            ));
        }
    }
    for (a, b) in &part.silk {
        let (a, b) = (xf(*a), xf(*b));
        top = top.max(a[1]).max(b[1]);
        out.push((l.silk, Instance::line(a, b, 0.12, colors::SILK)));
    }
    for (p, r) in &part.silk_dots {
        out.push((l.silk, Instance::circle(xf(*p), *r as f32 * 1e-6, colors::SILK)));
    }
    let c = fp.pos.to_mm();
    text(
        out,
        l.silk_text,
        refdes,
        [c[0], top + 0.3],
        0.8,
        HAlign::Center,
        VAlign::Bottom,
        colors::SILK,
    );
}

pub fn pcb_track(t: &Track, l: &PcbLayers, out: &mut Prims) {
    let (layer, color) = match t.layer {
        CopperLayer::Top => (l.f_cu, colors::F_CU),
        CopperLayer::Bottom => (l.b_cu, colors::B_CU),
    };
    out.push((
        layer,
        Instance::line(t.a.to_mm(), t.b.to_mm(), t.width as f32 * 1e-6, color),
    ));
}

pub fn pcb_via(v: &Via, l: &PcbLayers, out: &mut Prims) {
    let p = v.pos.to_mm();
    out.push((l.vias, Instance::circle(p, v.diameter as f32 * 0.5e-6, colors::VIA)));
    out.push((l.holes, Instance::circle(p, v.drill as f32 * 0.5e-6, colors::HOLE)));
}

pub fn pcb_outline(lo: Pt, hi: Pt, l: &PcbLayers, out: &mut Prims) {
    let c = [lo, Pt::new(hi.x, lo.y), hi, Pt::new(lo.x, hi.y)].map(Pt::to_mm);
    for i in 0..4 {
        out.push((l.edge, Instance::line(c[i], c[(i + 1) % 4], 0.15, colors::EDGE)));
    }
}

pub fn ratsnest_line(a: [f32; 2], b: [f32; 2], l: &PcbLayers, out: &mut Prims) {
    out.push((l.ratsnest, Instance::line(a, b, 0.04, colors::RATSNEST)));
}
