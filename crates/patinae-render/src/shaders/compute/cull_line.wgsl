// Line culling. The host expands side planes for the pixel footprint.
// Per-instance work matches the native LineList sphere test.

// {{INCLUDE_CULL_COMMON}}

const LINE_CULL_WORLD_PAD: f32 = 0.1;

struct LineInstance {
    p0_width: vec4<f32>,
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
    let p0 = inst.p0_width.xyz;
    let p1 = inst.p1_pad.xyz;
    let center = 0.5 * (p0 + p1);
    let half_len = 0.5 * length(p1 - p0);
    let radius = half_len + LINE_CULL_WORLD_PAD;
    if (should_cull(center, radius)) {
        return;
    }
    let slot = atomicAdd(&indirect[1], 1u);
    if (slot < params.raw_capacity) {
        out_inst[slot] = inst;
    }
}
