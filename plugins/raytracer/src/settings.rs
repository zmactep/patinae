//! Raytracing settings: dynamic settings reader and GPU settings struct.

// ---------------------------------------------------------------------------
// ResolvedRaySettings — values read from the plugin's dynamic setting store
// ---------------------------------------------------------------------------

pub(crate) struct ResolvedRaySettings {
    pub shadow: bool,
    pub max_passes: i32,
    pub mode: i32,
    pub fog: f32,
    pub color: patinae_color::ColorValue,
    pub opaque_background: i32,
    pub transparency_shadows: bool,
    pub depth_factor: f32,
    pub slope_factor: f32,
    pub disco_factor: f32,
    pub gain: f32,
    pub use_custom: bool,
    pub custom_ambient: f32,
    pub custom_direct: f32,
    pub custom_reflect: f32,
    pub custom_specular: f32,
    pub custom_shininess: f32,
}

pub(crate) fn read_ray_settings(
    get_bool: impl Fn(&str, bool) -> bool,
    get_i32: impl Fn(&str, i32) -> i32,
    get_f32: impl Fn(&str, f32) -> f32,
    color: patinae_color::ColorValue,
) -> ResolvedRaySettings {
    ResolvedRaySettings {
        shadow: get_bool("ray_shadow", true),
        max_passes: get_i32("ray_max_passes", 25),
        mode: get_i32("ray_trace_mode", 0),
        fog: get_f32("ray_trace_fog", -1.0),
        color,
        opaque_background: get_i32("ray_opaque_background", -1),
        transparency_shadows: get_bool("ray_transparency_shadows", true),
        depth_factor: get_f32("ray_trace_depth_factor", 0.1),
        slope_factor: get_f32("ray_trace_slope_factor", 0.6),
        disco_factor: get_f32("ray_trace_disco_factor", 0.05),
        gain: get_f32("ray_trace_gain", 0.12),
        use_custom: get_bool("rt_use_custom", false),
        custom_ambient: get_f32("rt_ambient", 0.14),
        custom_direct: get_f32("rt_direct", 0.45),
        custom_reflect: get_f32("rt_reflect", 0.45),
        custom_specular: get_f32("rt_specular", 0.5),
        custom_shininess: get_f32("rt_shininess", 40.0),
    }
}

// ---------------------------------------------------------------------------
// RaytraceSettings — flat parameter bag passed to the GPU pipeline
// ---------------------------------------------------------------------------

/// Maximum number of directional lights supported.
pub const MAX_LIGHTS: usize = 9;

/// Raytracing-specific settings used to build GPU uniforms.
#[derive(Clone, Debug)]
pub struct RaytraceSettings {
    /// Light directions (normalized, up to 9 lights)
    pub light_dirs: [[f32; 4]; MAX_LIGHTS],
    /// Number of active lights
    pub light_count: i32,
    /// Number of lights contributing specular (-1 = all positional lights)
    pub spec_count: i32,
    /// Ambient light intensity
    pub ambient: f32,
    /// Direct light intensity
    pub direct: f32,
    /// Reflect light intensity (secondary fill light)
    pub reflect: f32,
    /// Specular intensity
    pub specular: f32,
    /// Specular shininess
    pub shininess: f32,
    /// Background color (RGBA)
    pub bg_color: [f32; 4],
    /// Fog start distance
    pub fog_start: f32,
    /// Fog end distance
    pub fog_end: f32,
    /// Fog density (0 = no fog)
    pub fog_density: f32,
    /// Fog color (RGBA)
    pub fog_color: [f32; 4],
    /// Enable shadows
    pub ray_shadow: bool,
    /// Maximum passes for transparency
    pub ray_max_passes: u32,
    /// Apply fog in raytracing
    pub ray_trace_fog: bool,
    /// Shadows through transparent objects
    pub ray_transparency_shadows: bool,
    /// Ray trace mode (0=normal, 1=normal+outline, 2=outline only, 3=quantized+outline)
    pub ray_trace_mode: i32,
    /// Color for outlines in modes 1, 2, and 3 (RGBA)
    pub ray_trace_color: [f32; 4],
    /// Whether the background is opaque
    pub ray_opaque_background: bool,
    /// Transparency rendering mode (0=fast/opaque, 1=multi-layer, 2=uni-layer)
    pub transparency_mode: i32,
    /// Edge detection: gradient magnitude difference threshold
    pub ray_trace_slope_factor: f32,
    /// Edge detection: gradient direction difference threshold
    pub ray_trace_depth_factor: f32,
    /// Edge detection: gradient discontinuity threshold
    pub ray_trace_disco_factor: f32,
    /// Edge detection: pixel radius adjustment factor
    pub ray_trace_gain: f32,
    /// Silhouette edge thickness in pixels (sampling distance)
    pub silhouette_thickness: f32,
    /// Silhouette depth discontinuity threshold
    pub silhouette_depth_jump: f32,
}

/// Default view-space light directions for classic shading.
const DEFAULT_LIGHT_DIRS: [[f32; 4]; MAX_LIGHTS] = [
    [-0.4, -0.4, -1.0, 0.0],
    [-0.55, -0.7, 0.15, 0.0],
    [0.3, -0.6, -0.2, 0.0],
    [-1.2, 0.3, -0.2, 0.0],
    [0.3, 0.6, -0.75, 0.0],
    [-0.3, 0.5, 0.0, 0.0],
    [0.9, -0.1, -0.15, 0.0],
    [1.3, 2.0, 0.8, 0.0],
    [-1.7, -0.5, 1.2, 0.0],
];

impl Default for RaytraceSettings {
    fn default() -> Self {
        Self {
            light_dirs: DEFAULT_LIGHT_DIRS,
            light_count: 2,
            spec_count: -1,
            ambient: 0.14,
            direct: 0.45,
            reflect: 0.45,
            specular: 0.5,
            shininess: 40.0,
            bg_color: [0.0, 0.0, 0.0, 1.0],
            fog_start: 0.45,
            fog_end: 1.0,
            fog_density: 0.0,
            fog_color: [0.0, 0.0, 0.0, 1.0],
            ray_shadow: true,
            ray_max_passes: 25,
            ray_trace_fog: false,
            ray_transparency_shadows: true,
            ray_trace_mode: 0,
            ray_trace_color: [0.0, 0.0, 0.0, 1.0],
            ray_opaque_background: true,
            transparency_mode: 2,
            ray_trace_slope_factor: 0.6,
            ray_trace_depth_factor: 0.1,
            ray_trace_disco_factor: 0.05,
            ray_trace_gain: 0.12,
            silhouette_thickness: 1.0,
            silhouette_depth_jump: 0.03,
        }
    }
}
