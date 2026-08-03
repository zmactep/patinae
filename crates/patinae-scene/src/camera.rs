//! Camera and view management
//!
//! This module provides the camera system, including:
//! - [`SceneView`]: 25-value view state (PSE format compatible)
//! - [`Camera`]: Interactive camera controller
//! - [`CameraAnimation`]: Smooth transitions between views

use lin_alg::f32::{Mat4, Vec3};
use serde::{Deserialize, Serialize};
use std::f32::consts::PI;

/// Camera projection mode
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Projection {
    /// Perspective projection with depth foreshortening
    #[default]
    Perspective,
    /// Orthographic projection (no depth foreshortening)
    Orthographic,
}

/// 25-value view state (PSE format compatible)
///
/// Stored as a flat array of 25 floats:
/// - [0..16]: 4x4 rotation matrix (column-major)
/// - `[16..19]`: Position (camera position relative to origin)
/// - `[19..22]`: Origin (center of rotation in model space)
/// - `[22]`: Front clipping plane distance
/// - `[23]`: Back clipping plane distance
/// - `[24]`: Field of view (degrees)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SceneView {
    /// Rotation matrix (model -> camera), 4x4 stored as column-major
    #[serde(with = "crate::serde_helpers::mat4_serde")]
    pub rotation: Mat4,
    /// Camera position relative to origin (distance along z-axis typically)
    #[serde(with = "crate::serde_helpers::vec3_serde")]
    pub position: Vec3,
    /// Origin (center of rotation in model space)
    #[serde(with = "crate::serde_helpers::vec3_serde")]
    pub origin: Vec3,
    /// Front clipping plane distance
    pub clip_front: f32,
    /// Back clipping plane distance
    pub clip_back: f32,
    /// Field of view (degrees)
    pub fov: f32,
}

impl Default for SceneView {
    fn default() -> Self {
        Self {
            rotation: Mat4::new_identity(),
            position: Vec3::new(0.0, 0.0, 50.0), // Positive z: camera offset from origin in view space
            origin: Vec3::new(0.0, 0.0, 0.0),
            clip_front: 0.1,
            clip_back: 1000.0,
            fov: 14.0, // default FOV
        }
    }
}

impl SceneView {
    /// Create a new default view
    pub fn new() -> Self {
        Self::default()
    }

    /// Convert to PSE-compatible 25-float array
    pub fn to_pymol_array(&self) -> [f32; 25] {
        let mut arr = [0.0f32; 25];

        // Copy rotation matrix (column-major)
        arr[..16].copy_from_slice(&self.rotation.data);

        // Position
        arr[16] = self.position.x;
        arr[17] = self.position.y;
        arr[18] = self.position.z;

        // Origin
        arr[19] = self.origin.x;
        arr[20] = self.origin.y;
        arr[21] = self.origin.z;

        // Clipping and FOV
        arr[22] = self.clip_front;
        arr[23] = self.clip_back;
        arr[24] = self.fov;

        arr
    }

    /// Create from PSE 25-float array
    pub fn from_pymol_array(arr: &[f32; 25]) -> Self {
        // Convert flat [f32; 16] to [[f32; 4]; 4] (column-major)
        let rotation_data: [[f32; 4]; 4] = [
            [arr[0], arr[1], arr[2], arr[3]],
            [arr[4], arr[5], arr[6], arr[7]],
            [arr[8], arr[9], arr[10], arr[11]],
            [arr[12], arr[13], arr[14], arr[15]],
        ];

        Self {
            rotation: Mat4::from(rotation_data),
            position: Vec3::new(arr[16], arr[17], arr[18]),
            origin: Vec3::new(arr[19], arr[20], arr[21]),
            clip_front: arr[22],
            clip_back: arr[23],
            fov: arr[24],
        }
    }

    /// Compute the view matrix
    ///
    /// The view matrix transforms from world space to camera space.
    pub fn view_matrix(&self) -> Mat4 {
        // Create translation to origin
        let translate_to_origin = Mat4::new_translation(-self.origin);

        // Create translation for camera position
        let translate_position = Mat4::new_translation(-self.position);

        // View = position_translate * rotation * origin_translate
        translate_position * self.rotation.clone() * translate_to_origin
    }

    /// Compute the projection matrix
    pub fn projection_matrix(&self, aspect: f32, projection: Projection) -> Mat4 {
        match projection {
            Projection::Perspective => {
                perspective_matrix(self.fov, aspect, self.clip_front, self.clip_back)
            }
            Projection::Orthographic => {
                // For orthographic, use the z-distance to determine the view size
                let distance = self.position.z.abs();
                let half_height = distance * (self.fov * PI / 360.0).tan();
                let half_width = half_height * aspect;
                orthographic_matrix(
                    -half_width,
                    half_width,
                    -half_height,
                    half_height,
                    self.clip_front,
                    self.clip_back,
                )
            }
        }
    }

    /// Get the camera position in world space
    pub fn world_position(&self) -> Vec3 {
        // Camera position is origin - rotation^T * position
        if let Some(rot_inv) = self.rotation.inverse() {
            let pos = Vec3::new(
                rot_inv.data[0] * self.position.x
                    + rot_inv.data[4] * self.position.y
                    + rot_inv.data[8] * self.position.z,
                rot_inv.data[1] * self.position.x
                    + rot_inv.data[5] * self.position.y
                    + rot_inv.data[9] * self.position.z,
                rot_inv.data[2] * self.position.x
                    + rot_inv.data[6] * self.position.y
                    + rot_inv.data[10] * self.position.z,
            );
            self.origin - pos
        } else {
            self.origin - self.position
        }
    }
}

/// Animation state for smooth camera transitions
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CameraAnimation {
    /// Starting view
    pub start: SceneView,
    /// Target view
    pub target: SceneView,
    /// Animation duration in seconds
    pub duration: f32,
    /// Current elapsed time
    pub elapsed: f32,
}

impl CameraAnimation {
    /// Create a new animation
    pub fn new(start: SceneView, target: SceneView, duration: f32) -> Self {
        Self {
            start,
            target,
            duration,
            elapsed: 0.0,
        }
    }

    /// Update the animation, returns true if still animating
    pub fn update(&mut self, dt: f32) -> bool {
        self.elapsed += dt;
        self.elapsed < self.duration
    }

    /// Get the interpolation factor (0.0 to 1.0)
    pub fn progress(&self) -> f32 {
        (self.elapsed / self.duration).clamp(0.0, 1.0)
    }

    /// Get the current interpolated view
    pub fn current_view(&self) -> SceneView {
        let t = smooth_step(self.progress());

        // Interpolate position
        let position = lerp_vec3(&self.start.position, &self.target.position, t);

        // Interpolate origin
        let origin = lerp_vec3(&self.start.origin, &self.target.origin, t);

        // Interpolate rotation using SLERP-like behavior via matrices
        let rotation = slerp_rotation(&self.start.rotation, &self.target.rotation, t);

        // Interpolate scalars
        let clip_front = lerp(self.start.clip_front, self.target.clip_front, t);
        let clip_back = lerp(self.start.clip_back, self.target.clip_back, t);
        let fov = lerp(self.start.fov, self.target.fov, t);

        SceneView {
            rotation,
            position,
            origin,
            clip_front,
            clip_back,
            fov,
        }
    }
}

/// Interactive camera controller
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Camera {
    /// Current view state
    view: SceneView,
    /// Projection mode
    projection: Projection,
    /// Optional animation in progress
    animation: Option<CameraAnimation>,
    /// Viewport aspect ratio
    aspect: f32,
}

impl Default for Camera {
    fn default() -> Self {
        Self {
            view: SceneView::default(),
            projection: Projection::Perspective,
            animation: None,
            aspect: 1.0,
        }
    }
}

impl Camera {
    /// Create a new camera with default settings
    pub fn new() -> Self {
        Self::default()
    }

    /// Get the current view
    pub fn view(&self) -> &SceneView {
        &self.view
    }

    /// Get mutable access to the view
    pub fn view_mut(&mut self) -> &mut SceneView {
        self.animation = None; // Cancel any animation
        &mut self.view
    }

    /// Set the view directly
    pub fn set_view(&mut self, view: SceneView) {
        self.animation = None;
        self.view = view;
    }

    /// Get the projection mode
    pub fn projection(&self) -> Projection {
        self.projection
    }

    /// Set the projection mode
    pub fn set_projection(&mut self, projection: Projection) {
        self.projection = projection;
    }

    /// Toggle between perspective and orthographic
    pub fn toggle_projection(&mut self) {
        self.projection = match self.projection {
            Projection::Perspective => Projection::Orthographic,
            Projection::Orthographic => Projection::Perspective,
        };
    }

    /// Get the aspect ratio
    pub fn aspect(&self) -> f32 {
        self.aspect
    }

    /// Set the aspect ratio
    pub fn set_aspect(&mut self, aspect: f32) {
        self.aspect = aspect;
    }

    /// Get the view matrix
    pub fn view_matrix(&self) -> Mat4 {
        let view = if let Some(anim) = &self.animation {
            anim.current_view()
        } else {
            self.view.clone()
        };
        view.view_matrix()
    }

    /// Get the projection matrix
    pub fn projection_matrix(&self) -> Mat4 {
        self.projection_matrix_for_aspect(self.aspect)
    }

    /// Get the projection matrix for a given aspect ratio (without mutating the camera).
    pub fn projection_matrix_for_aspect(&self, aspect: f32) -> Mat4 {
        let view = if let Some(anim) = &self.animation {
            anim.current_view()
        } else {
            self.view.clone()
        };
        view.projection_matrix(aspect, self.projection)
    }

    /// Rotate the camera by axis-angle
    ///
    /// `axis` should be normalized, `angle` is in radians.
    pub fn rotate(&mut self, axis: Vec3, angle: f32) {
        self.animation = None;
        let rotation = axis_angle_to_matrix(&axis, angle);
        self.view.rotation = rotation * self.view.rotation.clone();
    }

    /// Rotate around the X axis (pitch)
    pub fn rotate_x(&mut self, angle: f32) {
        self.rotate(Vec3::new(1.0, 0.0, 0.0), angle);
    }

    /// Rotate around the Y axis (yaw)
    pub fn rotate_y(&mut self, angle: f32) {
        self.rotate(Vec3::new(0.0, 1.0, 0.0), angle);
    }

    /// Rotate around the Z axis (roll)
    pub fn rotate_z(&mut self, angle: f32) {
        self.rotate(Vec3::new(0.0, 0.0, 1.0), angle);
    }

    /// Translate the camera (pan)
    ///
    /// Translation is applied in camera space.
    pub fn translate(&mut self, delta: Vec3) {
        self.animation = None;
        // Transform delta from camera space to world space using R^T.
        // For orthogonal rotation matrices R^-1 = R^T, so we use the transpose
        // directly instead of calling inverse(). This avoids lin_alg's inverse()
        // which can return None or a scaled result after numerical drift from
        // accumulated rotation multiplications.
        // Column-major layout: R^T * v uses R.data[i*4 + j] for row i, col j.
        let r = &self.view.rotation;
        let world_delta = Vec3::new(
            r.data[0] * delta.x + r.data[1] * delta.y + r.data[2] * delta.z,
            r.data[4] * delta.x + r.data[5] * delta.y + r.data[6] * delta.z,
            r.data[8] * delta.x + r.data[9] * delta.y + r.data[10] * delta.z,
        );
        self.view.origin += world_delta;
    }

    /// Zoom by changing the camera distance
    ///
    /// Positive factor zooms in, negative zooms out.
    /// Clipping planes scale proportionally but stay centered on the new
    /// camera distance, with a minimum half-width to prevent clipping.
    pub fn zoom(&mut self, factor: f32) {
        self.animation = None;
        let old_distance = self.view.position.z;
        let new_distance = (old_distance * (1.0 - factor * 0.1)).max(1.0);

        if old_distance > 0.0 {
            // Scale slab proportionally, enforce minimum, re-center on new distance
            let old_half = ((self.view.clip_back - self.view.clip_front) * 0.5).max(0.1);
            let scale = new_distance / old_distance;
            let new_half = (old_half * scale).max(new_distance * 0.1).max(5.0);
            self.view.clip_front = (new_distance - new_half).max(0.01);
            self.view.clip_back = new_distance + new_half;
        }

        self.view.position.z = new_distance;
    }

    /// Zoom by changing the field of view
    pub fn zoom_fov(&mut self, delta: f32) {
        self.animation = None;
        self.view.fov = (self.view.fov + delta).clamp(1.0, 179.0);
    }

    /// Compute world-space units per pixel at the origin depth.
    ///
    /// Standard perspective projection: depth * 2*tan(fov/2) / viewport_height.
    pub fn screen_vertex_scale(&self, viewport_height: f32) -> f32 {
        let depth = self.view.position.z;
        let fov_rad = self.view.fov.to_radians();
        depth * 2.0 * (fov_rad / 2.0).tan() / viewport_height
    }

    /// Center the camera on a point
    pub fn center_on(&mut self, point: Vec3) {
        self.animation = None;
        self.view.origin = point;
    }

    /// Reset the camera to default orientation
    pub fn reset_orientation(&mut self) {
        self.animation = None;
        self.view.rotation = Mat4::new_identity();
    }

    /// Reset the camera to view all (requires bounding box).
    /// Resets rotation to identity.
    pub fn reset_view(&mut self, bbox_min: Vec3, bbox_max: Vec3) {
        self.zoom_to(bbox_min, bbox_max, 0.0);
        self.view.rotation = Mat4::new_identity();
    }

    /// Zoom to fit the bounding box while preserving the current rotation.
    ///
    /// `buffer` expands the bounding box by this many angstroms in each direction.
    pub fn zoom_to(&mut self, bbox_min: Vec3, bbox_max: Vec3, buffer: f32) {
        let center = Vec3::new(
            (bbox_min.x + bbox_max.x) * 0.5,
            (bbox_min.y + bbox_max.y) * 0.5,
            (bbox_min.z + bbox_max.z) * 0.5,
        );
        let extent = Vec3::new(
            (bbox_max.x - bbox_min.x) + buffer * 2.0,
            (bbox_max.y - bbox_min.y) + buffer * 2.0,
            (bbox_max.z - bbox_min.z) + buffer * 2.0,
        );
        let radius = (extent.x * extent.x + extent.y * extent.y + extent.z * extent.z).sqrt() * 0.5;
        self.apply_zoom(center, radius);
    }

    /// Zoom to fit a sphere defined by center and radius (used for `complete=1` mode).
    ///
    /// `buffer` is added to the radius.
    pub fn zoom_to_sphere(&mut self, center: Vec3, radius: f32, buffer: f32) {
        self.apply_zoom(center, radius + buffer);
    }

    /// Shared zoom logic: positions camera to frame a sphere of given radius at center.
    fn apply_zoom(&mut self, center: Vec3, radius: f32) {
        self.animation = None;
        self.view.origin = center;

        // Camera distance to fit the bounding sphere (minimum 10 Å)
        let fov_rad = self.view.fov * PI / 180.0;
        let distance = (radius / (fov_rad * 0.5).sin()).max(10.0);
        self.view.position.z = distance;

        // Symmetric clipping centered on camera distance
        let slab_half = radius.max(distance * 0.1).max(5.0);
        self.view.clip_front = (distance - slab_half).max(0.1);
        self.view.clip_back = distance + slab_half;
    }

    /// Center on a bounding box without changing zoom level or rotation
    pub fn center_to(&mut self, bbox_min: Vec3, bbox_max: Vec3) {
        self.animation = None;

        // Calculate center
        let center = Vec3::new(
            (bbox_min.x + bbox_max.x) * 0.5,
            (bbox_min.y + bbox_max.y) * 0.5,
            (bbox_min.z + bbox_max.z) * 0.5,
        );

        // Only update the origin - preserve rotation and distance
        self.view.origin = center;
    }

    /// Start an animated transition to a target view
    pub fn animate_to(&mut self, target: SceneView, duration: f32) {
        if duration <= 0.0 {
            self.view = target;
            self.animation = None;
        } else {
            let start = if let Some(anim) = &self.animation {
                anim.current_view()
            } else {
                self.view.clone()
            };
            self.animation = Some(CameraAnimation::new(start, target, duration));
        }
    }

    /// Update the camera (for animations)
    ///
    /// Returns true if the camera state changed (animation in progress).
    pub fn update(&mut self, dt: f32) -> bool {
        if let Some(anim) = &mut self.animation {
            if !anim.update(dt) {
                // Animation complete
                self.view = anim.target.clone();
                self.animation = None;
            }
            true
        } else {
            false
        }
    }

    /// Check if an animation is in progress
    pub fn is_animating(&self) -> bool {
        self.animation.is_some()
    }

    /// Get the camera position in world space
    pub fn world_position(&self) -> Vec3 {
        let view = if let Some(anim) = &self.animation {
            anim.current_view()
        } else {
            self.view.clone()
        };
        view.world_position()
    }

    /// Set the clipping planes
    pub fn set_clip_planes(&mut self, near: f32, far: f32) {
        self.view.clip_front = near;
        self.view.clip_back = far;
    }

    /// Set the field of view (degrees)
    pub fn set_fov(&mut self, fov: f32) {
        self.view.fov = fov.clamp(1.0, 179.0);
    }

    /// Get the field of view (degrees)
    pub fn fov(&self) -> f32 {
        self.view.fov
    }

    /// Get the current view (respecting animation)
    pub fn current_view(&self) -> SceneView {
        if let Some(anim) = &self.animation {
            anim.current_view()
        } else {
            self.view.clone()
        }
    }

    /// Project a 3D world position to 2D screen coordinates.
    ///
    /// `viewport` is `(x, y, width, height)` in caller-defined screen units.
    /// Returns `None` outside the camera's near/far clip interval.
    pub fn project_to_screen(
        &self,
        world_pos: Vec3,
        viewport: (f32, f32, f32, f32),
    ) -> Option<(f32, f32)> {
        let view = self.view_matrix();
        let proj = self.projection_matrix();
        let mvp = proj * view;

        // Multiply mvp * [x, y, z, 1] (column-major Mat4)
        let x = world_pos.x;
        let y = world_pos.y;
        let z = world_pos.z;
        let clip_x = mvp.data[0] * x + mvp.data[4] * y + mvp.data[8] * z + mvp.data[12];
        let clip_y = mvp.data[1] * x + mvp.data[5] * y + mvp.data[9] * z + mvp.data[13];
        let clip_z = mvp.data[2] * x + mvp.data[6] * y + mvp.data[10] * z + mvp.data[14];
        let clip_w = mvp.data[3] * x + mvp.data[7] * y + mvp.data[11] * z + mvp.data[15];

        if !clip_w.is_finite() || clip_w <= 0.0 {
            return None;
        }

        let ndc_x = clip_x / clip_w;
        let ndc_y = clip_y / clip_w;
        let ndc_z = clip_z / clip_w;
        if !ndc_x.is_finite()
            || !ndc_y.is_finite()
            || !ndc_z.is_finite()
            || !(0.0..=1.0).contains(&ndc_z)
        {
            return None;
        }

        let (vp_x, vp_y, vp_w, vp_h) = viewport;
        let screen_x = vp_x + (ndc_x + 1.0) * 0.5 * vp_w;
        let screen_y = vp_y + (1.0 - ndc_y) * 0.5 * vp_h; // flip Y for screen coords

        Some((screen_x, screen_y))
    }
}

// ============================================================================
// Helper Functions
// ============================================================================

/// Create a perspective projection matrix for wgpu (depth range [0, 1])
///
/// Right-handed coordinate system, camera looks down -Z.
/// Maps view-space z in [-far, -near] to NDC z in [0, 1].
fn perspective_matrix(fov_degrees: f32, aspect: f32, near: f32, far: f32) -> Mat4 {
    let fov_rad = fov_degrees * PI / 180.0;
    let f = 1.0 / (fov_rad * 0.5).tan();

    // wgpu uses [0, 1] depth range (not [-1, 1] like OpenGL)
    // For view.z = -near -> ndc.z = 0
    // For view.z = -far  -> ndc.z = 1
    let a = far / (near - far);
    let b = near * far / (near - far);

    // lin_alg Mat4::from expects ROW-major input (each inner array is a ROW).
    // The standard perspective matrix in ROW notation:
    //
    //         col0        col1    col2    col3
    // row 0: f/aspect     0       0       0
    // row 1:    0         f       0       0
    // row 2:    0         0       a       b
    // row 3:    0         0      -1       0
    //
    // This gives: clip.w = -view.z (from row 3, col 2)

    // Create the matrix by directly setting the data array.
    // lin_alg Mat4 stores data as a flat [f32; 16] array.
    // Based on testing, it appears to use column-major storage where
    // data[i*4 + j] = M[col i][row j].
    //
    // For the perspective projection matrix we need (in standard math notation):
    //         col0        col1    col2    col3
    // row 0: f/aspect     0       0       0
    // row 1:    0         f       0       0
    // row 2:    0         0       a       b
    // row 3:    0         0      -1       0
    //
    // In column-major: data = [col0, col1, col2, col3] flattened
    // = [f/asp, 0, 0, 0,  0, f, 0, 0,  0, 0, a, -1,  0, 0, b, 0]

    let mut result = Mat4::new_identity();
    result.data[0] = f / aspect; // [0][0]
    result.data[1] = 0.0;
    result.data[2] = 0.0;
    result.data[3] = 0.0;

    result.data[4] = 0.0;
    result.data[5] = f; // [1][1]
    result.data[6] = 0.0;
    result.data[7] = 0.0;

    result.data[8] = 0.0;
    result.data[9] = 0.0;
    result.data[10] = a; // [2][2]
    result.data[11] = -1.0; // [2][3] - for clip.w = -view.z

    result.data[12] = 0.0;
    result.data[13] = 0.0;
    result.data[14] = b; // [3][2] - for clip.z depth offset
    result.data[15] = 0.0;

    result
}

/// Create an orthographic projection matrix for wgpu (depth range [0, 1])
///
/// Maps view-space to clip-space.
/// Maps view-space z in [-far, -near] to NDC z in [0, 1].
fn orthographic_matrix(left: f32, right: f32, bottom: f32, top: f32, near: f32, far: f32) -> Mat4 {
    // wgpu uses [0, 1] depth range (not [-1, 1] like OpenGL)
    // Direct column-major construction (same approach as perspective_matrix)
    //
    // Standard ortho matrix:
    //         col0              col1              col2              col3
    // row 0: 2/(r-l)           0                 0                 -(r+l)/(r-l)
    // row 1: 0                 2/(t-b)           0                 -(t+b)/(t-b)
    // row 2: 0                 0                -1/(f-n)           -n/(f-n)
    // row 3: 0                 0                 0                  1

    let mut result = Mat4::new_identity();

    // Column 0
    result.data[0] = 2.0 / (right - left);
    result.data[1] = 0.0;
    result.data[2] = 0.0;
    result.data[3] = 0.0;

    // Column 1
    result.data[4] = 0.0;
    result.data[5] = 2.0 / (top - bottom);
    result.data[6] = 0.0;
    result.data[7] = 0.0;

    // Column 2
    result.data[8] = 0.0;
    result.data[9] = 0.0;
    result.data[10] = -1.0 / (far - near);
    result.data[11] = 0.0;

    // Column 3
    result.data[12] = -(right + left) / (right - left);
    result.data[13] = -(top + bottom) / (top - bottom);
    result.data[14] = -near / (far - near);
    result.data[15] = 1.0;

    result
}

/// Create a rotation matrix from axis-angle representation
fn axis_angle_to_matrix(axis: &Vec3, angle: f32) -> Mat4 {
    let c = angle.cos();
    let s = angle.sin();
    let t = 1.0 - c;

    let x = axis.x;
    let y = axis.y;
    let z = axis.z;

    let data: [[f32; 4]; 4] = [
        [t * x * x + c, t * x * y + s * z, t * x * z - s * y, 0.0],
        [t * x * y - s * z, t * y * y + c, t * y * z + s * x, 0.0],
        [t * x * z + s * y, t * y * z - s * x, t * z * z + c, 0.0],
        [0.0, 0.0, 0.0, 1.0],
    ];

    Mat4::from(data)
}

/// Linear interpolation for scalars
fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

/// Linear interpolation for vectors
fn lerp_vec3(a: &Vec3, b: &Vec3, t: f32) -> Vec3 {
    Vec3::new(lerp(a.x, b.x, t), lerp(a.y, b.y, t), lerp(a.z, b.z, t))
}

/// Smooth step interpolation (ease in/out)
fn smooth_step(t: f32) -> f32 {
    t * t * (3.0 - 2.0 * t)
}

/// Interpolate rotation matrices (simplified SLERP-like behavior)
///
/// This uses a simple linear interpolation of matrix elements followed by
/// orthonormalization. For small rotations this works well; for large
/// rotations consider implementing proper quaternion SLERP.
fn slerp_rotation(a: &Mat4, b: &Mat4, t: f32) -> Mat4 {
    // Linear interpolation of matrix elements
    let data: [[f32; 4]; 4] = [
        [
            lerp(a.data[0], b.data[0], t),
            lerp(a.data[1], b.data[1], t),
            lerp(a.data[2], b.data[2], t),
            lerp(a.data[3], b.data[3], t),
        ],
        [
            lerp(a.data[4], b.data[4], t),
            lerp(a.data[5], b.data[5], t),
            lerp(a.data[6], b.data[6], t),
            lerp(a.data[7], b.data[7], t),
        ],
        [
            lerp(a.data[8], b.data[8], t),
            lerp(a.data[9], b.data[9], t),
            lerp(a.data[10], b.data[10], t),
            lerp(a.data[11], b.data[11], t),
        ],
        [
            lerp(a.data[12], b.data[12], t),
            lerp(a.data[13], b.data[13], t),
            lerp(a.data[14], b.data[14], t),
            lerp(a.data[15], b.data[15], t),
        ],
    ];

    // Orthonormalize the rotation part (Gram-Schmidt)
    let mut result = Mat4::from(data);
    orthonormalize_rotation(&mut result);

    result
}

/// Orthonormalize the rotation part of a matrix using Gram-Schmidt
fn orthonormalize_rotation(m: &mut Mat4) {
    // Extract basis vectors
    let mut x = Vec3::new(m.data[0], m.data[1], m.data[2]);
    let mut y = Vec3::new(m.data[4], m.data[5], m.data[6]);
    // Normalize x
    x = x.to_normalized();

    // y = y - proj(y, x)
    let dot_yx = y.x * x.x + y.y * x.y + y.z * x.z;
    y = Vec3::new(y.x - dot_yx * x.x, y.y - dot_yx * x.y, y.z - dot_yx * x.z);
    y = y.to_normalized();

    // z = x cross y (ensures right-handed)
    let z = Vec3::new(
        x.y * y.z - x.z * y.y,
        x.z * y.x - x.x * y.z,
        x.x * y.y - x.y * y.x,
    );

    // Write back
    m.data[0] = x.x;
    m.data[1] = x.y;
    m.data[2] = x.z;
    m.data[4] = y.x;
    m.data[5] = y.y;
    m.data[6] = y.z;
    m.data[8] = z.x;
    m.data[9] = z.y;
    m.data[10] = z.z;
}

/// Normalize a matrix inverse in place.
///
/// `lin_alg`'s `Mat4::inverse()` can return a scaled result where the
/// homogeneous element `data[15]` is not 1.0.  Dividing every element
/// by `data[15]` restores proper affine-transform semantics.
pub fn normalize_matrix(m: &mut Mat4) {
    let w = m.data[15];
    if w.abs() > 1e-6 && (w - 1.0).abs() > 1e-6 {
        for v in &mut m.data {
            *v /= w;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_scene_view_default() {
        let view = SceneView::default();
        assert_eq!(view.fov, 14.0);
        assert!(view.position.z > 0.0); // Camera distance from origin (positive)
    }

    #[test]
    fn test_scene_view_roundtrip() {
        let view = SceneView {
            rotation: Mat4::new_identity(),
            position: Vec3::new(1.0, 2.0, 30.0),
            origin: Vec3::new(10.0, 20.0, 30.0),
            clip_front: 0.5,
            clip_back: 500.0,
            fov: 20.0,
        };

        let arr = view.to_pymol_array();
        let view2 = SceneView::from_pymol_array(&arr);

        assert!((view.position.x - view2.position.x).abs() < 0.001);
        assert!((view.origin.z - view2.origin.z).abs() < 0.001);
        assert!((view.fov - view2.fov).abs() < 0.001);
    }

    #[test]
    fn test_camera_rotate() {
        let mut camera = Camera::new();
        camera.rotate_y(PI / 4.0);
        // After rotation, the rotation matrix should not be identity
        assert!((camera.view.rotation.data[0] - 1.0).abs() > 0.001);
    }

    #[test]
    fn test_camera_zoom() {
        let mut camera = Camera::new();
        let initial_z = camera.view.position.z;
        camera.zoom(1.0); // Zoom in
        assert!(camera.view.position.z < initial_z); // Closer to origin (smaller positive z)
    }

    #[test]
    fn test_camera_animation() {
        let mut camera = Camera::new();
        let target = SceneView {
            position: Vec3::new(0.0, 0.0, 100.0),
            ..SceneView::default()
        };

        camera.animate_to(target.clone(), 1.0);
        assert!(camera.is_animating());

        // Update halfway
        camera.update(0.5);
        assert!(camera.is_animating());

        // Update past duration
        camera.update(0.6);
        assert!(!camera.is_animating());
        assert!((camera.view.position.z - target.position.z).abs() < 0.001);
    }

    #[test]
    fn test_projection_matrices() {
        // Test perspective matrix
        let persp = perspective_matrix(60.0, 1.5, 0.1, 100.0);
        // Matrix should not be identity
        assert!(
            (persp.data[0] - 1.0).abs() > 0.001,
            "Perspective should differ from identity"
        );
        // Should have non-zero values (not all zeros)
        let non_zero_count = persp.data.iter().filter(|&&v| v.abs() > 0.0001).count();
        assert!(
            non_zero_count >= 4,
            "Perspective matrix should have several non-zero values"
        );

        // Test orthographic matrix
        let ortho = orthographic_matrix(-10.0, 10.0, -10.0, 10.0, 0.1, 100.0);
        // Should have non-zero values
        let non_zero_count = ortho.data.iter().filter(|&&v| v.abs() > 0.0001).count();
        assert!(
            non_zero_count >= 4,
            "Orthographic matrix should have several non-zero values"
        );
        // Should have 1.0 somewhere (the w=1 term)
        let has_one = ortho.data.iter().any(|&v| (v - 1.0).abs() < 0.001);
        assert!(has_one, "Orthographic matrix should contain 1.0");
    }

    #[test]
    fn screen_projection_rejects_points_outside_clip_depth() {
        let camera = Camera::new();
        let viewport = (0.0, 0.0, 100.0, 100.0);

        assert!(camera
            .project_to_screen(Vec3::new(0.0, 0.0, 0.0), viewport)
            .is_some());
        assert!(camera
            .project_to_screen(Vec3::new(0.0, 0.0, 49.95), viewport)
            .is_none());
        assert!(camera
            .project_to_screen(Vec3::new(0.0, 0.0, 51.0), viewport)
            .is_none());
        assert!(camera
            .project_to_screen(Vec3::new(0.0, 0.0, -951.0), viewport)
            .is_none());
    }
}
