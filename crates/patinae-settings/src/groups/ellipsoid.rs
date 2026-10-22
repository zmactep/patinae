//! Ellipsoid representation settings (object-overridable).
//!
//! Ellipsoids visualize per-atom anisotropic displacement parameters (ADPs)
//! from the U-tensor (`Atom.anisou`). Each atom with an ADP renders as a
//! 3-axis impostor whose principal axes are the eigenvectors of its U-tensor
//! scaled by `sqrt(eigenvalue) · ellipsoid_scale`. Atoms without an ADP fall
//! back to an isotropic ellipsoid sized from `b_factor`.

use crate::define_settings_group;
use crate::ColorValue;

define_settings_group! {
    /// Ellipsoid representation parameters.
    group EllipsoidSettings / EllipsoidOverrides {
        scale: f32 = 1.0,
            name = "ellipsoid_scale",
            min = 0.01, max = 10.0,
            side_effects = [RepresentationRebuild];
        probability: f32 = 0.5,
            name = "ellipsoid_probability",
            min = 0.01, max = 0.99,
            side_effects = [RepresentationRebuild];
        transparency: f32 = 0.0,
            name = "ellipsoid_transparency",
            min = 0.0, max = 1.0,
            side_effects = [ColorRebuild];
        @global {
            color: ColorValue = ColorValue::INHERIT,
                name = "ellipsoid_color",
                side_effects = [ColorRebuild];
        }
    }
}
