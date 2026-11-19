struct MarkingParams {
    // (1 / width, 1 / height, rim_px, _)
    inv_size_radii: vec4<f32>,
};

@group(0) @binding(0) var<uniform> params: MarkingParams;
@group(0) @binding(1) var scene_tex: texture_2d<f32>;
@group(0) @binding(2) var mask_tex: texture_2d<f32>;

const SELECTED_FILL: vec3<f32> = vec3<f32>(1.0, 0.0, 0.7843);
const SELECTED_RIM: vec3<f32> = vec3<f32>(1.0, 0.18, 0.92);
const HOVER_TINT: vec3<f32> = vec3<f32>(0.42, 0.96, 1.0);
const HOVER_RIM: vec3<f32> = vec3<f32>(0.82, 1.0, 1.0);

@vertex
fn vs_main(@builtin(vertex_index) vid: u32) -> @builtin(position) vec4<f32> {
    let x = f32((vid << 1u) & 2u);
    let y = f32(vid & 2u);
    return vec4<f32>(x * 2.0 - 1.0, 1.0 - y * 2.0, 0.0, 1.0);
}

fn fetch_rg(tex: texture_2d<f32>, coord: vec2<i32>, dims: vec2<u32>) -> vec2<f32> {
    let clamped = clamp(coord, vec2<i32>(0), vec2<i32>(dims) - vec2<i32>(1));
    return textureLoad(tex, clamped, 0).rg;
}

fn fetch_scene(coord: vec2<i32>, dims: vec2<u32>) -> vec4<f32> {
    let clamped = clamp(coord, vec2<i32>(0), vec2<i32>(dims) - vec2<i32>(1));
    return textureLoad(scene_tex, clamped, 0);
}

fn luminance(color: vec3<f32>) -> f32 {
    return dot(color, vec3<f32>(0.2126, 0.7152, 0.0722));
}

fn max_channel(color: vec3<f32>) -> f32 {
    return max(color.x, max(color.y, color.z));
}

fn compress_highlights(color: vec3<f32>, amount: f32) -> vec3<f32> {
    let luma = luminance(color);
    let highlight = smoothstep(0.58, 0.96, max_channel(color));
    let matte = mix(color, color * 0.58 + vec3<f32>(luma) * 0.16, highlight);
    return mix(color, matte, clamp(amount, 0.0, 1.0));
}

fn tint(base: vec4<f32>, color: vec3<f32>, alpha: f32) -> vec4<f32> {
    let a = clamp(alpha, 0.0, 1.0);
    return vec4<f32>(mix(base.rgb, color, a), base.a);
}

fn screen_add(base: vec4<f32>, color: vec3<f32>, amount: f32) -> vec4<f32> {
    let c = clamp(color * clamp(amount, 0.0, 1.0), vec3<f32>(0.0), vec3<f32>(1.0));
    return vec4<f32>(1.0 - (1.0 - base.rgb) * (1.0 - c), base.a);
}

fn neighbor_min(coord: vec2<i32>, dims: vec2<u32>, radius: i32) -> vec2<f32> {
    let r = max(radius, 1);
    var min_m = vec2<f32>(1.0);

    let offsets = array<vec2<i32>, 8>(
        vec2<i32>( r, 0), vec2<i32>(-r, 0), vec2<i32>(0,  r), vec2<i32>(0, -r),
        vec2<i32>( r,  r), vec2<i32>( r, -r), vec2<i32>(-r,  r), vec2<i32>(-r, -r)
    );

    for (var i = 0u; i < 8u; i = i + 1u) {
        min_m = min(min_m, fetch_rg(mask_tex, coord + offsets[i], dims));
    }
    return min_m;
}

@fragment
fn fs_main(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let dims = textureDimensions(scene_tex);
    let coord = vec2<i32>(floor(pos.xy));
    var out = fetch_scene(coord, dims);

    let mask = fetch_rg(mask_tex, coord, dims);
    if (mask.x == 0.0 && mask.y == 0.0) || out.a == 0.0 {
        return out;
    }

    let rim_radius = i32(round(params.inv_size_radii.z));
    let min_m = neighbor_min(coord, dims, rim_radius);
    let selected_rim = smoothstep(0.04, 0.55, mask.x * (mask.x - min_m.x));
    let hover_rim = smoothstep(0.04, 0.55, mask.y * (mask.y - min_m.y));

    // Tint the unassociated scene color, then restore its original coverage.
    out = vec4<f32>(compress_highlights(out.rgb / out.a, mask.x * 0.86), out.a);
    out = tint(out, SELECTED_FILL, mask.x * 0.60);
    out = screen_add(out, SELECTED_RIM, selected_rim * 0.42);

    // Hover is intentionally last: a hovered selected atom should read as
    // hover, not as a stronger selection.
    out = tint(out, HOVER_TINT, mask.y * 0.44);
    out = screen_add(out, HOVER_RIM, hover_rim * 0.70);
    return vec4<f32>(out.rgb * out.a, out.a);
}
