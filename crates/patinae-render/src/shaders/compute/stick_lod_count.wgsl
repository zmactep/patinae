// Count real viewport-visible source bonds for camera-aware stick LOD.
//
// Group 0 mirrors the compute-side SceneStore layout. Group 1 reuses the
// per-stick cull params buffer for frustum planes; `kind_radius` carries the
// current stick radius plus valence-offset pad for conservative bond bounds.

struct ObjectEntry {
    atom_offset: u32,
    atom_count:  u32,
    bond_offset: u32,
    bond_count:  u32,
    object_id:   u32,
    flags:       u32,
    _pad0_a:     u32,
    _pad0_b:     u32,
    model_matrix: mat4x4<f32>,
};

struct AtomGpu {
    vdw: f32,
    repr_flags: u32,
    alpha_pack_a: u32,
    alpha_pack_b: u32,
};

struct BondGpu {
    atom1: u32,
    atom2: u32,
    order: u32,
    flags: u32,
    valence_perp_pad: vec4<f32>,
};

struct CullParams {
    view_proj:      mat4x4<f32>,
    frustum_planes: array<vec4<f32>, 6>,
    raw_capacity:   u32,
    kind_radius:    f32,
    _pad0:          u32,
    _pad1:          u32,
};

@group(0) @binding(0) var<uniform>       obj      : ObjectEntry;
@group(0) @binding(1) var<storage, read> atoms    : array<AtomGpu>;
@group(0) @binding(2) var<storage, read> coords   : array<vec4<f32>>;
@group(0) @binding(3) var<storage, read> bonds    : array<BondGpu>;
@group(0) @binding(5) var<storage, read> mask_lut : array<u32>;

@group(1) @binding(0) var<uniform>             params: CullParams;
@group(1) @binding(1) var<storage, read_write> count : array<atomic<u32>, 1>;

const REP_BIT_STICKS: u32 = 1u << 0u;

fn scene_visible(gid: u32) -> bool {
    let word = gid >> 5u;
    let bit = gid & 31u;
    return (mask_lut[word] & (1u << bit)) != 0u;
}

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

@compute @workgroup_size(64)
fn cs_main(
    @builtin(global_invocation_id) gid_in: vec3<u32>,
    @builtin(num_workgroups) nwg: vec3<u32>,
) {
    let local = gid_in.x + gid_in.y * nwg.x * 64u;
    if local >= obj.bond_count { return; }

    let bond = bonds[obj.bond_offset + local];
    let a_local = bond.atom1;
    let b_local = bond.atom2;
    if a_local >= obj.atom_count || b_local >= obj.atom_count {
        return;
    }

    let a_global = obj.atom_offset + a_local;
    let b_global = obj.atom_offset + b_local;
    let atom_a = atoms[a_global];
    let atom_b = atoms[b_global];
    let vis_a = (atom_a.repr_flags & REP_BIT_STICKS) != 0u && scene_visible(a_global);
    let vis_b = (atom_b.repr_flags & REP_BIT_STICKS) != 0u && scene_visible(b_global);
    if !vis_a && !vis_b { return; }

    let pa = coords[a_global].xyz;
    let pb = coords[b_global].xyz;
    let axis = pb - pa;
    let axis_len = length(axis);
    if axis_len < 1e-6 { return; }

    let center = 0.5 * (pa + pb);
    let radius = 0.5 * axis_len + params.kind_radius;
    if !frustum_visible(center, radius) { return; }

    _ = atomicAdd(&count[0], 1u);
}
