// Line representation — one bond drawn as two LineList segments so each half
// can take the colour of its endpoint atom (matches stick half-bond colouring).
// 4 vertices per instance:
//   vid 0 → p0 (provoking vertex of line 0, group_a)
//   vid 1 → midpoint (group_a)
//   vid 2 → midpoint (provoking vertex of line 1, group_b)
//   vid 3 → p1 (group_b)
// WGSL `flat` interpolation pulls from the provoking (first) vertex of each
// primitive, so each half-line carries its endpoint atom's group id.
//
// `instance.groups` carries object-local atom indices. The fragment
// shader prepends `obj.atom_offset` to fetch global SceneStore entries.

// {{INCLUDE_FRAME}}
// {{INCLUDE_SCENE}}
// {{INCLUDE_WBOIT}}

struct LineInstance {
    @location(0) p0_pad: vec4<f32>,  // xyz = atom1 world pos
    @location(1) p1_pad: vec4<f32>,  // xyz = atom2 world pos
    @location(2) groups: vec2<u32>,  // .x = group_a, .y = group_b (object-local)
};

struct VsOut {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) view_z: f32,
    @location(1) @interpolate(flat) group: u32,
};

@vertex
fn vs_main(
    @builtin(vertex_index) vid: u32,
    instance: LineInstance,
) -> VsOut {
    var out: VsOut;
    let p0 = instance.p0_pad.xyz;
    let p1 = instance.p1_pad.xyz;
    let mid = 0.5 * (p0 + p1);

    var world: vec3<f32>;
    var group: u32;
    let id = vid % 4u;
    if id == 0u {
        world = p0; group = instance.groups.x;
    } else if id == 1u {
        world = mid; group = instance.groups.x;
    } else if id == 2u {
        world = mid; group = instance.groups.y;
    } else {
        world = p1; group = instance.groups.y;
    }
    let view_pos = (frame.view * vec4<f32>(scene_position(world), 1.0)).xyz;
    out.clip_position = frame.proj * vec4<f32>(view_pos, 1.0);
    out.view_z = -view_pos.z;
    out.group = group;
    if !scene_in_subset(obj.atom_offset + instance.groups.x) || !scene_in_subset(obj.atom_offset + instance.groups.y) {
        out.clip_position = vec4<f32>(2.0, 2.0, 2.0, 1.0);
    }
    return out;
}

@fragment
fn fs_main(input: VsOut) -> TranslucentOut {
    let global_id = obj.atom_offset + input.group;
    if !scene_visible(global_id) {
        discard;
    }
    let base = scene_line_color(global_id);
    return write_translucent(base.rgb, base.a, input.view_z, frame.clip.w);
}

@fragment
fn fs_opaque(input: VsOut) -> @location(0) vec4<f32> {
    let global_id = obj.atom_offset + input.group;
    if !scene_visible(global_id) {
        discard;
    }
    let base = scene_line_color(global_id);
    return vec4<f32>(base.rgb, 1.0);
}

@fragment
fn fs_depth(input: VsOut) {
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
