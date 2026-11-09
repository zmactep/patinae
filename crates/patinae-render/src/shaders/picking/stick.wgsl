// Stick picking — same capsule impostor as the colour shader, writes
// `PackedId(rep_kind=Stick, object_id, atom_id=nearer endpoint)`.

// {{INCLUDE_FRAME}}
// {{INCLUDE_CAPSULE}}
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
    @location(0) @interpolate(flat) p0_view:   vec3<f32>,
    @location(1) @interpolate(flat) p1_view:   vec3<f32>,
    @location(2) @interpolate(linear) ndc: vec2<f32>,
    @location(3) @interpolate(flat) radius:    f32,
    @location(4) @interpolate(flat) group_a: u32,
    @location(5) @interpolate(flat) group_b: u32,
};

@vertex
fn vs_main(
    @builtin(vertex_index) vid: u32,
    instance: StickInstance,
) -> VsOut {
    var out: VsOut;
    let uv = capsule_quad_uv(vid);
    let radius = instance.p0_radius.w;
    let p0_view = (frame.view * vec4<f32>(scene_position(instance.p0_radius.xyz), 1.0)).xyz;
    let p1_view = (frame.view * vec4<f32>(scene_position(instance.p1_pad.xyz), 1.0)).xyz;
    out.clip_position = capsule_billboard(uv, p0_view, p1_view, radius);
    out.p0_view = p0_view;
    out.p1_view = p1_view;
    out.ndc = out.clip_position.xy;
    out.radius  = radius;
    out.group_a = instance.groups.x;
    out.group_b = instance.groups.y;
    if !scene_in_subset(obj.atom_offset + instance.groups.x) || !scene_in_subset(obj.atom_offset + instance.groups.y) {
        out.clip_position = vec4<f32>(2.0, 2.0, 2.0, 1.0);
    }
    return out;
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
    let h = capsule_hit(input.ndc, input.p0_view, input.p1_view, input.radius);
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
