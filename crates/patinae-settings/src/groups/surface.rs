//! Surface representation settings (object-overridable).

use crate::define_settings_group;
use crate::enums::SurfaceMode;
use crate::ColorValue;

define_settings_group! {
    /// Surface representation parameters.
    group SurfaceSettings / SurfaceOverrides {
        mode: SurfaceMode = SurfaceMode::Normal,
            name = "surface_mode",
            hints = SurfaceMode,
            side_effects = [RepresentationRebuild];
        solvent_radius: f32 = 1.4,
            name = "solvent_radius",
            min = 0.0, max = 10.0,
            side_effects = [RepresentationRebuild];
        quality: i32 = 1,
            name = "surface_quality",
            side_effects = [RepresentationRebuild];
        color_smoothing: i32 = 1,
            name = "surface_color_smoothing",
            side_effects = [ColorRebuild];
        color_smoothing_threshold: f32 = 0.05,
            name = "surface_color_smoothing_threshold",
            side_effects = [ColorRebuild];
        individual_chains: bool = false,
            name = "surface_individual_chains",
            side_effects = [RepresentationRebuild];
        transparency: f32 = 0.0,
            name = "surface_transparency",
            min = 0.0, max = 1.0,
            side_effects = [SurfaceTransparency];
        transparency_mode: i32 = 2,
            name = "transparency_mode",
            side_effects = [SceneInvalidate];
        solvent: bool = false,
            name = "surface_solvent",
            side_effects = [RepresentationRebuild];
        @global {
            color: ColorValue = ColorValue::INHERIT,
                name = "surface_color",
                side_effects = [ColorRebuild];
        }
    }
}
