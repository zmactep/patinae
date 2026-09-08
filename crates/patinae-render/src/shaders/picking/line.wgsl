// Line picking — same 4-vertex layout as line.wgsl. Flat-interp `group` on
// the provoking vertex of each half-segment encodes the endpoint atom_id.

// {{INCLUDE_FRAME}}
// {{INCLUDE_PICKING}}
// {{INCLUDE_PICKING_SCENE}}

@group(2) @binding(0) var<uniform> picking: PickingParams;

struct LineInstance {
    @location(0) p0_pad: vec4<f32>,
    @location(1) p1_pad: vec4<f32>,
    @location(2) groups: vec2<u32>,
};

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
    out.group = group;
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
