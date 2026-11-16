struct ConvertParams {
    instance_capacity: u32,
    cylinder_offset: u32,
    atom_offset: u32,
    rep_slot: u32,
    transparency: f32,
    radius: f32,
    dispatch_width: u32,
    _pad0: u32,
}

struct LineInstance {
    p0_width: vec4<f32>,
    p1_pad: vec4<f32>,
    groups: vec2<u32>,
    _pad: vec2<u32>,
}

struct Cylinder {
    start: vec3<f32>,
    radius: f32,
    end: vec3<f32>,
    _pad0: f32,
    color1: vec4<f32>,
    color2: vec4<f32>,
    transparency: f32,
    _pad_a: f32, _pad_b: f32, _pad_c: f32,
}

// {{INCLUDE_ARTIFACT_COLOR}}

@group(0) @binding(0) var<storage, read> instances: array<LineInstance>;
@group(0) @binding(1) var<storage, read> color_lut: array<ColorLutEntry>;
@group(0) @binding(2) var<storage, read> draw_args: array<u32>;
@group(0) @binding(3) var<storage, read_write> cylinders: array<Cylinder>;
@group(0) @binding(4) var<uniform> params: ConvertParams;

fn material_for_group(group_id: u32) -> vec4<f32> {
    let entry = color_lut[params.atom_offset + group_id];
    let packed = rep_packed_color(entry.reps, params.rep_slot);
    var rgba = select(unpack_rep_rgb8(packed), entry.base, packed == REP_COLOR_INHERIT);
    rgba.a = clamp(rgba.a * (1.0 - params.transparency), 0.0, 1.0);
    return rgba;
}

fn invalid_cylinder() -> Cylinder {
    return Cylinder(vec3<f32>(0.0), 0.0, vec3<f32>(0.0), 0.0, vec4<f32>(0.0), vec4<f32>(0.0), 1.0, 0.0, 0.0, 0.0);
}

@compute @workgroup_size(128)
fn build_cylinders(@builtin(global_invocation_id) gid: vec3<u32>) {
    let instance_index = gid.y * params.dispatch_width + gid.x;
    if instance_index >= params.instance_capacity {
        return;
    }
    let out_index = params.cylinder_offset + instance_index;
    if instance_index >= draw_args[1] {
        cylinders[out_index] = invalid_cylinder();
        return;
    }

    let inst = instances[instance_index];
    let color1 = material_for_group(inst.groups.x);
    let color2 = material_for_group(inst.groups.y);
    cylinders[out_index] = Cylinder(
        inst.p0_width.xyz,
        params.radius,
        inst.p1_pad.xyz,
        0.0,
        color1,
        color2,
        1.0 - min(color1.a, color2.a),
        0.0, 0.0, 0.0,
    );
}
