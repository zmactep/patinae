// Shared bits for the per-rep frustum cull kernels. Per-kind shaders pull
// this in via `// {{INCLUDE_CULL_COMMON}}` and add their own
// `<Kind>Instance` struct + bindings 1 / 3 + entry point.

struct CullParams {
    view_proj:      mat4x4<f32>,
    // World-space frustum planes (l, r, b, t, n, f).
    // `dot(plane.xyz, p) + plane.w >= 0` ⇔ p inside.
    frustum_planes: array<vec4<f32>, 6>,
    raw_capacity:   u32,
    // Per-kind bounding-sphere pad. Used by kinds that lack a
    // per-instance radius (line: half line-width in NDC units, converted
    // per instance in cull_line.wgsl; dot: dot radius).
    // Sphere cull ignores this; sphere viewport-count uses the same slot
    // for sphere_scale.
    kind_radius:    f32,
    _pad0:          u32,
    _pad1:          u32,
};

@group(0) @binding(0) var<uniform>             params:    CullParams;
@group(0) @binding(2) var<storage, read>       raw_count: array<u32, 1>;
@group(0) @binding(4) var<storage, read_write> indirect:  array<atomic<u32>, 4>;

fn frustum_visible(center_w: vec3<f32>, radius: f32) -> bool {
    for (var p: u32 = 0u; p < 6u; p = p + 1u) {
        let plane = params.frustum_planes[p];
        let dist = dot(plane.xyz, center_w) + plane.w;
        if (dist < -radius) {
            return false;
        }
    }
    return true;
}

fn should_cull(center_w: vec3<f32>, radius: f32) -> bool {
    return !frustum_visible(center_w, radius);
}
