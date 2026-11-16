// Four-vertex hit quad for native one-pixel lines. It shares the native
// draw count, avoiding a GPU copy solely to change the vertex count.
// Depth is clamped to centerline endpoints in the fragment stage; only
// cursor neighborhoods are rasterized for exact manual queries.
// {{INCLUDE_FRAME}}
// {{INCLUDE_PICKING}}
// {{INCLUDE_PICKING_SCENE}}
// {{INCLUDE_LINE_QUAD}}

@group(2) @binding(0) var<uniform> picking: PickingParams;

struct VsOut {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) @interpolate(flat) groups: vec2<u32>,
    @location(1) @interpolate(linear) local: vec2<f32>,
    @location(2) @interpolate(flat) span: f32,
    @location(3) @interpolate(flat) depths: vec2<f32>,
    @location(4) @interpolate(flat) ownership: vec4<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) vid: u32, instance: LineInstance) -> VsOut {
    // The outer pairs of the shared eight-vertex strip bound the entire capsule.
    let corner = select(vid, vid + 4u, vid >= 2u);
    let radius = picking.line_half_width;
    let quad = line_project_visible(corner, scene_position(instance.p0_width.xyz), scene_position(instance.p1_pad.xyz), instance.groups, radius);
    var out: VsOut;
    out.clip_position = quad.clip_position;
    out.groups = instance.groups;
    out.local = quad.local;
    out.span = quad.span;
    out.depths = quad.depths;
    out.ownership = quad.ownership;
    if !scene_in_subset(obj.atom_offset + instance.groups.x) || !scene_in_subset(obj.atom_offset + instance.groups.y) {
        out.clip_position = vec4<f32>(2.0, 2.0, 2.0, 1.0);
    }
    return out;
}

struct Hit {
    @location(0) id: vec2<u32>,
    @builtin(frag_depth) depth: f32,
};

@fragment
fn fs_main(input: VsOut) -> Hit {
    if line_quad_outside(input.local, input.span) { discard; }
    let screen_t = clamp(input.local.x / max(input.span, LINE_DEGENERATE_PX), 0.0, 1.0);
    let world_t = screen_t * input.ownership.z / mix(input.ownership.w, input.ownership.z, screen_t);
    let t = mix(input.ownership.x, input.ownership.y, world_t);
    let group = select(input.groups.x, input.groups.y, t > 0.5);
    if !scene_visible(obj.atom_offset + group) { discard; }
    var hit: Hit;
    hit.id = pack_id(picking.rep_object, group);
    hit.depth = mix(input.depths.x, input.depths.y, screen_t);
    return hit;
}
