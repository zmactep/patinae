//! Stick representation settings (object-overridable).

use crate::define_settings_group;
use crate::ColorValue;

define_settings_group! {
    /// Stick representation parameters.
    group StickSettings / StickOverrides {
        radius: f32 = 0.25,
            name = "stick_radius",
            min = 0.01, max = 5.0,
            side_effects = [RepresentationRebuild];
        valence_scale: f32 = 1.0,
            name = "stick_valence_scale",
            min = 0.0, max = 5.0,
            side_effects = [RepresentationRebuild];
        valence: bool = true,
            name = "valence",
            side_effects = [RepresentationRebuild];
        transparency: f32 = 0.0,
            name = "stick_transparency",
            min = 0.0, max = 1.0,
            side_effects = [ColorRebuild];
        @global {
            color: ColorValue = ColorValue::INHERIT,
                name = "stick_color",
                side_effects = [ColorRebuild];
        }
    }
}
