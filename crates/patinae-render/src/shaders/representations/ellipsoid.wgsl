// Ellipsoid impostor — billboard quad in view space + analytic ray-ellipsoid
// FS. Each atom carries three orthogonal semi-axis vectors (eigenvectors of
// the U-tensor scaled by sqrt(eigenvalue) · ellipsoid_scale · probability
// factor). Writes through WBOIT.
//
// `instance.group_id` is object-local; the FS prepends `obj.atom_offset`
// to fetch global SceneStore entries.
//
// Ray-ellipsoid: build the local frame [a0 | a1 | a2] (orthogonal columns,
// not orthonormal — column lengths encode the semi-axis radii). The
// inverse maps world-space points into the unit-sphere space of the
// ellipsoid. Since columns are orthogonal, M^{-1} row i = a_i / dot(a_i, a_i).
// Intersect the transformed ray with the unit sphere; transform back to
// view space for shading + depth.

// {{INCLUDE_FRAME}}
// {{INCLUDE_SCENE}}
// {{INCLUDE_WBOIT}}
// {{INCLUDE_LIGHTING}}

struct EllipsoidParams {
    alpha_mul: f32,
    _pad0: f32,
    _pad1: f32,
    _pad2: f32,
};

@group(3) @binding(0) var<uniform> ellipsoid_params: EllipsoidParams;

struct EllipsoidInstance {
    @location(0) center:     vec3<f32>,
    @location(1) max_extent: f32,
    @location(2) axis0:      vec3<f32>,
    @location(3) group_id:   u32,
    @location(4) axis1:      vec3<f32>,
    @location(5) _pad0:      u32,
    @location(6) axis2:      vec3<f32>,
    @location(7) _pad1:      u32,
};

struct VsOut {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) center_view: vec3<f32>,
    @location(1) ray_dir:     vec3<f32>,
    @location(2) ax0_view:    vec3<f32>,
    @location(3) ax1_view:    vec3<f32>,
    @location(4) ax2_view:    vec3<f32>,
    @location(5) @interpolate(flat) group_id: u32,
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
    instance: EllipsoidInstance,
) -> VsOut {
    var out: VsOut;

    let off = billboard_offset(vid);
    let center_view = (frame.view * vec4<f32>(scene_position(instance.center), 1.0)).xyz;
    let scale = instance.max_extent * 1.5;
    let billboard_pos = center_view + vec3<f32>(off * scale, 0.0);

    let view33 = mat3x3<f32>(
        frame.view[0].xyz,
        frame.view[1].xyz,
        frame.view[2].xyz,
    );

    out.clip_position = frame.proj * vec4<f32>(billboard_pos, 1.0);
    out.center_view   = center_view;
    out.ray_dir       = billboard_pos;
    out.ax0_view      = view33 * scene_direction(instance.axis0);
    out.ax1_view      = view33 * scene_direction(instance.axis1);
    out.ax2_view      = view33 * scene_direction(instance.axis2);
    out.group_id      = instance.group_id;
    return out;
}

struct EllipsoidShade {
    clip_z: f32,
    lit:    vec3<f32>,
    alpha:  f32,
    z_view: f32,
    world:  vec3<f32>,
};

fn shade_ellipsoid(input: VsOut, global_id: u32) -> EllipsoidShade {
    let ray_dir = normalize(input.ray_dir);
    let inv0 = input.ax0_view / max(dot(input.ax0_view, input.ax0_view), 1e-12);
    let inv1 = input.ax1_view / max(dot(input.ax1_view, input.ax1_view), 1e-12);
    let inv2 = input.ax2_view / max(dot(input.ax2_view, input.ax2_view), 1e-12);
    let oc_view = -input.center_view;
    let d_local = vec3<f32>(dot(ray_dir, inv0), dot(ray_dir, inv1), dot(ray_dir, inv2));
    let o_local = vec3<f32>(dot(oc_view, inv0), dot(oc_view, inv1), dot(oc_view, inv2));
    let a = dot(d_local, d_local);
    let b = 2.0 * dot(o_local, d_local);
    let c = dot(o_local, o_local) - 1.0;
    let disc = b * b - 4.0 * a * c;
    if disc < 0.0 { discard; }
    let t = (-b - sqrt(disc)) / (2.0 * a);
    if t < 0.0 { discard; }

    let hit_view = ray_dir * t;
    let n_local = o_local + d_local * t;
    let normal_raw = inv0 * n_local.x + inv1 * n_local.y + inv2 * n_local.z;
    let normal = normalize(normal_raw);

    let base = scene_ellipsoid_color(global_id);
    let view_dir = -hit_view;
    var lit = classic_lighting(normal, view_dir, base.rgb);
    lit = apply_fog(lit, -hit_view.z);
    lit = apply_depth_cue(lit, -hit_view.z);

    let clip = frame.proj * vec4<f32>(hit_view, 1.0);
    let atom = scene_atom(global_id);
    let alpha_mul = scene_atom_ellipsoid_alpha(atom, ellipsoid_params.alpha_mul);

    var out: EllipsoidShade;
    out.clip_z = clip.z / clip.w;
    out.lit    = lit;
    out.alpha  = base.a * alpha_mul;
    out.z_view = -hit_view.z;
    out.world  = (frame.view_inv * vec4<f32>(hit_view, 1.0)).xyz;
    return out;
}

@fragment
fn fs_main(input: VsOut) -> TranslucentOut {
    let global_id = obj.atom_offset + input.group_id;
    if !scene_visible(global_id) {
        discard;
    }
    let s = shade_ellipsoid(input, global_id);
    let lit = apply_shadow(s.lit, s.world);
    return write_translucent(lit, s.alpha, s.z_view, frame.clip.w);
}

struct EllipsoidOpaqueOut {
    @builtin(frag_depth) depth: f32,
    @location(0)         color: vec4<f32>,
};

@fragment
fn fs_opaque(input: VsOut) -> EllipsoidOpaqueOut {
    let global_id = obj.atom_offset + input.group_id;
    if !scene_visible(global_id) {
        discard;
    }
    let s = shade_ellipsoid(input, global_id);
    var out: EllipsoidOpaqueOut;
    out.depth = s.clip_z;
    out.color = vec4<f32>(apply_shadow(s.lit, s.world), 1.0);
    return out;
}

@fragment
fn fs_depth(input: VsOut) {
    let global_id = obj.atom_offset + input.group_id;
    if !scene_visible(global_id) {
        discard;
    }
    let s = shade_ellipsoid(input, global_id);
    if s.alpha < 0.999 {
        discard;
    }
    return;
}
