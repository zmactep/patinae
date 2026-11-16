// Eight-vertex line strip with perspective-correct midpoint ownership.
// Caps keep endpoint depth; the body interpolates centerline depth.

// {{INCLUDE_FRAME}}
// {{INCLUDE_SCENE}}
// {{INCLUDE_WBOIT}}
// {{INCLUDE_LINE_QUAD}}

struct VsOut {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) view_z: f32,
    @location(1) @interpolate(flat) groups: vec2<u32>,
    @location(4) group_t: f32,
    @location(2) @interpolate(linear) local: vec2<f32>,
    @location(3) @interpolate(flat) span: f32,
};

@vertex
fn vs_main(
    @builtin(vertex_index) vid: u32,
    instance: LineInstance,
) -> VsOut {
    var out: VsOut;
    let quad = line_project_visible(
        vid,
        scene_position(instance.p0_width.xyz),
        scene_position(instance.p1_pad.xyz),
        instance.groups,
        0.5 * instance.p0_width.w,
    );

    out.clip_position = quad.clip_position;
    out.view_z = quad.view_z;
    out.groups = instance.groups;
    out.group_t = quad.group_t;
    out.local = quad.local;
    out.span = quad.span;
    if !scene_in_subset(obj.atom_offset + instance.groups.x) || !scene_in_subset(obj.atom_offset + instance.groups.y) {
        out.clip_position = vec4<f32>(2.0, 2.0, 2.0, 1.0);
    }
    return out;
}

@fragment
fn fs_main(input: VsOut) -> TranslucentOut {
    if line_quad_outside(input.local, input.span) {
        discard;
    }
    let global_id = obj.atom_offset + select(input.groups.x, input.groups.y, input.group_t > 0.5);
    if !scene_visible(global_id) {
        discard;
    }
    let base = scene_line_color(global_id);
    return write_translucent(base.rgb, base.a, input.view_z, frame.clip.w);
}

@fragment
fn fs_opaque(input: VsOut) -> @location(0) vec4<f32> {
    if line_quad_outside(input.local, input.span) {
        discard;
    }
    let global_id = obj.atom_offset + select(input.groups.x, input.groups.y, input.group_t > 0.5);
    if !scene_visible(global_id) {
        discard;
    }
    let base = scene_line_color(global_id);
    return vec4<f32>(base.rgb, 1.0);
}

@fragment
fn fs_depth(input: VsOut) {
    if line_quad_outside(input.local, input.span) {
        discard;
    }
    let global_id = obj.atom_offset + select(input.groups.x, input.groups.y, input.group_t > 0.5);
    if !scene_visible(global_id) {
        discard;
    }
    let base = scene_line_color(global_id);
    if base.a < 0.999 {
        discard;
    }
    return;
}
