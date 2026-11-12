//! Gerber X2 (RS-274X) and Excellon reader, for showing exported files as they are on
//! disk. Everything is resolved to primitives in board coordinates (nm): flashes and
//! macros become circles, rectangles and polygons, arcs become line segments.
//!
//! Supported: FS (leading or trailing zero omission), MO, AD (C, R, O, P and macros), AM
//! (primitives 1, 4, 5, 20, 21 with expressions and variables), D01/D02/D03, G01/G02/G03
//! with G75, G36/G37, LP, LR, LS, TF/TA/TO/TD, deprecated G54/G70/G71/G90. Not supported
//! (warned about): G74 single quadrant arcs, G91, LM, thermal primitives, exposure-off
//! macro primitives.

use std::collections::HashMap;

use model::{Coord, Point};

/// A shape in board coordinates.
#[derive(Clone, Debug, PartialEq)]
pub enum Prim {
    Circle {
        center: Point,
        diameter: Coord,
    },
    /// A stroke with round ends.
    Line {
        a: Point,
        b: Point,
        width: Coord,
    },
    /// Rectangle around `center` with rounded corners, turned by `rotation` degrees.
    Rect {
        center: Point,
        w: Coord,
        h: Coord,
        radius: Coord,
        rotation: f64,
    },
    /// A filled contour (it may contain cut-in lines).
    Polygon(Vec<Point>),
}

impl Prim {
    pub fn bounds(&self) -> model::Rect {
        use model::Rect;
        match self {
            Prim::Circle { center, diameter } => Rect::around(*center, diameter / 2),
            Prim::Line { a, b, width } => {
                let r = Rect::from_points(*a, *b);
                Rect { min: r.min - Point::new(width / 2, width / 2), max: r.max + Point::new(width / 2, width / 2) }
            }
            Prim::Rect { center, w, h, .. } => {
                Rect::around(*center, ((*w as f64).hypot(*h as f64) / 2.0).ceil() as Coord)
            }
            Prim::Polygon(p) => {
                Rect::union_all(p.iter().map(|&q| Rect::from_points(q, q))).unwrap_or(Rect::around(Point::ZERO, 0))
            }
        }
    }

    /// Is `p` covered (edges included)?
    pub fn contains(&self, p: Point) -> bool {
        let f = |q: Point| (q.x as f64, q.y as f64);
        let (px, py) = f(p);
        match self {
            Prim::Circle { center, diameter } => {
                let (cx, cy) = f(*center);
                (px - cx).hypot(py - cy) <= *diameter as f64 / 2.0
            }
            Prim::Line { a, b, width } => {
                let ((ax, ay), (bx, by)) = (f(*a), f(*b));
                let (dx, dy) = (bx - ax, by - ay);
                let len2 = dx * dx + dy * dy;
                let t = if len2 == 0.0 { 0.0 } else { (((px - ax) * dx + (py - ay) * dy) / len2).clamp(0.0, 1.0) };
                (ax + t * dx - px).hypot(ay + t * dy - py) <= *width as f64 / 2.0
            }
            Prim::Rect { center, w, h, radius, rotation } => {
                let (cx, cy) = f(*center);
                let (s, c) = (-rotation).to_radians().sin_cos();
                let (x, y) = (px - cx, py - cy);
                let (lx, ly) = ((x * c - y * s).abs(), (x * s + y * c).abs());
                let r = *radius as f64;
                let (ex, ey) = (*w as f64 / 2.0 - r, *h as f64 / 2.0 - r);
                let (qx, qy) = ((lx - ex).max(0.0), (ly - ey).max(0.0));
                qx.hypot(qy) <= r + 1e-6 && lx <= *w as f64 / 2.0 + 1e-6 && ly <= *h as f64 / 2.0 + 1e-6
            }
            Prim::Polygon(pts) => {
                // Non-zero winding (cut-in lines cancel out).
                let mut wn = 0;
                for i in 0..pts.len() {
                    let ((ax, ay), (bx, by)) = (f(pts[i]), f(pts[(i + 1) % pts.len()]));
                    let cross = (bx - ax) * (py - ay) - (px - ax) * (by - ay);
                    if ay <= py && by > py && cross > 0.0 {
                        wn += 1;
                    } else if ay > py && by <= py && cross < 0.0 {
                        wn -= 1;
                    }
                }
                wn != 0
            }
        }
    }

    /// An outline polygon (circles and round ends with `segments` per full turn).
    pub fn outline(&self, segments: usize) -> Vec<Vec<Point>> {
        let arc = |c: Point, r: f64, from: f64, sweep: f64, n: usize, out: &mut Vec<Point>| {
            for i in 0..=n {
                let t = (from + sweep * i as f64 / n as f64).to_radians();
                out.push(c + Point::new((r * t.cos()).round() as Coord, (r * t.sin()).round() as Coord));
            }
        };
        match self {
            Prim::Circle { center, diameter } => {
                let mut v = Vec::new();
                arc(*center, *diameter as f64 / 2.0, 0.0, 360.0, segments, &mut v);
                v.pop();
                vec![v]
            }
            Prim::Line { a, b, width } => {
                let dir = (b.y - a.y) as f64;
                let angle = dir.atan2((b.x - a.x) as f64).to_degrees();
                let r = *width as f64 / 2.0;
                let mut v = Vec::new();
                arc(*b, r, angle - 90.0, 180.0, segments / 2, &mut v);
                arc(*a, r, angle + 90.0, 180.0, segments / 2, &mut v);
                vec![v]
            }
            Prim::Rect { center, w, h, radius, rotation } => {
                let (x, y, r) = ((w / 2 - radius) as f64, (h / 2 - radius) as f64, *radius as f64);
                let mut local = Vec::new();
                let n = (segments / 4).max(1);
                for (cx, cy, from) in [(x, -y, 270.0), (x, y, 0.0), (-x, y, 90.0), (-x, -y, 180.0)] {
                    if r > 0.0 {
                        for i in 0..=n {
                            let t = (from + 90.0 * i as f64 / n as f64).to_radians();
                            local.push((cx + r * t.cos(), cy + r * t.sin()));
                        }
                    } else {
                        local.push((cx, cy));
                    }
                }
                let (s, c) = rotation.to_radians().sin_cos();
                vec![
                    local
                        .into_iter()
                        .map(|(x, y)| {
                            *center + Point::new((x * c - y * s).round() as Coord, (x * s + y * c).round() as Coord)
                        })
                        .collect(),
                ]
            }
            Prim::Polygon(p) => vec![p.clone()],
        }
    }
}

/// One drawing operation (a flash, a stroke or a region) and its primitives.
#[derive(Clone, Debug, PartialEq)]
pub struct Object {
    /// Dark (adds) or clear (erases what lies below) polarity.
    pub dark: bool,
    /// The aperture used (`None` for regions).
    pub dcode: Option<u32>,
    pub prims: Vec<Prim>,
    /// Where a flash (or a drill hole) sits.
    pub flash: Option<Point>,
}

impl Object {
    /// Points worth measuring from: a flash's centre, a stroke's ends.
    pub fn snap_points(&self) -> Vec<Point> {
        if let Some(p) = self.flash {
            return vec![p];
        }
        let ends = |p: &Prim| match p {
            Prim::Line { a, b, .. } => Some((*a, *b)),
            _ => None,
        };
        match (self.dcode, self.prims.first().and_then(ends), self.prims.last().and_then(ends)) {
            (Some(_), Some((a, _)), Some((_, b))) => vec![a, b],
            _ => Vec::new(),
        }
    }
}

/// An aperture as defined in the file (for hover readouts).
#[derive(Clone, Debug, PartialEq)]
pub struct ApertureInfo {
    /// Template and parameters as written, e.g. `C,0.25`.
    pub template: String,
    /// `.AperFunction` attribute value, if any.
    pub function: Option<String>,
}

/// A parsed file.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Image {
    /// X2 file attributes without the `%TF`, e.g. `.FileFunction,Copper,L1,Top`.
    pub attributes: Vec<String>,
    pub objects: Vec<Object>,
    pub apertures: HashMap<u32, ApertureInfo>,
    /// Problems that were skipped over.
    pub warnings: Vec<String>,
}

impl Image {
    /// The `.FileFunction` value, e.g. `Copper,L1,Top`.
    pub fn file_function(&self) -> Option<&str> {
        self.attributes.iter().find_map(|a| a.strip_prefix(".FileFunction,"))
    }

    /// `.FilePolarity,Negative`: drawn objects are where the material is *not*.
    pub fn negative(&self) -> bool {
        self.attributes.iter().any(|a| a == ".FilePolarity,Negative")
    }

    pub fn bounds(&self) -> Option<model::Rect> {
        model::Rect::union_all(self.objects.iter().flat_map(|o| o.prims.iter().map(Prim::bounds)))
    }

    /// Is `p` covered by dark objects (clear ones drawn later erase)?
    pub fn covers(&self, p: Point) -> bool {
        let mut covered = false;
        for o in &self.objects {
            if o.prims.iter().any(|q| q.contains(p)) {
                covered = o.dark;
            }
        }
        covered
    }
}

#[derive(Debug, thiserror::Error)]
#[error("line {line}: {message}")]
pub struct ParseError {
    pub line: usize,
    pub message: String,
}

// ---------------------------------------------------------------------------------------
// Expressions (aperture macros)

fn eval(expr: &str, vars: &HashMap<u32, f64>) -> Result<f64, String> {
    let chars: Vec<char> = expr.chars().filter(|c| !c.is_whitespace()).collect();
    let mut pos = 0;
    let v = parse_sum(&chars, &mut pos, vars)?;
    if pos != chars.len() {
        return Err(format!("unexpected {:?} in expression {expr:?}", chars[pos]));
    }
    Ok(v)
}

fn parse_sum(c: &[char], pos: &mut usize, vars: &HashMap<u32, f64>) -> Result<f64, String> {
    let mut v = parse_product(c, pos, vars)?;
    while *pos < c.len() && matches!(c[*pos], '+' | '-') {
        let op = c[*pos];
        *pos += 1;
        let r = parse_product(c, pos, vars)?;
        v = if op == '+' { v + r } else { v - r };
    }
    Ok(v)
}

fn parse_product(c: &[char], pos: &mut usize, vars: &HashMap<u32, f64>) -> Result<f64, String> {
    let mut v = parse_unary(c, pos, vars)?;
    while *pos < c.len() && matches!(c[*pos], 'x' | 'X' | '/') {
        let op = c[*pos];
        *pos += 1;
        let r = parse_unary(c, pos, vars)?;
        v = if op == '/' { v / r } else { v * r };
    }
    Ok(v)
}

fn parse_unary(c: &[char], pos: &mut usize, vars: &HashMap<u32, f64>) -> Result<f64, String> {
    match c.get(*pos) {
        Some('-') => {
            *pos += 1;
            Ok(-parse_unary(c, pos, vars)?)
        }
        Some('+') => {
            *pos += 1;
            parse_unary(c, pos, vars)
        }
        Some('(') => {
            *pos += 1;
            let v = parse_sum(c, pos, vars)?;
            if c.get(*pos) != Some(&')') {
                return Err("missing )".into());
            }
            *pos += 1;
            Ok(v)
        }
        Some('$') => {
            *pos += 1;
            let start = *pos;
            while *pos < c.len() && c[*pos].is_ascii_digit() {
                *pos += 1;
            }
            let n: u32 = c[start..*pos].iter().collect::<String>().parse().map_err(|_| "bad variable")?;
            Ok(vars.get(&n).copied().unwrap_or(0.0))
        }
        Some(_) => {
            let start = *pos;
            while *pos < c.len() && (c[*pos].is_ascii_digit() || c[*pos] == '.') {
                *pos += 1;
            }
            let s: String = c[start..*pos].iter().collect();
            s.parse().map_err(|_| format!("bad number {s:?}"))
        }
        None => Err("unexpected end of expression".into()),
    }
}

// ---------------------------------------------------------------------------------------
// Apertures

#[derive(Clone, Debug)]
enum Template {
    Circle(f64),
    Rect(f64, f64),
    Oval(f64, f64),
    /// Outer diameter, vertices, rotation.
    Poly(f64, u32, f64),
    /// Macro body and its parameters.
    Macro(Vec<String>, Vec<f64>),
}

/// Primitives of an aperture flashed at the origin, in file units (unscaled).
fn aperture_prims(t: &Template, warnings: &mut Vec<String>) -> Vec<LocalPrim> {
    match t {
        Template::Circle(d) => vec![LocalPrim::Circle { c: (0.0, 0.0), d: *d }],
        Template::Rect(w, h) => vec![LocalPrim::Rect { c: (0.0, 0.0), w: *w, h: *h, r: 0.0, rot: 0.0 }],
        Template::Oval(w, h) => vec![LocalPrim::Rect { c: (0.0, 0.0), w: *w, h: *h, r: w.min(*h) / 2.0, rot: 0.0 }],
        Template::Poly(d, n, rot) => vec![LocalPrim::Poly(regular(*d, *n, *rot, (0.0, 0.0)))],
        Template::Macro(body, params) => {
            let mut vars: HashMap<u32, f64> = params.iter().enumerate().map(|(i, v)| (i as u32 + 1, *v)).collect();
            let mut out = Vec::new();
            for stmt in body {
                if let Err(e) = macro_statement(stmt, &mut vars, &mut out) {
                    warnings.push(e);
                }
            }
            out
        }
    }
}

fn regular(d: f64, n: u32, rot: f64, c: (f64, f64)) -> Vec<(f64, f64)> {
    let n = n.max(3);
    (0..n)
        .map(|i| {
            let t = (rot + 360.0 * i as f64 / n as f64).to_radians();
            (c.0 + d / 2.0 * t.cos(), c.1 + d / 2.0 * t.sin())
        })
        .collect()
}

/// A primitive in aperture-local file units.
#[derive(Clone, Debug)]
enum LocalPrim {
    Circle { c: (f64, f64), d: f64 },
    Rect { c: (f64, f64), w: f64, h: f64, r: f64, rot: f64 },
    Poly(Vec<(f64, f64)>),
}

fn rot((x, y): (f64, f64), deg: f64) -> (f64, f64) {
    if deg == 0.0 {
        return (x, y);
    }
    let (s, c) = deg.to_radians().sin_cos();
    (x * c - y * s, x * s + y * c)
}

fn macro_statement(stmt: &str, vars: &mut HashMap<u32, f64>, out: &mut Vec<LocalPrim>) -> Result<(), String> {
    let stmt = stmt.trim();
    if stmt.is_empty() || stmt.starts_with('0') && (stmt.len() == 1 || stmt.as_bytes()[1] == b' ') {
        return Ok(()); // comment
    }
    if let Some(def) = stmt.strip_prefix('$') {
        let (n, expr) = def.split_once('=').ok_or(format!("bad macro variable {stmt:?}"))?;
        let n: u32 = n.parse().map_err(|_| format!("bad macro variable {stmt:?}"))?;
        let v = eval(expr, vars)?;
        vars.insert(n, v);
        return Ok(());
    }
    let fields: Vec<&str> = stmt.split(',').collect();
    let code: u32 = fields[0].trim().parse().map_err(|_| format!("bad macro primitive {stmt:?}"))?;
    let args: Vec<f64> = fields[1..].iter().map(|f| eval(f, vars)).collect::<Result<_, _>>()?;
    let arg = |i: usize| args.get(i).copied().unwrap_or(0.0);
    if args.first().is_some_and(|&e| e == 0.0) {
        return Err(format!("macro primitive {code} with exposure off is not supported"));
    }
    match code {
        1 => {
            let rotation = arg(4);
            out.push(LocalPrim::Circle { c: rot((arg(2), arg(3)), rotation), d: arg(1) });
        }
        20 => {
            let (a, b) = (rot((arg(2), arg(3)), arg(6)), rot((arg(4), arg(5)), arg(6)));
            let len = (b.0 - a.0).hypot(b.1 - a.1);
            let angle = (b.1 - a.1).atan2(b.0 - a.0).to_degrees();
            out.push(LocalPrim::Rect {
                c: ((a.0 + b.0) / 2.0, (a.1 + b.1) / 2.0),
                w: len,
                h: arg(1),
                r: 0.0,
                rot: angle,
            });
        }
        21 => {
            let rotation = arg(5);
            out.push(LocalPrim::Rect {
                c: rot((arg(3), arg(4)), rotation),
                w: arg(1),
                h: arg(2),
                r: 0.0,
                rot: rotation,
            });
        }
        4 => {
            let n = arg(1) as usize;
            let rotation = arg(4 + 2 * n);
            let pts = (0..=n).map(|i| rot((arg(2 + 2 * i), arg(3 + 2 * i)), rotation)).collect();
            out.push(LocalPrim::Poly(pts));
        }
        // The whole polygon turns around the macro origin.
        5 => out.push(LocalPrim::Poly(
            regular(arg(4), arg(1) as u32, 0.0, (arg(2), arg(3))).into_iter().map(|p| rot(p, arg(5))).collect(),
        )),
        7 => return Err("thermal macro primitives are not supported".into()),
        _ => return Err(format!("unknown macro primitive {code}")),
    }
    Ok(())
}

// ---------------------------------------------------------------------------------------
// Gerber

struct State {
    /// File units → nm.
    scale: f64,
    /// Integer and decimal digits; trailing zeros omitted.
    format: (u32, u32),
    trailing: bool,
    macros: HashMap<String, Vec<String>>,
    templates: HashMap<u32, Template>,
    current: Option<u32>,
    pos: (f64, f64),
    mode: u8, // 1 linear, 2 clockwise, 3 counterclockwise
    region: Option<Vec<Vec<Point>>>,
    dark: bool,
    rotation: f64,
    scaling: f64,
    pending_function: Option<String>,
}

impl State {
    fn coordinate(&self, digits: &str) -> Result<f64, String> {
        if digits.contains('.') {
            return digits.parse::<f64>().map_err(|_| format!("bad coordinate {digits:?}"));
        }
        let (neg, d) = match digits.strip_prefix('-') {
            Some(d) => (true, d),
            None => (false, digits.strip_prefix('+').unwrap_or(digits)),
        };
        let total = (self.format.0 + self.format.1) as usize;
        let d = if self.trailing && d.len() < total { format!("{d:0<total$}") } else { d.to_string() };
        let v: f64 = d.parse::<i64>().map_err(|_| format!("bad coordinate {digits:?}"))? as f64;
        let v = v / 10f64.powi(self.format.1 as i32);
        Ok(if neg { -v } else { v })
    }

    fn nm(&self, p: (f64, f64)) -> Point {
        Point::new((p.0 * self.scale).round() as Coord, (p.1 * self.scale).round() as Coord)
    }

    fn len(&self, v: f64) -> Coord {
        (v * self.scale).round() as Coord
    }
}

/// Split into statements: `%…%` extended commands (one entry per block) and `…*` words.
fn statements(text: &str) -> Vec<(usize, String, bool)> {
    let mut out = Vec::new();
    let mut line = 1;
    let mut chars = text.chars().peekable();
    let mut cur = String::new();
    let mut start = 1;
    while let Some(c) = chars.next() {
        if c == '\n' {
            line += 1;
            continue;
        }
        if c == '\r' {
            continue;
        }
        if c == '%' {
            let begin = line;
            let mut block = String::new();
            for d in chars.by_ref() {
                if d == '\n' {
                    line += 1;
                    block.push('\n');
                    continue;
                }
                if d == '%' {
                    break;
                }
                block.push(d);
            }
            out.push((begin, block, true));
            continue;
        }
        if cur.is_empty() {
            start = line;
        }
        if c == '*' {
            out.push((start, std::mem::take(&mut cur), false));
        } else if !c.is_whitespace() {
            cur.push(c);
        }
    }
    out
}

/// Parse a Gerber file.
pub fn parse_gerber(text: &str) -> Result<Image, ParseError> {
    let mut img = Image::default();
    let mut st = State {
        scale: 1e6,
        format: (4, 6),
        trailing: false,
        macros: HashMap::new(),
        templates: HashMap::new(),
        current: None,
        pos: (0.0, 0.0),
        mode: 1,
        region: None,
        dark: true,
        rotation: 0.0,
        scaling: 1.0,
        pending_function: None,
    };
    for (line, stmt, extended) in statements(text) {
        let r = if extended { extended_command(&mut st, &mut img, &stmt) } else { word(&mut st, &mut img, &stmt) };
        match r {
            Ok(true) => break,
            Ok(false) => {}
            Err(message) => return Err(ParseError { line, message }),
        }
    }
    Ok(img)
}

/// Returns `true` at the end of the file.
fn extended_command(st: &mut State, img: &mut Image, block: &str) -> Result<bool, String> {
    let block = block.trim();
    if let Some(am) = block.strip_prefix("AM") {
        let mut parts = am.split('*').map(|s| s.trim().replace('\n', ""));
        let name = parts.next().unwrap_or_default();
        st.macros.insert(name, parts.filter(|p| !p.is_empty()).collect());
        return Ok(false);
    }
    for stmt in block.split('*').map(|s| s.replace('\n', "")).filter(|s| !s.is_empty()) {
        let s = stmt.as_str();
        if let Some(fs) = s.strip_prefix("FS") {
            st.trailing = fs.starts_with('T');
            let x = fs.find('X').ok_or("FS without X")?;
            let digits: Vec<u32> = fs[x + 1..].chars().take(2).filter_map(|c| c.to_digit(10)).collect();
            if digits.len() == 2 {
                st.format = (digits[0], digits[1]);
            }
        } else if let Some(mo) = s.strip_prefix("MO") {
            st.scale = if mo.starts_with("IN") { 25.4e6 } else { 1e6 };
        } else if let Some(ad) = s.strip_prefix("ADD") {
            let digits = ad.chars().take_while(char::is_ascii_digit).count();
            let code: u32 = ad[..digits].parse().map_err(|_| format!("bad aperture {s:?}"))?;
            let def = &ad[digits..];
            let (name, params) = def.split_once(',').unwrap_or((def, ""));
            let nums: Vec<f64> = if params.is_empty() {
                Vec::new()
            } else {
                params
                    .split('X')
                    .map(|p| p.trim().parse::<f64>().map_err(|_| format!("bad parameter in {s:?}")))
                    .collect::<Result<_, _>>()?
            };
            let n = |i: usize| nums.get(i).copied().unwrap_or(0.0);
            let template = match name {
                "C" => Template::Circle(n(0)),
                "R" => Template::Rect(n(0), n(1)),
                "O" => Template::Oval(n(0), n(1)),
                "P" => Template::Poly(n(0), n(1) as u32, n(2)),
                m => Template::Macro(st.macros.get(m).cloned().ok_or(format!("unknown macro {m:?}"))?, nums.clone()),
            };
            st.templates.insert(code, template);
            img.apertures
                .insert(code, ApertureInfo { template: def.to_string(), function: st.pending_function.clone() });
        } else if let Some(lp) = s.strip_prefix("LP") {
            st.dark = lp.starts_with('D');
        } else if let Some(lr) = s.strip_prefix("LR") {
            st.rotation = lr.parse().map_err(|_| format!("bad {s:?}"))?;
        } else if let Some(ls) = s.strip_prefix("LS") {
            st.scaling = ls.parse().map_err(|_| format!("bad {s:?}"))?;
        } else if let Some(lm) = s.strip_prefix("LM") {
            if lm != "N" {
                img.warnings.push("mirrored apertures (LM) are drawn unmirrored".into());
            }
        } else if let Some(tf) = s.strip_prefix("TF") {
            img.attributes.push(tf.to_string());
        } else if let Some(ta) = s.strip_prefix("TA") {
            if let Some(f) = ta.strip_prefix(".AperFunction,") {
                st.pending_function = Some(f.to_string());
            }
        } else if let Some(td) = s.strip_prefix("TD") {
            if td.is_empty() || td == ".AperFunction" {
                st.pending_function = None;
            }
        } else if s.starts_with("TO") || s.starts_with("IP") || s.starts_with("IN") || s.starts_with("OF") {
            // Object attributes, image polarity/name/offset (deprecated): not needed here.
        } else {
            img.warnings.push(format!("ignored %{s}*%"));
        }
    }
    Ok(false)
}

fn word(st: &mut State, img: &mut Image, w: &str) -> Result<bool, String> {
    let mut w = w;
    if w.starts_with("G04") || w.is_empty() {
        return Ok(false);
    }
    if w == "M02" || w == "M00" || w == "M30" {
        return Ok(true);
    }
    // Leading G codes (G01X…D01 is deprecated but common).
    while let Some(rest) = w.strip_prefix('G') {
        let digits = rest.chars().take_while(char::is_ascii_digit).count();
        let g: u32 = rest[..digits].parse().map_err(|_| format!("bad G code {w:?}"))?;
        w = &rest[digits..];
        match g {
            1 => st.mode = 1,
            2 => st.mode = 2,
            3 => st.mode = 3,
            75 | 54 | 55 | 90 => {}
            70 => st.scale = 25.4e6,
            71 => st.scale = 1e6,
            74 => img.warnings.push("single quadrant arcs (G74) are read as multi quadrant".into()),
            91 => img.warnings.push("incremental coordinates (G91) are not supported".into()),
            36 => st.region = Some(Vec::new()),
            37 => {
                if let Some(contours) = st.region.take() {
                    let prims: Vec<Prim> = contours.into_iter().filter(|c| c.len() >= 3).map(Prim::Polygon).collect();
                    if !prims.is_empty() {
                        img.objects.push(Object { dark: st.dark, dcode: None, prims, flash: None });
                    }
                }
            }
            _ => img.warnings.push(format!("ignored G{g:02}")),
        }
    }
    if w.is_empty() {
        return Ok(false);
    }
    // Aperture selection.
    if let Some(d) = w.strip_prefix('D')
        && let Ok(code) = d.parse::<u32>()
        && code >= 10
    {
        st.current = Some(code);
        return Ok(false);
    }
    // Coordinates and an operation.
    let mut x = None;
    let mut y = None;
    let mut i = 0.0;
    let mut j = 0.0;
    let mut op = None;
    let bytes = w.as_bytes();
    let mut k = 0;
    while k < bytes.len() {
        let letter = bytes[k] as char;
        k += 1;
        let start = k;
        while k < bytes.len() && (bytes[k].is_ascii_digit() || matches!(bytes[k], b'-' | b'+' | b'.')) {
            k += 1;
        }
        let v = &w[start..k];
        match letter {
            'X' => x = Some(st.coordinate(v)?),
            'Y' => y = Some(st.coordinate(v)?),
            'I' => i = st.coordinate(v)?,
            'J' => j = st.coordinate(v)?,
            'D' => op = Some(v.parse::<u32>().map_err(|_| format!("bad D code in {w:?}"))?),
            _ => return Err(format!("unexpected {letter:?} in {w:?}")),
        }
    }
    let target = (x.unwrap_or(st.pos.0), y.unwrap_or(st.pos.1));
    let from = st.pos;
    match op {
        Some(2) => {
            let p = st.nm(target);
            if let Some(region) = &mut st.region {
                region.push(vec![p]);
            }
        }
        Some(1) => {
            let points = if st.mode == 1 {
                vec![from, target]
            } else {
                arc_points(from, target, (from.0 + i, from.1 + j), st.mode == 3)
            };
            let start = st.nm(from);
            let pts: Vec<Point> = points[1..].iter().map(|&p| st.nm(p)).collect();
            if let Some(region) = &mut st.region {
                if region.is_empty() {
                    region.push(vec![start]);
                }
                region.last_mut().expect("a contour").extend(pts);
            } else {
                let code = st.current.ok_or("draw without an aperture")?;
                let width = match st.templates.get(&code) {
                    Some(Template::Circle(d)) => *d,
                    Some(Template::Rect(w, h)) | Some(Template::Oval(w, h)) => w.min(*h),
                    _ => return Err(format!("D{code} can't draw lines")),
                };
                let width = st.len(width * st.scaling);
                let prims = points.windows(2).map(|p| Prim::Line { a: st.nm(p[0]), b: st.nm(p[1]), width }).collect();
                img.objects.push(Object { dark: st.dark, dcode: Some(code), prims, flash: None });
            }
        }
        Some(3) => {
            let code = st.current.ok_or("flash without an aperture")?;
            let template = st.templates.get(&code).ok_or(format!("undefined aperture D{code}"))?.clone();
            let local = aperture_prims(&template, &mut img.warnings);
            let prims = local.into_iter().map(|p| place(st, p, target)).collect();
            img.objects.push(Object { dark: st.dark, dcode: Some(code), prims, flash: Some(st.nm(target)) });
        }
        None => {}
        Some(d) => return Err(format!("unknown operation D{d:02}")),
    }
    st.pos = target;
    Ok(false)
}

/// An aperture primitive flashed at `at`, with the current LR/LS.
fn place(st: &State, p: LocalPrim, at: (f64, f64)) -> Prim {
    let k = st.scaling;
    let xf = |q: (f64, f64)| {
        let q = rot((q.0 * k, q.1 * k), st.rotation);
        st.nm((at.0 + q.0, at.1 + q.1))
    };
    match p {
        LocalPrim::Circle { c, d } => Prim::Circle { center: xf(c), diameter: st.len(d * k) },
        LocalPrim::Rect { c, w, h, r, rot } => Prim::Rect {
            center: xf(c),
            w: st.len(w * k),
            h: st.len(h * k),
            radius: st.len(r * k),
            rotation: rot + st.rotation,
        },
        LocalPrim::Poly(pts) => Prim::Polygon(pts.into_iter().map(xf).collect()),
    }
}

/// Points along an arc from `a` to `b` around `c` (a full circle when `a == b`).
fn arc_points(a: (f64, f64), b: (f64, f64), c: (f64, f64), ccw: bool) -> Vec<(f64, f64)> {
    let r = (a.0 - c.0).hypot(a.1 - c.1);
    let start = (a.1 - c.1).atan2(a.0 - c.0);
    let end = (b.1 - c.1).atan2(b.0 - c.0);
    let mut sweep = end - start;
    let tau = std::f64::consts::TAU;
    if ccw {
        while sweep <= 0.0 {
            sweep += tau;
        }
    } else {
        while sweep >= 0.0 {
            sweep -= tau;
        }
    }
    if (a.0 - b.0).abs() < 1e-9 && (a.1 - b.1).abs() < 1e-9 {
        sweep = if ccw { tau } else { -tau };
    } else if sweep.abs() > tau - 1e-9 {
        sweep = if ccw { sweep - tau } else { sweep + tau };
    }
    let n = ((sweep.abs() / tau * 64.0).ceil() as usize).max(2);
    let mut out: Vec<(f64, f64)> =
        (0..=n).map(|i| start + sweep * i as f64 / n as f64).map(|t| (c.0 + r * t.cos(), c.1 + r * t.sin())).collect();
    out[0] = a;
    *out.last_mut().expect("points") = b;
    out
}

// ---------------------------------------------------------------------------------------
// Excellon

/// Parse an Excellon drill file: every hole becomes a circle (slots are skipped).
pub fn parse_excellon(text: &str) -> Result<Image, ParseError> {
    let mut img = Image::default();
    let mut scale = 1e6;
    let mut tools: HashMap<u32, f64> = HashMap::new();
    let mut current = None;
    let mut header = false;
    let mut pos = (0.0, 0.0);
    // Without decimal points: metric 3.3, inch 2.4, leading zeros kept.
    let implied = |v: &str, scale: f64| -> Option<f64> {
        if v.contains('.') {
            return v.parse().ok();
        }
        let n: f64 = v.parse::<i64>().ok()? as f64;
        Some(if scale > 1e6 { n / 1e4 } else { n / 1e3 })
    };
    for (n, raw) in text.lines().enumerate() {
        let line = raw.trim();
        let err = |message: String| ParseError { line: n + 1, message };
        if let Some(comment) = line.strip_prefix(';') {
            if let Some(attr) = comment.trim().strip_prefix("#@! TF") {
                img.attributes.push(attr.to_string());
            }
            continue;
        }
        match line {
            "M48" => header = true,
            "%" | "M95" => header = false,
            "M30" | "M00" => break,
            _ if line.starts_with("METRIC") => scale = 1e6,
            _ if line.starts_with("INCH") => scale = 25.4e6,
            _ if line.starts_with('T') => {
                let digits = line[1..].chars().take_while(char::is_ascii_digit).count();
                let code: u32 = line[1..1 + digits].parse().map_err(|_| err(format!("bad tool {line:?}")))?;
                if let Some(c) = line.find('C') {
                    let d: String = line[c + 1..].chars().take_while(|c| c.is_ascii_digit() || *c == '.').collect();
                    tools.insert(code, d.parse().map_err(|_| err(format!("bad tool {line:?}")))?);
                } else if !header {
                    current = (code != 0).then_some(code);
                }
            }
            _ if line.starts_with('X') || line.starts_with('Y') => {
                if line.contains("G85") {
                    img.warnings.push("slots (G85) are skipped".into());
                    continue;
                }
                let mut x = pos.0;
                let mut y = pos.1;
                let num = |s: &str| {
                    s.chars().take_while(|c| c.is_ascii_digit() || matches!(c, '-' | '+' | '.')).collect::<String>()
                };
                if let Some(i) = line.find('X') {
                    x = implied(&num(&line[i + 1..]), scale).ok_or_else(|| err(format!("bad coordinate {line:?}")))?;
                }
                if let Some(i) = line.find('Y') {
                    y = implied(&num(&line[i + 1..]), scale).ok_or_else(|| err(format!("bad coordinate {line:?}")))?;
                }
                pos = (x, y);
                let tool = current.ok_or_else(|| err("hole without a tool".into()))?;
                let d = tools.get(&tool).copied().ok_or_else(|| err(format!("undefined tool T{tool}")))?;
                let center = Point::new((x * scale).round() as Coord, (y * scale).round() as Coord);
                img.objects.push(Object {
                    dark: true,
                    dcode: Some(tool),
                    prims: vec![Prim::Circle { center, diameter: (d * scale).round() as Coord }],
                    flash: Some(center),
                });
            }
            _ => {}
        }
    }
    for (code, d) in &tools {
        img.apertures.insert(*code, ApertureInfo { template: format!("T{code} {d}"), function: None });
    }
    Ok(img)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expressions() {
        let vars: HashMap<u32, f64> = [(1, 2.0), (2, 0.5)].into();
        assert_eq!(eval("$1+$1", &vars).unwrap(), 4.0);
        assert_eq!(eval("$2-$1/2", &vars).unwrap(), -0.5);
        assert_eq!(eval("-$1x(1+$2)", &vars).unwrap(), -3.0);
    }

    #[test]
    fn strokes_flashes_and_regions() {
        let text = "%FSLAX46Y46*%\n%MOMM*%\n%ADD10C,0.5*%\n%ADD11R,2X1*%\nD10*\nX0Y0D02*\nX1000000Y0D01*\n\
                    D11*\nX5000000Y5000000D03*\nG36*\nX0Y0D02*\nX1000000Y0D01*\nY1000000D01*\nX0D01*\nG37*\nM02*\n";
        let img = parse_gerber(text).unwrap();
        assert_eq!(img.objects.len(), 3);
        assert_eq!(img.objects[0].prims, vec![Prim::Line { a: Point::ZERO, b: Point::mm(1.0, 0.0), width: mm(0.5) }]);
        assert!(
            matches!(img.objects[1].prims[0], Prim::Rect { center, w, h, .. } if center == Point::mm(5.0, 5.0) && w == mm(2.0) && h == mm(1.0))
        );
        let Prim::Polygon(p) = &img.objects[2].prims[0] else { panic!() };
        assert_eq!(p.len(), 4, "modal coordinates");
    }

    fn mm(v: f64) -> Coord {
        model::mm(v)
    }

    #[test]
    fn inches_and_trailing_zeros() {
        let img = parse_gerber("%FSTAX24Y24*%%MOIN*%%ADD10C,0.01*%D10*X01Y01D03*M02*").unwrap();
        let Prim::Circle { center, diameter } = img.objects[0].prims[0] else { panic!() };
        assert_eq!(center, Point::new(25_400_000, 25_400_000), "01 is 01.0000 inch");
        assert_eq!(diameter, 254_000);
    }

    #[test]
    fn excellon_holes() {
        let img = parse_excellon("M48\nMETRIC\nT1C0.300\n%\nT1\nX1.5Y-2.0\nX3.0\nM30\n").unwrap();
        assert_eq!(img.objects.len(), 2);
        assert_eq!(img.objects[1].prims[0], Prim::Circle { center: Point::mm(3.0, -2.0), diameter: mm(0.3) });
    }
}
