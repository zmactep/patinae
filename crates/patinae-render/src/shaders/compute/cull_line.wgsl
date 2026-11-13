// Per-rep frustum cull — line variant.
//
// LineInstance carries two endpoints. Lines are drawn `line_width` screen
// pixels wide, so the bounding pad is screen-space: `params.kind_radius`
// is the half-width (incl. picking minimum + margin) in NDC units of the
// smaller viewport axis. It is converted to a world-space pad at the
// farther endpoint's depth, plus a small constant world pad.

// {{INCLUDE_CULL_COMMON}}

const LINE_CULL_WORLD_PAD: f32 = 0.1;

// World-space length of `ndc_pad` at clip-space depth `w`. Uses the
// smaller of the projection's x/y scales so the pad stays conservative.
// Valid for both projections: perspective has w = view depth, ortho w = 1.
fn line_world_pad(ndc_pad: f32, w: f32) -> f32 {
    let vp = params.view_proj;
    let sx = length(vec3<f32>(vp[0][0], vp[1][0], vp[2][0]));
    let sy = length(vec3<f32>(vp[0][1], vp[1][1], vp[2][1]));
    let scale = max(min(sx, sy), 1e-6);
    return ndc_pad * max(w, 0.0) / scale;
}

struct LineInstance {
    p0_pad: vec4<f32>,
    p1_pad: vec4<f32>,
    groups: vec2<u32>,
    _pad:   vec2<u32>,
};

@group(0) @binding(1) var<storage, read>       raw_inst: array<LineInstance>;
@group(0) @binding(3) var<storage, read_write> out_inst: array<LineInstance>;

@compute @workgroup_size(64)
fn cs_cull_line(
    @builtin(global_invocation_id) gid: vec3<u32>,
    @builtin(num_workgroups)       nwg: vec3<u32>,
) {
    let i = gid.x + gid.y * nwg.x * 64u;
    if (i >= raw_count[0]) {
        return;
    }
    let inst = raw_inst[i];
    let p0 = inst.p0_pad.xyz;
    let p1 = inst.p1_pad.xyz;
    let center = 0.5 * (p0 + p1);
    let half_len = 0.5 * length(p1 - p0);
    let w0 = (params.view_proj * vec4<f32>(p0, 1.0)).w;
    let w1 = (params.view_proj * vec4<f32>(p1, 1.0)).w;
    let pad = LINE_CULL_WORLD_PAD + line_world_pad(params.kind_radius, max(w0, w1));
    let radius = half_len + pad;
    if (should_cull(center, radius)) {
        return;
    }
    let slot = atomicAdd(&indirect[1], 1u);
    if (slot < params.raw_capacity) {
        out_inst[slot] = inst;
    }
}
