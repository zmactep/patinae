use super::state::*;
use crate::passes::lighting::MAX_ATLAS_DIRECTIONS;
use crate::passes::selection_dots::uses_selection_dots_fallback;
use crate::postprocess::marking::MarkingPass;
use crate::postprocess::silhouette::SilhouetteParams;
use crate::postprocess::silhouette::SilhouettePass;
use crate::postprocess::ssao_compose::SsaoComposeParams;

impl RenderState {
    /// Configure the screen-space selection / hover outline width used by the
    /// marking pass. The host value comes from `ui.selection_width`.
    pub fn set_marking_width(&mut self, width: f32) {
        let width = width.clamp(0.5, 20.0);
        if (self.screen.marking_width - width).abs() <= f32::EPSILON {
            return;
        }
        self.screen.marking_width = width;
        self.screen.marking_params_dirty = true;
        self.refresh_marking_resources();
    }

    /// Enable / disable the selection and hover visual overlay. Picking
    /// readbacks and silhouettes are intentionally controlled separately.
    pub fn set_selection_overlay_enabled(&mut self, enabled: bool) {
        if !enabled {
            self.screen.selection_dots_enabled = false;
            self.screen.atom_markers_rebuild_all = true;
        }

        if enabled && uses_selection_dots_fallback(self.memory.policy) {
            let was_enabled = self.screen.selection_dots_enabled;
            if !self.memory.warned_selection_denied {
                let message = format!(
                    "Full selection and hover overlays disabled by render memory profile {}; using selected atom dots.",
                    self.memory.policy.profile
                );
                log::warn!("{message}");
                self.memory.pending_warnings.push(message);
                self.memory.warned_selection_denied = true;
            }
            self.screen.selection_overlay_enabled = false;
            self.screen.selection_dots_enabled = true;
            self.screen.bindings.marking_bind_groups = None;
            self.screen.bindings.fxaa_overlay_bind_group = None;
            self.targets.clear_marking_targets();
            if !was_enabled {
                self.screen.atom_markers_rebuild_all = true;
            }
            return;
        }

        if enabled && !self.memory.policy.overlays.selection_enabled {
            if !self.memory.warned_selection_denied {
                let message = format!(
                    "Selection overlay disabled by render memory profile {}.",
                    self.memory.policy.profile
                );
                log::warn!("{message}");
                self.memory.pending_warnings.push(message);
                self.memory.warned_selection_denied = true;
            }
            self.screen.selection_overlay_enabled = false;
            self.screen.bindings.marking_bind_groups = None;
            self.screen.bindings.fxaa_overlay_bind_group = None;
            self.targets.clear_marking_targets();
            return;
        }
        if enabled {
            self.screen.selection_dots_enabled = false;
            self.screen.atom_markers_rebuild_all = true;
        }
        if enabled && self.screen.marking.is_none() {
            self.screen.marking = Some(MarkingPass::new(&self.ctx));
        }
        if self.screen.selection_overlay_enabled == enabled {
            if !enabled {
                self.screen.bindings.marking_bind_groups = None;
                self.screen.bindings.fxaa_overlay_bind_group = None;
                self.targets.clear_marking_targets();
                self.screen.marking_bindings_dirty = true;
                self.screen.marking_params_dirty = true;
                self.screen.marking_offsets_dirty = true;
            }
            return;
        }
        self.screen.selection_overlay_enabled = enabled;
        if !enabled {
            self.screen.bindings.marking_bind_groups = None;
            self.screen.bindings.fxaa_overlay_bind_group = None;
            self.targets.clear_marking_targets();
            self.screen.marking_bindings_dirty = true;
            self.screen.marking_params_dirty = true;
            self.screen.marking_offsets_dirty = true;
        } else {
            self.screen.marking_bindings_dirty = true;
            self.screen.marking_params_dirty = true;
            self.screen.marking_offsets_dirty = true;
            self.refresh_marking_resources();
        }
    }

    /// Enable / configure the silhouette pass. Call with `enabled = false`
    /// to skip it (matches `silhouettes = false` in patinae-settings).
    /// `thickness` is in visible overlay pixels.
    pub fn set_silhouette(&mut self, enabled: bool, thickness: f32, color: [f32; 4]) {
        if enabled && !self.memory.policy.overlays.silhouette_enabled {
            if !self.memory.warned_silhouette_denied {
                let message = format!(
                    "Silhouette overlay disabled by render memory profile {}.",
                    self.memory.policy.profile
                );
                log::warn!("{message}");
                self.memory.pending_warnings.push(message);
                self.memory.warned_silhouette_denied = true;
            }
            self.screen.silhouette_params = None;
            self.screen.bindings.silhouette_bind_group = None;
            return;
        }
        if enabled != self.screen.silhouette_params.is_some() {
            self.discard_capture_targets();
        }
        if enabled && self.screen.silhouette.is_none() {
            self.screen.silhouette = Some(SilhouettePass::new(&self.ctx));
        }
        self.screen.silhouette_params = enabled.then_some(SilhouetteParams {
            color,
            thickness,
            padding: [0.0; 3],
        });
    }

    /// Enable / disable FXAA. When enabled every render pass routes to
    /// `targets.color_scratch_view` and a final fragment-shader pass
    /// reads it + writes to the host `target` with Lottes-style edge
    /// blending. Off by default.
    pub fn set_fxaa(&mut self, enabled: bool) {
        if enabled && !self.memory.policy.postprocess.fxaa_enabled {
            if !self.memory.warned_fxaa_denied {
                let message = format!(
                    "FXAA disabled by render memory profile {}.",
                    self.memory.policy.profile
                );
                log::warn!("{message}");
                self.memory.pending_warnings.push(message);
                self.memory.warned_fxaa_denied = true;
            }
            self.screen.fxaa_enabled = false;
            self.screen.bindings.fxaa_bind_group = None;
            self.screen.bindings.fxaa_overlay_bind_group = None;
            return;
        }
        if enabled == self.screen.fxaa_enabled {
            return;
        }

        self.discard_capture_targets();
        self.screen.fxaa_enabled = enabled;

        if enabled {
            self.targets.ensure_color_scratch(&self.ctx.device);
            self.refresh_fxaa_bind_group();
        }
    }

    /// Set the output background for compositing the premultiplied scene.
    pub fn set_clear_color(&mut self, color: [f32; 3], opaque_background: bool) {
        self.screen.clear_color = Self::background_color(color, opaque_background);
    }

    pub(crate) fn clear_color(&self) -> wgpu::Color {
        self.screen.clear_color
    }

    /// Convert the scene background to a premultiplied render-target clear value.
    pub fn background_color(color: [f32; 3], opaque_background: bool) -> wgpu::Color {
        if opaque_background {
            wgpu::Color {
                r: f64::from(color[0]),
                g: f64::from(color[1]),
                b: f64::from(color[2]),
                a: 1.0,
            }
        } else {
            wgpu::Color::TRANSPARENT
        }
    }

    /// Enable / configure SSAO. Off by default. `radius` and `bias` are
    /// world-space (Å); `intensity` is the strength multiplier on the
    /// final compose. Settings flow from `patinae-settings::SsaoSettings`
    /// via the host bridge; toggling on a host-side `set ssao_enabled,
    /// 1` propagates here.
    pub fn set_ssao(&mut self, enabled: bool, radius: f32, intensity: f32, bias: f32) {
        if enabled && !self.memory.policy.postprocess.ssao_enabled {
            if !self.memory.warned_ssao_denied {
                let message = format!(
                    "SSAO disabled by render memory profile {}.",
                    self.memory.policy.profile
                );
                log::warn!("{message}");
                self.memory.pending_warnings.push(message);
                self.memory.warned_ssao_denied = true;
            }
            self.screen.ssao_enabled = false;
            return;
        }
        if enabled != self.screen.ssao_enabled {
            self.discard_capture_targets();
        }
        self.screen.ssao_enabled = enabled;

        self.screen.ssao_radius_bias = [radius, bias];

        if enabled {
            if self.targets.ensure_ssao_targets(&self.ctx.device)
                || self.screen.bindings.ssao.is_none()
            {
                self.refresh_ssao_bind_groups();
            }

            self.ctx.queue.write_buffer(
                &self.screen.ssao_compose.params_buffer,
                0,
                bytemuck::bytes_of(&SsaoComposeParams {
                    intensity,

                    _pad0: 0.0,

                    _pad1: 0.0,

                    _pad2: 0.0,
                }),
            );
        }
    }

    /// Enable and configure the directional shadow-map pass. `map_size` is
    /// rounded up to a power of two inside the portable 64..=4096 range.
    /// `pcf_radius` is the comparison-filter radius in shadow-map texels.
    pub fn set_shadows(
        &mut self,

        enabled: bool,

        map_size: u32,

        bias: f32,

        intensity: f32,

        pcf_radius: u32,
    ) {
        if enabled && intensity > 0.0 {
            self.lighting.shadow_mode = ShadowPassMode::Directional;

            let requested = map_size.clamp(64, 4096).next_power_of_two().min(4096);
            let capped = requested.min(self.memory.policy.shadows.max_shadow_map_size);
            if capped < requested && !self.memory.warned_shadow_clamped {
                let message = format!(
                    "Shadow map size clamped from {} to {} by render memory profile {}.",
                    requested, capped, self.memory.policy.profile
                );
                log::warn!("{message}");
                self.memory.pending_warnings.push(message);
                self.memory.warned_shadow_clamped = true;
            }
            self.lighting.shadow_map_size = capped;

            self.lighting.shadow_bias = bias.max(0.0);

            self.lighting.shadow_intensity = intensity.clamp(0.0, 1.0);

            self.lighting.shadow_pcf_radius = pcf_radius.min(4) as f32;

            self.lighting
                .directional_shadow
                .ensure_size(&self.ctx.device, self.lighting.shadow_map_size);
        } else {
            if self.lighting.shadow_mode == ShadowPassMode::Directional {
                self.lighting.shadow_mode = ShadowPassMode::Disabled;
            }

            if self.lighting.shadow_mode == ShadowPassMode::Disabled {
                self.lighting.shadow_intensity = 0.0;

                self.ctx.lighting.reset_disabled(
                    &self.ctx.device,
                    &self.ctx.queue,
                    self.lighting.directional_shadow.size,
                );
            }
        }
    }

    /// Enable and configure multi-directional atlas ambient occlusion used
    /// by Skripkin shading. `map_size` is the tile size for each direction;
    /// the atlas grid is chosen from `directions`.
    pub fn set_skripkin_ao(
        &mut self,

        enabled: bool,

        directions: u32,

        map_size: u32,

        bias: f32,

        intensity: f32,
    ) {
        if enabled && intensity > 0.0 {
            self.lighting.shadow_mode = ShadowPassMode::AtlasAo;

            let requested_directions = directions.clamp(1, MAX_ATLAS_DIRECTIONS as u32);
            self.lighting.skripkin_directions =
                requested_directions.min(self.memory.policy.shadows.max_atlas_directions);

            let requested_map_size = map_size.clamp(32, 4096).next_power_of_two().min(4096);
            self.lighting.skripkin_map_size =
                requested_map_size.min(self.memory.policy.shadows.max_atlas_tile_size);
            if (self.lighting.skripkin_directions < requested_directions
                || self.lighting.skripkin_map_size < requested_map_size)
                && !self.memory.warned_atlas_clamped
            {
                let message = format!(
                    "Atlas AO clamped by render memory profile {}: directions {} -> {}, tile {} -> {}.",
                    self.memory.policy.profile,
                    requested_directions,
                    self.lighting.skripkin_directions,
                    requested_map_size,
                    self.lighting.skripkin_map_size
                );
                log::warn!("{message}");
                self.memory.pending_warnings.push(message);
                self.memory.warned_atlas_clamped = true;
            }

            self.lighting.skripkin_bias = bias.max(0.0);

            self.lighting.skripkin_intensity = intensity.clamp(0.0, 2.0);

            self.lighting.atlas_ao.ensure_atlas(
                &self.ctx.device,
                self.lighting.skripkin_directions,
                self.lighting.skripkin_map_size,
            );
        } else if self.lighting.shadow_mode == ShadowPassMode::AtlasAo {
            self.lighting.shadow_mode = ShadowPassMode::Disabled;

            self.lighting.skripkin_intensity = 0.0;

            self.lighting.atlas_ao.cache.invalidate();

            self.ctx.lighting.reset_disabled(
                &self.ctx.device,
                &self.ctx.queue,
                self.lighting.atlas_ao.size,
            );
        }
    }
}
