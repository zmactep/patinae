use super::state::{RenderState, SceneBounds};
use crate::uniforms::FrameUniforms;

impl RenderState {
    pub(super) fn shadow_signature(&self) -> u64 {
        let mut h: u64 = 0xcbf29ce484222325;
        for (object_id, kind) in &self.scene.draw_order {
            let casts_shadow = self
                .scene
                .reps
                .get(&(*object_id, *kind))
                .is_some_and(|entry| entry.rep.casts_shadow());
            h ^= *object_id as u64;
            h = h.wrapping_mul(0x100000001b3);
            h ^= kind.as_raw() as u64;
            h = h.wrapping_mul(0x100000001b3);
            h ^= casts_shadow as u64;
            h = h.wrapping_mul(0x100000001b3);
        }
        h
    }
}

pub(super) fn build_shadow_frame_for_direction(
    frame: &FrameUniforms,
    bounds: SceneBounds,
    direction: [f32; 3],
    shadow_size: u32,
) -> FrameUniforms {
    let light_world = normalize3(direction).unwrap_or([0.4, 0.4, 1.0]);
    let distance = bounds.radius * 2.5;
    let eye = add3(bounds.center, mul3(light_world, distance));
    let up_hint = if light_world[1].abs() > 0.9 {
        [1.0, 0.0, 0.0]
    } else {
        [0.0, 1.0, 0.0]
    };
    let view = look_at_rh(eye, bounds.center, up_hint);
    let half = bounds.radius * 1.15;
    let near = (distance - bounds.radius * 1.25).max(0.05);
    let far = distance + bounds.radius * 1.25;
    let proj = ortho_rh_zo(half, near, far);
    let view_proj = mat4_mul(&proj, &view);

    let mut shadow_frame = FrameUniforms {
        view,
        proj,
        // Analytic impostors reconstruct rays in the shadow camera too.
        proj_inv: [
            [1.0 / proj[0][0], 0.0, 0.0, 0.0],
            [0.0, 1.0 / proj[1][1], 0.0, 0.0],
            [0.0, 0.0, 1.0 / proj[2][2], 0.0],
            [0.0, 0.0, -proj[3][2] / proj[2][2], 1.0],
        ],
        view_proj,
        clip: [near, far, frame.clip[2], far],
        ..FrameUniforms::default()
    };
    shadow_frame.set_viewport(shadow_size, shadow_size);
    shadow_frame
}

pub(super) fn fibonacci_sphere_direction(index: u32, count: u32) -> [f32; 3] {
    let n = count.max(1);
    let t = if n > 1 {
        index as f32 / (n - 1) as f32
    } else {
        0.5
    };
    let inclination = (1.0 - 2.0 * t).acos();
    let golden_ratio = (1.0 + 5.0_f32.sqrt()) * 0.5;
    let azimuth = std::f32::consts::TAU * index as f32 / golden_ratio;
    [
        inclination.sin() * azimuth.cos(),
        inclination.sin() * azimuth.sin(),
        inclination.cos(),
    ]
}

fn look_at_rh(eye: [f32; 3], target: [f32; 3], up_hint: [f32; 3]) -> [[f32; 4]; 4] {
    let f = normalize3(sub3(target, eye)).unwrap_or([0.0, 0.0, -1.0]);
    let s = normalize3(cross3(f, up_hint)).unwrap_or([1.0, 0.0, 0.0]);
    let u = cross3(s, f);
    [
        [s[0], u[0], -f[0], 0.0],
        [s[1], u[1], -f[1], 0.0],
        [s[2], u[2], -f[2], 0.0],
        [-dot3(s, eye), -dot3(u, eye), dot3(f, eye), 1.0],
    ]
}

fn ortho_rh_zo(half: f32, near: f32, far: f32) -> [[f32; 4]; 4] {
    let h = half.max(0.01);
    let depth = (far - near).max(0.01);
    [
        [1.0 / h, 0.0, 0.0, 0.0],
        [0.0, 1.0 / h, 0.0, 0.0],
        [0.0, 0.0, -1.0 / depth, 0.0],
        [0.0, 0.0, -near / depth, 1.0],
    ]
}

fn mat4_mul(a: &[[f32; 4]; 4], b: &[[f32; 4]; 4]) -> [[f32; 4]; 4] {
    let mut out = [[0.0; 4]; 4];
    for col in 0..4 {
        for row in 0..4 {
            out[col][row] = a[0][row] * b[col][0]
                + a[1][row] * b[col][1]
                + a[2][row] * b[col][2]
                + a[3][row] * b[col][3];
        }
    }
    out
}

pub(super) fn transform_dir(m: &[[f32; 4]; 4], v: [f32; 3]) -> [f32; 3] {
    [
        m[0][0] * v[0] + m[1][0] * v[1] + m[2][0] * v[2],
        m[0][1] * v[0] + m[1][1] * v[1] + m[2][1] * v[2],
        m[0][2] * v[0] + m[1][2] * v[1] + m[2][2] * v[2],
    ]
}

pub(super) fn normalize3(v: [f32; 3]) -> Option<[f32; 3]> {
    let len = dot3(v, v).sqrt();
    (len > 1e-6).then(|| [v[0] / len, v[1] / len, v[2] / len])
}

fn add3(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

fn sub3(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn mul3(v: [f32; 3], s: f32) -> [f32; 3] {
    [v[0] * s, v[1] * s, v[2] * s]
}

fn dot3(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross3(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

pub(super) fn hash_object_ids(ids: impl Iterator<Item = u32>) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for id in ids {
        h ^= id as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

pub(super) fn hash_view_proj(m: &[[f32; 4]; 4]) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for row in m {
        for c in row {
            h ^= c.to_bits() as u64;
            h = h.wrapping_mul(0x100000001b3);
        }
    }
    h
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shadow_projection_inverse_reconstructs_impostor_rays() {
        for radius in [0.001, 1.0, 1000.0] {
            let frame = build_shadow_frame_for_direction(
                &FrameUniforms::default(),
                SceneBounds {
                    center: [3.0, -1.0, 2.0],
                    radius,
                },
                [0.0, 0.0, 1.0],
                512,
            );
            let identity = mat4_mul(&frame.proj, &frame.proj_inv);
            for (col, values) in identity.iter().enumerate() {
                for (row, value) in values.iter().enumerate() {
                    assert!((value - if row == col { 1.0 } else { 0.0 }).abs() < 1e-5);
                }
            }
        }
    }
}
