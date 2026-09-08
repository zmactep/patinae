// Surface representation — `StdVertex` triangle list emitted by the MC
// compute pipeline. 24 B per vertex (position + octahedral-encoded
// normal + group_id + flags).
//
// `instance.group_id` is the **object-local** primary owner-atom index emitted
// by the surface MC pass. `flags` carries the optional secondary owner used for
// narrow color-edge smoothing. FS prepends `obj.atom_offset` to fetch global
// SceneStore entries.
//
// Visibility check uses **per-rep** `REP_BIT_SURFACE` instead of the
// generic `scene_visible` (mask_lut OR) so atoms that have only MESH
// visible (wireframe) don't render as solid surface here.

// {{INCLUDE_FRAME}}
// {{INCLUDE_SCENE}}
// {{INCLUDE_WBOIT}}
// {{INCLUDE_LIGHTING}}
// {{INCLUDE_OCTAHEDRAL}}

struct SurfaceParams {
    alpha_mul: f32,
    color_smoothing: f32,
    color_smoothing_threshold: f32,
    _pad2: f32,
};

@group(3) @binding(0) var<uniform> surface_params: SurfaceParams;

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

fn surface_color_metric(world_pos: vec3<f32>, global_id: u32) -> f32 {
    let atom = scene_atom(global_id);
    let radius = max(atom.vdw, 1e-3);
    return length(world_pos - scene_position(scene_coord(global_id))) / radius;
}

fn surface_blended_color(owner_global: u32, secondary_local: u32, world_pos: vec3<f32>) -> vec4<f32> {
    let owner_color = scene_surface_color(owner_global);
    let smoothing = clamp(surface_params.color_smoothing, 0.0, 4.0);
    let width = max(surface_params.color_smoothing_threshold, 1e-4);
    if smoothing < 0.5 || width <= 1e-4 {
        return owner_color;
    }

    let owner_local = owner_global - obj.atom_offset;
    if secondary_local == owner_local || secondary_local >= obj.atom_count {
        return owner_color;
    }

    let secondary_global = obj.atom_offset + secondary_local;
    let secondary_atom = scene_atom(secondary_global);
    if !scene_atom_in_rep(secondary_atom, REP_BIT_SURFACE) {
        return owner_color;
    }

    let owner_metric = surface_color_metric(world_pos, owner_global);
    let secondary_metric = surface_color_metric(world_pos, secondary_global);
    let blend = smoothstep(-width, width, owner_metric - secondary_metric);
    return mix(owner_color, scene_surface_color(secondary_global), blend);
}

@fragment
fn fs_main(input: VsOut) -> TranslucentOut {
    let global_id = obj.atom_offset + input.group;
    if !scene_in_subset(global_id) { discard; }
    if !scene_atom_in_rep(scene_atom(global_id), REP_BIT_SURFACE) {
        discard;
    }
    let n = normalize(input.view_normal);
    let base = surface_blended_color(global_id, input.flags, input.world_pos);
    let view_dir = -input.view_pos;
    var lit = classic_lighting(n, view_dir, base.rgb);
    lit = apply_shadow(lit, input.world_pos);
    lit = apply_fog(lit, -input.view_pos.z);
    lit = apply_depth_cue(lit, -input.view_pos.z);
    let atom = scene_atom(global_id);
    let alpha_mul = scene_atom_surface_alpha(atom, surface_params.alpha_mul);
    let alpha = base.a * alpha_mul;
    let z_view = -input.view_pos.z;
    return write_translucent(lit, alpha, z_view, frame.clip.w);
}

@fragment
fn fs_opaque(input: VsOut) -> @location(0) vec4<f32> {
    let global_id = obj.atom_offset + input.group;
    if !scene_in_subset(global_id) { discard; }
    if !scene_atom_in_rep(scene_atom(global_id), REP_BIT_SURFACE) {
        discard;
    }
    let n = normalize(input.view_normal);
    let base = surface_blended_color(global_id, input.flags, input.world_pos);
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
    if !scene_atom_in_rep(scene_atom(global_id), REP_BIT_SURFACE) {
        discard;
    }
    let base = scene_surface_color(global_id);
    let atom = scene_atom(global_id);
    let alpha_mul = scene_atom_surface_alpha(atom, surface_params.alpha_mul);
    let alpha = base.a * alpha_mul;
    if alpha < 0.999 {
        discard;
    }
    return;
}
