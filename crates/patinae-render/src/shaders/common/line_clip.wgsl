// Shared near-plane clipping for native lines and wide strips.
const LINE_DEGENERATE_PX: f32 = 1e-4;
const LINE_MIN_CLIP_W: f32 = 1e-4;
// Moves `a` toward `b` onto the plane f = 0, given the plane function's
// values at both points (f >= 0 is kept). Callers ensure `fa < 0 <= fb`.
fn line_clip_to_plane(a: vec4<f32>, b: vec4<f32>, fa: f32, fb: f32) -> vec4<f32> {
    if fa >= 0.0 {
        return a;
    }
    return mix(a, b, fa / (fa - fb));
}

