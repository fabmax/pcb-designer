//! Colors and line widths of the canvases. Two palettes (dark, light); the active one is
//! global ([`set_theme`]) and baked into the scene when objects are drawn, so views are
//! rebuilt after a change ([`crate::Document::restyle`]).

use std::sync::atomic::{AtomicU8, Ordering};

use model::{Coord, LayerId};
use render::Color;
use render::gpu::Background;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Theme {
    #[default]
    Dark,
    Light,
}

impl Theme {
    pub const ALL: &[Theme] = &[Theme::Dark, Theme::Light];

    pub fn label(self) -> &'static str {
        match self {
            Theme::Dark => "Dark",
            Theme::Light => "Light",
        }
    }
}

/// Canvas colors.
#[derive(Clone, Debug)]
pub struct Palette {
    pub background: Color,
    pub grid: Color,
    pub axis: Color,

    pub sch_wire: Color,
    pub sch_body: Color,
    pub sch_body_fill: Color,
    pub sch_pin: Color,
    pub sch_pin_text: Color,
    pub sch_label: Color,
    pub sch_power: Color,
    pub sch_text: Color,
    pub sch_junction: Color,

    pub selection: Color,
    pub selection_fill: Color,
    /// Ratsnest lines (unrouted connections).
    pub ratsnest: Color,
    /// Halo around flagged tracks and vias (shorts, unconnected copper).
    pub flagged: Color,
    /// Design rule violation markers.
    pub drc_marker: Color,
    /// Copper of highlighted nets.
    pub net_highlight: Color,
    /// Objects selected in the other view (cross-probing).
    pub probe: Color,
    pub probe_fill: Color,
    pub hover: Color,
    /// What a tool would create (library editors).
    pub preview: Color,
    /// Item origin cross (library editors).
    pub origin: Color,
    /// Pad numbers.
    pub pad_text: Color,

    pub via: Color,
    pub hole: Color,

    pub top_copper: Color,
    pub bottom_copper: Color,
    pub inner_copper: [Color; 4],
    pub top_silk: Color,
    pub bottom_silk: Color,
    pub top_fab: Color,
    pub bottom_fab: Color,
    pub top_courtyard: Color,
    pub bottom_courtyard: Color,
    pub mask: Color,
    pub paste: Color,
    pub outline: Color,
    pub other_layer: Color,
}

pub static DARK: Palette = Palette {
    background: Color::rgb(0x16, 0x18, 0x1d),
    grid: Color::rgba(0x5a, 0x60, 0x70, 0xb0),
    axis: Color::rgba(0x80, 0x88, 0x98, 0x90),

    sch_wire: Color::rgb(0x5f, 0xd3, 0xa0),
    sch_body: Color::rgb(0xe0, 0xa0, 0x60),
    sch_body_fill: Color::rgba(0xe0, 0xa0, 0x60, 0x22),
    sch_pin: Color::rgb(0xd0, 0x70, 0x70),
    sch_pin_text: Color::rgb(0x9a, 0x9f, 0xae),
    sch_label: Color::rgb(0xf0, 0xd0, 0x70),
    sch_power: Color::rgb(0xe0, 0x80, 0x80),
    sch_text: Color::rgb(0x9f, 0xb4, 0xff),
    sch_junction: Color::rgb(0x5f, 0xd3, 0xa0),

    selection: Color::rgba(0x4c, 0x9a, 0xff, 0xe0),
    selection_fill: Color::rgba(0x4c, 0x9a, 0xff, 0x28),
    ratsnest: Color::rgba(0xe8, 0xe8, 0xf0, 0xb0),
    flagged: Color::rgba(0xff, 0x9a, 0x2e, 0x70),
    drc_marker: Color::rgba(0xff, 0x4d, 0x6a, 0xe0),
    net_highlight: Color::rgba(0xff, 0xff, 0xff, 0x60),
    probe: Color::rgba(0xff, 0xd0, 0x4c, 0xd0),
    probe_fill: Color::rgba(0xff, 0xd0, 0x4c, 0x22),
    hover: Color::rgba(0xff, 0xff, 0xff, 0x50),
    preview: Color::rgba(0x4c, 0x9a, 0xff, 0xc0),
    origin: Color::rgba(0x9a, 0xa0, 0xb0, 0x90),
    pad_text: Color::rgba(0xff, 0xff, 0xff, 0xd0),

    via: Color::rgb(0xa8, 0xa8, 0xa8),
    hole: Color::rgb(0x10, 0x11, 0x14),

    top_copper: Color::rgb(0xc8, 0x3c, 0x3c),
    bottom_copper: Color::rgb(0x3d, 0x6f, 0xb8),
    inner_copper: [
        Color::rgb(0xc8, 0xa0, 0x3c),
        Color::rgb(0x3c, 0xb8, 0x8c),
        Color::rgb(0xb0, 0x5c, 0xc8),
        Color::rgb(0x8c, 0xc8, 0x3c),
    ],
    top_silk: Color::rgb(0xe8, 0xe4, 0xc8),
    bottom_silk: Color::rgb(0xb8, 0xa0, 0xe0),
    top_fab: Color::rgb(0x8c, 0x8f, 0xa0),
    bottom_fab: Color::rgb(0x60, 0x64, 0x78),
    top_courtyard: Color::rgba(0xd0, 0x80, 0xd0, 0xb0),
    bottom_courtyard: Color::rgba(0xa0, 0x60, 0xa0, 0xb0),
    mask: Color::rgba(0x40, 0xa0, 0x60, 0x60),
    paste: Color::rgba(0x90, 0x90, 0x90, 0x60),
    outline: Color::rgb(0xe6, 0xc8, 0x4f),
    other_layer: Color::rgb(0x90, 0x90, 0x90),
};

pub static LIGHT: Palette = Palette {
    background: Color::rgb(0xf5, 0xf4, 0xef),
    grid: Color::rgba(0x8a, 0x8f, 0x9a, 0xa0),
    axis: Color::rgba(0x60, 0x66, 0x74, 0x90),

    sch_wire: Color::rgb(0x0f, 0x7a, 0x4a),
    sch_body: Color::rgb(0x8c, 0x1c, 0x1c),
    sch_body_fill: Color::rgba(0xff, 0xf6, 0xc8, 0xff),
    sch_pin: Color::rgb(0x8c, 0x1c, 0x1c),
    sch_pin_text: Color::rgb(0x1f, 0x5f, 0x66),
    sch_label: Color::rgb(0x6a, 0x4a, 0x00),
    sch_power: Color::rgb(0xa0, 0x20, 0x20),
    sch_text: Color::rgb(0x1a, 0x3a, 0xb0),
    sch_junction: Color::rgb(0x0f, 0x7a, 0x4a),

    selection: Color::rgba(0x1f, 0x6f, 0xe0, 0xe0),
    selection_fill: Color::rgba(0x1f, 0x6f, 0xe0, 0x22),
    ratsnest: Color::rgba(0x30, 0x30, 0x38, 0xb0),
    flagged: Color::rgba(0xff, 0x80, 0x00, 0x80),
    drc_marker: Color::rgba(0xd0, 0x10, 0x40, 0xe0),
    net_highlight: Color::rgba(0x00, 0x00, 0x00, 0x48),
    probe: Color::rgba(0xd0, 0x90, 0x00, 0xe0),
    probe_fill: Color::rgba(0xff, 0xc0, 0x00, 0x30),
    hover: Color::rgba(0x00, 0x00, 0x00, 0x38),
    preview: Color::rgba(0x1f, 0x6f, 0xe0, 0xc0),
    origin: Color::rgba(0x60, 0x66, 0x74, 0x90),
    pad_text: Color::rgba(0xff, 0xff, 0xff, 0xe0),

    via: Color::rgb(0x80, 0x80, 0x84),
    hole: Color::rgb(0xf5, 0xf4, 0xef),

    top_copper: Color::rgb(0xc8, 0x34, 0x34),
    bottom_copper: Color::rgb(0x2f, 0x62, 0xb0),
    inner_copper: [
        Color::rgb(0xb0, 0x88, 0x20),
        Color::rgb(0x20, 0x98, 0x70),
        Color::rgb(0x98, 0x48, 0xb0),
        Color::rgb(0x70, 0xa8, 0x20),
    ],
    top_silk: Color::rgb(0x3a, 0x3a, 0x40),
    bottom_silk: Color::rgb(0x6a, 0x4a, 0xa0),
    top_fab: Color::rgb(0x70, 0x74, 0x84),
    bottom_fab: Color::rgb(0x90, 0x94, 0xa4),
    top_courtyard: Color::rgba(0xb0, 0x30, 0xb0, 0xb0),
    bottom_courtyard: Color::rgba(0x80, 0x30, 0x80, 0xb0),
    mask: Color::rgba(0x20, 0x80, 0x40, 0x50),
    paste: Color::rgba(0x60, 0x60, 0x60, 0x50),
    outline: Color::rgb(0xa0, 0x78, 0x00),
    other_layer: Color::rgb(0x60, 0x60, 0x60),
};

static THEME: AtomicU8 = AtomicU8::new(0);

/// Select the palette used for drawing from now on.
pub fn set_theme(theme: Theme) {
    THEME.store(theme as u8, Ordering::Relaxed);
}

pub fn theme() -> Theme {
    match THEME.load(Ordering::Relaxed) {
        1 => Theme::Light,
        _ => Theme::Dark,
    }
}

/// The active palette.
pub fn pal() -> &'static Palette {
    match theme() {
        Theme::Dark => &DARK,
        Theme::Light => &LIGHT,
    }
}

/// Canvas background with the given grid spacing (world units), in the active palette.
pub fn background(grid: f32) -> Background {
    let p = pal();
    Background { color: p.background, grid_color: p.grid, axis_color: p.axis, grid, ..Default::default() }
}

/// Junction dot radius (mm).
pub const JUNCTION_RADIUS: f32 = 0.45;

/// Schematic wire width.
pub const WIRE_WIDTH: Coord = 152_400;

pub fn board_layer_color(layer: LayerId) -> Color {
    let p = pal();
    match layer {
        LayerId::TOP_COPPER => p.top_copper,
        LayerId::BOTTOM_COPPER => p.bottom_copper,
        l if l.is_copper() => p.inner_copper[(l.0 as usize - 1) % 4],
        LayerId::TOP_SILK => p.top_silk,
        LayerId::BOTTOM_SILK => p.bottom_silk,
        LayerId::TOP_FAB => p.top_fab,
        LayerId::BOTTOM_FAB => p.bottom_fab,
        LayerId::TOP_COURTYARD => p.top_courtyard,
        LayerId::BOTTOM_COURTYARD => p.bottom_courtyard,
        LayerId::TOP_MASK | LayerId::BOTTOM_MASK => p.mask,
        LayerId::TOP_PASTE | LayerId::BOTTOM_PASTE => p.paste,
        LayerId::OUTLINE => p.outline,
        _ => p.other_layer,
    }
}
