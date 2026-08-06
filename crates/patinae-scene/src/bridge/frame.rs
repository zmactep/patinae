//! `FrameUniforms` builder from the host's `Session`. Mirrors the camera
//! / clip / light / fog math the live render path needs.

use patinae_render::FrameUniforms;
use patinae_settings::{Settings, ShadingMode};

use crate::camera::{Camera, Projection};
use crate::session::Session;

const PROJECTION_INVERSE_EPSILON: f32 = 1.0e-8;

/// Build `patinae_render::FrameUniforms` from the host's `Session`.
/// `clear_color` is used as the fog tint when depth-cue / fog are on.
pub fn frame_uniforms_from_session(
    session: &Session,
    viewport_size: (u32, u32),
    clear_color: [f32; 3],
) -> FrameUniforms {
    frame_uniforms_from_camera(
        &session.camera,
        &session.settings,
        viewport_size,
        clear_color,
    )
}

/// Build `patinae_render::FrameUniforms` from an explicit camera + settings pair.
/// This is used by capture paths that render a different target size without
/// mutating the public camera projection state.
pub fn frame_uniforms_from_camera(
    camera: &Camera,
    settings: &Settings,
    viewport_size: (u32, u32),
    clear_color: [f32; 3],
) -> FrameUniforms {
    let mut u = FrameUniforms::default();
    let view = camera.view_matrix();
    let aspect = (viewport_size.0.max(1) as f32) / (viewport_size.1.max(1) as f32);
    let scene_view = camera.current_view();
    let projection = if settings.ui.orthoscopic {
        Projection::Orthographic
    } else {
        Projection::Perspective
    };
    let proj = scene_view.projection_matrix(aspect, projection);
    // `Mat4` is move-only and `Mul` consumes operands; capture column data
    // before forming view_proj / view_inv.
    let view_cols = mat4_to_cols(&view);
    let proj_cols = mat4_to_cols(&proj);
    let view_inv_cols = view
        .inverse()
        .map(|m| mat4_to_cols(&m))
        .unwrap_or(view_cols);
    // `lin_alg::Mat4::inverse` does not preserve the projective `w` row for
    // this column-major perspective matrix. Use the closed-form inverse of
    // the projection shapes produced by `SceneView::projection_matrix` so
    // depth unprojection remains finite in both camera modes.
    let proj_inv_cols = projection_inverse_cols(&proj, projection)
        .unwrap_or_else(|| FrameUniforms::default().proj_inv);
    let view_proj = proj.clone() * view.clone();
    u.view = view_cols;
    u.proj = proj_cols;
    u.view_proj = mat4_to_cols(&view_proj);
    u.view_inv = view_inv_cols;
    u.proj_inv = proj_inv_cols;
    u.set_viewport(viewport_size.0, viewport_size.1);

    // Lights live in view space and stay attached to the camera.
    // Direction vectors point _toward_ the light; the lighting shader
    // negates to get the from-fragment-to-light direction. We just
    // normalise the host-supplied vectors here.
    let classic = &settings.shading.classic;
    upload_light_directions(
        &mut u,
        [
            classic.light,
            classic.light2,
            classic.light3,
            classic.light4,
            classic.light5,
            classic.light6,
            classic.light7,
            classic.light8,
            classic.light9,
        ],
    );
    apply_shading_settings(settings, &mut u);

    u.clip[0] = scene_view.clip_front;
    u.clip[1] = scene_view.clip_back;
    u.clip[2] = patinae_render::frame::PICKING_SCALE;
    u.set_scene_max_depth(scene_view.clip_back);

    let common = &settings.shading.common;
    if common.depth_cue && common.fog > 0.0 {
        let fog_start_actual = (scene_view.clip_back - scene_view.clip_front) * common.fog_start
            + scene_view.clip_front;
        let fog_end_actual = if (common.fog - 1.0).abs() < 0.001 {
            scene_view.clip_back
        } else {
            fog_start_actual + (scene_view.clip_back - fog_start_actual) / common.fog
        };
        u.fog = [fog_start_actual, fog_end_actual, common.fog, 0.0];
        u.fog_color = [clear_color[0], clear_color[1], clear_color[2], 0.0];
    }

    u
}

fn upload_light_directions(u: &mut FrameUniforms, lights: [[f32; 3]; 9]) {
    for (i, dir) in lights.iter().enumerate() {
        let len = (dir[0] * dir[0] + dir[1] * dir[1] + dir[2] * dir[2])
            .sqrt()
            .max(1e-6);
        u.light_dirs[i] = [dir[0] / len, dir[1] / len, dir[2] / len, 0.0];
    }
}

fn apply_shading_settings(settings: &Settings, u: &mut FrameUniforms) {
    let depth_cue = if settings.shading.common.depth_cue {
        1.0
    } else {
        0.0
    };

    match settings.shading.mode {
        ShadingMode::Classic | ShadingMode::Full => {
            let classic = &settings.shading.classic;
            u.light_intensity = [
                classic.ambient,
                classic.direct,
                classic.reflect,
                classic.specular,
            ];
            u.light_spec = [
                classic.shininess,
                classic.spec_direct,
                classic.spec_direct_power,
                depth_cue,
            ];
            u.light_counts = [
                classic.light_count as f32,
                classic.spec_count as f32,
                0.0,
                0.0,
            ];
        }
        ShadingMode::Skripkin => {
            let skripkin = &settings.shading.skripkin;
            u.light_intensity = [skripkin.ambient, 0.0, 0.0, 0.0];
            u.light_spec = [
                skripkin.shininess,
                skripkin.specular,
                skripkin.shininess,
                depth_cue,
            ];
            u.light_counts = [1.0, 0.0, 0.0, 0.0];
        }
    }
}

/// `lin_alg::Mat4` stores its 16 floats column-major in `.data`.
/// `FrameUniforms.view` is `[[f32; 4]; 4]` indexed by `[col][row]` —
/// `view[col][row] = data[col*4 + row]`.
fn mat4_to_cols(m: &lin_alg::f32::Mat4) -> [[f32; 4]; 4] {
    let d = m.data;
    [
        [d[0], d[1], d[2], d[3]],
        [d[4], d[5], d[6], d[7]],
        [d[8], d[9], d[10], d[11]],
        [d[12], d[13], d[14], d[15]],
    ]
}

fn projection_inverse_cols(
    projection: &lin_alg::f32::Mat4,
    kind: Projection,
) -> Option<[[f32; 4]; 4]> {
    let d = projection.data;
    match kind {
        Projection::Perspective => {
            let scale_x = d[0];
            let scale_y = d[5];
            let depth_scale = d[10];
            let depth_offset = d[14];
            if scale_x.abs() <= PROJECTION_INVERSE_EPSILON
                || scale_y.abs() <= PROJECTION_INVERSE_EPSILON
                || depth_offset.abs() <= PROJECTION_INVERSE_EPSILON
            {
                return None;
            }
            Some([
                [scale_x.recip(), 0.0, 0.0, 0.0],
                [0.0, scale_y.recip(), 0.0, 0.0],
                [0.0, 0.0, 0.0, depth_offset.recip()],
                [0.0, 0.0, -1.0, depth_scale / depth_offset],
            ])
        }
        Projection::Orthographic => {
            let scale_x = d[0];
            let scale_y = d[5];
            let scale_z = d[10];
            if scale_x.abs() <= PROJECTION_INVERSE_EPSILON
                || scale_y.abs() <= PROJECTION_INVERSE_EPSILON
                || scale_z.abs() <= PROJECTION_INVERSE_EPSILON
            {
                return None;
            }
            Some([
                [scale_x.recip(), 0.0, 0.0, 0.0],
                [0.0, scale_y.recip(), 0.0, 0.0],
                [0.0, 0.0, scale_z.recip(), 0.0],
                [-d[12] / scale_x, -d[13] / scale_y, -d[14] / scale_z, 1.0],
            ])
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unproject_center_depth(proj_inv: [[f32; 4]; 4], depth: f32) -> [f32; 3] {
        let homogeneous = [
            proj_inv[2][0] * depth + proj_inv[3][0],
            proj_inv[2][1] * depth + proj_inv[3][1],
            proj_inv[2][2] * depth + proj_inv[3][2],
            proj_inv[2][3] * depth + proj_inv[3][3],
        ];
        assert!(
            homogeneous[3].abs() > 1.0e-6,
            "projection inverse produced a point at infinity: {homogeneous:?}"
        );
        [
            homogeneous[0] / homogeneous[3],
            homogeneous[1] / homogeneous[3],
            homogeneous[2] / homogeneous[3],
        ]
    }

    #[test]
    fn skripkin_mode_writes_ambient_ao_lighting_uniforms() {
        let mut settings = Settings::default();
        settings.shading.mode = ShadingMode::Skripkin;
        settings.shading.skripkin.ambient = 0.31;
        settings.shading.skripkin.specular = 0.42;
        settings.shading.skripkin.shininess = 73.0;

        let mut uniforms = FrameUniforms::default();
        apply_shading_settings(&settings, &mut uniforms);

        assert_eq!(uniforms.light_intensity, [0.31, 0.0, 0.0, 0.0]);
        assert_eq!(uniforms.light_spec, [73.0, 0.42, 73.0, 1.0]);
        assert_eq!(uniforms.light_counts, [1.0, 0.0, 0.0, 0.0]);
    }

    #[test]
    fn skripkin_defaults_are_material_color_plus_ao() {
        let mut settings = Settings::default();
        settings.shading.mode = ShadingMode::Skripkin;

        let mut uniforms = FrameUniforms::default();
        apply_shading_settings(&settings, &mut uniforms);

        assert_eq!(uniforms.light_intensity, [1.0, 0.0, 0.0, 0.0]);
        assert_eq!(uniforms.light_spec, [55.0, 0.0, 55.0, 1.0]);
        assert_eq!(uniforms.light_counts, [1.0, 0.0, 0.0, 0.0]);
    }

    #[test]
    fn orthoscopic_setting_selects_orthographic_projection() {
        let camera = Camera::new();
        let mut settings = Settings::default();
        let perspective = frame_uniforms_from_camera(&camera, &settings, (512, 512), [0.0; 3]);

        settings.ui.orthoscopic = true;
        let ortho = frame_uniforms_from_camera(&camera, &settings, (512, 512), [0.0; 3]);
        let expected = camera
            .current_view()
            .projection_matrix(1.0, Projection::Orthographic);

        assert_ne!(perspective.proj, ortho.proj);
        assert_eq!(ortho.proj, mat4_to_cols(&expected));
    }

    #[test]
    fn projection_inverse_reconstructs_perspective_and_orthographic_clip_planes() {
        let camera = Camera::new();
        let view = camera.current_view();

        for orthoscopic in [false, true] {
            let mut settings = Settings::default();
            settings.ui.orthoscopic = orthoscopic;
            let uniforms = frame_uniforms_from_camera(&camera, &settings, (512, 512), [0.0; 3]);

            let near = unproject_center_depth(uniforms.proj_inv, 0.0);
            let far = unproject_center_depth(uniforms.proj_inv, 1.0);

            assert!(near[0].abs() < 1.0e-5);
            assert!(near[1].abs() < 1.0e-5);
            assert!((near[2] + view.clip_front).abs() < 1.0e-3);
            assert!((far[2] + view.clip_back).abs() < view.clip_back * 1.0e-3);
        }
    }
}
