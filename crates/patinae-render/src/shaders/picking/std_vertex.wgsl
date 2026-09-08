// Picking shader for `StdVertex` representations (cartoon, ribbon, surface,
// mesh). Flat-interp `group` per primitive (provoking vertex) gives the
// picking atom_id. Surface/cartoon/ribbon bind a TriangleList pipeline;
// mesh binds a LineList variant of the same shader.

// {{INCLUDE_FRAME}}
// {{INCLUDE_PICKING}}
// {{INCLUDE_PICKING_SCENE}}

@group(2) @binding(0) var<uniform> picking: PickingParams;

struct StdVertex {
    @location(0) position:    vec3<f32>,
    @location(1) normal_oct:  u32,
    @location(2) group_id:    u32,
    @location(3) flags:       u32,
};

struct VsOut {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) @interpolate(flat) group: u32,
};

@vertex
fn vs_main(v: StdVertex) -> VsOut {
    var out: VsOut;
    let view_pos = (frame.view * vec4<f32>(scene_position(v.position), 1.0)).xyz;
    out.clip_position = frame.proj * vec4<f32>(view_pos, 1.0);
    out.group = v.group_id;
    return out;
}

@fragment
fn fs_main(input: VsOut) -> @location(0) vec2<u32> {
    if !scene_visible(obj.atom_offset + input.group) { discard; }
    return pack_id(picking.rep_object, input.group);
}
