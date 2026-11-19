// Silhouette edge detection — operates on the full-resolution overlay id
// texture (Rg32Uint), which is populated regardless of hit-test picking mode.
// An edge fragment is one where the centre pixel is non-empty and at least
// one of its 4 cardinal neighbours is empty (i.e. background) or carries a
// different `(rep_kind, object_id)` pair.
//
// Thickness is measured in pixels of the bound overlay id texture.
struct SilhouetteParams {
    color: vec4<f32>,
    thickness: f32,
};

@group(0) @binding(0) var id_tex: texture_2d<u32>;
@group(0) @binding(1) var<uniform> params: SilhouetteParams;

struct VsOut {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) vi: u32) -> VsOut {
    var out: VsOut;
    let x = f32(i32(vi & 1u) * 4 - 1);
    let y = f32(i32(vi >> 1u) * 4 - 1);
    out.position = vec4<f32>(x, y, 0.0, 1.0);
    out.uv = vec2<f32>(x * 0.5 + 0.5, 1.0 - (y * 0.5 + 0.5));
    return out;
}

fn fetch(uv: vec2<f32>) -> vec2<u32> {
    let dim = vec2<f32>(textureDimensions(id_tex, 0));
    let coord = clamp(vec2<i32>(uv * dim), vec2<i32>(0), vec2<i32>(dim - vec2<f32>(1.0)));
    return textureLoad(id_tex, coord, 0).rg;
}

fn is_empty(p: vec2<u32>) -> bool {
    return p.x == 0u && p.y == 0u;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let centre = fetch(in.uv);
    if is_empty(centre) {
        discard;
    }
    let step = vec2<f32>(params.thickness) / vec2<f32>(textureDimensions(id_tex, 0));
    let dx = step.x;
    let dy = step.y;

    let p_left = fetch(in.uv + vec2<f32>(-dx, 0.0));
    let p_right = fetch(in.uv + vec2<f32>(dx, 0.0));
    let p_up = fetch(in.uv + vec2<f32>(0.0, -dy));
    let p_down = fetch(in.uv + vec2<f32>(0.0, dy));

    // The top 4 bits of `r` carry rep_kind; the next 12 bits carry object_id.
    // Compare the high 16 bits to detect rep / object boundaries; ignore the
    // low 16 bits (atom_id low half) so silhouettes don't fire between
    // adjacent atoms inside the same object — only at object edges.
    let centre_obj = centre.x & 0xFFFF0000u;
    let is_edge = is_empty(p_left)
        || is_empty(p_right)
        || is_empty(p_up)
        || is_empty(p_down)
        || (p_left.x & 0xFFFF0000u) != centre_obj
        || (p_right.x & 0xFFFF0000u) != centre_obj
        || (p_up.x & 0xFFFF0000u) != centre_obj
        || (p_down.x & 0xFFFF0000u) != centre_obj;

    if !is_edge {
        discard;
    }
    return params.color;
}
