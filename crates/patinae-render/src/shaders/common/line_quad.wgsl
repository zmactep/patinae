// Eight-vertex strip: first cap, body, last cap. Endpoint pairs meet at
// their actual depths. Perspective-interpolated group_t splits ownership at
// the world-space midpoint without duplicating projection for two halves.
// Topology and footprint constants are injected by shader_source::expand.
// Bonds are clipped to the near plane (clip z >= 0, which the hardware
// clips anyway) before the perspective divide, so projected pixel
// coordinates stay bounded; endpoints behind the eye are covered by this in
// perspective. The w floor is a backstop for degenerate projections.

struct LineQuadVertex {
    clip_position: vec4<f32>,
    // x: along the bond from the first endpoint, in half-widths.
    // y: across the line, in half-widths ([-1, 1]).
    local: vec2<f32>,
    // Projected bond length, in half-widths.
    span: f32,
    view_z: f32,
    group_t: f32,
    depths: vec2<f32>,
    ownership: vec4<f32>, // clipped endpoint t values and homogeneous w
};

// Shared vertex ABI; host layout is representations::line::LineInstance.
struct LineInstance {
    @location(0) p0_width: vec4<f32>, // xyz = first endpoint, w = physical pixel width
    @location(1) p1_pad: vec4<f32>,
    @location(2) groups: vec2<u32>, // object-local endpoint atom indices
};

// The caller resolves SceneStore transforms; this helper owns projection and
// strip topology equally for color, depth, overlay, and hit-test passes.
fn line_project(
    vid: u32, p0: vec3<f32>, p1: vec3<f32>, half_width: f32,
) -> LineQuadVertex {
    return line_quad_vertex(
        frame.proj * (frame.view * vec4<f32>(p0, 1.0)),
        frame.proj * (frame.view * vec4<f32>(p1, 1.0)),
        vec2<f32>(f32(vid / 2u) - 1.0, select(-1.0, 1.0, (vid % 2u) == 1u)),
        half_width,
    );
}

fn line_quad_vertex(
    start_clip_in: vec4<f32>,
    end_clip_in: vec4<f32>,
    corner: vec2<f32>,
    half_width_px: f32,
) -> LineQuadVertex {
    var out: LineQuadVertex;
    out.clip_position = vec4<f32>(2.0, 2.0, 2.0, 1.0);
    out.local = vec2<f32>(2.0);
    out.span = 0.0;
    out.view_z = 0.0;

    // Near plane: z >= 0.
    let za = start_clip_in.z;
    let ze = end_clip_in.z;
    if za < 0.0 && ze < 0.0 {
        return out;
    }
    // The common fully-visible path performs no clip-fraction divisions.
    var start_clip = start_clip_in;
    var end_clip = end_clip_in;
    var t0 = 0.0;
    var t1 = 1.0;
    if za < 0.0 {
        t0 = za / (za - ze);
        start_clip = mix(start_clip_in, end_clip_in, t0);
    } else if ze < 0.0 {
        let fraction = ze / (ze - za);
        t1 = 1.0 - fraction;
        end_clip = mix(end_clip_in, start_clip_in, fraction);
    }
    // Backstop: w >= LINE_MIN_CLIP_W.
    let wa = start_clip.w - LINE_MIN_CLIP_W;
    let we = end_clip.w - LINE_MIN_CLIP_W;
    if wa < 0.0 && we < 0.0 { return out; }
    if wa < 0.0 {
        let fraction = wa / (wa - we);
        start_clip = mix(start_clip, end_clip, fraction);
        t0 = mix(t0, t1, fraction);
    } else if we < 0.0 {
        let fraction = we / (we - wa);
        end_clip = mix(end_clip, start_clip, fraction);
        t1 = mix(t1, t0, fraction);
    }

    let viewport = max(frame.viewport.xy, vec2<f32>(1.0));
    let start_px = start_clip.xy / start_clip.w * viewport * 0.5;
    let end_px = end_clip.xy / end_clip.w * viewport * 0.5;
    let delta = end_px - start_px;
    let pixel_length = length(delta);
    let degenerate = pixel_length <= LINE_DEGENERATE_PX;
    let dir = select(delta / max(pixel_length, LINE_DEGENERATE_PX), vec2<f32>(1.0, 0.0), degenerate);
    let normal = vec2<f32>(-dir.y, dir.x);
    // Never thinner than one pixel, otherwise the quad leaves raster gaps.
    let half_width = max(half_width_px, LINE_VISUAL_MIN_HALF_WIDTH_PX);
    let span = pixel_length / half_width;

    // Cap and body share vertices at the actual atom center, so extending
    // the footprint never stretches centerline depth interpolation.
    let at_end = corner.x > 0.5;
    let along_hw = select(min(corner.x, 0.0), max(corner.x - 1.0, 0.0), at_end);
    var endpoint = select(start_clip, end_clip, at_end);
    var group_t = select(t0, t1, at_end);
    var depths = vec2<f32>(start_clip.z / start_clip.w, end_clip.z / end_clip.w);
    var ownership = vec4<f32>(t0, t1, start_clip.w, end_clip.w);
    if degenerate {
        // An end-on bond is a disc at its nearest surviving endpoint,
        // rather than a depth ramp across an arbitrary screen direction.
        let nearest_end = end_clip.z / end_clip.w < start_clip.z / start_clip.w;
        endpoint = select(start_clip, end_clip, nearest_end);
        group_t = select(t0, t1, nearest_end);
        depths = vec2<f32>(endpoint.z / endpoint.w);
        ownership = vec4<f32>(group_t, group_t, endpoint.w, endpoint.w);
    }
    let offset_px = (dir * along_hw + normal * corner.y) * half_width;
    let offset_ndc = offset_px * 2.0 / viewport;

    out.clip_position = vec4<f32>(
        endpoint.xy + offset_ndc * endpoint.w,
        endpoint.z,
        endpoint.w,
    );
    out.local = vec2<f32>(select(min(corner.x, 0.0), span + max(corner.x - 1.0, 0.0), at_end), corner.y);
    out.span = span;
    out.group_t = group_t;
    out.depths = depths;
    out.ownership = ownership;
    let view_position = frame.proj_inv * endpoint;
    out.view_z = -view_position.z / view_position.w;
    return out;
}

// True when a fragment falls outside the round-capped bond footprint.
fn line_quad_outside(local: vec2<f32>, span: f32) -> bool {
    let d = local - vec2<f32>(clamp(local.x, 0.0, span), 0.0);
    return dot(d, d) > 1.0;
}

// A masked endpoint leaves a half-bond with its own midpoint cap. Project
// that half before clipping so coverage and constant cap depth are preserved.
fn line_project_visible(vid: u32, p0: vec3<f32>, p1: vec3<f32>, groups: vec2<u32>, half_width: f32) -> LineQuadVertex {
    let first = scene_visible(obj.atom_offset + groups.x);
    let second = scene_visible(obj.atom_offset + groups.y);
    if first == second { return line_project(vid, p0, p1, half_width); }
    let mid = 0.5 * (p0 + p1);
    var visible = line_project(vid, select(mid, p0, first), select(mid, p1, second), half_width);
    visible.group_t = select(0.0, 1.0, second);
    visible.ownership.x = visible.group_t;
    visible.ownership.y = visible.group_t;
    return visible;
}
