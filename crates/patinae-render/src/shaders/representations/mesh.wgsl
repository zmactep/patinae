// Mesh representation — wireframe of the molecular surface. `StdVertex`
// LineList layout emitted by `surface_mc.wgsl` with `emit_lines = 1`
// (6 vertices per MC triangle, edge permutation `[0,1, 1,2, 2,0]`).
//
// `instance.group_id` is the **object-local** owner-atom index (same as
// surface). FS prepends `obj.atom_offset` to fetch global SceneStore
// entries.

// {{INCLUDE_FRAME}}
// {{INCLUDE_SCENE}}
// {{INCLUDE_WBOIT}}
// {{INCLUDE_LIGHTING}}
// {{INCLUDE_OCTAHEDRAL}}

struct MeshParams {
    alpha_mul: f32,
    _pad0: f32,
    _pad1: f32,
    _pad2: f32,
};

@group(3) @binding(0) var<uniform> mesh_params: MeshParams;

struct StdVertex {
    @location(0) position:    vec3<f32>,
    @location(1) normal_oct:  u32,
    @location(2) group_id:    u32,
    @location(3) flags:       u32,
};

struct VsOut {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) view_pos:    vec3<f32>,
    @location(1) view_normal: vec3<f32>,
    @location(2) @interpolate(flat) group: u32,
    @location(3) @interpolate(flat) flags: u32,
    @location(4) world_pos: vec3<f32>,
};

@vertex
fn vs_main(v: StdVertex) -> VsOut {
    var out: VsOut;
    let world_pos = vec4<f32>(scene_position(v.position), 1.0);
    let view_pos = (frame.view * world_pos).xyz;
    out.clip_position = frame.proj * vec4<f32>(view_pos, 1.0);
    let world_n = scene_normal(oct_decode(v.normal_oct));
    let view_n = (frame.view * vec4<f32>(world_n, 0.0)).xyz;
    out.view_pos = view_pos;
    out.view_normal = view_n;
    out.group = v.group_id;
    out.flags = v.flags;
    out.world_pos = world_pos.xyz;
    return out;
}

// Per-rep visibility: atom must have the MESH bit set in repr_flags.
// `scene_visible(global)` (mask_lut OR-of-all-reps) would let through
// atoms that have only SURFACE visible — we'd render their mesh. The
// per-rep check keeps Mesh exclusive to atoms the user asked to see as
// wireframe.
fn mesh_visible(global_id: u32) -> bool {
    return scene_atom_in_rep(scene_atom(global_id), REP_BIT_MESH);
}

@fragment
fn fs_main(input: VsOut) -> TranslucentOut {
    let global_id = obj.atom_offset + input.group;
    if !scene_in_subset(global_id) { discard; }
    if !mesh_visible(global_id) {
        discard;
    }
    let n = normalize(input.view_normal);
    let base = scene_mesh_color(global_id);
    let view_dir = -input.view_pos;
    var lit = classic_lighting(n, view_dir, base.rgb);
    lit = apply_shadow(lit, input.world_pos);
    lit = apply_fog(lit, -input.view_pos.z);
    lit = apply_depth_cue(lit, -input.view_pos.z);
    let alpha = base.a * mesh_params.alpha_mul;
    let z_view = -input.view_pos.z;
    return write_translucent(lit, alpha, z_view, frame.clip.w);
}

@fragment
fn fs_opaque(input: VsOut) -> @location(0) vec4<f32> {
    let global_id = obj.atom_offset + input.group;
    if !scene_in_subset(global_id) { discard; }
    if !mesh_visible(global_id) {
        discard;
    }
    let n = normalize(input.view_normal);
    let base = scene_mesh_color(global_id);
    let view_dir = -input.view_pos;
    var lit = classic_lighting(n, view_dir, base.rgb);
    lit = apply_shadow(lit, input.world_pos);
    lit = apply_fog(lit, -input.view_pos.z);
    lit = apply_depth_cue(lit, -input.view_pos.z);
    return vec4<f32>(lit, 1.0);
}

@fragment
fn fs_depth(input: VsOut) {
    let global_id = obj.atom_offset + input.group;
    if !scene_in_subset(global_id) { discard; }
    if !mesh_visible(global_id) {
        discard;
    }
    let base = scene_mesh_color(global_id);
    let alpha = base.a * mesh_params.alpha_mul;
    if alpha < 0.999 {
        discard;
    }
    return;
}
