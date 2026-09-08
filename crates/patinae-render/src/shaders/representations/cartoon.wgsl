// Cartoon / ribbon representation — `StdVertex` triangle list. 24 B per
// vertex (position + octahedral-encoded normal + group_id + flags). One
// indexed draw per rep.
//
// `instance.group_id` carries the **object-local** Cα atom id (compute
// kernel writes `ExtrudePoint.atom_idx`). FS prepends `obj.atom_offset`
// to fetch global SceneStore entries.

// {{INCLUDE_FRAME}}
// {{INCLUDE_SCENE}}
// {{INCLUDE_WBOIT}}
// {{INCLUDE_LIGHTING}}
// {{INCLUDE_OCTAHEDRAL}}

struct CartoonParams {
    alpha_mul: f32,
    color_slot: u32,
    _pad1: u32,
    _pad2: u32,
};

@group(3) @binding(0) var<uniform> cartoon_params: CartoonParams;

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

// `flags` bit 0 — open mesh requests two-sided lighting (back-face normal
// flip). Set per-vertex on body / arrow / cap / shoulder geometry so the
// fragment shader lights both sides of those faces. Closed tubes
// (loops/helices from GPU compute) leave the bit clear and shade with the
// classic single-sided Lambert.
const FLAG_TWO_SIDED: u32 = 1u << 0u;
const CARTOON_COLOR_SLOT_CARTOON: u32 = 0u;
const CARTOON_COLOR_SLOT_RIBBON: u32 = 1u;

fn cartoon_base_color(global_id: u32) -> vec4<f32> {
    if cartoon_params.color_slot == CARTOON_COLOR_SLOT_RIBBON {
        return scene_ribbon_color(global_id);
    }
    return scene_cartoon_color(global_id);
}

fn cartoon_rep_bit() -> u32 {
    if cartoon_params.color_slot == CARTOON_COLOR_SLOT_RIBBON {
        return REP_BIT_RIBBON;
    }
    return REP_BIT_CARTOON;
}

fn cartoon_visible(atom: AtomGpu) -> bool {
    return scene_atom_in_rep(atom, cartoon_rep_bit());
}

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

@fragment
fn fs_main(input: VsOut, @builtin(front_facing) front_facing: bool) -> TranslucentOut {
    let global_id = obj.atom_offset + input.group;
    if !scene_in_subset(global_id) { discard; }
    let atom = scene_atom(global_id);
    if !cartoon_visible(atom) {
        discard;
    }
    var n = normalize(input.view_normal);
    if ((input.flags & FLAG_TWO_SIDED) != 0u) && !front_facing {
        n = -n;
    }
    let base = cartoon_base_color(global_id);
    let view_dir = -input.view_pos;
    var lit = classic_lighting(n, view_dir, base.rgb);
    lit = apply_shadow(lit, input.world_pos);
    lit = apply_fog(lit, -input.view_pos.z);
    lit = apply_depth_cue(lit, -input.view_pos.z);
    let alpha_mul = scene_atom_cartoon_alpha(atom, cartoon_params.alpha_mul);
    let alpha = base.a * alpha_mul;
    let z_view = -input.view_pos.z;
    return write_translucent(lit, alpha, z_view, frame.clip.w);
}

@fragment
fn fs_opaque(input: VsOut, @builtin(front_facing) front_facing: bool) -> @location(0) vec4<f32> {
    let global_id = obj.atom_offset + input.group;
    if !scene_in_subset(global_id) { discard; }
    let atom = scene_atom(global_id);
    if !cartoon_visible(atom) {
        discard;
    }
    var n = normalize(input.view_normal);
    if ((input.flags & FLAG_TWO_SIDED) != 0u) && !front_facing {
        n = -n;
    }
    let base = cartoon_base_color(global_id);
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
    let atom = scene_atom(global_id);
    if !cartoon_visible(atom) {
        discard;
    }
    let base = cartoon_base_color(global_id);
    let alpha_mul = scene_atom_cartoon_alpha(atom, cartoon_params.alpha_mul);
    let alpha = base.a * alpha_mul;
    if alpha < 0.999 {
        discard;
    }
    return;
}
