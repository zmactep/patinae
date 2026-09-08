// Stick (capsule) impostor — billboard around the bond axis, ray-capsule FS.
// One impostor per (sub-)bond covers the full capsule; no separate cap pass.
// Writes through WBOIT.
//
// `instance.groups` carries (atom_a_local, atom_b_local). The fragment
// shader prepends `obj.atom_offset` to fetch global SceneStore entries.
// Picking shader uses local ids directly so PackedId.atom_id matches the
// host's expectations.

// {{INCLUDE_FRAME}}
// {{INCLUDE_SCENE}}
// {{INCLUDE_WBOIT}}
// {{INCLUDE_LIGHTING}}

struct StickParams {
    alpha_mul: f32,
    _pad0: f32,
    _pad1: f32,
    _pad2: f32,
};

@group(3) @binding(0) var<uniform> stick_params: StickParams;

struct StickInstance {
    @location(0) p0_radius: vec4<f32>,  // xyz = atom1 world pos, w = radius
    @location(1) p1_pad:    vec4<f32>,  // xyz = atom2 world pos, w = unused
    @location(2) groups:    vec2<u32>,  // .x = group_id_a, .y = group_id_b (object-local)
};

struct VsOut {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) p0_view:   vec3<f32>,
    @location(1) p1_view:   vec3<f32>,
    @location(2) ray_dir:   vec3<f32>,
    @location(3) radius:    f32,
    @location(4) @interpolate(flat) group_a: u32,
    @location(5) @interpolate(flat) group_b: u32,
};

fn quad_uv(vid: u32) -> vec2<f32> {
    let id = vid % 6u;
    var x: f32 = -1.0;
    var y: f32 = -1.0;
    if id == 1u || id == 4u || id == 5u { x = 1.0; }
    if id == 2u || id == 3u || id == 5u { y = 1.0; }
    return vec2<f32>(x, y);
}

fn billboard_view_pos(uv: vec2<f32>, p0v: vec3<f32>, p1v: vec3<f32>, radius: f32) -> vec3<f32> {
    let mid = 0.5 * (p0v + p1v);
    let axis_xy = p1v.xy - p0v.xy;
    let axis_len = length(axis_xy);
    let pad = radius * 1.75;
    var axis_dir = vec2<f32>(1.0, 0.0);
    var half_len = pad;
    if axis_len > 1e-6 {
        axis_dir = axis_xy / axis_len;
        half_len = 0.5 * axis_len + pad;
    }
    let side = vec2<f32>(-axis_dir.y, axis_dir.x);
    let xy = mid.xy + axis_dir * (uv.x * half_len) + side * (uv.y * pad);
    return vec3<f32>(xy, mid.z);
}

@vertex
fn vs_main(
    @builtin(vertex_index) vid: u32,
    instance: StickInstance,
) -> VsOut {
    var out: VsOut;
    let uv = quad_uv(vid);
    let radius = instance.p0_radius.w;
    let p0_view = (frame.view * vec4<f32>(scene_position(instance.p0_radius.xyz), 1.0)).xyz;
    let p1_view = (frame.view * vec4<f32>(scene_position(instance.p1_pad.xyz), 1.0)).xyz;
    let pos_view = billboard_view_pos(uv, p0_view, p1_view, radius);
    out.clip_position = frame.proj * vec4<f32>(pos_view, 1.0);
    out.p0_view = p0_view;
    out.p1_view = p1_view;
    out.ray_dir = pos_view;
    out.radius  = radius;
    out.group_a = instance.groups.x;
    out.group_b = instance.groups.y;
    if !scene_in_subset(obj.atom_offset + instance.groups.x) || !scene_in_subset(obj.atom_offset + instance.groups.y) {
        out.clip_position = vec4<f32>(2.0, 2.0, 2.0, 1.0);
    }
    return out;
}

struct CapsuleHit {
    t:      f32,
    point:  vec3<f32>,
    normal: vec3<f32>,
    ok:     bool,
};

fn ray_capsule(ro: vec3<f32>, rd: vec3<f32>, pa: vec3<f32>, pb: vec3<f32>, r: f32) -> CapsuleHit {
    var hit: CapsuleHit;
    hit.ok = false;
    hit.t = 1e30;
    let ba = pb - pa;
    let oa = ro - pa;
    let baba = dot(ba, ba);
    let bard = dot(ba, rd);
    let baoa = dot(ba, oa);
    let rdoa = dot(rd, oa);
    let oaoa = dot(oa, oa);
    let a = baba - bard * bard;
    let b = baba * rdoa - baoa * bard;
    let c = baba * oaoa - baoa * baoa - r * r * baba;
    let h = b * b - a * c;
    if h >= 0.0 && a > 1e-8 {
        let t = (-b - sqrt(h)) / a;
        let y = baoa + t * bard;
        if t > 0.0 && y > 0.0 && y < baba {
            hit.t = t;
            hit.point = ro + rd * t;
            let axis_proj = pa + ba * (y / baba);
            hit.normal = normalize(hit.point - axis_proj);
            hit.ok = true;
            return hit;
        }
    }
    for (var i = 0u; i < 2u; i = i + 1u) {
        var center: vec3<f32>;
        if i == 0u { center = pa; } else { center = pb; }
        let oc = ro - center;
        let bb = dot(rd, oc);
        let cc = dot(oc, oc) - r * r;
        let dd = bb * bb - cc;
        if dd < 0.0 { continue; }
        let t = -bb - sqrt(dd);
        if t > 0.0 && t < hit.t {
            hit.t = t;
            hit.point = ro + rd * t;
            hit.normal = normalize(hit.point - center);
            hit.ok = true;
        }
    }
    return hit;
}

fn safe_ray_dir(v: vec3<f32>) -> vec3<f32> {
    let len2 = dot(v, v);
    if len2 < 1e-12 {
        return vec3<f32>(0.0, 0.0, -1.0);
    }
    return v * inverseSqrt(len2);
}

fn nearer_endpoint(hit_point: vec3<f32>, pa: vec3<f32>, pb: vec3<f32>) -> u32 {
    let mid = 0.5 * (pa + pb);
    let axis = pb - pa;
    if dot(hit_point - mid, axis) < 0.0 {
        return 0u;
    }
    return 1u;
}

// Resolve global atom id + sub-unit alpha for a half-bond endpoint.
struct EndpointShade {
    visible: bool,
    global:  u32,
    base:    vec4<f32>,
    alpha:   f32,
};

fn endpoint_shade(local_group: u32) -> EndpointShade {
    let global = obj.atom_offset + local_group;
    let atom = scene_atom(global);
    var s: EndpointShade;
    s.visible = scene_visible(global);
    s.global = global;
    s.base = scene_stick_color(global);
    s.alpha = scene_atom_stick_alpha(atom, stick_params.alpha_mul);
    return s;
}

fn hit_depth_or_discard(point: vec3<f32>) -> f32 {
    let clip = frame.proj * vec4<f32>(point, 1.0);
    if clip.w <= 0.0 {
        discard;
    }
    let depth = clip.z / clip.w;
    if depth < 0.0 || depth > 1.0 {
        discard;
    }
    return depth;
}

struct StickFsOut {
    @location(0) accum:  vec4<f32>,
    @location(1) reveal: f32,
    @builtin(frag_depth) depth: f32,
};

struct StickDepthOut {
    @builtin(frag_depth) depth: f32,
};

@fragment
fn fs_main(input: VsOut) -> StickFsOut {
    let sa = endpoint_shade(input.group_a);
    let sb = endpoint_shade(input.group_b);
    if !sa.visible && !sb.visible {
        discard;
    }

    let rd = safe_ray_dir(input.ray_dir);
    let h = ray_capsule(vec3<f32>(0.0), rd, input.p0_view, input.p1_view, input.radius);
    if !h.ok {
        discard;
    }

    let endpoint = nearer_endpoint(h.point, input.p0_view, input.p1_view);
    var s: EndpointShade = sa;
    if endpoint == 1u { s = sb; }
    if !s.visible {
        if endpoint == 0u { s = sb; } else { s = sa; }
    }

    let view_dir = -h.point;
    var lit = classic_lighting(h.normal, view_dir, s.base.rgb);
    let world_pos = (frame.view_inv * vec4<f32>(h.point, 1.0)).xyz;
    lit = apply_shadow(lit, world_pos);
    lit = apply_fog(lit, -h.point.z);
    lit = apply_depth_cue(lit, -h.point.z);
    let alpha = s.base.a * s.alpha;
    let z_view = -h.point.z;
    let wboit = write_translucent(lit, alpha, z_view, frame.clip.w);

    let depth = hit_depth_or_discard(h.point);

    var out: StickFsOut;
    out.accum  = wboit.accum;
    out.reveal = wboit.reveal;
    out.depth  = depth;
    return out;
}

struct StickOpaqueOut {
    @builtin(frag_depth) depth: f32,
    @location(0)         color: vec4<f32>,
};

@fragment
fn fs_opaque(input: VsOut) -> StickOpaqueOut {
    let sa = endpoint_shade(input.group_a);
    let sb = endpoint_shade(input.group_b);
    if !sa.visible && !sb.visible { discard; }

    let rd = safe_ray_dir(input.ray_dir);
    let h = ray_capsule(vec3<f32>(0.0), rd, input.p0_view, input.p1_view, input.radius);
    if !h.ok { discard; }

    let endpoint = nearer_endpoint(h.point, input.p0_view, input.p1_view);
    var s: EndpointShade = sa;
    if endpoint == 1u { s = sb; }
    if !s.visible {
        if endpoint == 0u { s = sb; } else { s = sa; }
    }

    let view_dir = -h.point;
    var lit = classic_lighting(h.normal, view_dir, s.base.rgb);
    let world_pos = (frame.view_inv * vec4<f32>(h.point, 1.0)).xyz;
    lit = apply_shadow(lit, world_pos);
    lit = apply_fog(lit, -h.point.z);
    lit = apply_depth_cue(lit, -h.point.z);

    let depth = hit_depth_or_discard(h.point);
    var out: StickOpaqueOut;
    out.depth = depth;
    out.color = vec4<f32>(lit, 1.0);
    return out;
}

@fragment
fn fs_depth(input: VsOut) -> StickDepthOut {
    let sa = endpoint_shade(input.group_a);
    let sb = endpoint_shade(input.group_b);
    if !sa.visible && !sb.visible { discard; }

    let rd = safe_ray_dir(input.ray_dir);
    let h = ray_capsule(vec3<f32>(0.0), rd, input.p0_view, input.p1_view, input.radius);
    if !h.ok { discard; }

    let endpoint = nearer_endpoint(h.point, input.p0_view, input.p1_view);
    var s: EndpointShade = sa;
    if endpoint == 1u { s = sb; }
    if !s.visible {
        if endpoint == 0u { s = sb; } else { s = sa; }
    }
    let alpha = s.base.a * s.alpha;
    if alpha < 0.999 { discard; }

    let depth = hit_depth_or_discard(h.point);
    var out: StickDepthOut;
    out.depth = depth;
    return out;
}
