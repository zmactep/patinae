// Sphere instance builder — replaces the CPU-side
// `for atom in molecule { instances.push(...) }` loop.
//
// One thread per object-local atom index. Reads the atom's `repr_flags`
// to gate sphere visibility, fetches its coord, scales the radius by the
// per-rep `sphere_scale`, and emits a `SphereInstance` into a compacted
// buffer via an atomic counter living inside the indirect draw args.
//
// **Group 0** here is the same `SceneStore` data the render shaders see at
// `@group(2)`. WGSL hardcodes group numbers so we duplicate the binding
// declarations rather than reuse `INCLUDE_SCENE` (which targets the
// render path). Layouts must stay in sync with `scene_store/mod.rs` and
// `shaders/common/scene.wgsl`.
//
// **Group 1** — per-rep build state.

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

@group(0) @binding(0) var<uniform>           obj         : ObjectEntry;
@group(0) @binding(1) var<storage, read>     atoms       : array<AtomGpu>;
@group(0) @binding(2) var<storage, read>     coords      : array<vec4<f32>>;

const REP_BIT_SPHERES: u32 = 1u << 1u;

struct SphereInstance {
    center:   vec3<f32>,
    radius:   f32,
    group_id: u32,
    _pad:     u32,
};

struct SphereBuildParams {
    sphere_scale: f32,
    sample_shift: u32,
    _pad0:        u32,
    _pad1:        u32,
};

// Build writes raw (un-culled) instances and increments a dedicated
// `raw_count` atomic. The per-rep cull pipeline later reads `raw_count`
// to bound its loop, tests each instance against frustum, and
// writes the surviving subset into a compacted buffer + the indirect-
// draw args. This decoupling makes culling a pure GPU pass that runs on
// every camera-changed frame without re-building geometry.
@group(1) @binding(0) var<uniform>             params:    SphereBuildParams;
@group(1) @binding(1) var<storage, read_write> instances: array<SphereInstance>;
@group(1) @binding(2) var<storage, read_write> raw_count: array<atomic<u32>, 1>;

fn passes_sphere_lod(local: u32) -> bool {
    if params.sample_shift == 0u {
        return true;
    }
    // CPU caps `sample_shift` below 32, so this shift is portable WGSL.
    let mask = (1u << params.sample_shift) - 1u;
    return (local & mask) == 0u;
}

@compute @workgroup_size(64)
fn cs_main(
    @builtin(global_invocation_id) gid_in: vec3<u32>,
    @builtin(num_workgroups) nwg: vec3<u32>,
) {
    // 2D dispatch when atom_count overflows the per-dimension workgroup
    // limit (65535). See `compute::split_1d_dispatch`.
    let local = gid_in.x + gid_in.y * nwg.x * 64u;
    if local >= obj.atom_count { return; }

    let global_id = obj.atom_offset + local;
    let atom = atoms[global_id];
    if (atom.repr_flags & REP_BIT_SPHERES) == 0u { return; }
    if !passes_sphere_lod(local) { return; }

    let radius = atom.vdw * params.sphere_scale;
    let pos = coords[global_id].xyz;

    let slot = atomicAdd(&raw_count[0], 1u);
    var ins: SphereInstance;
    ins.center   = pos;
    ins.radius   = radius;
    // `group_id` carries the object-local atom index. The color shader
    // adds `obj.atom_offset` to fetch into scene LUTs; picking uses the
    // local id directly so PackedId.atom_id matches the host's
    // expectations.
    ins.group_id = local;
    ins._pad     = 0u;
    instances[slot] = ins;
}
