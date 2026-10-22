//! Mesh representation settings (object-overridable).

use crate::define_settings_group;
use crate::ColorValue;

define_settings_group! {
    /// Mesh representation parameters. Mirrors SurfaceSettings for the
    /// quality/solvent/transparency knobs — mesh runs its own MC compute
    /// at its own voxel grid, independently of surface.
    group MeshSettings / MeshOverrides {
        quality: i32 = 1,
            name = "mesh_quality",
            side_effects = [RepresentationRebuild];
        solvent: bool = false,
            name = "mesh_solvent",
            side_effects = [RepresentationRebuild];
        solvent_radius: f32 = 1.4,
            name = "mesh_solvent_radius",
            min = 0.0, max = 10.0,
            side_effects = [RepresentationRebuild];
        transparency: f32 = 0.0,
            name = "mesh_transparency",
            min = 0.0, max = 1.0,
            side_effects = [SurfaceTransparency];
        @global {
            color: ColorValue = ColorValue::INHERIT,
                name = "mesh_color",
                side_effects = [ColorRebuild];
        }
    }
}
