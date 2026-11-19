use super::math::hash_view_proj;
use super::picking_flow::PickingStrategy;
use super::state::*;
use crate::picking::RepKind;
use crate::representations::sphere::{SphereLodDiagnostics, SphereRep};
use crate::representations::stick::{StickLodDiagnostics, StickRep};
#[cfg(feature = "stats")]
use crate::stats::FrameStats;

impl RenderState {
    /// Return aggregate sphere Auto-LOD diagnostics for hosts and benches.
    pub fn sphere_lod_diagnostics(&self) -> SphereLodDiagnostics {
        let mut out = SphereLodDiagnostics::default();
        for entry in self.scene.reps.values() {
            if entry.rep.kind() != RepKind::Sphere {
                continue;
            }
            if let Some(rep) = entry.rep.as_any().downcast_ref::<SphereRep>() {
                out.include(rep.lod_diagnostics());
            }
        }
        out
    }

    /// Return aggregate stick Auto-LOD diagnostics for hosts and benches.
    pub fn stick_lod_diagnostics(&self) -> StickLodDiagnostics {
        let mut out = StickLodDiagnostics::default();
        for entry in self.scene.reps.values() {
            if entry.rep.kind() != RepKind::Stick {
                continue;
            }
            if let Some(rep) = entry.rep.as_any().downcast_ref::<StickRep>() {
                out.include(rep.lod_diagnostics());
            }
        }
        out
    }

    /// Take the most recent fully-resolved frame's instrumentation. Returns
    /// `None` until the GPU has caught up with the first ~2 frames after
    /// startup. Compiled out when the `stats` feature is off.
    #[cfg(feature = "stats")]
    pub fn take_frame_stats(&self) -> Option<FrameStats> {
        self.stats.take_latest()
    }

    pub(super) fn prepare_instance_draws(&self, encoder: &mut wgpu::CommandEncoder) {
        for key in &self.scene.draw_order {
            if self
                .scene
                .scene_store
                .needs_raw_draw(crate::picking::ObjectId(key.0))
            {
                if let Some(entry) = self.scene.reps.get(key) {
                    entry.rep.prepare_shadow_depth(encoder, &self.ctx.queue);
                }
            }
        }
    }

    /// Run the full frame: culling, lighting, optional visual overlays,
    /// visible geometry, postprocess, and stats resolution.
    pub fn render(&mut self, target: &wgpu::TextureView, encoder: &mut wgpu::CommandEncoder) {
        if self.memory.sync_error.is_some() {
            return;
        }
        let needs_overlay_id = self.prepare_viewport_targets();
        let uniforms = self.uniforms;
        let hash = self.prepare_frame_geometry(encoder, &uniforms, FramePurpose::Viewport);
        let camera_changed = hash != self.picking.last_view_proj_hash;
        self.refresh_visible_picking_texture(encoder, hash, camera_changed, false);
        if needs_overlay_id {
            self.refresh_overlay_id_texture(encoder, hash);
        }

        let phase = (self.screen.ssao_frame_counter as f32 * 0.618_034).fract();
        if self.screen.ssao_enabled {
            self.screen.ssao_frame_counter = self.screen.ssao_frame_counter.wrapping_add(1);
        }
        self.render_frame(
            encoder,
            RenderFrame {
                targets: &self.targets,
                bindings: &self.screen.bindings,
                uniform_bind_group: &self.ctx.frame.bind_group,
                output: target,
                background: self.screen.clear_color,
                purpose: FramePurpose::Viewport,
                ssao_phase: phase,
            },
        );
        self.scene.scene_dirty = false;
        #[cfg(feature = "stats")]
        self.finish_frame_stats(encoder);
    }

    fn wants_selection_overlay(&self) -> bool {
        self.screen.selection_overlay_enabled
            && self.memory.policy.overlays.selection_enabled
            && self.scene.has_any_marker
            && self.screen.marking.is_some()
    }

    fn prepare_viewport_targets(&mut self) -> bool {
        let wants_selection_overlay = self.wants_selection_overlay();
        let has_silhouette = self.memory.policy.overlays.silhouette_enabled
            && self.screen.silhouette_params.is_some()
            && self.screen.silhouette.is_some();
        let wants_overlay_id = wants_selection_overlay || has_silhouette;
        let needs_color_scratch = self.screen.fxaa_enabled || wants_selection_overlay;
        if needs_color_scratch {
            let color_allocated = self.targets.ensure_color_scratch(&self.ctx.device);
            if color_allocated {
                self.refresh_fxaa_bind_group();
                self.screen.marking_bindings_dirty = true;
            }
        }

        if wants_overlay_id {
            self.ensure_id_pass();
            let id_allocated = self.targets.ensure_overlay_id_targets(&self.ctx.device);
            if id_allocated {
                self.invalidate_overlay_id_cache();
            }
            let marking_allocated = if wants_selection_overlay {
                self.targets.ensure_marking_targets(
                    &self.ctx.device,
                    self.ctx.color_format,
                    self.screen.fxaa_enabled,
                )
            } else {
                self.targets.clear_marking_targets();
                self.screen.bindings.marking_bind_groups = None;
                self.screen.bindings.fxaa_overlay_bind_group = None;
                false
            };
            if id_allocated || marking_allocated {
                self.screen.marking_bindings_dirty = true;
                self.screen.marking_params_dirty = true;
            }
            if id_allocated
                || marking_allocated
                || self.screen.bindings.silhouette_bind_group.is_none() && has_silhouette
                || self.screen.bindings.marking_bind_groups.is_none() && wants_selection_overlay
                || self.screen.bindings.fxaa_overlay_bind_group.is_none()
                    && wants_selection_overlay
                    && self.screen.fxaa_enabled
            {
                self.refresh_overlay_bind_groups();
            }
        } else if self.targets.overlay_id.is_some() {
            self.targets.clear_overlay_targets();
            self.screen.bindings.silhouette_bind_group = None;
            self.screen.bindings.marking_bind_groups = None;
            self.screen.bindings.fxaa_overlay_bind_group = None;
            self.invalidate_overlay_id_cache();
        }

        has_silhouette
            || wants_selection_overlay && self.screen.bindings.marking_bind_groups.is_some()
    }

    pub(super) fn prepare_frame_geometry(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        uniforms: &crate::FrameUniforms,
        purpose: FramePurpose,
    ) -> u64 {
        #[cfg(feature = "stats")]
        self.stats.mark_record_start();
        let hash = hash_view_proj(&uniforms.view_proj);
        if purpose == FramePurpose::Viewport {
            self.poll_viewport_lod();
        }
        let compute_rebuilt = self.record_pending_compute_builds(encoder);
        self.prepare_instance_draws(encoder);
        self.dispatch_cull(encoder, uniforms, hash, compute_rebuilt);
        if purpose == FramePurpose::Viewport {
            self.record_viewport_lod_readbacks(encoder);
        }
        self.record_shadow_pass(encoder, uniforms);
        hash
    }

    #[cfg(feature = "stats")]
    pub(super) fn finish_frame_stats(&mut self, encoder: &mut wgpu::CommandEncoder) {
        self.stats.mark_record_end();
        self.stats.resolve_and_map(encoder, &self.ctx.queue);
    }

    pub(super) fn render_frame(&self, encoder: &mut wgpu::CommandEncoder, frame: RenderFrame<'_>) {
        let has_selection_overlay =
            frame.bindings.marking_bind_groups.is_some() && self.wants_selection_overlay();
        let effective_target = if self.screen.fxaa_enabled || has_selection_overlay {
            frame
                .targets
                .color_scratch_view
                .as_ref()
                .unwrap_or(frame.output)
        } else {
            frame.output
        };
        let n_wboit = self.count_wboit_reps();
        let n_fast_overlay = self.count_fast_overlay_reps();
        self.record_opaque_pass(&frame, encoder, effective_target);
        self.record_ssao_chain(&frame, encoder, effective_target);
        self.record_fast_overlay_pass(&frame, encoder, effective_target, n_fast_overlay);
        self.record_translucent_pass(&frame, encoder, n_wboit);
        self.record_wboit_composite(&frame, encoder, effective_target, n_wboit);
        self.record_stroke_overlay(&frame, encoder, effective_target);
        self.record_silhouette_overlay(&frame, encoder, effective_target);
        if frame.purpose == FramePurpose::Viewport {
            self.record_atom_markers(&frame, encoder, effective_target);
        }
        if has_selection_overlay {
            if let Some(overlay_target) = if self.screen.fxaa_enabled {
                frame.targets.overlay_color_scratch.as_ref()
            } else {
                Some(frame.output)
            } {
                self.record_marking_overlay(&frame, encoder, overlay_target);
            }
        }
        self.record_fxaa(&frame, encoder, has_selection_overlay);
    }

    pub(super) fn refresh_visible_picking_texture(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        view_proj_hash: u64,
        camera_changed: bool,
        needs_picking_texture: bool,
    ) {
        let needs_exact_overlay_texture = needs_picking_texture
            && self.picking.picking_texture_source == Some(PickingTextureSource::Reprojected);
        let strategy = if needs_exact_overlay_texture {
            PickingStrategy::FullRecord
        } else {
            self.pick_picking_strategy(camera_changed, needs_picking_texture, false)
        };

        match strategy {
            PickingStrategy::Skip => {}
            PickingStrategy::FullRecord => {
                self.ensure_id_pass();
                self.record_picking_pass(encoder);
                self.picking.region = None;
                self.snapshot_picking_to_prev(encoder);
                self.picking.picking_pass_initialized = true;
                self.picking.last_view_proj_hash = view_proj_hash;
                self.picking.last_full_record_view_proj = Some(self.uniforms.view_proj);
                self.picking.reproject_count = 0;
                self.picking.picking_texture_source = Some(PickingTextureSource::FullRecord);
            }
            PickingStrategy::Reproject => {
                self.dispatch_reproject(encoder);
                self.picking.last_view_proj_hash = view_proj_hash;
                self.picking.reproject_count = self.picking.reproject_count.saturating_add(1);
                self.picking.picking_texture_source = Some(PickingTextureSource::Reprojected);
            }
        }
    }
}
