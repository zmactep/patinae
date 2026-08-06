use super::state::*;
use std::collections::HashMap;

use crate::passes::selection_dots::{object_marker_bits, should_rebuild_marker_indices};
use crate::render_input::RenderInput;
#[cfg(feature = "stats")]
use crate::stats::Pass as StatsPass;

impl RenderState {
    pub(super) fn sync_atom_markers(
        &mut self,
        input: &RenderInput<'_>,
        effective_dirty_by_object: &HashMap<u32, patinae_mol::DirtyFlags>,
    ) {
        let rebuild_all = self.screen.atom_markers_rebuild_all;
        let atom_markers = &mut self.screen.atom_markers;

        atom_markers.retain_objects(input.objects.iter().map(|object| object.object_id.0));
        for object in input.objects {
            let dirty = effective_dirty_by_object
                .get(&object.object_id.0)
                .copied()
                .unwrap_or(object.dirty);
            let recent_radius_scale = if object.draw_reps.is_visible(patinae_mol::RepMask::SPHERES)
            {
                object
                    .object_settings
                    .as_ref()
                    .unwrap_or(input.settings)
                    .sphere
                    .scale
                    .max(1.0)
            } else {
                1.0
            };
            let rebuild = rebuild_all
                || should_rebuild_marker_indices(
                    dirty,
                    atom_markers.selected_indices(object.object_id.0),
                    atom_markers.recent_indices(object.object_id.0),
                    object.marker_updates,
                    self.screen.selection_dots_enabled,
                );
            if !rebuild {
                continue;
            }
            if !rebuild_all
                && !dirty.contains(patinae_mol::DirtyFlags::TOPOLOGY)
                && !object.marker_updates.is_empty()
            {
                atom_markers.sync_object_updates(
                    &self.ctx.device,
                    &self.ctx.queue,
                    object.object_id.0,
                    object.marker_updates,
                    self.screen.selection_dots_enabled,
                    recent_radius_scale,
                );
                continue;
            }
            let Some(slot) = self.scene.scene_store.slot(object.object_id).copied() else {
                continue;
            };
            let marker_bits = object_marker_bits(self.scene.scene_store.marker_lut.cpu(), slot);
            atom_markers.sync_object(
                &self.ctx.device,
                &self.ctx.queue,
                object.object_id.0,
                marker_bits,
                self.screen.selection_dots_enabled,
                recent_radius_scale,
            );
        }
        self.screen.atom_markers_rebuild_all = false;
    }

    pub(super) fn refresh_marking_resources(&mut self) {
        let (Some(marking), Some(overlay_id), Some(marker_lut), Some(mask)) = (
            self.screen.marking.as_ref(),
            self.targets.overlay_id.as_ref(),
            self.scene.scene_store.marker_lut_buffer(),
            self.targets.marking_mask.as_ref(),
        ) else {
            self.screen.marking_bind_groups = None;
            self.screen.marking_bindings_dirty = true;
            self.screen.marking_params_dirty = true;
            self.screen.marking_offsets_dirty = true;

            return;
        };
        let Some(scene_color) = self.targets.color_scratch_view.as_ref() else {
            self.screen.marking_bind_groups = None;
            self.screen.marking_bindings_dirty = true;
            return;
        };

        if self.screen.marking_offsets_dirty {
            marking
                .upload_object_offsets(&self.ctx.queue, self.scene.scene_store.iter_atom_offsets());
            self.screen.marking_offsets_dirty = false;
        }

        if self.screen.marking_params_dirty {
            marking.upload_params(
                &self.ctx.queue,
                (self.targets.width, self.targets.height),
                self.screen.marking_width,
            );
            self.screen.marking_params_dirty = false;
        }

        if self.screen.marking_bind_groups.is_none() || self.screen.marking_bindings_dirty {
            self.screen.marking_bind_groups = Some(marking.make_bind_groups(
                &self.ctx.device,
                overlay_id,
                marker_lut,
                scene_color,
                mask,
            ));
            self.screen.marking_bindings_dirty = false;
        }
    }

    pub(super) fn refresh_ssao_bind_groups(&mut self) {
        let (Some(ssao_view), Some(ssao_blurred_view)) = (
            self.targets.ssao_view.as_ref(),
            self.targets.ssao_blurred_view.as_ref(),
        ) else {
            self.screen.ssao_bind_group = None;
            self.screen.ssao_blur_h_bind_group = None;
            self.screen.ssao_blur_v_bind_group = None;
            self.screen.ssao_compose_bind_group = None;
            return;
        };

        self.screen.ssao_bind_group = Some(self.screen.ssao_compute.make_bind_group(
            &self.ctx.device,
            &self.ctx.frame.buffer,
            &self.screen.ssao_resources.ssao_params_buffer,
            &self.targets.depth,
            ssao_view,
        ));
        self.screen.ssao_blur_h_bind_group = Some(self.screen.ssao_blur.make_bind_group(
            &self.ctx.device,
            &self.screen.ssao_resources.blur_h_params_buffer,
            &self.targets.depth,
            ssao_view,
            ssao_blurred_view,
        ));
        self.screen.ssao_blur_v_bind_group = Some(self.screen.ssao_blur.make_bind_group(
            &self.ctx.device,
            &self.screen.ssao_resources.blur_v_params_buffer,
            &self.targets.depth,
            ssao_blurred_view,
            ssao_view,
        ));
        self.screen.ssao_compose_bind_group = Some(
            self.screen
                .ssao_compose
                .make_bind_group(&self.ctx.device, ssao_view),
        );
    }

    pub(super) fn refresh_fxaa_bind_group(&mut self) {
        self.screen.fxaa_bind_group = self.targets.color_scratch_view.as_ref().map(|view| {
            self.screen
                .fxaa_pass
                .make_bind_group(&self.ctx.device, view)
        });
    }

    pub(super) fn refresh_overlay_bind_groups(&mut self) {
        self.screen.silhouette_bind_group = match (
            self.screen.silhouette.as_ref(),
            self.targets.overlay_id.as_ref(),
        ) {
            (Some(s), Some(overlay_id)) => Some(s.make_bind_group(&self.ctx.device, overlay_id)),
            _ => None,
        };

        self.screen.fxaa_overlay_bind_group = match self.targets.overlay_color_scratch.as_ref() {
            Some(view) => Some(
                self.screen
                    .fxaa_pass
                    .make_bind_group(&self.ctx.device, view),
            ),
            None => None,
        };

        self.refresh_marking_resources();
    }

    pub(super) fn record_ssao_chain(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
    ) {
        if !self.screen.ssao_enabled {
            return;
        }
        let (
            Some(ssao_bind_group),
            Some(ssao_blur_h_bind_group),
            Some(ssao_blur_v_bind_group),
            Some(ssao_compose_bind_group),
        ) = (
            self.screen.ssao_bind_group.as_ref(),
            self.screen.ssao_blur_h_bind_group.as_ref(),
            self.screen.ssao_blur_v_bind_group.as_ref(),
            self.screen.ssao_compose_bind_group.as_ref(),
        )
        else {
            return;
        };
        self.screen.ssao_frame_counter = self.screen.ssao_frame_counter.wrapping_add(1);

        #[cfg(feature = "stats")]
        let ssao_ts = self.stats.compute_pass_timestamp_writes(StatsPass::Ssao);
        #[cfg(not(feature = "stats"))]
        let ssao_ts = None;
        self.screen
            .ssao_compute
            .dispatch(encoder, ssao_bind_group, &self.targets, ssao_ts);

        #[cfg(feature = "stats")]
        let blur_ts = self
            .stats
            .compute_pass_timestamp_writes(StatsPass::SsaoBlur);
        #[cfg(not(feature = "stats"))]
        let blur_ts = None;
        self.screen
            .ssao_blur
            .dispatch(encoder, ssao_blur_h_bind_group, &self.targets, blur_ts);
        self.screen
            .ssao_blur
            .dispatch(encoder, ssao_blur_v_bind_group, &self.targets, None);

        #[cfg(feature = "stats")]
        let compose_ts = self
            .stats
            .render_pass_timestamp_writes(StatsPass::SsaoCompose);
        #[cfg(not(feature = "stats"))]
        let compose_ts = None;
        self.screen
            .ssao_compose
            .record(target, encoder, ssao_compose_bind_group, compose_ts);
    }

    pub(super) fn record_marking_overlay(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        has_marker: bool,
    ) {
        if !has_marker {
            return;
        }
        let (Some(marking), Some(marking_bg), Some(mask)) = (
            self.screen.marking.as_ref(),
            self.screen.marking_bind_groups.as_ref(),
            self.targets.marking_mask.as_ref(),
        ) else {
            return;
        };

        #[cfg(feature = "stats")]
        let marking_ts = self.stats.render_pass_timestamp_writes(StatsPass::Marking);
        #[cfg(not(feature = "stats"))]
        let marking_ts = None;
        marking.record_mask(encoder, mask, &marking_bg.mask, marking_ts);
        marking.record_composite(encoder, target, &marking_bg.composite);
    }

    pub(super) fn record_atom_markers(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
    ) {
        let atom_markers = &self.screen.atom_markers;
        if !atom_markers.has_markers() {
            return;
        }
        atom_markers.upload_params(&self.ctx.queue, self.screen.marking_width);
        atom_markers.record(
            encoder,
            target,
            &self.targets.depth,
            &self.ctx.frame.bind_group,
            &self.ctx.lighting.bind_group,
            &self.scene.scene_store,
        );
    }

    pub(super) fn record_silhouette_overlay(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
    ) {
        let (Some(silhouette), Some(silhouette_bg), Some(params)) = (
            self.screen.silhouette.as_ref(),
            self.screen.silhouette_bind_group.as_ref(),
            self.screen.silhouette_params,
        ) else {
            return;
        };
        silhouette.upload_params(&self.ctx.queue, params);

        #[cfg(feature = "stats")]
        let silhouette_ts = self
            .stats
            .render_pass_timestamp_writes(StatsPass::Silhouette);
        #[cfg(not(feature = "stats"))]
        let silhouette_ts = None;
        silhouette.record(encoder, target, silhouette_bg, silhouette_ts);
    }

    pub(super) fn record_fxaa(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        use_overlay_source: bool,
    ) {
        if !self.screen.fxaa_enabled {
            return;
        }
        let bg = if use_overlay_source {
            self.screen
                .fxaa_overlay_bind_group
                .as_ref()
                .or(self.screen.fxaa_bind_group.as_ref())
        } else {
            self.screen.fxaa_bind_group.as_ref()
        };
        let Some(bg) = bg else {
            return;
        };
        #[cfg(feature = "stats")]
        let fxaa_ts = self.stats.render_pass_timestamp_writes(StatsPass::Fxaa);
        #[cfg(not(feature = "stats"))]
        let fxaa_ts = None;
        self.screen.fxaa_pass.record(target, encoder, bg, fxaa_ts);
    }
}
