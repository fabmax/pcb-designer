//! Render the synthetic schematic and board to PNG files (development aid).
//!
//! Usage: snapshot [components=200] [out_dir=out]

use render::offscreen::Offscreen;
use render::{Camera, gpu::Background};
use spike::Project;

fn main() {
    let mut args = std::env::args().skip(1);
    let n: usize = args.next().map(|a| a.parse().unwrap()).unwrap_or(200);
    let out = args.next().unwrap_or_else(|| "out".into());
    std::fs::create_dir_all(&out).unwrap();

    let (mut p, _) = Project::generate(n, 1);
    let off = Offscreen::new().expect("GPU");
    let (w, h) = (1600u32, 1000u32);

    let shots: [(&str, bool, Option<f64>); 4] = [
        ("schematic_fit", true, None),
        ("schematic_zoom", true, Some(6.0)),
        ("board_fit", false, None),
        ("board_zoom", false, Some(40.0)),
    ];
    for (name, sch, zoom) in shots {
        let scene = if sch { &mut p.sch_scene } else { &mut p.pcb_scene };
        let b = scene.bounds().unwrap();
        let mut cam = Camera::default();
        cam.fit(b, [w as f64, h as f64], 20.0);
        if let Some(z) = zoom {
            // Zoom into the top left corner of the content.
            cam.zoom = z;
            let half = [w as f64 * 0.5 / z, h as f64 * 0.5 / z];
            cam.center = [b[0] + half[0], b[3] - half[1]];
        }
        let bg = Background {
            grid: if sch { 2.54 } else { 1.0 },
            ..Default::default()
        };
        let img = off.render(scene, &cam, &bg, w, h);
        let path = format!("{out}/{name}.png");
        img.save_png(&path).unwrap();
        println!("wrote {path}");
    }
}
