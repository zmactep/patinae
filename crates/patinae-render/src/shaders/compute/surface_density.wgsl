// SAS Gaussian density splat.
//
// One thread per voxel; each thread gathers atoms from a surface-local
// spatial binning buffer and accumulates `ρ(p) = Σ_a exp(-α · |p - c_a|² /
// r_eff²)` with `r_eff = r_a + probe`. The analytical gradient of the same
// expression doubles as the surface normal at MC time. Owner-atom (nearest
// atom by Euclidean distance) is written into the companion `owner_3d`
// texture so the MC pass can stamp `group_id` per vertex.
//
// **owner_id is object-local** (in `[0, obj.atom_count)`). The fragment
// shader prepends `obj.atom_offset` for scene-wide LUT lookups; picking
// uses the local id directly — same convention as sphere/stick/etc.
//
// Group 0 — `SceneStore` (compute side; mirrors the render path's
//           INCLUDE_SCENE layout but at @group(0)).
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

struct DensityParams {
    bbox_min: vec3<f32>,
    voxel_size: f32,
    dims: vec3<u32>,
    _pad_a: u32,
    probe_radius: f32,
    alpha: f32,
    _pad0: f32,
    _pad1: f32,
};

struct SurfaceAccelParams {
    origin: vec3<f32>,
    cell_size: f32,
    dims: vec3<u32>,
    _pad: u32,
};

@group(1) @binding(0) var<uniform> params: DensityParams;
@group(1) @binding(1) var density_tex: texture_storage_3d<rgba16float, write>;
@group(1) @binding(2) var owner_tex: texture_storage_3d<r32uint, write>;
@group(1) @binding(3) var<storage, read> cell_offsets: array<u32>;
@group(1) @binding(4) var<storage, read> cell_indices: array<u32>;
@group(1) @binding(5) var<uniform> accel: SurfaceAccelParams;

fn accel_cell_id(c: vec3<u32>) -> u32 {
    return c.x + c.y * accel.dims.x + c.z * accel.dims.x * accel.dims.y;
}

fn accel_base_cell(p: vec3<f32>) -> vec3<i32> {
    let rel = (p - accel.origin) / accel.cell_size;
    return vec3<i32>(floor(rel));
}

@compute @workgroup_size(4, 4, 4)
fn cs_main(@builtin(global_invocation_id) gid: vec3<u32>) {
    if (gid.x >= params.dims.x || gid.y >= params.dims.y || gid.z >= params.dims.z) {
        return;
    }
    let p = params.bbox_min + vec3<f32>(gid) * params.voxel_size;

    var density = 0.0;
    var grad = vec3<f32>(0.0, 0.0, 0.0);
    var best_d2 = 1e30;
    var best_idx: u32 = 0u;
    let alpha = params.alpha;
    let base_cell = accel_base_cell(p);

    for (var dz: i32 = -1; dz <= 1; dz = dz + 1) {
        for (var dy: i32 = -1; dy <= 1; dy = dy + 1) {
            for (var dx: i32 = -1; dx <= 1; dx = dx + 1) {
                let c = base_cell + vec3<i32>(dx, dy, dz);
                if (c.x < 0 || c.y < 0 || c.z < 0) {
                    continue;
                }
                if (u32(c.x) >= accel.dims.x || u32(c.y) >= accel.dims.y || u32(c.z) >= accel.dims.z) {
                    continue;
                }
                let cid = accel_cell_id(vec3<u32>(u32(c.x), u32(c.y), u32(c.z)));
                let start = cell_offsets[cid];
                let end = cell_offsets[cid + 1u];
                for (var cursor = start; cursor < end; cursor = cursor + 1u) {
                    let i = cell_indices[cursor];
                    if (i >= obj.atom_count) {
                        continue;
                    }
                    let global_id = obj.atom_offset + i;
                    let a = atoms[global_id];
                    if (a.vdw <= 0.0) {
                        continue;
                    }
                    let r_eff = a.vdw + params.probe_radius;
                    if (r_eff <= 0.0) {
                        continue;
                    }
                    let inv_r2 = 1.0 / (r_eff * r_eff);
                    let pos = coords[global_id].xyz;
                    let delta = p - pos;
                    let d2 = dot(delta, delta);
                    let g = exp(-alpha * d2 * inv_r2);
                    density = density + g;
                    // ∂/∂x exp(-α |p-c|² / r²) = -2α/r² · (p-c) · g
                    grad = grad + (-2.0 * alpha * inv_r2 * g) * delta;
                    if (d2 < best_d2) {
                        best_d2 = d2;
                        best_idx = i;
                    }
                }
            }
        }
    }

    textureStore(density_tex, vec3<i32>(gid), vec4<f32>(grad, density));
    textureStore(owner_tex, vec3<i32>(gid), vec4<u32>(best_idx, 0u, 0u, 0u));
}
