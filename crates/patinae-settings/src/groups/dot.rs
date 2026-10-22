//! Dot representation settings (object-overridable).

use crate::define_settings_group;
use crate::ColorValue;

define_settings_group! {
    /// Dot representation parameters.
    group DotSettings / DotOverrides {
        density: i32 = 2,
            name = "dot_density",
            min = 0.0, max = 4.0,
            side_effects = [RepresentationRebuild];
        width: f32 = 2.0,
            name = "dot_width",
            min = 0.5, max = 20.0,
            side_effects = [RepresentationRebuild];
        @global {
            color: ColorValue = ColorValue::INHERIT,
                name = "dot_color",
                side_effects = [ColorRebuild];
        }
    }
}
