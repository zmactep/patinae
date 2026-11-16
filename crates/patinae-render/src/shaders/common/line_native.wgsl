// Native one-pixel color and overlay coverage. Hardware clips the segments
// and interpolates centerline depth; the quad path remains responsible for hits.
struct LineInstance {
    @location(0) p0_width: vec4<f32>,
    @location(1) p1_pad: vec4<f32>,
    @location(2) groups: vec2<u32>,
};
struct NativeLineVertex {
    clip_position: vec4<f32>,
    view_z: f32,
};
fn line_native_vertex(vid: u32, p0: vec3<f32>, p1: vec3<f32>) -> NativeLineVertex {
    var out: NativeLineVertex;
    out.clip_position = vec4<f32>(2.0, 2.0, 2.0, 1.0);
    let world = select(select(p0, p1, vid == 3u), 0.5 * (p0 + p1), vid == 1u || vid == 2u);
    let view = (frame.view * vec4<f32>(scene_position(world), 1.0)).xyz;
    out.clip_position = frame.proj * vec4<f32>(view, 1.0);
    out.view_z = -view.z;
    // Most lines need only the native vertex above. A view-space collinearity
    // test reserves projection/clipping work for the otherwise invisible
    // end-on case. Relative tolerance handles rotated perspective cameras.
    let delta = (frame.view * vec4<f32>(scene_direction(p1 - p0), 0.0)).xyz;
    let perspective = frame.proj[2][3] != 0.0;
    var cross_xy = delta.xy;
    var scale = max(dot(delta, delta), 1.0);
    if perspective {
        cross_xy = delta.xy * view.z - view.xy * delta.z;
        scale = max(dot(delta, delta) * dot(view.xyz, view.xyz), 1.0);
    }
    if dot(cross_xy, cross_xy) > 1e-12 * scale { return out; }

    let atom = scene_position(select(p0, p1, vid >= 2u));
    let mid = scene_position(0.5 * (p0 + p1));
    let a = frame.proj * (frame.view * vec4<f32>(atom, 1.0));
    let b = frame.proj * (frame.view * vec4<f32>(mid, 1.0));
    if (a.z < 0.0 && b.z < 0.0) {
        out.clip_position = vec4<f32>(2.0, 2.0, 2.0, 1.0);
        return out;
    }
    let an = line_clip_to_plane(a, b, a.z, b.z);
    let bn = line_clip_to_plane(b, a, b.z, a.z);
    let wa = an.w - LINE_MIN_CLIP_W;
    let wb = bn.w - LINE_MIN_CLIP_W;
    if wa < 0.0 && wb < 0.0 {
        out.clip_position = vec4<f32>(2.0, 2.0, 2.0, 1.0);
        return out;
    }
    let ac = line_clip_to_plane(an, bn, wa, wb);
    let bc = line_clip_to_plane(bn, an, wb, wa);
    let endpoint = select(ac, bc, bc.z / bc.w < ac.z / ac.w);
    let viewport = max(frame.viewport.xy, vec2<f32>(1.0));
    let pixel = floor((endpoint.xy / endpoint.w + 1.0) * viewport * 0.5) + 0.5;
    let offset = vec2<f32>(select(-0.5, 0.5, (vid % 2u) == 1u), 0.0);
    out.clip_position = vec4<f32>(((pixel + offset) * 2.0 / viewport - 1.0) * endpoint.w, endpoint.z, endpoint.w);
    let endpoint_view = frame.proj_inv * endpoint;
    out.view_z = -endpoint_view.z / endpoint_view.w;
    return out;
}
