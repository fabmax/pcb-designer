//! Colors and line widths of the canvases. Two palettes (dark, light); the active one is
//! global ([`set_theme`]) and baked into the scene when objects are drawn, so views are
//! rebuilt after a change ([`crate::Document::restyle`]).

use std::collections::BTreeMap;
use std::sync::RwLock;
use std::sync::atomic::{AtomicU8, Ordering};

use model::{Coord, LayerId};
use render::Color;
use render::gpu::Background;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
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
    pub sch_global_label: Color,
    pub sch_hier_label: Color,
    pub sch_power: Color,
    /// Sheet symbols: outline (and name), fill.
    pub sch_sheet: Color,
    pub sch_sheet_fill: Color,
    pub sch_text: Color,
    pub sch_junction: Color,
    /// No-connect flags (blue, as in KiCad).
    pub sch_no_connect: Color,
    /// The cross over a "do not populate" symbol.
    pub sch_dnp: Color,
    /// Bus wires and bus entries.
    pub sch_bus: Color,
    /// The drawing frame (border, zone grid, title block) and the paper's edge.
    pub sch_frame: Color,
    pub sch_page: Color,

    pub selection: Color,
    pub selection_fill: Color,
    /// Ratsnest lines (unrouted connections).
    pub ratsnest: Color,
    /// Halo around flagged tracks and vias (shorts, unconnected copper).
    pub flagged: Color,
    /// Markers of errors (design rule violations, ERC errors).
    pub drc_marker: Color,
    /// Markers of warnings.
    pub warning_marker: Color,
    /// Copper of highlighted nets.
    pub net_highlight: Color,
    /// Objects selected in the other view (cross-probing).
    pub probe: Color,
    pub probe_fill: Color,
    /// Simulation values on the schematic (voltage and current tags) and their background.
    pub sim_value: Color,
    pub sim_value_bg: Color,
    /// The waveform plot: its chart's background, grid and frame, tick labels.
    pub plot_bg: Color,
    pub plot_grid: Color,
    pub plot_text: Color,
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
    sch_global_label: Color::rgb(0xf0, 0xa0, 0x60),
    sch_hier_label: Color::rgb(0xd0, 0xb0, 0x40),
    sch_power: Color::rgb(0xe0, 0x80, 0x80),
    sch_sheet: Color::rgb(0xb0, 0x90, 0xe0),
    sch_sheet_fill: Color::rgba(0xb0, 0x90, 0xe0, 0x18),
    sch_text: Color::rgb(0x9f, 0xb4, 0xff),
    sch_junction: Color::rgb(0x5f, 0xd3, 0xa0),
    sch_no_connect: Color::rgb(0x70, 0x9c, 0xf2),
    sch_dnp: Color::rgb(0xe8, 0x5a, 0x5a),
    sch_bus: Color::rgb(0x7c, 0x8a, 0xff),
    sch_frame: Color::rgb(0xa0, 0x78, 0x7c),
    sch_page: Color::rgb(0x34, 0x38, 0x42),

    selection: Color::rgba(0x4c, 0x9a, 0xff, 0xe0),
    selection_fill: Color::rgba(0x4c, 0x9a, 0xff, 0x28),
    ratsnest: Color::rgba(0xe8, 0xe8, 0xf0, 0xb0),
    flagged: Color::rgba(0xff, 0x9a, 0x2e, 0x70),
    drc_marker: Color::rgba(0xff, 0x4d, 0x6a, 0xe0),
    warning_marker: Color::rgba(0xff, 0xb4, 0x30, 0xe0),
    net_highlight: Color::rgba(0xff, 0xff, 0xff, 0x60),
    probe: Color::rgba(0xff, 0xd0, 0x4c, 0xd0),
    probe_fill: Color::rgba(0xff, 0xd0, 0x4c, 0x22),
    sim_value: Color::rgb(0x8c, 0xd0, 0xff),
    sim_value_bg: Color::rgba(0x16, 0x18, 0x1d, 0xd8),
    plot_bg: Color::rgb(0x0d, 0x0e, 0x11),
    plot_grid: Color::rgb(0x2a, 0x2e, 0x36),
    plot_text: Color::rgb(0x8a, 0x92, 0xa3),
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
    sch_global_label: Color::rgb(0xb0, 0x40, 0x00),
    sch_hier_label: Color::rgb(0x70, 0x50, 0x10),
    sch_power: Color::rgb(0xa0, 0x20, 0x20),
    sch_sheet: Color::rgb(0x6a, 0x3a, 0xa0),
    sch_sheet_fill: Color::rgba(0xf3, 0xee, 0xfb, 0xff),
    sch_text: Color::rgb(0x1a, 0x3a, 0xb0),
    sch_junction: Color::rgb(0x0f, 0x7a, 0x4a),
    sch_no_connect: Color::rgb(0x1f, 0x4c, 0xc4),
    sch_dnp: Color::rgb(0xd0, 0x30, 0x30),
    sch_bus: Color::rgb(0x22, 0x30, 0xa0),
    sch_frame: Color::rgb(0x9a, 0x3c, 0x44),
    sch_page: Color::rgb(0xd2, 0xd0, 0xc8),

    selection: Color::rgba(0x1f, 0x6f, 0xe0, 0xe0),
    selection_fill: Color::rgba(0x1f, 0x6f, 0xe0, 0x22),
    ratsnest: Color::rgba(0x30, 0x30, 0x38, 0xb0),
    flagged: Color::rgba(0xff, 0x80, 0x00, 0x80),
    drc_marker: Color::rgba(0xd0, 0x10, 0x40, 0xe0),
    warning_marker: Color::rgba(0xc0, 0x78, 0x00, 0xe0),
    net_highlight: Color::rgba(0x00, 0x00, 0x00, 0x48),
    probe: Color::rgba(0xd0, 0x90, 0x00, 0xe0),
    probe_fill: Color::rgba(0xff, 0xc0, 0x00, 0x30),
    sim_value: Color::rgb(0x0b, 0x5c, 0xad),
    sim_value_bg: Color::rgba(0xff, 0xff, 0xff, 0xe0),
    plot_bg: Color::rgb(0xff, 0xff, 0xff),
    plot_grid: Color::rgb(0xc4, 0xc8, 0xd0),
    plot_text: Color::rgb(0x5c, 0x63, 0x70),
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

/// Waveform trace colours, a categorical palette in a fixed order (blue, orange, aqua,
/// yellow, magenta, green, violet, red), stepped per theme for the canvas background.
/// Validated for colour-vision deficiencies between neighbours (worst ΔE 9.1 light, 8.4
/// dark). On the light background four sit below 3:1 contrast, so traces are always
/// named next to their colour (the plot's readout table).
pub const TRACES_DARK: [Color; 8] = [
    Color::rgb(0x39, 0x87, 0xe5),
    Color::rgb(0xd9, 0x59, 0x26),
    Color::rgb(0x19, 0x9e, 0x70),
    Color::rgb(0xc9, 0x85, 0x00),
    Color::rgb(0xd5, 0x51, 0x81),
    Color::rgb(0x0c, 0xa3, 0x0c),
    Color::rgb(0x90, 0x85, 0xe9),
    Color::rgb(0xe6, 0x67, 0x67),
];
pub const TRACES_LIGHT: [Color; 8] = [
    Color::rgb(0x2a, 0x78, 0xd6),
    Color::rgb(0xeb, 0x68, 0x34),
    Color::rgb(0x1b, 0xaf, 0x7a),
    Color::rgb(0xed, 0xa1, 0x00),
    Color::rgb(0xe8, 0x7b, 0xa4),
    Color::rgb(0x00, 0x83, 0x00),
    Color::rgb(0x62, 0x50, 0xd6),
    Color::rgb(0xe3, 0x49, 0x48),
];

/// The colour of signal `i` of a plot (past eight they repeat; the plot dashes the
/// second round).
pub fn trace_color(i: usize) -> Color {
    let traces = match theme() {
        Theme::Dark => &TRACES_DARK,
        Theme::Light => &TRACES_LIGHT,
    };
    traces[i % traces.len()]
}

/// Canvas background with the given grid spacing (world units), in the active palette.
pub fn background(grid: f32) -> Background {
    let p = pal();
    Background { color: p.background, grid_color: p.grid, axis_color: p.axis, grid, ..Default::default() }
}

/// Junction dot radius.
pub const JUNCTION_RADIUS: Coord = 450_000;

/// Schematic wire width.
pub const WIRE_WIDTH: Coord = 152_400;
/// Bus wires and entries: 2.5 wire widths.
pub const BUS_WIDTH: Coord = WIRE_WIDTH * 5 / 2;

/// A board layer colour the user can change (per theme; Settings → Layer colours).
/// Inner copper layers cycle through four colours.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LayerColor {
    TopCopper,
    BottomCopper,
    Inner1,
    Inner2,
    Inner3,
    Inner4,
    TopSilk,
    BottomSilk,
    TopFab,
    BottomFab,
    TopCourtyard,
    BottomCourtyard,
    Mask,
    Paste,
    Outline,
}

impl LayerColor {
    pub const ALL: [LayerColor; 15] = [
        LayerColor::TopCopper,
        LayerColor::BottomCopper,
        LayerColor::Inner1,
        LayerColor::Inner2,
        LayerColor::Inner3,
        LayerColor::Inner4,
        LayerColor::TopSilk,
        LayerColor::BottomSilk,
        LayerColor::TopFab,
        LayerColor::BottomFab,
        LayerColor::TopCourtyard,
        LayerColor::BottomCourtyard,
        LayerColor::Mask,
        LayerColor::Paste,
        LayerColor::Outline,
    ];

    pub fn label(self) -> &'static str {
        match self {
            LayerColor::TopCopper => "Top copper",
            LayerColor::BottomCopper => "Bottom copper",
            LayerColor::Inner1 => "Inner copper 1, 5, …",
            LayerColor::Inner2 => "Inner copper 2, 6, …",
            LayerColor::Inner3 => "Inner copper 3, 7, …",
            LayerColor::Inner4 => "Inner copper 4, 8, …",
            LayerColor::TopSilk => "Top silkscreen",
            LayerColor::BottomSilk => "Bottom silkscreen",
            LayerColor::TopFab => "Top fabrication",
            LayerColor::BottomFab => "Bottom fabrication",
            LayerColor::TopCourtyard => "Top courtyard",
            LayerColor::BottomCourtyard => "Bottom courtyard",
            LayerColor::Mask => "Solder mask",
            LayerColor::Paste => "Solder paste",
            LayerColor::Outline => "Board outline",
        }
    }

    /// The colour of a board layer, if it's one of these.
    pub fn of(layer: LayerId) -> Option<LayerColor> {
        Some(match layer {
            LayerId::TOP_COPPER => LayerColor::TopCopper,
            LayerId::BOTTOM_COPPER => LayerColor::BottomCopper,
            l if l.is_copper() => {
                [LayerColor::Inner1, LayerColor::Inner2, LayerColor::Inner3, LayerColor::Inner4][(l.0 as usize - 1) % 4]
            }
            LayerId::TOP_SILK => LayerColor::TopSilk,
            LayerId::BOTTOM_SILK => LayerColor::BottomSilk,
            LayerId::TOP_FAB => LayerColor::TopFab,
            LayerId::BOTTOM_FAB => LayerColor::BottomFab,
            LayerId::TOP_COURTYARD => LayerColor::TopCourtyard,
            LayerId::BOTTOM_COURTYARD => LayerColor::BottomCourtyard,
            LayerId::TOP_MASK | LayerId::BOTTOM_MASK => LayerColor::Mask,
            LayerId::TOP_PASTE | LayerId::BOTTOM_PASTE => LayerColor::Paste,
            LayerId::OUTLINE => LayerColor::Outline,
            _ => return None,
        })
    }

    /// The palette's colour.
    pub fn default_in(self, p: &Palette) -> Color {
        match self {
            LayerColor::TopCopper => p.top_copper,
            LayerColor::BottomCopper => p.bottom_copper,
            LayerColor::Inner1 => p.inner_copper[0],
            LayerColor::Inner2 => p.inner_copper[1],
            LayerColor::Inner3 => p.inner_copper[2],
            LayerColor::Inner4 => p.inner_copper[3],
            LayerColor::TopSilk => p.top_silk,
            LayerColor::BottomSilk => p.bottom_silk,
            LayerColor::TopFab => p.top_fab,
            LayerColor::BottomFab => p.bottom_fab,
            LayerColor::TopCourtyard => p.top_courtyard,
            LayerColor::BottomCourtyard => p.bottom_courtyard,
            LayerColor::Mask => p.mask,
            LayerColor::Paste => p.paste,
            LayerColor::Outline => p.outline,
        }
    }
}

/// Changed layer colours per theme (the palettes are the defaults).
pub type LayerColors = BTreeMap<Theme, BTreeMap<LayerColor, Color>>;

static LAYER_COLORS: RwLock<LayerColors> = RwLock::new(BTreeMap::new());

/// Use these layer colours from now on (redraw with [`crate::Document::restyle`]).
pub fn set_layer_colors(colors: LayerColors) {
    *LAYER_COLORS.write().unwrap() = colors;
}

/// A layer colour in the active theme, changed or the palette's.
pub fn layer_color(c: LayerColor) -> Color {
    let t = theme();
    LAYER_COLORS.read().unwrap().get(&t).and_then(|m| m.get(&c)).copied().unwrap_or_else(|| c.default_in(pal()))
}

/// `#rrggbbaa` (alpha left out when opaque).
pub fn color_hex(c: Color) -> String {
    let [r, g, b, a] = c.0;
    if a == 0xff { format!("#{r:02x}{g:02x}{b:02x}") } else { format!("#{r:02x}{g:02x}{b:02x}{a:02x}") }
}

/// `#rrggbb` or `#rrggbbaa`.
pub fn parse_color_hex(s: &str) -> Option<Color> {
    let h = s.strip_prefix('#')?;
    if !matches!(h.len(), 6 | 8) || !h.is_ascii() {
        return None;
    }
    let byte = |i: usize| u8::from_str_radix(&h[i..i + 2], 16).ok();
    Some(Color::rgba(byte(0)?, byte(2)?, byte(4)?, if h.len() == 8 { byte(6)? } else { 0xff }))
}

pub fn board_layer_color(layer: LayerId) -> Color {
    LayerColor::of(layer).map_or(pal().other_layer, layer_color)
}
