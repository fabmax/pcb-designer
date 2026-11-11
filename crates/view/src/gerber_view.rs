//! Read-only view of exported Gerber and drill files, as read back from disk: one scene
//! layer per file, stacked like the board (bottom side first), in the board's colours.
//! Negative files (solder mask) are drawn inverted: mask everywhere but the openings.

use std::path::{Path, PathBuf};

use gerber::reader::{Image, Prim, parse_excellon, parse_gerber};
use gerber::{FileKind, Manifest, read_manifest};
use i_overlay::core::fill_rule::FillRule;
use i_overlay::core::overlay_rule::OverlayRule;
use i_overlay::float::single::SingleFloatOverlay;
use model::{Coord, LayerId, Point, Rect};
use render::{Color, Instance, Scene};
use rstar::{AABB, RTree, RTreeObject};

use crate::draw::mmf;
use crate::style::{board_layer_color, pal};
use crate::view::LayerEntry;

type RLayer = render::LayerId;

/// One file of the export.
pub struct GerberFile {
    pub name: String,
    /// What it holds ("Top Copper", "Plated Drill").
    pub label: String,
    /// The board layer it maps to (`None`: drills or unknown).
    pub layer: Option<LayerId>,
    pub drill: bool,
    pub image: Image,
    /// Its scene layer.
    pub render_layer: RLayer,
}

struct Hit {
    file: usize,
    object: usize,
    env: AABB<[f64; 2]>,
}

impl RTreeObject for Hit {
    type Envelope = AABB<[f64; 2]>;
    fn envelope(&self) -> Self::Envelope {
        self.env
    }
}

pub struct GerberView {
    /// The export folder.
    pub dir: PathBuf,
    pub scene: Scene,
    pub files: Vec<GerberFile>,
    /// Files that couldn't be read: name and reason.
    pub errors: Vec<(String, String)>,
    /// The manifest, if the folder has one.
    pub manifest: Option<Manifest>,
    index: RTree<Hit>,
}

/// The board layer of an X2 `.FileFunction` value.
pub fn layer_of_function(function: &str, copper_count: usize) -> Option<LayerId> {
    let f: Vec<&str> = function.split(',').collect();
    let top = |i: usize| f.get(i).is_some_and(|s| *s == "Top");
    match f[0] {
        "Copper" => {
            let n: usize = f.get(1)?.strip_prefix('L')?.parse().ok()?;
            match *f.get(2)? {
                "Top" => Some(LayerId::TOP_COPPER),
                "Bot" => Some(LayerId::BOTTOM_COPPER),
                _ if n >= 2 && n < copper_count.max(3) => Some(LayerId::inner(n as u16 - 1)),
                _ => None,
            }
        }
        "Soldermask" => Some(if top(1) { LayerId::TOP_MASK } else { LayerId::BOTTOM_MASK }),
        "Paste" => Some(if top(1) { LayerId::TOP_PASTE } else { LayerId::BOTTOM_PASTE }),
        "Legend" => Some(if top(1) { LayerId::TOP_SILK } else { LayerId::BOTTOM_SILK }),
        "Profile" => Some(LayerId::OUTLINE),
        _ => None,
    }
}

/// Stacking order, bottom first: bottom technical layers, copper from the bottom up,
/// drills, top technical layers, the outline, unknown files.
fn rank(file: &GerberFile) -> u32 {
    match file.layer {
        Some(LayerId::BOTTOM_PASTE) => 0,
        Some(LayerId::BOTTOM_MASK) => 1,
        Some(LayerId::BOTTOM_SILK) => 2,
        Some(l) if l.is_copper() => 10 + (63 - l.0 as u32),
        _ if file.drill => 90,
        Some(LayerId::TOP_MASK) => 91,
        Some(LayerId::TOP_PASTE) => 92,
        Some(LayerId::TOP_SILK) => 93,
        Some(LayerId::OUTLINE) => 95,
        _ => 96,
    }
}

impl GerberView {
    /// Read the files of an export folder: those listed in its manifest, else every
    /// `.gbr`, `.gtl`, … and `.drl` file in it.
    pub fn load(dir: &Path) -> std::io::Result<GerberView> {
        let manifest = dir.parent().and_then(Path::parent).and_then(read_manifest);
        let mut listed: Vec<(String, Option<String>, Option<FileKind>)> = match &manifest {
            Some(m) => m.files.iter().map(|f| (f.name.clone(), Some(f.label.clone()), Some(f.kind))).collect(),
            None => {
                let mut names: Vec<String> = std::fs::read_dir(dir)?
                    .filter_map(|e| e.ok())
                    .map(|e| e.file_name().to_string_lossy().into_owned())
                    .filter(|n| is_fab_file(n))
                    .collect();
                names.sort();
                names.into_iter().map(|n| (n, None, None)).collect()
            }
        };
        let mut files = Vec::new();
        let mut errors = Vec::new();
        let copper_count =
            listed.iter().filter(|(_, _, k)| matches!(k, Some(FileKind::Layer(l)) if l.is_copper())).count();
        for (name, label, kind) in listed.drain(..) {
            let text = match std::fs::read_to_string(dir.join(&name)) {
                Ok(t) => t,
                Err(e) => {
                    errors.push((name, e.to_string()));
                    continue;
                }
            };
            let drill = matches!(kind, Some(FileKind::PlatedDrill | FileKind::NonPlatedDrill))
                || (kind.is_none() && is_drill_name(&name));
            let parsed = if drill { parse_excellon(&text) } else { parse_gerber(&text) };
            let image = match parsed {
                Ok(i) => i,
                Err(e) => {
                    errors.push((name, e.to_string()));
                    continue;
                }
            };
            let function = image.file_function().map(str::to_string);
            let layer = match kind {
                Some(FileKind::Layer(l)) => Some(l),
                _ if drill => None,
                _ => function.as_deref().and_then(|f| layer_of_function(f, copper_count)),
            };
            let label = label.or(function).unwrap_or_else(|| name.clone());
            files.push(GerberFile { name, label, layer, drill, image, render_layer: 0 });
        }
        files.sort_by_key(rank);
        let mut view =
            GerberView { dir: dir.to_path_buf(), scene: Scene::new(), files, errors, manifest, index: RTree::new() };
        view.rebuild();
        Ok(view)
    }

    /// Draw everything again (after a theme change); layer visibility is kept.
    pub fn rebuild(&mut self) {
        let visible: Vec<bool> = (0..self.scene.layer_count() as u16).map(|i| self.scene.layer(i).visible).collect();
        let mut scene = Scene::new();
        let area = self.board_area();
        let mut hits = Vec::new();
        for (i, file) in self.files.iter_mut().enumerate() {
            file.render_layer = scene.add_layer(file.label.clone());
            let color = file_color(file);
            let mut instances = Vec::new();
            if file.image.negative() {
                if let Some(area) = area {
                    instances = inverted(&file.image, area, color);
                }
            } else {
                for o in &file.image.objects {
                    let c = if o.dark { color } else { pal().background };
                    for p in &o.prims {
                        push_prim(p, c, &mut instances);
                    }
                }
            }
            scene.set_item(i as u64, instances.into_iter().map(|inst| (file.render_layer, inst)));
            for (j, o) in file.image.objects.iter().enumerate() {
                if let Some(b) = Rect::union_all(o.prims.iter().map(Prim::bounds)) {
                    hits.push(Hit { file: i, object: j, env: AABB::from_corners(mm(b.min), mm(b.max)) });
                }
            }
        }
        for (i, v) in visible.into_iter().enumerate().take(scene.layer_count()) {
            scene.layer_mut(i as u16).visible = v;
        }
        self.scene = scene;
        self.index = RTree::bulk_load(hits);
    }

    /// The area mask layers cover: the outline file's extent, else everything's.
    fn board_area(&self) -> Option<Rect> {
        let outline = self.files.iter().find(|f| f.layer == Some(LayerId::OUTLINE)).and_then(|f| f.image.bounds());
        outline.or_else(|| Rect::union_all(self.files.iter().filter_map(|f| f.image.bounds())))
    }

    /// The layers popup: one entry per file, in stacking order from the top.
    pub fn layer_menu(&self) -> Vec<LayerEntry> {
        let mut v: Vec<LayerEntry> = self
            .files
            .iter()
            .rev()
            .map(|f| LayerEntry {
                layer: f.render_layer,
                color: Some(file_color(f)),
                group: if f.drill { 1 } else { u8::from(f.layer.is_none()) * 2 },
            })
            .collect();
        v.sort_by_key(|e| e.group);
        v
    }

    /// Show only the file `name` (or all files for `None`).
    pub fn show_only(&mut self, name: Option<&str>) {
        for f in &self.files {
            self.scene.layer_mut(f.render_layer).visible = name.is_none_or(|n| n == f.name);
        }
    }

    /// What lies under `p` on the topmost visible file: file, aperture and its function.
    pub fn describe(&self, p: [f64; 2]) -> Option<String> {
        let q = Point::mm(p[0], p[1]);
        let hit = self
            .index
            .locate_in_envelope_intersecting(AABB::from_point(p))
            .filter(|h| self.scene.layer(self.files[h.file].render_layer).visible)
            .filter(|h| self.files[h.file].image.objects[h.object].prims.iter().any(|pr| pr.contains(q)))
            .max_by_key(|h| (rank(&self.files[h.file]), h.object))?;
        let file = &self.files[hit.file];
        let object = &file.image.objects[hit.object];
        let what = match object.dcode {
            None => "region".to_string(),
            Some(d) if file.drill => {
                let size = file.image.apertures.get(&d).map(|a| a.template.clone()).unwrap_or_default();
                format!("hole, tool {size} mm")
            }
            Some(d) => match file.image.apertures.get(&d) {
                Some(a) => match &a.function {
                    Some(f) => format!("D{d} {} ({f})", a.template),
                    None => format!("D{d} {}", a.template),
                },
                None => format!("D{d}"),
            },
        };
        Some(format!("{} · {what}", file.name))
    }
}

fn is_drill_name(name: &str) -> bool {
    let lower = name.to_lowercase();
    [".drl", ".xln", ".exc", ".txt"].iter().any(|e| lower.ends_with(e))
}

fn is_fab_file(name: &str) -> bool {
    let lower = name.to_lowercase();
    is_drill_name(name)
        || [".gbr", ".ger", ".gtl", ".gbl", ".gts", ".gbs", ".gtp", ".gbp", ".gto", ".gbo", ".gm1", ".gko"]
            .iter()
            .any(|e| lower.ends_with(e))
        || lower.ends_with(".g2")
        || lower.ends_with(".g3")
}

fn mm(p: Point) -> [f64; 2] {
    [p.x as f64 * 1e-6, p.y as f64 * 1e-6]
}

fn len(c: Coord) -> f32 {
    (c as f64 * 1e-6) as f32
}

fn file_color(f: &GerberFile) -> Color {
    match f.layer {
        _ if f.drill => pal().hole,
        Some(l) => board_layer_color(l),
        None => pal().other_layer,
    }
}

fn push_prim(p: &Prim, color: Color, out: &mut Vec<Instance>) {
    match p {
        Prim::Circle { center, diameter } => out.push(Instance::circle(mmf(*center), len(*diameter) / 2.0, color)),
        Prim::Line { a, b, width } => out.push(Instance::line(mmf(*a), mmf(*b), len(*width), color)),
        Prim::Rect { center, w, h, radius, rotation } => out.push(Instance::rect(
            mmf(*center),
            [len(*w), len(*h)],
            len(*radius),
            rotation.to_radians() as f32,
            color,
        )),
        Prim::Polygon(points) => {
            // Cut-in lines make the contour self-touching: resolve it into shapes first.
            let contour: Vec<[f64; 2]> = points.iter().map(|q| [q.x as f64, q.y as f64]).collect();
            let shapes = vec![contour].overlay(&Vec::<Vec<[f64; 2]>>::new(), OverlayRule::Subject, FillRule::NonZero);
            for shape in shapes {
                let contours: Vec<Vec<[f32; 2]>> = shape
                    .iter()
                    .map(|c| c.iter().map(|q| [(q[0] * 1e-6) as f32, (q[1] * 1e-6) as f32]).collect())
                    .collect();
                out.extend(render::tess::fill_region(&contours, color));
            }
        }
    }
}

/// A negative image as positive: `area` minus everything drawn.
fn inverted(image: &Image, area: Rect, color: Color) -> Vec<Instance> {
    let rect = vec![
        [area.min.x as f64, area.min.y as f64],
        [area.max.x as f64, area.min.y as f64],
        [area.max.x as f64, area.max.y as f64],
        [area.min.x as f64, area.max.y as f64],
    ];
    let openings: Vec<Vec<[f64; 2]>> = image
        .objects
        .iter()
        .filter(|o| o.dark)
        .flat_map(|o| o.prims.iter().flat_map(|p| p.outline(32)))
        .map(|c| c.into_iter().map(|q| [q.x as f64, q.y as f64]).collect())
        .collect();
    let shapes = vec![rect].overlay(&openings, OverlayRule::Difference, FillRule::NonZero);
    let mut out = Vec::new();
    for shape in shapes {
        let contours: Vec<Vec<[f32; 2]>> =
            shape.iter().map(|c| c.iter().map(|q| [(q[0] * 1e-6) as f32, (q[1] * 1e-6) as f32]).collect()).collect();
        out.extend(render::tess::fill_region(&contours, color));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_functions() {
        assert_eq!(layer_of_function("Copper,L1,Top", 4), Some(LayerId::TOP_COPPER));
        assert_eq!(layer_of_function("Copper,L3,Inr", 4), Some(LayerId::inner(2)));
        assert_eq!(layer_of_function("Copper,L4,Bot", 4), Some(LayerId::BOTTOM_COPPER));
        assert_eq!(layer_of_function("Soldermask,Bot", 2), Some(LayerId::BOTTOM_MASK));
        assert_eq!(layer_of_function("Profile,NP", 2), Some(LayerId::OUTLINE));
        assert_eq!(layer_of_function("AssemblyDrawing,Top", 2), None);
    }
}
