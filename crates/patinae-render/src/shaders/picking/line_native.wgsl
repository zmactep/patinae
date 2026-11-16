// Native line overlay IDs use four vertices. Flat-interp `group` on
// the provoking vertex of each half-segment encodes the endpoint atom_id.

// {{INCLUDE_FRAME}}
// {{INCLUDE_PICKING}}
// {{INCLUDE_PICKING_SCENE}}

@group(2) @binding(0) var<uniform> picking: PickingParams;

// {{INCLUDE_LINE_NATIVE}}

struct VsOut {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) @interpolate(flat) group: u32,
};

@vertex
fn vs_main(
    @builtin(vertex_index) vid: u32,
    instance: LineInstance,
) -> VsOut {
    var out: VsOut;
    let vertex = line_native_vertex(vid, instance.p0_width.xyz, instance.p1_pad.xyz);
    out.clip_position = vertex.clip_position;
    out.group = select(instance.groups.x, instance.groups.y, vid >= 2u);
    if !scene_in_subset(obj.atom_offset + instance.groups.x) || !scene_in_subset(obj.atom_offset + instance.groups.y) {
        out.clip_position = vec4<f32>(2.0, 2.0, 2.0, 1.0);
    }
    return out;
}

@fragment
fn fs_main(input: VsOut) -> @location(0) vec2<u32> {
    if !scene_visible(obj.atom_offset + input.group) { discard; }
    return pack_id(picking.rep_object, input.group);
}
