// Eight-vertex line strip with perspective-correct midpoint ownership.
// Caps keep endpoint depth; the body interpolates centerline depth.

// {{INCLUDE_FRAME}}
// {{INCLUDE_PICKING}}
// {{INCLUDE_PICKING_SCENE}}
// {{INCLUDE_LINE_QUAD}}

@group(2) @binding(0) var<uniform> picking: PickingParams;

struct VsOut {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) @interpolate(flat) groups: vec2<u32>,
    @location(4) group_t: f32,
    @location(1) @interpolate(linear) local: vec2<f32>,
    @location(2) @interpolate(flat) span: f32,
};

fn line_id_vertex(
    vid: u32,
    instance: LineInstance,
    min_half_width: f32,
) -> VsOut {
    var out: VsOut;
    let quad = line_project_visible(
        vid,
        scene_position(instance.p0_width.xyz),
        scene_position(instance.p1_pad.xyz),
        instance.groups,
        max(0.5 * instance.p0_width.w, min_half_width),
    );

    out.clip_position = quad.clip_position;
    out.groups = instance.groups;
    out.group_t = quad.group_t;
    out.local = quad.local;
    out.span = quad.span;
    if !scene_in_subset(obj.atom_offset + instance.groups.x) || !scene_in_subset(obj.atom_offset + instance.groups.y) {
        out.clip_position = vec4<f32>(2.0, 2.0, 2.0, 1.0);
    }
    return out;
}

@vertex
fn vs_main(@builtin(vertex_index) vid: u32, instance: LineInstance) -> VsOut {
    // The target owner resolves the physical-pixel footprint when ID dimensions change.
    return line_id_vertex(vid, instance, picking.line_half_width);
}

@vertex
fn vs_overlay(@builtin(vertex_index) vid: u32, instance: LineInstance) -> VsOut {
    return line_id_vertex(vid, instance, 0.0);
}

@fragment
fn fs_main(input: VsOut) -> @location(0) vec2<u32> {
    if line_quad_outside(input.local, input.span) { discard; }
    if !scene_visible(obj.atom_offset + select(input.groups.x, input.groups.y, input.group_t > 0.5)) { discard; }
    return pack_id(picking.rep_object, select(input.groups.x, input.groups.y, input.group_t > 0.5));
}
