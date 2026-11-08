// Canvas shaders: background grid and instanced SDF primitives.
// World units are millimeters, Y up.

struct View {
    center: vec2<f32>,
    // NDC units per world unit (x, y).
    ndc_scale: vec2<f32>,
    // World units per pixel.
    px: f32,
    // Minor grid spacing in world units (0 = no grid).
    grid: f32,
    // Major grid line every N minor steps.
    grid_major: f32,
    _pad: f32,
    bg: vec4<f32>,
    grid_color: vec4<f32>,
    axis_color: vec4<f32>,
};

@group(0) @binding(0) var<uniform> view: View;

// ---------------------------------------------------------------------------------------
// Background + grid (one fullscreen triangle)

struct GridOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) world: vec2<f32>,
};

@vertex
fn vs_grid(@builtin(vertex_index) vi: u32) -> GridOut {
    let ndc = vec2<f32>(f32((vi << 1u) & 2u) * 2.0 - 1.0, f32(vi & 2u) * 2.0 - 1.0);
    var out: GridOut;
    out.pos = vec4<f32>(ndc, 0.0, 1.0);
    out.world = view.center + ndc / view.ndc_scale;
    return out;
}

fn line_alpha(coord: f32, spacing: f32, half_width_px: f32) -> f32 {
    let d = abs(coord - spacing * round(coord / spacing)) / view.px;
    return clamp(half_width_px + 0.5 - d, 0.0, 1.0);
}

@fragment
fn fs_grid(in: GridOut) -> @location(0) vec4<f32> {
    var color = view.bg.rgb;
    if (view.grid > 0.0) {
        // Minor grid: dots.
        let g = view.grid;
        let q = in.world - g * round(in.world / g);
        let dot = clamp(1.5 - length(q) / view.px, 0.0, 1.0);
        // Major grid: thin lines.
        let major = g * view.grid_major;
        let ml = max(line_alpha(in.world.x, major, 0.0), line_alpha(in.world.y, major, 0.0)) * 0.35;
        color = mix(color, view.grid_color.rgb, max(dot * view.grid_color.a, ml * view.grid_color.a));
    }
    // Origin axes.
    let ax = max(line_alpha(in.world.x, 1e9, 0.0), line_alpha(in.world.y, 1e9, 0.0));
    color = mix(color, view.axis_color.rgb, ax * view.axis_color.a);
    return vec4<f32>(color, 1.0);
}

// ---------------------------------------------------------------------------------------
// Instanced primitives

struct InstanceIn {
    @location(0) p0: vec2<f32>,
    @location(1) p1: vec2<f32>,
    @location(2) r: f32,
    @location(3) r2: f32,
    @location(4) kind: u32,
    @location(5) color: vec4<f32>,
};

struct PrimOut {
    @builtin(position) pos: vec4<f32>,
    // Fragment position in the primitive's local frame (world units).
    @location(0) local: vec2<f32>,
    @location(1) @interpolate(flat) params: vec4<f32>,
    @location(2) @interpolate(flat) kind: u32,
    @location(3) @interpolate(flat) color: vec4<f32>,
};

const KIND_CAPSULE: u32 = 0u;
const KIND_CIRCLE: u32 = 1u;
const KIND_RECT: u32 = 2u;
const KIND_ARC: u32 = 3u;

fn rotate(v: vec2<f32>, a: f32) -> vec2<f32> {
    let c = cos(a);
    let s = sin(a);
    return vec2<f32>(v.x * c - v.y * s, v.x * s + v.y * c);
}

@vertex
fn vs_prim(@builtin(vertex_index) vi: u32, inst: InstanceIn) -> PrimOut {
    // Quad corner in [-1, 1]^2, triangle strip order.
    let c = vec2<f32>(f32(vi & 1u) * 2.0 - 1.0, f32((vi >> 1u) & 1u) * 2.0 - 1.0);
    // Anti-aliasing margin.
    let m = view.px;
    // Strokes never get thinner than one pixel so they stay visible when zoomed out.
    let min_r = 0.5 * view.px;

    var out: PrimOut;
    out.kind = inst.kind;
    out.color = inst.color;
    var world = vec2<f32>(0.0);

    switch inst.kind {
        case KIND_CAPSULE: {
            let d = inst.p1 - inst.p0;
            let len = length(d);
            var dir = vec2<f32>(1.0, 0.0);
            if (len > 1e-9) {
                dir = d / len;
            }
            let n = vec2<f32>(-dir.y, dir.x);
            let hw = max(inst.r, min_r);
            let e = hw + m;
            let u = select(-e, len + e, c.x > 0.0);
            let v = c.y * e;
            world = inst.p0 + dir * u + n * v;
            out.local = vec2<f32>(u, v);
            out.params = vec4<f32>(len, hw, 0.0, 0.0);
        }
        case KIND_CIRCLE: {
            let r = max(inst.r, min_r);
            out.local = c * (r + m);
            world = inst.p0 + out.local;
            // Keep rings at least one pixel wide.
            let inner = min(inst.r2, r - view.px);
            out.params = vec4<f32>(r, inner, 0.0, 0.0);
        }
        case KIND_RECT: {
            let hs = max(inst.p1, vec2<f32>(min_r));
            out.local = c * (hs + vec2<f32>(m));
            world = inst.p0 + rotate(out.local, inst.r2);
            out.params = vec4<f32>(hs, min(inst.r, min(hs.x, hs.y)), 0.0);
        }
        case KIND_ARC: {
            let hw = max(inst.r, min_r);
            let q = c * (inst.r2 + hw + m);
            world = inst.p0 + q;
            // Rotate so the arc's middle points along +y (symmetric arc SDF).
            let mid = inst.p1.x + 0.5 * inst.p1.y;
            out.local = rotate(q, 1.5707963 - mid);
            let ta = min(abs(inst.p1.y) * 0.5, 3.1415927);
            out.params = vec4<f32>(sin(ta), cos(ta), inst.r2, hw);
        }
        default: {
            // Empty slot: degenerate quad, no fragments.
            out.pos = vec4<f32>(2.0, 2.0, 0.0, 1.0);
            out.local = vec2<f32>(0.0);
            out.params = vec4<f32>(0.0);
            return out;
        }
    }

    out.pos = vec4<f32>((world - view.center) * view.ndc_scale, 0.0, 1.0);
    return out;
}

@fragment
fn fs_prim(in: PrimOut) -> @location(0) vec4<f32> {
    var d = 0.0;
    switch in.kind {
        case KIND_CAPSULE: {
            let u = in.local.x - clamp(in.local.x, 0.0, in.params.x);
            d = length(vec2<f32>(u, in.local.y)) - in.params.y;
        }
        case KIND_CIRCLE: {
            let l = length(in.local);
            d = l - in.params.x;
            if (in.params.y > 0.0) {
                d = max(d, in.params.y - l);
            }
        }
        case KIND_RECT: {
            let cr = in.params.z;
            let q = abs(in.local) - in.params.xy + vec2<f32>(cr);
            d = length(max(q, vec2<f32>(0.0))) + min(max(q.x, q.y), 0.0) - cr;
        }
        case KIND_ARC: {
            var p = in.local;
            p.x = abs(p.x);
            let sc = in.params.xy;
            let ra = in.params.z;
            if (sc.y * p.x > sc.x * p.y) {
                d = length(p - sc * ra);
            } else {
                d = abs(length(p) - ra);
            }
            d = d - in.params.w;
        }
        default: {
            discard;
        }
    }
    let coverage = clamp(0.5 - d / view.px, 0.0, 1.0);
    if (coverage <= 0.0) {
        discard;
    }
    let a = in.color.a * coverage;
    // Premultiplied alpha output.
    return vec4<f32>(in.color.rgb * a, a);
}
