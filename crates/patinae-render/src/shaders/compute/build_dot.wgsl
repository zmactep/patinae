// Dot atom-instance builder. One thread per atom. Visibility gate: atom must
// have the `DOTS` rep enabled and pass the scene mask. Each visible atom emits
// one instance; the vertex shader expands it into surface samples.
//
// gid_in.x = atom_local
//
// Group 0 — `SceneStore` (compute side; mirrors render's INCLUDE_SCENE
//           layout but at @group(0)).
// Group 1 — per-rep build state.

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

@group(0) @binding(0) var<uniform>           obj    : ObjectEntry;
@group(0) @binding(1) var<storage, read>     atoms  : array<AtomGpu>;
@group(0) @binding(2) var<storage, read>     coords : array<vec4<f32>>;
@group(0) @binding(5) var<storage, read>     mask_lut: array<u32>;

const REP_BIT_DOTS: u32 = 1u << 9u;

struct DotAtomInstance {
    center: vec3<f32>,
    group_id: u32,
    vdw_radius: f32,
    _pad0: u32,
    _pad1: u32,
    _pad2: u32,
};

struct DotBuildParams {
    instance_capacity: u32,
    _pad0:             u32,
    _pad1:             u32,
    _pad2:             u32,
};

@group(1) @binding(0) var<uniform>             params:    DotBuildParams;
@group(1) @binding(1) var<storage, read_write> instances: array<DotAtomInstance>;
@group(1) @binding(2) var<storage, read_write> raw_count: array<atomic<u32>, 1>;

fn scene_visible(gid: u32) -> bool {
    let word = gid >> 5u;
    let bit = gid & 31u;
    return (mask_lut[word] & (1u << bit)) != 0u;
}

@compute @workgroup_size(64)
fn cs_main(
    @builtin(global_invocation_id) gid_in: vec3<u32>,
    @builtin(num_workgroups) nwg: vec3<u32>,
) {
    // 2D dispatch handles atom_count > 65535*64.
    let tid = gid_in.x + gid_in.y * nwg.x * 64u;
    if tid >= obj.atom_count { return; }

    let global_atom = obj.atom_offset + tid;
    let atom = atoms[global_atom];
    if (atom.repr_flags & REP_BIT_DOTS) == 0u { return; }
    if !scene_visible(global_atom) { return; }

    let dst = atomicAdd(&raw_count[0], 1u);
    if dst >= params.instance_capacity { return; }

    var ins: DotAtomInstance;
    ins.center = coords[global_atom].xyz;
    ins.group_id = tid;
    ins.vdw_radius = atom.vdw;
    ins._pad0 = 0u;
    ins._pad1 = 0u;
    ins._pad2 = 0u;
    instances[dst] = ins;
}
