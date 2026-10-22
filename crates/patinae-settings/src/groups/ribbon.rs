//! Ribbon representation settings (object-overridable).

use crate::define_settings_group;
use crate::ColorValue;

define_settings_group! {
    /// Ribbon representation parameters.
    group RibbonSettings / RibbonOverrides {
        sampling: i32 = 1,
            name = "ribbon_sampling",
            side_effects = [RepresentationRebuild];
        power: f32 = 2.0,
            name = "ribbon_power",
            min = 0.0, max = 10.0,
            side_effects = [RepresentationRebuild];
        power_b: f32 = 0.5,
            name = "ribbon_power_b",
            min = 0.0, max = 10.0,
            side_effects = [RepresentationRebuild];
        throw: f32 = 1.35,
            name = "ribbon_throw",
            min = 0.0, max = 10.0,
            side_effects = [RepresentationRebuild];
        radius: f32 = 0.0,
            name = "ribbon_radius",
            min = 0.0, max = 5.0,
            side_effects = [RepresentationRebuild];
        @global {
            color: ColorValue = ColorValue::INHERIT,
                name = "ribbon_color",
                side_effects = [ColorRebuild];
        }
    }
}
