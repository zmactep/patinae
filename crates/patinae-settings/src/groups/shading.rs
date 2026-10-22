//! Shading settings — mode selection + per-mode parameters.

use crate::define_settings_group;
use crate::macros::{append_nested_global_descriptors, FieldDescriptor};
use crate::shading_mode::ShadingMode;
use crate::ColorValue;

/// Top-level shading container. Global-only (not object-overridable).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ShadingSettings {
    pub mode: ShadingMode,
    pub common: CommonShadingSettings,
    pub classic: ClassicShadingSettings,
    pub skripkin: SkripkinShadingSettings,
    pub full: FullShadingSettings,
}

impl Default for ShadingSettings {
    fn default() -> Self {
        Self {
            mode: ShadingMode::Classic,
            common: CommonShadingSettings::default(),
            classic: ClassicShadingSettings::default(),
            skripkin: SkripkinShadingSettings::default(),
            full: FullShadingSettings::default(),
        }
    }
}

impl ShadingSettings {
    /// Return descriptor entries for the top-level shading mode plus all
    /// nested shading groups.
    pub fn field_descriptors() -> Vec<FieldDescriptor<Self, ()>> {
        let mut descs = vec![FieldDescriptor::<Self, ()>::global::<ShadingMode>(
            "shading_mode",
            |s| &s.mode,
            |s| &mut s.mode,
            None,
            None,
            <ShadingMode as crate::SettingEnum>::value_hints(),
            &[
                crate::SideEffectCategory::ShaderReload,
                crate::SideEffectCategory::ShaderComputeLighting,
                crate::SideEffectCategory::SceneInvalidate,
            ],
        )];

        append_nested_global_descriptors(
            &mut descs,
            CommonShadingSettings::field_descriptors(),
            |s| &s.common,
            |s| &mut s.common,
        );
        append_nested_global_descriptors(
            &mut descs,
            ClassicShadingSettings::field_descriptors(),
            |s| &s.classic,
            |s| &mut s.classic,
        );
        append_nested_global_descriptors(
            &mut descs,
            SkripkinShadingSettings::field_descriptors(),
            |s| &s.skripkin,
            |s| &mut s.skripkin,
        );
        append_nested_global_descriptors(
            &mut descs,
            FullShadingSettings::field_descriptors(),
            |s| &s.full,
            |s| &mut s.full,
        );

        descs
    }
}

define_settings_group! {
    /// Settings active in all shading modes (fog, silhouettes, depth cue).
    group_global CommonShadingSettings {
        silhouettes: bool = false,
            name = "silhouettes",
            side_effects = [ShaderReload, SceneInvalidate];
        silhouette_width: f32 = 2.0,
            name = "silhouette_width",
            min = 0.1, max = 10.0,
            side_effects = [SceneInvalidate];
        silhouette_color: ColorValue = ColorValue::INHERIT,
            name = "silhouette_color",
            side_effects = [SceneInvalidate];
        silhouette_depth_jump: f32 = 0.03,
            name = "silhouette_depth_jump",
            min = 0.0, max = 1.0,
            side_effects = [SceneInvalidate];
        depth_cue: bool = true,
            name = "depth_cue",
            side_effects = [ShaderReload, SceneInvalidate];
        fog: f32 = 1.0,
            name = "fog",
            min = 0.0, max = 1.0,
            side_effects = [SceneInvalidate];
        fog_start: f32 = 0.45,
            name = "fog_start",
            min = 0.0, max = 1.0,
            side_effects = [SceneInvalidate];
    }
}

define_settings_group! {
    /// Classic multi-light model (ambient + direct + reflect + specular).
    /// Also used by Full mode (Classic lighting + shadow maps).
    group_global ClassicShadingSettings {
        ambient: f32 = 0.14,
            name = "ambient",
            min = 0.0, max = 1.0,
            side_effects = [ShaderComputeLighting, SceneInvalidate];
        direct: f32 = 0.45,
            name = "direct",
            min = 0.0, max = 1.0,
            side_effects = [ShaderComputeLighting, SceneInvalidate];
        reflect: f32 = 0.45,
            name = "reflect",
            min = 0.0, max = 1.0,
            side_effects = [ShaderComputeLighting, SceneInvalidate];
        specular: f32 = 1.0,
            name = "specular",
            min = 0.0, max = 1.0,
            side_effects = [ShaderComputeLighting, SceneInvalidate];
        shininess: f32 = 55.0,
            name = "shininess",
            min = 0.0, max = 1000.0,
            side_effects = [ShaderComputeLighting, SceneInvalidate];
        spec_direct: f32 = 0.0,
            name = "spec_direct",
            min = 0.0, max = 1.0,
            side_effects = [ShaderComputeLighting, SceneInvalidate];
        spec_direct_power: f32 = 55.0,
            name = "spec_direct_power",
            min = 0.0, max = 1000.0,
            side_effects = [ShaderComputeLighting, SceneInvalidate];
        spec_count: i32 = -1,
            name = "spec_count",
            side_effects = [ShaderComputeLighting, SceneInvalidate];
        light_count: i32 = 2,
            name = "light_count",
            min = 1.0, max = 9.0,
            side_effects = [ShaderComputeLighting, SceneInvalidate];
        light: [f32; 3] = [-0.4, -0.4, -1.0],
            name = "light",
            side_effects = [ShaderComputeLighting, SceneInvalidate];
        light2: [f32; 3] = [-0.55, -0.7, 0.15],
            name = "light2",
            side_effects = [ShaderComputeLighting, SceneInvalidate];
        light3: [f32; 3] = [0.3, -0.6, -0.2],
            name = "light3",
            side_effects = [ShaderComputeLighting, SceneInvalidate];
        light4: [f32; 3] = [-1.2, 0.3, -0.2],
            name = "light4",
            side_effects = [ShaderComputeLighting, SceneInvalidate];
        light5: [f32; 3] = [0.3, 0.6, -0.75],
            name = "light5",
            side_effects = [ShaderComputeLighting, SceneInvalidate];
        light6: [f32; 3] = [-0.3, 0.5, 0.0],
            name = "light6",
            side_effects = [ShaderComputeLighting, SceneInvalidate];
        light7: [f32; 3] = [0.9, -0.1, -0.15],
            name = "light7",
            side_effects = [ShaderComputeLighting, SceneInvalidate];
        light8: [f32; 3] = [1.3, 2.0, 0.8],
            name = "light8",
            side_effects = [ShaderComputeLighting, SceneInvalidate];
        light9: [f32; 3] = [-1.7, -0.5, 1.2],
            name = "light9",
            side_effects = [ShaderComputeLighting, SceneInvalidate];
    }
}

define_settings_group! {
    /// Skripkin AO mode: pure ambient lighting with AO-controlled contrast.
    /// Has its own ambient/specular/shininess values so mode switching preserves them.
    group_global SkripkinShadingSettings {
        directions: i32 = 64,
            name = "skripkin_directions",
            min = 1.0, max = 256.0,
            side_effects = [ShaderReload, SceneInvalidate];
        map_size: i32 = 128,
            name = "skripkin_map_size",
            min = 32.0, max = 4096.0,
            side_effects = [ShaderReload, SceneInvalidate];
        bias: f32 = 0.025,
            name = "skripkin_bias",
            min = 0.0, max = 1.0,
            side_effects = [SceneInvalidate];
        intensity: f32 = 1.0,
            name = "skripkin_intensity",
            min = 0.0, max = 2.0,
            side_effects = [SceneInvalidate];
        ambient: f32 = 1.0,
            name = "skripkin_ambient",
            min = 0.0, max = 1.0,
            side_effects = [ShaderComputeLighting, SceneInvalidate];
        specular: f32 = 0.0,
            name = "skripkin_specular",
            min = 0.0, max = 1.0,
            side_effects = [ShaderComputeLighting, SceneInvalidate];
        shininess: f32 = 55.0,
            name = "skripkin_shininess",
            min = 0.0, max = 1000.0,
            side_effects = [ShaderComputeLighting, SceneInvalidate];
    }
}

define_settings_group! {
    /// Full mode: Classic lighting + a directional shadow map.
    group_global FullShadingSettings {
        shadow_map_size: i32 = 1024,
            name = "shadow_map_size",
            min = 64.0, max = 4096.0,
            side_effects = [ShaderReload, SceneInvalidate];
        shadow_bias: f32 = 0.01,
            name = "shadow_bias",
            min = 0.0, max = 1.0,
            side_effects = [SceneInvalidate];
        shadow_intensity: f32 = 0.5,
            name = "shadow_intensity",
            min = 0.0, max = 1.0,
            side_effects = [SceneInvalidate];
        shadow_pcf: i32 = 2,
            name = "shadow_pcf",
            min = 1.0, max = 4.0,
            side_effects = [SceneInvalidate];
    }
}
