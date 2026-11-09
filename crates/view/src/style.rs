//! Colors and line widths of the canvases (dark theme).

use model::{Coord, LayerId};
use render::Color;

pub const SCH_BG: Color = Color::rgb(0x16, 0x18, 0x1d);
pub const SCH_WIRE: Color = Color::rgb(0x5f, 0xd3, 0xa0);
pub const SCH_BODY: Color = Color::rgb(0xe0, 0xa0, 0x60);
pub const SCH_BODY_FILL: Color = Color::rgba(0xe0, 0xa0, 0x60, 0x22);
pub const SCH_PIN: Color = Color::rgb(0xd0, 0x70, 0x70);
pub const SCH_PIN_TEXT: Color = Color::rgb(0x9a, 0x9f, 0xae);
pub const SCH_LABEL: Color = Color::rgb(0xf0, 0xd0, 0x70);
pub const SCH_POWER: Color = Color::rgb(0xe0, 0x80, 0x80);
pub const SCH_TEXT: Color = Color::rgb(0x9f, 0xb4, 0xff);
pub const SCH_JUNCTION: Color = SCH_WIRE;

pub const SELECTION: Color = Color::rgba(0x4c, 0x9a, 0xff, 0xe0);
pub const SELECTION_FILL: Color = Color::rgba(0x4c, 0x9a, 0xff, 0x28);
pub const HOVER: Color = Color::rgba(0xff, 0xff, 0xff, 0x50);

pub const VIA: Color = Color::rgb(0xa8, 0xa8, 0xa8);
pub const HOLE: Color = Color::rgb(0x10, 0x11, 0x14);

/// Junction dot radius (mm).
pub const JUNCTION_RADIUS: f32 = 0.45;

/// Schematic wire width.
pub const WIRE_WIDTH: Coord = 152_400;

pub fn board_layer_color(layer: LayerId) -> Color {
    match layer {
        LayerId::TOP_COPPER => Color::rgb(0xc8, 0x3c, 0x3c),
        LayerId::BOTTOM_COPPER => Color::rgb(0x3d, 0x6f, 0xb8),
        l if l.is_copper() => {
            const INNER: [Color; 4] = [
                Color::rgb(0xc8, 0xa0, 0x3c),
                Color::rgb(0x3c, 0xb8, 0x8c),
                Color::rgb(0xb0, 0x5c, 0xc8),
                Color::rgb(0x8c, 0xc8, 0x3c),
            ];
            INNER[(l.0 as usize - 1) % 4]
        }
        LayerId::TOP_SILK => Color::rgb(0xe8, 0xe4, 0xc8),
        LayerId::BOTTOM_SILK => Color::rgb(0xb8, 0xa0, 0xe0),
        LayerId::TOP_FAB => Color::rgb(0x8c, 0x8f, 0xa0),
        LayerId::BOTTOM_FAB => Color::rgb(0x60, 0x64, 0x78),
        LayerId::TOP_COURTYARD => Color::rgba(0xd0, 0x80, 0xd0, 0xb0),
        LayerId::BOTTOM_COURTYARD => Color::rgba(0xa0, 0x60, 0xa0, 0xb0),
        LayerId::TOP_MASK | LayerId::BOTTOM_MASK => Color::rgba(0x40, 0xa0, 0x60, 0x60),
        LayerId::TOP_PASTE | LayerId::BOTTOM_PASTE => Color::rgba(0x90, 0x90, 0x90, 0x60),
        LayerId::OUTLINE => Color::rgb(0xe6, 0xc8, 0x4f),
        _ => Color::rgb(0x90, 0x90, 0x90),
    }
}
