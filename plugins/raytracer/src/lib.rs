//! GPU raytracing plugin.
//!
//! Provides the `ray` command and ray tracing settings. All raytracing logic
//! (BVH, compute shaders, primitive collection) lives in this plugin.

use patinae_plugin::{define_plugin_settings, patinae_plugin};

// Raytracer engine modules
mod artifact_gpu;
pub mod bvh;
pub mod collect;
pub mod commands;
pub mod edge_pipeline;
pub mod error;
pub mod gpu;
mod panel;
pub mod pipeline;
pub mod primitive;
pub mod scene;
pub mod settings;
mod shaders;
// ---------------------------------------------------------------------------
// Plugin settings
// ---------------------------------------------------------------------------

define_plugin_settings! {
    RaySettings {
        max_passes: i32 = 25, name = "ray_max_passes",
            min = 1.0, max = 100.0;
        shadow: bool = true, name = "ray_shadow";
        mode: i32 = 0, name = "ray_trace_mode";
        fog: f32 = -1.0, name = "ray_trace_fog";
        color: i32 = -6, name = "ray_trace_color";
        opaque_background: i32 = -1, name = "ray_opaque_background";
        transparency_shadows: bool = true, name = "ray_transparency_shadows";
        depth_factor: f32 = 0.1, name = "ray_trace_depth_factor",
            min = 0.0, max = 1.0;
        slope_factor: f32 = 0.6, name = "ray_trace_slope_factor",
            min = 0.0, max = 2.0;
        disco_factor: f32 = 0.05, name = "ray_trace_disco_factor",
            min = 0.0, max = 1.0;
        gain: f32 = 0.12, name = "ray_trace_gain",
            min = 0.0, max = 1.0;
        use_custom: bool = false, name = "rt_use_custom";
        custom_ambient: f32 = 0.14, name = "rt_ambient",
            min = 0.0, max = 1.0;
        custom_direct: f32 = 0.45, name = "rt_direct",
            min = 0.0, max = 1.0;
        custom_reflect: f32 = 0.45, name = "rt_reflect",
            min = 0.0, max = 1.0;
        custom_specular: f32 = 0.5, name = "rt_specular",
            min = 0.0, max = 1.0;
        custom_shininess: f32 = 40.0, name = "rt_shininess",
            min = 1.0, max = 128.0;
    }
}

// ---------------------------------------------------------------------------
// Plugin declaration
// ---------------------------------------------------------------------------

patinae_plugin! {
    name: "raytracer",
    description: "GPU compute shader raytracing — provides the 'ray' command",
    commands: [commands::RayCommand],
    panels: [panel::RtPanel::new()],
    settings: [RaySettings],
    background_registration: true,
}
