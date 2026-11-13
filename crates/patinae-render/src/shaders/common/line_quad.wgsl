// Screen-space line quads shared by the line colour, depth/shadow, and
// picking passes. Requires INCLUDE_FRAME.
//
// WebGPU rasterises `LineList` primitives at a fixed 1 px, so each bond is
// expanded into two half-bond quads (6 vertices each, 12 per instance). A
// quad starts at its atom end with a round cap of radius `half_width` and
// stops flat at the bond midpoint, so each half keeps its endpoint colour
// and the halves abut without overlapping. Widths are physical pixels.

const LINE_VERTS_PER_HALF: u32 = 6u;
// Half-bonds are clipped to the near plane (clip z >= 0, which the hardware
// clips anyway) before the perspective divide, so projected pixel
// coordinates stay bounded; endpoints behind the eye are covered by this in
// perspective. The w floor is a backstop for degenerate projections.
const LINE_MIN_CLIP_W: f32 = 1e-4;
const LINE_DEGENERATE_PX: f32 = 1e-4;

struct LineQuadVertex {
    clip_position: vec4<f32>,
    // x: along the half-bond from the atom end, in half-widths.
    // y: across the line, in half-widths ([-1, 1]).
    local: vec2<f32>,
    // Projected half-bond length, in half-widths.
    span: f32,
};

// 0 → half-bond owned by atom 1 (`p0`), 1 → half-bond owned by atom 2 (`p1`).
fn line_quad_half(vid: u32) -> u32 {
    return (vid / LINE_VERTS_PER_HALF) % 2u;
}

// (along, side): along 0 = atom end, 1 = midpoint end; side = ±1.
fn line_quad_corner(vid: u32) -> vec2<f32> {
    let v = vid % LINE_VERTS_PER_HALF;
    let along = select(0.0, 1.0, v == 1u || v == 2u || v == 4u);
    let side = select(-1.0, 1.0, v == 2u || v == 4u || v == 5u);
    return vec2<f32>(along, side);
}

// Moves `a` toward `b` onto the plane f = 0, given the plane function's
// values at both points (f >= 0 is kept). Callers ensure `fa < 0 <= fb`.
fn line_clip_to_plane(a: vec4<f32>, b: vec4<f32>, fa: f32, fb: f32) -> vec4<f32> {
    if fa >= 0.0 {
        return a;
    }
    return mix(a, b, fa / (fa - fb));
}

fn line_quad_vertex(
    atom_clip_in: vec4<f32>,
    mid_clip_in: vec4<f32>,
    corner: vec2<f32>,
    half_width_px: f32,
) -> LineQuadVertex {
    var out: LineQuadVertex;
    out.clip_position = vec4<f32>(2.0, 2.0, 2.0, 1.0);
    out.local = vec2<f32>(2.0);
    out.span = 0.0;

    // Near plane: z >= 0.
    let za = atom_clip_in.z;
    let zm = mid_clip_in.z;
    if za < 0.0 && zm < 0.0 {
        return out;
    }
    let atom_near = line_clip_to_plane(atom_clip_in, mid_clip_in, za, zm);
    let mid_near = line_clip_to_plane(mid_clip_in, atom_clip_in, zm, za);
    // Backstop: w >= LINE_MIN_CLIP_W.
    let wa = atom_near.w - LINE_MIN_CLIP_W;
    let wm = mid_near.w - LINE_MIN_CLIP_W;
    if wa < 0.0 && wm < 0.0 {
        return out;
    }
    let atom_clip = line_clip_to_plane(atom_near, mid_near, wa, wm);
    let mid_clip = line_clip_to_plane(mid_near, atom_near, wm, wa);

    let viewport = max(frame.viewport.xy, vec2<f32>(1.0));
    let atom_px = atom_clip.xy / atom_clip.w * viewport * 0.5;
    let mid_px = mid_clip.xy / mid_clip.w * viewport * 0.5;
    let delta = mid_px - atom_px;
    let pixel_length = length(delta);
    let degenerate = pixel_length <= LINE_DEGENERATE_PX;
    let dir = select(delta / max(pixel_length, LINE_DEGENERATE_PX), vec2<f32>(1.0, 0.0), degenerate);
    let normal = vec2<f32>(-dir.y, dir.x);
    // Never thinner than one pixel, otherwise the quad leaves raster gaps.
    let half_width = max(half_width_px, 0.5);
    let span = pixel_length / half_width;

    // Atom end: extend backwards by one half-width for the round cap.
    // Midpoint end: flat, except for a bond seen end-on, which renders as a
    // full disc around the atom.
    let at_mid = corner.x > 0.5;
    let far_ext = select(0.0, 1.0, degenerate);
    let along_hw = select(-1.0, far_ext, at_mid);
    let endpoint = select(atom_clip, mid_clip, at_mid);
    let offset_px = (dir * along_hw + normal * corner.y) * half_width;
    let offset_ndc = offset_px * 2.0 / viewport;

    out.clip_position = vec4<f32>(
        endpoint.xy + offset_ndc * endpoint.w,
        endpoint.z,
        endpoint.w,
    );
    out.local = vec2<f32>(select(-1.0, span + far_ext, at_mid), corner.y);
    out.span = span;
    return out;
}

// True when a fragment falls outside the round-capped half-bond footprint.
fn line_quad_outside(local: vec2<f32>, span: f32) -> bool {
    let d = local - vec2<f32>(clamp(local.x, 0.0, span), 0.0);
    return dot(d, d) > 1.0;
}
