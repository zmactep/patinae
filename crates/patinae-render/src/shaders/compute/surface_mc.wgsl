// Marching cubes — atomic-counter emit.
//
// One thread per voxel block of (1×1×1) — i.e. each invocation classifies
// one cube of 8 corners. Triangles are written directly to a vertex buffer
// at slots reserved via `atomicAdd(vertex_count, n)`.
//
// **Convention.** A corner is *inside* when the field value compares true
// against `iso` per `params.invert_inside`:
//   - `invert_inside == 0`: inside ⇔ value >= iso  (SAS density: bigger = inside)
//   - `invert_inside == 1`: inside ⇔ value <= iso  (SES SDF: smaller = inside)
// Both modes set a case bit for an interior corner. With our corner layout,
// TRI_TABLE emits inward-facing triangles for that mask, so reverse the
// edge slots in both modes to face outward. The scalar field's sign changes
// the classification and gradient, not the winding of an identical mask.
//
// Vertex normal is the gradient of the scalar field, computed by central
// differences on the rgba16float .w channel. Sign is flipped under
// `invert_inside == 0` so the normal always points from interior to exterior.

// {{INCLUDE_OCTAHEDRAL}}

struct McParams {
    bbox_min: vec3<f32>,
    voxel_size: f32,
    dims: vec3<u32>,
    iso: f32,
    max_vertices: u32,
    invert_inside: u32,
    emit_lines:    u32,
    _pad1: u32,
    emit_core_min: vec3<f32>,
    _pad2: f32,
    emit_core_max: vec3<f32>,
    _pad3: f32,
};

// 24 B packed mirror of `StdVertex` (mesh.rs / mesh.wgsl). Position is split
// into three scalars so the storage layout is 4-B aligned (a `vec3<f32>` field
// would force 16-B struct alignment → 32 B total, breaking the vertex-buffer
// stride contract).
struct StdVertex {
    px: f32,
    py: f32,
    pz: f32,
    normal_oct: u32,
    group_id: u32,
    flags: u32,
};

// MC table layout — must mirror GpuMcTablesLayout in mc_tables.rs.
const EDGE_TABLE_OFFSET: u32 = 0u;
const TRI_TABLE_OFFSET: u32 = 256u;
const EDGE_VERTICES_OFFSET: u32 = 256u + 256u * 16u;       // 4352
const TRI_COUNT_OFFSET: u32 = EDGE_VERTICES_OFFSET + 24u;  // 4376

@group(0) @binding(0) var<uniform> params: McParams;
@group(0) @binding(1) var field_3d: texture_storage_3d<rgba16float, read>;
@group(0) @binding(2) var owner_3d: texture_storage_3d<r32uint, read>;
@group(0) @binding(3) var<storage, read> mc_tables: array<u32>;
@group(0) @binding(4) var<storage, read_write> vertex_count: atomic<u32>;
@group(0) @binding(5) var<storage, read_write> vertices: array<StdVertex>;

fn sample_scalar(i: i32, j: i32, k: i32) -> f32 {
    let dx = i32(params.dims.x);
    let dy = i32(params.dims.y);
    let dz = i32(params.dims.z);
    let ci = clamp(i, 0, dx - 1);
    let cj = clamp(j, 0, dy - 1);
    let ck = clamp(k, 0, dz - 1);
    return textureLoad(field_3d, vec3<i32>(ci, cj, ck)).w;
}

fn sample_owner(i: u32, j: u32, k: u32) -> u32 {
    return textureLoad(owner_3d, vec3<i32>(i32(i), i32(j), i32(k))).r;
}

fn corner_inside(v: f32) -> bool {
    if (params.invert_inside == 0u) {
        return v >= params.iso;
    } else {
        return v <= params.iso;
    }
}

fn central_gradient(i: i32, j: i32, k: i32) -> vec3<f32> {
    let gx = sample_scalar(i + 1, j, k) - sample_scalar(i - 1, j, k);
    let gy = sample_scalar(i, j + 1, k) - sample_scalar(i, j - 1, k);
    let gz = sample_scalar(i, j, k + 1) - sample_scalar(i, j, k - 1);
    let raw = vec3<f32>(gx, gy, gz) / (2.0 * params.voxel_size);
    // Surface normal points from interior to exterior. For SAS (inside means
    // density >= iso), the field DECREASES outward, so invert.
    if (params.invert_inside == 0u) {
        return -raw;
    } else {
        return raw;
    }
}

fn interp_position(p_a: vec3<f32>, p_b: vec3<f32>, va: f32, vb: f32) -> vec3<f32> {
    let denom = vb - va;
    var t = 0.5;
    if (abs(denom) > 1e-6) {
        t = clamp((params.iso - va) / denom, 0.0, 1.0);
    }
    return p_a + t * (p_b - p_a);
}

fn interp_gradient(g_a: vec3<f32>, g_b: vec3<f32>, va: f32, vb: f32) -> vec3<f32> {
    let denom = vb - va;
    var t = 0.5;
    if (abs(denom) > 1e-6) {
        t = clamp((params.iso - va) / denom, 0.0, 1.0);
    }
    return g_a + t * (g_b - g_a);
}

@compute @workgroup_size(4, 4, 4)
fn cs_main(@builtin(global_invocation_id) gid: vec3<u32>) {
    if (gid.x + 1u >= params.dims.x || gid.y + 1u >= params.dims.y || gid.z + 1u >= params.dims.z) {
        return;
    }
    let cube_center = params.bbox_min + (vec3<f32>(gid) + vec3<f32>(0.5, 0.5, 0.5)) * params.voxel_size;
    if (cube_center.x < params.emit_core_min.x ||
        cube_center.y < params.emit_core_min.y ||
        cube_center.z < params.emit_core_min.z ||
        cube_center.x >= params.emit_core_max.x ||
        cube_center.y >= params.emit_core_max.y ||
        cube_center.z >= params.emit_core_max.z) {
        return;
    }

    // 8 cube corners — order matches CORNER_OFFSETS in mc_tables.rs.
    var corner_val: array<f32, 8>;
    var corner_pos: array<vec3<f32>, 8>;
    var corner_grad: array<vec3<f32>, 8>;
    var corner_owner: array<u32, 8>;

    let base = vec3<i32>(i32(gid.x), i32(gid.y), i32(gid.z));
    let offsets = array<vec3<u32>, 8>(
        vec3<u32>(0u, 0u, 0u),
        vec3<u32>(1u, 0u, 0u),
        vec3<u32>(1u, 1u, 0u),
        vec3<u32>(0u, 1u, 0u),
        vec3<u32>(0u, 0u, 1u),
        vec3<u32>(1u, 0u, 1u),
        vec3<u32>(1u, 1u, 1u),
        vec3<u32>(0u, 1u, 1u),
    );

    var case_idx: u32 = 0u;
    for (var c: u32 = 0u; c < 8u; c = c + 1u) {
        let off = offsets[c];
        let ix = base.x + i32(off.x);
        let iy = base.y + i32(off.y);
        let iz = base.z + i32(off.z);
        let v = sample_scalar(ix, iy, iz);
        corner_val[c] = v;
        corner_pos[c] = params.bbox_min + vec3<f32>(f32(ix), f32(iy), f32(iz)) * params.voxel_size;
        corner_grad[c] = central_gradient(ix, iy, iz);
        corner_owner[c] = sample_owner(u32(ix), u32(iy), u32(iz));
        if (corner_inside(v)) {
            case_idx = case_idx | (1u << c);
        }
    }

    let edge_mask = mc_tables[EDGE_TABLE_OFFSET + case_idx];
    if (edge_mask == 0u) {
        return;
    }

    // Interpolate vertex positions + gradients on each crossed edge.
    var edge_pos: array<vec3<f32>, 12>;
    var edge_grad: array<vec3<f32>, 12>;
    for (var e: u32 = 0u; e < 12u; e = e + 1u) {
        if ((edge_mask & (1u << e)) == 0u) {
            continue;
        }
        let a = mc_tables[EDGE_VERTICES_OFFSET + e * 2u];
        let b = mc_tables[EDGE_VERTICES_OFFSET + e * 2u + 1u];
        edge_pos[e] = interp_position(corner_pos[a], corner_pos[b], corner_val[a], corner_val[b]);
        edge_grad[e] = interp_gradient(corner_grad[a], corner_grad[b], corner_val[a], corner_val[b]);
    }

    let n_tri = mc_tables[TRI_COUNT_OFFSET + case_idx];
    if (n_tri == 0u) {
        return;
    }

    // Per-triangle slot count: 3 in TriangleList mode, 6 in LineList mode
    // (3 line segments per triangle, 2 endpoints each). The host sets
    // `params.emit_lines` to switch — same MC kernel, different output
    // topology.
    let n_per_tri = select(3u, 6u, params.emit_lines == 1u);

    // Reserve up-front. Reject the whole voxel if it would overflow
    // `max_vertices` — partial writes would leave garbage. Roll the
    // reservation back so the indirect draw never exceeds the allocated
    // vertex buffer even when many cubes race near the cap.
    let needed = n_tri * n_per_tri;
    let base_idx = atomicAdd(&vertex_count, needed);
    if (base_idx + needed > params.max_vertices) {
        atomicSub(&vertex_count, needed);
        return;
    }

    // Permutation `[0,1, 1,2, 2,0]` maps a LineList vertex slot in `[0..6)`
    // to a triangle vertex slot in `[0..3)`. Adjacent pairs form line
    // segments: (0→1), (1→2), (2→0).
    var line_permute: array<u32, 6> = array<u32, 6>(0u, 1u, 1u, 2u, 2u, 0u);

    for (var t: u32 = 0u; t < n_tri; t = t + 1u) {
        // Pick a stable primary/secondary owner pair for this triangle from
        // the owners at the crossed edge endpoints. The fragment shader uses
        // the pair for narrow color-edge smoothing without widening StdVertex.
        var tri_owners: array<u32, 6>;
        var tri_owner_count = 0u;
        for (var pair_slot: u32 = 0u; pair_slot < 3u; pair_slot = pair_slot + 1u) {
            var pair_local: u32 = pair_slot;
            if (params.emit_lines == 0u) {
                pair_local = 2u - pair_slot;
            }
            let pair_edge_signed = i32(mc_tables[TRI_TABLE_OFFSET + case_idx * 16u + t * 3u + pair_local]);
            if (pair_edge_signed < 0 || pair_edge_signed >= 12) {
                continue;
            }
            let pair_edge = u32(pair_edge_signed);
            let ca = mc_tables[EDGE_VERTICES_OFFSET + pair_edge * 2u];
            let cb = mc_tables[EDGE_VERTICES_OFFSET + pair_edge * 2u + 1u];
            tri_owners[tri_owner_count] = corner_owner[ca];
            tri_owner_count = tri_owner_count + 1u;
            tri_owners[tri_owner_count] = corner_owner[cb];
            tri_owner_count = tri_owner_count + 1u;
        }

        let fallback_owner = sample_owner(gid.x, gid.y, gid.z);
        var primary_owner = fallback_owner;
        var secondary_owner = fallback_owner;
        var primary_count = 0u;
        var secondary_count = 0u;
        for (var i: u32 = 0u; i < tri_owner_count; i = i + 1u) {
            let candidate = tri_owners[i];
            var count = 0u;
            for (var j: u32 = 0u; j < tri_owner_count; j = j + 1u) {
                if (tri_owners[j] == candidate) {
                    count = count + 1u;
                }
            }
            if (count > primary_count || (count == primary_count && candidate == fallback_owner)) {
                if (candidate != primary_owner) {
                    secondary_owner = primary_owner;
                    secondary_count = primary_count;
                }
                primary_owner = candidate;
                primary_count = count;
            } else if (candidate != primary_owner && count > secondary_count) {
                secondary_owner = candidate;
                secondary_count = count;
            }
        }
        if (secondary_count == 0u) {
            secondary_owner = primary_owner;
        }

        for (var slot: u32 = 0u; slot < n_per_tri; slot = slot + 1u) {
            // In TriangleList mode `slot` indexes the triangle vertex
            // directly; in LineList mode we map through `line_permute` so
            // adjacent pairs (0,1)/(2,3)/(4,5) form the triangle's edges.
            var tri_slot: u32 = slot;
            if (params.emit_lines == 1u) {
                tri_slot = line_permute[slot];
            }
            // Both field modes classify interior corners into the same mask.
            // Reverse triangle winding in either mode; LineList edges are
            // direction-agnostic.
            var local: u32 = tri_slot;
            if (params.emit_lines == 0u) {
                local = 2u - tri_slot;
            }
            let edge_id_signed = i32(mc_tables[TRI_TABLE_OFFSET + case_idx * 16u + t * 3u + local]);
            if (edge_id_signed < 0 || edge_id_signed >= 12) {
                continue;
            }
            let edge_id = u32(edge_id_signed);
            let v_idx = base_idx + t * n_per_tri + slot;
            let normal = edge_grad[edge_id];
            let p = edge_pos[edge_id];
            vertices[v_idx].px = p.x;
            vertices[v_idx].py = p.y;
            vertices[v_idx].pz = p.z;
            vertices[v_idx].normal_oct = oct_encode(normal);
            vertices[v_idx].group_id = primary_owner;
            vertices[v_idx].flags = secondary_owner;
        }
    }
}
