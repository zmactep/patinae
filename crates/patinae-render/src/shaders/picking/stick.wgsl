// Stick picking — same capsule impostor as the colour shader, writes
// `PackedId(rep_kind=Stick, object_id, atom_id=nearer endpoint)`.

// {{INCLUDE_FRAME}}
// {{INCLUDE_PICKING}}
// {{INCLUDE_PICKING_SCENE}}

@group(2) @binding(0) var<uniform> picking: PickingParams;

struct StickInstance {
    @location(0) p0_radius: vec4<f32>,
    @location(1) p1_pad:    vec4<f32>,
    @location(2) groups:    vec2<u32>,
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

struct PickOut {
    @location(0) id:    vec2<u32>,
    @builtin(frag_depth) depth: f32,
};

@fragment
fn fs_main(input: VsOut) -> PickOut {
    if !scene_visible(obj.atom_offset + input.group_a) { discard; }
    let rd = safe_ray_dir(input.ray_dir);
    let h = ray_capsule(vec3<f32>(0.0), rd, input.p0_view, input.p1_view, input.radius);
    if !h.ok {
        discard;
    }
    // Pick atom owning the half of the capsule the fragment lies on.
    let mid = 0.5 * (input.p0_view + input.p1_view);
    let axis = input.p1_view - input.p0_view;
    var atom_id = input.group_a;
    if dot(h.point - mid, axis) > 0.0 { atom_id = input.group_b; }
    // Override rasterized depth with the actual ray-capsule hit so picking
    // depth-test ranks impostors against meshes correctly.
    let depth = hit_depth_or_discard(h.point);
    var out: PickOut;
    out.id    = pack_id(picking.rep_object, atom_id);
    out.depth = depth;
    return out;
}
