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
    Agent => "tabler/bot-id",
    Simulation => "tabler/wave-sine",
    /// Analysis kinds (a transient is `Simulation`).
    OperatingPoint => "tabler/gauge",
    DcSweep => "tabler/chart-line",
    AcSweep => "tabler/chart-bar",
    Probe => "probe",
    /// Run and stop a simulation.
    Play => "tabler/player-play",
    Stop => "tabler/player-stop",
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
    Check => "tabler/check",
    Copy => "tabler/copy",
    Spreadsheet => "tabler/file-spreadsheet",
    FileCode => "tabler/file-code",
    FilePdf => "tabler/file-type-pdf",
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

/// The app icon's pixels (RGBA, 8 bits each) and its width (it's square).
fn app_icon_rgba() -> (Vec<u8>, usize) {
    let reader = png::Decoder::new(std::io::Cursor::new(APP_ICON_PNG)).read_info();
    let mut reader = reader.expect("embedded icon");
    let mut rgba = vec![0; reader.output_buffer_size().expect("icon size")];
    let info = reader.next_frame(&mut rgba).expect("embedded icon");
    assert_eq!((info.color_type, info.bit_depth), (png::ColorType::Rgba, png::BitDepth::Eight));
    rgba.truncate(info.buffer_size());
    (rgba, info.width as usize)
}

/// The app icon as a texture of `px`² pixels (a divisor of its 1024²), box-filtered so
/// it's sharp when drawn at a quarter or half of that in points.
pub fn app_icon(ctx: &Context, px: usize) -> TextureHandle {
    let (rgba, size) = app_icon_rgba();
    let small = downsample(&rgba, size, px);
    let image = ColorImage::from_rgba_unmultiplied([px, px], &small);
    ctx.load_texture(format!("app icon {px}"), image, TextureOptions::LINEAR)
}

/// Box-filters a square RGBA image of `size`² down to `to`² (`size` a multiple of `to`).
/// Averages premultiplied colours so transparent pixels don't darken the edge.
pub fn downsample(rgba: &[u8], size: usize, to: usize) -> Vec<u8> {
    let f = size / to;
    let mut out = vec![0u8; to * to * 4];
    for y in 0..to {
        for x in 0..to {
            let mut sum = [0u32; 4];
            for sy in 0..f {
                for sx in 0..f {
                    let i = ((y * f + sy) * size + x * f + sx) * 4;
                    let a = rgba[i + 3] as u32;
                    for c in 0..3 {
                        sum[c] += rgba[i + c] as u32 * a;
                    }
                    sum[3] += a;
                }
            }
            let o = (y * to + x) * 4;
            let n = (f * f) as u32;
            for c in 0..3 {
                out[o + c] = sum[c].checked_div(sum[3]).unwrap_or(0) as u8;
            }
            out[o + 3] = (sum[3] / n) as u8;
        }
    }
    out
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

    #[test]
    fn downsampling_keeps_edges_clean() {
        // Half opaque red, half transparent black: the average stays pure red, half alpha.
        let mut img = vec![0u8; 2 * 2 * 4];
        img[0..4].copy_from_slice(&[255, 0, 0, 255]);
        img[8..12].copy_from_slice(&[255, 0, 0, 255]);
        assert_eq!(downsample(&img, 2, 1), [255, 0, 0, 127]);
        let (rgba, size) = app_icon_rgba();
        assert_eq!((rgba.len(), size), (1024 * 1024 * 4, 1024));
    }
}
