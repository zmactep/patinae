// Screen-space annotation strokes. The CPU supplies patterned world-space
// segments with fully resolved per-instance presentation.

// {{INCLUDE_FRAME}}

struct SegmentInstance {
    @location(0) start: vec3<f32>,
    @location(1) width_px: f32,
    @location(2) end: vec3<f32>,
    @location(3) round_ends: u32,
    @location(4) color: vec4<f32>,
};

struct VsOut {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) local_position: vec2<f32>,
    @location(1) @interpolate(flat) length_in_radii: f32,
    @location(2) @interpolate(flat) round_ends: u32,
    @location(3) @interpolate(flat) color: vec4<f32>,
};

fn endpoint_and_side(vertex_index: u32) -> vec2<f32> {
    let v = vertex_index % 6u;
    if v == 0u { return vec2<f32>(0.0, -1.0); }
    if v == 1u { return vec2<f32>(1.0, -1.0); }
    if v == 2u { return vec2<f32>(1.0,  1.0); }
    if v == 3u { return vec2<f32>(0.0, -1.0); }
    if v == 4u { return vec2<f32>(1.0,  1.0); }
    return vec2<f32>(0.0, 1.0);
}

@vertex
fn vs_main(
    segment: SegmentInstance,
    @builtin(vertex_index) vertex_index: u32,
) -> VsOut {
    let start_clip = frame.view_proj * vec4<f32>(segment.start, 1.0);
    let end_clip = frame.view_proj * vec4<f32>(segment.end, 1.0);
    let start_ndc = start_clip.xy / start_clip.w;
    let end_ndc = end_clip.xy / end_clip.w;
    let viewport = max(frame.viewport.xy, vec2<f32>(1.0));
    let pixel_delta = (end_ndc - start_ndc) * viewport * 0.5;
    let pixel_length = length(pixel_delta);

    var out: VsOut;
    if start_clip.w <= 0.0 || end_clip.w <= 0.0 || pixel_length <= 0.0001 {
        out.clip_position = vec4<f32>(2.0, 2.0, 1.0, 1.0);
        out.local_position = vec2<f32>(2.0);
        out.length_in_radii = 0.0;
        out.round_ends = segment.round_ends;
        out.color = segment.color;
        return out;
    }

    let selector = endpoint_and_side(vertex_index);
    let endpoint_clip = mix(start_clip, end_clip, selector.x);
    let direction_px = pixel_delta / pixel_length;
    let normal_px = vec2<f32>(-direction_px.y, direction_px.x);
    let half_width = max(segment.width_px * 0.5, 0.5);
    let has_round_ends = segment.round_ends != 0u;
    let cap_direction = select(0.0, selector.x * 2.0 - 1.0, has_round_ends);
    let offset_px =
        (normal_px * selector.y + direction_px * cap_direction) * half_width;
    let offset_ndc = offset_px * 2.0 / viewport;

    out.clip_position = vec4<f32>(
        endpoint_clip.x + offset_ndc.x * endpoint_clip.w,
        endpoint_clip.y + offset_ndc.y * endpoint_clip.w,
        endpoint_clip.z,
        endpoint_clip.w,
    );
    let length_in_radii = pixel_length / half_width;
    let cap_offset = select(0.0, 1.0, has_round_ends);
    out.local_position = vec2<f32>(
        mix(-cap_offset, length_in_radii + cap_offset, selector.x),
        selector.y,
    );
    out.length_in_radii = length_in_radii;
    out.round_ends = segment.round_ends;
    out.color = segment.color;
    return out;
}

@fragment
fn fs_main(input: VsOut) -> @location(0) vec4<f32> {
    var distance = abs(input.local_position.y);
    if input.round_ends != 0u {
        let closest_x = clamp(input.local_position.x, 0.0, input.length_in_radii);
        distance = length(input.local_position - vec2<f32>(closest_x, 0.0));
    }
    let coverage = 1.0 - smoothstep(0.82, 1.0, distance);
    return vec4<f32>(input.color.rgb, input.color.a * coverage);
}
