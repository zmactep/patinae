// Stick instance builder. One thread per object-local bond. For each
// visible bond:
//   1. Read both endpoint atoms; gate on `STICKS` bit (at least one
//      endpoint has the rep enabled).
//   2. Read the CPU-precomputed local-plane perpendicular from BondGpu.
//   3. Emit 1, 2, or 3 instances based on bond order, each with a
//      perpendicular offset baked into both endpoint positions. Bond
//      radius shrinks to half for multiple bonds.
//
// Group 0 — `SceneStore` (scene-wide LUTs, dyn-offset uniform `obj`).
//   See render path's `INCLUDE_SCENE`; here we duplicate the bindings
//   because WGSL hard-codes group numbers.
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

struct BondGpu {
    atom1: u32,
    atom2: u32,
    order: u32,
    flags: u32,
    valence_perp_pad: vec4<f32>,
};

@group(0) @binding(0) var<uniform>           obj          : ObjectEntry;
@group(0) @binding(1) var<storage, read>     atoms        : array<AtomGpu>;
@group(0) @binding(2) var<storage, read>     coords       : array<vec4<f32>>;
@group(0) @binding(3) var<storage, read>     bonds        : array<BondGpu>;

const REP_BIT_STICKS: u32 = 1u << 0u;

const ORDER_SINGLE:   u32 = 1u;
const ORDER_DOUBLE:   u32 = 2u;
const ORDER_TRIPLE:   u32 = 3u;
const ORDER_AROMATIC: u32 = 4u;

struct StickInstance {
    p0_radius: vec4<f32>,
    p1_pad:    vec4<f32>,
    groups:    vec2<u32>,
    _pad:      vec2<u32>,
};

struct StickBuildParams {
    radius:           f32,
    valence_scale:    f32,
    valence_enabled:  u32,
    sample_shift:     u32,
};

@group(1) @binding(0) var<uniform>             params:    StickBuildParams;
@group(1) @binding(1) var<storage, read_write> instances: array<StickInstance>;
@group(1) @binding(2) var<storage, read_write> raw_count: array<atomic<u32>, 1>;

fn rep_count_for_order(order: u32) -> u32 {
    if order == ORDER_TRIPLE { return 3u; }
    if order == ORDER_DOUBLE || order == ORDER_AROMATIC { return 2u; }
    return 1u;
}

fn passes_stick_lod(local: u32) -> bool {
    if params.sample_shift == 0u {
        return true;
    }
    // CPU caps `sample_shift` below 32, so this shift is portable WGSL.
    let mask = (1u << params.sample_shift) - 1u;
    return (local & mask) == 0u;
}

// Mirrors `patinae_mol::bond_utils::get_bond_offsets`.
fn offset_factor(order: u32, slot: u32) -> f32 {
    if order == ORDER_DOUBLE || order == ORDER_AROMATIC {
        if slot == 0u { return -0.5; }
        return 0.5;
    }
    if order == ORDER_TRIPLE {
        if slot == 0u { return -1.0; }
        if slot == 1u { return  0.0; }
        return 1.0;
    }
    return 0.0;
}

// Fixed-axis fallback if the bond is colinear with the chosen reference.
fn perp_fallback(bond_dir: vec3<f32>) -> vec3<f32> {
    let up = vec3<f32>(0.0, 1.0, 0.0);
    var perp = cross(bond_dir, up);
    if dot(perp, perp) < 1e-4 {
        let right = vec3<f32>(1.0, 0.0, 0.0);
        perp = cross(bond_dir, right);
    }
    let len = length(perp);
    if len > 1e-4 {
        return perp / len;
    }
    return vec3<f32>(0.0, 1.0, 0.0);
}

fn bond_valence_perp(bond: BondGpu, bond_dir: vec3<f32>) -> vec3<f32> {
    let raw = bond.valence_perp_pad.xyz;
    let len2 = dot(raw, raw);
    if len2 > 1e-4 {
        return raw * inverseSqrt(len2);
    }
    return perp_fallback(bond_dir);
}

@compute @workgroup_size(64)
fn cs_main(
    @builtin(global_invocation_id) gid_in: vec3<u32>,
    @builtin(num_workgroups) nwg: vec3<u32>,
) {
    // 2D dispatch handles bond_count > 65535*64. See `compute::split_1d_dispatch`.
    let local = gid_in.x + gid_in.y * nwg.x * 64u;
    if local >= obj.bond_count { return; }
    if !passes_stick_lod(local) { return; }

    let self_global_bond = obj.bond_offset + local;
    let bond = bonds[self_global_bond];
    let a_local = bond.atom1;
    let b_local = bond.atom2;
    if a_local >= obj.atom_count || b_local >= obj.atom_count {
        return;
    }

    let atom_a = atoms[obj.atom_offset + a_local];
    let atom_b = atoms[obj.atom_offset + b_local];
    let vis_a = (atom_a.repr_flags & REP_BIT_STICKS) != 0u;
    let vis_b = (atom_b.repr_flags & REP_BIT_STICKS) != 0u;
    if !vis_a && !vis_b { return; }

    let pa = coords[obj.atom_offset + a_local].xyz;
    let pb = coords[obj.atom_offset + b_local].xyz;
    let axis = pb - pa;
    let axis_len = length(axis);
    if axis_len < 1e-6 { return; }
    let bond_dir = axis / axis_len;

    let order = bond.order;
    let multi = order == ORDER_DOUBLE || order == ORDER_TRIPLE || order == ORDER_AROMATIC;
    let n = select(1u, rep_count_for_order(order), params.valence_enabled != 0u);

    var radius = params.radius;
    if multi && params.valence_enabled != 0u {
        radius = radius * 0.5;
    }

    var perp = vec3<f32>(0.0);
    if n > 1u {
        perp = bond_valence_perp(bond, bond_dir);
    }

    for (var slot: u32 = 0u; slot < n; slot = slot + 1u) {
        let factor = offset_factor(order, slot);
        let off = perp * (factor * params.valence_scale);
        let q0 = pa + off;
        let q1 = pb + off;
        let dst = atomicAdd(&raw_count[0], 1u);
        if dst >= arrayLength(&instances) {
            continue;
        }
        var ins: StickInstance;
        ins.p0_radius = vec4<f32>(q0, radius);
        ins.p1_pad    = vec4<f32>(q1, 0.0);
        ins.groups    = vec2<u32>(a_local, b_local);
        ins._pad      = vec2<u32>(0u, 0u);
        instances[dst] = ins;
    }
}
