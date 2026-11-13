// Line representation — one bond drawn as two screen-space half-bond quads
// so each half can take the colour of its endpoint atom (matches stick
// half-bond colouring) and the width follows the `line_width` setting.
// 12 vertices per instance (see `common/line_quad.wgsl`):
//   vid 0..5  → half-bond p0 → midpoint (group_a)
//   vid 6..11 → half-bond p1 → midpoint (group_b)
//
// `instance.groups` carries object-local atom indices. The fragment
// shader prepends `obj.atom_offset` to fetch global SceneStore entries.

// {{INCLUDE_FRAME}}
// {{INCLUDE_SCENE}}
// {{INCLUDE_WBOIT}}
// {{INCLUDE_LINE_QUAD}}

struct LineInstance {
    @location(0) p0_pad: vec4<f32>,  // xyz = atom1 world pos, w = width (px)
    @location(1) p1_pad: vec4<f32>,  // xyz = atom2 world pos
    @location(2) groups: vec2<u32>,  // .x = group_a, .y = group_b (object-local)
};

struct VsOut {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) view_z: f32,
    @location(1) @interpolate(flat) group: u32,
    @location(2) @interpolate(linear) local: vec2<f32>,
    @location(3) @interpolate(flat) span: f32,
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
        0.5 * instance.p0_pad.w,
    );

    out.clip_position = quad.clip_position;
    out.view_z = -mix(atom_view.z, mid_view.z, corner.x);
    out.group = select(instance.groups.x, instance.groups.y, half == 1u);
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
    let global_id = obj.atom_offset + input.group;
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
    let global_id = obj.atom_offset + input.group;
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
    let global_id = obj.atom_offset + input.group;
    if !scene_visible(global_id) {
        discard;
    }
    let base = scene_line_color(global_id);
    if base.a < 0.999 {
        discard;
    }
    return;
}
