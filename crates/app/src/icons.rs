//! UI icons: SVG files in `crates/app/icons/` (our own, in Tabler's style) and
//! `crates/app/icons/tabler/` (copied from Tabler, MIT). Every icon is drawn in
//! `currentColor` (white when rasterised; faint parts as fill or stroke opacity) and tinted
//! when painted, so it follows the theme. Rasterised with resvg once per pixel size and
//! cached as a texture.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use egui::{Color32, ColorImage, Context, Painter, Rect, TextureHandle, TextureOptions, pos2, vec2};

/// The app icon, 1024² PNG (drawn by `cargo run -p render --example app_icon`).
pub const APP_ICON_PNG: &[u8] = include_bytes!("../assets/icon.png");

/// Declares `Icon` with the SVG file of each variant (relative to `icons/`, no extension).
macro_rules! icons {
    ($($(#[$m:meta])* $name:ident => $file:literal,)*) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
        pub enum Icon { $($(#[$m])* $name,)* }

        impl Icon {
            #[cfg(test)]
            pub const ALL: &[Icon] = &[$(Icon::$name),*];

            fn svg(self) -> &'static [u8] {
                match self { $(Icon::$name => include_bytes!(concat!("../icons/", $file, ".svg")),)* }
            }
        }
    };
}

icons! {
    Select => "tabler/pointer",
    Wire => "wire",
    Bus => "bus",
    BusEntry => "bus-entry",
    Label => "label",
    Power => "power",
    Junction => "junction",
    NoConnect => "no-connect",
    PowerFlag => "power-flag",
    Place => "place",
    Route => "route",
    Via => "via",
    Outline => "outline",
    Zone => "zone",
    Keepout => "keepout",
    Cutout => "cutout",
    Arc => "arc",
    Polygon => "tabler/polygon",
    Measure => "tabler/ruler-measure",
    Pin => "pin",
    Pad => "pad",
    Line => "tabler/line",
    Rect => "tabler/rectangle",
    Circle => "circle",
    Text => "tabler/letter-t",
    Layers => "tabler/stack-2",
    Grid => "tabler/grid-4x4",
    /// Zoom to fit.
    Fit => "tabler/zoom-scan",
    Schematic => "schematic",
    Board => "board",
    Library => "tabler/books",
    /// Panels.
    Tree => "tabler/list-tree",
    Properties => "tabler/adjustments-horizontal",
    Wrench => "tabler/tool",
    Messages => "tabler/alert-triangle",
    Info => "tabler/info-circle",
    BoardSetup => "board-setup",
    SchematicSetup => "schematic-setup",
    Nets => "nets",
    Activity => "tabler/activity",
    /// Hide a panel.
    Minus => "tabler/minus",
    /// Panel header buttons.
    Plus => "tabler/plus",
    Search => "tabler/search",
    CollapseAll => "tabler/fold",
    /// Back to the default value.
    Reset => "tabler/arrow-back-up",
    /// Project tree nodes.
    Project => "project",
    Folder => "tabler/folder",
    Part => "part",
    /// A part without symbol, placed on the board only.
    BoardPart => "board-part",
    Symbol => "symbol",
    Footprint => "footprint",
    Missing => "tabler/ban",
    /// Corner styles of drawn wires and tracks.
    Corner90 => "corner-90",
    Corner45 => "corner-45",
    CornerFree => "corner-free",
    CornerArc45 => "corner-arc-45",
    CornerArc90 => "corner-arc-90",
}

/// Paint `icon` into `rect` (the largest square in it, snapped to pixels) in `color`.
pub fn paint(painter: &Painter, rect: Rect, icon: Icon, color: Color32) {
    let ctx = painter.ctx();
    let ppp = ctx.pixels_per_point();
    let px = (rect.width().min(rect.height()) * ppp).round().max(1.0);
    let min = rect.center() - vec2(px, px) / ppp / 2.0;
    let min = pos2((min.x * ppp).round() / ppp, (min.y * ppp).round() / ppp);
    let square = Rect::from_min_size(min, vec2(px, px) / ppp);
    let texture = texture(ctx, icon, px as u32);
    let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
    painter.image(texture.id(), square, uv, color);
}

type Cache = Arc<Mutex<HashMap<(Icon, u32), TextureHandle>>>;

/// The icon rasterised at `px`² pixels, cached per context.
fn texture(ctx: &Context, icon: Icon, px: u32) -> TextureHandle {
    let cache: Cache = ctx.data_mut(|d| d.get_temp_mut_or_default::<Cache>(egui::Id::NULL).clone());
    let mut cache = cache.lock().expect("icon cache");
    cache
        .entry((icon, px))
        .or_insert_with(|| {
            let image = rasterize(icon, px).unwrap_or_else(|e| {
                log::error!("icon {icon:?}: {e}");
                ColorImage::filled([px as usize; 2], Color32::TRANSPARENT)
            });
            ctx.load_texture(format!("icon-{icon:?}-{px}"), image, TextureOptions::LINEAR)
        })
        .clone()
}

/// The icon in white on transparent, `px`² pixels.
fn rasterize(icon: Icon, px: u32) -> Result<ColorImage, String> {
    let svg = String::from_utf8_lossy(icon.svg()).replace("currentColor", "#fff");
    let tree = resvg::usvg::Tree::from_str(&svg, &resvg::usvg::Options::default()).map_err(|e| e.to_string())?;
    let mut pixmap = resvg::tiny_skia::Pixmap::new(px, px).ok_or("empty icon size")?;
    let size = tree.size();
    let transform = resvg::tiny_skia::Transform::from_scale(px as f32 / size.width(), px as f32 / size.height());
    resvg::render(&tree, transform, &mut pixmap.as_mut());
    Ok(ColorImage::from_rgba_premultiplied([px as usize; 2], pixmap.data()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_icon_renders() {
        for &icon in Icon::ALL {
            let image = rasterize(icon, 24).unwrap_or_else(|e| panic!("{icon:?}: {e}"));
            let opaque = image.pixels.iter().filter(|p| p.a() > 128).count();
            assert!(opaque > 20, "{icon:?} is (nearly) empty: {opaque} pixels");
            // Drawn in white only (black where something is cut out), tinted when painted.
            assert!(
                image.pixels.iter().all(|p| p.r() == p.g() && p.g() == p.b()),
                "{icon:?} has colours; draw it in currentColor"
            );
        }
    }
}
