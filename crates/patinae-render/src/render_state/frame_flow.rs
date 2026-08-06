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

    /// Run the full frame: culling, lighting, optional visual overlays,
    /// visible geometry, postprocess, and stats resolution.
    pub fn render(&mut self, target: &wgpu::TextureView, encoder: &mut wgpu::CommandEncoder) {
        #[cfg(feature = "stats")]
        self.stats.mark_record_start();

        let wants_selection_overlay = self.screen.selection_overlay_enabled
            && self.memory.policy.overlays.selection_enabled
            && self.scene.has_any_marker
            && self.screen.marking.is_some();
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
                self.screen.marking_bind_groups = None;
                self.screen.fxaa_overlay_bind_group = None;
                false
            };
            if id_allocated || marking_allocated {
                self.screen.marking_bindings_dirty = true;
                self.screen.marking_params_dirty = true;
            }
            if id_allocated
                || marking_allocated
                || self.screen.silhouette_bind_group.is_none() && has_silhouette
                || self.screen.marking_bind_groups.is_none() && wants_selection_overlay
                || self.screen.fxaa_overlay_bind_group.is_none()
                    && wants_selection_overlay
                    && self.screen.fxaa_enabled
            {
                self.refresh_overlay_bind_groups();
            }
        } else if self.targets.overlay_id.is_some() {
            self.targets.clear_overlay_targets();
            self.screen.silhouette_bind_group = None;
            self.screen.marking_bind_groups = None;
            self.screen.fxaa_overlay_bind_group = None;
            self.invalidate_overlay_id_cache();
        }

        let has_selection_overlay =
            wants_selection_overlay && self.screen.marking_bind_groups.is_some();
        let needs_overlay_id = has_selection_overlay || has_silhouette;
        let render_scene_to_scratch = self.screen.fxaa_enabled || has_selection_overlay;
        let effective_target: wgpu::TextureView = if render_scene_to_scratch {
            self.targets
                .color_scratch_view
                .as_ref()
                .cloned()
                .unwrap_or_else(|| target.clone())
        } else {
            target.clone()
        };
        let effective_target = &effective_target;

        let view_proj_hash = hash_view_proj(&self.uniforms.view_proj);
        let camera_changed = view_proj_hash != self.picking.last_view_proj_hash;

        self.poll_viewport_lod();
        let compute_rebuilt = self.record_pending_compute_builds(encoder);
        self.dispatch_cull(encoder, view_proj_hash, compute_rebuilt);
        self.record_viewport_lod_readbacks(encoder);
        self.record_shadow_pass(encoder);

        self.refresh_visible_picking_texture(encoder, view_proj_hash, camera_changed, false);
        if needs_overlay_id {
            self.refresh_overlay_id_texture(encoder, view_proj_hash);
        }

        let n_wboit = self.count_wboit_reps();
        let n_fast_overlay = self.count_fast_overlay_reps();
        self.record_opaque_pass(encoder, effective_target);
        self.record_ssao_chain(encoder, effective_target);
        self.record_fast_overlay_pass(encoder, effective_target, n_fast_overlay);
        self.record_translucent_pass(encoder, n_wboit);
        self.record_wboit_composite(encoder, effective_target, n_wboit);
        self.record_stroke_overlay(encoder, effective_target);
        self.record_silhouette_overlay(encoder, effective_target);
        self.record_atom_markers(encoder, effective_target);
        if has_selection_overlay {
            if let Some(overlay_target) = if self.screen.fxaa_enabled {
                self.targets.overlay_color_scratch.as_ref()
            } else {
                Some(target)
            } {
                self.record_marking_overlay(encoder, overlay_target, true);
            }
        }
        self.record_fxaa(encoder, target, has_selection_overlay);

        self.scene.scene_dirty = false;

        #[cfg(feature = "stats")]
        {
            self.stats.mark_record_end();
            self.stats.resolve_and_map(encoder, &self.ctx.queue);
        }
    }

    fn refresh_visible_picking_texture(
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
                self.record_picking_pass(encoder);
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
