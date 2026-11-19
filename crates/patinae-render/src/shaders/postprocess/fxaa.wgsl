// Screen-space AA on a five-tap cross of premultiplied scene samples.
// Both luminance and coverage can define a visible edge.

@group(0) @binding(0) var src_color: texture_2d<f32>;

// Absolute and relative contrast thresholds from FXAA 3.11.
const EDGE_MIN: f32 = 0.0312;
const EDGE_RELATIVE: f32 = 0.125;

fn rgb2luma(rgb: vec3<f32>) -> f32 {
    return sqrt(dot(rgb, vec3<f32>(0.299, 0.587, 0.114)));
}

fn signal_range(center: f32, neighbors: vec4<f32>) -> f32 {
    let low = min(center, min(min(neighbors.x, neighbors.y), min(neighbors.z, neighbors.w)));
    let high = max(center, max(max(neighbors.x, neighbors.y), max(neighbors.z, neighbors.w)));
    return high - low;
}

@vertex
fn vs_main(@builtin(vertex_index) vid: u32) -> @builtin(position) vec4<f32> {
    let x = f32((vid << 1u) & 2u);
    let y = f32(vid & 2u);
    return vec4<f32>(x * 2.0 - 1.0, 1.0 - y * 2.0, 0.0, 1.0);
}

@fragment
fn fs_main(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let p = vec2<i32>(pos.xy);
    let last = vec2<i32>(textureDimensions(src_color)) - 1;
    let center = textureLoad(src_color, p, 0);
    let north = textureLoad(src_color, vec2<i32>(p.x, max(p.y - 1, 0)), 0);
    let south = textureLoad(src_color, vec2<i32>(p.x, min(p.y + 1, last.y)), 0);
    let west = textureLoad(src_color, vec2<i32>(max(p.x - 1, 0), p.y), 0);
    let east = textureLoad(src_color, vec2<i32>(min(p.x + 1, last.x), p.y), 0);
    let luma = vec4<f32>(rgb2luma(north.rgb), rgb2luma(south.rgb), rgb2luma(west.rgb), rgb2luma(east.rgb));
    let coverage = vec4<f32>(north.a, south.a, west.a, east.a);
    let center_luma = rgb2luma(center.rgb);

    // Use the stronger contrast signal for this neighborhood. This detects
    // black silhouettes as well as color boundaries inside opaque geometry.
    let use_coverage = signal_range(center.a, coverage) > signal_range(center_luma, luma);
    let c = select(center_luma, center.a, use_coverage);
    let neighbors = select(luma, coverage, use_coverage);
    let peak = max(c, max(max(neighbors.x, neighbors.y), max(neighbors.z, neighbors.w)));
    let contrast = signal_range(c, neighbors);
    if (contrast < max(EDGE_MIN, peak * EDGE_RELATIVE)) {
        return center;
    }

    // Smooth across the edge. The symmetric pair treats opposite silhouettes
    // identically, and color and alpha always receive the same filter weights.
    let delta = abs(neighbors - vec4<f32>(c));
    let across_x = delta.z + delta.w > delta.x + delta.y;
    let sides = select(north + south, west + east, across_x);
    return mix(center, 0.5 * sides, 0.5 * contrast / peak);
}
