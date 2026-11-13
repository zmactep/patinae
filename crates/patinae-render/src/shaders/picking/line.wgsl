// Line picking — same 12-vertex half-bond quad layout as line.wgsl. Flat
// `group` per half-bond encodes the endpoint atom_id.

// {{INCLUDE_FRAME}}
// {{INCLUDE_PICKING}}
// {{INCLUDE_PICKING_SCENE}}
// {{INCLUDE_LINE_QUAD}}

@group(2) @binding(0) var<uniform> picking: PickingParams;

// The ID target is rendered at a reduced picking scale (as low as 0.25 for
// the lite memory profile). Keep pick quads at least one ID texel wide so
// thin lines stay clickable.
const LINE_PICK_MIN_HALF_WIDTH_PX: f32 = 2.0;

struct LineInstance {
    @location(0) p0_pad: vec4<f32>,
    @location(1) p1_pad: vec4<f32>,
    @location(2) groups: vec2<u32>,
};

struct VsOut {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) @interpolate(flat) group: u32,
    @location(1) @interpolate(linear) local: vec2<f32>,
    @location(2) @interpolate(flat) span: f32,
};

@vertex
fn vs_main(
    @builtin(vertex_index) vid: u32,
    instance: LineInstance,
) -> VsOut {
    var out: VsOut;
    let p0 = instance.p0_pad.xyz;
    let p1 = instance.p1_pad.xyz;
    let half = line_quad_half(vid);
    let corner = line_quad_corner(vid);

    let atom_world = select(p0, p1, half == 1u);
    let mid_world = 0.5 * (p0 + p1);
    let atom_view = (frame.view * vec4<f32>(scene_position(atom_world), 1.0)).xyz;
    let mid_view = (frame.view * vec4<f32>(scene_position(mid_world), 1.0)).xyz;
    let quad = line_quad_vertex(
        frame.proj * vec4<f32>(atom_view, 1.0),
        frame.proj * vec4<f32>(mid_view, 1.0),
        corner,
        max(0.5 * instance.p0_pad.w, LINE_PICK_MIN_HALF_WIDTH_PX),
    );

    out.clip_position = quad.clip_position;
    out.group = select(instance.groups.x, instance.groups.y, half == 1u);
    out.local = quad.local;
    out.span = quad.span;
    if !scene_in_subset(obj.atom_offset + instance.groups.x) || !scene_in_subset(obj.atom_offset + instance.groups.y) {
        out.clip_position = vec4<f32>(2.0, 2.0, 2.0, 1.0);
    }
    return out;
}

@fragment
fn fs_main(input: VsOut) -> @location(0) vec2<u32> {
    if line_quad_outside(input.local, input.span) { discard; }
    if !scene_visible(obj.atom_offset + input.group) { discard; }
    return pack_id(picking.rep_object, input.group);
}
