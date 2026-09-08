// Dot picking — procedural screen-space quads per atom, atom_id = owner atom.

// {{INCLUDE_FRAME}}
// {{INCLUDE_PICKING}}
// {{INCLUDE_DOT_PICKING_SCENE}}

@group(2) @binding(0) var<uniform> picking: PickingParams;

struct DotAtomInstance {
    @location(0) center: vec3<f32>,
    @location(1) group_id: u32,
    @location(2) vdw_radius: f32,
};

struct DotDrawParams {
    samples_per_atom: u32,
    dir_offset: u32,
    radius_px: f32,
    _pad0: u32,
};

@group(3) @binding(0) var<uniform> dot_params: DotDrawParams;
@group(3) @binding(1) var<uniform> dot_dirs: array<vec4<f32>, 504>;

struct VsOut {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) @interpolate(flat) group: u32,
    @location(1) uv: vec2<f32>,
};

fn quad_corner(vertex_index: u32) -> vec2<f32> {
    let v = vertex_index % 6u;
    if v == 0u { return vec2<f32>(-1.0, -1.0); }
    if v == 1u { return vec2<f32>( 1.0, -1.0); }
    if v == 2u { return vec2<f32>( 1.0,  1.0); }
    if v == 3u { return vec2<f32>(-1.0, -1.0); }
    if v == 4u { return vec2<f32>( 1.0,  1.0); }
    return vec2<f32>(-1.0, 1.0);
}

@vertex
fn vs_main(instance: DotAtomInstance, @builtin(vertex_index) vertex_index: u32) -> VsOut {
    var out: VsOut;
    let sample = vertex_index / 6u;
    let dir = dot_dirs[dot_params.dir_offset + sample].xyz;
    let world_pos = scene_position(instance.center) + dir * instance.vdw_radius;
    let view_pos = (frame.view * vec4<f32>(world_pos, 1.0)).xyz;
    var clip = frame.proj * vec4<f32>(view_pos, 1.0);
    let corner = quad_corner(vertex_index);
    let pixel_to_clip = vec2<f32>(frame.viewport.z * 2.0, frame.viewport.w * 2.0);
    let offset = corner * dot_params.radius_px * pixel_to_clip * clip.w;
    clip = vec4<f32>(clip.x + offset.x, clip.y + offset.y, clip.z, clip.w);
    out.clip_position = clip;
    out.group = instance.group_id;
    out.uv = corner;
    return out;
}

@fragment
fn fs_main(input: VsOut) -> @location(0) vec2<u32> {
    if !scene_visible(obj.atom_offset + input.group) { discard; }
    if dot(input.uv, input.uv) > 1.0 {
        discard;
    }
    return pack_id(picking.rep_object, input.group);
}
