use super::picking_budget::uses_lazy_manual_picking;
use super::state::*;
use crate::frame::FrameTargets;
use crate::picking::PickingMode;

impl RenderState {
    pub fn resize(&mut self, viewport: (u32, u32)) {
        self.discard_capture_targets();
        let lazy_manual_picking = uses_lazy_manual_picking(self.memory.policy);
        let with_picking =
            self.picking.picking_mode != PickingMode::Disabled && !lazy_manual_picking;

        self.targets = FrameTargets::new_with_policy(
            &self.ctx.device,
            viewport.0,
            viewport.1,
            with_picking,
            self.ctx.color_format,
            self.memory.policy.frame_targets,
            self.memory.policy.picking.scale,
        );
        if lazy_manual_picking {
            self.clear_hit_test_picking_resources();
        } else {
            self.picking.budget_allowed = with_picking;
        }

        self.uniforms.set_viewport(viewport.0, viewport.1);

        self.ctx.upload_frame(&self.uniforms);

        self.screen.bindings.composite_bind_group = self.screen.composite.make_bind_group(
            &self.ctx.device,
            &self.targets.accum,
            &self.targets.reveal,
        );

        let needs_selection_overlay = self.screen.selection_overlay_enabled
            && self.memory.policy.overlays.selection_enabled
            && self.scene.has_any_marker;
        let needs_overlay_id = self.screen.silhouette_params.is_some() || needs_selection_overlay;
        if self.screen.fxaa_enabled || needs_selection_overlay {
            self.targets.ensure_color_scratch(&self.ctx.device);
        }
        self.refresh_fxaa_bind_group();
        if needs_overlay_id {
            self.targets.ensure_overlay_id_targets(&self.ctx.device);
        }
        if needs_selection_overlay {
            self.targets.ensure_marking_targets(
                &self.ctx.device,
                self.ctx.color_format,
                self.screen.fxaa_enabled,
            );
        } else {
            self.targets.clear_marking_targets();
        }
        self.screen.marking_bindings_dirty = true;
        self.screen.marking_params_dirty = true;
        self.refresh_overlay_bind_groups();

        self.scene.scene_dirty = true;
        self.invalidate_cull_cache();

        // Fresh `FrameTargets` — picking texture starts uninitialized, force
        // a re-record on the next pick/overlay regardless of camera state.
        self.invalidate_picking_cache();
        self.invalidate_overlay_id_cache();

        if let (Some(reproject), (pw, ph)) = (
            self.picking.picking_reproject.as_mut(),
            self.targets.picking_dims(),
        ) {
            reproject.ensure_best_depth_capacity(&self.ctx.device, pw, ph);
        }

        if self.screen.ssao_enabled {
            self.targets.ensure_ssao_targets(&self.ctx.device);
        }
        self.refresh_ssao_bind_groups();

        // Per-rep cull bind groups reference per-rep storage buffers

        // (raw/compacted/raw_count/indirect) — those are NOT recreated by

        // resize, so the cull bind groups stay valid. The frustum params

        // uniform is refreshed every frame inside `render()`.
    }
}
