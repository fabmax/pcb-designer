//! Board connectivity: which pads, tracks and vias touch (copper islands), which net each
//! island belongs to (from the schematic), and the ratsnest of unrouted connections.
//!
//! Nets are defined by the schematic only. A pad gets the net of its pin; tracks and vias
//! get the net of the pads in their island. An island with pads of several nets is a
//! short; an island without pads has no net.

use model::*;
use rustc_hash::FxHashMap as HashMap;

use crate::connectivity::Connectivity;
use crate::union_find::UnionFind;

/// A piece of copper.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CopperItem {
    Pad {
        footprint: Id,
        pad: Id,
    },
    Track(Id),
    Via(Id),
    /// One connected piece of a zone's fill.
    Zone {
        zone: Id,
        part: u32,
    },
}

impl CopperItem {
    /// The project object this copper belongs to (a pad: its footprint).
    pub fn object(self) -> ObjectId {
        match self {
            CopperItem::Pad { footprint, .. } => ObjectId::Footprint(footprint),
            CopperItem::Track(t) => ObjectId::Track(t),
            CopperItem::Via(v) => ObjectId::Via(v),
            CopperItem::Zone { zone, .. } => ObjectId::Zone(zone),
        }
    }
}

/// Copper outline in board coordinates: a segment with round ends (tracks, vias, round
/// and oval pads) or a polygon (other pads).
#[derive(Clone, Debug, PartialEq)]
pub enum Geo {
    Capsule {
        a: Point,
        b: Point,
        r: Coord,
    },
    Polygon(Vec<Point>),
    /// Polygon with holes (zone fills).
    Region {
        outer: Vec<Point>,
        holes: Vec<Vec<Point>>,
    },
}

impl Geo {
    pub fn bounds(&self) -> Rect {
        match self {
            Geo::Capsule { a, b, r } => Rect::from_points(*a, *b).grow(*r),
            Geo::Polygon(p) | Geo::Region { outer: p, .. } => {
                Rect::union_all(p.iter().map(|&q| Rect::from_points(q, q)))
                    .unwrap_or(Rect::from_points(Point::ZERO, Point::ZERO))
            }
        }
    }

    /// Do the two shapes touch or overlap?
    pub fn touches(&self, other: &Geo) -> bool {
        match (self, other) {
            (Geo::Capsule { a, b, r }, Geo::Capsule { a: c, b: d, r: s }) => {
                seg_seg_dist(*a, *b, *c, *d) <= (r + s) as f64
            }
            (Geo::Capsule { a, b, r }, Geo::Polygon(p)) | (Geo::Polygon(p), Geo::Capsule { a, b, r }) => {
                capsule_polygon(*a, *b, *r, p)
            }
            (Geo::Polygon(p), Geo::Polygon(q)) => polygon_polygon(p, q),
            (Geo::Region { outer, holes }, other) | (other, Geo::Region { outer, holes }) => {
                // Touches the outline, and isn't entirely inside a hole.
                let o = Geo::Polygon(outer.clone());
                match other {
                    Geo::Region { outer: q, holes: qh } => {
                        o.touches(&Geo::Polygon(q.clone()))
                            && !holes.iter().any(|h| polygon_in_hole(q, h))
                            && !qh.iter().any(|h| polygon_in_hole(outer, h))
                    }
                    _ => o.touches(other) && !holes.iter().any(|h| geo_in_hole(other, h)),
                }
            }
        }
    }

    /// Distance between the shapes (0 when they touch).
    pub fn distance(&self, other: &Geo) -> f64 {
        if self.touches(other) {
            return 0.0;
        }
        let (ea, ra) = self.edges_and_radius();
        let (eb, rb) = other.edges_and_radius();
        let mut d = f64::MAX;
        for &(a, b) in &ea {
            for &(c, e) in &eb {
                d = d.min(seg_seg_dist(a, b, c, e));
            }
        }
        (d - (ra + rb) as f64).max(0.0)
    }

    /// Boundary segments (a capsule is its axis) and the radius around them.
    fn edges_and_radius(&self) -> (Vec<(Point, Point)>, Coord) {
        match self {
            Geo::Capsule { a, b, r } => (vec![(*a, *b)], *r),
            Geo::Polygon(p) => (edges(p).collect(), 0),
            Geo::Region { outer, holes } => (edges(outer).chain(holes.iter().flat_map(|h| edges(h))).collect(), 0),
        }
    }

    /// Does the shape contain `p`?
    pub fn contains(&self, p: Point) -> bool {
        match self {
            Geo::Capsule { a, b, r } => point_seg_dist(p, *a, *b) <= *r as f64,
            Geo::Polygon(poly) => point_in_polygon(p, poly),
            Geo::Region { outer, holes } => {
                point_in_polygon(p, outer)
                    && !holes
                        .iter()
                        .any(|h| point_in_polygon(p, h) && edges(h).all(|(a, b)| point_seg_dist(p, a, b) > 0.5))
            }
        }
    }
}

/// Copper layers as a bit set (bit = [`LayerId`] number).
pub type LayerMask = u64;

pub fn layer_bit(l: LayerId) -> LayerMask {
    if l.is_copper() { 1 << l.0 } else { 0 }
}

#[derive(Clone, Debug)]
pub struct Copper {
    pub item: CopperItem,
    pub layers: LayerMask,
    pub geo: Geo,
    /// Where ratsnest lines attach (pad center, track end, via center).
    pub anchors: Vec<Point>,
    /// Island (set of touching items) index.
    pub island: u32,
    /// Net index into [`BoardConnectivity::nets`].
    pub net: Option<u32>,
    /// Tracks and vias that need attention after schematic changes.
    pub flag: Option<Flag>,
}

/// Why a track or via is flagged. Copper is never deleted because the schematic changed;
/// it is flagged instead, for the user to fix.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Flag {
    /// Its island connects pads of different nets.
    Short,
    /// It touches no pad (e.g. the component it was routed to was deleted).
    Unconnected,
}

/// An unrouted connection.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RatsLine {
    pub net: u32,
    pub a: Point,
    pub b: Point,
}

#[derive(Clone, Debug, Default)]
pub struct BoardConnectivity {
    pub items: Vec<Copper>,
    /// Net names (schematic nets that have pads on the board).
    pub nets: Vec<String>,
    pub ratsnest: Vec<RatsLine>,
    /// Islands connecting pads of different nets: the nets and a point on the island.
    pub shorts: Vec<(Vec<String>, Point)>,
    /// Filled copper of the zones.
    pub fills: Vec<crate::zones::ZoneFill>,
    index: HashMap<CopperItem, usize>,
    net_index: HashMap<String, u32>,
}

impl BoardConnectivity {
    pub fn build(project: &Project, sch: &Connectivity) -> BoardConnectivity {
        // Zone fills need the nets of tracks and vias, and connect copper themselves:
        // first without zones, then with the fills.
        let first = Self::build_with(project, sch, &[]);
        if project.board.zones.is_empty() {
            return first;
        }
        let fills = crate::zones::fill_zones(project, &first);
        let mut bc = Self::build_with(project, sch, &fills);
        bc.fills = fills;
        bc
    }

    fn build_with(project: &Project, sch: &Connectivity, fills: &[crate::zones::ZoneFill]) -> BoardConnectivity {
        let mut bc = BoardConnectivity::default();
        // Pad nets: (component, pad name) → net.
        let mut pad_net: HashMap<(Id, &str), u32> = HashMap::default();
        for net in &sch.nets {
            for pin in &net.pins {
                for pad in &pin.pads {
                    let n = bc.intern(&net.name);
                    pad_net.insert((pin.component, pad.as_str()), n);
                }
            }
        }
        let board = &project.board;
        let all_copper: LayerMask = board.copper_layers().map(|l| layer_bit(l.id)).fold(0, |a, b| a | b);

        // Items.
        let mut pad_nets: Vec<Option<u32>> = Vec::new();
        for f in board.footprints.iter() {
            let Some(fp) = crate::query::footprint_of_instance(project, f) else { continue };
            let t = f.transform();
            let side_copper = layer_bit(LayerId::for_footprint(FootprintLayer::Copper, f.side));
            for pad in fp.pads.iter() {
                let layers = match pad.kind {
                    PadKind::Smd => side_copper,
                    PadKind::ThroughHole { .. } => all_copper,
                    PadKind::NonPlatedHole { .. } => continue,
                };
                let center = t.point(pad.pos);
                bc.items.push(Copper {
                    item: CopperItem::Pad { footprint: f.id, pad: pad.id },
                    layers,
                    geo: pad_geo(pad, &t),
                    anchors: vec![center],
                    island: 0,
                    net: None,
                    flag: None,
                });
                pad_nets.push(pad_net.get(&(f.component, pad.name.as_str())).copied());
            }
        }
        let pads = bc.items.len();
        for tr in board.tracks.iter() {
            bc.items.push(Copper {
                item: CopperItem::Track(tr.id),
                layers: layer_bit(tr.layer),
                geo: Geo::Capsule { a: tr.a, b: tr.b, r: tr.width / 2 },
                anchors: vec![tr.a, tr.b],
                island: 0,
                net: None,
                flag: None,
            });
        }
        for v in board.vias.iter() {
            let (lo, hi) = (v.from.0.min(v.to.0), v.from.0.max(v.to.0));
            let span =
                board.copper_layers().filter(|l| (lo..=hi).contains(&l.id.0)).fold(0, |a, l| a | layer_bit(l.id));
            bc.items.push(Copper {
                item: CopperItem::Via(v.id),
                layers: span,
                geo: Geo::Capsule { a: v.pos, b: v.pos, r: v.diameter / 2 },
                anchors: vec![v.pos],
                island: 0,
                net: None,
                flag: None,
            });
        }
        for fill in fills {
            for (part, shape) in fill.shapes.iter().enumerate() {
                let Some((outer, holes)) = shape.split_first() else { continue };
                bc.items.push(Copper {
                    item: CopperItem::Zone { zone: fill.zone, part: part as u32 },
                    layers: layer_bit(fill.layer),
                    geo: Geo::Region { outer: outer.clone(), holes: holes.to_vec() },
                    anchors: vec![outer[0]],
                    island: 0,
                    net: None,
                    flag: None,
                });
            }
        }
        bc.index = bc.items.iter().enumerate().map(|(i, c)| (c.item, i)).collect();

        // Islands.
        let mut uf = UnionFind::new(bc.items.len());
        let bounds: Vec<Rect> = bc.items.iter().map(|c| c.geo.bounds()).collect();
        // Zone fills can have many thousand vertices: indexed.
        let regions: HashMap<usize, RegionIndex> = (0..bc.items.len())
            .filter_map(|i| match &bc.items[i].geo {
                Geo::Region { outer, holes } => Some((i, RegionIndex::new(outer, holes))),
                _ => None,
            })
            .collect();
        let grid = Grid::new(&bounds);
        grid.pairs(|i, j| {
            let (a, b) = (&bc.items[i], &bc.items[j]);
            if a.layers & b.layers == 0 || !bounds[i].intersects(&bounds[j]) || uf.find(i) == uf.find(j) {
                return;
            }
            let touch = match (regions.get(&i), regions.get(&j)) {
                (Some(r), Some(q)) => r.touches_region(q),
                (Some(r), None) => r.touches(&b.geo),
                (None, Some(q)) => q.touches(&a.geo),
                (None, None) => a.geo.touches(&b.geo),
            };
            if touch {
                uf.union(i, j);
            }
        });
        let mut island_of_root: HashMap<usize, u32> = HashMap::default();
        for i in 0..bc.items.len() {
            let root = uf.find(i);
            let n = island_of_root.len() as u32;
            bc.items[i].island = *island_of_root.entry(root).or_insert(n);
        }

        // Island nets and shorts.
        let islands = island_of_root.len();
        let mut island_nets: Vec<Vec<u32>> = vec![Vec::new(); islands];
        for (i, net) in pad_nets.iter().enumerate() {
            if let Some(n) = *net {
                let v = &mut island_nets[bc.items[i].island as usize];
                if !v.contains(&n) {
                    v.push(n);
                }
            }
        }
        for (i, net) in pad_nets.into_iter().enumerate() {
            bc.items[i].net = net;
        }
        for c in &mut bc.items[pads..] {
            let nets = &island_nets[c.island as usize];
            c.net = nets.first().copied();
            c.flag = match nets.len() {
                _ if matches!(c.item, CopperItem::Zone { .. }) => None,
                0 => Some(Flag::Unconnected),
                1 => None,
                _ => Some(Flag::Short),
            };
        }
        for (island, nets) in island_nets.iter().enumerate() {
            if nets.len() > 1 {
                let at = bc.items.iter().find(|c| c.island as usize == island).map_or(Point::ZERO, |c| c.anchors[0]);
                let mut names: Vec<String> = nets.iter().map(|&n| bc.nets[n as usize].clone()).collect();
                names.sort();
                bc.shorts.push((names, at));
            }
        }

        // Ratsnest per net: anchors of the net's copper, already-connected ones by island.
        let mut by_net: Vec<Vec<usize>> = vec![Vec::new(); bc.nets.len()];
        for (i, c) in bc.items.iter().enumerate() {
            if let Some(n) = c.net {
                by_net[n as usize].push(i);
            }
        }
        for (net, items) in by_net.iter().enumerate() {
            let pads_islands: std::collections::HashSet<u32> =
                items.iter().filter(|&&i| i < pads).map(|&i| bc.items[i].island).collect();
            if pads_islands.len() < 2 {
                continue;
            }
            let mut points = Vec::new();
            let mut island = Vec::new();
            for &i in items.iter().filter(|&&i| !matches!(bc.items[i].item, CopperItem::Zone { .. })) {
                for &p in &bc.items[i].anchors {
                    points.push([p.x as f64, p.y as f64]);
                    island.push(bc.items[i].island);
                }
            }
            for (a, b) in mst(&points, &island) {
                let pt = |k: usize| Point::new(points[k][0] as Coord, points[k][1] as Coord);
                bc.ratsnest.push(RatsLine { net: net as u32, a: pt(a), b: pt(b) });
            }
        }
        bc
    }

    fn intern(&mut self, name: &str) -> u32 {
        if let Some(&n) = self.net_index.get(name) {
            return n;
        }
        let n = self.nets.len() as u32;
        self.nets.push(name.to_string());
        self.net_index.insert(name.to_string(), n);
        n
    }

    pub fn get(&self, item: CopperItem) -> Option<&Copper> {
        self.index.get(&item).map(|&i| &self.items[i])
    }

    /// Net of a pad, track or via.
    pub fn net_of(&self, item: CopperItem) -> Option<&str> {
        self.get(item).and_then(|c| c.net).map(|n| self.nets[n as usize].as_str())
    }

    pub fn net_index(&self, name: &str) -> Option<u32> {
        self.net_index.get(name).copied()
    }

    /// Copper of a net.
    pub fn items_of_net(&self, net: u32) -> impl Iterator<Item = &Copper> {
        self.items.iter().filter(move |c| c.net == Some(net))
    }

    /// Flagged tracks and vias.
    pub fn flagged(&self) -> impl Iterator<Item = &Copper> {
        self.items.iter().filter(|c| c.flag.is_some())
    }

    /// Number of unrouted connections.
    pub fn unrouted(&self) -> usize {
        self.ratsnest.len()
    }

    /// The anchor (pad or via center, track end) on `layer` nearest to `p`, within `tol`.
    pub fn nearest_anchor(&self, p: Point, layer: LayerId, tol: Coord) -> Option<(Point, &Copper)> {
        let bit = layer_bit(layer);
        self.items
            .iter()
            .filter(|c| c.layers & bit != 0 && !matches!(c.item, CopperItem::Zone { .. }))
            .flat_map(|c| c.anchors.iter().map(move |&a| (a, c)))
            .map(|(a, c)| ((a - p).length(), a, c))
            .filter(|(d, _, _)| *d <= tol as f64)
            .min_by(|x, y| x.0.total_cmp(&y.0))
            .map(|(_, a, c)| (a, c))
    }

    /// Copper on `layer` containing `p` (pads first, then vias, then tracks).
    pub fn at(&self, p: Point, layer: LayerId) -> Option<&Copper> {
        let bit = layer_bit(layer);
        let rank = |c: &Copper| match c.item {
            CopperItem::Pad { .. } => 0,
            CopperItem::Via(_) => 1,
            CopperItem::Track(_) => 2,
            CopperItem::Zone { .. } => 3,
        };
        self.items.iter().filter(|c| c.layers & bit != 0 && c.geo.contains(p)).min_by_key(|c| rank(c))
    }
}

/// Outline of a placed pad.
pub fn pad_geo(pad: &Pad, t: &Transform) -> Geo {
    let (w, h) = (pad.size.w, pad.size.h);
    let local = |x: Coord, y: Coord| t.point(pad.pos + Point::new(x, y).rotated(pad.rotation));
    match &pad.shape {
        PadShape::Circle => Geo::Capsule { a: t.point(pad.pos), b: t.point(pad.pos), r: w.min(h) / 2 },
        PadShape::Oval => {
            let r = w.min(h) / 2;
            let (dx, dy) = if w >= h { (w / 2 - r, 0) } else { (0, h / 2 - r) };
            Geo::Capsule { a: local(-dx, -dy), b: local(dx, dy), r }
        }
        PadShape::Rect | PadShape::RoundRect { .. } => {
            let (x, y) = (w / 2, h / 2);
            Geo::Polygon(vec![local(-x, -y), local(x, -y), local(x, y), local(-x, y)])
        }
        PadShape::Polygon { points } => Geo::Polygon(points.iter().map(|&q| local(q.x, q.y)).collect()),
    }
}

// ---------------------------------------------------------------------------------------
// Geometry (f64; coordinates are nm, well within f64 precision)

fn f(p: Point) -> (f64, f64) {
    (p.x as f64, p.y as f64)
}

fn cross(o: (f64, f64), a: (f64, f64), b: (f64, f64)) -> f64 {
    (a.0 - o.0) * (b.1 - o.1) - (a.1 - o.1) * (b.0 - o.0)
}

pub fn point_seg_dist(p: Point, a: Point, b: Point) -> f64 {
    let (p, a, b) = (f(p), f(a), f(b));
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let len2 = dx * dx + dy * dy;
    let t = if len2 == 0.0 { 0.0 } else { (((p.0 - a.0) * dx + (p.1 - a.1) * dy) / len2).clamp(0.0, 1.0) };
    ((a.0 + t * dx - p.0).powi(2) + (a.1 + t * dy - p.1).powi(2)).sqrt()
}

fn segments_cross(a: Point, b: Point, c: Point, d: Point) -> bool {
    let (a, b, c, d) = (f(a), f(b), f(c), f(d));
    let d1 = cross(c, d, a);
    let d2 = cross(c, d, b);
    let d3 = cross(a, b, c);
    let d4 = cross(a, b, d);
    ((d1 > 0.0 && d2 < 0.0) || (d1 < 0.0 && d2 > 0.0)) && ((d3 > 0.0 && d4 < 0.0) || (d3 < 0.0 && d4 > 0.0))
}

pub fn seg_seg_dist(a: Point, b: Point, c: Point, d: Point) -> f64 {
    if segments_cross(a, b, c, d) {
        return 0.0;
    }
    point_seg_dist(a, c, d).min(point_seg_dist(b, c, d)).min(point_seg_dist(c, a, b)).min(point_seg_dist(d, a, b))
}

pub fn point_in_polygon(p: Point, poly: &[Point]) -> bool {
    let n = poly.len();
    if n < 3 {
        return false;
    }
    let (px, py) = f(p);
    let mut inside = false;
    for i in 0..n {
        let (a, b) = (f(poly[i]), f(poly[(i + 1) % n]));
        if (a.1 > py) != (b.1 > py) && px < (b.0 - a.0) * (py - a.1) / (b.1 - a.1) + a.0 {
            inside = !inside;
        }
    }
    // Points on the boundary count as inside.
    inside || (0..n).any(|i| point_seg_dist(p, poly[i], poly[(i + 1) % n]) < 0.5)
}

/// A polygon with holes, with its edges in a grid for fast point and shape queries
/// (zone fills have thousands of vertices).
pub struct RegionIndex {
    edges: Vec<(Point, Point)>,
    first: Point,
    origin: Point,
    cell: Coord,
    nx: i64,
    ny: i64,
    /// Edges per cell (row-major).
    cells: Vec<Vec<u32>>,
}

impl RegionIndex {
    pub fn new(outer: &[Point], holes: &[Vec<Point>]) -> RegionIndex {
        let edges: Vec<(Point, Point)> =
            std::iter::once(outer).chain(holes.iter().map(Vec::as_slice)).flat_map(edges).collect();
        let b = Rect::union_all(outer.iter().map(|&p| Rect::from_points(p, p)))
            .unwrap_or(Rect::from_points(Point::ZERO, Point::ZERO));
        let n = ((edges.len() as f64).sqrt().ceil() as i64).clamp(1, 512);
        let cell = (b.width().max(b.height()) / n).max(1_000) + 1;
        let (nx, ny) = (b.width() / cell + 1, b.height() / cell + 1);
        let mut cells = vec![Vec::new(); (nx * ny) as usize];
        let mut index = RegionIndex {
            edges: Vec::new(),
            first: outer.first().copied().unwrap_or_default(),
            origin: b.min,
            cell,
            nx,
            ny,
            cells: Vec::new(),
        };
        for (k, &(a, c)) in edges.iter().enumerate() {
            index.for_cells(Rect::from_points(a, c), |i| cells[i].push(k as u32));
        }
        index.edges = edges;
        index.cells = cells;
        index
    }

    fn for_cells(&self, r: Rect, mut f: impl FnMut(usize)) {
        let cx = |x: Coord| ((x - self.origin.x) / self.cell).clamp(0, self.nx - 1);
        let cy = |y: Coord| ((y - self.origin.y) / self.cell).clamp(0, self.ny - 1);
        if r.max.x < self.origin.x || r.max.y < self.origin.y {
            return;
        }
        for y in cy(r.min.y)..=cy(r.max.y) {
            for x in cx(r.min.x)..=cx(r.max.x) {
                f((y * self.nx + x) as usize);
            }
        }
    }

    /// Edges whose cells overlap `r` (possibly repeated).
    fn edges_near(&self, r: Rect, mut f: impl FnMut((Point, Point)) -> bool) -> bool {
        let mut hit = false;
        self.for_cells(r, |i| {
            if !hit {
                hit = self.cells[i].iter().any(|&k| f(self.edges[k as usize]));
            }
        });
        hit
    }

    /// Is `p` inside (or on the boundary)? Crossings of a ray to the right, along the
    /// cells of `p`'s row.
    pub fn contains(&self, p: Point) -> bool {
        let row = Rect::from_points(p, Point::new(Coord::MAX / 4, p.y));
        let mut crossings = std::collections::HashSet::new();
        let mut on_edge = false;
        self.for_cells(row, |i| {
            for &k in &self.cells[i] {
                let (a, b) = self.edges[k as usize];
                if point_seg_dist(p, a, b) < 0.5 {
                    on_edge = true;
                }
                let (fa, fb, (px, py)) = (f(a), f(b), f(p));
                if (fa.1 > py) != (fb.1 > py) && px < (fb.0 - fa.0) * (py - fa.1) / (fb.1 - fa.1) + fa.0 {
                    crossings.insert(k);
                }
            }
        });
        on_edge || crossings.len() % 2 == 1
    }

    /// Does the shape overlap the region (not just lie in a hole)?
    pub fn touches(&self, g: &Geo) -> bool {
        let b = g.bounds();
        let crosses = match g {
            Geo::Capsule { a, b: c, r } => self.edges_near(b.grow(1), |(p, q)| seg_seg_dist(*a, *c, p, q) <= *r as f64),
            Geo::Polygon(poly) | Geo::Region { outer: poly, .. } => edges(poly)
                .any(|(p, q)| self.edges_near(Rect::from_points(p, q), |(c, d)| seg_seg_dist(p, q, c, d) < 0.5)),
        };
        if crosses {
            return true;
        }
        // No boundary contact: entirely inside, entirely outside, or the region inside it.
        let probe = match g {
            Geo::Capsule { a, .. } => *a,
            Geo::Polygon(p) | Geo::Region { outer: p, .. } => p[0],
        };
        self.contains(probe) || g.contains(self.first)
    }

    pub fn touches_region(&self, other: &RegionIndex) -> bool {
        other
            .edges
            .iter()
            .any(|&(p, q)| self.edges_near(Rect::from_points(p, q), |(c, d)| seg_seg_dist(p, q, c, d) < 0.5))
            || self.contains(other.first)
            || other.contains(self.first)
    }
}

fn edges(poly: &[Point]) -> impl Iterator<Item = (Point, Point)> + '_ {
    (0..poly.len()).map(move |i| (poly[i], poly[(i + 1) % poly.len()]))
}

/// Is the shape strictly inside the hole (not touching its edge)?
fn geo_in_hole(g: &Geo, hole: &[Point]) -> bool {
    match g {
        Geo::Capsule { a, b, r } => {
            point_in_polygon(*a, hole) && edges(hole).all(|(c, d)| seg_seg_dist(*a, *b, c, d) > *r as f64)
        }
        Geo::Polygon(p) | Geo::Region { outer: p, .. } => polygon_in_hole(p, hole),
    }
}

fn polygon_in_hole(p: &[Point], hole: &[Point]) -> bool {
    p.first().is_some_and(|&v| point_in_polygon(v, hole))
        && edges(p).all(|(a, b)| edges(hole).all(|(c, d)| seg_seg_dist(a, b, c, d) > 0.0))
}

fn capsule_polygon(a: Point, b: Point, r: Coord, poly: &[Point]) -> bool {
    point_in_polygon(a, poly) || edges(poly).any(|(c, d)| seg_seg_dist(a, b, c, d) <= r as f64)
}

fn polygon_polygon(p: &[Point], q: &[Point]) -> bool {
    p.first().is_some_and(|&v| point_in_polygon(v, q))
        || q.first().is_some_and(|&v| point_in_polygon(v, p))
        || edges(p).any(|(a, b)| edges(q).any(|(c, d)| seg_seg_dist(a, b, c, d) == 0.0))
}

// ---------------------------------------------------------------------------------------
// Broad phase, minimum spanning tree

/// Uniform grid over item bounds for finding candidate pairs.
pub(crate) struct Grid {
    cells: HashMap<(i64, i64), Vec<usize>>,
    /// Items too large for the grid; tested against everything.
    large: Vec<usize>,
    n: usize,
}

impl Grid {
    pub(crate) fn new(bounds: &[Rect]) -> Grid {
        let cell: Coord = 2_000_000;
        let mut cells: HashMap<(i64, i64), Vec<usize>> = HashMap::default();
        let mut large = Vec::new();
        for (i, b) in bounds.iter().enumerate() {
            let (x0, y0, x1, y1) = (b.min.x / cell, b.min.y / cell, b.max.x / cell, b.max.y / cell);
            if (x1 - x0 + 1) * (y1 - y0 + 1) > 16_384 {
                large.push(i);
                continue;
            }
            for x in x0..=x1 {
                for y in y0..=y1 {
                    cells.entry((x, y)).or_default().push(i);
                }
            }
        }
        Grid { cells, large, n: bounds.len() }
    }

    /// Calls `f(i, j)` for candidate pairs (possibly more than once per pair).
    pub(crate) fn pairs(&self, mut f: impl FnMut(usize, usize)) {
        for items in self.cells.values() {
            for (k, &i) in items.iter().enumerate() {
                for &j in &items[k + 1..] {
                    f(i, j);
                }
            }
        }
        for &i in &self.large {
            for j in 0..self.n {
                if j != i {
                    f(i, j);
                }
            }
        }
    }
}

/// Minimum spanning tree over `points`, where points of the same `island` are connected
/// already. Returns index pairs of the missing connections.
pub fn mst(points: &[[f64; 2]], island: &[u32]) -> Vec<(usize, usize)> {
    let k = points.len();
    if k < 2 {
        return Vec::new();
    }
    let mut uf = UnionFind::new(k);
    let mut first: HashMap<u32, usize> = HashMap::default();
    for (i, &isl) in island.iter().enumerate() {
        match first.get(&isl) {
            Some(&j) => uf.union(i, j),
            None => {
                first.insert(isl, i);
            }
        }
    }
    let d2 = |a: usize, b: usize| {
        let (p, q) = (points[a], points[b]);
        (p[0] - q[0]).powi(2) + (p[1] - q[1]).powi(2)
    };
    let mut edges: Vec<(f64, usize, usize)> = Vec::new();
    if k <= 32 {
        for a in 0..k {
            for b in a + 1..k {
                if island[a] != island[b] {
                    edges.push((d2(a, b), a, b));
                }
            }
        }
    } else {
        let pts: Vec<delaunator::Point> = points.iter().map(|p| delaunator::Point { x: p[0], y: p[1] }).collect();
        let tri = delaunator::triangulate(&pts);
        for t in tri.triangles.as_chunks::<3>().0 {
            for (a, b) in [(t[0], t[1]), (t[1], t[2]), (t[2], t[0])] {
                edges.push((d2(a, b), a, b));
            }
        }
        // Points delaunator skipped (duplicates, collinear input): nearest neighbour.
        let mut seen = vec![false; k];
        for &i in &tri.triangles {
            seen[i] = true;
        }
        for i in (0..k).filter(|&i| !seen[i]) {
            if let Some(j) = (0..k).filter(|&j| island[j] != island[i]).min_by(|&a, &b| d2(i, a).total_cmp(&d2(i, b))) {
                edges.push((d2(i, j), i, j));
            }
        }
    }
    edges.sort_unstable_by(|a, b| a.0.total_cmp(&b.0));
    let mut out = Vec::new();
    for (_, a, b) in edges {
        let (ra, rb) = (uf.find(a), uf.find(b));
        if ra != rb {
            uf.union(ra, rb);
            out.push((a, b));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shapes_touch() {
        let track = Geo::Capsule { a: Point::mm(0.0, 0.0), b: Point::mm(10.0, 0.0), r: 125_000 };
        let pad =
            Geo::Polygon(vec![Point::mm(9.0, -1.0), Point::mm(11.0, -1.0), Point::mm(11.0, 1.0), Point::mm(9.0, 1.0)]);
        assert!(track.touches(&pad));
        let far = Geo::Polygon(vec![
            Point::mm(20.0, -1.0),
            Point::mm(22.0, -1.0),
            Point::mm(22.0, 1.0),
            Point::mm(20.0, 1.0),
        ]);
        assert!(!track.touches(&far));
        // Crossing tracks on the same layer touch.
        let cross = Geo::Capsule { a: Point::mm(5.0, -5.0), b: Point::mm(5.0, 5.0), r: 125_000 };
        assert!(track.touches(&cross));
        // Parallel with a gap.
        let par = Geo::Capsule { a: Point::mm(0.0, 0.3), b: Point::mm(10.0, 0.3), r: 125_000 };
        assert!(!track.touches(&par));
        let par = Geo::Capsule { a: Point::mm(0.0, 0.25), b: Point::mm(10.0, 0.25), r: 125_000 };
        assert!(track.touches(&par), "edges touching");
        assert!(pad.touches(&pad.clone()));
    }

    #[test]
    fn mst_skips_connected_islands() {
        let pts = [[0.0, 0.0], [1.0, 0.0], [5.0, 0.0]];
        assert_eq!(mst(&pts, &[7, 7, 9]), vec![(1, 2)]);
        let grid: Vec<[f64; 2]> = (0..100).map(|i| [(i % 10) as f64, (i / 10) as f64]).collect();
        let islands: Vec<u32> = (0..100).collect();
        assert_eq!(mst(&grid, &islands).len(), 99);
        let line: Vec<[f64; 2]> = (0..50).map(|i| [i as f64, 0.0]).collect();
        assert_eq!(mst(&line, &(0..50).collect::<Vec<_>>()).len(), 49);
    }
}
