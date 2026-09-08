// Sphere impostor — billboard quad in view space + analytic ray-sphere FS.
// Fully opaque spheres write colour + depth; transparent spheres use WBOIT.
//
// `instance.group_id` carries the **object-local** atom index. The fragment
// shader prepends `obj.atom_offset` to fetch global SceneStore entries.
// Picking shader uses the local id directly so PackedId.atom_id is what
// the host expects.

// {{INCLUDE_FRAME}}
// {{INCLUDE_SCENE}}
// {{INCLUDE_WBOIT}}
// {{INCLUDE_LIGHTING}}

struct SphereParams {
    alpha_mul: f32,
    _pad0: f32,
    _pad1: f32,
    _pad2: f32,
};

@group(3) @binding(0) var<uniform> sphere_params: SphereParams;

struct SphereInstance {
    @location(0) center:   vec3<f32>,
    @location(1) radius:   f32,
    @location(2) group_id: u32,
};

struct VsOut {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) center_view: vec3<f32>,
    @location(1) ray_dir:     vec3<f32>,
    @location(2) radius:      f32,
    @location(3) @interpolate(flat) group_id: u32,
};

fn billboard_offset(vid: u32) -> vec2<f32> {
    let id = vid % 6u;
    var x: f32 = -1.0;
    var y: f32 = -1.0;
    if id == 1u || id == 4u || id == 5u { x = 1.0; }
    if id == 2u || id == 3u || id == 5u { y = 1.0; }
    return vec2<f32>(x, y);
}

@vertex
fn vs_main(
    @builtin(vertex_index) vid: u32,
    instance: SphereInstance,
) -> VsOut {
    var out: VsOut;

    let off = billboard_offset(vid);
    let center_view = (frame.view * vec4<f32>(scene_position(instance.center), 1.0)).xyz;
    let scale = instance.radius * 1.5;
    let billboard_pos = center_view + vec3<f32>(off * scale, 0.0);

    out.clip_position = frame.proj * vec4<f32>(billboard_pos, 1.0);
    out.center_view = center_view;
    out.radius      = instance.radius;
    out.ray_dir     = billboard_pos;
    out.group_id    = instance.group_id;
    return out;
}

struct OpaqueOut {
    @builtin(frag_depth) depth: f32,
    @location(0)         color: vec4<f32>,
};

struct SphereDepthOut {
    @builtin(frag_depth) depth: f32,
};

struct SphereShade {
    clip_z:  f32,
    lit:     vec3<f32>,
    alpha:   f32,
    z_view:  f32,
    world:   vec3<f32>,
};

fn shade_sphere(input: VsOut, global_id: u32) -> SphereShade {
    let ray_dir = normalize(input.ray_dir);
    let oc = -input.center_view;
    let a = dot(ray_dir, ray_dir);
    let b = 2.0 * dot(oc, ray_dir);
    let c = dot(oc, oc) - input.radius * input.radius;
    let disc = b * b - 4.0 * a * c;
    if disc < 0.0 {
        discard;
    }
    let t = (-b - sqrt(disc)) / (2.0 * a);
    if t < 0.0 {
        discard;
    }
    let hit = ray_dir * t;
    let normal = normalize(hit - input.center_view);

    let base = scene_sphere_color(global_id);
    let view_dir = -hit;
    var lit = classic_lighting(normal, view_dir, base.rgb);
    lit = apply_fog(lit, -hit.z);
    lit = apply_depth_cue(lit, -hit.z);

    let clip = frame.proj * vec4<f32>(hit, 1.0);
    let clip_z = clip.z / clip.w;

    let atom = scene_atom(global_id);
    let alpha_mul = scene_atom_sphere_alpha(atom, sphere_params.alpha_mul);

    var out: SphereShade;
    out.clip_z = clip_z;
    out.lit    = lit;
    out.alpha  = base.a * alpha_mul;
    out.z_view = -hit.z;
    out.world  = (frame.view_inv * vec4<f32>(hit, 1.0)).xyz;
    return out;
}

@fragment
fn fs_main(input: VsOut) -> TranslucentOut {
    let global_id = obj.atom_offset + input.group_id;
    if !scene_visible(global_id) {
        discard;
    }
    let s = shade_sphere(input, global_id);
    let lit = apply_shadow(s.lit, s.world);
    return write_translucent(lit, s.alpha, s.z_view, frame.clip.w);
}

@fragment
fn fs_opaque(input: VsOut) -> OpaqueOut {
    let global_id = obj.atom_offset + input.group_id;
    if !scene_visible(global_id) {
        discard;
    }
    let s = shade_sphere(input, global_id);
    var out: OpaqueOut;
    out.depth = s.clip_z;
    out.color = vec4<f32>(apply_shadow(s.lit, s.world), 1.0);
    return out;
}

@fragment
fn fs_depth(input: VsOut) -> SphereDepthOut {
    let global_id = obj.atom_offset + input.group_id;
    if !scene_visible(global_id) {
        discard;
    }
    let ray_dir = normalize(input.ray_dir);
    let oc = -input.center_view;
    let a = dot(ray_dir, ray_dir);
    let b = 2.0 * dot(oc, ray_dir);
    let c = dot(oc, oc) - input.radius * input.radius;
    let disc = b * b - 4.0 * a * c;
    if disc < 0.0 { discard; }
    let t = (-b - sqrt(disc)) / (2.0 * a);
    if t < 0.0 { discard; }
    let hit = ray_dir * t;
    let base = scene_sphere_color(global_id);
    let atom = scene_atom(global_id);
    let alpha_mul = scene_atom_sphere_alpha(atom, sphere_params.alpha_mul);
    let alpha = base.a * alpha_mul;
    if alpha < 0.999 { discard; }

    let clip = frame.proj * vec4<f32>(hit, 1.0);
    if clip.w <= 0.0 {
        discard;
    }
    let depth = clip.z / clip.w;
    if depth < 0.0 || depth > 1.0 {
        discard;
    }
    var out: SphereDepthOut;
    out.depth = depth;
    return out;
}
