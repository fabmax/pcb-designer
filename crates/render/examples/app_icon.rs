//! Draws the application icon (1024×1024 PNG with alpha).
//!
//! `cargo run -p render --example app_icon -- crates/app/assets/icon.png`
//!
//! The icon is a macOS-style squircle of green solder mask with copper traces, pads and
//! a via. The renderer's background is opaque, so the icon is drawn on black and on
//! white, and the alpha is recovered from the difference.

use render::gpu::Background;
use render::{Camera, Color, Instance, Scene};

const SIZE: u32 = 1024;
/// macOS icon grid: the body is 824 of 1024, centred.
const BODY: f32 = 824.0;

const MASK: Color = Color::rgb(0x1e, 0x6a, 0x45);
const MASK_DARK: Color = Color::rgb(0x15, 0x4f, 0x33);
const COPPER: Color = Color::rgb(0xe8, 0xb4, 0x5c);
const DRILL: Color = Color::rgb(0x10, 0x2a, 0x1d);

fn main() {
    let out = std::env::args().nth(1).unwrap_or_else(|| "icon.png".into());
    let scene = icon();
    let r = render::offscreen::Offscreen::new().expect("GPU");
    let camera = Camera { center: [SIZE as f64 / 2.0, SIZE as f64 / 2.0], zoom: 1.0 };
    let bg = |c: Color| Background {
        color: c,
        grid_color: Color::rgba(0, 0, 0, 0),
        axis_color: Color::rgba(0, 0, 0, 0),
        grid: 0.0,
        min_grid_px: 1.0,
        show_grid: false,
    };
    let black = r.render(&scene, &camera, &bg(Color::rgb(0, 0, 0)), SIZE, SIZE);
    let white = r.render(&scene, &camera, &bg(Color::rgb(255, 255, 255)), SIZE, SIZE);

    // On black: c·a. On white: c·a + (1 - a). So a = 1 - (white - black).
    let mut rgba = vec![0u8; (SIZE * SIZE * 4) as usize];
    for (i, px) in rgba.as_chunks_mut::<4>().0.iter_mut().enumerate() {
        let b = &black.rgba[i * 4..i * 4 + 3];
        let w = &white.rgba[i * 4..i * 4 + 3];
        let diff = (0..3).map(|c| w[c] as f32 - b[c] as f32).sum::<f32>() / 3.0;
        let a = (1.0 - diff / 255.0).clamp(0.0, 1.0);
        for c in 0..3 {
            px[c] = if a > 0.0 { (b[c] as f32 / a).round().min(255.0) as u8 } else { 0 };
        }
        px[3] = (a * 255.0).round() as u8;
    }
    let file = std::fs::File::create(&out).expect("create output");
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), SIZE, SIZE);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header().and_then(|mut w| w.write_image_data(&rgba)).expect("write PNG");
    println!("wrote {out}");
}

/// Coordinates: pixels of the 1024 canvas, Y up.
fn icon() -> Scene {
    let mut scene = Scene::new();
    let layer = scene.add_layer("icon");
    let mut p = Vec::new();
    let c = SIZE as f32 / 2.0;

    squircle(&mut p, [c, c], BODY / 2.0, MASK_DARK);
    squircle(&mut p, [c, c], BODY / 2.0 - 12.0, MASK);

    let w = 64.0;
    let trace = |p: &mut Vec<Instance>, pts: &[[f32; 2]]| {
        for s in pts.windows(2) {
            p.push(Instance::line(s[0], s[1], w, COPPER));
        }
    };
    // Through-hole pad with its drill.
    let pad = |p: &mut Vec<Instance>, at: [f32; 2]| {
        p.push(Instance::circle(at, 78.0, COPPER));
        p.push(Instance::circle(at, 30.0, DRILL));
    };
    // SMD pad.
    let smd = |p: &mut Vec<Instance>, at: [f32; 2]| p.push(Instance::rect(at, [120.0, 150.0], 22.0, 0.0, COPPER));

    trace(&mut p, &[[300.0, 690.0], [560.0, 690.0], [730.0, 520.0], [730.0, 350.0]]);
    trace(&mut p, &[[300.0, 450.0], [420.0, 450.0], [550.0, 320.0]]);
    pad(&mut p, [300.0, 690.0]);
    pad(&mut p, [300.0, 450.0]);
    smd(&mut p, [730.0, 320.0]);
    // Via at the end of the second trace.
    p.push(Instance::circle([550.0, 320.0], 56.0, COPPER));
    p.push(Instance::circle([550.0, 320.0], 24.0, DRILL));

    scene.set_item(1, p.into_iter().map(|i| (layer, i)));
    scene
}

/// macOS-style continuous-corner square: the superellipse |x|^5 + |y|^5 = r^5.
fn squircle(p: &mut Vec<Instance>, center: [f32; 2], r: f32, color: Color) {
    const N: usize = 360;
    // Two fans, shifted against each other: the hairline gaps between one fan's triangles
    // are covered by the other's.
    for offset in [0.25, 0.75] {
        let point = |i: usize| {
            let t = (i as f32 + offset) / N as f32 * std::f32::consts::TAU;
            let (s, c) = t.sin_cos();
            let e = 2.0 / 5.0;
            [center[0] + r * c.signum() * c.abs().powf(e), center[1] + r * s.signum() * s.abs().powf(e)]
        };
        for i in 0..N {
            p.push(Instance::triangle(center, point(i), point(i + 1), [false, true, false], color));
        }
    }
}
